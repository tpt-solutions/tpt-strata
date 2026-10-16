//! Runs one benchmark query many times, so a sampling profiler has enough
//! samples to read. Build with the `parallel` feature to match the benchmark:
//!
//! ```text
//! cargo build --release -p tpt-strata --features parallel --example profile_q4
//! samply record --save-only -o q4.json.gz -- target/release/examples/profile_q4 300
//! samply record --save-only -o q5.json.gz -- target/release/examples/profile_q4 300 q5
//! ```
//!
//! Arguments: the number of timed runs (default 300), then the query: `q4`
//! (default, GROUP BY id with HAVING) or `q5` (LIKE filter with COUNT and SUM).

use std::time::Instant;
use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

const REGIONS: [&str; 8] = [
    "north", "central", "coastal", "south", "east", "west", "cabin", "delta",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let runs: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(300);
    let which = args.next().unwrap_or_else(|| "q4".to_string());
    let n = 1_000_000usize;

    let mut columns = vec![
        Column::new("id", (0..n).map(|i| Scalar::I64(i as i64)).collect()),
        Column::new(
            "quantity",
            (0..n).map(|i| Scalar::I64((i % 97) as i64)).collect(),
        ),
    ];
    if which == "q5" {
        columns.push(Column::new(
            "region",
            (0..n)
                .map(|i| Scalar::Str(REGIONS[(i * 7 + i / 13) % REGIONS.len()].into()))
                .collect(),
        ));
    }
    let facts = Table::try_new(columns).unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);

    let (sql, rows) = match which.as_str() {
        "q5" => (
            "SELECT COUNT(*) AS n, SUM(quantity) AS units FROM facts WHERE region LIKE 'c%'",
            1,
        ),
        _ => (
            "SELECT id, SUM(quantity) AS units FROM facts GROUP BY id \
             HAVING SUM(quantity) > 1000",
            0,
        ),
    };
    ctx.run(sql).unwrap();

    let start = Instant::now();
    for _ in 0..runs {
        let out = ctx.run(sql).unwrap();
        assert_eq!(out.row_count(), rows);
    }
    let per_run = start.elapsed().as_secs_f64() * 1e3 / runs as f64;
    println!("{which}: {runs} runs, {per_run:.2} ms per run");
}
