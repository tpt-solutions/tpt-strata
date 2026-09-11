//! SQL surface end-to-end via `SqlContext`, including a join.
//!
//! Run with `cargo run -p tpt-strata --example sql_query`. The SQL string is
//! parsed and executed inside the core crate (zero external dependencies).

use tpt_strata::{print_table, sql::SqlContext, Column, Scalar, Table};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sales = Table::try_new(vec![
        Column::new(
            "region",
            vec![
                Scalar::Str("east".into()),
                Scalar::Str("east".into()),
                Scalar::Str("west".into()),
            ],
        ),
        Column::new(
            "amount",
            vec![Scalar::F64(100.0), Scalar::F64(49.0), Scalar::F64(60.0)],
        ),
    ])?;
    let regions = Table::try_new(vec![
        Column::new(
            "rname",
            vec![Scalar::Str("east".into()), Scalar::Str("west".into())],
        ),
        Column::new(
            "country",
            vec![Scalar::Str("us".into()), Scalar::Str("us".into())],
        ),
    ])?;

    let mut ctx = SqlContext::new();
    ctx.add_table("sales", &sales);
    ctx.add_table("regions", &regions);

    // Aggregate by region over joined, filtered sales.
    let result = ctx.run(
        "SELECT sales.region, SUM(sales.amount) \
         FROM sales JOIN regions ON sales.region = regions.rname \
         WHERE sales.amount >= 50 \
         GROUP BY sales.region ORDER BY region",
    )?;

    print_table(&result);
    Ok(())
}
