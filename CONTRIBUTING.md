# Contributing to tpt-strata

## Getting Started

1. Install the Rust toolchain (see MSRV in `Cargo.toml`).
2. Clone the repository.
3. Run `cargo build --workspace` and `cargo test --workspace`.

## Project Structure

This is a Cargo workspace. Core crates must not add external dependencies.

- `crates/tpt-strata` — the core engine. **Zero external dependencies.**
- `crates/tpt-strata-parquet` — the only crate allowed to depend on external
  columnar interop libraries.

## Rules

- **Never add an external dependency to `tpt-strata` core.** This is a
  hard requirement. If a feature seems to need one, discuss it first.
- External interop (Parquet, Arrow, etc.) lives only in the bridge crate.
- Run `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test --all` before submitting.
- Keep diagnostics caller-facing: name the caller's columns and types.
- Add tests for any new error message — wording is a contract.

## Where things live

| Area | Files |
|---|---|
| Values, comparisons, NULL and NaN rules | `crates/tpt-strata/src/types.rs` |
| Physical operators (scan, filter, project, aggregate, join, sort, limit) | `crates/tpt-strata/src/engine.rs` |
| Builder API | `crates/tpt-strata/src/query.rs` |
| SQL lexer, parser, binder and planner | `crates/tpt-strata/src/sql.rs` |
| Errors and "did you mean" | `crates/tpt-strata/src/error.rs` |
| Tables, rendering, `from_rows` | `crates/tpt-strata/src/format.rs` |
| CSV reader and writer | `crates/tpt-strata/src/csv.rs` |
| Parquet import and export | `crates/tpt-strata-parquet/src/lib.rs` |
| The `strata` command | `crates/tpt-strata-cli/src/main.rs` |

Tests:

- `tests/regressions.rs`: one test per fixed bug. Add a test here for each fix.
- `tests/diagnostics.rs`: the exact wording of every error the user can see.
- `tests/sql.rs`, `tests/engine.rs`, `tests/integration.rs`: queries and operators.
- `tests/csv.rs`: CSV reading and writing.
- `crates/tpt-strata-parquet/tests/import.rs`: Parquet fixtures written with
  `ArrowWriter`, plus round trips.

## Adding a SQL feature

1. Start with the grammar: the parser in `sql.rs` (`parse_pred`,
   `parse_select`). Reject anything you do not implement with
   `unsupported_v1_1`, so the user gets a named diagnostic.
2. Bind it: `build_plan` resolves names with `resolve`, which is the one place
   that decides how a column reference maps to a field. Do not resolve names
   anywhere else.
3. Execute it: add an operator in `engine.rs`, or reuse one. Give the operator
   an `explain` line so `SqlContext::explain` shows it.
4. Test it: a positive test in `regressions.rs`, a test for the rejected
   variants in `diagnostics.rs`, and update `docs/scope.md` and the README
   grammar.

## Adding a data type

A new `DataType` touches the whole native format: `Scalar`, `DataType`, the
array types, `numeric_cmp` and `compare_values`, `format.rs` inference, the CSV
reader, and the Parquet bridge. Add it only when a real consumer schema needs
it, and write the list of places in the pull request. Prefer widening an
imported type to a native one (see the bridge README) when that is lossless.

## Diagnostics

Error text is a contract. If you change a message, update the matching test in
`tests/diagnostics.rs` in the same change. Name the caller's columns and types,
and say what the user can do next. Keep `QueryError::kind()` stable: tools
branch on it.

## Commit Messages

Follow conventional-commit style:

```
feat: add join operator
fix: correct sort ordering for nullable columns
docs: clarify bridge isolation rule
```

## CI

CI enforces formatting, linting, tests, and the MSRV. Local runs should pass
the same checks before a PR is opened.