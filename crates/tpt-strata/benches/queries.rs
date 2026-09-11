//! Zero-dependency benchmarks for tpt-strata.
//!
//! These mirror the Phase 0 workloads at scale so the numbers can be compared
//! against the incumbent measurements recorded in validation-memo.md (see
//! docs/benchmarks.md). Runs via the std test harness (`harness = false`):
//!
//! ```sh
//! cargo bench -p tpt-strata
//! ```

use std::hint::black_box;
use std::time::Instant;

use tpt_strata::sql::SqlContext;
use tpt_strata::{AggFunc, Column, QueryBuilder, Scalar, Table};

const REGIONS: [&str; 5] = ["us-east", "us-west", "eu-west", "eu-central", "ap-south"];
const SERVICES: [&str; 4] = ["auth", "billing", "search", "gateway"];

fn metrics_table(rows: usize) -> Table {
    let region = (0..rows)
        .map(|i| Scalar::Str(REGIONS[i % REGIONS.len()].to_string()))
        .collect();
    let service = (0..rows)
        .map(|i| Scalar::Str(SERVICES[i % SERVICES.len()].to_string()))
        .collect();
    let latency = (0..rows)
        .map(|i| Scalar::F64((i as f64 % 50.0) + 1.0))
        .collect();
    let ok = (0..rows).map(|i| Scalar::Bool(i % 10 != 0)).collect();
    Table::try_new(vec![
        Column::new("region", region),
        Column::new("service", service),
        Column::new("latency", latency),
        Column::new("ok", ok),
    ])
    .unwrap()
}

fn bench_group_by_sum(table: &Table, samples: usize) {
    for _ in 0..samples {
        let start = Instant::now();
        let out = QueryBuilder::new(table)
            .group_by(vec!["region"])
            .unwrap()
            .aggregate("latency", AggFunc::Sum)
            .unwrap()
            .execute()
            .unwrap();
        let elapsed = start.elapsed();
        black_box(out);
        report("group_by_sum (100k rows)", elapsed, table.row_count());
    }
}

fn bench_filter_order_limit(table: &Table, samples: usize) {
    for _ in 0..samples {
        let start = Instant::now();
        let out = QueryBuilder::new(table)
            .filter("ok", "=", Scalar::Bool(true))
            .unwrap()
            .filter("latency", ">", Scalar::F64(10.0))
            .unwrap()
            .project(vec!["service", "latency"])
            .unwrap()
            .order_by("latency", true)
            .unwrap()
            .limit(10)
            .execute()
            .unwrap();
        let elapsed = start.elapsed();
        black_box(out);
        report(
            "filter_project_order_limit (100k rows)",
            elapsed,
            table.row_count(),
        );
    }
}

fn bench_sql_group_by(table: &Table, samples: usize) {
    let table_ref = Box::leak(Box::new(table));
    for _ in 0..samples {
        let mut ctx = SqlContext::new();
        ctx.add_table("metrics", table_ref);
        let start = Instant::now();
        let out = ctx
            .run(
                "SELECT region, SUM(latency) FROM metrics WHERE ok = TRUE GROUP BY region ORDER BY region",
            )
            .unwrap();
        let elapsed = start.elapsed();
        black_box(out);
        report("sql group_by_sum (100k rows)", elapsed, table.row_count());
    }
}

fn bench_join(samples: usize) {
    let left = metrics_table(20_000);
    let right = {
        let code: Vec<Scalar> = REGIONS.iter().map(|r| Scalar::Str(r.to_string())).collect();
        let tier: Vec<Scalar> = code.iter().map(|_| Scalar::Str("prod".into())).collect();
        Table::try_new(vec![Column::new("code", code), Column::new("tier", tier)]).unwrap()
    };
    for _ in 0..samples {
        let start = Instant::now();
        let out = tpt_strata::join(&left, &right, "region", "code").unwrap();
        let elapsed = start.elapsed();
        black_box(out);
        report("equi_join (20k x 5 rows)", elapsed, left.row_count());
    }
}

fn report(name: &str, elapsed: std::time::Duration, rows: usize) {
    let per_op = elapsed.as_nanos() / rows.max(1) as u128;
    println!("{name}: {elapsed:?} per op ({rows} rows, {per_op} ns/row)");
}

fn main() {
    let table = metrics_table(100_000);
    bench_group_by_sum(&table, 7);
    bench_filter_order_limit(&table, 7);
    bench_sql_group_by(&table, 7);
    bench_join(7);
}
