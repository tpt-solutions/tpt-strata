# Changelog

All notable changes to tpt-strata will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

Each crate also keeps its own changelog, with only the changes that affect it:
[`tpt-strata`](crates/tpt-strata/CHANGELOG.md) and
[`tpt-strata-parquet`](crates/tpt-strata-parquet/CHANGELOG.md). This file
records workspace-wide changes (CI, docs, licensing, and release-level notes).

## [0.1.0] - 2026-10-18
### Added

- Native `DATE` and `TIMESTAMP` types, in the core crate and the Parquet
  bridge. They compare, sort, group and join. The Parquet bridge reads and
  writes them, and SQL accepts `DATE '...'` and `TIMESTAMP '...'` literals.

- SQL: `OR` and parentheses in `WHERE` and `HAVING`; `IN (...)`; `LEFT [OUTER]
  JOIN`; several joins in one query (each `ON` clause may refer to any table
  joined before it); column-to-column comparisons. `RIGHT`/`FULL` joins and
  `NOT` are still rejected with a targeted message.

- `tpt_strata::json::parse_json`: JSON arrays of flat objects, a single object,
  or newline-delimited objects. The CLI reads `.json` files.

- Columns are shared (`Arc`), so projecting a column copies no data. Joins read
  only the columns the query uses.

- Typed execution: grouping, aggregation, filtering, sorting and joins use
  borrowed typed keys with no per-row `Scalar`. A non-cryptographic hasher is
  used for the internal indexes.

- `tpt-strata-serde`: convert flat structs to a table (`to_table`) and read
  query results back into structs (`from_table`). Not published.

- `tpt-strata-wasm` and a browser playground in `playground/`: the engine
  compiled to WebAssembly, with a Node smoke test. Not published.

- `tpt-strata-python`: Python bindings (`import tpt_strata`) built with maturin,
  with `Table`, `sql`, `explain`, `check`, and `StrataError` carrying `kind`.
  Not published.

- `bench/compare`: a head-to-head benchmark of tpt-strata, DataFusion and DuckDB
  on the same Parquet data. See `docs/benchmark-comparison.md`.

- SQL: `IS [NOT] NULL`, `BETWEEN`, `[NOT] LIKE` (`%` and `_`), `DISTINCT`,
  `HAVING` (on any aggregate, even one not in `SELECT`), `OFFSET`, several
  `ORDER BY` keys, `COUNT(DISTINCT ...)` diagnostic, and `--`/`/* */` comments.

- CSV: `tpt_strata::csv::parse_csv` (type inference, quoting, NULL for empty
  fields) and `to_csv`. No dependencies.

- `Table::from_rows`, `Table::rename_columns`, `QueryBuilder::count_all()`.

- `SqlContext::explain` prints the physical plan; `SqlContext::check` binds a
  query without running it, for CI.

- `QueryError::kind()` returns a stable kebab-case name for each error kind.

- `strata` command-line tool (`crates/tpt-strata-cli`): SQL over CSV and
  Parquet files, joins across files, `--explain`, `--check`, `--csv`, and a
  prompt. Not published.

- Documentation: `docs/quickstart.md`, `docs/cookbook.md`, `docs/faq.md`; the
  `examples/csv_analytics.rs` and `examples/joins_and_aliases.rs` examples with
  sample data; the `templates/embedded-analytics` starter project.

- CI: tests on Linux, Windows and macOS; a rustdoc job that fails on warnings;
  a release-check workflow that verifies packaging and never uploads; dependabot;
  issue and pull request templates; CODEOWNERS.

- Targeted SQL diagnostics for the constructs that are still outside the
  subset: `CAST`, `COUNT(DISTINCT ...)`, and functions other than the
  aggregates. Each names the construct and points at `docs/scope.md`.
  Snapshot tests in `tests/diagnostics.rs`.

- Runnable examples: `examples/basic_query.rs` (builder),
  `examples/sql_query.rs` (SQL incl. JOIN), `examples/diagnostics.rs` (every
  `QueryError` variant), and
  `crates/tpt-strata-parquet/examples/read_parquet.rs` (bridge, with a
  self-generating sample file).

- `docs/scope.md` — deliberate scope decisions for deferred SQL
  features, data types, and forward-looking ideas.

- `docs/integration-recipe.md` — a template for wiring tpt-strata into a
  struct-backed store (for tpt-keystone-db, tpt-aion, tpt-cloud-observability).

- README badges (crates.io/docs.rs/CI/license), a condensed DataFusion
  comparison table, and an examples section.

- `SECURITY.md` now points at GitHub private vulnerability reporting instead
  of a placeholder email.

- `ARCHITECTURE.md` documents that `Sort`/`Join` fully materialize in memory
  (no streaming/spill).

### Changed

- **Behavior change:** grouped results come out in first-seen order. They were
  sorted by group key before. Add `ORDER BY` when the order matters.

- **Behavior change:** a whole float prints with a decimal point (`100.0`), so
  it does not look like an integer.

- **Behavior change:** `-0.0` and `0.0` sort as equal. They used to sort with
  `-0.0` first. `NaN` still sorts last.

- Per-crate minimum Rust versions are recorded in each `Cargo.toml`. The core
  and serde crates are verified on 1.75. The WASM crate needs 1.85. The
  Parquet, CLI and Python crates need 1.88, because Arrow and Parquet 60 do.
  The workspace as a whole needs 1.88.

