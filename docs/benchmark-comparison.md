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

### Phase 15, run-based join gather

The join's output gather copies runs of consecutive rows as slices, and builds
values and validity directly. Q3 on the Parquet-loaded tables now clears the
pre-validity baseline.

| | Before validity | After validity | This change |
|---|---|---|---|
| Parquet load | 101.6 ms | 71–75 ms | 73.7 ms |
| Q1 group by + 3 aggregates | 27.3 ms | 25.7–29.2 ms | 24.0 ms |
| Q2 filter + sort + limit | 14.8 ms | 13.6–15.1 ms | 13.4 ms |
| Q3 join + group by | 52.6 ms | 65.3–65.5 ms | 43.7 ms |

### Phase 15, parallel join probe (opt-in `parallel` feature)

With `--features parallel`, the join probe runs on the same 64k-row segments as
aggregation. Workers probe contiguous runs of segments, and their results merge
in order, so the output is the same as the single pass. Q3 on the Parquet tables
(one-thread default vs `parallel`, median of 5):

| | Default (1 thread) | `parallel` |
|---|---|---|
| Q3 join + group by | 52.2 ms | 41.0 ms |
| join, no output columns | 27.2 ms | 21.8 ms |

The default build is unchanged in behaviour. Its timing is within the usual run-to-run spread.

### Phase 15, parallel gather and filter: tried, not kept

Both were tried under the `parallel` feature, on the same 1M-row Parquet tables:

| change | Q2 (filter, sort, limit) | Q3 (join, group by) |
|---|---|---|
| none (probe and aggregation parallel) | 15.9 ms | 39.6–41 ms |
| parallel filter (predicates and copies on workers) | 16.5 ms | - |
| parallel gather for all columns | - | 51–57 ms |
| parallel gather for numeric columns only | - | 51 ms |

Neither helped. The filter's copies and the gather's reference-count updates on
shared strings do not parallelise well, and the thread start-up and merge cost
outweighs the gain on these sizes. Both were reverted. The probe and the
aggregation are the parallel wins.

### Join index run (2026-10-14)

The join now uses a direct layout for `Int64` keys in a small range, with
no hashing. Other keys still use the hash map. Measured on the same data and
method:

| query | tpt-strata (1 thread) | hand loop | tpt-strata ÷ loop | % of bound |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | 24.9 ms | 0.95 ms | 26× | 5% |
| Q2 filter + sort + limit | 14.6 ms | 4.34 ms | 3.4× | 8% |
| Q3 join + group by | 45.8 ms | 1.14 ms | 40× | 2% |

Stage timing on Q3 before this change: probe 14.5 ms, gather 16 ms, build
0.1 ms. After it, the probe is about 10 ms. The gather is now the larger
cost: it copies every needed column into `Vec<Option<T>>` per output row. A
layout that does not box nulls would be the next step.

The first bound warning in the run is the same as before: the probe is a sum
loop and can be slower than the data's real read rate.

### Group-key cache run (2026-10-14)

Single-key GROUP BY on a string column now checks a 256-slot cache keyed by
each string's allocation before the content hash map. Strings shared through
an `Arc` skip hashing. A miss falls back to the map, so equal strings in
different allocations still form one group. Measured on the same data and
method:

| query | tpt-strata (1 thread) | hand loop | tpt-strata ÷ loop | % of bound |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | 6.17 ms | 1.15 ms | 5.4× | 18% |
| Q2 filter + sort + limit | 14.0 ms | 4.21 ms | 3.3× | 8% |
| Q3 join + group by | 38.5 ms | 0.82 ms | 47× | 3% |

Q1 was 24.9 ms. A stage split showed the group lookup took about 21 ms of it,
and the aggregate folds about 3 ms. The lookup now takes about 3.3 ms. Q1 is
now faster than DataFusion on one thread (8.63 ms).

Not measured yet: high-cardinality GROUP BY, where almost every string has its
own allocation. There the cache only adds one store per row on a miss.

### Join gather breakdown (2026-10-14)

Per-column gather time for Q3 on 1M rows, measured with temporary timers
(removed afterwards):

| column | type | time |
|---|---|---|
| facts.quantity | Int64 | 2.4 ms |
| facts.dim_id | Int64 | 2.4 ms |
| dims.id | Int64 | 4.4 ms |
| dims.label | Utf8 | 9.7 ms |

The string column is the largest cost. Every output row clones an `Arc<str>`,
which is an atomic increment, and drops it later. A single-row fast path for
the integer gather was tried and made no difference, so it was reverted.
The next step is a string layout with one byte buffer and offsets, which
needs no per-row reference count. That changes `StrArray`, a public type, so
it is a decision for the owner, not a quiet change.

### Int64 filter run (2026-10-14)

A filter on an `Int64` column against an `Int64` literal now reads the values
directly, the same way `Float64` columns already did. NULLs never match, as
before. Q2 is the only query this changes.

| query | tpt-strata (1 thread) | hand loop | tpt-strata ÷ loop | % of bound |
|---|---|---|---|---|
| Q2 filter + sort + limit | 8.06 ms | 4.06 ms | 2.0× | 14% |

