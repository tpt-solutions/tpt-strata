//! A minimal, hand-written SQL surface for the v1 subset.
//!
//! Keeping the parser in this crate preserves the zero-external-dependency
//! promise for the core engine. Supported grammar:
//!
//! ```text
//! SELECT <column | agg(col) | *> [, ...]
//!   FROM <table> [JOIN <table> ON left.col = right.col]
//!   [WHERE col op literal [AND col op literal ...]]
//!   [GROUP BY col [, ...]]
//!   [ORDER BY col [ASC | DESC]]
//!   [LIMIT n] [;]
//! ```
//!
//! Supported aggregate functions: `SUM`, `COUNT`, `MIN`, `MAX`, `AVG`,
//! including `COUNT(*)`. Comparison ops: `=`, `!=`, `<>`, `>`, `>=`, `<`,
//! `<=`. Literals may be integers, floats, single-quoted strings, `TRUE`,
//! `FALSE`, and `NULL`.

use std::sync::Arc;

use crate::batch::Batch;
use crate::engine::{
    AggFunc as EngAggFunc, Aggregate, AggregateExec, Comparison, DataSource, Expr, FilterExec,
    JoinExec, LimitExec, PhysicalPlan, Predicate, ProjectExec, SortExec, SortExpr,
};
use crate::error::QueryError;
use crate::format::Table;
use crate::query::AggFunc;
use crate::schema::{DataType, Field, Schema};
use crate::types::Scalar;

const SUPPORTED_AGGS: &str = "SUM, COUNT, MIN, MAX, AVG";

// === AST ===

#[derive(Debug, Clone, PartialEq)]
enum ProjectionItem {
    Wild,
    Column {
        name: String,
        alias: Option<String>,
    },
    Aggregate {
        func: AggFunc,
        column: Option<String>,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Pred {
    Compare {
        column: String,
        op: Comparison,
        value: Scalar,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct JoinClause {
    right: String,
    on: (String, String),
}

#[derive(Debug, Clone, PartialEq)]
struct Select {
    projections: Vec<ProjectionItem>,
    from: Option<String>,
    join: Option<JoinClause>,
    predicates: Vec<Pred>,
    group_by: Vec<String>,
    order_by: Option<(String, bool)>,
    limit: Option<usize>,
}

// === Lexer ===

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Select,
    From,
    Where,
    Group,
    By,
    Order,
    Asc,
    Desc,
    Limit,
    Join,
    On,
    And,
    As,
    Ident(String),
    Number(f64),
    Int(i64),
    StrLit(String),
    True,
    False,
    Null,
    Star,
    Comma,
    Dot,
    LParen,
    RParen,
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    Minus,
    Semi,
    End,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Select => "SELECT".into(),
            Tok::From => "FROM".into(),
            Tok::Where => "WHERE".into(),
            Tok::Group => "GROUP".into(),
            Tok::By => "BY".into(),
            Tok::Order => "ORDER".into(),
            Tok::Asc => "ASC".into(),
            Tok::Desc => "DESC".into(),
            Tok::Limit => "LIMIT".into(),
            Tok::Join => "JOIN".into(),
            Tok::On => "ON".into(),
            Tok::And => "AND".into(),
            Tok::As => "AS".into(),
            Tok::Ident(s) => s.clone(),
            Tok::Number(n) => format!("{n}"),
            Tok::Int(n) => format!("{n}"),
            Tok::StrLit(_) => "string literal".into(),
            Tok::True => "TRUE".into(),
            Tok::False => "FALSE".into(),
            Tok::Null => "NULL".into(),
            Tok::Star => "*".into(),
            Tok::Comma => ",".into(),
            Tok::Dot => ".".into(),
            Tok::LParen => "(".into(),
            Tok::RParen => ")".into(),
            Tok::Eq => "=".into(),
            Tok::NotEq => "!=".into(),
            Tok::Gt => ">".into(),
            Tok::Gte => ">=".into(),
            Tok::Lt => "<".into(),
            Tok::Lte => "<=".into(),
            Tok::Minus => "-".into(),
            Tok::Semi => ";".into(),
            Tok::End => "end of input".into(),
        }
    }
}

fn sql_err(message: impl Into<String>) -> QueryError {
    QueryError::Sql {
        message: message.into(),
    }
}

/// Convert a character offset into a rendered diagnostic with line/column and
/// a caret pointing at the offending character.
fn sql_err_at(source: &str, offset: usize, message: impl Into<String>) -> QueryError {
    let (line, col) = line_col(source, offset);
    let mut rendered = format!("{} at line {line}, column {col}", message.into());
    if let Some(line_text) = source.lines().nth(line - 1) {
        rendered.push_str(&format!(
            "\n  {line_text}\n  {}^",
            " ".repeat(col.saturating_sub(1))
        ));
    }
    QueryError::Sql { message: rendered }
}

fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.chars().enumerate() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn lex(input: &str) -> Result<(Vec<Tok>, Vec<usize>), QueryError> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut toks = Vec::new();
    let mut posns = Vec::new();

