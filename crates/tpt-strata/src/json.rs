//! JSON ingest with no dependencies.
//!
//! Reads a JSON array of flat objects (`[{"a": 1}, {"a": 2}]`), or a single
//! object, or newline-delimited objects (one object per line). Keys become
//! columns in order of first appearance. A key missing from a record is NULL.
//! Column types are inferred from the values: `i64` if every value is an
//! integer, then `f64`, then `bool`, then `str`. Nested arrays and objects
//! are rejected with the column name, because a table cell holds one scalar.

use std::sync::Arc;

use crate::array::{ArrayRef, BoolArray, F64Array, I64Array, StrArray};
use crate::batch::Batch;
use crate::error::QueryError;
use crate::format::Table;
use crate::schema::{DataType, Field, Schema};

#[derive(Debug, Clone)]
enum Value {
    Null,
    Bool(bool),
    /// The number's text, kept exactly as written.
    Number(String),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

fn import_err(message: impl Into<String>) -> QueryError {
    QueryError::Import {
        message: format!("JSON: {}", message.into()),
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
        }
    }

    fn err(&self, message: &str) -> QueryError {
        let line = self.chars[..self.pos.min(self.chars.len())]
            .iter()
            .filter(|&&c| c == '\n')
            .count()
            + 1;
        import_err(format!("{message} at line {line}"))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn expect(&mut self, c: char) -> Result<(), QueryError> {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{c}'")))
        }
    }

    fn keyword(&mut self, word: &str, value: Value) -> Result<Value, QueryError> {
        let end = self.pos + word.len();
        let matches =
            end <= self.chars.len() && self.chars[self.pos..end].iter().copied().eq(word.chars());
        if matches {
            self.pos = end;
            Ok(value)
        } else {
            Err(self.err("unexpected token"))
        }
    }

    fn value(&mut self) -> Result<Value, QueryError> {
        self.skip_ws();
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => Ok(Value::Str(self.string()?)),
            Some('t') => self.keyword("true", Value::Bool(true)),
            Some('f') => self.keyword("false", Value::Bool(false)),
            Some('n') => self.keyword("null", Value::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn object(&mut self) -> Result<Value, QueryError> {
        self.expect('{')?;
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.err("expected a string key"));
            }
            let key = self.string()?;
            self.expect(':')?;
            let value = self.value()?;
            fields.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {
                    self.pos += 1;
                    return Ok(Value::Object(fields));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self) -> Result<Value, QueryError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, QueryError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        let digits = |p: &mut Self| {
            let from = p.pos;
            while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                p.pos += 1;
            }
            p.pos > from
        };
        if !digits(self) {
            return Err(self.err("invalid number"));
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            if !digits(self) {
                return Err(self.err("invalid number"));
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            if !digits(self) {
                return Err(self.err("invalid number"));
            }
        }
        Ok(Value::Number(self.chars[start..self.pos].iter().collect()))
    }

    fn string(&mut self) -> Result<String, QueryError> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
            self.pos += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.pos += 1;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                other => out.push(other),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, QueryError> {
        let end = self.pos + 4;
        if end > self.chars.len() {
            return Err(self.err("invalid unicode escape"));
        }
        let text: String = self.chars[self.pos..end].iter().collect();
        let v = u32::from_str_radix(&text, 16).map_err(|_| self.err("invalid unicode escape"))?;
        self.pos = end;
        Ok(v)
    }

    /// `\uXXXX`, including a UTF-16 surrogate pair written as two escapes.
    fn unicode_escape(&mut self) -> Result<char, QueryError> {
        let hi = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&hi) {
            if self.peek() == Some('\\') && self.chars.get(self.pos + 1) == Some(&'u') {
                self.pos += 2;
                let lo = self.hex4()?;
                if !(0xDC00..0xE000).contains(&lo) {
                    return Err(self.err("invalid surrogate pair"));
                }
                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
            } else {
                return Err(self.err("unpaired surrogate"));
            }
        } else {
            hi
        };
        char::from_u32(code).ok_or_else(|| self.err("invalid unicode escape"))
    }

    /// Parse the whole input: an array, one object, or newline-delimited objects.
    fn document(&mut self) -> Result<Vec<Value>, QueryError> {
        self.skip_ws();
        if self.peek() == Some('[') {
            let Value::Array(items) = self.array()? else {
                unreachable!("array() returns Value::Array");
            };
            self.skip_ws();
            if self.pos < self.chars.len() {
                return Err(self.err("unexpected text after the array"));
            }
            return Ok(items);
        }
        let mut records = Vec::new();
        while self.peek().is_some() {
            records.push(self.value()?);
            self.skip_ws();
        }
        Ok(records)
    }
}

