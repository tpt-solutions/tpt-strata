# Changelog

All notable changes to the `tpt-strata-parquet` crate will be documented in
this file.

This file covers the Parquet bridge only. Workspace-wide changes are recorded
in the [root changelog](../../CHANGELOG.md), and core engine changes are in the
[`tpt-strata` changelog](../tpt-strata/CHANGELOG.md).

The format is based on [Keep a Changelog](https://keepachangelog.com/), and
this crate follows [Semantic Versioning](https://semver.org/).

## [0.1.0] - unreleased
### Added

- `write_parquet(&table, path)`: writes a table with the same four native types
  it reads, so files round-trip.

- `read_parquet_columns(path, &[names])`: reads only the named columns, so a
  file with an unsupported column can still be read.

- `read_parquet_bytes(bytes)`: reads Parquet data already in memory.

- Runnable example `examples/read_parquet.rs`. With no argument it writes a
  small sample file to the temp directory and reads it back, so it runs out of
  the box. Pass a path to read your own file.

### Changed

- **Minimum Rust version is 1.88** (was 1.85). Arrow and Parquet are updated
  from 55 to 60, which need it. Gzip pages use the pure-Rust `zlib-rs` backend.

- **Behavior change:** supported types are wider. `INT8`, `INT16`, `UINT8`,
  `UINT16`, `UINT32`, `FLOAT`, `LARGE_STRING`, `STRING_VIEW`, `DATE32` and
  `TIMESTAMP` are imported (widened to `i32`, `i64`, `f64` or `str`). `DATE32` and `TIMESTAMP` import as
  native `Date` and `Timestamp` values. A zone-aware timestamp is read as UTC.
  Binary and `UINT64` are still rejected.

- Errors are split by kind. A file that cannot be opened or written is
  `QueryError::Io` (with `source()`). A file that cannot be decoded is
  `QueryError::Import`. Unsupported column types stay `UnsupportedOperation`.

## Pre-release history

Internal milestones. They were never published to crates.io, and their
changes ship in 0.1.0.

### 1.0.1 (internal, 2026-09-11)

#### Changed

- Version bump in lockstep with `tpt-strata` 1.0.1. The bridge's own code did
  not change. The fixes in that release are in the core crate.

### 1.0.0 (internal, 2026-09-11)

#### Added

- Parquet import bridge: `read_parquet(path)` reads a Parquet file into a
  `tpt_strata::Table`.
- Supported Parquet columns: `BOOLEAN`, `INT32`, `INT64`, `DOUBLE`, and
  `STRING` (UTF-8). Unsupported types are rejected with a diagnostic naming
  the offending column.
- Null preservation across row groups.
- Integration tests in `tests/import.rs` that generate Parquet fixtures with
  `ArrowWriter` in a temp directory. They cover round-trip reads, unsupported
  types, zero-row files, and multi-row-group files.

[0.1.0]: https://github.com/tpt-solutions/tpt-strata/releases/tag/v0.1.0
