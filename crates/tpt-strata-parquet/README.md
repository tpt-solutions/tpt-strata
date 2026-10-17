# tpt-strata-parquet

[![crates.io](https://img.shields.io/crates/v/tpt-strata-parquet.svg)](https://crates.io/crates/tpt-strata-parquet)
[![docs.rs](https://docs.rs/tpt-strata-parquet/badge.svg)](https://docs.rs/tpt-strata-parquet)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/tpt-solutions/tpt-strata/blob/master/LICENSE-MIT)

The Parquet import bridge for [tpt-strata](https://crates.io/crates/tpt-strata).

`tpt-strata-parquet` reads a Parquet file and returns a `tpt_strata::Table`,
which you can then query with the builder API or with SQL. It is the only crate
in the tpt-strata workspace that depends on Parquet and Arrow. The core
`tpt-strata` crate stays dependency-free.

## Contents

- [Why a separate crate](#why-a-separate-crate)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Supported types](#supported-types)
- [Errors](#errors)
- [Behavior and limits](#behavior-and-limits)
- [Examples](#examples)
- [Minimum supported Rust version](#minimum-supported-rust-version)
- [Related crates and docs](#related-crates-and-docs)
- [License](#license)

## Why a separate crate

tpt-strata's design has two non-negotiables that matter here:

1. The execution engine and its native columnar format do not depend on, or
   match the wire format of, any external in-memory columnar standard.
2. Interchange with other formats is a boundary concern, handled by a dedicated
   bridge crate that is isolated from the core.

This crate is that bridge. Keeping Parquet and Arrow here means that anyone who
does not need Parquet pays nothing for it. See
[`ARCHITECTURE.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/ARCHITECTURE.md)
for the full rule.

## Installation

```toml
[dependencies]
tpt-strata = "0.1"
tpt-strata-parquet = "0.1"
```

Both crates are released together and share a version number. The bridge pins
the exact core version it was built against.

## Quick start

```rust
use tpt_strata::{AggFunc, QueryBuilder, print_table};
use tpt_strata_parquet::read_parquet;

let table = read_parquet("metrics.parquet")?;

let result = QueryBuilder::new(&table)
    .group_by(vec!["region"])?
    .aggregate("latency_ms", AggFunc::Avg)?
    .execute()?;

print_table(&result);
```

`read_parquet` takes any `AsRef<Path>` and returns `Result<Table, QueryError>`.
The result is an ordinary `Table`, so everything in the core API works on it,
including `SqlContext`:

```rust
use tpt_strata::sql::SqlContext;

let mut ctx = SqlContext::new();
ctx.add_table("metrics", &table);
let result = ctx.run("SELECT region, MAX(latency_ms) FROM metrics GROUP BY region")?;
```

## Supported types

Each top-level column is widened losslessly to one of tpt-strata's four native
types:

| Parquet / Arrow type | tpt-strata `DataType` |
|---|---|
| `BOOLEAN` / `Boolean` | `Boolean` |
| `INT8`, `INT16`, `INT32`, `UINT8`, `UINT16` | `Int32` |
| `INT64`, `UINT32` | `Int64` |
| `FLOAT`, `DOUBLE` | `Float64` |
| `STRING`, `LARGE_STRING`, `STRING_VIEW` | `Utf8` |
| `DATE32` (days since 1970-01-01) | `Date` |
| `TIMESTAMP` (any unit, with or without a zone) | `Timestamp`, in microseconds since the epoch (a zone-aware value is read as UTC) |

Dates and timestamps are native types. They compare, sort, group and join, and
they print as ISO-8601 text. Writing a `Date` or `Timestamp` column produces
`DATE32` or `TIMESTAMP(MICROS)`.

Nullability is preserved. Null values in the file become `Scalar::Null` in the
table, across every row group.

Any other type is rejected. The error names the column and the type it found,
and lists the supported types. This includes:

- unsigned 64-bit integers (`UINT64`), which do not fit `i64` safely
- decimals
- binary
- nested types (lists, structs, maps)

To read a file that has columns of these types, select the columns you need
with `read_parquet_columns` (see below). Unselected columns are never decoded.

## Errors

Failures come back as `tpt_strata::QueryError`, and each variant says what went
wrong:

| Situation | Variant (`kind()`) | Message names |
|---|---|---|
| The file cannot be opened or written | `Io` (`io`) | The path, and the OS error (also available as `source()`). |
| The file is not valid Parquet, or cannot be decoded | `Import` (`import`) | The path and the decoder error. |
| A column has an unsupported type | `UnsupportedOperation` (`unsupported`) | The column name, its type, and the supported list. |
| A requested column does not exist | `MissingColumn` (`missing-column`) | The name and the columns the file has. |

## Reading part of a file, and in-memory data

```rust
use tpt_strata_parquet::{read_parquet_bytes, read_parquet_columns};

// Only these columns are decoded. Others can have unsupported types.
let t = read_parquet_columns("events.parquet", &["id", "amount"])?;

// Data that is already in memory, for example from an HTTP response.
let t = read_parquet_bytes(bytes::Bytes::from(body))?;
```

## Writing Parquet

`write_parquet(&table, path)` writes a `Table` back out with the same four
native types. A file written this way reads back with the same types and
nulls, which the crate's round-trip tests check.

## Behavior and limits

- **The whole file (or the selected columns) is loaded into memory.**
  `read_parquet` decodes every row group and builds one `Table`. It does not
  stream, and it does not push predicates down to the file. Use
  `read_parquet_columns` to avoid decoding columns you do not need.
- **Row groups are concatenated** into a single batch in file order.
- **Zero-row files** return an empty `Table` with the file's schema.

## Examples

```sh
# Writes a small sample file to the temp directory and reads it back.
cargo run -p tpt-strata-parquet --example read_parquet

# Reads a file you supply.
cargo run -p tpt-strata-parquet --example read_parquet -- path/to/file.parquet
```

The integration tests in `tests/import.rs` write fixtures with `ArrowWriter`
and cover typed columns with nulls, querying an imported table, widening of
small integer and float types, rejection of unsupported types, round trips
through `write_parquet`, reads from in-memory bytes, a missing file, a zero-row
file, and multiple row groups.

## Minimum supported Rust version

Rust **1.88**, set by `rust-version` in `Cargo.toml`. The core crate needs
1.75, but Arrow and Parquet 60, which this crate depends on, need 1.88.

## Related crates and docs

- [`tpt-strata`](https://crates.io/crates/tpt-strata): the core engine and
  the native format this crate produces.
- [`ARCHITECTURE.md`](https://github.com/tpt-solutions/tpt-strata/blob/master/ARCHITECTURE.md):
  crate boundaries and the bridge-isolation rule.
- [`CHANGELOG.md`](CHANGELOG.md): release history for this crate.

## License

Licensed under either of

- [MIT license](https://github.com/tpt-solutions/tpt-strata/blob/master/LICENSE-MIT)
- [Apache License, Version 2.0](https://github.com/tpt-solutions/tpt-strata/blob/master/LICENSE-APACHE)

at your option.

Copyright (c) TPT Solutions.
