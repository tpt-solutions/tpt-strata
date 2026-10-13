//! # tpt-strata-parquet
//!
//! Parquet import and export bridge for tpt-strata.
//!
//! This crate reads Parquet files into tpt-strata's native columnar format and
//! writes tables back out. It is the only crate in the workspace permitted to
//! depend on external columnar interop libraries (per tpt-strata's
//! non-negotiable #2).
//!
//! ## Type mapping
//!
//! Types are widened losslessly to the native types. Date and timestamp
//! columns keep their raw numbers, so they are queryable but not formatted.
//!
//! | Parquet / Arrow                         | tpt-strata |
//! |-----------------------------------------|------------|
//! | BOOLEAN                                 | bool       |
//! | INT8, INT16, INT32, UINT8, UINT16       | i32        |
//! | INT64, UINT32                           | i64        |
//! | FLOAT, DOUBLE                           | f64        |
//! | STRING, LARGE_STRING, STRING_VIEW       | str        |
//! | DATE32 (days since 1970-01-01)          | i32        |
//! | TIMESTAMP (raw epoch value, in its unit)| i64        |
//!
//! Other types (unsigned 64-bit, decimals, binary, nested) are rejected with a
//! diagnostic naming the column. Use [`read_parquet_columns`] to read only the
//! supported columns of a file that also has unsupported ones.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef as ArrowArrayRef, BooleanArray, Float64Array, Int32Array, Int64Array,
    StringArray,
};
use arrow::compute::cast;
use arrow::datatypes::{DataType as ArrowType, Field as ArrowField, Schema as ArrowSchema};
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::{ArrowWriter, ProjectionMask};
use parquet::file::reader::ChunkReader;

use tpt_strata::{ArrayRef, Batch, DataType, Field, QueryError, Schema, Table};

const SUPPORTED_TYPES: &str = "BOOLEAN, INT8/16/32, UINT8/16/32, INT64, UINT32, FLOAT, DOUBLE, STRING, LARGE_STRING, STRING_VIEW, DATE32, TIMESTAMP";

fn unsupported(message: impl Into<String>) -> QueryError {
    QueryError::UnsupportedOperation {
        message: message.into(),
    }
}

fn import_err(message: impl Into<String>) -> QueryError {
    QueryError::Import {
        message: message.into(),
    }
}

fn io_err(context: impl Into<String>, source: std::io::Error) -> QueryError {
    QueryError::Io {
        context: context.into(),
        source,
    }
}

