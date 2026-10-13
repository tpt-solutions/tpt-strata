//! Joins, aliases, and the newer clauses on two CSV files. Run with:
//!
//!     cargo run -p tpt-strata --example joins_and_aliases

use tpt_strata::csv::parse_csv;
use tpt_strata::format::print_table;
use tpt_strata::sql::SqlContext;

const ORDERS: &str = include_str!("data/orders.csv");
const CUSTOMERS: &str = include_str!("data/customers.csv");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let orders = parse_csv(ORDERS)?;
    let customers = parse_csv(CUSTOMERS)?;

    let mut ctx = SqlContext::new();
    ctx.add_table("orders", &orders);
    ctx.add_table("customers", &customers);

    // Both tables have a `customer_id` column. Qualify it to say which one you
    // mean, and the output names stay clean. The ON clause may name either side
    // first.
    println!("Units per customer, filtered with HAVING:");
    let spend = ctx.run(
        "SELECT customers.name, customers.tier, SUM(orders.quantity) AS units \
         FROM orders JOIN customers ON customers.customer_id = orders.customer_id \
         GROUP BY customers.name, customers.tier \
         HAVING SUM(orders.quantity) >= 5 ORDER BY units DESC",
    )?;
    print_table(&spend);

    println!("\nUnits per customer, with a range filter and paging:");
    let page = ctx.run(
        "SELECT customers.name, SUM(orders.quantity) AS units \
         FROM orders JOIN customers ON orders.customer_id = customers.customer_id \
         WHERE orders.unit_price BETWEEN 2 AND 10 \
         GROUP BY customers.name ORDER BY units DESC LIMIT 2 OFFSET 1",
    )?;
    print_table(&page);

    println!("\nDistinct products, sorted by two keys:");
    let products = ctx.run(
        "SELECT DISTINCT product, region FROM orders ORDER BY product ASC, region DESC LIMIT 4",
    )?;
    print_table(&products);

    // `check` binds the SQL against the table schemas without running it, so
    // a typo is caught before any data is read. Use it in CI.
    for sql in [
        "SELECT product FROM orders WHERE quantity > 1",
        "SELECT prodcut FROM orders",
    ] {
        match ctx.check(sql) {
            Ok(()) => println!("ok:    {sql}"),
            Err(e) => println!("error: {sql}\n       {e}"),
        }
    }
    Ok(())
}
