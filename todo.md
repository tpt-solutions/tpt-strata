# tpt-strata — Project Roadmap

License: MIT OR Apache-2.0 · Copyright TPT Solutions

> Status: Conditional. Do not begin Phase 1 until every Phase 0 checkbox
> is checked and the go/no-go call is a **go**.

## Phase 0 — Validation & Go/No-Go

- [x] Run "point at data, get an answer" against the incumbent's *current*
      release: record exact line count and any trait implementations required
- [x] Repeat the same basic case in a tpt-strata prototype/sketch; compare
      line count and trait/boilerplate burden directly
- [x] Diagnostic comparison: pick one broken query (bad type, missing
      column, unsupported cast), run it on the incumbent, capture the exact
      error text a caller sees
- [x] Produce the equivalent tpt-strata diagnostic for the same broken query
      and compare wording/clarity side by side
- [x] Decide minimal viable bridge surface for v1: confirm export is
      actually needed at launch, or defer it
- [x] Write up the Section 3 validation result (ease-of-use, compile times/
      error messages, integration boilerplate) as a short go/no-go memo
- [x] **Decision checkpoint:** go/no-go call recorded, dated, and agreed
      before any Phase 1 work starts

**Milestone:** A documented, evidence-based decision to build tpt-strata,
not an assumption.

## Phase 1 — Project Setup & Governance

- [x] Initialize git repository
- [x] `Cargo.toml` workspace (`resolver = "2"`) with `crates/` layout
- [x] `[workspace.package]`: version, edition, rust-version (MSRV), license,
      authors, repository, keywords, categories
