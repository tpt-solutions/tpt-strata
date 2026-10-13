use crate::array::{empty_array, ArrayRef, BoolArray, F64Array, I32Array, I64Array, StrArray};
use crate::batch::{Batch, BatchRef};
use crate::error::QueryError;
use crate::schema::{DataType, Field, Schema};
use crate::types::Scalar;
use std::sync::Arc;

/// A named column of scalar values, used to construct a [`Table`] from Rust data.
#[derive(Debug, Clone)]
pub struct Column {
    /// The column name, as shown in query output and diagnostics.
    pub name: String,
    /// The values in row order; `Scalar::Null` marks a missing value.
    pub values: Vec<Scalar>,
}

impl Column {
    pub fn new(name: impl Into<String>, values: Vec<Scalar>) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }
}

/// The result of executing a query: a schema plus one or more batches.
///
/// Batches are shared (`Arc`), so cloning a table or reading it into a query
/// does not copy column data.
#[derive(Debug, Clone)]
pub struct Table {
    schema: Arc<Schema>,
    batches: Vec<BatchRef>,
}

impl Table {
    /// Build a table from value-based columns, inferring the schema.
    ///
    /// Returns an error if a column is entirely null/empty (type cannot be
    /// inferred) or if columns have differing lengths. Use [`Table::from_rows`]
    /// for row-oriented data, or build [`Batch`]es directly for an explicit schema.
    pub fn try_new(columns: Vec<Column>) -> Result<Self, QueryError> {
        let row_count = columns.first().map_or(0, |c| c.values.len());
        for col in &columns {
            if col.values.len() != row_count {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "column '{}' has {} values but '{}' has {}",
                        col.name,
                        col.values.len(),
                        columns[0].name,
                        row_count
                    ),
                });
            }
        }

        let mut fields: Vec<Field> = Vec::with_capacity(columns.len());
        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(columns.len());

        for col in &columns {
            // Infer the type from the first non-null value.
            let data_type = col
                .values
                .iter()
                .find(|v| !matches!(v, Scalar::Null))
                .map(|v| match v {
                    Scalar::Bool(_) => DataType::Boolean,
                    Scalar::I32(_) => DataType::Int32,
                    Scalar::I64(_) => DataType::Int64,
                    Scalar::F64(_) => DataType::Float64,
                    Scalar::Str(_) => DataType::Utf8,
                    Scalar::Date(_) => DataType::Date,
                    Scalar::Timestamp(_) => DataType::Timestamp,
                    Scalar::Null => unreachable!("filtered above"),
                })
                .ok_or_else(|| QueryError::UnsupportedOperation {
                    message: format!(
                        "cannot infer the type of column '{}': it is all null/empty; construct a schema explicitly",
                        col.name
                    ),
                })?;

            for v in col.values.iter().filter(|v| !matches!(v, Scalar::Null)) {
                if v.type_name() != data_type.name() {
                    return Err(QueryError::TypeMismatch {
                        column: col.name.clone(),
                        expected: data_type.name(),
                        actual: v.type_name(),
                        context: format!(
                            "column '{}' contains a mix of {} and {} values; each column must have a single type",
                            col.name,
                            data_type.name(),
                            v.type_name()
                        ),
                    });
                }
            }

            let nullable = col.values.iter().any(|v| matches!(v, Scalar::Null));
            fields.push(Field::new(col.name.clone(), data_type, nullable));
            arrays.push(value_to_array(data_type, &col.values)?);
        }

        let schema = Arc::new(Schema::new(fields));
        let batch = Batch::try_new(schema.clone(), arrays)?;
        Ok(Table {
            schema,
            batches: vec![Arc::new(batch)],
        })
    }

    /// Build a table from row-oriented data.
    ///
    /// `names` gives the column names. Each row must supply one value per
    /// name. Types are inferred per column exactly as in [`Table::try_new`].
    ///
    /// ```rust
    /// use tpt_strata::{Scalar, Table};
    ///
    /// let t = Table::from_rows(
    ///     &["city", "pop"],
    ///     vec![
    ///         vec![Scalar::Str("nyc".into()), Scalar::I64(8_000_000)],
    ///         vec![Scalar::Str("sfo".into()), Scalar::I64(800_000)],
    ///     ],
    /// ).unwrap();
    /// assert_eq!(t.row_count(), 2);
    /// ```
    pub fn from_rows(names: &[&str], rows: Vec<Vec<Scalar>>) -> Result<Self, QueryError> {
        let mut values: Vec<Vec<Scalar>> = (0..names.len())
            .map(|_| Vec::with_capacity(rows.len()))
            .collect();
        for (i, row) in rows.into_iter().enumerate() {
            if row.len() != names.len() {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "row {} has {} values but {} columns were named",
                        i + 1,
                        row.len(),
                        names.len()
                    ),
                });
            }
            for (col, v) in values.iter_mut().zip(row) {
                col.push(v);
            }
        }
        let columns = names
            .iter()
            .zip(values)
            .map(|(name, vals)| Column::new(*name, vals))
            .collect();
        Self::try_new(columns)
    }

    /// Build a table from batches with a shared schema.
    pub fn from_batches(schema: Arc<Schema>, batches: Vec<Batch>) -> Self {
        Self {
            schema,
            batches: batches.into_iter().map(Arc::new).collect(),
        }
    }

    /// Build a table from batches that are already shared.
    pub fn from_shared_batches(schema: Arc<Schema>, batches: Vec<BatchRef>) -> Self {
        Self { schema, batches }
    }

    /// An empty table with a declared schema (for all-null/empty inputs).
    pub fn empty(schema: Arc<Schema>) -> Self {
        let arrays: Vec<ArrayRef> = schema
            .fields
            .iter()
            .map(|f| empty_array(f.data_type))
            .collect();
        let batch = Batch::new_unchecked(schema.clone(), arrays);
        Self {
            schema,
            batches: vec![Arc::new(batch)],
        }
    }

    /// The table's schema: field names, types, and nullability.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The batches that hold the table's data.
    pub fn batches(&self) -> &[BatchRef] {
        &self.batches
    }

    /// The total number of rows across all batches.
    pub fn row_count(&self) -> usize {
        self.batches.iter().map(|b| b.row_count()).sum()
    }

    /// The column names in schema order.
    pub fn column_names(&self) -> Vec<&str> {
        self.schema.fields.iter().map(|f| f.name.as_str()).collect()
    }

    /// The index of the named column, or a [`QueryError::MissingColumn`]
    /// diagnostic listing the schema's actual columns.
    pub fn column_index(&self, name: &str) -> Result<usize, QueryError> {
        self.schema.index_of(name).ok_or_else(|| {
            let available: Vec<String> =
                self.schema.fields.iter().map(|f| f.name.clone()).collect();
            QueryError::MissingColumn {
                column: name.to_string(),
                available,
            }
        })
    }

    /// Return a copy of this table with its columns renamed, in order.
    ///
    /// Use this to make two tables' column names distinct before a join.
    pub fn rename_columns(&self, names: &[&str]) -> Result<Table, QueryError> {
        if names.len() != self.schema.len() {
            return Err(QueryError::UnsupportedOperation {
                message: format!(
                    "rename_columns needs {} names but got {}",
                    self.schema.len(),
                    names.len()
                ),
            });
        }
        let fields: Vec<Field> = self
            .schema
            .fields
            .iter()
            .zip(names)
            .map(|(f, n)| Field::new(*n, f.data_type, f.nullable))
            .collect();
        let schema = Arc::new(Schema::new(fields));
        let batches = self
            .batches
            .iter()
            .map(|b| Arc::new(Batch::new_shared(schema.clone(), b.columns.clone())))
            .collect();
        Ok(Table::from_shared_batches(schema, batches))
    }

    /// A table with only the columns at `idx`, in that order. The columns are
    /// shared, not copied.
    pub(crate) fn select_columns(&self, idx: &[usize]) -> Table {
        let fields: Vec<Field> = idx.iter().map(|&i| self.schema.fields[i].clone()).collect();
        let schema = Arc::new(Schema::new(fields));
        let batches = self
            .batches
            .iter()
            .map(|b| {
                let cols = idx.iter().map(|&i| b.columns[i].clone()).collect();
                Arc::new(Batch::new_shared(schema.clone(), cols))
            })
            .collect();
        Table::from_shared_batches(schema, batches)
    }

    /// Concatenate two tables with the same column names and types.
    ///
    /// Nullability may differ: the result marks a column nullable if either
    /// input does.
    pub fn concat(&self, other: &Table) -> Result<Table, QueryError> {
        let same_shape = self.schema.len() == other.schema.len()
            && self
                .schema
                .fields
                .iter()
                .zip(&other.schema.fields)
                .all(|(a, b)| a.name == b.name && a.data_type == b.data_type);
        if !same_shape {
            return Err(QueryError::UnsupportedOperation {
                message: "cannot concatenate tables with different schemas".to_string(),
            });
        }
        let fields: Vec<Field> = self
            .schema
            .fields
            .iter()
            .zip(&other.schema.fields)
            .map(|(a, b)| Field::new(a.name.clone(), a.data_type, a.nullable || b.nullable))
            .collect();
        let schema = Arc::new(Schema::new(fields));
        let mut batches = self.batches.clone();
        batches.extend(other.batches.iter().cloned());
        Ok(Table::from_shared_batches(schema, batches))
    }
}