    let mut push = |p: usize, t: Tok| {
        posns.push(p);
        toks.push(t);
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' | '\n' => i += 1,
            '(' => {
                push(i, Tok::LParen);
                i += 1;
            }
            ')' => {
                push(i, Tok::RParen);
                i += 1;
            }
            ',' => {
                push(i, Tok::Comma);
                i += 1;
            }
            '.' => {
                push(i, Tok::Dot);
                i += 1;
            }
            '*' => {
                push(i, Tok::Star);
                i += 1;
            }
            ';' => {
                push(i, Tok::Semi);
                i += 1;
            }
            '=' => {
                push(i, Tok::Eq);
                i += 1;
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    push(i, Tok::NotEq);
                    i += 2;
                } else {
                    return Err(sql_err_at(
                        input,
                        i,
                        format!("unexpected '!' at position {i}; use '!=' or '<>'"),
                    ));
                }
            }
            '<' => match chars.get(i + 1) {
                Some('=') => {
                    push(i, Tok::Lte);
                    i += 2;
                }
                Some('>') => {
                    push(i, Tok::NotEq);
                    i += 2;
                }
                _ => {
                    push(i, Tok::Lt);
                    i += 1;
                }
            },
            '>' => match chars.get(i + 1) {
                Some('=') => {
                    push(i, Tok::Gte);
                    i += 2;
                }
                _ => {
                    push(i, Tok::Gt);
                    i += 1;
                }
            },
            '-' => {
                push(i, Tok::Minus);
                i += 1;
            }
            '\'' => {
                let start = i;
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(sql_err_at(input, start, "unterminated string literal"))
                        }
                        Some('\'') => {
                            if chars.get(i + 1) == Some(&'\'') {
                                s.push('\'');
                                i += 2;
                            } else {
                                i += 1;
                                break;
                            }
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                push(start, Tok::StrLit(s));
            }
            '"' => {
                let start = i;
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(sql_err_at(input, start, "unterminated quoted identifier"))
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                push(start, Tok::Ident(s));
            }
            ch if ch.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let mut is_float = false;
                if chars.get(i) == Some(&'.')
                    && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())
                {
                    is_float = true;
                    i += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                if is_float {
                    let n: f64 = text.parse().map_err(|_| {
                        sql_err_at(input, start, format!("invalid number '{text}'"))
                    })?;
                    push(start, Tok::Number(n));
                } else {
                    let n: i64 = text.parse().map_err(|_| {
                        sql_err_at(input, start, format!("integer out of range '{text}'"))
                    })?;
                    push(start, Tok::Int(n));
                }
            }
            ch if ch.is_alphabetic() || ch == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let upper = text.to_ascii_uppercase();
                push(
                    start,
                    match upper.as_str() {
                        "SELECT" => Tok::Select,
                        "FROM" => Tok::From,
                        "WHERE" => Tok::Where,
                        "GROUP" => Tok::Group,
                        "BY" => Tok::By,
                        "ORDER" => Tok::Order,
                        "ASC" => Tok::Asc,
                        "DESC" => Tok::Desc,
                        "LIMIT" => Tok::Limit,
                        "JOIN" => Tok::Join,
                        "ON" => Tok::On,
                        "AND" => Tok::And,
                        "AS" => Tok::As,
                        "TRUE" => Tok::True,
                        "FALSE" => Tok::False,
                        "NULL" => Tok::Null,
                        _ => Tok::Ident(text),
                    },
                );
            }
            other => {
                return Err(sql_err_at(
                    input,
                    i,
                    format!("unexpected character '{other}' at position {i}"),
                ));
            }
        }
    }
    push(i, Tok::End);
    Ok((toks, posns))
}

