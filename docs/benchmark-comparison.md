# Benchmark: tpt-strata vs DataFusion vs DuckDB

A head-to-head run of the same three queries on the same Parquet data, in one
process per engine. The harness is `bench/compare`. It is a separate workspace,
because DataFusion and DuckDB are large dependencies.

## Result

All three engines return the same results on all three queries. Every
engine's output was checked against tpt-strata's (row count, numeric sum, and
text length).

**tpt-strata is slower on these analytic queries.** On one thread it is about
10–40× slower than DataFusion on one thread and 4–20× slower than DuckDB on one
thread. The gap is largest on the join and the group-by. It narrows only
against DuckDB on the join, where DuckDB is also single-threaded.

Query times, 1,000,000 rows, median of 5 runs after one warm-up:

| query | tpt-strata (1 thread) | DataFusion (1 partition) | DataFusion (20 cores) | DuckDB (1 thread) | DuckDB (20 threads) |
|---|---|---|---|---|---|
| Q1 group by, SUM, AVG, COUNT | 107.4 ms | 8.06 ms | 3.83 ms | 22.4 ms | 6.76 ms |
| Q2 filter, sort, limit 100 | 68.5 ms | 4.20 ms | 2.61 ms | 13.7 ms | 3.30 ms |
| Q3 join to 1,000 rows, group by | 268.8 ms | 9.44 ms | 4.27 ms | 5.87 ms | 4.55 ms |

Load (read the Parquet file into each engine's format, single run):

| engine | load |
|---|---|
| tpt-strata (1 thread) | 101 ms |
| DataFusion (1 partition / 20) | 37 ms / 32 ms |
| DuckDB (1 thread / 20) | 118 ms / 117 ms |

Cost of the dependencies, measured on the whole benchmark:

| measure | value |
|---|---|
| tpt-strata core crate, dependency count | 0 (the crate itself is the only entry in `cargo tree`) |
| benchmark binary, unique crates in the graph | 398 (includes tpt-strata, DataFusion and DuckDB) |
| benchmark cold build time, including DuckDB C++ and DataFusion | 846 s |
| benchmark release binary, all three engines | 169 MB |

These cover all three engines together. Per-engine dependency counts are not
measured here.

## What this says

- **For the analytic workload, tpt-strata does not compete on speed.** The
  engine works one row at a time through `Scalar` values. The DataFusion and
  DuckDB engines process columns in vectorised kernels, which is where the gap
  comes from. The earlier internal numbers (about 100 ns per row for GROUP BY)
  match Q1 here, and DataFusion's vectorised path runs at about 8 ns per row.
- **What tpt-strata offers is elsewhere.** Zero dependencies in the core crate,
  a core crate with zero dependencies that builds in seconds, integrations for
  Rust, Python, WebAssembly and serde, and errors that name
  the caller's columns. Those are the reasons to choose it, and they are not
  measured by this benchmark.
- **The planned typed hot paths (Phase 12 in `todo.md`) address this gap.** They
  are not done. Until they are, this document is the honest baseline.

## Method

- Data: `facts` has 1,000,000 rows of `id` (i64), `region` (8 strings),
  `quantity` (i64, 1–100), `price` (f64), `dim_id` (i64, 0–999). `dims` has
  1,000 rows of `id` and `label` (20 distinct). Generated from a fixed xorshift
  seed, so every run uses the same data.
- The data is written once as Parquet with tpt-strata's own writer. Each
  engine loads that file, and the load time is recorded separately.
- Queries are the same SQL in all three, except for explicit `::BIGINT`
  casts on the DuckDB side, because DuckDB widens `SUM` of a BIGINT.
- Timing: one warm-up run, then five timed runs; the median is reported. Each
  timed run includes converting the result to Rust values, so the cost is
  comparable across engines.
- Thread settings: tpt-strata is single-threaded by design. DataFusion and
  DuckDB are shown on one thread and on all 20 logical CPUs.
- Machine: Windows 11, 20 logical CPUs, release builds. Rust 1.98, clang for
  DuckDB's bundled build.

## Caveats

- This is one machine and one data shape. Larger data, wider rows, or string
  predicates would change the numbers.
- The DataFusion and DuckDB engines were run with their default settings
  apart from the thread count. No tuning was done for either.
- The benchmark measures in-memory query time after load. It does not measure
  a streaming workload, and tpt-strata has no streaming mode.
- Results are a snapshot. Re-run `cargo run --release -- 1000000` in
  `bench/compare` after any engine change, and record the date.

## Reproduce

```sh
cd bench/compare
cargo run --release -- 1000000
```

The first build compiles DataFusion and DuckDB, which takes about fourteen
minutes on the reference machine. The output is in `bench/compare/results.md`
(not committed).
