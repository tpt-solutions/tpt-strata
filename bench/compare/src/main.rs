//! Head-to-head query benchmark: tpt-strata vs DataFusion vs DuckDB.
//!
//! All three engines load the same Parquet files (written once by tpt-strata's
//! own writer) and run the same three queries. Each query runs once as a
//! warm-up and then `RUNS` times; the median is reported. Every engine's result
//! is summarised (row count, numeric sum, text length) and compared with
//! tpt-strata's, so a fast wrong answer shows up as a mismatch.
//!
//! Run: `cargo run --release -- [ROWS]` from `bench/compare`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use datafusion::arrow::array::{Array, Float64Array, Int64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::arrow::util::display::array_value_to_string;
use datafusion::datasource::MemTable;
use datafusion::prelude::{ParquetReadOptions, SessionConfig, SessionContext};
use duckdb::types::Value;
use duckdb::Connection;
use tpt_strata::sql::SqlContext;
use tpt_strata::{Batch, DataType, Field, Schema, Table};
use tpt_strata::{ArrayRef, I64Array, StrArray, F64Array};

const RUNS: usize = 5;
const REGIONS: [&str; 8] = [
    "north", "south", "east", "west", "central", "coastal", "alpine", "desert",
];

const Q1_TPT: &str = "SELECT region, SUM(quantity) AS units, AVG(price) AS avg_price, COUNT(*) AS n \
     FROM facts GROUP BY region ORDER BY region";
const Q1_DUCK: &str = "SELECT region, SUM(quantity)::BIGINT AS units, AVG(price) AS avg_price, \
     COUNT(*)::BIGINT AS n FROM facts GROUP BY region ORDER BY region";
const Q2: &str = "SELECT id, region, price FROM facts WHERE quantity > 50 AND price < 40.0 \
     ORDER BY price DESC LIMIT 100";
const Q3_TPT: &str = "SELECT dims.label, SUM(facts.quantity) AS units FROM facts \
     JOIN dims ON facts.dim_id = dims.id GROUP BY dims.label ORDER BY label";
const Q3_DUCK: &str = "SELECT dims.label, SUM(facts.quantity)::BIGINT AS units FROM facts \
     JOIN dims ON facts.dim_id = dims.id GROUP BY dims.label ORDER BY label";

// === Data ===

struct Data {
    id: Vec<i64>,
    region: Vec<String>,
    quantity: Vec<i64>,
    price: Vec<f64>,
    dim_id: Vec<i64>,
}

/// Deterministic pseudo-random data (xorshift), so every run sees the same rows.
fn generate(n: usize) -> Data {
    let mut s: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut d = Data {
        id: Vec::with_capacity(n),
        region: Vec::with_capacity(n),
        quantity: Vec::with_capacity(n),
        price: Vec::with_capacity(n),
        dim_id: Vec::with_capacity(n),
    };
    for i in 0..n {
        d.id.push(i as i64);
        d.region.push(REGIONS[(next() % 8) as usize].to_string());
        d.quantity.push((next() % 100 + 1) as i64);
        d.price.push((next() % 1_000_000) as f64 / 100.0);
        d.dim_id.push((next() % 1000) as i64);
    }
    d
}

fn facts_table(d: &Data) -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, false),
        Field::new("quantity", DataType::Int64, false),
        Field::new("price", DataType::Float64, false),
        Field::new("dim_id", DataType::Int64, false),
    ]));
    let cols = vec![
        ArrayRef::Int64(I64Array::new(d.id.clone())),
        ArrayRef::Utf8(StrArray::new(d.region.clone())),
        ArrayRef::Int64(I64Array::new(d.quantity.clone())),
        ArrayRef::Float64(F64Array::new(d.price.clone())),
        ArrayRef::Int64(I64Array::new(d.dim_id.clone())),
    ];
    Table::from_batches(schema.clone(), vec![Batch::new_unchecked(schema, cols)])
}

