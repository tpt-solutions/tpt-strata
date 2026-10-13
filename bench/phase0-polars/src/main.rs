//! Phase 0 comparison: the "point at data, get an answer" case on Polars.
//!
//! Same data and the same broken queries as the DataFusion and tpt-strata
//! sketches recorded in validation-memo.md.

use polars::prelude::*;
use polars::sql::SQLContext;

fn main() -> PolarsResult<()> {
    let df = df! {
        "department" => ["eng", "eng", "sales", "sales", "hr"],
        "salary" => [120000.0, 110000.0, 90000.0, 95000.0, 70000.0],
        "years" => [5i32, 3, 7, 2, 10],
    }?;

    let mut ctx = SQLContext::new();
    ctx.register("employees", df.lazy());

    let basic = ctx.execute(
        "SELECT department, SUM(salary) AS total FROM employees GROUP BY department ORDER BY department",
    )?;
    println!("basic query ok:\n{}", basic.collect()?);

    for (label, sql) in [
        ("missing column", "SELECT nonexistent FROM employees"),
        ("type mismatch", "SELECT department FROM employees WHERE department > 42"),
        ("unsupported cast", "SELECT department::INT FROM employees"),
    ] {
        match ctx.execute(sql).and_then(|lf| lf.collect()) {
            Ok(out) => println!("{label}: unexpectedly ok:\n{out}"),
            Err(e) => println!("{label}: {e}"),
        }
    }
    Ok(())
}
