use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{BooleanArray, Date32Array, Float64Array, Int32Array, StringArray};
use arrow::datatypes::{DataType as AType, Field as AField, Schema as ASchema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;

use tpt_strata::sql::SqlContext;
use tpt_strata::{DataType, Scalar};

fn write_parquet(path: &Path, schema: ASchema, batches: Vec<RecordBatch>) {
    let file = File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, Arc::new(schema), None).unwrap();
    for batch in batches {
        writer.write(&batch).unwrap();
    }
    writer.close().unwrap();
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("tpt-strata-test").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn emp_schema() -> ASchema {
    ASchema::new(vec![
        AField::new("department", AType::Utf8, false),
        AField::new("salary", AType::Float64, false),
        AField::new("years", AType::Int32, true),
        AField::new("active", AType::Boolean, false),
    ])
}

fn emp_batch() -> RecordBatch {
    RecordBatch::try_new(
        Arc::new(emp_schema()),
        vec![
            Arc::new(StringArray::from(vec!["eng", "eng", "sales", "sales"])),
            Arc::new(Float64Array::from(vec![100.0, 120.0, 80.0, 90.0])),
            Arc::new(Int32Array::from(vec![Some(5), None, Some(3), Some(6)])),
            Arc::new(BooleanArray::from(vec![true, false, true, true])),
        ],
    )
    .unwrap()
}

#[test]
fn reads_typed_columns_with_nulls() {
    let path = fixture_dir("basic").join("employees.parquet");
    write_parquet(&path, emp_schema(), vec![emp_batch()]);

    let table = tpt_strata_parquet::read_parquet(&path).unwrap();
    assert_eq!(table.row_count(), 4);
    assert_eq!(
        table.column_names(),
        vec!["department", "salary", "years", "active"]
    );
    assert_eq!(table.schema().field(0).unwrap().data_type, DataType::Utf8);
    assert_eq!(table.schema().field(2).unwrap().data_type, DataType::Int32);
    assert!(table.schema().field(2).unwrap().nullable);
    assert!(!table.schema().field(0).unwrap().nullable);

    let batch = &table.batches()[0];
    // years column has a null at row 1
    assert_eq!(batch.columns[2].get(1), None);
    assert_eq!(batch.columns[2].get(0), Some(Scalar::I32(5)));
    assert_eq!(batch.columns[0].get(3), Some(Scalar::Str("sales".into())));
    assert_eq!(batch.columns[3].get(0), Some(Scalar::Bool(true)));
}

#[test]
fn imported_table_is_queryable() {
    let path = fixture_dir("query").join("employees.parquet");
    write_parquet(&path, emp_schema(), vec![emp_batch()]);

    let table = tpt_strata_parquet::read_parquet(&path).unwrap();

    let mut ctx = SqlContext::new();
    let table_ref = Box::leak(Box::new(table));
    ctx.add_table("employees", table_ref);

    let out = ctx
        .run("SELECT department, SUM(salary) FROM employees WHERE active = TRUE GROUP BY department ORDER BY department")
        .unwrap();
    assert_eq!(out.row_count(), 2);
    // active = TRUE keeps eng(100), sales(80), sales(90); eng(120) filtered out
    let mut sums = std::collections::HashMap::new();
    for i in 0..out.row_count() {
        let b = &out.batches()[0];
        sums.insert(b.columns[0].get(i).unwrap(), b.columns[1].get(i).unwrap());
    }
    assert_eq!(sums[&Scalar::Str("eng".into())], Scalar::F64(100.0));
    assert_eq!(sums[&Scalar::Str("sales".into())], Scalar::F64(170.0));
}

#[test]
fn rejects_unsupported_type_with_diagnostic() {
    let schema = ASchema::new(vec![
        AField::new("id", AType::Int32, false),
        AField::new("hired", AType::Date32, true),
    ]);
    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(Int32Array::from(vec![1, 2])),
            Arc::new(Date32Array::from(vec![100, 200])),
        ],
    )
    .unwrap();
    let path = fixture_dir("unsupported").join("dates.parquet");
    write_parquet(&path, schema, vec![batch]);

    let err = tpt_strata_parquet::read_parquet(&path).unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("unsupported Parquet type for column 'hired'"),
        "{text}"
    );
    assert!(
        text.contains("Supported: BOOLEAN, INT32, INT64, DOUBLE, STRING"),
        "{text}"
    );
}

#[test]
fn errors_on_missing_file() {
    let err = tpt_strata_parquet::read_parquet("does-not-exist.parquet").unwrap_err();
    assert!(err.to_string().contains("cannot open"));
}

#[test]
fn reads_zero_row_file() {
    let schema = ASchema::new(vec![AField::new("x", AType::Int32, true)]);
    let empty_batch = RecordBatch::new_empty(Arc::new(schema.clone()));
    let path = fixture_dir("empty").join("empty.parquet");
    write_parquet(&path, schema, vec![empty_batch]);

    let table = tpt_strata_parquet::read_parquet(&path).unwrap();
    assert_eq!(table.row_count(), 0);
    assert_eq!(table.column_names(), vec!["x"]);
}

#[test]
fn reads_multiple_row_groups() {
    let path = fixture_dir("multi").join("multi.parquet");
    // Two batches -> two row groups.
    write_parquet(&path, emp_schema(), vec![emp_batch(), emp_batch()]);

    let table = tpt_strata_parquet::read_parquet(&path).unwrap();
    assert_eq!(table.row_count(), 8);
    let depts: Vec<Scalar> = (0..table.row_count())
        .map(|i| table.batches()[0].columns[0].get(i).unwrap())
        .collect();
    assert!(depts
        .iter()
        .all(|d| d == &Scalar::Str("eng".into()) || d == &Scalar::Str("sales".into())));
}
