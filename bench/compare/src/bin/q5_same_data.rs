//! Q5 on the same data as `profile_q4 q5`, in both engines, so the comparison
//! is not affected by the benchmark's different data. The data is the driver's:
//! 1M rows, `id`, `quantity` (i % 97), and `region` (8 values).
//!
//! Run with: `cargo run --release --features parallel --bin q5_same_data`

use std::sync::Arc;
use std::time::Instant;

use datafusion::arrow::array::{ArrayRef as DfArrayRef, Int64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::datasource::MemTable;
use datafusion::prelude::{SessionConfig, SessionContext};
use tpt_strata::sql::SqlContext;
use tpt_strata::{Column, Scalar, Table};

const N: usize = 1_000_000;
const RUNS: usize = 200;
const REGIONS: [&str; 8] = [
    "north", "central", "coastal", "south", "east", "west", "cabin", "delta",
];
const SQL: &str = "SELECT COUNT(*) AS n, SUM(quantity) AS units FROM facts \
                   WHERE region LIKE 'c%'";

fn region(i: usize) -> &'static str {
    REGIONS[(i * 7 + i / 13) % REGIONS.len()]
}

fn tpt_table() -> Table {
    Table::try_new(vec![
        Column::new("id", (0..N).map(|i| Scalar::I64(i as i64)).collect()),
        Column::new(
            "quantity",
            (0..N).map(|i| Scalar::I64((i % 97) as i64)).collect(),
        ),
        Column::new(
            "region",
            (0..N).map(|i| Scalar::Str(region(i).into())).collect(),
        ),
    ])
    .unwrap()
}

fn df_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("quantity", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
    ]));
    let columns: Vec<DfArrayRef> = vec![
        Arc::new(Int64Array::from_iter_values(0..N as i64)),
        Arc::new(Int64Array::from_iter_values((0..N).map(|i| (i % 97) as i64))),
        Arc::new(StringArray::from_iter_values((0..N).map(region))),
    ];
    RecordBatch::try_new(schema, columns).unwrap()
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let facts = tpt_table();
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &facts);
    ctx.run(SQL).unwrap();
    let mut times = Vec::new();
    for _ in 0..RUNS {
        let t = Instant::now();
        let out = ctx.run(SQL).unwrap();
        assert_eq!(out.row_count(), 1);
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("tpt-strata (parallel build): median {:.3} ms", median(times));

    let batch = df_batch();
    let schema = batch.schema();
    for (label, partitions) in [
        ("DataFusion, all cores", None),
        ("DataFusion, 1 partition", Some(1)),
    ] {
        let config = match partitions {
            Some(p) => SessionConfig::new().with_target_partitions(p),
            None => SessionConfig::new(),
        };
        let df = SessionContext::new_with_config(config);
        let mem = MemTable::try_new(schema.clone(), vec![vec![batch.clone()]]).unwrap();
        df.register_table("facts", Arc::new(mem)).unwrap();
        df.sql(SQL).await.unwrap().collect().await.unwrap();
        let mut times = Vec::new();
        for _ in 0..RUNS {
            let t = Instant::now();
            let rows: usize = df
                .sql(SQL)
                .await
                .unwrap()
                .collect()
                .await
                .unwrap()
                .iter()
                .map(|b| b.num_rows())
                .sum();
            assert_eq!(rows, 1);
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        println!("{label}: median {:.3} ms", median(times));
    }
}
