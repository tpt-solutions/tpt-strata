# Benchmark: tpt-strata vs DataFusion vs DuckDB

A head-to-head run of the same three queries on the same Parquet data, in one
process per engine. The harness is `bench/compare`. It is a separate workspace,
because DataFusion and DuckDB are large dependencies.

## Result

All three engines return the same results on all three queries. Every
engine's output was checked against tpt-strata's (row count, numeric sum, and
text length).

On one thread, tpt-strata is slower than both other engines on these queries.
DataFusion is about 4–13× faster and DuckDB is about 1.5–16× faster. The
largest gap is on the join, where tpt-strata is about 11× slower than
DataFusion and 16× slower than DuckDB. On the group-by, tpt-strata is close to
DuckDB and about 4× slower than DataFusion.

Query times, 1,000,000 rows, median of 5 runs after one warm-up:

| query | tpt-strata (1 thread) | DataFusion (1 partition) | DataFusion (20 cores) | DuckDB (1 thread) | DuckDB (20 threads) |
|---|---|---|---|---|---|
| Q1 group by, SUM, AVG, COUNT | 34.7 ms | 8.50 ms | 4.22 ms | 23.4 ms | 7.48 ms |
| Q2 filter, sort, limit 100 | 53.7 ms | 4.20 ms | 2.01 ms | 13.1 ms | 3.80 ms |
| Q3 join to 1,000 rows, group by | 103.7 ms | 9.39 ms | 4.41 ms | 6.62 ms | 5.69 ms |

Load (read the Parquet file into each engine's format, single run):

| engine | load |
|---|---|
| tpt-strata (1 thread) | 104 ms |
| DataFusion (1 partition / 20) | 38 ms / 32 ms |
| DuckDB (1 thread / 20) | 121 ms / 117 ms |

Cost of the dependencies, measured on the whole benchmark:

| measure | value |
|---|---|
| tpt-strata core crate, dependency count | 0 (the crate itself is the only entry in `cargo tree`) |
| benchmark binary, unique crates in the graph | 398 (includes tpt-strata, DataFusion and DuckDB) |
| benchmark cold build time, including DuckDB C++ and DataFusion | 846 s |
| benchmark release binary, all three engines | 169 MB |

These cover all three engines together. Per-engine dependency counts are not
measured here.

## History of tpt-strata's numbers

The same query, Q1, measured at each stage of the work:

| stage | Q1 | Q2 | Q3 |
|---|---|---|---|
| first measurement (boxed `Scalar` values everywhere) | 107 ms | 68 ms | 269 ms |
| typed group keys and accumulators, typed filter, typed sort and join keys, Fx-style hasher | 33 ms | 55 ms | 178 ms |
| shared (`Arc`) column storage and column pruning before joins | 31–35 ms | 49–54 ms | 101–104 ms |

Q2 sorts roughly 200,000 rows after its filter (an estimate from the data
distribution). The sort has not been profiled on its own, so the next step is
to profile it before changing it.

## What this says

- **For analytic queries on one thread, tpt-strata is not competitive on speed
  yet.** It processes values through typed keys and accumulators, but the
  engine still works one row at a time within each operator. DataFusion and
  DuckDB process columns in vectorised kernels.
- **What tpt-strata offers is elsewhere.** A core crate with zero dependencies
  that builds in seconds, integrations for Rust, Python, WebAssembly and serde,
  and errors that name the caller's columns. Those are the reasons to choose it,
  and this benchmark does not measure them.
- **Remaining work** is in `todo.md` under Phase 12. Vectorised filter kernels
  and an index-based sort are not done. String columns still copy their values
  when a result is built, because strings are stored as `String` values. Moving
  strings to an offset buffer is a likely next step; the cost has not been
  measured on its own.

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

## Phase 15 run (2026-10-13)

Same data and queries, 1,000,000 rows, median of 5 after one warm-up. All
engines return the same results. This machine ran slower than the earlier
runs: DataFusion's load time rose from 37 ms to 60 ms, so compare ratios
within this table, not across tables.

| query | tpt-strata (1 thread) | DataFusion (1 partition) | DataFusion (all cores) | DuckDB (1 thread) | DuckDB (all cores) |
|---|---|---|---|---|---|
| Q1 group by + 3 aggregates | 32.4 ms | 10.1 ms | 5.76 ms | 27.9 ms | 8.59 ms |
| Q2 filter + sort + limit | 17.7 ms | 5.70 ms | 2.51 ms | 14.4 ms | 4.39 ms |
| Q3 join + group by | 94.5 ms | 11.6 ms | 5.65 ms | 8.96 ms | 6.13 ms |

Load (Parquet to engine format): tpt-strata 186 ms, DataFusion 60 ms,
DuckDB 969 ms (one thread) and 189 ms (20 threads).

What changed, and what the measurements showed:

- Q2 dropped from 54 ms to 18 ms. A stage breakdown on 100k rows found that
  filtering spent most of its time copying `String`s. Strings are now shared
  as `Arc<str>`, two predicates run in one pass, and `ORDER BY … LIMIT`
  keeps only the top rows.
- Q1 barely moved. Single-key grouping is faster, but each row still
  dispatches every aggregate separately. Batch-at-a-time accumulation is the
  next step.
- Q3 is still about 8 times DataFusion on one thread. The join is the largest
  gap and has not been profiled on its own.

Parity with DataFusion and DuckDB is not reached. The remaining gaps are the
join, the per-row aggregate dispatch, and the layout of arrays as
`Vec<Option<T>>`.
