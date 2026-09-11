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