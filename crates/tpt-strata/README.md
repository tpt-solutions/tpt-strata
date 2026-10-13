# tpt-strata

[![crates.io](https://img.shields.io/crates/v/tpt-strata.svg)](https://crates.io/crates/tpt-strata)
[![docs.rs](https://docs.rs/tpt-strata/badge.svg)](https://docs.rs/tpt-strata)
[![CI](https://github.com/tpt-solutions/tpt-strata/actions/workflows/ci.yml/badge.svg)](https://github.com/tpt-solutions/tpt-strata/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/tpt-solutions/tpt-strata/blob/master/LICENSE-MIT)

An embeddable columnar query engine for analytical, in-process queries, with
**zero external dependencies**.

tpt-strata runs `GROUP BY`-style aggregations, filters, joins, sorts and limits
over in-memory columnar data. You can drive it with a Rust builder API or with a
small hand-written SQL surface. There is no JVM, no server process, and no
large trait hierarchy to implement.

## Contents

- [Features](#features)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Building tables](#building-tables)
- [Query builder API](#query-builder-api)
- [SQL API](#sql-api)
- [Joins](#joins)
- [Errors and diagnostics](#errors-and-diagnostics)
- [Data model](#data-model)
- [Limitations](#limitations)
- [Examples](#examples)
- [Minimum supported Rust version](#minimum-supported-rust-version)
- [Related crates and docs](#related-crates-and-docs)
- [License](#license)

## Features

- **Native columnar format.** Typed, nullable columns grouped into `Table`s and
  `Batch`es. The format is independent of Arrow and Parquet.
- **Physical execution engine.** Filter, project, aggregate (with `GROUP BY`),
  hash equi-join, sort, and limit operators.
- **Builder API.** `QueryBuilder` composes a query in Rust with no trait
  implementations required.
- **SQL surface.** `SqlContext` parses a deliberately small SQL subset with a
  hand-written parser, so the core keeps its zero-dependency promise.
- **Null-aware semantics.** `COUNT(col)` skips `NULL`s, `COUNT(*)` counts every
  row, `SUM`/`AVG` return `NULL` for empty input, and `NULL` never matches a
  join key.
- **Diagnostics first.** Errors name your own columns and types, and suggest
  the closest column name when one is misspelled.

## Installation

```toml
[dependencies]
tpt-strata = "1.0"
```

The crate has no external dependencies. Check with `cargo tree -p tpt-strata`.

To read Parquet files, add the companion bridge crate
[`tpt-strata-parquet`](https://crates.io/crates/tpt-strata-parquet). The core
crate does not depend on it.

## Quick start

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

The same query as SQL:

```rust
use tpt_strata::{Table, Column, Scalar, sql::SqlContext};

let mut ctx = SqlContext::new();
ctx.add_table("employees", &table);

let result = ctx.run(
    "SELECT department, AVG(salary) FROM employees GROUP BY department",
)?;
```

## Building tables

A `Column` is a name plus a vector of `Scalar` values. A `Table` is built from
columns, and `Table::try_new` checks the whole set at once.

```rust
use tpt_strata::{Column, Scalar, Table};

let table = Table::try_new(vec![
    Column::new("id", vec![Scalar::I64(1), Scalar::I64(2), Scalar::Null]),
    Column::new("name", vec![Scalar::Str("a".into()), Scalar::Null, Scalar::Str("c".into())]),
])?;

assert_eq!(table.row_count(), 3);
assert_eq!(table.column_names(), vec!["id", "name"]);
```

`Table::try_new` rejects:

- a column whose values mix types (for example `I64` and `Str` in one column),
  with a diagnostic naming the column;
- a column that is entirely `Scalar::Null`, because its type cannot be inferred
  (construct the schema explicitly instead);
- columns of unequal length.

A column's type is inferred from its first non-null value. A column is marked
nullable if it contains any `Scalar::Null`.

Useful accessors on `Table`: `schema()`, `batches()`, `row_count()`,
`column_names()`, `column_index(name)`, `concat(&other)`, and
`Table::empty(schema)`.

Use `print_table(&table)` to print a result in a readable grid.

## Query builder API

`QueryBuilder::new(&table)` starts a query. Each method returns
`Result<Self, QueryError>`, so mistakes fail at the call that caused them.

| Method | Purpose |
|---|---|
| `filter(column, op, value)` | Keep rows where `column op value`. `op` is one of `=`, `!=`, `<`, `<=`, `>`, `>=`. |
| `project(vec![...])` | Keep only the named columns. |
| `group_by(vec![...])` | Group rows by one or more columns. |
| `aggregate(column, AggFunc)` | Apply `Sum`, `Count`, `Min`, `Max`, or `Avg` to a column. |
| `order_by(column, desc)` | Sort the result. `desc = false` sorts ascending. |
| `limit(n)` | Keep at most `n` rows. |
| `execute()` | Run the query and return a `Table`. |

```rust
use tpt_strata::{AggFunc, QueryBuilder, Scalar};

let result = QueryBuilder::new(&table)
    .filter("salary", ">=", Scalar::F64(80000.0))?
    .group_by(vec!["department"])?
    .aggregate("salary", AggFunc::Sum)?
    .order_by("department", false)?
    .limit(10)
    .execute()?;
```

`filter` checks the literal against the column's type. A `Scalar::Null` literal
is accepted for any column.

### Aggregate semantics

| Function | Behavior |
|---|---|
| `COUNT(*)` | Counts every row, including rows with `NULL`s. |
| `COUNT(col)` | Counts non-null values only. |
| `SUM(col)` | Numeric columns only. Returns `NULL` when the group has no non-null values. |
| `AVG(col)` | Numeric columns only. Returns `NULL` when the group has no non-null values. |
| `MIN(col)` / `MAX(col)` | Ordering is checked when the plan is built. Every current type is orderable. |

## SQL API

`SqlContext` holds named tables and runs a query against them:

```rust
use tpt_strata::sql::SqlContext;

let mut ctx = SqlContext::new();
ctx.add_table("orders", &orders);
ctx.add_table("customers", &customers);

let result = ctx.run(
    "SELECT name, SUM(amount) FROM orders JOIN customers ON orders.cust_id = customers.id \
     WHERE amount >= 10 GROUP BY name ORDER BY name LIMIT 5;",
)?;
```

The supported grammar:

```text
SELECT [DISTINCT] <column | agg(col) | agg(*) | *> [AS alias] [, ...]
  FROM <table> [[LEFT | RIGHT | FULL] [OUTER] JOIN <table> ON left.col = right.col]...
  [WHERE cond]
  [GROUP BY col [, ...]]
  [HAVING cond]
  [ORDER BY col-or-alias [ASC | DESC] [, ...]]
  [LIMIT n] [OFFSET n] [;]

cond := primary [AND | OR primary ...]     -- AND binds tighter than OR
primary := NOT primary | ( cond ) | pred
pred := col op literal | col op col
      | col [NOT] IN (literal, ...) | col [NOT] BETWEEN literal AND literal
      | col IS [NOT] NULL | col [NOT] LIKE 'pattern'
```

- Keywords are case-insensitive. Column and table names are case-sensitive.
  Quote a name with `"..."` to use characters such as `.` or spaces.
- Aggregates: `SUM`, `COUNT` (including `COUNT(*)`), `MIN`, `MAX`, `AVG`.
  An aggregate can appear in `HAVING` and `ORDER BY` even when it is not in
  `SELECT`.
- Comparison operators: `=`, `!=`, `<>`, `>`, `>=`, `<`, `<=`.
- `LIKE` supports `%` (any run of characters) and `_` (one character).
- Literals: integers, floats (`1.5`, `1e3`), single-quoted strings, `TRUE`,
  `FALSE`, `NULL`. Integer and float literals compare exactly against numeric
  columns.
- `AS` names an output column. The alias can be used in `ORDER BY` and `HAVING`.
- Comments: `-- to end of line` and `/* block */`.

### Constructs outside the subset

These are rejected on purpose, each with a targeted message that names the
construct and points at [`docs/v1.1-scope.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/docs/v1.1-scope.md):

- `CAST`, `COUNT(DISTINCT ...)`, and arithmetic or functions other than the
  aggregates

Each one is easy to express in the supported subset. The v1.1 scope document
records the reasoning and the rewrite for each.

Each one is easy to express in the supported subset. The v1.1 scope document
records the reasoning and the rewrite for each.

## Joins

Two forms are available:

```rust
use tpt_strata::join;

// Free function: inner equi-join of two tables on one column each.
let joined = join(&orders, &customers, "cust_id", "id")?;
```

or the SQL form `FROM a JOIN b ON a.col = b.col`.

The join is a hash equi-join. Rows whose key is `NULL` never match, so
`NULL = NULL` is not a match.

## Errors and diagnostics

Every failure is a `QueryError`:

| Variant | When it happens |
|---|---|
| `MissingColumn { column, available }` | A column name does not exist. The message lists the available columns and suggests the closest match. |
| `TypeMismatch { column, expected, actual, context }` | An operation does not fit the column's type, for example `SUM` on a string column. |
| `UnsupportedOperation { message }` | The operation is not supported at this layer. |
| `Sql { message }` | The SQL string is malformed, or uses a deferred construct. |

Run `cargo run -p tpt-strata --example diagnostics` to print every variant.
The exact wording is pinned by snapshot tests in `tests/diagnostics.rs`.

## Data model

| Type | Meaning |
|---|---|
| `DataType` | `Boolean`, `Int32`, `Int64`, `Float64`, `Utf8`. |
| `Scalar` | One value: `Null`, `Bool`, `I32`, `I64`, `F64`, `Str`. |
| `Field` / `Schema` | A named, typed, nullable column description and an ordered list of fields. |
| `Column` | A named vector of `Scalar` values, used to build a `Table`. |
| `Table` | Schema plus one or more `Batch`es. |
| `Batch` | A horizontal slice of a table, holding one array per column. |
| `Array` / `ArrayRef` | Typed, nullable column storage (`BoolArray`, `I32Array`, `I64Array`, `F64Array`, `StrArray`). |

Engine internals (`PhysicalPlan`, `FilterExec`, `ProjectExec`, `AggregateExec`,
`JoinExec`, `SortExec`, `LimitExec`) are public in `tpt_strata::engine` if you
want to compose plans yourself.

## Limitations

- **In-memory only.** Data must fit in memory.
- **`Sort` and `Join` fully materialize their input.** There is no streaming or
  spill-to-disk. See [`ARCHITECTURE.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/ARCHITECTURE.md).
- **Five native types.** Anything else needs converting before it reaches the
  engine. Use [`tpt-strata-parquet`](https://crates.io/crates/tpt-strata-parquet)
  for Parquet input.
- **Equi-joins only.** Inner, LEFT, RIGHT and FULL joins, chained, on one key
  column per side. Non-equality join conditions are not supported.
- **No query optimizer.** Plans run as written.

## Examples

```sh
cargo run -p tpt-strata --example basic_query     # builder API end-to-end
cargo run -p tpt-strata --example sql_query       # SqlContext end-to-end (incl. JOIN)
cargo run -p tpt-strata --example diagnostics     # every QueryError variant, printed
```

Benchmarks: `cargo bench -p tpt-strata`. Results and methodology are in
[`docs/benchmarks.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/docs/benchmarks.md).

## Minimum supported Rust version

Rust **1.75**. CI checks this with a pinned toolchain.

## Related crates and docs

- [`tpt-strata-parquet`](https://crates.io/crates/tpt-strata-parquet): reads
  Parquet files into a `Table`. This is the only crate in the workspace that
  depends on Arrow or Parquet.
- [`ARCHITECTURE.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/ARCHITECTURE.md):
  crate boundaries and the bridge-isolation rule.
- [`docs/v1.1-scope.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/docs/v1.1-scope.md):
  deliberate scope decisions for deferred SQL features and types.
- [`docs/integration-recipe.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/docs/integration-recipe.md):
  wiring tpt-strata into a struct-backed store.
- [`CHANGELOG.md`](CHANGELOG.md): release history for this crate.

## License

Licensed under either of

- [MIT license](https://github.com/tpt-solutions/tpt-strata/blob/master/LICENSE-MIT)
- [Apache License, Version 2.0](https://github.com/tpt-solutions/tpt-strata/blob/master/LICENSE-APACHE)

at your option.

Copyright (c) TPT Solutions.