// === Parser ===

struct Parser {
    toks: Vec<Tok>,
    posns: Vec<usize>,
    source: String,
    pos: usize,
}

impl Parser {
    fn new(toks: Vec<Tok>, posns: Vec<usize>, source: String) -> Self {
        Self {
            toks,
            posns,
            source,
            pos: 0,
        }
    }

    fn perr(&self, message: impl Into<String>) -> QueryError {
        let offset = self.posns.get(self.pos).copied().unwrap_or(0);
        sql_err_at(&self.source, offset, message)
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }

    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn accept(&mut self, tok: Tok) -> bool {
        if *self.peek() == tok {
            self.next();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: Tok) -> Result<(), QueryError> {
        if *self.peek() == tok {
            self.next();
            Ok(())
        } else {
            Err(self.perr(format!(
                "expected '{}' but found '{}'",
                tok.describe(),
                self.peek().describe()
            )))
        }
    }

    fn parse_column(&mut self) -> Result<String, QueryError> {
        let first = match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                s
            }
            other => {
                return Err(self.perr(format!(
                    "expected a column name but found '{}'",
                    other.describe()
                )));
            }
        };
        if self.accept(Tok::Dot) {
            let second = match self.peek().clone() {
                Tok::Ident(s) => {
                    self.next();
                    s
                }
                other => {
                    return Err(self.perr(format!(
                        "expected a column name after '.' but found '{}'",
                        other.describe()
                    )));
                }
            };
            Ok(format!("{first}.{second}"))
        } else {
            Ok(first)
        }
    }

    fn parse_projection_item(&mut self) -> Result<ProjectionItem, QueryError> {
        if self.accept(Tok::Star) {
            return Ok(ProjectionItem::Wild);
        }
        let name = self.parse_column()?;
        let mut item = if self.accept(Tok::LParen) {
            if name.eq_ignore_ascii_case("CAST") {
                return Err(self.perr(
                    "unsupported cast: tpt-strata v1 has no CAST support; provide values in the target type directly",
                ));
            }
            let column = if self.accept(Tok::Star) {
                None
            } else {
                Some(self.parse_column()?)
            };
            self.expect(Tok::RParen)?;
            let func = match name.to_ascii_uppercase().as_str() {
                "SUM" => AggFunc::Sum,
                "COUNT" => AggFunc::Count,
                "MIN" => AggFunc::Min,
                "MAX" => AggFunc::Max,
                "AVG" => AggFunc::Avg,
                other => {
                    return Err(self.perr(format!(
                        "unsupported function '{other}' in SELECT; supported: {SUPPORTED_AGGS}"
                    )));
                }
            };
            ProjectionItem::Aggregate {
                func,
                column,
                alias: None,
            }
        } else {
            ProjectionItem::Column { name, alias: None }
        };
        if self.accept(Tok::As) {
            match self.peek().clone() {
                Tok::Ident(a) => {
                    self.next();
                    match &mut item {
                        ProjectionItem::Column { alias, .. } => *alias = Some(a),
                        ProjectionItem::Aggregate { alias, .. } => *alias = Some(a),
                        ProjectionItem::Wild => {}
                    }
                }
                other => {
                    return Err(self.perr(format!(
                        "expected an alias name after AS but found '{}'",
                        other.describe()
                    )));
                }
            }
        }
        Ok(item)
    }

    fn parse_order_by_expr(&mut self) -> Result<String, QueryError> {
        let word = match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                s
            }
            other => {
                return Err(self.perr(format!(
                    "expected a column name in ORDER BY but found '{}'",
                    other.describe()
                )));
            }
        };
        if self.accept(Tok::LParen) {
            let inner = if self.accept(Tok::Star) {
                "*".to_string()
            } else {
                self.parse_column()?
            };
            self.expect(Tok::RParen)?;
            let func = word.to_ascii_uppercase();
            match func.as_str() {
                "SUM" | "COUNT" | "MIN" | "MAX" | "AVG" => Ok(format!("{func}({inner})")),
                _ => Err(self.perr(format!(
                    "unsupported aggregate function '{func}' in ORDER BY; supported: {SUPPORTED_AGGS}"
                ))),
            }
        } else {
            Ok(word)
        }
    }

    fn parse_literal(&mut self) -> Result<Scalar, QueryError> {
        let neg = self.accept(Tok::Minus);
        let tok = self.next();
        let value = match tok {
            Tok::Int(n) => Scalar::I64(if neg { -n } else { n }),
            Tok::Number(n) => Scalar::F64(if neg { -n } else { n }),
            Tok::StrLit(s) => {
                if neg {
                    return Err(self.perr("cannot negate a string literal"));
                }
                Scalar::Str(s)
            }
            Tok::True => {
                if neg {
                    return Err(self.perr("cannot negate TRUE"));
                }
                Scalar::Bool(true)
            }
            Tok::False => {
                if neg {
                    return Err(self.perr("cannot negate FALSE"));
                }
                Scalar::Bool(false)
            }
            Tok::Null => {
                if neg {
                    return Err(self.perr("cannot negate NULL"));
                }
                Scalar::Null
            }
            other => {
                return Err(self.perr(format!(
                    "expected a literal value but found '{}'",
                    other.describe()
                )));
            }
        };
        Ok(value)
    }

    fn parse_pred(&mut self) -> Result<Pred, QueryError> {
        let column = self.parse_column()?;
        let op = match self.next() {
            Tok::Eq => Comparison::Eq,
            Tok::NotEq => Comparison::NotEq,
            Tok::Gt => Comparison::Gt,
            Tok::Gte => Comparison::Gte,
            Tok::Lt => Comparison::Lt,
            Tok::Lte => Comparison::Lte,
            other => {
                return Err(self.perr(format!(
                    "expected a comparison operator but found '{}'",
                    other.describe()
                )));
            }
        };
        let value = self.parse_literal()?;
        Ok(Pred::Compare { column, op, value })
    }

    fn parse_select(&mut self) -> Result<Select, QueryError> {
        self.expect(Tok::Select)?;

        let mut projections = Vec::new();
        loop {
            projections.push(self.parse_projection_item()?);
            if !self.accept(Tok::Comma) {
                break;
            }
        }

        self.expect(Tok::From)?;
        let from = match self.peek().clone() {
            Tok::Ident(s) => {
                self.next();
                Some(s)
            }
            other => {
                return Err(self.perr(format!(
                    "expected a table name after FROM but found '{}'",
                    other.describe()
                )));
            }
        };

        let mut join = None;
        if self.accept(Tok::Join) {
            let right = match self.peek().clone() {
                Tok::Ident(s) => {
                    self.next();
                    s
                }
                other => {
                    return Err(self.perr(format!(
                        "expected a table name after JOIN but found '{}'",
                        other.describe()
                    )));
                }
            };
            self.expect(Tok::On)?;
            let left_col = self.parse_column()?;
            self.expect(Tok::Eq)?;
            let right_col = self.parse_column()?;
            join = Some(JoinClause {
                right,
                on: (left_col, right_col),
            });
        }

        let mut predicates = Vec::new();
        if self.accept(Tok::Where) {
            loop {
                predicates.push(self.parse_pred()?);
                if !self.accept(Tok::And) {
                    break;
                }
            }
        }

        let mut group_by = Vec::new();
        if self.accept(Tok::Group) {
            self.expect(Tok::By)?;
            loop {
                group_by.push(self.parse_column()?);
                if !self.accept(Tok::Comma) {
                    break;
                }
            }
        }

        let mut order_by = None;
        if self.accept(Tok::Order) {
            self.expect(Tok::By)?;
            let column = self.parse_order_by_expr()?;
            let desc = if self.accept(Tok::Desc) {
                true
            } else {
                self.accept(Tok::Asc);
                false
            };
            order_by = Some((column, desc));
        }

        let mut limit = None;
        if self.accept(Tok::Limit) {
            match self.peek().clone() {
                Tok::Int(n) => {
                    if n < 0 {
                        return Err(self.perr("LIMIT requires a non-negative integer"));
                    }
                    limit = Some(n as usize);
                    self.next();
                }
                other => {
                    return Err(self.perr(format!(
                        "expected an integer after LIMIT but found '{}'",
                        other.describe()
                    )));
                }
            }
        }

        self.accept(Tok::Semi);
        if *self.peek() != Tok::End {
            return Err(self.perr(format!(
                "unexpected trailing input '{}'",
                self.peek().describe()
            )));
        }

        Ok(Select {
            projections,
            from,
            join,
            predicates,
            group_by,
            order_by,
            limit,
        })
    }
}

