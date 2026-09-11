# Changelog

All notable changes to tpt-strata will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [1.0.0] - 2026-09-11

### Added

#### Core (`tpt-strata`)

- Native columnar in-memory format: `DataType` (`Boolean`, `Int32`, `Int64`,
  `Float64`, `Utf8`), nullable arrays, `Schema`/`Field`/`Batch`.
- Physical execution engine: `FilterExec`, `ProjectExec`, `AggregateExec`
  (SUM, COUNT, MIN, MAX, AVG, with GROUP BY), `JoinExec` (hash equi-join),
  `SortExec`, `LimitExec`.
- Builder API (`QueryBuilder` + `join()` free function) — zero trait
  implementations required for the common case.
- Hand-written SQL surface (`SqlContext`): `SELECT`/`FROM`/`JOIN ... ON`/
  `WHERE` (with `AND`)/`GROUP BY`/`ORDER BY`/`LIMIT`/trailing `;`.
  `COUNT(*)`, all five aggregates, numeric literal coercion to the compared
  column's type. AS aliases are accepted but the underlying expression name
  is used in the output (a v1 simplification).
- Diagnostics layer: `QueryError` with `MissingColumn` (lists available
  columns), `TypeMismatch` (names both types and the compared column),
  `UnsupportedOperation`, and `Sql` (malformed/unsupported SQL construct).
  Exact wording is pinned by snapshot regression tests.
- `Table` construction from Rust data (`Column::new` / `Table::try_new`).
- `print_table` for formatted output.
- Full rustdoc with runnable doctests for the builder API and `SqlContext`.
- Zero external dependencies (re-verified via `cargo tree`).

#### Bridge (`tpt-strata-parquet`)

- Parquet import bridge: `read_parquet(path)` reads a Parquet file into a
  `tpt_strata::Table`.
- Supported Parquet columns: `BOOLEAN`, `INT32`, `INT64`, `DOUBLE`,
  `STRING` (UTF-8). Unsupported types are rejected with a diagnostic naming
  the offending column.
- Null preservation across row groups.
- Tests using `ArrowWriter` to generate Parquet fixtures in a temp directory,
  with round-trip, unsupported-type, zero-row, and multi-row-group coverage.

#### Workspace and CI

- MIT/Apache-2.0 dual license (Copyright (c) TPT Solutions).
- CI (GitHub Actions): fmt check, clippy `-D warnings`, `test --all`,
  `build --all`, pinned MSRV (1.75) job, `cargo-deny` license/dependency
  compliance.
- `ARCHITECTURE.md` explaining the crate boundary, format/engine split, and
  bridge-isolation rule (spec.txt non-negotiables #1 and #2).
- `docs/benchmarks.md` with zero-dependency benchmarks and Phase 0
  incumbent comparison records.
- `CONTRIBUTING.md`, `SECURITY.md`.

[1.0.0]: https://github.com/tpt-solutions/tpt-strata/releases/tag/v1.0.0