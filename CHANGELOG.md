# Changelog

All notable changes to tpt-strata will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Fixed

- `ORDER BY` no longer mishandles `NULL`: nulls now sort **first** ascending
  (`NULLS FIRST`) and last descending, instead of landing in an arbitrary
  position via `scalar_cmp`'s equal-for-incomparable fallback. New
  `sort_cmp` helper in `types.rs`; regression tests in `tests/fixes.rs`.
- `MIN`/`MAX` now validate the column is orderable at plan-build time,
  mirroring the numeric-only guard `SUM`/`AVG` already had. Every native v1
  type is orderable today, so this is a future-proofing guard, not a behavior
  change for current types.
- Internal `QueryError::unsupported_format` renamed to
  `QueryError::internal_downcast` and re-framed as a "should never happen"
  coding-bug path rather than a caller-facing diagnostic.
- Fragile `.expect()` calls in `sql.rs` (wildcard expansion) and `format.rs`
  (equal column lengths) are now annotated with their invariants and a
  `debug_assert!`.

### Added

- Targeted SQL diagnostics for the deliberately-deferred v1.1 constructs:
  `OR`, parentheses, `IS NULL`/`IS NOT NULL`, `IN`, `BETWEEN`, `LIKE`,
  `DISTINCT`, `HAVING`, `LEFT`/`OUTER` joins, multiple joins, and
  column-to-column comparisons each get a message naming the construct and
  pointing at `docs/v1.1-scope.md`. Snapshot tests in `tests/diagnostics.rs`.
- Runnable examples: `examples/basic_query.rs` (builder),
  `examples/sql_query.rs` (SQL incl. JOIN), `examples/diagnostics.rs` (every
  `QueryError` variant), and
  `crates/tpt-strata-parquet/examples/read_parquet.rs` (bridge, with a
  self-generating sample file).
- `docs/v1.1-scope.md` — deliberate scope decisions for deferred SQL
  features, data types, and forward-looking ideas.
- `docs/integration-recipe.md` — a template for wiring tpt-strata into a
  struct-backed store (for tpt-keystone-db, tpt-aion, tpt-cloud-observability).
- README badges (crates.io/docs.rs/CI/license), a condensed DataFusion
  comparison table, and an examples section.
- `SECURITY.md` now points at GitHub private vulnerability reporting instead
  of a placeholder email.
- `ARCHITECTURE.md` documents that `Sort`/`Join` fully materialize in memory
  (no streaming/spill).

## [1.0.1] - 2026-09-11

### Fixed

- `COUNT(col)` no longer counts `NULL` rows; `COUNT(*)` still counts every
  row (engine-level `count_star` flag distinguishes the two).
- `SUM`/`AVG` now return `NULL` when the column or group has no non-null
  values, instead of `0.0` (`SUM(dept)` on a string column now fails with a
  diagnostic naming the column and its type rather than silently returning
  `0.0`).
- `Table::try_new` rejects columns that mix value types (previously the
  mismatched values were silently dropped and stored as `NULL`).
- `ORDER BY` no longer matches columns by substring; unknown names produce a
  `MissingColumn` diagnostic listing the actual columns.
- `JOIN ... ON` no longer matches `NULL` keys (`NULL = NULL` no longer
  matches), matching SQL semantics.

### Added

- Regression tests covering all of the above (`tests/fixes.rs`).

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

[1.0.1]: https://github.com/tpt-solutions/tpt-strata/releases/tag/v1.0.1
[1.0.0]: https://github.com/tpt-solutions/tpt-strata/releases/tag/v1.0.0