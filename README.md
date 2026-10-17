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

| Crate | Purpose | External deps | Docs |
|---|---|---|---|
| [`tpt-strata`](crates/tpt-strata) | Core engine: native format, builder API, SQL surface, diagnostics | 0 | [README](crates/tpt-strata/README.md), [CHANGELOG](crates/tpt-strata/CHANGELOG.md) |
| [`tpt-strata-parquet`](crates/tpt-strata-parquet) | Parquet import and export bridge | Parquet/Arrow only | [README](crates/tpt-strata-parquet/README.md), [CHANGELOG](crates/tpt-strata-parquet/CHANGELOG.md) |
| [`tpt-strata-cli`](crates/tpt-strata-cli) | `strata` command: SQL over CSV and Parquet files | the two crates above | not published |
| [`tpt-strata-serde`](crates/tpt-strata-serde) | Serde: convert structs to tables and back | serde | not published |
| [`tpt-strata-wasm`](crates/tpt-strata-wasm) | WebAssembly build, used by the [browser playground](playground/) | wasm-bindgen, serde_json | not published |
| [`tpt-strata-python`](crates/tpt-strata-python) | Python bindings (`import tpt_strata`), built with maturin | PyO3 | not published |

The serde and WASM crates build with the workspace. The Python binding needs
maturin and a Python toolchain, so it sits outside the Rust workspace. The core
crate stays dependency-free in every case. See [docs/bindings.md](docs/bindings.md).

New here? Start with the [quickstart](docs/quickstart.md) and the
[cookbook](docs/cookbook.md).

## Installation

```toml
[dependencies]
tpt-strata = "0.1"
tpt-strata-parquet = "0.1"   # only if you read or write Parquet
```

The core crate has no external dependencies. Check with
`cargo tree -p tpt-strata`. The two crates are released together and share a
version number. Per-crate details are in the
[core README](crates/tpt-strata/README.md) and the
[Parquet README](crates/tpt-strata-parquet/README.md).

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

| Construct | Status |
|---|---|
| `SELECT` columns, `*`, `AS` aliases, `DISTINCT` | supported |
| `SUM`, `COUNT` (incl. `COUNT(*)`), `MIN`, `MAX`, `AVG` | supported |
| `FROM`, `[LEFT \| RIGHT \| FULL] JOIN ... ON a.x = b.y`, several joins in a chain | supported (equi-joins) |
| `WHERE` / `HAVING` with `AND`, `OR`, and parentheses | supported |
| `=`, `!=`, `<>`, `<`, `<=`, `>`, `>=`, column-to-column comparisons | supported |
| `NOT`, `IN (...)`, `BETWEEN`, `IS [NOT] NULL`, `[NOT] LIKE` (`%`, `_`) | supported |
| `GROUP BY`, `HAVING`, `ORDER BY` (several keys, aliases), `LIMIT`, `OFFSET` | supported |
| `CAST`, arithmetic, other functions | rejected, with a targeted message |
| Subqueries, CTEs, window functions, `COUNT(DISTINCT ...)` | rejected |

Keywords are case-insensitive; column names are case-sensitive. A rejected
construct gets a message that names it and points at
[docs/scope.md](docs/scope.md). The
[FAQ](docs/faq.md) explains the reasons.

## Reading Parquet files

The import bridge reads Parquet into tpt-strata's native format with no
core-crate dependency on Parquet/Arrow:

```rust
use tpt_strata_parquet::{read_parquet, write_parquet};

let table = read_parquet("metrics.parquet")?;
write_parquet(&table, "copy.parquet")?;
```

Integers, floats, strings, booleans, dates and timestamps are read; narrow and
unsigned integers and `FLOAT` are widened. Other types, such as binary,
decimals and `UINT64`, are rejected with a diagnostic naming the column. Read
only the columns you need with `read_parquet_columns`. See the
[crate README](crates/tpt-strata-parquet/README.md) for the full mapping.

CSV and JSON need no extra crate: `tpt_strata::csv::parse_csv` and `to_csv`, and
`tpt_strata::json::parse_json` (an array of flat objects, or one object per line).

## Command line

The `strata` binary runs SQL over CSV and Parquet files. Each file becomes a
table named after the file:

```sh
cargo run -p tpt-strata-cli -- crates/tpt-strata/examples/data/orders.csv \
  "SELECT region, SUM(quantity) AS units FROM orders GROUP BY region ORDER BY units DESC"
```

Pass two files to join them, use `--explain` to print the plan, `--check` to
validate a query against the schemas, or `--csv` for CSV output. Run it with
no query for a prompt.

## Examples

Runnable end-to-end examples:

```sh
cargo run -p tpt-strata --example basic_query     # builder API end-to-end
cargo run -p tpt-strata --example sql_query       # SqlContext end-to-end (incl. JOIN)
cargo run -p tpt-strata --example csv_analytics   # CSV in, report out, plan printed
cargo run -p tpt-strata --example joins_and_aliases   # joins, HAVING, paging, check()
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

A three-way comparison with DataFusion and DuckDB on the same Parquet data is
in [docs/benchmark-comparison.md](docs/benchmark-comparison.md). The harness
lives in `bench/compare`, as a separate workspace.

## Optional `parallel` feature

The default build is single-threaded and has no dependencies. Build with the
`parallel` feature to run aggregation and the join probe across threads. It
uses only `std`:

```sh
cargo build -p tpt-strata --features parallel
```

Results are the same with or without the feature. The
[benchmark notes](docs/benchmark-comparison.md) show where it helps.

## Docs

- [docs/quickstart.md](docs/quickstart.md) — from nothing to a query in five
  minutes
- [docs/cookbook.md](docs/cookbook.md) — recipes for filters, aggregation,
  joins, paging, reading and writing files, and CI checks
- [docs/faq.md](docs/faq.md) — what it is for, what it is not for, NULL and
  number semantics, and what is not supported yet
- [docs/errors.md](docs/errors.md) — each error kind, a real message, and
  what to change
- [ARCHITECTURE.md](ARCHITECTURE.md) — crate boundaries, the format/engine
  split, and the bridge-isolation rule
- [docs/scope.md](docs/scope.md) — deliberate scope decisions
  (SQL features, data types, forward-looking ideas)
- [docs/integration-recipe.md](docs/integration-recipe.md) — a template for
  wiring tpt-strata into a struct-backed store
- [docs/benchmarks.md](docs/benchmarks.md) — zero-dependency benchmarks and
  the Phase 0 comparison records
- [docs/benchmark-comparison.md](docs/benchmark-comparison.md) — tpt-strata vs
  DataFusion vs DuckDB, and the `parallel` feature runs
- [PUBLISHING.md](PUBLISHING.md) — release steps for the crates.io packages

## License

MIT OR Apache-2.0, Copyright (c) TPT Solutions.