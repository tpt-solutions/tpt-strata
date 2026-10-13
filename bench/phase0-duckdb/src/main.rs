//! Phase 0 comparison: the "point at data, get an answer" case on DuckDB.
//!
//! Same data and the same broken queries as the DataFusion, Polars and
//! tpt-strata sketches recorded in validation-memo.md.

use duckdb::Connection;

fn main() -> duckdb::Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE employees (department VARCHAR, salary DOUBLE, years INTEGER);
         INSERT INTO employees VALUES
           ('eng', 120000, 5), ('eng', 110000, 3), ('sales', 90000, 7),
           ('sales', 95000, 2), ('hr', 70000, 10);",
    )?;

    let mut stmt = conn.prepare(
        "SELECT department, SUM(salary) AS total FROM employees GROUP BY department ORDER BY department",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
    println!("basic query ok:");
    for row in rows {
        let (dept, total) = row?;
        println!("  {dept}: {total}");
    }

    for (label, sql) in [
        ("missing column", "SELECT nonexistent FROM employees"),
        ("type mismatch", "SELECT department FROM employees WHERE department > 42"),
        ("unsupported cast", "SELECT department::INT FROM employees"),
    ] {
        match conn.prepare(sql).and_then(|mut s| s.query([]).map(|_| ())) {
            Ok(()) => println!("{label}: unexpectedly ok"),
            Err(e) => println!("{label}: {e}"),
        }
    }
    Ok(())
}
