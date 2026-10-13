//! A minimal, hand-written SQL surface for the v1 subset.
//!
//! Keeping the parser in this crate preserves the zero-external-dependency
//! promise for the core engine. Supported grammar:
//!
//! ```text
//! SELECT [DISTINCT] <column | agg(col) | agg(*) | *> [AS alias] [, ...]
//!   FROM <table> [[LEFT | RIGHT | FULL] [OUTER] JOIN <table> ON left.col = right.col]...
//!   [WHERE cond]
//!   [GROUP BY col [, ...]]
//!   [HAVING cond]
//!   [ORDER BY col-or-alias [ASC | DESC] [, ...]]
//!   [LIMIT n] [OFFSET n] [;]
//!
//! cond := and-cond [OR and-cond ...]
//! and-cond := primary [AND primary ...]
//! primary := NOT primary | ( cond ) | comparison
//! comparison := operand op literal | operand op operand
//!             | operand [NOT] IN (literal [, ...]) | operand [NOT] BETWEEN literal AND literal
//!             | operand IS [NOT] NULL | operand [NOT] LIKE 'pattern'
//! ```
//!
//! `AND` binds tighter than `OR`. Parentheses override that. `NOT` is pushed
//! down to the comparisons, so `NOT (a AND b)` becomes `NOT a OR NOT b`. A
//! comparison involving NULL is unknown and its negation is unknown too, so
//! unknown rows are filtered out either way, as in SQL.
//!
//! Anything outside this subset (e.g. `CAST`, arithmetic, subqueries) is
//! rejected with a targeted diagnostic pointing at `docs/v1.1-scope.md`. The
//! omissions are decisions, not accidents.
//!
//! Supported aggregate functions: `SUM`, `COUNT`, `MIN`, `MAX`, `AVG`,
//! including `COUNT(*)`. Comparison ops: `=`, `!=`, `<>`, `>`, `>=`, `<`,
//! `<=`. Literals may be integers, floats, single-quoted strings, `TRUE`,
//! `FALSE`, and `NULL`.

use std::sync::Arc;

use crate::engine::{
    AggFunc as EngAggFunc, Aggregate, AggregateExec, Comparison, CondFilterExec, Condition,
    DataSource, Expr, FilterExec, JoinExec, JoinKind, LimitExec, PhysicalPlan, Predicate,
    ProjectExec, SortExec, SortExpr,
};
use crate::error::QueryError;
use crate::format::Table;
use crate::query::{coerce_literal, AggFunc};
use crate::schema::{DataType, Field, Schema};
use crate::types::Scalar;

const SUPPORTED_AGGS: &str = "SUM, COUNT, MIN, MAX, AVG";

// === AST ===

/// Something a predicate, sort key, or projection refers to.
#[derive(Debug, Clone, PartialEq)]
enum Operand {
    /// A column reference as written: `col`, `table.col`, or a quoted name.
    Column(String),
    /// An aggregate. `column: None` means `*`, which only `COUNT` accepts.
    Aggregate {
        func: AggFunc,
        column: Option<String>,
    },
}

