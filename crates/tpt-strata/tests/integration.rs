//! End-to-end integration scenarios from the v1 scope (spec.txt, Section 4):
//! a realistic analytical pipeline spanning the builder API, the SQL surface,
//! and the streaming (multi-batch) engine path.

use std::collections::HashMap;

use std::sync::Arc;
use tpt_strata::sql::SqlContext;
use tpt_strata::{
    join, AggFunc, Batch, Column, DataType, Field, QueryBuilder, Scalar, Schema, Table,
};

/// A multi-batch "metrics" table modeled after the observability consumer.
fn metrics_table(batch_count: usize) -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new("ts_ms", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("service", DataType::Utf8, false),
        Field::new("latency_ms", DataType::Float64, true),
        Field::new("ok", DataType::Boolean, false),
    ]));

    let mut batches = Vec::new();
    for _ in 0..batch_count {
        let columns = vec![
            tpt_strata::I64Array::new(vec![100, 101, 102, 103]).into(),
            tpt_strata::StrArray::new(vec![
                "us-east".into(),
                "us-east".into(),
                "eu-west".into(),
                "eu-west".into(),
            ])
            .into(),
            tpt_strata::StrArray::new(vec![
                "auth".into(),
                "billing".into(),
                "auth".into(),
                "search".into(),
            ])
            .into(),
            tpt_strata::F64Array::from_nulls(vec![Some(5.0), Some(12.0), Some(9.0), Some(3.0)])
                .into(),
            tpt_strata::BoolArray::new(vec![true, false, true, true]).into(),
        ];
        batches.push(Batch::new_unchecked(schema.clone(), columns));
    }
    Table::from_batches(schema, batches)
}

/// SQL over multi-batch data: filter + group-by + sort + limit.
#[test]
fn sql_query_over_streaming_batches() {
    let table = metrics_table(3);
    assert_eq!(table.row_count(), 12);

    let table_ref = Box::leak(Box::new(table));
    let mut ctx = SqlContext::new();
    ctx.add_table("metrics", table_ref);

    // ok = TRUE leaves auth and search rows across all three batches.
    // auth: (5+9)*3/6 = 7.0; search: 3.0. Sorted by avg desc -> auth, search.
    let out = ctx
        .run(
            "SELECT service, AVG(latency_ms) \
             FROM metrics WHERE ok = TRUE \
             GROUP BY service ORDER BY AVG(latency_ms) DESC LIMIT 2",
        )
        .unwrap();

    assert_eq!(out.row_count(), 2);
    let b = &out.batches()[0];
    let values: Vec<Scalar> = (0..b.row_count())
        .map(|r| b.columns[0].get(r).unwrap())
        .collect();
    assert_eq!(
        values,
        vec![Scalar::Str("auth".into()), Scalar::Str("search".into())]
    );
    let avgs: Vec<Scalar> = (0..b.row_count())
        .map(|r| b.columns[1].get(r).unwrap())
        .collect();
    assert_eq!(avgs, vec![Scalar::F64(7.0), Scalar::F64(3.0)]);
}

/// The same pipeline expressed through the builder API must match the SQL
/// result.
#[test]
fn builder_matches_sql_end_to_end() {
    let table = metrics_table(1);
    let table_ref = Box::leak(Box::new(table));

    let sql_out = {
        let mut ctx = SqlContext::new();
        ctx.add_table("metrics", table_ref);
        ctx.run(
            "SELECT service, SUM(latency_ms) AS sum FROM metrics GROUP BY service ORDER BY service",
        )
        .unwrap()
    };

    let builder_out = QueryBuilder::new(table_ref)
        .group_by(vec!["service"])
        .unwrap()
        .aggregate("latency_ms", AggFunc::Sum)
        .unwrap()
        .order_by("service", false)
        .unwrap()
        .execute()
        .unwrap();

    let rows = |t: &Table| {
        let b = &t.batches()[0];
        let mut map = HashMap::new();
        for r in 0..t.row_count() {
            map.insert(b.columns[0].get(r).unwrap(), b.columns[1].get(r).unwrap());
        }
        map
    };
    assert_eq!(rows(&sql_out), rows(&builder_out));
    assert_eq!(
        rows(&sql_out)[&Scalar::Str("auth".into())],
        Scalar::F64(14.0)
    );
    assert_eq!(
        rows(&sql_out)[&Scalar::Str("billing".into())],
        Scalar::F64(12.0)
    );
    assert_eq!(
        rows(&sql_out)[&Scalar::Str("search".into())],
        Scalar::F64(3.0)
    );
}

