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
// Q4: one group per id (1,000,000 groups). HAVING keeps no group, so the result is
// empty and the cost is the grouping itself, not building output rows.
const Q4_TPT: &str = "SELECT id, SUM(quantity) AS units FROM facts GROUP BY id HAVING SUM(quantity) > 1000";
const Q4_DUCK: &str = "SELECT id, SUM(quantity)::BIGINT AS units FROM facts GROUP BY id HAVING SUM(quantity) > 1000";
// Q5: a string predicate (LIKE) that keeps two of eight regions, then a count and sum.
const Q5_TPT: &str = "SELECT COUNT(*) AS n, SUM(quantity) AS units FROM facts WHERE region LIKE 'c%'";
const Q5_DUCK: &str = "SELECT COUNT(*)::BIGINT AS n, SUM(quantity)::BIGINT AS units FROM facts \
     WHERE region LIKE 'c%'";

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
        ArrayRef::Utf8(StrArray::new(d.region.iter().map(|s| std::sync::Arc::<str>::from(s.as_str())).collect())),
        ArrayRef::Int64(I64Array::new(d.quantity.clone())),
        ArrayRef::Float64(F64Array::new(d.price.clone())),
        ArrayRef::Int64(I64Array::new(d.dim_id.clone())),
    ];
    Table::from_batches(schema.clone(), vec![Batch::new_unchecked(schema, cols)])
}

