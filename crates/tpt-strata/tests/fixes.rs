use std::sync::Arc;
use tpt_strata::schema::{DataType, Field, Schema};
use tpt_strata::sql::SqlContext;
use tpt_strata::{AggFunc, Column, QueryBuilder, Scalar, Table};

fn ctx_with(table: Table) -> SqlContext<'static> {
    let table: &'static Table = Box::leak(Box::new(table));
    let mut ctx = SqlContext::new();
    ctx.add_table("t", table);
    ctx
}

fn sample() -> Table {
    Table::try_new(vec![
        Column::new(
            "dept",
            vec![
                Scalar::Str("a".into()),
                Scalar::Str("a".into()),
                Scalar::Str("b".into()),
                Scalar::Str("b".into()),
            ],
        ),
        Column::new(
            "v",
            vec![
                Scalar::F64(1.0),
                Scalar::F64(2.0),
                Scalar::F64(3.0),
                Scalar::Null,
            ],
        ),
    ])
    .unwrap()
}

#[test]
fn count_col_excludes_nulls() {
    let ctx = ctx_with(sample());
    // COUNT(v) over [1,2,3,NULL] must be 3; COUNT(*) must be 4.
    let out = ctx.run("SELECT COUNT(v) FROM t").unwrap();
    assert_eq!(out.batches()[0].columns[0].get(0), Some(Scalar::I64(3)));
    let out = ctx.run("SELECT COUNT(*) FROM t").unwrap();
    assert_eq!(out.batches()[0].columns[0].get(0), Some(Scalar::I64(4)));
    // Grouped: dept b has COUNT(v) 1 (row with 3.0; NULL row excluded).
    let out = ctx
        .run("SELECT dept, COUNT(v) FROM t GROUP BY dept ORDER BY dept")
        .unwrap();
    let b = &out.batches()[0];
    let mut b_count = None;
    for r in 0..b.row_count() {
        if b.columns[0].get(r) == Some(Scalar::Str("b".into())) {
            b_count = b.columns[1].get(r);
        }
    }
    assert_eq!(b_count, Some(Scalar::I64(1)));
}

#[test]
fn count_col_excludes_nulls_builder() {
    let table = sample();
    let out = QueryBuilder::new(&table)
        .aggregate("v", AggFunc::Count)
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(out.batches()[0].columns[0].get(0), Some(Scalar::I64(3)));
}

#[test]
fn sum_all_null_group_is_null() {
    let ctx = ctx_with(
        Table::try_new(vec![
            Column::new(
                "dept",
                vec![
                    Scalar::Str("a".into()),
                    Scalar::Str("a".into()),
                    Scalar::Str("b".into()),
                ],
            ),
            Column::new("v", vec![Scalar::F64(1.0), Scalar::Null, Scalar::Null]),
        ])
        .unwrap(),
    );
    let out = ctx
        .run("SELECT dept, SUM(v) FROM t GROUP BY dept ORDER BY dept")
        .unwrap();
    let b = &out.batches()[0];
    let mut b_sum = None;
    let mut a_sum = None;
    for r in 0..b.row_count() {
        match b.columns[0].get(r) {
            Some(Scalar::Str(s)) if s == "a" => a_sum = b.columns[1].get(r),
            Some(Scalar::Str(s)) if s == "b" => b_sum = b.columns[1].get(r),
            _ => {}
        }
    }
    assert_eq!(a_sum, Some(Scalar::F64(1.0)));
    assert_eq!(b_sum, None); // null SUM value -> no Scalar present
                             // Global SUM over an all-null column is NULL too.
    let nulls = ctx_with(Table::empty(Arc::new(Schema::new(vec![Field::new(
        "v",
        DataType::Float64,
        true,
    )]))));
    let out = nulls
        .run("SELECT SUM(v), COUNT(*), COUNT(v) FROM t")
        .unwrap();
    assert_eq!(out.batches()[0].columns[0].get(0), None);
    assert_eq!(out.batches()[0].columns[1].get(0), Some(Scalar::I64(0)));
    assert_eq!(out.batches()[0].columns[2].get(0), Some(Scalar::I64(0)));
}

#[test]
fn sum_of_string_column_rejected() {
    let ctx = ctx_with(sample());
    let err = ctx.run("SELECT SUM(dept) FROM t").unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("numeric") && text.contains("dept") && text.contains("str"),
        "unexpected diagnostic: {text}"
    );
    let table = sample();
    let builder_err = QueryBuilder::new(&table)
        .aggregate("dept", AggFunc::Sum)
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(builder_err.to_string().contains("numeric"));
}

#[test]
fn order_by_unknown_column_errors() {
    let ctx = ctx_with(
        Table::try_new(vec![
            Column::new(
                "service",
                vec![Scalar::Str("auth".into()), Scalar::Str("billing".into())],
            ),
            Column::new(
                "region",
                vec![Scalar::Str("east".into()), Scalar::Str("west".into())],
            ),
        ])
        .unwrap(),
    );
    // 's' must NOT substring-match 'service'; it is a missing column.
    let err = ctx.run("SELECT region FROM t ORDER BY s").unwrap_err();
    assert!(
        matches!(
            err,
            tpt_strata::QueryError::MissingColumn { ref available, .. }
                if available == &vec!["service".to_string(), "region".to_string()]
        ),
        "unexpected error: {err}"
    );
}

#[test]
fn join_excludes_null_keys() {
    let l = Table::try_new(vec![Column::new("id", vec![Scalar::I32(1), Scalar::Null])]).unwrap();
    let r = Table::try_new(vec![Column::new("id", vec![Scalar::I32(1), Scalar::Null])]).unwrap();
    let l: &'static Table = Box::leak(Box::new(l));
    let r: &'static Table = Box::leak(Box::new(r));
    let mut ctx = SqlContext::new();
    ctx.add_table("l", l);
    ctx.add_table("r", r);
    let out = ctx.run("SELECT * FROM l JOIN r ON l.id = r.id").unwrap();
    assert_eq!(out.row_count(), 1);
}

#[test]
fn mixed_type_column_rejected_by_try_new() {
    let err = Table::try_new(vec![Column::new(
        "x",
        vec![Scalar::I32(7), Scalar::Str("boom".into())],
    )])
    .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("x") && text.contains("i32") && text.contains("str"),
        "unexpected diagnostic: {text}"
    );
    // Nulls alongside a single non-null type are still fine.
    Table::try_new(vec![Column::new("x", vec![Scalar::I32(7), Scalar::Null])]).unwrap();
}