/// SQL JOIN and the builder `join()` free function over the same data agree.
#[test]
fn sql_join_matches_builder_join() {
    let regions = Table::try_new(vec![
        Column::new(
            "code",
            vec![Scalar::Str("us-east".into()), Scalar::Str("eu-west".into())],
        ),
        Column::new(
            "tier",
            vec![Scalar::Str("prod".into()), Scalar::Str("prod".into())],
        ),
    ])
    .unwrap();
    let regions_ref = Box::leak(Box::new(regions));

    let metrics = metrics_table(1);
    let metrics_ref = Box::leak(Box::new(metrics));

    let sql_out = {
        let mut ctx = SqlContext::new();
        ctx.add_table("regions", regions_ref);
        ctx.add_table("metrics", metrics_ref);
        ctx.run(
            "SELECT metrics.service, regions.tier \
             FROM metrics JOIN regions ON metrics.region = regions.code \
             WHERE metrics.ok = TRUE",
        )
        .unwrap()
    };

    let wide = join(metrics_ref, regions_ref, "region", "code").unwrap();
    let builder_out = QueryBuilder::new(&wide)
        .project(vec!["service", "tier"])
        .unwrap()
        .filter("ok", "=", Scalar::Bool(true))
        .unwrap()
        .execute()
        .unwrap();

    let sql_names: Vec<Scalar> = sql_out
        .batches()
        .first()
        .map(|b| {
            (0..b.row_count())
                .map(|r| b.columns[0].get(r).unwrap())
                .collect()
        })
        .unwrap_or_default();
    let builder_names: Vec<Scalar> = builder_out
        .batches()
        .first()
        .map(|b| {
            (0..b.row_count())
                .map(|r| b.columns[0].get(r).unwrap())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(sql_names, builder_names);
    assert_eq!(builder_names.len(), 3);
}

/// A full observability-style pipeline: ingest -> filter -> aggregate ->
/// project only what a dashboard needs.
#[test]
fn observability_pipeline_ingest_to_dashboard() {
    let table = metrics_table(1);
    let table_ref = Box::leak(Box::new(table));

    let mut ctx = SqlContext::new();
    ctx.add_table("metrics", table_ref);

    let out = ctx
        .run(
            "SELECT region, service, COUNT(*) AS calls, MAX(latency_ms) AS p_max \
             FROM metrics WHERE ok = TRUE AND region <> 'eu-west' \
             GROUP BY region, service",
        )
        .unwrap();

    // ok=TRUE, region us-east only: rows 0 (auth, 5.0) and 1 (billing, 12.0, ok=false).
    // So just auth: 1 call, max 5.0.
    assert_eq!(
        out.column_names(),
        vec!["region", "service", "COUNT(*)", "MAX(latency_ms)"]
    );
    let b = &out.batches()[0];
    assert_eq!(b.row_count(), 1);
    assert_eq!(b.columns[0].get(0), Some(Scalar::Str("us-east".into())));
    assert_eq!(b.columns[1].get(0), Some(Scalar::Str("auth".into())));
    assert_eq!(b.columns[2].get(0), Some(Scalar::I64(1)));
    assert_eq!(b.columns[3].get(0), Some(Scalar::F64(5.0)));
}

/// Diagnostics surface the caller's own schema on both API surfaces in one
/// end-to-end scenario (missing column + type mismatch).
#[test]
fn end_to_end_diagnostics_are_caller_facing() {
    let table = metrics_table(1);
    let table_ref = Box::leak(Box::new(table));

    let mut ctx = SqlContext::new();
    ctx.add_table("metrics", table_ref);

    let missing = ctx.run("SELECT nope FROM metrics").unwrap_err().to_string();
    assert!(missing.contains("No column named 'nope'"), "{missing}");
    assert!(
        missing.contains("Available columns: ts_ms, region, service, latency_ms, ok"),
        "{missing}"
    );

    let mismatch = ctx
        .run("SELECT * FROM metrics WHERE latency_ms > 'fast'")
        .unwrap_err()
        .to_string();
    assert!(
        mismatch.contains("Type mismatch in 'latency_ms'"),
        "{mismatch}"
    );
    assert!(mismatch.contains("expected f64, found str"), "{mismatch}");

    // Builder surface reports the same style of diagnostics.
    let builder_err = QueryBuilder::new(table_ref)
        .filter("ok", "=", Scalar::Str("yes".into()))
        .unwrap_err()
        .to_string();
    assert!(
        builder_err.contains("Type mismatch in 'ok': expected bool, found str"),
        "{builder_err}"
    );
}

/// Global aggregation (no GROUP BY) over the whole table, including COUNT(*).
#[test]
fn end_to_end_global_aggregates() {
    let table = metrics_table(3);
    let table_ref = Box::leak(Box::new(table));
    let mut ctx = SqlContext::new();
    ctx.add_table("metrics", table_ref);

    let out = ctx
        .run("SELECT COUNT(*), SUM(latency_ms) FROM metrics")
        .unwrap();
    let b = &out.batches()[0];
    assert_eq!(b.row_count(), 1);
    assert_eq!(b.columns[0].get(0), Some(Scalar::I64(12)));
    assert_eq!(b.columns[1].get(0), Some(Scalar::F64(29.0 * 3.0)));
}