fn dims_table() -> Table {
    let ids: Vec<i64> = (0..1000).collect();
    let labels: Vec<std::sync::Arc<str>> = ids.iter().map(|i| std::sync::Arc::from(format!("label-{:02}", i % 20))).collect();
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
                        Some(d @ tpt_strata::Scalar::Date(_)) | Some(d @ tpt_strata::Scalar::Timestamp(_)) => Cell::Text(d.to_string()),
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

// === Ceiling: hardware bound and hand-written floor ===

/// Single-thread read rate in GB/s: a sum over a buffer far larger than cache,
/// best of `RUNS`. This is the speed a scan can reach if nothing else is slow.
fn measure_bandwidth() -> f64 {
    const WORDS: usize = 32 * 1024 * 1024; // 256 MB of u64
    let buf: Vec<u64> = (0..WORDS as u64).collect();
    let bytes = (WORDS * std::mem::size_of::<u64>()) as f64;
    let mut best = f64::MAX;
    for _ in 0..RUNS {
        let t = Instant::now();
        let s = std::hint::black_box(buf.iter().fold(0u64, |a, &x| a.wrapping_add(x)));
        std::hint::black_box(s);
        best = best.min(t.elapsed().as_secs_f64());
    }
    bytes / best / 1e9
}

/// Region names as small codes, so the floor loops index an array. Built once,
/// outside the timed runs.
fn region_codes(d: &Data) -> Vec<u8> {
    d.region
        .iter()
        .map(|r| REGIONS.iter().position(|x| *x == r.as_str()).unwrap() as u8)
        .collect()
}

/// Q1 as one pass over typed columns, with an array of accumulators per region.
fn q1_floor(d: &Data, codes: &[u8]) -> Vec<Vec<Cell>> {
    let mut units = [0i64; 8];
    let mut price_sum = [0f64; 8];
    let mut n = [0i64; 8];
    for ((&c, &q), &p) in codes.iter().zip(&d.quantity).zip(&d.price) {
        let c = c as usize;
        units[c] += q;
        price_sum[c] += p;
        n[c] += 1;
    }
    let mut order: Vec<usize> = (0..8).collect();
    order.sort_by_key(|&c| REGIONS[c]);
    order
        .iter()
        .map(|&c| {
            vec![
                Cell::Text(REGIONS[c].to_string()),
                Cell::Int(units[c]),
                Cell::Float(price_sum[c] / n[c] as f64),
                Cell::Int(n[c]),
            ]
        })
        .collect()
}

/// Q2 as one filter pass, then a top-100 selection over the survivors only.
/// Ties on price keep input order, as the SQL engines do.
fn q2_floor(d: &Data, codes: &[u8]) -> Vec<Vec<Cell>> {
    let mut hits: Vec<(f64, u32)> = Vec::new();
    for (i, (&q, &p)) in d.quantity.iter().zip(&d.price).enumerate() {
        if q > 50 && p < 40.0 {
            hits.push((p, i as u32));
        }
    }
    let cmp = |a: &(f64, u32), b: &(f64, u32)| b.0.partial_cmp(&a.0).unwrap().then(a.1.cmp(&b.1));
    if hits.len() > 100 {
        hits.select_nth_unstable_by(99, cmp);
        hits.truncate(100);
    }
    hits.sort_by(cmp);
    hits.iter()
        .map(|&(p, i)| {
            let i = i as usize;
            vec![
                Cell::Int(d.id[i]),
                Cell::Text(REGIONS[codes[i] as usize].to_string()),
                Cell::Float(p),
            ]
        })
        .collect()
}

/// Q3 as a direct lookup from dim_id to label (dims ids are 0..1000 and the
/// label of id i is i % 20, as in `dims_table`), then 20 sum accumulators.
fn q3_floor(d: &Data) -> Vec<Vec<Cell>> {
    let label_of: Vec<usize> = (0..1000).map(|i| i % 20).collect();
    let mut units = [0i64; 20];
    for (&dim, &q) in d.dim_id.iter().zip(&d.quantity) {
        units[label_of[dim as usize]] += q;
    }
    (0..20)
        .map(|c| {
            vec![
                Cell::Text(format!("label-{:02}", c)),
                Cell::Int(units[c]),
            ]
        })
        .collect()
}

/// Q4 with the ids dense from 0: one sum per id, then the HAVING filter. Only
/// the sums are built, so the output is empty as the engines' is.
fn q4_floor(d: &Data) -> Vec<Vec<Cell>> {
    let mut units = vec![0i64; d.id.len()];
    for (&id, &q) in d.id.iter().zip(&d.quantity) {
        units[id as usize] += q;
    }
    units
        .iter()
        .enumerate()
        .filter(|(_, &u)| u > 1000)
        .map(|(id, &u)| vec![Cell::Int(id as i64), Cell::Int(u)])
        .collect()
}

/// Q5: count and sum over the regions whose code is 4 (central) or 5 (coastal).
fn q5_floor(d: &Data, codes: &[u8]) -> Vec<Vec<Cell>> {
    let (mut n, mut units) = (0i64, 0i64);
    for (&c, &q) in codes.iter().zip(&d.quantity) {
        if c == 4 || c == 5 {
            n += 1;
            units += q;
        }
    }
    vec![vec![Cell::Int(n), Cell::Int(units)]]
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

    let bw = measure_bandwidth();
    let codes = region_codes(&data);
    // Ideal bytes read per row: Q1 region code + quantity + price (17);
    // Q2 scan reads quantity + price (16); Q3 reads dim_id + quantity (16).
    // Q4 reads id + quantity (16); Q5 reads region code + quantity (9).
    let bound_ms = |name: &str| -> f64 {
        let bytes_per_row = if name.starts_with("Q1") {
            17.0
        } else if name.starts_with("Q5") {
            9.0
        } else {
            16.0
        };
        rows as f64 * bytes_per_row / (bw * 1e9) * 1e3
    };

    let mut results: Vec<Measured> = Vec::new();
    let queries: [(&'static str, &str); 5] = [
        ("Q1 group by + 3 aggregates", Q1_TPT),
        ("Q2 filter + sort + limit", Q2),
        ("Q3 join + group by", Q3_TPT),
        ("Q4 group by id (high cardinality)", Q4_TPT),
        ("Q5 LIKE filter + count and sum", Q5_TPT),
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

        let (floor_ms, floor_cells) = match name {
            n if n.starts_with("Q1") => bench_sync(|| Ok(q1_floor(&data, &codes)))?,
            n if n.starts_with("Q2") => bench_sync(|| Ok(q2_floor(&data, &codes)))?,
            n if n.starts_with("Q4") => bench_sync(|| Ok(q4_floor(&data)))?,
            n if n.starts_with("Q5") => bench_sync(|| Ok(q5_floor(&data, &codes)))?,
            _ => bench_sync(|| Ok(q3_floor(&data)))?,
        };
        let s = summarize(&floor_cells);
        results.push(Measured {
            query: name,
            engine: "hand-written loop (1 thread)".into(),
            median_ms: floor_ms,
            matches_reference: same(&s, &reference),
            summary: s,
        });


        for (label, ctx_df) in [
            ("DataFusion (1 partition)", &df_one),
            ("DataFusion (all cores)", &df_all),
        ] {
            let sql = if name.starts_with("Q1") {
                Q1_TPT
            } else if name.starts_with("Q3") {
                Q3_TPT
            } else if name.starts_with("Q4") {
                Q4_TPT
            } else if name.starts_with("Q5") {
                Q5_TPT
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
            } else if name.starts_with("Q4") {
                Q4_DUCK
            } else if name.starts_with("Q5") {
                Q5_DUCK
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
    println!("\n## Ceiling and distance (1 thread)");
    println!("- single-thread read rate: {bw:.1} GB/s (sum over 256 MB, best of {RUNS})");
    println!("- bandwidth bound = rows × ideal bytes per row ÷ read rate");
    println!("- ×floor = engine time ÷ hand-written loop time; % of bound = bound ÷ engine time");
    println!("| query | bandwidth bound ms | hand loop ms | tpt-strata ×floor | DataFusion ×floor | DuckDB ×floor | tpt-strata % of bound |");
    println!("|---|---|---|---|---|---|---|");
    for (name, _) in queries {
        let get = |engine: &str| -> f64 {
            results
                .iter()
                .find(|m| m.query == name && m.engine == engine)
                .map_or(f64::NAN, |m| m.median_ms)
        };
        let floor = get("hand-written loop (1 thread)");
        let tpt = get("tpt-strata (1 thread)");
        let df = get("DataFusion (1 partition)");
        let duck = get("DuckDB (1 thread)");
        let bound = bound_ms(name);
        if floor < bound {
            eprintln!("WARNING: {name}: hand loop ({floor:.2} ms) is faster than the bandwidth bound ({bound:.2} ms); check the byte count");
        }
        println!(
            "| {name} | {} | {} | {:.1}× | {:.1}× | {:.1}× | {:.0}% |",
            fmt_ms(bound),
            fmt_ms(floor),
            tpt / floor,
            df / floor,
            duck / floor,
            bound / tpt * 100.0
        );
    }
    Ok(())
}