fn parse_sql(sql: &str) -> Result<Select, QueryError> {
    if sql.trim().is_empty() {
        return Err(sql_err("empty query"));
    }
    let (toks, posns) = lex(sql)?;
    let mut parser = Parser::new(toks, posns, sql.to_string());
    parser.parse_select()
}

// === Binder / executor ===

/// Strip a table qualifier from a column reference (`a.b` -> `b`).
fn unqualified(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(_, col)| col)
}

fn resolve_in(schema: &Schema, column: &str) -> Result<usize, QueryError> {
    let name = unqualified(column);
    schema
        .index_of(name)
        .ok_or_else(|| QueryError::MissingColumn {
            column: name.to_string(),
            available: schema.fields.iter().map(|f| f.name.clone()).collect(),
        })
}

fn apply_sort(
    plan: Arc<dyn PhysicalPlan>,
    col: &str,
    desc: bool,
) -> Result<Arc<dyn PhysicalPlan>, QueryError> {
    let schema = plan.schema();
    match schema.index_of(col) {
        Some(i) => Ok(Arc::new(SortExec::new(
            plan,
            vec![SortExpr { column: i, desc }],
        ))),
        None => Err(QueryError::MissingColumn {
            column: col.to_string(),
            available: schema.fields.iter().map(|f| f.name.clone()).collect(),
        }),
    }
}

