# tpt-strata — Project Roadmap

License: MIT OR Apache-2.0 · Copyright TPT Solutions

> Status: Conditional. Do not begin Phase 1 until every Phase 0 checkbox
> is checked and the go/no-go call is a **go**.

## Phase 0 — Validation & Go/No-Go

- [ ] Run "point at data, get an answer" against the incumbent's *current*
      release: record exact line count and any trait implementations required
- [ ] Repeat the same basic case in a tpt-strata prototype/sketch; compare
      line count and trait/boilerplate burden directly
- [ ] Diagnostic comparison: pick one broken query (bad type, missing
      column, unsupported cast), run it on the incumbent, capture the exact
      error text a caller sees
- [ ] Produce the equivalent tpt-strata diagnostic for the same broken query
      and compare wording/clarity side by side
- [ ] Decide minimal viable bridge surface for v1: confirm export is
      actually needed at launch, or defer it
- [ ] Write up the Section 3 validation result (ease-of-use, compile times/
      error messages, integration boilerplate) as a short go/no-go memo
- [ ] **Decision checkpoint:** go/no-go call recorded, dated, and agreed
      before any Phase 1 work starts

**Milestone:** A documented, evidence-based decision to build tpt-strata,
not an assumption.

## Phase 1 — Project Setup & Governance

- [ ] Initialize git repository
- [ ] `Cargo.toml` workspace (`resolver = "2"`) with `crates/` layout
- [ ] `[workspace.package]`: version, edition, rust-version (MSRV), license,
      authors, repository, keywords, categories
- [ ] `[workspace.dependencies]` for shared deps
- [ ] `LICENSE-MIT` and `LICENSE-APACHE` (Copyright (c) TPT Solutions)
- [ ] `license = "MIT OR Apache-2.0"` set on every crate
- [ ] `README.md` (what/why, links back to spec.txt's non-negotiables)
- [ ] `CONTRIBUTING.md`
- [ ] `SECURITY.md`
- [ ] `.gitignore`
- [ ] `.github/workflows/ci.yml`: fmt check, `clippy --all-targets -D
      warnings`, `test --all`, `build --all`, pinned MSRV job
- [ ] `deny.toml` (cargo-deny) for license/dependency compliance
- [ ] Decide final crate names/split (facade + core + bridge, e.g.
      `tpt-strata`, `tpt-strata-core`, `tpt-strata-bridge`)

**Milestone:** Empty but fully scaffolded workspace, CI green on a hello-world.

## Phase 2 — Native Columnar Format & Execution Core

- [ ] Define native columnar in-memory data format (column/array
      representations, null handling, schema type)
- [ ] Implement core scalar/column types needed for v1 analytical queries
- [ ] Batch/chunk abstraction for streaming execution
- [ ] Physical execution engine: filter operator
- [ ] Physical execution engine: project operator
- [ ] Physical execution engine: aggregate operator (sum, count, min, max,
      avg, group by)
- [ ] Physical execution engine: join operator
- [ ] Physical execution engine: sort operator
- [ ] Physical execution engine: limit operator
- [ ] Confirm the format has zero wire-level dependency on any external
      in-memory columnar standard (non-negotiable #1)

**Milestone:** Can execute a hand-built physical plan over in-memory data
with no SQL or builder layer yet.

## Phase 3 — Builder API

- [ ] Design builder-style API surface (data source in -> query steps ->
      execute -> results out)
- [ ] Builder support for filter/project/aggregate/join/sort/limit without
      requiring callers to implement execution-plan traits
- [ ] Validate a "zero trait implementations for the common case" example
      compiles and runs end-to-end
- [ ] API docs + runnable example for the builder path

**Milestone:** A caller can go from raw data to a query result using only
the builder API, no SQL required.

## Phase 4 — SQL Surface

- [ ] SQL parser (or vetted minimal-dependency parser) for the v1 subset
- [ ] Bind SQL AST to the builder/logical plan layer
- [ ] Support: filter (WHERE), project (SELECT), aggregate (GROUP BY +
      aggregate functions), join, sort (ORDER BY), limit
- [ ] SQL-level test suite covering each supported clause combination
- [ ] Confirm compile times/ergonomics still hold for SQL-surface users
      (per Section 3 criterion)

**Milestone:** A basic SQL string can be run end-to-end against tpt-strata
data and return correct results.

## Phase 5 — Diagnostics Layer

- [ ] Design error type hierarchy tied to caller-supplied schema (column
      names/types), not internal plan-node identifiers
- [ ] Diagnostic: missing column error (names the actual column)
- [ ] Diagnostic: type mismatch error (names the actual types involved)
- [ ] Diagnostic: unsupported cast error
- [ ] Diagnostic: malformed/unsupported SQL construct error
- [ ] Snapshot/regression tests asserting exact diagnostic wording for each
      case above
- [ ] Re-run the Phase 0 diagnostic comparison against the finished
      implementation to confirm the ease-of-use thesis held

**Milestone:** Every user-facing error names the caller's own problem in
the caller's own terms.

## Phase 6 — Import Bridge (Parquet)

- [ ] Isolate bridge as its own crate (only place an external columnar
      interop dependency is allowed, per non-negotiable #2)
- [ ] Read Parquet files into tpt-strata's native format
- [ ] Schema mapping/validation on import, with diagnostics-layer errors on
      mismatch
- [ ] Import bridge test suite (representative Parquet fixtures)

**Milestone:** A Parquet file can be loaded and queried through tpt-strata
with no core-crate dependency on the Parquet/interchange ecosystem.

## Phase 7 — Export Bridge

- [ ] Confirm from Phase 0 whether v1 needs export or it can wait
- [ ] (If in scope) Emit query results as external interchange-format
      record batches from the bridge crate
- [ ] (If in scope) Export bridge test suite, round-trip tests against a
      consumer (e.g. PyArrow-readable output)

**Milestone:** Results can flow out to external BI/interchange tooling
without the core engine depending on that format.

## Phase 8 — Testing, Benchmarking & Documentation

- [ ] Unit test coverage across core engine, builder API, SQL surface,
      diagnostics, bridges
- [ ] Integration tests: end-to-end query scenarios per Section 4 scope
- [ ] Benchmark suite comparing tpt-strata vs. the incumbent on the same
      workloads used in the Phase 0 validation
- [ ] Full API documentation (rustdoc) with runnable examples
- [ ] Architecture doc describing the native format/engine boundary and the
      bridge-isolation rule (non-negotiables #1 and #2)

**Milestone:** A newcomer can read the docs, run the examples, and trust
the benchmark numbers.

## Phase 9 — Release

- [ ] Finalize crate metadata for publishing (description, categories,
      keywords, docs.rs config)
- [ ] `CHANGELOG.md` for v1.0.0
- [ ] Tag and publish crates to crates.io
- [ ] Publish rustdoc / project site
- [ ] Announce availability to prospective in-house consumers

**Milestone:** tpt-strata v1.0.0 is published and usable as a dependency.