/// The native type an Arrow type imports as, plus the Arrow type the column
/// is cast to first so the collector sees one of a few concrete arrays.
/// Returns `None` for types that are not imported.
fn import_target(dt: &ArrowType) -> Option<(DataType, ArrowType)> {
    Some(match dt {
        ArrowType::Boolean => (DataType::Boolean, ArrowType::Boolean),
        ArrowType::Int8
        | ArrowType::Int16
        | ArrowType::Int32
        | ArrowType::UInt8
        | ArrowType::UInt16
        | ArrowType::Date32 => (DataType::Int32, ArrowType::Int32),
        ArrowType::Int64 | ArrowType::UInt32 | ArrowType::Timestamp(_, _) => {
            (DataType::Int64, ArrowType::Int64)
        }
        ArrowType::Float16 | ArrowType::Float32 | ArrowType::Float64 => {
            (DataType::Float64, ArrowType::Float64)
        }
        ArrowType::Utf8 | ArrowType::LargeUtf8 | ArrowType::Utf8View => {
            (DataType::Utf8, ArrowType::Utf8)
        }
        _ => return None,
    })
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

    /// Append `arr`, which has already been cast to this collector's Arrow type.
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
/// Every column must be of a supported type (see the crate docs). To read a
/// file that also has unsupported columns, use [`read_parquet_columns`].
///
/// # Errors
///
/// Returns [`QueryError::Io`] if the file cannot be opened, and
/// [`QueryError::Import`] if it cannot be decoded. A column of an unsupported
/// type is an [`QueryError::UnsupportedOperation`] that names the column.
pub fn read_parquet(path: impl AsRef<Path>) -> Result<Table, QueryError> {
    read_path(path.as_ref(), None)
}

/// Read only the named columns of a Parquet file, in file order.
///
/// Unselected columns are never decoded, so a file with, say, one timestamp
/// column can still be read by selecting the others.
///
/// # Errors
///
/// As [`read_parquet`], plus [`QueryError::MissingColumn`] for a name the
/// file does not contain.
pub fn read_parquet_columns(path: impl AsRef<Path>, columns: &[&str]) -> Result<Table, QueryError> {
    read_path(path.as_ref(), Some(columns))
}

/// Read Parquet data that is already in memory.
pub fn read_parquet_bytes(data: Bytes) -> Result<Table, QueryError> {
    read_source(data, "<in-memory Parquet data>", None)
}

fn read_path(path: &Path, columns: Option<&[&str]>) -> Result<Table, QueryError> {
    let display = path.display().to_string();
    let file = File::open(path).map_err(|e| io_err(format!("cannot open '{display}'"), e))?;
    read_source(file, &display, columns)
}

fn read_source<T: ChunkReader + 'static>(
    source: T,
    display: &str,
    columns: Option<&[&str]>,
) -> Result<Table, QueryError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(source).map_err(|e| {
        import_err(format!(
            "cannot read Parquet metadata from '{display}': {e}"
        ))
    })?;
    let arrow_schema = builder.schema().clone();

    let mut selected: Vec<usize> = match columns {
        None => (0..arrow_schema.fields().len()).collect(),
        Some(names) => names
            .iter()
            .map(|name| {
                arrow_schema
                    .index_of(name)
                    .map_err(|_| QueryError::MissingColumn {
                        column: name.to_string(),
                        available: arrow_schema
                            .fields()
                            .iter()
                            .map(|f| f.name().clone())
                            .collect(),
                    })
            })
            .collect::<Result<_, _>>()?,
    };
    // File order, no duplicates: the reader returns columns in file order.
    selected.sort_unstable();
    selected.dedup();

    let mut fields = Vec::with_capacity(selected.len());
    let mut collectors = Vec::with_capacity(selected.len());
    let mut casts = Vec::with_capacity(selected.len());
    for &i in &selected {
        let f = arrow_schema.field(i);
        let (native, target) = import_target(f.data_type()).ok_or_else(|| {
            unsupported(format!(
                "unsupported Parquet type for column '{}': {}. Supported: {SUPPORTED_TYPES}. Read the supported columns with read_parquet_columns.",
                f.name(),
                f.data_type()
            ))
        })?;
        fields.push(Field::new(f.name().clone(), native, f.is_nullable()));
        collectors.push(Collector::new(native));
        casts.push(target);
    }

    let mask = ProjectionMask::roots(builder.parquet_schema(), selected.iter().copied());
    let reader = builder
        .with_projection(mask)
        .build()
        .map_err(|e| import_err(format!("cannot build Parquet reader for '{display}': {e}")))?;

    for batch in reader {
        let batch = batch
            .map_err(|e| import_err(format!("error reading Parquet data from '{display}': {e}")))?;
        for (i, column) in batch.columns().iter().enumerate() {
            let name = fields[i].name.as_str();
            let cast_col = cast(column.as_ref(), &casts[i]).map_err(|e| {
                unsupported(format!(
                    "column '{name}': cannot convert to a native type: {e}"
                ))
            })?;
            collectors[i].push(cast_col.as_ref(), name)?;
        }
    }

    let schema = Arc::new(Schema::new(fields));
    if schema.is_empty() {
        return Ok(Table::empty(schema));
    }
    let columns: Vec<ArrayRef> = collectors.into_iter().map(Collector::finish).collect();
    let batches = vec![Batch::new_unchecked(schema.clone(), columns)];
    Ok(Table::from_batches(schema, batches))
}

/// Write a table to a Parquet file.
///
/// Column types map back one-to-one: bool, i32, i64, f64 and str. The file is
/// written with Arrow schema metadata, so it reads back with the same types.
pub fn write_parquet(table: &Table, path: impl AsRef<Path>) -> Result<(), QueryError> {
    let display = path.as_ref().display().to_string();
    let schema = Arc::new(ArrowSchema::new(
        table
            .schema()
            .fields
            .iter()
            .map(|f| ArrowField::new(f.name.clone(), arrow_type(f.data_type), f.nullable))
            .collect::<Vec<_>>(),
    ));
    let file =
        File::create(path.as_ref()).map_err(|e| io_err(format!("cannot create '{display}'"), e))?;
    let mut writer = ArrowWriter::try_new(file, schema.clone(), None)
        .map_err(|e| import_err(format!("cannot start Parquet writer for '{display}': {e}")))?;
    for batch in table.batches() {
        let columns: Vec<ArrowArrayRef> = batch.columns.iter().map(|c| to_arrow(c)).collect();
        let record = RecordBatch::try_new(schema.clone(), columns)
            .map_err(|e| import_err(format!("cannot build Arrow batch: {e}")))?;
        writer
            .write(&record)
            .map_err(|e| import_err(format!("cannot write '{display}': {e}")))?;
    }
    writer
        .close()
        .map_err(|e| import_err(format!("cannot finish '{display}': {e}")))?;
    Ok(())
}

fn arrow_type(dt: DataType) -> ArrowType {
    match dt {
        DataType::Boolean => ArrowType::Boolean,
        DataType::Int32 => ArrowType::Int32,
        DataType::Int64 => ArrowType::Int64,
        DataType::Float64 => ArrowType::Float64,
        DataType::Utf8 => ArrowType::Utf8,
    }
}

fn to_arrow(col: &ArrayRef) -> ArrowArrayRef {
    match col {
        ArrayRef::Boolean(a) => Arc::new(BooleanArray::from(a.0.clone())),
        ArrayRef::Int32(a) => Arc::new(Int32Array::from(a.0.clone())),
        ArrayRef::Int64(a) => Arc::new(Int64Array::from(a.0.clone())),
        ArrayRef::Float64(a) => Arc::new(Float64Array::from(a.0.clone())),
        ArrayRef::Utf8(a) => Arc::new(StringArray::from(a.0.clone())),
    }
}