fn to_engine_func(func: AggFunc) -> EngAggFunc {
    match func {
        AggFunc::Sum => EngAggFunc::Sum,
        AggFunc::Count => EngAggFunc::Count,
        AggFunc::Min => EngAggFunc::Min,
        AggFunc::Max => EngAggFunc::Max,
        AggFunc::Avg => EngAggFunc::Avg,
    }
}

/// Coerce a SQL literal to the type of the column it is compared against.
fn coerce_literal(mut value: Scalar, field: &Field, column: &str) -> Result<Scalar, QueryError> {
    if !matches!(value, Scalar::Null) && value.type_name() != field.data_type.name() {
        // Allow integer literals against float and int columns.
        match (value, field.data_type) {
            (Scalar::I64(n), DataType::Int32) => {
                let n32 = i32::try_from(n).map_err(|_| QueryError::TypeMismatch {
                    column: column.to_string(),
                    expected: field.data_type.name(),
                    actual: "literal i64",
                    context: format!("integer literal {n} does not fit in an i32"),
                })?;
                value = Scalar::I32(n32);
            }
            (Scalar::I64(n), DataType::Float64) => value = Scalar::F64(n as f64),
            (Scalar::I64(n), DataType::Int64) => value = Scalar::I64(n),
            (Scalar::F64(n), DataType::Float64) => value = Scalar::F64(n),
            (Scalar::F64(n), DataType::Int32) => {
                if n.fract() != 0.0 || n < i32::MIN as f64 || n > i32::MAX as f64 {
                    return Err(QueryError::TypeMismatch {
                        column: column.to_string(),
                        expected: field.data_type.name(),
                        actual: "literal f64",
                        context: format!("literal {n} is not an integer in i32 range"),
                    });
                }
                value = Scalar::I32(n as i32);
            }
            (Scalar::F64(n), DataType::Int64) => {
                if n.fract() != 0.0 || n < i64::MIN as f64 || n > i64::MAX as f64 {
                    return Err(QueryError::TypeMismatch {
                        column: column.to_string(),
                        expected: field.data_type.name(),
                        actual: "literal f64",
                        context: format!("literal {n} is not an integer in i64 range"),
                    });
                }
                value = Scalar::I64(n as i64);
            }
            (Scalar::Bool(b), DataType::Boolean) => value = Scalar::Bool(b),
            (Scalar::Str(s), DataType::Utf8) => value = Scalar::Str(s),
            (v, _) => {
                return Err(QueryError::TypeMismatch {
                    column: column.to_string(),
                    expected: field.data_type.name(),
                    actual: v.type_name(),
                    context: format!(
                        "Cannot compare '{column}' ({}) with a literal of {} type",
                        field.data_type,
                        v.type_name()
                    ),
                });
            }
        }
    }
    Ok(value)
}

