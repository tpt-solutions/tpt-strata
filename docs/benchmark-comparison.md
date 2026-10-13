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

### Phase 15, second run (after batch-at-a-time aggregation and join changes)

Same machine and method as above.

| query | tpt-strata (1 thread) | DataFusion (1 partition) | DataFusion (all cores) | DuckDB (1 thread) | DuckDB (all cores) |
|---|---|---|---|---|---|
| Q1 group by + 3 aggregates | 29.2 ms | 7.94 ms | 4.28 ms | 23.3 ms | 7.25 ms |
| Q2 filter + sort + limit | 14.6 ms | 3.76 ms | 2.03 ms | 14.1 ms | 3.60 ms |
| Q3 join + group by | 58.6 ms | 9.42 ms | 4.46 ms | 6.37 ms | 6.07 ms |

Load: tpt-strata 146 ms (before the Parquet string-collector change, which is
measured in the next run), DataFusion 39 ms, DuckDB 686 ms and 112 ms.

Changes in this step: aggregates fold each batch with typed loops, not one
dispatch per row (SQL GROUP BY, 100k rows: 86 → 47 ns/row); the join sizes its
output vectors up front (probe 28 → 15 ms on 1M rows); the join's gather
resolves each batch's array once. Q3 dropped from 95 to 59 ms.

Parity is still not reached. Q3 is about 6× DataFusion on one thread; Q1 and
Q2 are about 3–4×.

### Phase 15, third run (packed join positions, shared Parquet strings)

| query | tpt-strata (1 thread) | DataFusion (1 partition) | DataFusion (all cores) | DuckDB (1 thread) | DuckDB (all cores) |
|---|---|---|---|---|---|
| Q1 group by + 3 aggregates | 28.4 ms | 12.8 ms | 3.97 ms | 22.8 ms | 6.73 ms |
| Q2 filter + sort + limit | 14.5 ms | 3.81 ms | 2.19 ms | 13.5 ms | 3.41 ms |
| Q3 join + group by | 53.0 ms | 9.45 ms | 4.25 ms | 6.48 ms | 5.96 ms |

Load: tpt-strata 97 ms (was 146), DataFusion 38 ms. The DataFusion numbers
moved between runs on this machine (Q1 was 7.9 ms in the last run), so
compare the tpt-strata figures across runs, not against DataFusion's.

Joins now write one packed 8-byte position per row, not a 24-byte `Option`.
Q3 went from 59 to 53 ms, and load from 146 to 97 ms, mostly from sharing one
`Arc` per distinct Parquet string.

## Ceiling and distance (hand-written floor, bandwidth bound)

Run with the same data and method (1,000,000 rows, median of 5 after one
warm-up, 2026-10-14). Added to `bench/compare`:

- **Hand-written loop (floor):** one thread, plain typed `Vec`s, region as a
  one-byte code, direct lookup for the join. It is the fastest a loop on these
  columns is likely to get. It is not a true ceiling: DataFusion beats it on Q2.
- **Bandwidth bound:** rows × ideal bytes per row ÷ single-thread read rate.
  Q1 reads 17 bytes per row, Q2 and Q3 read 16.

| query | bound ms | hand loop ms | tpt-strata ÷ loop | DataFusion ÷ loop | DuckDB ÷ loop | tpt-strata % of bound |
|---|---|---|---|---|---|---|
| Q1 group by + 3 aggregates | 1.15 | 1.12 | 28× | 8× | 20× | 4% |
| Q2 filter + sort + limit | 1.08 | 4.16 | 3.5× | 0.9× | 3.3× | 7% |
| Q3 join + group by | 1.08 | 0.72 | 75× | 14× | 8× | 2% |

Read-rate probe: 14.8 GB/s single-thread (sum over 256 MB, best of 5).

What this says:

- **The distance is large.** tpt-strata runs at 2–7% of the bound on all
  three queries. The gap is in the engine, not in memory bandwidth. Q1 and Q3
  are 28× and 75× the hand loop.
- **The bound is not a hard ceiling.** Q3 and Q1 hand loops ran faster than
  the bound, so the probe is pessimistic. It is one sum loop, which may be
  limited by the CPU rather than DRAM, and Q3's 16 MB of data partly fits in
  cache. The warnings in the run flag this. Treat the bound as a rough guide.
- **The floor is not the maximum.** On Q2, DataFusion (3.9 ms) beats the
  hand loop (4.2 ms). Its filter must be branchless or vectorised. The real
  ceiling is at most DataFusion's time on each query.
- **Maximum reachable, practically:** the Q1 and Q3 hand loops suggest well
  under 10 ms each for tpt-strata on one thread. Q2 is around 4 ms on a plain
  loop. Those are the targets, not promises.

Stop rule for this work: within about 20% of the best engine on a query, at
or near the bound on scan-bound queries, and a flat profile.

### Phase 15, parallel aggregation (opt-in `parallel` feature)

The default build is still single-threaded. With `--features parallel` in
tpt-strata, aggregation splits its input into 64k-row segments and folds
contiguous runs of them on `std::thread::scope` workers, then merges the
partials in order. Only aggregation runs in parallel; sort, join and
filter still run on one thread.

| query | tpt-strata, default (1 thread) | tpt-strata, `parallel` | DataFusion (all cores) |
|---|---|---|---|
| Q1 group by + 3 aggregates | 27.3 ms | 5.46 ms | 4.06 ms |
| Q2 filter + sort + limit | 14.8 ms | 15.3 ms (no parallel stage) | 2.40 ms |
| Q3 join + group by | 52.6 ms | 42.6 ms | 4.32 ms |

Integer results are exact in both modes. Float `SUM` and `AVG` can differ in
the last bit between the modes, because the addition order changes.

### Phase 15, validity bitmaps (one thread, 1M rows)

Columns store values plus an optional validity vector. A column with no NULLs
stores no validity. Numeric columns are half the size they were.

| | Before | After |
|---|---|---|
| Parquet load | 101.6 ms | 71–73 ms |
| Q1 group by + 3 aggregates | 27.3 ms | 25.7–29.2 ms |
| Q2 filter + sort + limit | 14.8 ms | 13.6–15.1 ms |
| Q3 join + group by | 52.6 ms | 65.3–65.5 ms (regression) |

Q3 regressed on the Parquet-loaded tables; the same join in memory is at the
earlier time (58 ms against 57 ms). The cause is not yet identified.

### Phase 15, validity fix (`value_ref` on the no-NULL path)

Q3 on the Parquet-loaded tables: 75 ms before this fix, 60 ms in the Parquet
stage timing, and 59.5 ms in the comparison. The single-stage numbers
against the pre-validity commit were: join count-only 24 ms before and 30 ms
now; numeric gather 2.5 ms before and about 6 ms now. Remaining gap to the
pre-validity Q3 (52.6 ms): about 7 ms.

| | Before validity | After validity | After `value_ref` fix |
|---|---|---|---|
| Parquet load | 101.6 ms | 71–73 ms | 75 ms |
| Q1 group by + 3 aggregates | 27.3 ms | 25.7–29.2 ms | 24.7 ms |
| Q2 filter + sort + limit | 14.8 ms | 13.6–15.1 ms | 13.4 ms |
| Q3 join + group by | 52.6 ms | 65.3–65.5 ms | 59.5 ms |
