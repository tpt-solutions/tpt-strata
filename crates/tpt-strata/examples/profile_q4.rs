//! Runs the Q4 query (one group per id, HAVING keeps none) many times, so a
//! sampling profiler has enough samples to read. Build with the `parallel`
//! feature to match the benchmark:
//!
//! ```text
//! cargo build --release -p tpt-strata --features parallel --example profile_q4
//! samply record -o q4.json.gz -- target\release\examples\profile_q4.exe 300
//! ```
//!
//! The argument is the number of timed runs (default 300).

use std::time::Instant;
use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

fn main() {
    let runs: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let n = 1_000_000usize;
    let facts = Table::try_new(vec![
        Column::new("id", (0..n).map(|i| Scalar::I64(i as i64)).collect()),
        Column::new(
            "quantity",
            (0..n).map(|i| Scalar::I64((i % 97) as i64)).collect(),
        ),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    let sql = "SELECT id, SUM(quantity) AS units FROM facts GROUP BY id \
               HAVING SUM(quantity) > 1000";
    ctx.run(sql).unwrap();

    let start = Instant::now();
    for _ in 0..runs {
        let out = ctx.run(sql).unwrap();
        assert_eq!(out.row_count(), 0);
    }
    let per_run = start.elapsed().as_secs_f64() * 1e3 / runs as f64;
    println!("{runs} runs, {per_run:.2} ms per run");
}
