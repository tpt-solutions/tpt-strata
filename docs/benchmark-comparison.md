# Benchmark: tpt-strata vs DataFusion vs DuckDB

A head-to-head run of the same three queries on the same Parquet data, in one
process per engine. The harness is `bench/compare`. It is a separate workspace,
because DataFusion and DuckDB are large dependencies.

## Result

All three engines return the same results on all three queries. Every
engine's output was checked against tpt-strata's (row count, numeric sum, and
text length).

Latest run: 2026-10-14, 1,000,000 rows, one thread unless marked. Times are the
median of 5 runs after one warm-up. All engines return the same results on all
five queries.

On one thread, tpt-strata is faster than DataFusion on Q1 to Q4 and slower on
Q5 (21.8 ms against 15.3 ms). It is faster than DuckDB on every query. Against
DataFusion on all cores, tpt-strata (one thread) is 1.1× to 2.6× slower on
Q1 to Q4, and 4.2× slower on Q5.

| query | tpt-strata (1 thread) | DataFusion (1 partition) | DataFusion (all cores) | DuckDB (1 thread) | DuckDB (all cores) |
|---|---|---|---|---|---|
| Q1 group by, SUM, AVG, COUNT | 14.1 ms | 20.0 ms | 7.81 ms | 47.5 ms | 14.7 ms |
| Q2 filter, sort, limit 100 | 6.91 ms | 9.25 ms | 4.89 ms | 26.3 ms | 6.76 ms |
| Q3 join to 1,000 rows, group by | 11.0 ms | 23.6 ms | 9.61 ms | 14.0 ms | 9.54 ms |
| Q4 group by id, HAVING keeps none (1M groups) | 107 ms | 132 ms | 40.7 ms | 191 ms | 41.6 ms |
| Q5 LIKE filter, count and sum | 21.8 ms | 15.3 ms | 5.25 ms | 26.5 ms | 4.93 ms |

A hand-written loop over the same data, one thread, is the floor:

| query | hand loop | tpt-strata ÷ loop | % of bandwidth bound |
|---|---|---|---|
| Q1 | 2.89 ms | 4.9× | 16% |
| Q2 | 7.12 ms | 1.0× | 30% |
| Q3 | 2.76 ms | 4.0× | 19% |
| Q4 | 8.46 ms | 12.7× | 2% |
| Q5 | 3.68 ms | 5.9× | 5% |

The bandwidth bound uses a single-thread read rate of 7.8 GB/s. It is the
floor for a scan that reads the ideal bytes per row, so the percentages show
how far each query is from a memory-bound scan.

