//! Splits Q5's cost: the same table, four queries, tpt-strata only.
//! Run with: `cargo run --release --features parallel --bin q5_split`

use std::time::Instant;
use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

const N: usize = 1_000_000;
const REGIONS: [&str; 8] = [
    "north", "central", "coastal", "south", "east", "west", "cabin", "delta",
];

fn main() {
    let facts = Table::try_new(vec![
        Column::new("id", (0..N).map(|i| Scalar::I64(i as i64)).collect()),
        Column::new("quantity", (0..N).map(|i| Scalar::I64((i % 97) as i64)).collect()),
        Column::new(
            "region",
            (0..N)
                .map(|i| Scalar::Str(REGIONS[(i * 7 + i / 13) % REGIONS.len()].into()))
                .collect(),
        ),
    ])
    .unwrap();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    let queries = [
        ("scan only: COUNT(*)", "SELECT COUNT(*) AS n FROM facts"),
        ("no filter: SUM(quantity)", "SELECT SUM(quantity) AS units FROM facts"),
        ("int filter only: COUNT(*)", "SELECT COUNT(*) AS n FROM facts WHERE quantity > 50"),
        ("int filter + SUM", "SELECT SUM(quantity) AS units FROM facts WHERE quantity > 50"),
        ("LIKE filter: COUNT(*)", "SELECT COUNT(*) AS n FROM facts WHERE region LIKE 'c%'"),
        ("LIKE filter + SUM", "SELECT SUM(quantity) AS units FROM facts WHERE region LIKE 'c%'"),
        ("int filter + COUNT and SUM", "SELECT COUNT(*) AS n, SUM(quantity) AS units FROM facts WHERE quantity > 50"),
        ("Q5: LIKE filter, COUNT and SUM", "SELECT COUNT(*) AS n, SUM(quantity) AS units FROM facts WHERE region LIKE 'c%'"),
    ];
    for (label, sql) in queries {
        ctx.run(sql).unwrap();
        let mut times = Vec::new();
        for _ in 0..100 {
            let t = Instant::now();
            ctx.run(sql).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("{label}: median {:.3} ms", times[times.len() / 2]);
    }
}
