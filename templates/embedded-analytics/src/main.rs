//! Starter: embed tpt-strata in an application that already holds its data.
//!
//! The pattern: keep your rows in your own structs, convert them to a `Table`
//! when a report is requested, and run SQL over it. Replace `Order` and the
//! sample rows with your own types.

use tpt_strata::sql::SqlContext;
use tpt_strata::{Scalar, Table};

/// Your domain type. Keep it however your application needs.
struct Order {
    region: String,
    quantity: i64,
    unit_price: f64,
}

/// Convert your rows into a `Table`. This is the only place that knows the
/// column names, so queries elsewhere stay readable.
fn orders_table(rows: &[Order]) -> Result<Table, tpt_strata::QueryError> {
    let data = rows
        .iter()
        .map(|o| {
            vec![
                Scalar::Str(o.region.clone()),
                Scalar::I64(o.quantity),
                Scalar::F64(o.unit_price),
            ]
        })
        .collect();
    Table::from_rows(&["region", "quantity", "unit_price"], data)
}

/// A report is just a query. `check` at startup (or in CI) so a typo fails
/// fast rather than on the first request.
const REVENUE_BY_REGION: &str = "\
    SELECT region, SUM(quantity) AS units \
    FROM orders WHERE unit_price > 0 \
    GROUP BY region HAVING SUM(quantity) > 0 \
    ORDER BY units DESC";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows = vec![
        Order { region: "north".into(), quantity: 4, unit_price: 2.5 },
        Order { region: "south".into(), quantity: 1, unit_price: 19.99 },
        Order { region: "north".into(), quantity: 6, unit_price: 2.5 },
    ];

    let orders = orders_table(&rows)?;
    let mut ctx = SqlContext::new();
    ctx.add_table("orders", &orders);

    ctx.check(REVENUE_BY_REGION)?;
    let report = ctx.run(REVENUE_BY_REGION)?;
    tpt_strata::format::print_table(&report);
    Ok(())
}