- The `cargo-deny` config works with current cargo-deny (the removed
  `unlicensed` key is gone) and allows `CC0-1.0`, which Arrow's `tiny-keccak`
  build dependency uses. Dependabot updates Arrow and Parquet together.

- **Behavior change:** `AS` aliases now name output columns. They were parsed
  and ignored before, so `SUM(x) AS total` came back as `SUM(x)`.

- **Behavior change:** `SUM` over an integer column returns `Int64` (it was
  `Float64`), and overflow is an error instead of a rounded result.

- **Behavior change:** a comparison with `NULL` (`= NULL`, `!= NULL`) matches no
  rows. Use `IS NULL` / `IS NOT NULL`.

- `Table::batches()` returns `&[Arc<Batch>]`. Tables and sources share batches,
  so queries no longer copy the input.

- `JoinExec::new` is replaced by `JoinExec::try_new`, which validates keys and
  output names.

- `print_table` measures width in characters, shows the header for a zero-row
  result, and escapes embedded newlines.

- Parquet errors are now `QueryError::Io` and `QueryError::Import` (see the
  bridge changelog).

### Fixed

- SQL: `SELECT COUNT(*) ... WHERE ...` returned 0 when the query read no column
  above its WHERE clause. Fixed in the core crate; see its changelog.

- String and boolean range predicates (`>`, `>=`, `<`, `<=`) always returned no
  rows. They now compare in string and boolean order.

- Joins with a shared column name resolved to the wrong side. Qualified names
  resolve correctly, `ON b.id = a.id` works in either order, and an ambiguous
  unqualified name is an error that lists the choices.

- Float keys: `NaN` values now group and join together, and `-0.0` groups with
  `0.0`. Equality and hashing agree.

- `LIMIT 0`, and joins with an empty side, could panic in `ORDER BY` or
  projection. Empty results keep their schema.

- Integer comparisons go through exact integer arithmetic, so values above
  2^53 compare correctly. `WHERE id > 2.5` works on an integer column.

- `ORDER BY t.col` (qualified) now resolves. `SELECT *, col` keeps `col`.
  Quoted identifiers containing a dot (`"http.status"`) resolve as one name.

- Table-qualified references with an unknown table now report a missing column
  instead of silently using the bare name.

- `Table::concat` accepts inputs that differ only in nullability.

- Builder `project()` after an aggregate works; it was ignored.

- The "Did you mean" suggestion is separated from the column list by a full stop.

- `Array::get` and `is_null` return null for an out-of-range index instead of
  panicking.

- `-9223372036854775808` is accepted as a literal. `9223372036854775808` on its
  own is still an error.

- `ArrayRef::take` returns NULL for an out-of-range index instead of panicking.

- `ORDER BY` no longer mishandles `NULL`: nulls now sort **first** ascending
  (`NULLS FIRST`) and last descending, instead of landing in an arbitrary
  position via `scalar_cmp`'s equal-for-incomparable fallback. New
  `sort_cmp` helper in `types.rs`; regression tests in `tests/fixes.rs`.

- `MIN`/`MAX` now validate the column is orderable at plan-build time,
  mirroring the numeric-only guard `SUM`/`AVG` already had. Every native
  type is orderable today, so this is a future-proofing guard, not a behavior
  change for current types.

- Internal `QueryError::unsupported_format` renamed to
  `QueryError::internal_downcast` and re-framed as a "should never happen"
  coding-bug path rather than a caller-facing diagnostic.

- Fragile `.expect()` calls in `sql.rs` (wildcard expansion) and `format.rs`
  (equal column lengths) are now annotated with their invariants and a
  `debug_assert!`.

## Pre-release history

Internal milestones. They were never published to crates.io, and their
changes ship in 0.1.0.

### 1.0.1 (internal, 2026-09-11)

#### Fixed

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

#### Added

- Regression tests covering all of the above (`tests/fixes.rs`).

### 1.0.0 (internal, 2026-09-11)

#### Added

##### Core (`tpt-strata`)

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

##### Bridge (`tpt-strata-parquet`)

- Parquet import bridge: `read_parquet(path)` reads a Parquet file into a
  `tpt_strata::Table`.
- Supported Parquet columns: `BOOLEAN`, `INT32`, `INT64`, `DOUBLE`,
  `STRING` (UTF-8). Unsupported types are rejected with a diagnostic naming
  the offending column.
- Null preservation across row groups.
- Tests using `ArrowWriter` to generate Parquet fixtures in a temp directory,
  with round-trip, unsupported-type, zero-row, and multi-row-group coverage.

##### Workspace and CI

- MIT/Apache-2.0 dual license (Copyright (c) TPT Solutions).
- CI (GitHub Actions): fmt check, clippy `-D warnings`, `test --all`,
  `build --all`, pinned MSRV (1.75) job, `cargo-deny` license/dependency
  compliance.
- `ARCHITECTURE.md` explaining the crate boundary, format/engine split, and
  bridge-isolation rule (spec.txt non-negotiables #1 and #2).
- `docs/benchmarks.md` with zero-dependency benchmarks and Phase 0
  incumbent comparison records.
- `CONTRIBUTING.md`, `SECURITY.md`.

[0.1.0]: https://github.com/tpt-solutions/tpt-strata/releases/tag/v0.1.0