Load (read the Parquet file into each engine's format, single run):

| engine | load |
|---|---|
| tpt-strata (1 thread) | 198 ms |
| DataFusion (1 partition / 20) | 868 ms / 90 ms |
| DuckDB (1 thread / 20) | 331 ms / 291 ms |

Load times vary more between runs than query times. The 2026-10-13 run
measured tpt-strata at 104 ms, so treat load as noisy.

### All cores: tpt-strata with the `parallel` feature (2026-10-14)

The same harness, built with `cargo run --release --features parallel`. This
is a separate run from the single-thread one above. Read-rate probe and hand
loops moved between the two runs, so compare ratios within each run, not times
across them.

| query | tpt-strata, default (1 thread, earlier run) | tpt-strata, `parallel` | DataFusion (1 partition) | DataFusion (all cores) | DuckDB (all cores) |
|---|---|---|---|---|---|
| Q1 group by + 3 aggregates | 14.1 ms | 4.00 ms | 16.9 ms | 6.25 ms | 10.4 ms |
| Q2 filter + sort + limit | 6.91 ms | 5.85 ms | 5.33 ms | 3.12 ms | 4.66 ms |
| Q3 join + group by | 11.0 ms | 5.66 ms | 19.3 ms | 6.49 ms | 8.45 ms |
| Q4 group by id (1M groups) | 107 ms | **564 ms** | 74.9 ms | 34.0 ms | 30.4 ms |
| Q5 LIKE filter, count and sum | 21.8 ms | 15.7 ms | 12.4 ms | 3.68 ms | 4.03 ms |

The default-build column is from the single-thread run above, not this one.
The `parallel` column and the DataFusion and DuckDB columns are from this run.
Results match on all queries in both builds.

- **Q1 and Q3 scale.** With `parallel`, tpt-strata is ahead of DataFusion on
  all cores on Q1 (4.00 ms against 6.25 ms), and close on Q3 (5.66 ms against
  6.49 ms).
- **Q2 and Q5 do not scale.** Only aggregation and the join probe run in
  parallel, so Q2 (sort-bound) gains nothing, and Q5's filter is still
  single-threaded.
- **Q4 regresses about 5×, and the merge is the cost.** Timed in a temporary
  build (timers since removed), Q4's segment workers take about 18 ms, and the
  merge takes about 300 ms for 1,000,000 groups. The merge rebuilds a `Vec` key
  and does a hash lookup for every group in every partial. It never uses the
  dense-slot or single-key paths the workers use, so each group pays for the
  generic path. Q1 and Q3 merge only a few groups, which is why they scale.
  Fixing this means giving the merge the same fast paths, or merging dense
  partials by slot. The first step is in the next section.

### Q2 and Q5 in the parallel build (2026-10-14)

Timed with temporary per-batch timers (removed afterwards): Q5's time was in
the `LIKE` predicate, 8 to 12 ms per run, not in the copy of the kept rows,
which was about 1 ms. That corrects the earlier guess that the copy was the
cost. Q2's predicate took 2.5 to 4.7 ms.

The row selection for a predicate now splits the rows across threads under
the `parallel` feature, joins the chunks in order, and keeps the same
branch-free compaction within each chunk. Inputs under 256k rows stay on one
thread. The default build is unchanged, and the tests pass with and without
the feature.

| query | tpt-strata (parallel) | hand loop (1 thread) | DataFusion (all cores) | DuckDB (all cores) |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | 2.55 ms | 1.05 ms | 3.59 ms | 7.39 ms |
| Q2 filter + sort + limit | 3.35 ms | 4.31 ms | 2.14 ms | 3.13 ms |
| Q3 join + group by | 2.23 ms | 0.64 ms | 4.05 ms | 5.15 ms |
| Q4 group by id (1M groups) | 73.4 ms | 3.28 ms | 32.4 ms | 24.8 ms |
| Q5 LIKE filter, count and sum | 4.76 ms | 2.85 ms | 2.48 ms | 3.49 ms |

Q5 fell from 9.56 ms to 4.76 ms and is now within about 2× of DataFusion on
all cores. Q2 was 4.68 ms in the run with timers and 3.35 ms here, so its
run-to-run spread is wide; treat it as about 1.5× DataFusion, not a
precise figure. Q4 is about 2.3× DataFusion and 3× DuckDB on all cores.

### Dense fast path and HAVING on accumulators (2026-10-14)

Two changes for Q4, both measured in the parallel build:
- A dense single-key path for integer `SUM` and `COUNT`. Each worker aggregates
  its own segments into arrays over only the keys those segments hold. The
  merge keeps the first-seen order, and `tests/dense_fast_path.rs` checks the
  results against a plain pass over three batches with NULLs and negative keys.
- HAVING on an integer `SUM` against an integer literal is decided from the
  accumulators, so no output column is built for every group. Timed with
  temporary timers (removed), this step went from about 15 ms to about 2.5 ms.

Phase timings before the HAVING change, from temporary timers (removed):
workers about 14 ms, merge about 20 ms (down from about 32 ms after reserving
the merge's vectors), ordering about 3 ms.

| query | tpt-strata (parallel) | DataFusion (all cores) |
|---|---|---|
| Q1 group by + 3 aggregates | 2.29 ms | 3.54 ms |
| Q2 filter + sort + limit | 3.36 ms | 1.72 ms |
| Q3 join + group by | 4.71 ms | 3.72 ms |
| Q4 group by id (1M groups) | 54.8 ms | 29.7 ms |
| Q5 LIKE filter, count and sum | 4.00 ms | 2.30 ms |

Q4 is about 1.85× DataFusion on all cores. Q2 is about 2×, and Q5 about 1.7×.

### Baseline on the committed code, 9b78dc3 (2026-10-14)

A clean run of the committed dense path with the parallel feature. Times are
the median of 5 after one warm-up, and every engine matches tpt-strata's result.

| query | tpt-strata (parallel) | hand loop (1 thread) | DataFusion (all cores) |
|---|---|---|---|
| Q1 group by + 3 aggregates | 2.81 ms | 1.14 ms | 3.78 ms |
| Q2 filter + sort + limit | 3.21 ms | 3.95 ms | 1.82 ms |
| Q3 join + group by | 5.05 ms | 0.73 ms | 4.04 ms |
| Q4 group by id (1M groups) | 41.6 ms | 3.16 ms | 28.5 ms |
| Q5 LIKE filter, count and sum | 5.33 ms | 2.81 ms | 2.78 ms |

Against DataFusion on all cores, Q1 is faster, Q3 is about 1.25× behind, Q4
about 1.5×, Q2 about 1.8×, and Q5 about 1.9×. Q2 is already at the speed of a
plain loop, so its gap is in the sort and the limit, not in the scan.

### Q5: where the time goes (2026-10-14)

Timed with temporary timers (removed). The aggregate's input step, the
`LIKE` filter over 1M rows, takes about 3 ms per run. The aggregate itself
takes about 0.6 ms. So the gap to DataFusion (about 2.3 to 2.8 ms) is in the
scan. The string column stores a fat pointer per row, 16 bytes, so the scan
reads about 16 MB for 1M rows. DataFusion's layout (offsets into one byte
buffer) reads a few megabytes for the same column. Closing this gap means
changing the string storage, which is a core format change, not a filter change.

### Clean run, after the filter and the COUNT(*) fix (2026-10-15)

Two back-to-back runs on the release build, with the parallel feature and
without it. Times are the median of 5 after one warm-up. Every engine returns
the same result on every query.

| query | tpt-strata (parallel) | tpt-strata (1 thread) | hand loop (1 thread) | DataFusion (all cores) |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | 2.25 ms | 12.4 ms | 0.99 ms | 3.79 ms |
| Q2 filter + sort + limit | 2.51 ms | 19.8 ms | 4.30 ms | 1.97 ms |
| Q3 join + group by | 5.27 ms | 12.2 ms | 0.62 ms | 4.18 ms |
| Q4 group by id (1M groups) | 46.8 ms | 84.2 ms | 3.36 ms | 30.9 ms |
| Q5 LIKE filter, count and sum | 4.48 ms | 23.7 ms | 2.15 ms | 2.44 ms |

Against DataFusion on all cores, the parallel build is faster on Q1 (2.25 vs
3.79 ms). It is slower on the rest: about 1.3× on Q2 and Q3, 1.5× on Q4, and
1.8× on Q5. The single-thread build is 3 to 10× behind DataFusion, which is
expected: DataFusion uses 20 threads. Q4 and Q5 are the slowest against
DataFusion, for the reasons in the sections above. The machine's speed moved between the two runs, so compare each run's
columns with each other.

A bug found while writing the new cases is fixed in the same change: a
`SELECT COUNT(*) ... WHERE ...` failed when the filter kept no columns.

### Fused filter (2026-10-14)

Each row is tested against all of a filter's predicates in one pass, instead of
the first predicate building a row list of its matches that the second then
narrows. Each predicate's test is chosen once per batch, so the per-row work
is the same comparison as before. The first predicate of Q2 kept 499,672 rows
and cost about 1.85 ms, and the second cost about 1.05 ms. The fused pass
writes only the 2,003 final matches, and the output is built by pushing, so
it is only as large as the matches.

| query | tpt-strata (parallel) | hand loop (1 thread) | DataFusion (all cores) |
|---|---|---|---|
| Q1 group by + 3 aggregates | 2.31 ms | 1.06 ms | 3.44 ms |
| Q2 filter + sort + limit | 2.24 ms | 4.07 ms | 1.74 ms |
| Q3 join + group by | 4.89 ms | 0.82 ms | 3.82 ms |
| Q4 group by id (1M groups) | 38.3 ms | 3.02 ms | 30.0 ms |
| Q5 LIKE filter, count and sum | 3.92 ms | 2.03 ms | 2.78 ms |

Q2 is now about 1.3× DataFusion, and Q5 about 1.4×. The Q5 filter also uses
the push-based row list, which is faster in this run than the branch-free one
it replaced. Q4 and Q3 are about 1.3× behind, within run-to-run noise of
earlier runs. Results match on every query.

### Q2: where the time goes (2026-10-14)

Timed with temporary timers (removed). Q2's filter leaves 2,003 rows for the
sort, and the top-100 sort of those takes about 0.05 ms. The sort is not the
cost. Most of Q2's time is in the input step, the filter over the 1M-row table,
at about 3 ms. Running the second predicate in parallel, like the first, did
not change Q2 (3.58 ms in a slower run, against 3.21 to 3.36 ms before), so it
was reverted. What remains in the filter is not yet separated: predicate
evaluation, the row-index writes, or the copy of the kept rows.

### Partitioned merge: tried, no gain (2026-10-14)

Each worker lists its groups by key bucket, and one merge thread per bucket
merges only its own groups. The NULL group is merged after the buckets. Tests
and clippy pass, and the results match. Q4 did not improve: 49.3 ms, against
43.3 ms with the narrower records alone, and about 8% worse against the hand
loop. The machine ran a little slower in this run (the hand loop is about 5%
slower), so the regression may be partly noise. The bucket lists grow from
empty in every worker, which adds allocation and page-fault work, the same
cost that slowed the workers before. That is a guess, not measured.

### Narrower group records (2026-10-14)

The dense path's group records are narrower: keys are plain `i64`, first rows
are packed into one `u64`, and integer sums are `i64` with an overflow check.
An overflow takes the general path, which reports it as before. The records
went from about 57 bytes per group to about 33.

| query | tpt-strata (parallel) | hand loop (1 thread) | DataFusion (all cores) |
|---|---|---|---|
| Q1 group by + 3 aggregates | 2.28 ms | 0.98 ms | 4.11 ms |
| Q2 filter + sort + limit | 4.35 ms | 4.07 ms | 1.94 ms |
| Q3 join + group by | 4.95 ms | 0.71 ms | 4.44 ms |
| Q4 group by id (1M groups) | 43.3 ms | 3.05 ms | 31.1 ms |
| Q5 LIKE filter, count and sum | 4.67 ms | 2.18 ms | 2.12 ms |

Q4 is about 1.4× DataFusion here, against about 1.8× in the previous run. The
machine moved between runs, and against the hand-loop floor the gain is about
8%, so treat it as a modest improvement, not a confirmed one.

Worker scaling for Q4 (temporary timers, removed): in the default single-thread
build, the one worker over all 1M rows takes about 16 ns per row. In the
parallel build, each worker over its 64k-row segment takes about 8–9 ms, about
130 ns per row. The parallel stage therefore barely speeds up: Q4's aggregation
is about 57 ms on one thread and about 50 ms on all cores. DataFusion gains
about 2.2× from its threads on the same query. A probe program showed that
plain arithmetic scales across 16 threads on this machine, and that fresh
allocation and page faulting slow down about 4× when 16 threads do it at once.
That accounts for about 2 ms of each worker's time. The rest is not yet
explained. Pre-sizing the worker vectors made no difference.

Merge breakdown for Q4 (temporary timers, removed): allocating and filling the
merge's vectors takes about 3 ms, the merge loop about 14 to 17 ms, and the
ordering under 1 ms. The loop moves roughly 100 bytes per group across the
worker records, the slot table and the merged arrays. It is limited by memory
traffic, not by per-group work. Filling the output vectors by index instead of
pushing made no measurable difference (53.9 ms against 54.8 ms). Narrower
records would help a little. Closing the gap to DataFusion needs a different
layout for dense groups.
Across runs the machine speed moves by up to about 2×, so compare ratios within
a run.

### Q4 after reserving the merge and typed HAVING (2026-10-14)

Two changes, both measured in the parallel build:
- The merge reserves its output vectors up front, since the total group count
  is known. Q4 went from about 73 ms to 66 ms.
- HAVING now runs on the typed path for a comparison, and for an `AND` of
  comparisons. It used to check each of the 1,000,000 groups through a boxed
  `Scalar`. Q4 went from 66 ms to 53 ms.

| query | tpt-strata (parallel) | DataFusion (all cores) | DuckDB (all cores) |
|---|---|---|---|
| Q1 group by + 3 aggregates | 2.41 ms | 3.18 ms | - |
| Q2 filter + sort + limit | 3.39 ms | 1.70 ms | - |
| Q3 join + group by | 2.22 ms | 3.56 ms | - |
| Q4 group by id (1M groups) | 52.7 ms | 30.6 ms | 23.7 ms |
| Q5 LIKE filter, count and sum | 3.88 ms | 2.25 ms | - |

Q4 is about 1.7× DataFusion on all cores, down from 2.3×. Q2 is about 2×, and
Q5 about 1.7×. The remaining Q4 cost is still per group: the partial pass and
the merge each touch all 1,000,000 groups, with a few vector pushes per group.

### Parallel merge with a single-key index (2026-10-14)

The merge now indexes a single group column on the key itself, as the segment
pass does, instead of building a `Vec` key per group. Tests and clippy pass
with `--features parallel`. Same harness, same run for all engines and the
hand loop, so compare ratios within this table:

| query | tpt-strata (parallel) | hand loop (1 thread) | DataFusion (all cores) | DuckDB (all cores) |
|---|---|---|---|---|
| Q1 group by + 3 aggregates | 2.34 ms | 0.92 ms | 3.61 ms | 8.31 ms |
| Q2 filter + sort + limit | 2.99 ms | 3.86 ms | 1.90 ms | 3.56 ms |
| Q3 join + group by | 2.60 ms | 0.71 ms | 4.12 ms | 4.93 ms |
| Q4 group by id (1M groups) | 166 ms | 3.57 ms | 30.0 ms | 21.7 ms |
| Q5 LIKE filter, count and sum | 9.56 ms | 1.87 ms | 2.45 ms | 2.74 ms |

Timed with temporary timers (removed afterwards), Q4's parallel build spends
about 20 ms in the partial pass and about 35 ms in the merge, out of roughly
80 ms for the run. The merge is still the largest part: it visits each of the
1,000,000 groups once per partial and makes random accesses into the slot
table. Merging in parallel by key range is the next step, and it is not done.

Q4 fell from 564 ms to 166 ms in the parallel build, and it still trails
DataFusion and DuckDB on all cores by about 5× and 8×. The machine was faster
in this run (the hand loops are about half their earlier times), so the
absolute numbers are not comparable with the earlier tables, only the ratios.
The remaining Q4 cost has not been measured. The next step is the slot-based
merge for a dense key, which would avoid hashing in the merge altogether.

Cost of the dependencies, measured on the whole benchmark (from the
2026-10-13 build; not re-measured):

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
| 2026-10-14 run, after the later performance passes (table above) | 14.1 ms | 6.91 ms | 11.0 ms |

The 2026-10-14 row is from a single run and uses a different query set, so
treat the two earlier rows as a record of the work, not a like-for-like
comparison.

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
  casts on the DuckDB side, because DuckDB widens `SUM` of a BIGINT. There
  are five queries: Q1 to Q3 as before, Q4 (group by id, with a HAVING that
  keeps no group, so the output is empty) and Q5 (`region LIKE 'c%'`, then
  count and sum).
- Timing: one warm-up run, then five timed runs; the median is reported. Each
  timed run includes converting the result to Rust values, so the cost is
  comparable across engines.
- Thread settings: tpt-strata is single-threaded in the main results. An
  opt-in `parallel` feature exists and is covered in its own sections below.
  DataFusion and DuckDB are shown on one thread and on all 20 logical CPUs.
- A hand-written loop over the same data is also timed, as a floor for each
  query. Percentages of the bandwidth bound use a measured single-thread read
  rate.
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

### Q4 reserve and the run after it (2026-10-16)

The dense worker now reserves its group vectors up front, bounded by the rows
it sees and its key range. Q4 creates one group per id, and before this the
five per-group vectors regrew many times. One run each, parallel build, same
session, so compare only the two Q4 rows:

| query | tpt-strata (parallel), before | after |
|---|---|---|
| Q4 group by id (1M groups) | 56.3 ms | 40.6 ms |

Q4 is still about 12× the hand loop. Group creation and the merge of the
per-worker tables are the remaining cost. Not profiled: the sampling profiler
needs elevation this session does not have.

Q5 uses the column pruning from `filter_keep` (already in place). Its gain was
not measured on its own.

### Q4 stage breakdown (2026-10-16)

The sampling profiler (`samply`) needs Administrator rights on Windows, so the
profile was taken with temporary timers in the dense path instead. One run of
Q4 on 1M rows, parallel build, five runs in a row (timings from the first runs
were within 1 ms of each other):

| stage | ms | share |
|---|---|---|
| per-worker group pass (16 segments) | 7.5 | 20% |
| merge of worker tables into one | 13.5 | 36% |
| first-seen order (`by_row`, `order`) | 2.7 | 7% |
| building output state vectors | about 12 | 32% |
| HAVING over 1M groups | 2.5 | 7% |

Total about 38 ms. The merge and the output build together are about two
thirds of the time. Both allocate and fill 1M-entry vectors several times over,
and the merge does per-group work even though no key appears in two workers.
The next change is to avoid the merge: give each worker a key range instead of a
run of rows, so each group is built by exactly one worker, and build output
vectors once, at their final size. Not started.

### Key-range partitioning for Q4 (tried, not kept, 2026-10-16)

The merge in the stage breakdown above was replaced by giving each thread a
contiguous key range, so no group is shared and the results concatenate. Each
thread then scans every row and keeps only its own keys. Q4, minimum of 15 runs
in one process, parallel build:

| variant | min ms | median ms |
|---|---|---|
| HEAD (row split + merge) | 34.8 | 37.7 |
| key ranges, 20 threads | 40.9 | 43.5 |
| key ranges, 8 threads | 36.4 | 38.8 |
| key ranges, 4 threads | 33.4 | 36.5 |

With 20 threads, each thread reads the key and value columns in full, so the
input is read about 20 times. The extra memory traffic costs more than the
merge saves. With 4 threads the gain is inside run-to-run noise. The change was
reverted. Q3 also regressed to 11.6 ms when the same split was applied to a
small key range, which is one reason the change was not kept.

The merge is still the largest Q4 stage. Options not yet tried: a per-group
state that is 16 bytes or less, and output vectors built directly from the
workers' ranges.

### Skipping the first-seen sort when groups are already in order (tried, not kept, 2026-10-16)

When the merged groups are already sorted by their first row, the by-row table
and the sort can be skipped. Q4 qualifies. Minimum of 15 runs, twice, against
HEAD: 35.6 vs 35.5 ms and 34.6 vs 34.6 ms. No measurable gain, so it was
reverted. The stage timers had overstated that step, which is why this was
measured end to end before keeping anything.

### HAVING on a single SUM decided before the output is built (2026-10-16)

`dense_aggregate` now tests a single `HAVING SUM(col) op literal` on the
accumulators, in first-seen order, before it builds the output vectors. Only the
groups it keeps get output. Execution's own HAVING pass then re-tests those and
keeps them all, so results do not change. Q4's HAVING keeps no group, so its
output build was wasted before this.

Q4, minimum of 15 runs, parallel build, two rounds:

| variant | min ms | median ms |
|---|---|---|
| HEAD | 37.2, 43.6 | 42.5, 46.1 |
| this change | 27.3, 28.1 | 29.7, 31.1 |

Full run, same change, single run per query (parallel build): Q1 2.40 ms, Q2
2.36 ms, Q3 4.69 ms, Q4 27.1 ms, Q5 5.14 ms. Q4 is now below DataFusion's
all-cores time (34 ms, from the earlier runs; DataFusion was not rerun for this
change).

`tests/fast_path_cases.rs` checks every comparison operator against a plain
pass over the data, with the groups in the same order.

### How to profile Q4 with samply

`samply` samples a program's stacks. Its own help says recording is supported on
Linux and macOS. On Windows, the installed version (0.13.1) tried to start an
elevated helper and reported that it needs Administrator rights. So Windows
recording is not a reliable route. Use WSL, which runs Linux.

The Q4 driver is `crates/tpt-strata/examples/profile_q4.rs`. It runs the query
many times, so the profiler has enough samples. Inside WSL (Ubuntu 24.04), from
the repository, with the target directory outside the Windows checkout:

```bash
# One-time: install samply into the WSL Rust toolchain.
cargo install samply --locked

# Build the driver with line tables, so the profile shows source lines.
CARGO_TARGET_DIR=$HOME/target-tpt CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
  cargo build --release -p tpt-strata --features parallel --example profile_q4

# Record 300 timed runs. Save to a file; open it later with `samply load`.
samply record --save-only -o q4.json.gz -- \
  $HOME/target-tpt/release/examples/profile_q4 300
```

`samply load q4.json.gz` opens the Firefox Profiler in a browser. Loading a file
there does not upload it. The useful views are the Call Tree and Flame Graph, on
each worker thread. Look for `dense_worker`, `merge_overlapping` and
`dense_aggregate` to match the stage breakdown above.

### Q4 profile in WSL, and the sum-only path (2026-10-16)

Recorded with samply in WSL Ubuntu 24.04 (`profile_q4`, 300 runs, about 27,000
samples on the worker threads). Leaf samples, grouped by source line, with the
code as it was before the change below:

| share | where |
|---|---|
| 16.5% | `alloc/vec/mod.rs:1046` in `dense_worker` (a Vec operation, by line) |
| 11.4% | `aggregate.rs:843`, the per-row key column match |
| 8.0% | `core/intrinsics/mod.rs:457` in `dense_worker` (not identified) |
| 8.0% and 6.5% | `aggregate.rs:842` (row loop) and `int_macros.rs:2499` (checked add) |
| 6.2% | `alloc/vec/mod.rs:1037` in `dense_worker` (a Vec operation, by line) |
| 8.3% | unresolved (`??`) |

Most of the time is in `dense_worker`, and most of that is writing group state.
Two changes followed:

- **Column match hoisted** out of the row loop, and the group pass written as a
  single `extend` over the segment. On Windows the difference was inside the
  run-to-run noise (27.6–28.5 ms before, 27.2–28.2 ms after).
- **No row counts unless a COUNT needs them.** A query with only `SUM` never
  reads the count vector, but it was written for every group, in the workers and
  again in the merge. `DenseGroups` now keeps counts only when a `COUNT` is
  present. Q4 (`SUM` only), Windows, minimum of 200-run means:

| variant | ms per run |
|---|---|
| before this change (two runs) | 27.6, 28.5 |
| counts skipped (two runs) | 24.3, 23.3 |

The full benchmark, same build, one run per query (parallel build): Q1 1.95 ms,
Q2 2.27 ms, Q3 4.66 ms, Q4 24.3 ms, Q5 4.08 ms. DataFusion on all cores in the
same run: Q1 3.78, Q2 1.99, Q3 4.09, Q4 28.0, Q5 2.66 ms. Q4 is now below
DataFusion; Q2 and Q5 are still above it, and Q3 is about level.

The WSL timings are not comparable with the Windows ones: the first WSL run of
the driver took 72 ms per run, and the profiled run took 21 ms. Compare ratios
within one environment.

### Q5 profile (2026-10-16, partial)

The same driver runs Q5 (`profile_q4 300 q5`). 300 runs, WSL, about 5,200 leaf
samples on the driver's thread, which include the setup of the 1M-row table.

| share | where |
|---|---|
| 19% | `RowTest::test` (`engine.rs`): the per-row LIKE test |
| 11% | `Vec` building (`from_iter`), mostly table setup |
| 65% | `libc.so.6` |

The libc share is not reliable. Symbol lookup in libc has no debug info, and
its largest entry is a misattributed name. Thread creation (`__clone`, about
10% of samples) is the one libc item that fits the code: the parallel build
starts a new set of threads for each aggregate or filter in a query. For a
query that takes about 5 ms, a fixed cost of that size matters. That is the
next thing to measure, before changing the per-row LIKE test.

### Thread start-up cost (measured, not changed, 2026-10-16)

Spawning and joining 20 scoped threads takes about 0.75 ms on this machine
(Windows, 500 rounds). The parallel filter and the parallel aggregate both spawn
a fresh set of threads for each operator, so a pool that reuses threads was the
next candidate. Two measurements say the gain would be small:

| Q5, 300 runs, Windows | ms per run |
|---|---|
| parallel build | 6.3, 6.3 |
| sequential build (no `parallel` feature) | 12.7, 12.6 |
| parallel, filter capped at 8 threads | 6.8, 6.6 |

Parallelism halves Q5, so the spawn cost is worth paying. Capping the threads
changed nothing, which means the spawn cost overlaps with work that has already
started on the earlier threads. A pool would therefore save much less than the
0.75 ms measured alone. A pool would also keep threads alive for the life of the
process, and that is a design decision for the crate's maintainers, so it was not
made here.

### String sharing and the Q5 cost (measured, 2026-10-16)

The Q5 cost is the per-row string access. Two facts:

- The Parquet bridge already shares strings. `tpt-strata-parquet` reuses an
  `Arc<str>` for a value it has already seen, from a list of up to 32 distinct
  values. The benchmark loads its table from Parquet, so Q5 already runs on
  shared strings.
- The sharing that is left is limited by layout. `StrArray` stores a
  `Vec<Arc<str>>`, a 16-byte fat pointer per row. Every test reads that pointer
  and then the string it points to.

A throwaway experiment, 1M rows, 8 regions, Q5, Windows, 60 runs each: one
`Arc` per row gave a minimum of 6.2–6.6 ms, and one shared `Arc` per region gave
4.5–5.5 ms. The benchmark path is the second case already.

Two follow-ups:

- The 32-value cap in `tpt-strata-parquet` meant a column with more than 32
  distinct values allocated again for each row whose value was not in the first
  32. Fixed: values past the first 32 now go in a hash set, per batch. The test
  `repeated_strings_share_one_allocation_past_the_first_32_values` fails on the
  old reader and passes on the new one. The benchmark's Q5 column has 8 distinct
  values, so its numbers do not change. A high-cardinality string column now
  pays one hash per row at load time.
- Removing the 16-byte pointer needs an offset-buffer layout for `StrArray`. The
  decision above still applies: it is a breaking change to a public type.

### Q3 profile, and first-seen order with few groups (2026-10-17)

`EXPLAIN` for Q3 shows the join pushdown at work: the left side is aggregated
first, to one row per key, and the hash join then sees 1,000 rows, not 1M. The
join gather is not on Q3's path, so the design note's expectation that Q3 is
limited by the gather was wrong. The profile (samply in WSL, `profile_q4 300 q3`)
showed the cost in the dense aggregate instead. Most of it was one table over
every input row, built only to put 1,000 groups in first-seen order.

Now, with fewer than one group per 16 rows, the groups are sorted by their first
row. The table over all rows is kept for large group counts. Q3, Windows, the
driver's mean of 200 runs, before and after, two rounds:

| variant | ms per run |
|---|---|
| before | 5.45, 5.24 |
| after | 2.66, 2.81 |

Full benchmark, one run per query, parallel build, after the change: Q1 2.03 ms,
Q2 2.49 ms, Q3 2.11 ms, Q4 21.6 ms, Q5 4.44 ms. DataFusion on all cores in the
same run: Q1 3.73, Q2 1.85, Q3 4.05, Q4 28.7, Q5 3.05 ms. Q3 and Q4 are now below
DataFusion. Q2 and Q5 are still above it.
