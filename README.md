# tpt-strata

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

## Reading Parquet files

The import bridge reads Parquet into tpt-strata's native format with no
core-crate dependency on Parquet/Arrow:

```rust
use tpt_strata_parquet::read_parquet;

let table = read_parquet("metrics.parquet")?;
```

Supported columns: `BOOLEAN`, `INT32`, `INT64`, `DOUBLE`, `STRING`.
Unsupported types are rejected with a diagnostic naming the offending column.

## Docs

- [ARCHITECTURE.md](ARCHITECTURE.md) — crate boundaries, the format/engine
  split, and the bridge-isolation rule
- [docs/benchmarks.md](docs/benchmarks.md) — zero-dependency benchmarks and
  the Phase 0 comparison records

## License

MIT OR Apache-2.0, Copyright (c) TPT Solutions.