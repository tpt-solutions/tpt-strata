//! Runs one benchmark query many times, so a sampling profiler has enough
//! samples to read. Build with the `parallel` feature to match the benchmark:
//!
//! ```text
//! cargo build --release -p tpt-strata --features parallel --example profile_q4
//! samply record --save-only -o q4.json.gz -- target/release/examples/profile_q4 300
//! samply record --save-only -o q5.json.gz -- target/release/examples/profile_q4 300 q5
//! samply record --save-only -o q3.json.gz -- target/release/examples/profile_q4 300 q3
//! ```
//!
//! Arguments: the number of timed runs (default 300), then the query: `q4`
//! (default, GROUP BY id with HAVING), `q5` (LIKE filter with COUNT and SUM), or
//! `q3` (join to a 1000-row dimension, then GROUP BY).

use std::sync::Arc;
use std::time::Instant;
use tpt_strata::array::{ArrayRef, I64Array, StrArray};
use tpt_strata::batch::Batch;
use tpt_strata::schema::{DataType, Field, Schema};
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

    // The tables outlive the context that borrows them.
    let facts = match which.as_str() {
        "q3" => Table::try_new(vec![
            Column::new("id", (0..n).map(|i| Scalar::I64(i as i64)).collect()),
            Column::new(
                "quantity",
                (0..n).map(|i| Scalar::I64((i % 97) as i64)).collect(),
            ),
            Column::new(
                "dim_id",
                (0..n)
                    .map(|i| Scalar::I64(((i * 7) % 1000) as i64))
                    .collect(),
            ),
        ])
        .unwrap(),
        "q2" => Table::try_new(vec![
            Column::new("id", (0..n).map(|i| Scalar::I64(i as i64)).collect()),
            Column::new(
                "region",
                (0..n)
                    .map(|i| Scalar::Str(REGIONS[(i * 7 + i / 13) % REGIONS.len()].into()))
                    .collect(),
            ),
            Column::new(
                "quantity",
                (0..n).map(|i| Scalar::I64((i % 97) as i64)).collect(),
            ),
            Column::new(
                "price",
                (0..n)
                    .map(|i| Scalar::F64(((i as u64 * 2_654_435_761) % 1_000_000) as f64 / 100.0))
                    .collect(),
            ),
        ])
        .unwrap(),
        "q5" | "sumf" | "sumall" => Table::try_new(vec![
            Column::new("id", (0..n).map(|i| Scalar::I64(i as i64)).collect()),
            Column::new(
                "quantity",
                (0..n).map(|i| Scalar::I64((i % 97) as i64)).collect(),
            ),
            Column::new(
                "region",
                (0..n)
                    .map(|i| Scalar::Str(REGIONS[(i * 7 + i / 13) % REGIONS.len()].into()))
                    .collect(),
            ),
        ])
        .unwrap(),
        _ => Table::try_new(vec![
            Column::new("id", (0..n).map(|i| Scalar::I64(i as i64)).collect()),
            Column::new(
                "quantity",
                (0..n).map(|i| Scalar::I64((i % 97) as i64)).collect(),
            ),
        ])
        .unwrap(),
    };
    let dims = dims_table();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    if which == "q3" {
        ctx.add_table("dims", &dims);
    }
    let (sql, rows) = match which.as_str() {
        "q3" => (
            "SELECT dims.label, SUM(facts.quantity) AS units FROM facts \
             JOIN dims ON facts.dim_id = dims.id GROUP BY dims.label ORDER BY label",
            20,
        ),
        "q2" => (
            "SELECT id, region, price FROM facts WHERE quantity > 50 AND price < 40.0 \
             ORDER BY price DESC LIMIT 100",
            100,
        ),
        "sumall" => ("SELECT SUM(quantity) AS units FROM facts", 1),
        "sumf" => (
            "SELECT SUM(quantity) AS units FROM facts WHERE quantity > 50",
            1,
        ),
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

/// The 1000-row dimension of the benchmark: `id` and a 20-value `label`.
fn dims_table() -> Table {
    let ids: Vec<i64> = (0..1000).collect();
    let labels: Vec<Arc<str>> = ids
        .iter()
        .map(|i| Arc::from(format!("label-{:02}", i % 20)))
        .collect();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let cols = vec![
        ArrayRef::Int64(I64Array::new(ids)),
        ArrayRef::Utf8(StrArray::new(labels)),
    ];
    Table::from_batches(schema.clone(), vec![Batch::new_unchecked(schema, cols)])
}