impl Operand {
    /// The operand as the user would write it, used in diagnostics.
    fn label(&self) -> String {
        match self {
            Operand::Column(c) => c.clone(),
            Operand::Aggregate { func, column } => {
                format!("{func}({})", column.as_deref().unwrap_or("*"))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ProjectionItem {
    Wild,
    Expr {
        operand: Operand,
        alias: Option<String>,
    },
}

/// A single predicate. `op` is one of the engine's comparisons; `value` is
/// `Null` for `IS [NOT] NULL`.
#[derive(Debug, Clone, PartialEq)]
struct Pred {
    target: Operand,
    op: Comparison,
    value: Scalar,
}

/// A boolean condition: a comparison with a literal, a comparison between two
/// operands, or an AND or OR of conditions.
#[derive(Debug, Clone, PartialEq)]
enum Cond {
    Leaf(Pred),
    ColCmp {
        left: Operand,
        op: Comparison,
        right: Operand,
    },
    And(Vec<Cond>),
    Or(Vec<Cond>),
}

/// Every operand a condition refers to, in source order.
fn cond_operands(c: &Cond, out: &mut Vec<Operand>) {
    match c {
        Cond::Leaf(p) => out.push(p.target.clone()),
        Cond::ColCmp { left, right, .. } => {
            out.push(left.clone());
            out.push(right.clone());
        }
        Cond::And(parts) | Cond::Or(parts) => {
            for p in parts {
                cond_operands(p, out);
            }
        }
    }
}

/// Negate a condition by pushing `NOT` down to the comparisons. Every
/// comparison has an exact opposite that is also NULL when an input is NULL,
/// so filtering with NULL-as-false gives the same rows as SQL three-valued
/// logic. AND and OR swap by De Morgan's laws.
fn negate_cond(c: Cond) -> Cond {
    match c {
        Cond::Leaf(mut p) => {
            p.op = negate_comparison(p.op);
            Cond::Leaf(p)
        }
        Cond::ColCmp { left, op, right } => Cond::ColCmp {
            left,
            op: negate_comparison(op),
            right,
        },
        Cond::And(parts) => Cond::Or(parts.into_iter().map(negate_cond).collect()),
        Cond::Or(parts) => Cond::And(parts.into_iter().map(negate_cond).collect()),
    }
}

fn negate_comparison(op: Comparison) -> Comparison {
    match op {
        Comparison::Eq => Comparison::NotEq,
        Comparison::NotEq => Comparison::Eq,
        Comparison::Gt => Comparison::Lte,
        Comparison::Gte => Comparison::Lt,
        Comparison::Lt => Comparison::Gte,
        Comparison::Lte => Comparison::Gt,
        Comparison::IsNull => Comparison::IsNotNull,
        Comparison::IsNotNull => Comparison::IsNull,
        Comparison::Like => Comparison::NotLike,
        Comparison::NotLike => Comparison::Like,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct JoinClause {
    right: String,
    on: (String, String),
    /// Which unmatched rows are kept (`LEFT`, `RIGHT`, `FULL`, or inner).
    kind: JoinKind,
}

#[derive(Debug, Clone, PartialEq)]
struct Select {
    distinct: bool,
    projections: Vec<ProjectionItem>,
    from: String,
    joins: Vec<JoinClause>,
    filter: Option<Cond>,
    group_by: Vec<String>,
    having: Option<Cond>,
    order_by: Vec<(Operand, bool)>,
    limit: Option<usize>,
    offset: Option<usize>,
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
    Offset,
    Join,
    On,
    And,
    As,
    // Keywords lexed so the deliberate v1.1 scope gaps get targeted
    // diagnostics instead of generic parse errors (see docs/v1.1-scope.md).
    Or,
    In,
    Between,
    Is,
    Not,
    Like,
    Distinct,
    Having,
    Left,
    Outer,
    Ident(String),
    Number(f64),
    Int(i64),
    /// The magnitude 2^63, which only `-9223372036854775808` can use.
    MinMagnitude,
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
            Tok::Offset => "OFFSET".into(),
            Tok::Join => "JOIN".into(),
            Tok::On => "ON".into(),
            Tok::And => "AND".into(),
            Tok::As => "AS".into(),
            Tok::Or => "OR".into(),
            Tok::In => "IN".into(),
            Tok::Between => "BETWEEN".into(),
            Tok::Is => "IS".into(),
            Tok::Not => "NOT".into(),
            Tok::Like => "LIKE".into(),
            Tok::Distinct => "DISTINCT".into(),
            Tok::Having => "HAVING".into(),
            Tok::Left => "LEFT".into(),
            Tok::Outer => "OUTER".into(),
            Tok::Ident(s) => s.clone(),
            Tok::Number(n) => format!("{n}"),
            Tok::Int(n) => format!("{n}"),
            Tok::MinMagnitude => "9223372036854775808".into(),
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
            '-' if chars.get(i + 1) == Some(&'-') => {
                // `-- comment` runs to the end of the line.
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '-' => {
                push(i, Tok::Minus);
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                // `/* comment */` may span lines and does not nest.
                let start = i;
                i += 2;
                loop {
                    match chars.get(i) {
                        None => return Err(sql_err_at(input, start, "unterminated block comment")),
                        Some('*') if chars.get(i + 1) == Some(&'/') => {
                            i += 2;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
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
                            // A doubled quote inside an identifier is one quote.
                            if chars.get(i + 1) == Some(&'"') {
                                s.push('"');
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
                // Exponent: `1e5`, `2.5E-3`. Only taken when digits follow.
                if matches!(chars.get(i), Some('e' | 'E')) {
                    let sign = usize::from(matches!(chars.get(i + 1), Some('+' | '-')));
                    if chars.get(i + 1 + sign).is_some_and(|d| d.is_ascii_digit()) {
                        is_float = true;
                        i += 1 + sign;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text: String = chars[start..i].iter().collect();
                if is_float {
                    let n: f64 = text.parse().map_err(|_| {
                        sql_err_at(input, start, format!("invalid number '{text}'"))
                    })?;
                    push(start, Tok::Number(n));
                } else if text == "9223372036854775808" {
                    // Only valid right after a minus sign (see parse_literal).
                    push(start, Tok::MinMagnitude);
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
                        "OFFSET" => Tok::Offset,
                        "JOIN" => Tok::Join,
                        "ON" => Tok::On,
                        "AND" => Tok::And,
                        "AS" => Tok::As,
                        "OR" => Tok::Or,
                        "IN" => Tok::In,
                        "BETWEEN" => Tok::Between,
                        "IS" => Tok::Is,
                        "NOT" => Tok::Not,
                        "LIKE" => Tok::Like,
                        "DISTINCT" => Tok::Distinct,
                        "HAVING" => Tok::Having,
                        "LEFT" => Tok::Left,
                        "OUTER" => Tok::Outer,
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

    /// The token `k` positions ahead of the cursor, or `End` past the end.
    fn peek_at(&self, k: usize) -> &Tok {
        self.toks.get(self.pos + k).unwrap_or(&Tok::End)
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

    /// Reject a v1.1-scope SQL construct with a clear, deliberate diagnostic.
    ///
    /// These are *decided* omissions (see `docs/v1.1-scope.md`), spelled out
    /// instead of a generic parse error, so callers know the gap is
    /// intentional and how to express the query in the supported subset.
    fn unsupported_v1_1(&self, what: &str, hint: &str) -> QueryError {
        self.perr(format!(
            "'{what}' is not supported in tpt-strata v1.1 (deliberate scope decision); {hint}. See docs/v1.1-scope.md"
        ))
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

    /// Parse a column or an aggregate call. `context` names the clause for
    /// the unsupported-function diagnostic.
    fn parse_operand(&mut self, context: &str) -> Result<Operand, QueryError> {
        let is_call = matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::LParen;
        if !is_call {
            return Ok(Operand::Column(self.parse_column()?));
        }
        let name = self.parse_column()?;
        self.expect(Tok::LParen)?;
        if name.eq_ignore_ascii_case("CAST") {
            return Err(self.perr(
                "unsupported cast: tpt-strata v1 has no CAST support; provide values in the target type directly",
            ));
        }
        if *self.peek() == Tok::Distinct {
            return Err(self.unsupported_v1_1(
                "COUNT(DISTINCT ...)",
                "count with COUNT(col) or use GROUP BY to list the distinct values",
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
                    "unsupported function '{other}' in {context}; supported: {SUPPORTED_AGGS}"
                )));
            }
        };
        if column.is_none() && func != AggFunc::Count {
            return Err(self.perr(format!("'*' is only valid in COUNT(*), not {func}(*)")));
        }
        Ok(Operand::Aggregate { func, column })
    }

    fn parse_projection_item(&mut self) -> Result<ProjectionItem, QueryError> {
        if self.accept(Tok::Star) {
            return Ok(ProjectionItem::Wild);
        }
        let operand = self.parse_operand("SELECT")?;
        let alias = if self.accept(Tok::As) {
            match self.peek().clone() {
                Tok::Ident(a) => {
                    self.next();
                    Some(a)
                }
                other => {
                    return Err(self.perr(format!(
                        "expected an alias name after AS but found '{}'",
                        other.describe()
                    )));
                }
            }
        } else {
            None
        };
        Ok(ProjectionItem::Expr { operand, alias })
    }

    /// One ORDER BY key: a column, alias, or aggregate.
    fn parse_order_key(&mut self) -> Result<Operand, QueryError> {
        if !matches!(self.peek(), Tok::Ident(_)) {
            return Err(self.perr(format!(
                "expected a column name in ORDER BY but found '{}'",
                self.peek().describe()
            )));
        }
        self.parse_operand("ORDER BY")
    }

    fn parse_literal(&mut self) -> Result<Scalar, QueryError> {
        let neg = self.accept(Tok::Minus);
        let tok = self.next();
        let value = match tok {
            Tok::Int(n) => Scalar::I64(if neg { -n } else { n }),
            Tok::MinMagnitude if neg => Scalar::I64(i64::MIN),
            Tok::MinMagnitude => {
                return Err(self.perr("integer out of range '9223372036854775808'"));
            }
            Tok::Number(n) => Scalar::F64(if neg { -n } else { n }),
            Tok::Ident(word)
                if word.eq_ignore_ascii_case("DATE") || word.eq_ignore_ascii_case("TIMESTAMP") =>
            {
                if neg {
                    return Err(self.perr("cannot negate a date or timestamp"));
                }
                let is_date = word.eq_ignore_ascii_case("DATE");
                let text = match self.next() {
                    Tok::StrLit(s) => s,
                    other => {
                        return Err(self.perr(format!(
                            "expected a quoted {} value after {word}, found '{}'",
                            if is_date { "date" } else { "timestamp" },
                            other.describe()
                        )));
                    }
                };
                if is_date {
                    match crate::temporal::parse_date(&text) {
                        Some(d) => Scalar::Date(d),
                        None => {
                            return Err(self.perr(format!("invalid date '{text}'; use YYYY-MM-DD")))
                        }
                    }
                } else {
                    match crate::temporal::parse_timestamp(&text) {
                        Some(t) => Scalar::Timestamp(t),
                        None => {
                            return Err(self.perr(format!(
                                "invalid timestamp '{text}'; use YYYY-MM-DD HH:MM[:SS[.ffffff]]"
                            )))
                        }
                    }
                }
            }
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

    /// Parse a condition for `clause` (`WHERE` or `HAVING`).
    ///
    /// Precedence, lowest first: `OR`, then `AND`, then a parenthesised
    /// condition or one comparison. `NOT` is still outside the subset.
    fn parse_cond(&mut self, clause: &str) -> Result<Cond, QueryError> {
        let mut parts = vec![self.parse_and(clause)?];
        while self.accept(Tok::Or) {
            parts.push(self.parse_and(clause)?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Cond::Or(parts)
        })
    }

    fn parse_and(&mut self, clause: &str) -> Result<Cond, QueryError> {
        let mut parts = vec![self.parse_primary(clause)?];
        while self.accept(Tok::And) {
            parts.push(self.parse_primary(clause)?);
        }
        Ok(if parts.len() == 1 {
            parts.remove(0)
        } else {
            Cond::And(parts)
        })
    }

    fn parse_primary(&mut self, clause: &str) -> Result<Cond, QueryError> {
        if self.accept(Tok::Not) {
            return Ok(negate_cond(self.parse_primary(clause)?));
        }
        if self.accept(Tok::LParen) {
            let inner = self.parse_cond(clause)?;
            self.expect(Tok::RParen)?;
            return Ok(inner);
        }
        self.parse_leaf(clause)
    }

    /// Parse one comparison, `IN (...)`, `BETWEEN`, `IS [NOT] NULL`, or
    /// `[NOT] LIKE`. `BETWEEN` and `IN` expand into AND and OR of comparisons.
    fn parse_leaf(&mut self, clause: &str) -> Result<Cond, QueryError> {
        let target = self.parse_operand(clause)?;
        if matches!(target, Operand::Aggregate { .. }) && clause == "WHERE" {
            return Err(self.perr(
                "aggregate functions are not allowed in WHERE; use HAVING to filter on an aggregate",
            ));
        }
        // `col NOT IN (...)` and `col NOT BETWEEN a AND b` negate the
        // positive form, the same way a leading NOT does.
        let negated =
            matches!(self.peek(), Tok::Not) && matches!(self.peek_at(1), Tok::In | Tok::Between);
        if negated {
            self.next();
        }
        match self.peek().clone() {
            Tok::In => {
                self.next();
                self.expect(Tok::LParen)?;
                let mut alternatives = Vec::new();
                loop {
                    let value = self.parse_literal()?;
                    alternatives.push(Cond::Leaf(Pred {
                        target: target.clone(),
                        op: Comparison::Eq,
                        value,
                    }));
                    if !self.accept(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen)?;
                let positive = if alternatives.len() == 1 {
                    alternatives.remove(0)
                } else {
                    Cond::Or(alternatives)
                };
                return Ok(if negated {
                    negate_cond(positive)
                } else {
                    positive
                });
            }
            Tok::Between => {
                self.next();
                let low = self.parse_literal()?;
                self.expect(Tok::And)?;
                let high = self.parse_literal()?;
                let positive = Cond::And(vec![
                    Cond::Leaf(Pred {
                        target: target.clone(),
                        op: Comparison::Gte,
                        value: low,
                    }),
                    Cond::Leaf(Pred {
                        target,
                        op: Comparison::Lte,
                        value: high,
                    }),
                ]);
                return Ok(if negated {
                    negate_cond(positive)
                } else {
                    positive
                });
            }
            Tok::Is => {
                self.next();
                let negated = self.accept(Tok::Not);
                self.expect(Tok::Null)?;
                let op = if negated {
                    Comparison::IsNotNull
                } else {
                    Comparison::IsNull
                };
                return Ok(Cond::Leaf(Pred {
                    target,
                    op,
                    value: Scalar::Null,
                }));
            }
            Tok::Like => {
                self.next();
                let value = self.parse_like_pattern()?;
                return Ok(Cond::Leaf(Pred {
                    target,
                    op: Comparison::Like,
                    value,
                }));
            }
            Tok::Not if *self.peek_at(1) == Tok::Like => {
                self.next();
                self.next();
                let value = self.parse_like_pattern()?;
                return Ok(Cond::Leaf(Pred {
                    target,
                    op: Comparison::NotLike,
                    value,
                }));
            }
            _ => {}
        }
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
        // A column on the right is a column-to-column comparison.
        // `DATE '...'` and `TIMESTAMP '...'` are literals, not column names.
        let typed_literal = matches!(
            self.peek(),
            Tok::Ident(w) if w.eq_ignore_ascii_case("DATE") || w.eq_ignore_ascii_case("TIMESTAMP")
        ) && matches!(self.peek_at(1), Tok::StrLit(_));
        if matches!(self.peek(), Tok::Ident(_)) && !typed_literal {
            let right = self.parse_operand(clause)?;
            if matches!(right, Operand::Aggregate { .. }) && clause == "WHERE" {
                return Err(self.perr(
                    "aggregate functions are not allowed in WHERE; use HAVING to filter on an aggregate",
                ));
            }
            return Ok(Cond::ColCmp {
                left: target,
                op,
                right,
            });
        }
        let value = self.parse_literal()?;
        Ok(Cond::Leaf(Pred { target, op, value }))
    }

    fn parse_like_pattern(&mut self) -> Result<Scalar, QueryError> {
        match self.peek().clone() {
            Tok::StrLit(s) => {
                self.next();
                Ok(Scalar::Str(s))
            }
            other => Err(self.perr(format!(
                "LIKE needs a string pattern but found '{}'",
                other.describe()
            ))),
        }
    }

    fn parse_select(&mut self) -> Result<Select, QueryError> {
        self.expect(Tok::Select)?;
        let distinct = self.accept(Tok::Distinct);

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
                s
            }
            other => {
                return Err(self.perr(format!(
                    "expected a table name after FROM but found '{}'",
                    other.describe()
                )));
            }
        };

        // `[LEFT | RIGHT | FULL [OUTER]] JOIN t ON a.x = b.y`, repeated. Only
        // equi-joins. RIGHT and FULL are not reserved words, so they are
        // matched as identifiers in this position only.
        let mut joins = Vec::new();
        loop {
            let kind = if matches!(self.peek(), Tok::Left) {
                self.next();
                self.accept(Tok::Outer);
                self.expect(Tok::Join)?;
                JoinKind::Left
            } else if self.accept(Tok::Join) {
                JoinKind::Inner
            } else if let Tok::Ident(word) = self.peek().clone() {
                let kind = if word.eq_ignore_ascii_case("RIGHT") {
                    JoinKind::Right
                } else if word.eq_ignore_ascii_case("FULL") {
                    JoinKind::Full
                } else {
                    break;
                };
                self.next();
                self.accept(Tok::Outer);
                self.expect(Tok::Join)?;
                kind
            } else {
                break;
            };
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
            joins.push(JoinClause {
                right,
                on: (left_col, right_col),
                kind,
            });
        }

        let mut filter = None;
        if self.accept(Tok::Where) {
            filter = Some(self.parse_cond("WHERE")?);
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

        let mut having = None;
        if self.accept(Tok::Having) {
            having = Some(self.parse_cond("HAVING")?);
        }

        let mut order_by = Vec::new();
        if self.accept(Tok::Order) {
            self.expect(Tok::By)?;
            loop {
                let key = self.parse_order_key()?;
                let desc = if self.accept(Tok::Desc) {
                    true
                } else {
                    self.accept(Tok::Asc);
                    false
                };
                order_by.push((key, desc));
                if !self.accept(Tok::Comma) {
                    break;
                }
            }
        }

        let limit = self.parse_count_clause(Tok::Limit, "LIMIT")?;
        let offset = self.parse_count_clause(Tok::Offset, "OFFSET")?;

        self.accept(Tok::Semi);
        if *self.peek() != Tok::End {
            return Err(self.perr(format!(
                "unexpected trailing input '{}'",
                self.peek().describe()
            )));
        }

        Ok(Select {
            distinct,
            projections,
            from,
            joins,
            filter,
            group_by,
            having,
            order_by,
            limit,
            offset,
        })
    }

    /// Parse an optional `LIMIT n` or `OFFSET n` clause.
    fn parse_count_clause(&mut self, kw: Tok, name: &str) -> Result<Option<usize>, QueryError> {
        if !self.accept(kw) {
            return Ok(None);
        }
        match self.peek().clone() {
            Tok::Int(n) => {
                if n < 0 {
                    return Err(self.perr(format!("{name} requires a non-negative integer")));
                }
                self.next();
                Ok(Some(n as usize))
            }
            other => Err(self.perr(format!(
                "expected an integer after {name} but found '{}'",
                other.describe()
            ))),
        }
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

// === Binder / planner ===

/// The tables a query reads from, and whether its input is a join.
///
/// Joined schemas name fields `table.column`, so references are resolved
/// against qualified names. A single-table schema uses bare names.
struct Scope {
    tables: Vec<String>,
    joined: bool,
}

/// If `reference` starts with `table.` for one of `tables`, return that
/// table's index and the bare remainder. Splitting on a known table name
/// (rather than on the last dot) keeps dotted column names intact.
fn split_table<'r>(reference: &'r str, tables: &[String]) -> Option<(usize, &'r str)> {
    tables.iter().enumerate().find_map(|(i, t)| {
        reference
            .strip_prefix(t.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
            .map(|bare| (i, bare))
    })
}

fn names(schema: &Schema) -> Vec<String> {
    schema.fields.iter().map(|f| f.name.clone()).collect()
}

/// Resolve a column reference to an index in `schema`.
///
/// Resolution order: an exact field name first (this covers aggregate names
/// such as `SUM(x)`, quoted names containing dots, and qualified join
/// fields). Then, for a single table, the bare name after a table qualifier.
/// Then, for a join, an unqualified name that matches exactly one qualified
/// field. A name matching two or more fields is an ambiguity error.
fn resolve(schema: &Schema, scope: &Scope, reference: &str) -> Result<usize, QueryError> {
    if let Some(i) = schema.index_of(reference) {
        return Ok(i);
    }
    let missing = || QueryError::MissingColumn {
        column: reference.to_string(),
        available: names(schema),
    };
    if let Some((_, bare)) = split_table(reference, &scope.tables) {
        // Joined fields carry their qualifier, so an unmatched qualified name
        // is genuinely missing. Single-table fields are bare.
        return if scope.joined {
            Err(missing())
        } else {
            schema.index_of(bare).ok_or_else(missing)
        };
    }
    if scope.joined {
        let suffix = format!(".{reference}");
        let hits: Vec<usize> = schema
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.name.ends_with(&suffix))
            .map(|(i, _)| i)
            .collect();
        return match hits.as_slice() {
            [i] => Ok(*i),
            [] => Err(missing()),
            many => {
                let options: Vec<&str> = many
                    .iter()
                    .map(|&i| schema.fields[i].name.as_str())
                    .collect();
                Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "column '{reference}' is ambiguous; qualify it as one of: {}",
                        options.join(", ")
                    ),
                })
            }
        };
    }
    Err(missing())
}

/// Locate the two ON-clause columns of a join: one in the tables joined so far
/// (the accumulated plan, whose fields are qualified `table.column`), and one
/// in the table being joined, in either order.
fn join_keys(
    acc: &Schema,
    acc_tables: &[String],
    right: &Table,
    rname: &str,
    a: &str,
    b: &str,
) -> Result<(usize, usize), QueryError> {
    let mut all: Vec<String> = acc_tables.to_vec();
    all.push(rname.to_string());
    let missing = |reference: &str| {
        let mut available = names(acc);
        available.extend(names(right.schema()));
        QueryError::MissingColumn {
            column: reference.to_string(),
            available,
        }
    };
    // Returns (is_accumulated, index in that side's schema).
    let locate = |reference: &str| -> Result<(bool, usize), QueryError> {
        if let Some((which, bare)) = split_table(reference, &all) {
            if which == all.len() - 1 {
                return Ok((false, right.column_index(bare)?));
            }
            let qualified = format!("{}.{}", all[which], bare);
            return acc
                .index_of(&qualified)
                .map(|i| (true, i))
                .ok_or_else(|| missing(reference));
        }
        let suffix = format!(".{reference}");
        let hits: Vec<usize> = acc
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.name.ends_with(&suffix))
            .map(|(i, _)| i)
            .collect();
        match hits.as_slice() {
            [i] => return Ok((true, *i)),
            [] => {}
            many => {
                let options: Vec<&str> =
                    many.iter().map(|&i| acc.fields[i].name.as_str()).collect();
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "column '{reference}' is ambiguous; qualify it as one of: {}",
                        options.join(", ")
                    ),
                });
            }
        }
        match right.schema().index_of(reference) {
            Some(i) => Ok((false, i)),
            None => Err(missing(reference)),
        }
    };
    let (a_acc, ia) = locate(a)?;
    let (b_acc, ib) = locate(b)?;
    match (a_acc, b_acc) {
        (true, false) => Ok((ia, ib)),
        (false, true) => Ok((ib, ia)),
        _ => Err(QueryError::UnsupportedOperation {
            message: format!(
                "the ON clause must compare a column from the tables joined so far with one from '{rname}'"
            ),
        }),
    }
}

/// Every column reference in the query, or `None` when the query reads every
/// column (a `*` in the select list).
fn needed_refs(sel: &Select) -> Option<Vec<String>> {
    if sel
        .projections
        .iter()
        .any(|p| matches!(p, ProjectionItem::Wild))
    {
        return None;
    }
    fn push(out: &mut Vec<String>, op: &Operand) {
        match op {
            Operand::Column(c)
            | Operand::Aggregate {
                column: Some(c), ..
            } => out.push(c.clone()),
            Operand::Aggregate { column: None, .. } => {}
        }
    }
    let mut out = Vec::new();
    for p in &sel.projections {
        if let ProjectionItem::Expr { operand, .. } = p {
            push(&mut out, operand);
        }
    }
    let mut conds = Vec::new();
    for c in sel.filter.iter().chain(sel.having.iter()) {
        cond_operands(c, &mut conds);
    }
    for o in &conds {
        push(&mut out, o);
    }
    // An ORDER BY that names a SELECT alias is not a column reference.
    let aliases: Vec<&str> = sel
        .projections
        .iter()
        .filter_map(|p| match p {
            ProjectionItem::Expr { alias: Some(a), .. } => Some(a.as_str()),
            _ => None,
        })
        .collect();
    for (o, _) in &sel.order_by {
        match o {
            Operand::Column(c) if aliases.contains(&c.as_str()) => {}
            _ => push(&mut out, o),
        }
    }
    out.extend(sel.group_by.iter().cloned());
    for j in &sel.joins {
        out.push(j.on.0.clone());
        out.push(j.on.1.clone());
    }
    Some(out)
}

/// Whether every reference names a column of one of the query's tables.
fn refs_resolve(refs: &[String], inputs: &[(&str, &Table)]) -> bool {
    refs.iter().all(|r| {
        inputs.iter().any(|(name, table)| {
            let prefix = format!("{name}.");
            table
                .schema()
                .fields
                .iter()
                .any(|f| r == &f.name || r.strip_prefix(&prefix) == Some(f.name.as_str()))
        })
    })
}

/// Keep only the columns a query can read from table `name`. A column is
/// kept when a reference names it, bare or qualified with `name`. The column
/// count never drops to zero, so the row count survives `COUNT(*)`.
fn prune(table: &Table, name: &str, needed: Option<&[String]>) -> Table {
    let Some(needed) = needed else {
        return table.clone();
    };
    let prefix = format!("{name}.");
    let mut keep: Vec<usize> = table
        .schema()
        .fields
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            needed
                .iter()
                .any(|r| r == &f.name || r.strip_prefix(&prefix) == Some(f.name.as_str()))
        })
        .map(|(i, _)| i)
        .collect();
    if keep.is_empty() && !table.schema().is_empty() {
        keep.push(0);
    }
    if keep.len() == table.schema().len() {
        return table.clone();
    }
    table.select_columns(&keep)
}

/// A table as a plan source, with every column renamed to `name.column`.
fn qualified_source(table: &Table, name: &str) -> Arc<dyn PhysicalPlan> {
    let fields = table
        .schema()
        .fields
        .iter()
        .map(|f| Field::new(format!("{name}.{}", f.name), f.data_type, f.nullable))
        .collect();
    Arc::new(DataSource::from_shared(
        Arc::new(Schema::new(fields)),
        table.batches().to_vec(),
    ))
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

/// Turn a parsed predicate into an engine predicate against column `idx`.
fn bind_predicate(schema: &Schema, idx: usize, pred: &Pred) -> Result<Predicate, QueryError> {
    let field = schema
        .field(idx)
        .ok_or_else(|| QueryError::UnsupportedOperation {
            message: "internal error: resolved column missing".to_string(),
        })?;
    let value = match pred.op {
        Comparison::IsNull | Comparison::IsNotNull => Scalar::Null,
        Comparison::Like | Comparison::NotLike => {
            if field.data_type != DataType::Utf8 {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "LIKE needs a text column; '{}' has type {}",
                        pred.target.label(),
                        field.data_type
                    ),
                });
            }
            pred.value.clone()
        }
        _ => coerce_literal(pred.value.clone(), field, &pred.target.label())?,
    };
    Ok(Predicate {
        column: idx,
        op: pred.op,
        value,
    })
}

/// The canonical name, input column, and `COUNT(*)` flag of an aggregate.
/// The name is built from the resolved field name, so `SUM(t.x)` and
/// `SUM(x)` agree when they refer to the same column.
fn aggregate_name(
    input: &Schema,
    scope: &Scope,
    func: AggFunc,
    column: Option<&str>,
) -> Result<(String, usize, bool), QueryError> {
    match column {
        None => Ok((format!("{func}(*)"), 0, true)),
        Some(c) => {
            let idx = resolve(input, scope, c)?;
            Ok((format!("{func}({})", input.fields[idx].name), idx, false))
        }
    }
}

/// Register an aggregate once and return its output name.
fn add_aggregate(
    aggs: &mut Vec<Aggregate>,
    input: &Schema,
    scope: &Scope,
    func: AggFunc,
    column: Option<&str>,
) -> Result<String, QueryError> {
    let (out_name, idx, count_star) = aggregate_name(input, scope, func, column)?;
    if !aggs.iter().any(|a| a.out_name == out_name) {
        aggs.push(Aggregate {
            func: to_engine_func(func),
            column: idx,
            out_name: out_name.clone(),
            count_star,
        });
    }
    Ok(out_name)
}

/// The output names of a projection. Aliases win. Joined outputs drop their
/// table qualifier unless that makes two outputs collide.
fn output_names(items: &[(usize, String, Option<String>)], scope: &Scope) -> Vec<String> {
    let mut out: Vec<String> = items
        .iter()
        .map(|(_, field, alias)| match alias {
            Some(a) => a.clone(),
            None if scope.joined => strip_table(field, &scope.tables),
            None => field.clone(),
        })
        .collect();
    if scope.joined {
        for i in 0..items.len() {
            if items[i].2.is_some() {
                continue;
            }
            let current = out[i].clone();
            if out.iter().filter(|n| **n == current).count() > 1 {
                out[i] = items[i].1.clone();
            }
        }
    }
    out
}

fn strip_table(field: &str, tables: &[String]) -> String {
    match split_table(field, tables) {
        Some((_, bare)) => bare.to_string(),
        None => field.to_string(),
    }
}

/// Project `items` (schema index, field name, alias) off `plan`. Skips the
/// projection when it would be the identity.
fn project_items(
    plan: Arc<dyn PhysicalPlan>,
    items: &[(usize, String, Option<String>)],
    scope: &Scope,
) -> Result<Arc<dyn PhysicalPlan>, QueryError> {
    let out_names = output_names(items, scope);
    let identity = {
        let schema = plan.schema();
        items.len() == schema.len()
            && items
                .iter()
                .enumerate()
                .all(|(i, (idx, _, alias))| *idx == i && alias.is_none())
            && out_names
                .iter()
                .zip(&schema.fields)
                .all(|(n, f)| *n == f.name)
    };
    if identity {
        return Ok(plan);
    }
    let exprs = items
        .iter()
        .map(|(idx, _, _)| Expr { column: *idx })
        .collect();
    Ok(Arc::new(ProjectExec::new(plan, exprs, out_names)?))
}

fn has_aggregates(sel: &Select) -> bool {
    sel.projections.iter().any(|p| {
        matches!(
            p,
            ProjectionItem::Expr {
                operand: Operand::Aggregate { .. },
                ..
            }
        )
    }) || sel.having.iter().any(cond_has_aggregate)
        || sel
            .order_by
            .iter()
            .any(|(o, _)| matches!(o, Operand::Aggregate { .. }))
}

/// The projection item an alias names, if any.
fn alias_operand<'s>(sel: &'s Select, name: &str) -> Option<&'s Operand> {
    sel.projections.iter().find_map(|p| match p {
        ProjectionItem::Expr {
            operand,
            alias: Some(a),
        } if a == name => Some(operand),
        _ => None,
    })
}

/// Plan a query with no aggregation: sort on source columns, then project.
fn plan_plain(
    mut plan: Arc<dyn PhysicalPlan>,
    scope: &Scope,
    sel: &Select,
) -> Result<Arc<dyn PhysicalPlan>, QueryError> {
    // ORDER BY runs before projection, so keys need not be in the SELECT list.
    if !sel.order_by.is_empty() {
        let mut keys = Vec::with_capacity(sel.order_by.len());
        for (operand, desc) in &sel.order_by {
            let Operand::Column(name) = operand else {
                return Err(QueryError::UnsupportedOperation {
                    message: "internal error: aggregate ORDER BY in a non-aggregate query"
                        .to_string(),
                });
            };
            let target = match alias_operand(sel, name) {
                Some(Operand::Column(c)) => c.clone(),
                _ => name.clone(),
            };
            let idx = resolve(plan.schema(), scope, &target)?;
            keys.push(SortExpr {
                column: idx,
                desc: *desc,
            });
        }
        plan = Arc::new(SortExec::new(plan, keys).with_keep(top_k(sel)));
    }

    let input = plan.schema().clone();
    let mut items = Vec::new();
    for item in &sel.projections {
        match item {
            ProjectionItem::Wild => {
                for (i, f) in input.fields.iter().enumerate() {
                    items.push((i, f.name.clone(), None));
                }
            }
            ProjectionItem::Expr {
                operand: Operand::Column(name),
                alias,
            } => {
                let idx = resolve(&input, scope, name)?;
                items.push((idx, input.fields[idx].name.clone(), alias.clone()));
            }
            ProjectionItem::Expr { .. } => {
                return Err(QueryError::UnsupportedOperation {
                    message: "internal error: aggregate projection in a non-aggregate query"
                        .to_string(),
                });
            }
        }
    }
    project_items(plan, &items, scope)
}

/// Plan a grouped query: aggregate, filter (HAVING), sort, then project.
fn plan_grouped(
    plan: Arc<dyn PhysicalPlan>,
    scope: &Scope,
    sel: &Select,
) -> Result<Arc<dyn PhysicalPlan>, QueryError> {
    let input = plan.schema().clone();

    // DISTINCT over plain columns is a GROUP BY over those columns.
    let group_names: Vec<String> = if !sel.group_by.is_empty() {
        sel.group_by.clone()
    } else if sel.distinct && !has_aggregates(sel) {
        sel.projections
            .iter()
            .flat_map(|p| match p {
                ProjectionItem::Wild => input.fields.iter().map(|f| f.name.clone()).collect(),
                ProjectionItem::Expr {
                    operand: Operand::Column(c),
                    ..
                } => vec![c.clone()],
                ProjectionItem::Expr { .. } => Vec::new(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let group_idx: Vec<usize> = group_names
        .iter()
        .map(|n| resolve(&input, scope, n))
        .collect::<Result<_, _>>()?;

    // Collect every aggregate the query uses, including ones that only
    // appear in HAVING or ORDER BY.
    let mut aggs: Vec<Aggregate> = Vec::new();
    for item in &sel.projections {
        match item {
            ProjectionItem::Wild => {}
            ProjectionItem::Expr {
                operand: Operand::Column(name),
                ..
            } => {
                let idx = resolve(&input, scope, name)?;
                if !group_idx.contains(&idx) {
                    let bare = name.rsplit('.').next().unwrap_or(name);
                    return Err(QueryError::UnsupportedOperation {
                        message: format!(
                            "column '{bare}' must appear in the GROUP BY clause or be used in an aggregate function"
                        ),
                    });
                }
            }
            ProjectionItem::Expr {
                operand: Operand::Aggregate { func, column },
                ..
            } => {
                add_aggregate(&mut aggs, &input, scope, *func, column.as_deref())?;
            }
        }
    }
    if let Some(c) = &sel.having {
        let mut ops = Vec::new();
        cond_operands(c, &mut ops);
        for o in &ops {
            if let Operand::Aggregate { func, column } = o {
                add_aggregate(&mut aggs, &input, scope, *func, column.as_deref())?;
            }
        }
    }
    for (o, _) in &sel.order_by {
        if let Operand::Aggregate { func, column } = o {
            add_aggregate(&mut aggs, &input, scope, *func, column.as_deref())?;
        }
    }

    let agg_plan: Arc<dyn PhysicalPlan> = Arc::new(AggregateExec::new(plan, group_idx, aggs)?);
    let agg_schema = agg_plan.schema().clone();
    let mut plan = agg_plan;

    // Resolve an operand against the aggregate output. A bare name may also
    // be an alias, which takes precedence in HAVING and ORDER BY.
    let agg_ref = |operand: &Operand, use_alias: bool| -> Result<usize, QueryError> {
        let target = match operand {
            Operand::Column(name) if use_alias => alias_operand(sel, name).unwrap_or(operand),
            _ => operand,
        };
        match target {
            Operand::Aggregate { func, column } => {
                let (name, _, _) = aggregate_name(&input, scope, *func, column.as_deref())?;
                agg_schema
                    .index_of(&name)
                    .ok_or_else(|| QueryError::MissingColumn {
                        column: name.clone(),
                        available: names(&agg_schema),
                    })
            }
            Operand::Column(name) => resolve(&agg_schema, scope, name),
        }
    };

    if let Some(c) = &sel.having {
        let cond = bind_cond(c, &|op: &Operand| agg_ref(op, true), &agg_schema)?;
        plan = apply_filter(plan, cond);
    }

    if !sel.order_by.is_empty() {
        let mut keys = Vec::with_capacity(sel.order_by.len());
        for (operand, desc) in &sel.order_by {
            keys.push(SortExpr {
                column: agg_ref(operand, true)?,
                desc: *desc,
            });
        }
        plan = Arc::new(SortExec::new(plan, keys).with_keep(top_k(sel)));
    }

    let mut items = Vec::new();
    for item in &sel.projections {
        match item {
            ProjectionItem::Wild => {
                for (i, f) in agg_schema.fields.iter().enumerate() {
                    items.push((i, f.name.clone(), None));
                }
            }
            ProjectionItem::Expr { operand, alias } => {
                let idx = agg_ref(operand, false)?;
                items.push((idx, agg_schema.fields[idx].name.clone(), alias.clone()));
            }
        }
    }
    project_items(plan, &items, scope)
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

    /// Parse, plan, and return the physical plan for `sql` without running it.
    fn build_plan(&self, sql: &str) -> Result<Arc<dyn PhysicalPlan>, QueryError> {
        let sel = parse_sql(sql)?;
        let left_base = self.lookup(&sel.from)?;
        // Prune only when every reference resolves. Otherwise the query fails
        // as before, and its error lists the table's full set of columns.
        let mut inputs: Vec<(&str, &Table)> = vec![(sel.from.as_str(), left_base)];
        for join in &sel.joins {
            inputs.push((join.right.as_str(), self.lookup(&join.right)?));
        }
        let needed = needed_refs(&sel).filter(|refs| refs_resolve(refs, &inputs));
        let left_owned = prune(left_base, &sel.from, needed.as_deref());
        let left = &left_owned;

        let mut tables = vec![sel.from.clone()];
        let mut plan: Arc<dyn PhysicalPlan> = if sel.joins.is_empty() {
            Arc::new(DataSource::from_shared(
                Arc::new(left.schema().clone()),
                left.batches().to_vec(),
            ))
        } else {
            qualified_source(left, &sel.from)
        };
        for join in &sel.joins {
            if tables.contains(&join.right) {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "self-joins are not supported: '{}' cannot be joined with itself (table aliases are not supported yet)",
                        join.right
                    ),
                });
            }
            let right_base = self.lookup(&join.right)?;
            let right_owned = prune(right_base, &join.right, needed.as_deref());
            let (lk, rk) = join_keys(
                plan.schema(),
                &tables,
                &right_owned,
                &join.right,
                &join.on.0,
                &join.on.1,
            )?;
            let rp = qualified_source(&right_owned, &join.right);
            plan = Arc::new(JoinExec::try_new_join(plan, rp, lk, rk, join.kind)?);
            tables.push(join.right.clone());
        }
        let scope = Scope {
            tables,
            joined: !sel.joins.is_empty(),
        };

        if let Some(c) = &sel.filter {
            let cond = bind_cond(
                c,
                &|op: &Operand| resolve_operand(op, plan.schema(), &scope),
                plan.schema(),
            )?;
            plan = apply_filter(plan, cond);
        }

        let grouped = has_aggregates(&sel)
            || !sel.group_by.is_empty()
            || sel.having.is_some()
            || sel.distinct;
        plan = if grouped {
            plan_grouped(plan, &scope, &sel)?
        } else {
            plan_plain(plan, &scope, &sel)?
        };

        if sel.limit.is_some() || sel.offset.is_some() {
            plan = Arc::new(LimitExec::with_offset(
                plan,
                sel.offset.unwrap_or(0),
                sel.limit.unwrap_or(usize::MAX),
            ));
        }
        Ok(plan)
    }

    /// Parse and execute `sql`, returning the result table.
    pub fn run(&self, sql: &str) -> Result<Table, QueryError> {
        let plan = self.build_plan(sql)?;
        let batches = plan.execute()?;
        Ok(Table::from_shared_batches(
            Arc::new(plan.schema().clone()),
            batches,
        ))
    }

    /// Return the physical plan for `sql` as an indented tree, without
    /// executing it. Each operator is one line, with its inputs indented
    /// beneath it.
    pub fn explain(&self, sql: &str) -> Result<String, QueryError> {
        let plan = self.build_plan(sql)?;
        let mut out = String::new();
        plan.explain(0, &mut out);
        Ok(out)
    }

    /// Check that `sql` parses and binds against the registered tables,
    /// without executing it. Useful as a CI or linting step.
    pub fn check(&self, sql: &str) -> Result<(), QueryError> {
        self.build_plan(sql).map(|_| ())
    }
}

impl<'a> Default for SqlContext<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a condition refers to an aggregate.
fn cond_has_aggregate(c: &Cond) -> bool {
    let mut ops = Vec::new();
    cond_operands(c, &mut ops);
    ops.iter().any(|o| matches!(o, Operand::Aggregate { .. }))
}

/// Resolve a `WHERE` operand. Aggregates are rejected earlier, in the parser.
fn resolve_operand(op: &Operand, schema: &Schema, scope: &Scope) -> Result<usize, QueryError> {
    match op {
        Operand::Column(c) => resolve(schema, scope, c),
        Operand::Aggregate { .. } => Err(sql_err(
            "aggregate functions are not allowed in WHERE; use HAVING to filter on an aggregate",
        )),
    }
}

/// Bind a condition: resolve each operand with `resolve`, and check each
/// comparison against `schema`.
fn bind_cond(
    c: &Cond,
    resolve: &dyn Fn(&Operand) -> Result<usize, QueryError>,
    schema: &Schema,
) -> Result<Condition, QueryError> {
    Ok(match c {
        Cond::Leaf(p) => {
            let idx = resolve(&p.target)?;
            Condition::Cmp(bind_predicate(schema, idx, p)?)
        }
        Cond::ColCmp { left, op, right } => {
            let l = resolve(left)?;
            let r = resolve(right)?;
            check_comparable(schema, l, r, left, right)?;
            Condition::ColCmp {
                left: l,
                op: *op,
                right: r,
            }
        }
        Cond::And(parts) => Condition::And(
            parts
                .iter()
                .map(|p| bind_cond(p, resolve, schema))
                .collect::<Result<_, _>>()?,
        ),
        Cond::Or(parts) => Condition::Or(
            parts
                .iter()
                .map(|p| bind_cond(p, resolve, schema))
                .collect::<Result<_, _>>()?,
        ),
    })
}

/// Two columns can be compared when they have the same type, or both are numeric.
fn check_comparable(
    schema: &Schema,
    l: usize,
    r: usize,
    left: &Operand,
    right: &Operand,
) -> Result<(), QueryError> {
    let (Some(lf), Some(rf)) = (schema.field(l), schema.field(r)) else {
        return Err(sql_err("internal error: resolved column missing"));
    };
    let numeric =
        |dt: DataType| matches!(dt, DataType::Int32 | DataType::Int64 | DataType::Float64);
    if lf.data_type == rf.data_type || (numeric(lf.data_type) && numeric(rf.data_type)) {
        return Ok(());
    }
    Err(QueryError::TypeMismatch {
        column: left.label(),
        expected: lf.data_type.name(),
        actual: rf.data_type.name(),
        context: format!(
            "cannot compare '{}' ({}) with '{}' ({})",
            left.label(),
            lf.data_type,
            right.label(),
            rf.data_type
        ),
    })
}

/// The number of rows a LIMIT keeps from the sort below it: `offset + limit`.
/// Sort and limit both run after any DISTINCT or GROUP BY, so a top-k taken
/// at the sort gives the same rows as sorting everything and then limiting.
fn top_k(sel: &Select) -> Option<usize> {
    sel.limit.map(|n| sel.offset.unwrap_or(0).saturating_add(n))
}

/// Apply a bound condition. A conjunction of plain comparisons becomes one
/// fused filter, which keeps the typed fast path and copies the columns once.
/// Anything else is a condition tree in one filter.
fn apply_filter(plan: Arc<dyn PhysicalPlan>, cond: Condition) -> Arc<dyn PhysicalPlan> {
    match cond {
        Condition::Cmp(p) => Arc::new(FilterExec::new(plan, p)),
        Condition::And(parts) if parts.iter().all(|c| matches!(c, Condition::Cmp(_))) => {
            let predicates = parts
                .into_iter()
                .filter_map(|part| match part {
                    Condition::Cmp(p) => Some(p),
                    _ => None,
                })
                .collect();
            Arc::new(FilterExec::new_all(plan, predicates))
        }
        other => Arc::new(CondFilterExec::new(plan, other)),
    }
}