/// A named set of tables that SQL can query against.
///
/// ```rust
/// use tpt_strata::{Table, Column, Scalar, sql::SqlContext};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let sales = Table::try_new(vec![
///     Column::new("dept", vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())]),
///     Column::new("amount", vec![Scalar::F64(100.0), Scalar::F64(50.0)]),
/// ])?;
/// let mut ctx = SqlContext::new();
/// ctx.add_table("sales", &sales);
/// let out = ctx.run("SELECT dept, SUM(amount) FROM sales GROUP BY dept ORDER BY dept")?;
/// assert_eq!(out.row_count(), 2);
/// # Ok(())
/// # }
/// ```
pub struct SqlContext<'a> {
    tables: Vec<(&'a str, &'a Table)>,
}

impl<'a> SqlContext<'a> {
    pub fn new() -> Self {
        Self { tables: Vec::new() }
    }

    /// Register a table under a name usable in `FROM`/`JOIN`.
    pub fn add_table(&mut self, name: &'a str, table: &'a Table) -> &mut Self {
        if let Some(entry) = self.tables.iter_mut().find(|(n, _)| *n == name) {
            entry.1 = table;
        } else {
            self.tables.push((name, table));
        }
        self
    }

    fn lookup(&self, name: &str) -> Result<&'a Table, QueryError> {
        self.tables
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| *t)
            .ok_or_else(|| {
                let available: Vec<String> =
                    self.tables.iter().map(|(n, _)| n.to_string()).collect();
                QueryError::UnsupportedOperation {
                    message: format!(
                        "no table named '{name}' in the query context; available tables: {}",
                        available.join(", ")
                    ),
                }
            })
    }

    /// Parse and execute `sql`, returning the result table.
    pub fn run(&self, sql: &str) -> Result<Table, QueryError> {
        let sel = parse_sql(sql)?;

        let from = sel
            .from
            .clone()
            .ok_or_else(|| sql_err("expected a FROM clause"))?;
        let left = self.lookup(&from)?;

        let mut plan: Arc<dyn PhysicalPlan> = match &sel.join {
            Some(join) => {
                let right = self.lookup(&join.right)?;
                let left_idx = left.column_index(unqualified(&join.on.0))?;
                let right_idx = right.column_index(unqualified(&join.on.1))?;
                let lp: Arc<dyn PhysicalPlan> = Arc::new(DataSource::new(
                    Arc::new(left.schema().clone()),
                    left.batches().to_vec(),
                ));
                let rp: Arc<dyn PhysicalPlan> = Arc::new(DataSource::new(
                    Arc::new(right.schema().clone()),
                    right.batches().to_vec(),
                ));
                Arc::new(JoinExec::new(lp, rp, left_idx, right_idx))
            }
            None => Arc::new(DataSource::new(
                Arc::new(left.schema().clone()),
                left.batches().to_vec(),
            )),
        };

        for pred in &sel.predicates {
            let (idx, value) = match pred {
                Pred::Compare {
                    column,
                    op: _,
                    value,
                } => {
                    let schema = plan.schema();
                    let idx = resolve_in(schema, column)?;
                    let field = schema
                        .field(idx)
                        .ok_or_else(|| sql_err("internal error: resolved column missing"))?;
                    let value = coerce_literal(value.clone(), field, column)?;
                    (idx, value)
                }
            };
            let op = match pred {
                Pred::Compare { op, .. } => *op,
            };
            plan = Arc::new(FilterExec::new(
                plan,
                Predicate {
                    column: idx,
                    op,
                    value,
                },
            ));
        }

        let has_agg = sel
            .projections
            .iter()
            .any(|p| matches!(p, ProjectionItem::Aggregate { .. }));
        let has_group = !sel.group_by.is_empty();

        if has_agg || has_group {
            let group_indices = {
                let schema = plan.schema();
                let mut idxs = Vec::with_capacity(sel.group_by.len());
                for g in &sel.group_by {
                    idxs.push(resolve_in(schema, g)?);
                }
                idxs
            };

            let mut aggregates: Vec<Aggregate> = Vec::new();
            for p in &sel.projections {
                if let ProjectionItem::Aggregate {
                    func,
                    column,
                    alias: _,
                } = p
                {
                    let (col_idx, out_name) = match column {
                        None => (0usize, format!("{func}(*)")),
                        Some(c) => {
                            let idx = resolve_in(plan.schema(), c)?;
                            (idx, format!("{func}({c})"))
                        }
                    };
                    aggregates.push(Aggregate {
                        func: to_engine_func(*func),
                        column: col_idx,
                        out_name,
                        count_star: matches!(*func, AggFunc::Count) && column.is_none(),
                    });
                }
            }

            for p in &sel.projections {
                if let ProjectionItem::Column { name, .. } = p {
                    let base = unqualified(name).to_string();
                    let grouped = sel.group_by.iter().any(|g| unqualified(g) == base);
                    if !grouped {
                        return Err(QueryError::UnsupportedOperation {
                            message: format!(
                                "column '{base}' must appear in the GROUP BY clause or be used in an aggregate function"
                            ),
                        });
                    }
                }
            }

            plan = Arc::new(AggregateExec::new(plan, group_indices, aggregates)?);

            if let Some((col, desc)) = &sel.order_by {
                plan = apply_sort(plan, col, *desc)?;
            }

            if let Some((exprs, names)) = build_final_projection(&sel.projections, plan.schema())? {
                plan = Arc::new(ProjectExec::new(plan, exprs, names)?);
            }
        } else {
            let proj_schema = plan.schema().clone();
            let wild = sel
                .projections
                .iter()
                .any(|p| matches!(p, ProjectionItem::Wild));

            // ORDER BY may reference source columns that are not projected,
            // so it is applied before the projection step.
            if let Some((col, desc)) = &sel.order_by {
                plan = apply_sort(plan, col, *desc)?;
            }

            if !wild {
                let mut exprs = Vec::new();
                let mut names = Vec::new();
                for p in &sel.projections {
                    if let ProjectionItem::Column { name, .. } = p {
                        let idx = resolve_in(&proj_schema, name)?;
                        exprs.push(Expr { column: idx });
                        names.push(unqualified(name).to_string());
                    }
                }
                if !exprs.is_empty() {
                    plan = Arc::new(ProjectExec::new(plan, exprs, names)?);
                }
            }
        }

        if let Some(n) = sel.limit {
            plan = Arc::new(LimitExec::new(plan, n));
        }

        let batches: Vec<Batch> = plan.execute()?;
        Ok(Table::from_batches(
            Arc::new(plan.schema().clone()),
            batches,
        ))
    }
}

