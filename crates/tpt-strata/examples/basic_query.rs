//! Builder API end-to-end: filter, group by, aggregate, order, limit.
//!
//! Run with `cargo run -p tpt-strata --example basic_query`. No trait
//! implementations are required for the common case.

use tpt_strata::{print_table, AggFunc, Column, QueryBuilder, Scalar, Table};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Point at data: three columns as plain Rust values.
    let metrics = Table::try_new(vec![
        Column::new(
            "service",
            vec![
                Scalar::Str("auth".into()),
                Scalar::Str("auth".into()),
                Scalar::Str("billing".into()),
                Scalar::Str("billing".into()),
            ],
        ),
        Column::new(
            "region",
            vec![
                Scalar::Str("east".into()),
                Scalar::Str("west".into()),
                Scalar::Str("east".into()),
                Scalar::Str("west".into()),
            ],
        ),
        Column::new(
            "latency_ms",
            vec![
                Scalar::F64(12.5),
                Scalar::F64(18.0),
                Scalar::F64(9.9),
                Scalar::Null,
            ],
        ),
    ])?;

    // Get an answer: average latency per service, east region only, sorted.
    let result = QueryBuilder::new(&metrics)
        .filter("region", "=", Scalar::Str("east".into()))?
        .group_by(vec!["service"])?
        .aggregate("latency_ms", AggFunc::Avg)?
        .order_by("service", false)?
        .execute()?;

    print_table(&result);
    Ok(())
}
