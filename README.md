# tpt-strata

[![crates.io](https://img.shields.io/crates/v/tpt-strata.svg)](https://crates.io/crates/tpt-strata)
[![docs.rs](https://docs.rs/tpt-strata/badge.svg)](https://docs.rs/tpt-strata)
[![CI](https://github.com/tpt-solutions/tpt-strata/actions/workflows/ci.yml/badge.svg)](https://github.com/tpt-solutions/tpt-strata/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

An embeddable columnar query engine for analytical, in-process queries with
zero external dependencies.

## What and why

tpt-keystone-db, tpt-aion, and tpt-cloud-observability all need to answer
analytical questions over embedded data: "sum this column grouped by that one."
tpt-strata is a small, independent, embeddable columnar query engine built for
that, without a JVM, a server process, or a large external trait hierarchy.

See `spec.txt` for the full design spec, including the non-negotiables:

1. **Core independence** — the execution engine and its native columnar format
   are not built on, or wire-compatible with, any external in-memory columnar
   standard.
2. **Interchange is a boundary concern** — reading Parquet in and emitting
   interchange-format batches out is handled by a dedicated bridge crate,
   isolated from the core.
3. **Diagnostics are first-class** — every error names the caller's own problem
   in the caller's own terms (their column names, their types).

## Crates

| Crate | Purpose | External deps |
|---|---|---|
| `tpt-strata` | Core engine: native format, builder API, SQL surface, diagnostics | 0 |
| `tpt-strata-parquet` | Parquet import bridge | Parquet/Arrow only |

## Quick Start

```rust
use tpt_strata::{Table, Column, Scalar, QueryBuilder, AggFunc, print_table};

let table = Table::try_new(vec![
    Column::new("department", vec![Scalar::Str("eng".into()), Scalar::Str("sales".into())]),
    Column::new("salary", vec![Scalar::F64(100000.0), Scalar::F64(80000.0)]),
])?;

let result = QueryBuilder::new(&table)
    .group_by(vec!["department"])?
    .aggregate("salary", AggFunc::Avg)?
    .execute()?;

print_table(&result);
```

No trait implementations required for the common case. The same query can be
run as SQL, still inside the core crate and still with zero external deps:

```rust
use tpt_strata::{Table, Column, Scalar, sql::SqlContext};

let mut ctx = SqlContext::new();
ctx.add_table("employees", &table);

let result = ctx.run(
    "SELECT department, AVG(salary) FROM employees GROUP BY department",
)?;
```

Supported SQL: `SELECT` (columns, `*`, `SUM`/`COUNT`/`MIN`/`MAX`/`AVG`),
`FROM`/`JOIN ... ON`, `WHERE` (with `AND`), `GROUP BY`, `ORDER BY`, `LIMIT`.
Keywords are case-insensitive; column names are case-sensitive.

Anything outside that subset — `OR`, `IS NULL`, `IN`, `BETWEEN`, `LIKE`,
`DISTINCT`, `HAVING`, parentheses, `LEFT`/`OUTER` joins, multiple joins,
column-to-column comparisons — is rejected with a targeted diagnostic naming
the construct and pointing at [docs/v1.1-scope.md](docs/v1.1-scope.md): those
omissions are deliberate decisions for the v1.1 milestone, not accidents.

## Reading Parquet files

The import bridge reads Parquet into tpt-strata's native format with no
core-crate dependency on Parquet/Arrow:

```rust
use tpt_strata_parquet::read_parquet;

let table = read_parquet("metrics.parquet")?;
```

Supported columns: `BOOLEAN`, `INT32`, `INT64`, `DOUBLE`, `STRING`.
Unsupported types are rejected with a diagnostic naming the offending column.

## Examples

Runnable end-to-end examples for both crates:

```sh
cargo run -p tpt-strata --example basic_query     # builder API end-to-end
cargo run -p tpt-strata --example sql_query       # SqlContext end-to-end (incl. JOIN)
cargo run -p tpt-strata --example diagnostics     # every QueryError variant, printed
cargo run -p tpt-strata-parquet --example read_parquet   # Parquet file -> table
```

## Comparison with the incumbent (DataFusion)

[validation-memo.md](validation-memo.md) measured tpt-strata against
DataFusion v55 on identical workloads (the Phase 0 "point at data, get an
answer" case). Condensed to the three decided-on dimensions:

| Metric | DataFusion v55 | tpt-strata |
|---|---|---|
| Embedded dependencies | 282 packages | **0** (core); Parquet-only bridge |
| First-build compile time (debug) | ~2 min | ~1.2 s |
| Error clarity | Leaks `Arrow ... Int64` internals | Names the caller's column and types |

Full numbers, per-workload benchmarks, and side-by-side diagnostic wording:
[validation-memo.md](validation-memo.md), [docs/benchmarks.md](docs/benchmarks.md).

## Docs

- [ARCHITECTURE.md](ARCHITECTURE.md) — crate boundaries, the format/engine
  split, and the bridge-isolation rule
- [docs/v1.1-scope.md](docs/v1.1-scope.md) — deliberate v1.1 scope decisions
  (SQL features, data types, forward-looking ideas)
- [docs/integration-recipe.md](docs/integration-recipe.md) — a template for
  wiring tpt-strata into a struct-backed store
- [docs/benchmarks.md](docs/benchmarks.md) — zero-dependency benchmarks and
  the Phase 0 comparison records

## License

MIT OR Apache-2.0, Copyright (c) TPT Solutions.