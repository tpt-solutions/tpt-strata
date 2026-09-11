//! # tpt-strata-parquet
//!
//! Parquet import bridge for tpt-strata.
//!
//! This crate reads Parquet files into tpt-strata's native columnar format.
//! It is the only crate in the workspace permitted to depend on external
//! columnar interop libraries (per tpt-strata's non-negotiable #2).

use std::fs::File;
use std::sync::Arc;

use arrow::array::{Array, BooleanArray, Float64Array, Int32Array, Int64Array, StringArray};
use arrow::datatypes::DataType as ArrowType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use tpt_strata::{ArrayRef, DataType, Field, QueryError, Schema, Table};

const SUPPORTED_TYPES: &str = "BOOLEAN, INT32, INT64, DOUBLE, STRING";

fn unsupported(message: impl Into<String>) -> QueryError {
    QueryError::UnsupportedOperation {
        message: message.into(),
    }
}

/// Map an Arrow/Parquet type to tpt-strata's native type, or produce a
/// caller-facing diagnostic naming the offending column.
fn map_arrow_type(dt: &ArrowType, name: &str) -> Result<DataType, QueryError> {
    match dt {
        ArrowType::Boolean => Ok(DataType::Boolean),
        ArrowType::Int32 => Ok(DataType::Int32),
        ArrowType::Int64 => Ok(DataType::Int64),
        ArrowType::Float64 => Ok(DataType::Float64),
        ArrowType::Utf8 => Ok(DataType::Utf8),
        other => Err(unsupported(format!(
            "unsupported Parquet type for column '{name}': {other}. Supported: {SUPPORTED_TYPES}"
        ))),
    }
}

enum Collector {
    Bool(Vec<Option<bool>>),
    I32(Vec<Option<i32>>),
    I64(Vec<Option<i64>>),
    F64(Vec<Option<f64>>),
    Str(Vec<Option<String>>),
}

impl Collector {
    fn new(dt: DataType) -> Self {
        match dt {
            DataType::Boolean => Collector::Bool(Vec::new()),
            DataType::Int32 => Collector::I32(Vec::new()),
            DataType::Int64 => Collector::I64(Vec::new()),
            DataType::Float64 => Collector::F64(Vec::new()),
            DataType::Utf8 => Collector::Str(Vec::new()),
        }
    }

    fn push(&mut self, arr: &dyn Array, column: &str) -> Result<(), QueryError> {
        match self {
            Collector::Bool(v) => {
                let a = arrow_downcast::<BooleanArray>(arr, column, "BOOLEAN")?;
                for i in 0..a.len() {
                    v.push(if a.is_null(i) { None } else { Some(a.value(i)) });
                }
            }
            Collector::I32(v) => {
                let a = arrow_downcast::<Int32Array>(arr, column, "INT32")?;
                for i in 0..a.len() {
                    v.push(if a.is_null(i) { None } else { Some(a.value(i)) });
                }
            }
            Collector::I64(v) => {
                let a = arrow_downcast::<Int64Array>(arr, column, "INT64")?;
                for i in 0..a.len() {
                    v.push(if a.is_null(i) { None } else { Some(a.value(i)) });
                }
            }
            Collector::F64(v) => {
                let a = arrow_downcast::<Float64Array>(arr, column, "DOUBLE")?;
                for i in 0..a.len() {
                    v.push(if a.is_null(i) { None } else { Some(a.value(i)) });
                }
            }
            Collector::Str(v) => {
                let a = arrow_downcast::<StringArray>(arr, column, "STRING")?;
                for i in 0..a.len() {
                    v.push(if a.is_null(i) {
                        None
                    } else {
                        Some(a.value(i).to_string())
                    });
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> ArrayRef {
        match self {
            Collector::Bool(v) => tpt_strata::BoolArray::from_nulls(v).into(),
            Collector::I32(v) => tpt_strata::I32Array::from_nulls(v).into(),
            Collector::I64(v) => tpt_strata::I64Array::from_nulls(v).into(),
            Collector::F64(v) => tpt_strata::F64Array::from_nulls(v).into(),
            Collector::Str(v) => tpt_strata::StrArray::from_nulls(v).into(),
        }
    }
}

fn arrow_downcast<'a, T: Array + 'static>(
    arr: &'a dyn Array,
    column: &str,
    type_name: &str,
) -> Result<&'a T, QueryError> {
    arr.as_any().downcast_ref::<T>().ok_or_else(|| {
        unsupported(format!(
            "column '{column}': the Parquet file produced a {type_name} column with an unexpected in-memory representation"
        ))
    })
}

/// Read a Parquet file into a [`tpt_strata::Table`].
///
/// Supported Parquet columns: `BOOLEAN`, `INT32`, `INT64`, `DOUBLE`, and
/// `STRING` (UTF-8). Any other type is rejected with a diagnostic naming the
/// offending column.
///
/// # Errors
///
/// Returns a [`QueryError`] if the file cannot be opened/decoded or if the
/// schema contains an unsupported column type.
pub fn read_parquet(path: impl AsRef<std::path::Path>) -> Result<Table, QueryError> {
    let display = path.as_ref().display();

    let file =
        File::open(&path).map_err(|e| unsupported(format!("cannot open '{display}': {e}")))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
        unsupported(format!(
            "cannot read Parquet metadata from '{display}': {e}"
        ))
    })?;

    let arrow_schema = builder.schema().clone();
    let mut fields = Vec::with_capacity(arrow_schema.fields().len());
    let mut collectors = Vec::with_capacity(arrow_schema.fields().len());
    for f in arrow_schema.fields() {
        let dt = map_arrow_type(f.data_type(), f.name())?;
        fields.push(Field::new(f.name().clone(), dt, f.is_nullable()));
        collectors.push(Collector::new(dt));
    }
    let schema = Arc::new(Schema::new(fields));

    let reader = builder
        .build()
        .map_err(|e| unsupported(format!("cannot build Parquet reader for '{display}': {e}")))?;

    for batch in reader {
        let batch = batch.map_err(|e| {
            unsupported(format!("error reading Parquet data from '{display}': {e}"))
        })?;
        for (i, column) in batch.columns().iter().enumerate() {
            let name = schema.field(i).map(|f| f.name.as_str()).unwrap_or("?");
            collectors[i].push(column.as_ref(), name)?;
        }
    }

    let columns: Vec<ArrayRef> = collectors.into_iter().map(Collector::finish).collect();
    if columns.is_empty() {
        return Ok(Table::empty(schema));
    }

    let batches = vec![tpt_strata::Batch::new_unchecked(schema.clone(), columns)];
    Ok(Table::from_batches(schema, batches))
}