fn value_to_array(data_type: DataType, values: &[Scalar]) -> Result<ArrayRef, QueryError> {
    match data_type {
        DataType::Boolean => {
            let v: Vec<Option<bool>> = values
                .iter()
                .map(|s| match s {
                    Scalar::Bool(b) => Some(*b),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Boolean(BoolArray::from_nulls(v)))
        }
        DataType::Int32 => {
            let v: Vec<Option<i32>> = values
                .iter()
                .map(|s| match s {
                    Scalar::I32(b) => Some(*b),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Int32(I32Array::from_nulls(v)))
        }
        DataType::Int64 => {
            let v: Vec<Option<i64>> = values
                .iter()
                .map(|s| match s {
                    Scalar::I64(b) => Some(*b),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Int64(I64Array::from_nulls(v)))
        }
        DataType::Float64 => {
            let v: Vec<Option<f64>> = values
                .iter()
                .map(|s| match s {
                    Scalar::F64(b) => Some(*b),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Float64(F64Array::from_nulls(v)))
        }
        DataType::Utf8 => {
            let v: Vec<Option<String>> = values
                .iter()
                .map(|s| match s {
                    Scalar::Str(b) => Some(b.clone()),
                    _ => None,
                })
                .collect();
            let v = v
                .into_iter()
                .map(|s| s.map(std::sync::Arc::<str>::from))
                .collect();
            Ok(ArrayRef::Utf8(StrArray::from_nulls(v)))
        }
        DataType::Date => {
            let v: Vec<Option<i32>> = values
                .iter()
                .map(|s| match s {
                    Scalar::Date(d) => Some(*d),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Date(I32Array::from_nulls(v)))
        }
        DataType::Timestamp => {
            let v: Vec<Option<i64>> = values
                .iter()
                .map(|s| match s {
                    Scalar::Timestamp(t) => Some(*t),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Timestamp(I64Array::from_nulls(v)))
        }
    }
}

/// Render a table as a grid of text, as [`print_table`] prints it.
///
/// Column widths count characters, not bytes, so non-ASCII values line up.
/// Newlines inside values are shown as `\n`. A zero-row result still shows
/// its header.
pub fn render_table(table: &Table) -> String {
    let fields = &table.schema.fields;
    if fields.is_empty() {
        return "(no columns)\n".to_string();
    }

    let mut cells: Vec<Vec<String>> = (0..fields.len()).map(|_| Vec::new()).collect();
    for batch in table.batches() {
        for r in 0..batch.row_count() {
            for (ci, col) in batch.columns.iter().enumerate() {
                let text = match col.get(r) {
                    Some(v) => v.to_string(),
                    None => "NULL".to_string(),
                };
                cells[ci].push(text.replace('\n', "\\n"));
            }
        }
    }

    let header: Vec<String> = fields.iter().map(|f| f.name.replace('\n', "\\n")).collect();
    let widths: Vec<usize> = (0..fields.len())
        .map(|ci| {
            let data = cells[ci]
                .iter()
                .map(|s| display_width(s))
                .max()
                .unwrap_or(0);
            display_width(&header[ci]).max(data)
        })
        .collect();

    let mut out = String::new();
    let rule: String = {
        let parts: Vec<String> = widths.iter().map(|w| "-".repeat(w + 2)).collect();
        format!("+{}+\n", parts.join("+"))
    };
    let line = |vals: &[&str]| -> String {
        let mut s = String::from("|");
        for (v, w) in vals.iter().zip(&widths) {
            s.push_str(&format!(" {v}{} |", " ".repeat(w - display_width(v))));
        }
        s.push('\n');
        s
    };

    out.push_str(&rule);
    let header_refs: Vec<&str> = header.iter().map(String::as_str).collect();
    out.push_str(&line(&header_refs));
    out.push_str(&rule);
    for r in 0..table.row_count() {
        let row: Vec<&str> = cells.iter().map(|c| c[r].as_str()).collect();
        out.push_str(&line(&row));
    }
    out.push_str(&rule);
    if table.row_count() == 0 {
        out.push_str("(0 rows)\n");
    }
    out
}

fn display_width(s: &str) -> usize {
    s.chars().count()
}

/// Print a table to stdout in a formatted grid. See [`render_table`].
pub fn print_table(table: &Table) {
    print!("{}", render_table(table));
}