fn scalar_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) | Value::Str(n) => Some(n.clone()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// Parse JSON text into a [`Table`]. See the module docs for the accepted shapes.
pub fn parse_json(text: &str) -> Result<Table, QueryError> {
    let records = Parser::new(text).document()?;
    if records.is_empty() {
        return Err(import_err("no records found"));
    }

    // Column names, in order of first appearance across all records.
    let mut names: Vec<String> = Vec::new();
    for rec in &records {
        let Value::Object(fields) = rec else {
            return Err(import_err("every record must be a JSON object"));
        };
        for (k, _) in fields {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
    }

    // One cell per (record, column); None means the key is absent or null.
    let mut cells: Vec<Vec<Option<&Value>>> = vec![Vec::with_capacity(records.len()); names.len()];
    for rec in &records {
        let Value::Object(fields) = rec else {
            unreachable!("checked above");
        };
        for (ci, name) in names.iter().enumerate() {
            let found = fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
            if let Some(Value::Array(_) | Value::Object(_)) = found {
                return Err(import_err(format!(
                    "column '{name}' holds a nested array or object; flatten it first"
                )));
            }
            cells[ci].push(found.filter(|v| !matches!(v, Value::Null)));
        }
    }

    let mut fields = Vec::with_capacity(names.len());
    let mut arrays = Vec::with_capacity(names.len());
    for (name, column) in names.iter().zip(&cells) {
        let present: Vec<&Value> = column.iter().flatten().copied().collect();
        let dt = if present.is_empty() {
            DataType::Utf8
        } else if present.iter().all(|v| matches!(v, Value::Bool(_))) {
            DataType::Boolean
        } else if present
            .iter()
            .all(|v| matches!(v, Value::Number(n) if n.parse::<i64>().is_ok()))
        {
            DataType::Int64
        } else if present
            .iter()
            .all(|v| matches!(v, Value::Number(n) if n.parse::<f64>().is_ok()))
        {
            DataType::Float64
        } else {
            DataType::Utf8
        };
        let nullable = column.iter().any(|c| c.is_none());
        fields.push(Field::new(name.clone(), dt, nullable));
        arrays.push(array_for(dt, column));
    }
    let schema = Arc::new(Schema::new(fields));
    Ok(Table::from_batches(
        schema.clone(),
        vec![Batch::new_unchecked(schema, arrays)],
    ))
}

fn array_for(dt: DataType, column: &[Option<&Value>]) -> ArrayRef {
    match dt {
        DataType::Boolean => ArrayRef::Boolean(BoolArray::from_nulls(
            column
                .iter()
                .map(|c| match c {
                    Some(Value::Bool(b)) => Some(*b),
                    _ => None,
                })
                .collect(),
        )),
        DataType::Int64 => ArrayRef::Int64(I64Array::from_nulls(
            column
                .iter()
                .map(|c| match c {
                    Some(Value::Number(n)) => n.parse::<i64>().ok(),
                    _ => None,
                })
                .collect(),
        )),
        DataType::Float64 => ArrayRef::Float64(F64Array::from_nulls(
            column
                .iter()
                .map(|c| match c {
                    Some(Value::Number(n)) => n.parse::<f64>().ok(),
                    _ => None,
                })
                .collect(),
        )),
        DataType::Utf8 | DataType::Int32 => ArrayRef::Utf8(StrArray::from_nulls(
            column.iter().map(|c| c.and_then(scalar_text)).collect(),
        )),
    }
}