- [x] `[workspace.dependencies]` for shared deps
- [x] `LICENSE-MIT` and `LICENSE-APACHE` (Copyright (c) TPT Solutions)
- [x] `license = "MIT OR Apache-2.0"` set on every crate
- [x] `README.md` (what/why, links back to spec.txt's non-negotiables)
- [x] `CONTRIBUTING.md`
- [x] `SECURITY.md`
- [x] `.gitignore` (exists)
- [x] `.github/workflows/ci.yml`: fmt check, `clippy --all-targets -D
      warnings`, `test --all`, `build --all`, pinned MSRV job
- [x] `deny.toml` (cargo-deny) for license/dependency compliance
- [x] Decide final crate names/split: `tpt-strata` (core, zero deps) +
      `tpt-strata-parquet` (bridge, only crate allowed external deps)

**Milestone:** Empty but fully scaffolded workspace, CI green on a hello-world.

## Phase 2 — Native Columnar Format & Execution Core

- [x] Define native columnar in-memory data format (column/array
      representations, null handling, schema type)
- [x] Implement core scalar/column types needed for v1 analytical queries
- [x] Batch/chunk abstraction for streaming execution
- [x] Physical execution engine: filter operator
- [x] Physical execution engine: project operator
- [x] Physical execution engine: aggregate operator (sum, count, min, max,
      avg, group by)
- [x] Physical execution engine: join operator
- [x] Physical execution engine: sort operator
- [x] Physical execution engine: limit operator
- [x] Confirm the format has zero wire-level dependency on any external
      in-memory columnar standard (non-negotiable #1)

**Milestone:** Can execute a hand-built physical plan over in-memory data
with no SQL or builder layer yet.

## Phase 3 — Builder API

- [x] Design builder-style API surface (data source in -> query steps ->
      execute -> results out)
- [x] Builder support for filter/project/aggregate/join/sort/limit without
      requiring callers to implement execution-plan traits
- [x] Validate a "zero trait implementations for the common case" example
      compiles and runs end-to-end
- [x] API docs + runnable example for the builder path

**Milestone:** A caller can go from raw data to a query result using only
the builder API, no SQL required.

## Phase 4 — SQL Surface

- [x] SQL parser (or vetted minimal-dependency parser) for the v1 subset
      (hand-written lexer + recursive-descent parser in the core crate,
      preserving the zero-external-dependency promise)
- [x] Bind SQL AST to the builder/logical plan layer
- [x] Support: filter (WHERE), project (SELECT), aggregate (GROUP BY +
      aggregate functions), join, sort (ORDER BY), limit
- [x] SQL-level test suite covering each supported clause combination
- [x] Confirm compile times/ergonomics still hold for SQL-surface users
      (per Section 3 criterion)

**Milestone:** A basic SQL string can be run end-to-end against tpt-strata
data and return correct results.

## Phase 5 — Diagnostics Layer

- [x] Design error type hierarchy tied to caller-supplied schema (column
      names/types), not internal plan-node identifiers
- [x] Diagnostic: missing column error (names the actual column)
- [x] Diagnostic: type mismatch error (names the actual types involved)
- [x] Diagnostic: unsupported cast error
- [x] Diagnostic: malformed/unsupported SQL construct error
- [x] Snapshot/regression tests asserting exact diagnostic wording for each
      case above
- [x] Re-run the Phase 0 diagnostic comparison against the finished
      implementation to confirm the ease-of-use thesis held

**Milestone:** Every user-facing error names the caller's own problem in
the caller's own terms.

## Phase 6 — Import Bridge (Parquet)

- [x] Isolate bridge as its own crate (only place an external columnar
      interop dependency is allowed, per non-negotiable #2)
- [x] Read Parquet files into tpt-strata's native format
- [x] Schema mapping/validation on import, with diagnostics-layer errors on
      mismatch
- [x] Import bridge test suite (representative Parquet fixtures)

**Milestone:** A Parquet file can be loaded and queried through tpt-strata
with no core-crate dependency on the Parquet/interchange ecosystem.

## Phase 7 — Export Bridge

- [x] Confirm from Phase 0 whether v1 needs export or it can wait
      (Phase 0 memo, Section 4: **defer** — no consumer needs it at launch)
- [ ] (If in scope) Emit query results as external interchange-format
      record batches from the bridge crate
- [ ] (If in scope) Export bridge test suite, round-trip tests against a
      consumer (e.g. PyArrow-readable output)

**Milestone:** Results can flow out to external BI/interchange tooling
without the core engine depending on that format.

## Phase 8 — Testing, Benchmarking & Documentation

- [x] Unit test coverage across core engine, builder API, SQL surface,
      diagnostics, bridges (engine.rs 11, sql.rs 23, diagnostics.rs 10,
      import.rs 6, lib doctests)
- [x] Integration tests: end-to-end query scenarios per Section 4 scope
      (tests/integration.rs — multi-batch streaming, builder-vs-SQL
      equivalence, SQL/builder join equivalence, global aggregates,
      caller-facing diagnostics across both surfaces)
- [x] Benchmark suite comparing tpt-strata vs. the incumbent on the same
      workloads used in the Phase 0 validation (benches/queries.rs, zero
      external deps; results recorded in docs/benchmarks.md alongside the
      Phase 0 incumbent records)
- [x] Full API documentation (rustdoc) with runnable examples (lib.rs
      builder doctest, sql.rs SqlContext doctest, README examples)
- [x] Architecture doc describing the native format/engine boundary and the
      bridge-isolation rule (non-negotiables #1 and #2) — ARCHITECTURE.md

**Milestone:** A newcomer can read the docs, run the examples, and trust
the benchmark numbers.

## Phase 9 — Release

- [x] Finalize crate metadata for publishing (description, categories,
      keywords, docs.rs config, readme, homepage, documentation)
- [x] `CHANGELOG.md` for v1.0.0
- [ ] Tag and publish crates to crates.io
- [ ] Publish rustdoc / project site
- [ ] Announce availability to prospective in-house consumers

**Milestone:** tpt-strata v1.0.0 is published and usable as a dependency.

## Phase 10 — Post-1.0.1 Review Findings

> Source: platform review (bugs/todos/missing features/adoption audit),
> 2026-09-12. See conversation/plan for full detail on each item.

### Bugs

- [x] Fix `ORDER BY` silently mishandling `NULL`: `scalar_cmp`
      (`types.rs:79-89`) returns `Equal` for any incomparable pair,
      including `NULL` vs. a value. MIN/MAX already guard against this
      (`engine.rs:286`, `:308`) but the `SortExec` comparator
      (`engine.rs:721`) does not, so nulls land in arbitrary sort positions
      with no diagnostic. **Decision: NULLS FIRST ascending** (`sort_cmp`,
      `types.rs`), applied in `SortExec`; regression tests in `tests/fixes.rs`.
- [x] Add MIN/MAX type-validation diagnostic mirroring the numeric-only
      guard SUM/AVG already has (`engine.rs:460-472`), so unorderable
      inputs get a named diagnostic instead of falling through to
      `scalar_cmp`'s silent-equal fallback. (Every native v1 type is
      orderable, so the guard is a no-op today but locks in a named
      diagnostic for future types.)
- [x] Harden/annotate fragile `.expect()` calls that rely on invariants
      enforced elsewhere: `sql.rs:1134` (wildcard expansion), `format.rs:289`
      (equal column lengths). Add a comment naming the invariant, or a
      `debug_assert!`. **Added both.**
- [x] Clarify `error.rs:60-65` (`unsupported_format`) as an internal
      "should never happen" path (rename or `debug_assert!`) rather than a
      caller-facing diagnostic with a raw `{:?}` dump. **Renamed to
      `internal_downcast` and re-framed as internal.**

### Adoption / examples (highest leverage, no design risk)

- [x] Add `crates/tpt-strata/examples/basic_query.rs` — builder API
      end-to-end.
- [x] Add `crates/tpt-strata/examples/sql_query.rs` — `SqlContext`
      end-to-end (incl. JOIN).
- [x] Add `crates/tpt-strata/examples/diagnostics.rs` — deliberately trigger
      each `QueryError` variant and print it (diagnostics is a headline
      feature with zero example coverage today).
- [x] Add `crates/tpt-strata-parquet/examples/read_parquet.rs` — the
      Parquet bridge currently has no example and no doctest at all.
- [x] Add crates.io/docs.rs/CI/license badges to README.md.
- [x] Surface a condensed (3-row) version of the DataFusion comparison
      (deps, compile time, error clarity) directly in README.md, not just
      `validation-memo.md`/`docs/benchmarks.md`.
- [x] Replace SECURITY.md's "email the maintainers" with a real contact or
      advisory link. **GitHub private vulnerability reporting**
      (`https://github.com/tpt-solutions/tpt-strata/security/advisories/new`).
- [x] Write a short integration "recipe" doc/template for wiring
      tpt-strata into a struct-backed store, aimed at the three known
      consumers (tpt-keystone-db, tpt-aion, tpt-cloud-observability).
      **`docs/integration-recipe.md`.**

### Missing SQL features (scope as a deliberate v1.1 decision, not bugs)

Decision: **all deferred as deliberate v1.1 scope**, recorded in
`docs/v1.1-scope.md`, and each now gets a targeted parser diagnostic (tests in
`tests/diagnostics.rs`) instead of a generic parse error — so they are
rejected *by decision*, not by omission.

- [x] `OR` in `WHERE` (only `AND`-chains supported today, `sql.rs:9`,
      `:686-693`)
- [x] `IS NULL` / `IS NOT NULL`
- [x] `IN (...)` and `BETWEEN`
- [x] Parentheses / operator precedence in `WHERE`
- [x] `LIKE`, `DISTINCT`, `HAVING`
- [x] Multiple `JOIN`s per query; LEFT/OUTER join (today: single inner
      equi-join only, `engine.rs:557-670`)
- [x] Column-to-column comparisons in `WHERE` (today: column-vs-literal
      only)

### Missing data types

Decision: **evaluated and deferred pending consumer confirmation**, recorded
in `docs/v1.1-scope.md`. Adding a type touches the whole native format
(`Scalar`, `DataType`, arrays, engine comparisons, `format.rs`, bridge), so
each must be bought by a real consumer schema.

- [x] Evaluate adding `Date`/`Timestamp` (most Parquet-relevant gap) —
      check tpt-cloud-observability's actual schema needs first.
- [x] Evaluate `Decimal`; lower priority: `Int8`/`Int16`, unsigned ints,
      `Float32`, `Binary`.

### Documentation

- [x] Note in ARCHITECTURE.md that Sort/Join fully materialize in memory
      (no streaming/spill), so consumers don't assume otherwise.

### Innovative / forward-looking (optional, gauge consumer demand first)

Decision: **demand-gated**, recorded in `docs/v1.1-scope.md` with the named
consumer gate for each.

- [x] `Table::from_rows`-style row-oriented convenience constructor
      alongside `Table::try_new`.
- [x] `EXPLAIN`-style plan-printing function using the existing diagnostics
      infrastructure.
- [x] Roadmap line for a minimal streaming/incremental aggregation mode,
      given tpt-cloud-observability is a named consumer.

**Milestone:** Known bugs from the post-1.0.1 review are fixed, an
`examples/` surface exists for both crates, and SQL/type-gap decisions are
made deliberately rather than by omission.
