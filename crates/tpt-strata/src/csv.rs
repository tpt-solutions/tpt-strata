//! CSV reading and writing with no dependencies.
//!
//! The reader follows RFC 4180: fields may be quoted, a quoted field may hold
//! commas, newlines and doubled quotes (`""`), and the first record is the
//! header. Column types are inferred from the data, in this order: `i64`,
//! then `f64`, then `bool` (`true`/`false`), then `str`. An empty unquoted
//! field is NULL. A column with only NULLs is a `str` column.

use std::sync::Arc;

use crate::array::{ArrayRef, BoolArray, F64Array, I64Array, StrArray};
use crate::batch::Batch;
use crate::error::QueryError;
use crate::format::Table;
use crate::schema::{DataType, Field, Schema};
use crate::types::Scalar;

/// One parsed field. `None` marks an empty unquoted field (NULL).
type Cell = Option<String>;

fn csv_err(message: impl Into<String>) -> QueryError {
    QueryError::UnsupportedOperation {
        message: format!("CSV: {}", message.into()),
    }
}

/// Split CSV text into records of fields.
fn parse_records(text: &str) -> Result<Vec<Vec<Cell>>, QueryError> {
    let mut records: Vec<Vec<Cell>> = Vec::new();
    let mut record: Vec<Cell> = Vec::new();
    let mut field = String::new();
    // Whether the current field was quoted, which decides NULL vs empty string.
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    let mut any = false;

    let finish_field = |field: &mut String, quoted: &mut bool, record: &mut Vec<Cell>| {
        let cell = if field.is_empty() && !*quoted {
            None
        } else {
            Some(std::mem::take(field))
        };
        field.clear();
        *quoted = false;
        record.push(cell);
    };

    while let Some(c) = chars.next() {
        any = true;
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !quoted => {
                in_quotes = true;
                quoted = true;
            }
            ',' => finish_field(&mut field, &mut quoted, &mut record),
            '\r' => {}
            '\n' => {
                // A blank line is not a record.
                let blank = record.is_empty() && field.is_empty() && !quoted;
                if !blank {
                    finish_field(&mut field, &mut quoted, &mut record);
                    records.push(std::mem::take(&mut record));
                }
            }
            other => field.push(other),
        }
    }
    if in_quotes {
        return Err(csv_err("unterminated quoted field at end of input"));
    }
    if any && (!field.is_empty() || quoted || !record.is_empty()) {
        finish_field(&mut field, &mut quoted, &mut record);
        records.push(record);
    }
    Ok(records)
}

/// Infer the narrowest type that every non-null cell in a column parses as.
fn infer_type(cells: &[Cell]) -> DataType {
    if cells.iter().all(|c| c.is_none()) {
        return DataType::Utf8;
    }
    if cells
        .iter()
        .flatten()
        .all(|v| v.trim().parse::<i64>().is_ok())
    {
        return DataType::Int64;
    }
    if cells
        .iter()
        .flatten()
        .all(|v| v.trim().parse::<f64>().is_ok())
    {
        return DataType::Float64;
    }
    let is_bool = |v: &str| {
        let t = v.trim();
        t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false")
    };
    if cells.iter().flatten().all(|v| is_bool(v)) {
        return DataType::Boolean;
    }
    DataType::Utf8
}

fn column_array(dt: DataType, cells: &[Cell]) -> ArrayRef {
    match dt {
        DataType::Int64 => ArrayRef::Int64(I64Array::from_nulls(
            cells
                .iter()
                .map(|c| {
                    c.as_ref()
                        .map(|v| v.trim().parse::<i64>().unwrap_or_default())
                })
                .collect(),
        )),
        DataType::Float64 => ArrayRef::Float64(F64Array::from_nulls(
            cells
                .iter()
                .map(|c| {
                    c.as_ref()
                        .map(|v| v.trim().parse::<f64>().unwrap_or_default())
                })
                .collect(),
        )),
        DataType::Boolean => ArrayRef::Boolean(BoolArray::from_nulls(
            cells
                .iter()
                .map(|c| c.as_ref().map(|v| v.trim().eq_ignore_ascii_case("true")))
                .collect(),
        )),
        DataType::Utf8 | DataType::Int32 => ArrayRef::Utf8(StrArray::from_nulls(cells.to_vec())),
    }
}

/// Parse CSV text into a [`Table`]. The first record is the header.
///
/// Every record must have as many fields as the header. Column types are
/// inferred as described in the module docs; integers become `i64`.
///
/// ```rust
/// use tpt_strata::csv::parse_csv;
/// use tpt_strata::Scalar;
///
/// let t = parse_csv("city,pop\nnyc,8000000\n\"sf, ca\",\n").unwrap();
/// assert_eq!(t.row_count(), 2);
/// assert_eq!(t.batches()[0].columns[0].get(1), Some(Scalar::Str("sf, ca".into())));
/// assert_eq!(t.batches()[0].columns[1].get(1), None);
/// ```
pub fn parse_csv(text: &str) -> Result<Table, QueryError> {
    let mut records = parse_records(text)?;
    if records.is_empty() {
        return Err(csv_err("input is empty; a header row is required"));
    }
    let header: Vec<String> = records
        .remove(0)
        .into_iter()
        .map(|c| c.unwrap_or_default())
        .collect();
    if header.iter().any(|h| h.is_empty()) {
        return Err(csv_err("the header row has an empty column name"));
    }
    for (i, rec) in records.iter().enumerate() {
        if rec.len() != header.len() {
            return Err(csv_err(format!(
                "record {} has {} fields but the header has {}",
                i + 2,
                rec.len(),
                header.len()
            )));
        }
    }

    let mut fields = Vec::with_capacity(header.len());
    let mut arrays = Vec::with_capacity(header.len());
    for (ci, name) in header.iter().enumerate() {
        let cells: Vec<Cell> = records.iter().map(|r| r[ci].clone()).collect();
        let dt = infer_type(&cells);
        let nullable = cells.iter().any(|c| c.is_none());
        fields.push(Field::new(name.clone(), dt, nullable));
        arrays.push(column_array(dt, &cells));
    }
    let schema = Arc::new(Schema::new(fields));
    Ok(Table::from_batches(
        schema.clone(),
        vec![Batch::new_unchecked(schema, arrays)],
    ))
}

/// Render a table as CSV text, with a header row. NULL is an empty field.
///
/// Fields containing a comma, quote, or line break are quoted.
pub fn to_csv(table: &Table) -> String {
    fn quote(s: &str) -> String {
        if s.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    let mut out = String::new();
    let header: Vec<String> = table
        .schema()
        .fields
        .iter()
        .map(|f| quote(&f.name))
        .collect();
    out.push_str(&header.join(","));
    out.push('\n');
    for batch in table.batches() {
        for r in 0..batch.row_count() {
            let row: Vec<String> = batch
                .columns
                .iter()
                .map(|c| match c.get(r) {
                    Some(Scalar::Null) | None => String::new(),
                    Some(v) => quote(&v.to_string()),
                })
                .collect();
            out.push_str(&row.join(","));
            out.push('\n');
        }
    }
    out
}