Q2 was 14.0 ms. It is now close to DataFusion on one thread (4.0–4.5 ms).

Decision recorded: string columns keep `Arc<str>` for now. Changing them to
an offset buffer is a breaking change to a public type and would help only the
join's string gather (about 10 ms of Q3). Revisit it if the join gather
remains the main cost after other work.

### Join pushdown run (2026-10-14)

A grouped inner join is rewritten to aggregate the left side by its join key
first, then join the small result to the right table and aggregate again. The
rewrite applies only when the group keys come from the right table, there is
no WHERE or DISTINCT, and each aggregate is SUM over an integer column,
COUNT, MIN or MAX over a left column. Float sums and AVG keep the ordinary
plan, so the rounding does not change. Tests in `tests/pushdown.rs` compare the
rewritten result with a brute-force answer and with the ordinary plan.

| query | tpt-strata (1 thread) | hand loop | tpt-strata ÷ loop | % of bound |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | 8.84 ms | 1.24 ms | 7.2× | 16% |
| Q2 filter + sort + limit | 9.08 ms | 4.51 ms | 2.0× | 14% |
| Q3 join + group by | 7.13 ms | 0.84 ms | 8.5× | 18% |

Q3 was about 38 ms and is now 7.1 ms, faster than DataFusion on one thread
(9.95 ms) and about equal to DuckDB (6.99 ms). Q1 and Q2 have no join, so
the change does not touch them. Their times rose by about 40% this run, and
the hand loop and DataFusion also rose, so the machine was slower. Read the
ratios, not the absolute values, when comparing with earlier runs.

### Branch-free filter run (2026-10-14)

A filter's first predicate writes every row index and advances the output
count only on a match, rather than branching on each row. Over random data a
selective predicate mispredicts about half its branches, which cost most of
the filter's time. Tests and clippy pass.

| query | tpt-strata (1 thread) | hand loop | tpt-strata ÷ loop | % of bound |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | 5.98 ms | 1.00 ms | 6.0× | 16% |
| Q2 filter + sort + limit | 2.97 ms | 4.31 ms | 0.7× | 30% |
| Q3 join + group by | 7.48 ms | 0.67 ms | 11.2× | 12% |

Q2 was about 9 ms. It is now faster than the hand-written loop, which had the
same mispredicted branch, so that loop is no longer a floor for this query.
The bound is also a loose guide here: the bound's probe is slower than the
data's real read rate.

### Where this stands against the stop rule (2026-10-14)

Stop rule: within about 10–20% of the best reference engine on the target
workloads, near the bound on scan-bound queries, and a flat profile.

Best reference per query (one thread, 1M rows, this run's DataFusion and
DuckDB numbers):

| query | tpt-strata | best reference | result |
|---|---|---|---|
| Q1 group by | 5.98 ms | DataFusion 9.55 ms | ahead |
| Q2 filter, sort, limit | 2.97 ms | DataFusion ~4.5–5.4 ms | ahead |
| Q3 join, group by | 7.48 ms | DuckDB 6.99 ms | within 7% |

On these three queries the rule is met against the engines. Two parts are
not yet shown:
- **Bound:** the probe is a single-thread sum loop and is not a true memory
  ceiling, so "near the bound" is not measured.
- **Profile:** no sampling profiler is available here, so the profile is not
  known to be flat.

Limits of this result: three queries, one data shape, one machine, and only
1M rows. The next useful work is a wider benchmark (more query shapes, string
filters, high-cardinality GROUP BY, several sizes, and multi-thread scaling),
not further tuning of these three queries.

### Wider benchmark: five queries (2026-10-14)

Two shapes were added to the harness, at 1M rows:
- **Q4:** GROUP BY id, one group per row (1,000,000 groups).
- **Q5:** `WHERE region LIKE 'c%'`, then COUNT and SUM.

Results (one thread, median of 5, all results match tpt-strata):

| query | tpt-strata | hand loop | DataFusion (1) | DuckDB (1) | tpt-strata ÷ loop |
|---|---|---|---|---|---|
| Q1 group by | 6.3 ms | 1.1 ms | 8.3 ms | 21.8 ms | 5.8× |
| Q2 filter, sort, limit | 2.9 ms | 4.1 ms | 3.9 ms | 13.5 ms | 0.7× |
| Q3 join, group by | 7.8 ms | 0.8 ms | 9.6 ms | 6.1 ms | 10× |
| Q4 group by id | 224 ms | 56 ms | 171 ms | 217 ms | 4.0× |
| Q5 LIKE filter, count, sum | 14.4 ms | 2.1 ms | 8.2 ms | 13.8 ms | 6.8× |

Q4 is about 1.3× DataFusion on one thread and about level with DuckDB. Its
cost is mostly per-group hashing and output, which the grouping code does
not yet avoid.

