# tpt-strata Benchmarks

**Source of truth:** `crates/tpt-strata/benches/queries.rs` (zero
dependencies, std test harness). Run locally with:

```sh
cargo bench -p tpt-strata
```

Benchmarks mirror the Phase 0 "point at data, get an answer" workloads at
scale so the numbers stay comparable to the incumbent records in
`validation-memo.md`.

## Environments measured

| Run | Engine | Date | Toolchain | Mode |
|---|---|---|---|---|
| A (this doc) | tpt-strata | 2026-09-11 | stable (MSRV 1.75+ compatible) | `bench` (opt) |
| Phase 0 | tpt-strata sketch | 2026-09-10 | debug | sketch, compile-only |
| Phase 0 | DataFusion v55 | 2026-09-10 | debug | recorded |

## Workloads and results (tpt-strata, optimized build)

Workloads are the Phase 0 cases (GROUP BY aggregate; filtered query) plus a
join, scaled to 100k rows. Medians of 7 samples.

| Workload | Rows | Median | ns/row |
|---|---|---|---|
| GROUP BY region, SUM(latency) — builder | 100 000 | 28.1 ms | ~281 |
| filter (2 predicates) + project + ORDER BY desc + LIMIT 10 — builder | 100 000 | 53.1 ms | ~531 |
| GROUP BY region, SUM(latency), WHERE, ORDER BY — SQL surface | 100 000 | 37.3 ms | ~373 |
| Equi-join (20 000 × 5 rows, hash join) — `join()` | 20 000 out | 11.9 ms | ~595 |

Notes on the numbers:

- The SQL surface adds parsing/binding overhead (~33% over the equivalent
  builder plan), which is expected: the builder path skips parse entirely.
- The engine materializes intermediate batches per operator as native
  arrays; measured numbers include that materialization.
- These are single-threaded, in-process, in-memory numbers. tpt-strata makes
  no claim to match multi-threaded engines on raw throughput; its value
  proposition is the embedded dependency surface (see below).

## Comparison vs. DataFusion (Phase 0 records, validation-memo.md)

Phase 0 did not record DataFusion execution latency on these workloads, so a
direct ns/row comparison would be an artifact. What the Phase 0 memo *does*
record, and what still holds, are these embedded-engine comparisons:

| Metric | DataFusion v55 | tpt-strata (finished) |
|---|---|---|
| Total dependencies (embedded) | 282 packages | **0** (core) |
| First-build compile time | ~2 min (debug) | ~1.2 s (sketch, debug) |
| Caller code (basic query) | ~40 lines | ~15 lines |
| Async runtime required | Yes (tokio) | No |
| Crate(s) reaching only the interop boundary | n/a | `tpt-strata-parquet` only |

For raw query throughput reference, DataFusion publishes its own benchmark
positioning (GB/s-class scan rates on parallelism); tpt-strata's niche is
the embedded, dependency-free call path, not a throughput race.

## How to re-run

```sh
cargo bench -p tpt-strata                   # all bench targets
cargo bench -p tpt-strata --bench queries   # this suite only
```

Numbers are captured manually in this file per environment so they can be
traced to a toolchain and date. Recapture and update the table above whenever
changes land that materially affect the engine's hot path.