fn dims_table() -> Table {
    let ids: Vec<i64> = (0..1000).collect();
    let labels: Vec<String> = ids.iter().map(|i| format!("label-{:02}", i % 20)).collect();
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

// === Results and checksums ===

#[derive(Clone, Debug)]
enum Cell {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
}

#[derive(Debug, PartialEq)]
struct Summary {
    rows: usize,
    numeric: f64,
    text_bytes: usize,
}

fn summarize(rows: &[Vec<Cell>]) -> Summary {
    let mut numeric = 0.0;
    let mut text_bytes = 0;
    for row in rows {
        for c in row {
            match c {
                Cell::Null => {}
                Cell::Int(v) => numeric += *v as f64,
                Cell::Float(v) => numeric += v,
                Cell::Text(s) => text_bytes += s.len(),
            }
        }
    }
    Summary {
        rows: rows.len(),
        numeric,
        text_bytes,
    }
}

fn same(a: &Summary, b: &Summary) -> bool {
    a.rows == b.rows
        && a.text_bytes == b.text_bytes
        && (a.numeric - b.numeric).abs() <= 1e-6 * a.numeric.abs().max(1.0)
}

fn table_cells(t: &Table) -> Vec<Vec<Cell>> {
    let mut out = Vec::with_capacity(t.row_count());
    for batch in t.batches() {
        for r in 0..batch.row_count() {
            out.push(
                batch
                    .columns
                    .iter()
                    .map(|c| match c.get(r) {
                        None | Some(tpt_strata::Scalar::Null) => Cell::Null,
                        Some(tpt_strata::Scalar::I32(v)) => Cell::Int(v as i64),
                        Some(tpt_strata::Scalar::I64(v)) => Cell::Int(v),
                        Some(tpt_strata::Scalar::F64(v)) => Cell::Float(v),
                        Some(tpt_strata::Scalar::Bool(b)) => Cell::Int(b as i64),
                        Some(tpt_strata::Scalar::Str(s)) => Cell::Text(s),
                    })
                    .collect(),
            );
        }
    }
    out
}

fn batches_cells(batches: &[RecordBatch]) -> Result<Vec<Vec<Cell>>, String> {
    let mut out = Vec::new();
    for b in batches {
        for r in 0..b.num_rows() {
            let mut row = Vec::with_capacity(b.num_columns());
            for col in b.columns() {
                let cell = if col.is_null(r) {
                    Cell::Null
                } else if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
                    Cell::Int(a.value(r))
                } else if let Some(a) = col.as_any().downcast_ref::<Float64Array>() {
                    Cell::Float(a.value(r))
                } else if let Some(a) = col.as_any().downcast_ref::<StringArray>() {
                    Cell::Text(a.value(r).to_string())
                } else {
                    Cell::Text(array_value_to_string(col, r).map_err(|e| e.to_string())?)
                };
                row.push(cell);
            }
            out.push(row);
        }
    }
    Ok(out)
}

fn duck_value(v: Value) -> Cell {
    match v {
        Value::Null => Cell::Null,
        Value::BigInt(n) => Cell::Int(n),
        Value::Int(n) => Cell::Int(n as i64),
        Value::Double(f) => Cell::Float(f),
        Value::Float(f) => Cell::Float(f as f64),
        Value::Text(s) => Cell::Text(s),
        other => Cell::Text(format!("{other:?}")),
    }
}

// === Timing ===

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn bench_sync(
    mut f: impl FnMut() -> Result<Vec<Vec<Cell>>, String>,
) -> Result<(f64, Vec<Vec<Cell>>), String> {
    f()?; // warm-up
    let mut times = Vec::with_capacity(RUNS);
    let mut last = Vec::new();
    for _ in 0..RUNS {
        let t = Instant::now();
        last = f()?;
        times.push(ms(t));
    }
    Ok((median(times), last))
}