impl<'a> Default for SqlContext<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// A reordering/renaming projection over an aggregate result, if needed.
type FinalProjection = Option<(Vec<Expr>, Vec<String>)>;

/// Produce a projection that reorders/renames the aggregate output to match
/// the SELECT list. Returns `None` when the output is already in order.
fn build_final_projection(
    projections: &[ProjectionItem],
    schema: &Schema,
) -> Result<FinalProjection, QueryError> {
    let mut exprs = Vec::new();
    let mut names = Vec::new();
    for p in projections {
        match p {
            ProjectionItem::Column { name, .. } => {
                let base = unqualified(name).to_string();
                let idx = schema
                    .index_of(&base)
                    .ok_or_else(|| QueryError::MissingColumn {
                        column: base.clone(),
                        available: schema.fields.iter().map(|f| f.name.clone()).collect(),
                    })?;
                exprs.push(Expr { column: idx });
                names.push(base);
            }
            ProjectionItem::Aggregate { func, column, .. } => {
                let out_name = match column {
                    None => format!("{func}(*)"),
                    Some(c) => format!("{func}({c})"),
                };
                let idx = schema
                    .index_of(&out_name)
                    .ok_or_else(|| QueryError::MissingColumn {
                        column: out_name.clone(),
                        available: schema.fields.iter().map(|f| f.name.clone()).collect(),
                    })?;
                exprs.push(Expr { column: idx });
                names.push(out_name);
            }
            ProjectionItem::Wild => {
                for field in &schema.fields {
                    let name = field.name.clone();
                    let idx = schema.index_of(&name).expect("schema field exists");
                    if !names.contains(&name) {
                        exprs.push(Expr { column: idx });
                        names.push(name);
                    }
                }
            }
        }
    }
    if exprs.is_empty() {
        return Ok(None);
    }
    let aligned = exprs.iter().enumerate().all(|(i, e)| e.column == i);
    if aligned {
        return Ok(None);
    }
    Ok(Some((exprs, names)))
}
