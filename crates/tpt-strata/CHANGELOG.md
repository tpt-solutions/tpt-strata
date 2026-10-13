# Changelog

All notable changes to the `tpt-strata` crate will be documented in this file.

This file covers the core crate only. Workspace-wide changes (CI, docs, and
the Parquet bridge) are recorded in the [root changelog](../../CHANGELOG.md).

The format is based on [Keep a Changelog](https://keepachangelog.com/), and
this crate follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- SQL: `NOT`, `NOT IN` and `NOT BETWEEN`. `NOT` is pushed down to the
  comparisons, so NULL rows behave as in SQL.
- SQL: `RIGHT` and `FULL` joins (with optional `OUTER`).
- `JoinKind` and `JoinExec::try_new_join`. `try_new_kind(bool)` still works and
  maps to `Left` or `Inner`.
- `docs/errors.md`: one section per error `kind()`, with real messages.

### Changed

- `JoinExec` plans print as `Left HashJoin`, `Right HashJoin` or `Full HashJoin`
  instead of `HashJoin` for outer joins.

- **Behavior change:** grouped results come out in first-seen order, not sorted
  by key. A whole float prints as `100.0`. `-0.0` sorts equal to `0.0`.
- `Batch::columns` holds shared columns (`Arc<ArrayRef>`). `Batch::new_shared`
  builds a batch from columns without copying them. Existing constructors are
  unchanged.
- `ArrayRef::take` returns NULL for an out-of-range index.
- **Behavior change:** `AS` aliases name output columns, and work in `ORDER BY`
  and `HAVING`. They used to be parsed and ignored.
- **Behavior change:** `SUM` over an integer column returns `Int64` and fails on
  overflow. It used to return `Float64`.
- **Behavior change:** comparisons with `NULL` match no rows. Use `IS NULL`.
- `Table::batches()` returns `&[Arc<Batch>]`, and `DataSource` shares batches.
  Queries no longer copy their input.
- `JoinExec::new` is replaced by `JoinExec::try_new`, which validates key
  indices, key types, and output names. `QueryError` gains `Io` and `Import`
  variants and a `kind()` method.
- `print_table` measures width in characters and shows the header for zero-row
  results. `render_table` returns the same grid as a `String`.

### Added

- SQL: `OR`, parentheses, `IN (...)`, `LEFT [OUTER] JOIN`, several joins, and
  column-to-column comparisons. `-9223372036854775808` is a valid literal.
- `json` module: `parse_json` reads arrays of flat objects, a single object, or
  one object per line, with the same type inference as `csv`.
- `key` module: borrowed typed keys (`KeyPart`) and a fast hasher used for the
  engine's internal indexes. `aggregate` module: typed grouped aggregation.
- SQL: `IS [NOT] NULL`, `BETWEEN`, `[NOT] LIKE`, `DISTINCT`, `HAVING`, `OFFSET`,
  several `ORDER BY` keys, `--` and `/* */` comments, exponent literals, and
  doubled quotes in quoted identifiers.
- `SqlContext::explain` and `SqlContext::check`.
- `csv` module: `parse_csv` and `to_csv`.
- `Table::from_rows` and `Table::rename_columns`; `QueryBuilder::count_all()`.
- `Comparison::symbol()`, used by the plan printer.

### Fixed

- String and boolean range predicates always returned no rows. They now compare.
- Joins with a shared column name resolved to the wrong side. Qualified names
  resolve correctly, the `ON` clause works in either order, and an ambiguous
  unqualified name is an error.
- Float keys: `NaN` groups together, and `-0.0` groups with `0.0`.
- Empty inputs (`LIMIT 0`, a join with an empty side) no longer risk a panic.
- Integer comparisons are exact above 2^53, and `WHERE id > 2.5` works on an
  integer column.
- `ORDER BY t.col` resolves. `SELECT *, col` keeps `col`. Quoted identifiers
  with a dot resolve as one name.
- `Table::concat` accepts inputs that differ only in nullability.
- Builder `project()` after an aggregate is honoured instead of ignored.
- `Array::get` and `is_null` return null for an out-of-range index, not a panic.
- `ORDER BY` no longer mishandles `NULL`: nulls now sort **first** ascending
  (`NULLS FIRST`) and last descending, instead of landing in an arbitrary
  position via `scalar_cmp`'s equal-for-incomparable fallback. New `sort_cmp`
  helper in `types.rs`; regression tests in `tests/fixes.rs`.
- `MIN`/`MAX` now validate that the column is orderable at plan-build time,
  mirroring the numeric-only guard `SUM`/`AVG` already had. Every native v1
  type is orderable today, so this is a future-proofing guard, not a behavior
  change for current types.
- Internal `QueryError::unsupported_format` renamed to
  `QueryError::internal_downcast` and re-framed as a "should never happen"
  coding-bug path rather than a caller-facing diagnostic.
- Fragile `.expect()` calls in `sql.rs` (wildcard expansion) and `format.rs`
  (equal column lengths) are annotated with their invariants and guarded by a
  `debug_assert!`.

### Added

- Targeted SQL diagnostics for the deliberately deferred v1.1 constructs:
  `OR`, parentheses, `IS NULL`/`IS NOT NULL`, `IN`, `BETWEEN`, `LIKE`,
  `DISTINCT`, `HAVING`, `LEFT`/`OUTER` joins, multiple joins, and
  column-to-column comparisons each get a message naming the construct and
  pointing at `docs/v1.1-scope.md`. Snapshot tests in `tests/diagnostics.rs`.
- Runnable examples: `examples/basic_query.rs` (builder API),
  `examples/sql_query.rs` (SQL, including JOIN), and `examples/diagnostics.rs`
  (every `QueryError` variant).
- `ARCHITECTURE.md` notes that `Sort` and `Join` fully materialize their input
  in memory (no streaming or spill).

## [1.0.1] - 2026-09-11

### Fixed

- `COUNT(col)` no longer counts `NULL` rows. `COUNT(*)` still counts every row;
  an engine-level `count_star` flag distinguishes the two.
- `SUM`/`AVG` return `NULL` when the column or group has no non-null values,
  instead of `0.0`. `SUM(dept)` on a string column now fails with a diagnostic
  naming the column and its type rather than silently returning `0.0`.
- `Table::try_new` rejects columns that mix value types. Previously the
  mismatched values were silently dropped and stored as `NULL`.
- `ORDER BY` no longer matches columns by substring. Unknown names produce a
  `MissingColumn` diagnostic listing the actual columns.
- `JOIN ... ON` no longer matches `NULL` keys (`NULL = NULL` no longer matches),
  matching SQL semantics.

### Added

- Regression tests covering all of the above (`tests/fixes.rs`).

## [1.0.0] - 2026-09-11

### Added

- Native columnar in-memory format: `DataType` (`Boolean`, `Int32`, `Int64`,
  `Float64`, `Utf8`), nullable arrays, and `Schema`/`Field`/`Batch`.
- Physical execution engine: `FilterExec`, `ProjectExec`, `AggregateExec`
  (SUM, COUNT, MIN, MAX, AVG, with GROUP BY), `JoinExec` (hash equi-join),
  `SortExec`, and `LimitExec`.
- Builder API (`QueryBuilder` and the `join()` free function). No trait
  implementations are required for the common case.
- Hand-written SQL surface (`SqlContext`): `SELECT`, `FROM`, `JOIN ... ON`,
  `WHERE` (with `AND`), `GROUP BY`, `ORDER BY`, `LIMIT`, and a trailing `;`.
  Supports `COUNT(*)`, all five aggregates, and numeric literal coercion to the
  compared column's type. `AS` aliases are accepted, but the output keeps the
  underlying expression name (a v1 simplification).
- Diagnostics layer: `QueryError` with `MissingColumn` (lists available
  columns), `TypeMismatch` (names both types and the compared column),
  `UnsupportedOperation`, and `Sql` (malformed or unsupported SQL). Exact
  wording is pinned by snapshot regression tests.
- `Table` construction from Rust data (`Column::new` / `Table::try_new`).
- `print_table` for formatted output.
- Rustdoc with runnable doctests for the builder API and `SqlContext`.
- Zero external dependencies, re-verified with `cargo tree`.

[Unreleased]: https://github.com/tpt-solutions/tpt-strata/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/tpt-solutions/tpt-strata/releases/tag/v1.0.1
[1.0.0]: https://github.com/tpt-solutions/tpt-strata/releases/tag/v1.0.0
