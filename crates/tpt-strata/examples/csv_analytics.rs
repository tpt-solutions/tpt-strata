//! CSV to report in a few lines: parse a file, aggregate it with SQL, and
//! inspect the plan. Run with:
//!
//!     cargo run -p tpt-strata --example csv_analytics

use tpt_strata::csv::parse_csv;
use tpt_strata::format::print_table;
use tpt_strata::sql::SqlContext;

const ORDERS: &str = include_str!("data/orders.csv");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Column types are inferred: order_id, quantity and customer_id are i64,
    // unit_price is f64, and region is str. Empty fields are NULL.
    let orders = parse_csv(ORDERS)?;
    println!("Loaded {} orders", orders.row_count());

    let mut ctx = SqlContext::new();
    ctx.add_table("orders", &orders);

    // Revenue per region. Rows with a NULL quantity still count as orders but
    // contribute nothing to SUM, which skips NULLs.
    let report = ctx.run(
        "SELECT region, COUNT(*) AS orders, SUM(quantity) AS units \
         FROM orders GROUP BY region ORDER BY units DESC",
    )?;
    print_table(&report);

    // Ask the engine how it will run a query, without running it.
    println!("\nPlan for a top-products query:");
    print!(
        "{}",
        ctx.explain(
            "SELECT product, SUM(quantity) AS units FROM orders \
             WHERE unit_price > 5 GROUP BY product ORDER BY units DESC LIMIT 2"
        )?
    );
    Ok(())
}
