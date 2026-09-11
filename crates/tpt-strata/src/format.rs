use crate::array::{ArrayRef, BoolArray, F64Array, I32Array, I64Array, StrArray};
use crate::batch::Batch;
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
#[derive(Debug, Clone)]
pub struct Table {
    schema: Arc<Schema>,
    batches: Vec<Batch>,
}

impl Table {
    /// Build a table from value-based columns, inferring the schema.
    ///
    /// Returns an error if a column is entirely null/empty (type cannot be
    /// inferred) or if columns have differing lengths.
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
            batches: vec![batch],
        })
    }

    /// Build a table from batches with a shared schema.
    pub fn from_batches(schema: Arc<Schema>, batches: Vec<Batch>) -> Self {
        Self { schema, batches }
    }

    /// An empty table with a declared schema (for all-null/empty inputs).
    pub fn empty(schema: Arc<Schema>) -> Self {
        let fields = schema.fields.clone();
        let arrays: Vec<ArrayRef> = fields.iter().map(|f| empty_array(f.data_type)).collect();
        let batch = Batch::new_unchecked(schema.clone(), arrays);
        Self {
            schema,
            batches: vec![batch],
        }
    }

    /// The table's schema: field names, types, and nullability.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The batches that hold the table's data.
    pub fn batches(&self) -> &[Batch] {
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

    /// Concatenate two tables with identical schemas into a single table.
    pub fn concat(&self, other: &Table) -> Result<Table, QueryError> {
        if self.schema != other.schema {
            return Err(QueryError::UnsupportedOperation {
                message: "cannot concatenate tables with different schemas".to_string(),
            });
        }
        let mut batches = self.batches.clone();
        batches.extend(other.batches.iter().cloned());
        Ok(Table::from_batches(self.schema.clone(), batches))
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
            Ok(ArrayRef::Boolean(BoolArray(v)))
        }
        DataType::Int32 => {
            let v: Vec<Option<i32>> = values
                .iter()
                .map(|s| match s {
                    Scalar::I32(b) => Some(*b),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Int32(I32Array(v)))
        }
        DataType::Int64 => {
            let v: Vec<Option<i64>> = values
                .iter()
                .map(|s| match s {
                    Scalar::I64(b) => Some(*b),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Int64(I64Array(v)))
        }
        DataType::Float64 => {
            let v: Vec<Option<f64>> = values
                .iter()
                .map(|s| match s {
                    Scalar::F64(b) => Some(*b),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Float64(F64Array(v)))
        }
        DataType::Utf8 => {
            let v: Vec<Option<String>> = values
                .iter()
                .map(|s| match s {
                    Scalar::Str(b) => Some(b.clone()),
                    _ => None,
                })
                .collect();
            Ok(ArrayRef::Utf8(StrArray(v)))
        }
    }
}

fn empty_array(data_type: DataType) -> ArrayRef {
    match data_type {
        DataType::Boolean => ArrayRef::Boolean(BoolArray(Vec::new())),
        DataType::Int32 => ArrayRef::Int32(I32Array(Vec::new())),
        DataType::Int64 => ArrayRef::Int64(I64Array(Vec::new())),
        DataType::Float64 => ArrayRef::Float64(F64Array(Vec::new())),
        DataType::Utf8 => ArrayRef::Utf8(StrArray(Vec::new())),
    }
}

/// Print a table to stdout in a formatted grid.
pub fn print_table(table: &Table) {
    if table.schema.is_empty() || table.row_count() == 0 {
        println!("(empty result)");
        return;
    }

    // Collect all row values as strings in column order.
    let mut col_strs: Vec<Vec<String>> = (0..table.schema.len()).map(|_| Vec::new()).collect();
    for batch in table.batches() {
        let rows = batch.row_count();
        for r in 0..rows {
            for (ci, col) in batch.columns.iter().enumerate() {
                match col.get(r) {
                    Some(v) => col_strs[ci].push(format!("{v}")),
                    None => col_strs[ci].push("NULL".to_string()),
                }
            }
        }
    }

    let widths: Vec<usize> = table
        .schema
        .fields
        .iter()
        .enumerate()
        .map(|(ci, f)| {
            let data_width = col_strs[ci].iter().map(|s| s.len()).max().unwrap_or(0);
            f.name.len().max(data_width)
        })
        .collect();

    print!("+");
    for w in &widths {
        print!("{}+", "-".repeat(w + 2));
    }
    println!();

    print!("|");
    for (f, w) in table.schema.fields.iter().zip(&widths) {
        print!(" {} {} |", f.name, " ".repeat(w - f.name.len()));
    }
    println!();

    print!("+");
    for w in &widths {
        print!("{}+", "-".repeat(w + 2));
    }
    println!();

    let n_rows = col_strs.first().map_or(0, Vec::len);
    let mut col_iters: Vec<_> = col_strs.iter().map(|c| c.iter()).collect();
    for _ in 0..n_rows {
        print!("|");
        for (ci, w) in widths.iter().enumerate() {
            let val = col_iters[ci].next().expect("equal column lengths");
            print!(" {} {} |", val, " ".repeat(w.saturating_sub(val.len())));
        }
        println!();
    }

    print!("+");
    for w in &widths {
        print!("{}+", "-".repeat(w + 2));
    }
    println!();
}