Q5 was 179 ms before the LIKE changes. Two changes brought it to 14 ms:
the matcher works on bytes and allocates nothing, and a `Utf8` column
against a pattern is matched in place, with `starts_with`, `ends_with`,
`contains` or `==` for the four common shapes. A further change to the
matcher made no difference, so the remaining cost is elsewhere. The likely
cause is the filter copying the string column for every kept row, even
though the aggregate never reads it. That is not yet measured.

### Filter pruning, dense groups, integer SUM (2026-10-14)

Three changes, measured together on the five-query harness:
- A filter outputs only the columns the query reads above it (`filter_keep`).
- A GROUP BY on one `Int64` key with dense values uses direct slots, not hashing.
- An integer SUM no longer keeps an unused float total per row.

| query | tpt-strata | hand loop | DataFusion (1), from ratio | DuckDB (1), from ratio | tpt-strata ÷ loop |
|---|---|---|---|---|---|
| Q1 group by | 7.9 ms | 1.5 ms | 9.3 ms | 24 ms | 5.2× |
| Q2 filter, sort, limit | 3.2 ms | 4.4 ms | 4.4 ms | 15 ms | 0.7× |
| Q3 join, group by | 5.0 ms | 1.0 ms | 10.5 ms | 6.7 ms | 5.0× |
| Q4 group by id | 243 ms | 109 ms | 175 ms | 207 ms | 2.2× |
| Q5 LIKE filter | 11.6 ms | 2.3 ms | 9.0 ms | 15 ms | 5.2× |

DataFusion and DuckDB times are the hand-loop time multiplied by each
engine's ratio in this run's output, not separately timed figures.

The machine ran about 25% slower this time (the hand loops rose too), so
compare ratios, not absolute times, with earlier runs.

- **Q3 improved** (7.6 → 5.0 ms): the pushed-down partial aggregate uses the
  dense slots.
- **Q1 barely moved** in ratio terms. Its cost is per-row work in two
  passes (the group lookup, then three aggregate updates).
- **Q4 did not improve**, and the reason is in the harness, not the engine.
  Each run builds 1,000,000 result rows as `Vec<Cell>` values, for every
  engine. That cost is in the timed loop, so it dominates Q4. A fair Q4 needs
  an aggregate that returns few rows; this is a harness fix still to do.

### Q4 harness fix (2026-10-14)

Q4 now has `HAVING SUM(quantity) > 1000`. It still makes one group per id
(1,000,000 groups), but returns no rows, so the timing no longer includes
building a million output rows. The hand-written floor applies the same filter.

| engine | Q4 time | result rows |
|---|---|---|
| tpt-strata (1 thread) | 52.1 ms | 0 |
| hand-written loop (1 thread) | 3.18 ms | 0 |
| DataFusion (1 partition) | 56.5 ms | 0 |
| DuckDB (1 thread) | 87.6 ms | 0 |

tpt-strata is now level with DataFusion on one thread and 16× the hand loop.
The remaining cost is inside the engine. The aggregate builds all one million
group rows, then the HAVING filter discards them. Applying HAVING while the
groups are finished, so only kept groups are built, is the next step for this
query.

### Q4 phase breakdown (2026-10-14)

Q4 (one group per id, 1M groups, HAVING keeps none), timed by phase with
temporary timers, now removed:

| phase | time (median of 5) |
|---|---|
| build the 1M group states (grouping pass) | about 38 ms |
| build the 1M output group rows (`finish`) | about 33 ms |
| everything else | the remainder of the run |

Both phases are per-group work. The grouping pass allocates and fills
per-group state vectors for every group, and the output step boxes every
group's key and sum before HAVING discards them. Neither is a hot loop that a
small change will remove. The next gains for this query come from building only
the groups HAVING keeps, and from a lighter per-group state, which is a larger
change. Not started.

### HAVING inside the aggregate (2026-10-14)

HAVING is now evaluated by the aggregate on the output columns it reads, and
only the kept groups' remaining output columns are built. Tests in
`tests/having.rs` check it against hand-computed results.

The machine was heavily loaded in this run: the hand-written loops ran 2–3×
slower than in earlier runs. Compare ratios only.

| query | tpt-strata ÷ hand loop, before | after |
|---|---|---|
| Q4 group by id (HAVING keeps none) | 16× | 16.5× |

No measurable gain on Q4. The change skips the group-key column for the
groups HAVING rejects, but the probe still builds the SUM column for every
group, and the grouping pass that creates each group's state is the larger
cost. The next gain for Q4 is a lighter per-group state, not HAVING placement.

### Lighter per-group state (2026-10-14)

Integer SUMs no longer keep a float total per group, and the per-group
vectors are reserved up front on the dense path, bounded by the rows
the segment will see. The grouping pass does fewer pushes and no
reallocation. Tests and clippy pass.

The machine was still loaded, so compare ratios:

| query | tpt-strata ÷ hand loop, before | after |
|---|---|---|
| Q1 group by | 5.4× | 3.5× |
| Q2 filter, sort, limit | 0.9× | 1.1× |
| Q3 join, group by | 4.8× | 4.7× |
| Q4 group by id | 16.5× | 10.4× |
| Q5 LIKE filter | 5.7× | 5.5× |