async fn bench_datafusion(
    ctx: &SessionContext,
    sql: &str,
) -> Result<(f64, Vec<Vec<Cell>>), String> {
    let mut times = Vec::with_capacity(RUNS);
    let mut last = Vec::new();
    for i in 0..=RUNS {
        let t = Instant::now();
        let batches = ctx
            .sql(sql)
            .await
            .map_err(|e| e.to_string())?
            .collect()
            .await
            .map_err(|e| e.to_string())?;
        let cells = batches_cells(&batches)?;
        if i > 0 {
            times.push(ms(t));
        }
        last = cells;
    }
    Ok((median(times), last))
}

// === Engines ===

async fn load_datafusion(
    config: SessionConfig,
    facts: &Path,
    dims: &Path,
) -> Result<(SessionContext, f64), String> {
    let ctx = SessionContext::new_with_config(config);
    let t = Instant::now();
    for (name, path) in [("facts", facts), ("dims", dims)] {
        let batches = ctx
            .read_parquet(path.to_string_lossy().as_ref(), ParquetReadOptions::default())
            .await
            .map_err(|e| e.to_string())?
            .collect()
            .await
            .map_err(|e| e.to_string())?;
        let schema = batches[0].schema();
        let mem = MemTable::try_new(schema, vec![batches]).map_err(|e| e.to_string())?;
        ctx.register_table(name, Arc::new(mem))
            .map_err(|e| e.to_string())?;
    }
    Ok((ctx, ms(t)))
}

fn load_duckdb(threads: usize, facts: &Path, dims: &Path) -> Result<(Connection, f64), String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.execute_batch(&format!("PRAGMA threads={threads};"))
        .map_err(|e| e.to_string())?;
    let t = Instant::now();
    for (name, path) in [("facts", facts), ("dims", dims)] {
        let p = path.to_string_lossy().replace('\\', "/");
        conn.execute_batch(&format!(
            "CREATE TABLE {name} AS SELECT * FROM read_parquet('{p}');"
        ))
        .map_err(|e| e.to_string())?;
    }
    Ok((conn, ms(t)))
}

fn duck_query(conn: &Connection, sql: &str) -> Result<Vec<Vec<Cell>>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let ncols = rows.as_ref().map_or(0, |s| s.column_count());
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut cells = Vec::with_capacity(ncols);
        for i in 0..ncols {
            cells.push(duck_value(row.get::<_, Value>(i).map_err(|e| e.to_string())?));
        }
        out.push(cells);
    }
    Ok(out)
}

// === Driver ===

struct Measured {
    query: &'static str,
    engine: String,
    median_ms: f64,
    summary: Summary,
    matches_reference: bool,
}

fn fmt_ms(v: f64) -> String {
    if v < 10.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.1}")
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    println!("# Comparison run");
    println!("- rows (facts): {rows}, dims: 1000");
    println!("- timed runs per query: {RUNS} (after one warm-up), median reported");
    println!("- logical CPUs: {cpus}");
    println!("- tpt-strata 1.0.1 (path dependency), DataFusion 55.2, DuckDB 1.10501");

    let dir: PathBuf = std::env::temp_dir().join("tpt-strata-compare");
    std::fs::create_dir_all(&dir)?;
    let facts_path = dir.join(format!("facts-{rows}.parquet"));
    let dims_path = dir.join("dims.parquet");
    let data = generate(rows);
    tpt_strata_parquet::write_parquet(&facts_table(&data), &facts_path)?;
    tpt_strata_parquet::write_parquet(&dims_table(), &dims_path)?;
    drop(data);

    // --- load: read the Parquet files into each engine's own format ---
    let t = Instant::now();
    let t_facts = tpt_strata_parquet::read_parquet(&facts_path)?;
    let t_dims = tpt_strata_parquet::read_parquet(&dims_path)?;
    let tpt_load = ms(t);

    let (df_one, df_one_load) = load_datafusion(
        SessionConfig::new().with_target_partitions(1),
        &facts_path,
        &dims_path,
    )
    .await?;
    let (df_all, df_all_load) = load_datafusion(
        SessionConfig::new().with_target_partitions(cpus),
        &facts_path,
        &dims_path,
    )
    .await?;
    let (duck_one, duck_one_load) = load_duckdb(1, &facts_path, &dims_path)?;
    let (duck_all, duck_all_load) = load_duckdb(cpus, &facts_path, &dims_path)?;

    println!("\n## Load (Parquet to engine format, median not used: single run)");
    println!("| engine | load ms |\n|---|---|");
    println!("| tpt-strata (1 thread) | {} |", fmt_ms(tpt_load));
    println!("| DataFusion (1 partition) | {} |", fmt_ms(df_one_load));
    println!("| DataFusion ({cpus} partitions) | {} |", fmt_ms(df_all_load));
    println!("| DuckDB (1 thread) | {} |", fmt_ms(duck_one_load));
    println!("| DuckDB ({cpus} threads) | {} |", fmt_ms(duck_all_load));

    // --- queries ---
    let mut ctx = SqlContext::new();
    ctx.add_table("facts", &t_facts);
    ctx.add_table("dims", &t_dims);

    let mut results: Vec<Measured> = Vec::new();
    let queries: [(&'static str, &str); 3] = [
        ("Q1 group by + 3 aggregates", Q1_TPT),
        ("Q2 filter + sort + limit", Q2),
        ("Q3 join + group by", Q3_TPT),
    ];

    for (name, tpt_sql) in queries {
        let (tpt_ms, tpt_rows) = bench_sync(|| {
            let t = ctx.run(tpt_sql).map_err(|e| e.to_string())?;
            Ok(table_cells(&t))
        })?;
        let reference = summarize(&tpt_rows);
        results.push(Measured {
            query: name,
            engine: "tpt-strata (1 thread)".into(),
            median_ms: tpt_ms,
            summary: summarize(&tpt_rows),
            matches_reference: true,
        });


        for (label, ctx_df) in [
            ("DataFusion (1 partition)", &df_one),
            ("DataFusion (all cores)", &df_all),
        ] {
            let sql = if name.starts_with("Q1") {
                Q1_TPT
            } else if name.starts_with("Q3") {
                "SELECT dims.label, SUM(facts.quantity) AS units FROM facts \
                 JOIN dims ON facts.dim_id = dims.id GROUP BY dims.label ORDER BY label"
            } else {
                Q2
            };
            let (m, cells) = bench_datafusion(ctx_df, sql).await?;
            let s = summarize(&cells);
            results.push(Measured {
                query: name,
                engine: label.into(),
                median_ms: m,
                matches_reference: same(&s, &reference),
                summary: s,
            });
        }

        for (label, conn) in [
            ("DuckDB (1 thread)", &duck_one),
            ("DuckDB (all cores)", &duck_all),
        ] {
            let sql = if name.starts_with("Q1") {
                Q1_DUCK
            } else if name.starts_with("Q3") {
                Q3_DUCK
            } else {
                Q2
            };
            let (m, cells) = bench_sync(|| duck_query(conn, sql))?;
            let s = summarize(&cells);
            results.push(Measured {
                query: name,
                engine: label.into(),
                median_ms: m,
                matches_reference: same(&s, &reference),
                summary: s,
            });
        }
    }

    println!("\n## Queries (median of {RUNS} runs after one warm-up, ms)");
    println!("| query | engine | median ms | result rows | matches tpt-strata |");
    println!("|---|---|---|---|---|");
    for m in &results {
        println!(
            "| {} | {} | {} | {} | {} |",
            m.query,
            m.engine,
            fmt_ms(m.median_ms),
            m.summary.rows,
            if m.matches_reference { "yes" } else { "NO" }
        );
    }

    let mismatches: Vec<&Measured> = results.iter().filter(|m| !m.matches_reference).collect();
    if !mismatches.is_empty() {
        eprintln!("\nWARNING: {} result(s) differ from tpt-strata", mismatches.len());
        for m in mismatches {
            eprintln!("  {} / {}: {:?}", m.query, m.engine, m.summary);
        }
    }
    Ok(())
}

