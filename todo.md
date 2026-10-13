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
- [x] (If in scope) Emit query results as external interchange-format
      record batches from the bridge crate — `write_parquet` (Phase 13)
- [x] Export bridge round-trip tests (Parquet write then read, Phase 13).
      Still open: a check against an external consumer such as PyArrow.

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

Status: the items below were first *decided* as v1.1 scope (recorded in
`docs/v1.1-scope.md`). Checkboxes now reflect whether the feature is
**implemented** (`[x]`) or still **deferred** (`[ ]`, rejected with a targeted
parser diagnostic). See Phase 13 for the implementation log.

- [ ] `OR` in `WHERE` — deferred; needs a boolean expression tree
- [x] `IS NULL` / `IS NOT NULL` — implemented (Phase 13)
- [ ] `IN (...)` — deferred; needs `OR`. `BETWEEN` is implemented (Phase 13)
- [ ] Parentheses / operator precedence in `WHERE` — deferred with `OR`
- [x] `LIKE` / `NOT LIKE`, `DISTINCT`, `HAVING` — implemented (Phase 13)
- [ ] Multiple `JOIN`s per query; LEFT/OUTER join — deferred (single inner
      equi-join only)
- [ ] Column-to-column comparisons in `WHERE` — deferred (column-vs-literal
      only)

### Missing data types

Decision: **evaluated and deferred pending consumer confirmation**, recorded
in `docs/v1.1-scope.md`. Adding a type touches the whole native format
(`Scalar`, `DataType`, arrays, engine comparisons, `format.rs`, bridge), so
each must be bought by a real consumer schema.

- [ ] Native `Date`/`Timestamp` types — deferred. Parquet DATE32 and
      TIMESTAMP columns import as their raw integers (Phase 13), which covers
      the queryable case without a new native type.
- [ ] Native `Decimal`, `Binary`, `Float32`, `Int8`/`Int16`, unsigned types —
      deferred. Narrow and unsigned integers and Float32 are widened to
      `i32`/`i64`/`f64` at import (Phase 13), so no new native type is needed.

### Documentation

- [x] Note in ARCHITECTURE.md that Sort/Join fully materialize in memory
      (no streaming/spill), so consumers don't assume otherwise.

### Innovative / forward-looking (optional, gauge consumer demand first)

Decision: **demand-gated**, recorded in `docs/v1.1-scope.md` with the named
consumer gate for each.

- [x] `Table::from_rows`-style row-oriented convenience constructor
      alongside `Table::try_new` — implemented (Phase 13).
- [x] `EXPLAIN`-style plan-printing function — implemented as
      `SqlContext::explain` (Phase 13).
- [ ] Minimal streaming/incremental aggregation mode — still deferred; the
      engine materializes every operator (see ARCHITECTURE.md).

**Milestone:** Known bugs from the post-1.0.1 review are fixed, an
`examples/` surface exists for both crates, and SQL/type-gap decisions are
made deliberately rather than by omission.

## Phase 11 — Recommendations (SQL coverage & comparison rigor)

> Source: review of spec.txt §3/§6, validation-memo.md, docs/benchmarks.md,
> and docs/v1.1-scope.md, 2026-10-13. These reopen some Phase 10 "deferred"
> decisions. The Phase 10 checkboxes stay as recorded; each item below
> supersedes its deferral once done.

### 11a. Verify the comparison claims

- [ ] Ask each consumer (tpt-keystone-db, tpt-aion, tpt-cloud-observability)
      for the SQL they plan to run; record the list in
      `docs/consumer-queries.md`. This is the demand gate every deferred
      feature below depends on.
- [ ] Add Polars and DuckDB (`duckdb-rs`) to the Phase 0 comparison: same
      "point at data" case, same broken queries; record dependency count,
      debug and release compile time, and error text. Update
      `validation-memo.md` with a dated section.
- [ ] Re-check the Phase 8 benchmark item: `docs/benchmarks.md` says no direct
      comparison exists, so the "compare vs. the incumbent" claim is not met.
      Either add a DataFusion (and ideally Polars) run on the same workloads,
      or reword the item to say it is not done.
- [ ] Reconcile CAST: `validation-memo.md` says "add at the SQL layer," but
      `docs/v1.1-scope.md` does not list CAST. Add a row with a decision, or
      correct the memo.
- [ ] Revisit the spec §3 thesis in writing: state the ergonomic win over
      DataFusion using the consumer query list, not the five-row sample.

### 11b. Expand the SQL subset (ordered by cost)

Start with the parser change, since the cheap predicates fit into it.

- [ ] **OR and parentheses in WHERE.** Replace the flat AND list with an
      expression tree and precedence (`AND` binds tighter than `OR`). This is
      the prerequisite for everything below. Tests: precedence, nested parens,
      mixed AND/OR, rejection of unbalanced parens with a named diagnostic.
- [x] **IS NULL / IS NOT NULL.** Engine already has null-aware semantics; add
      a predicate variant. Tests: null vs. non-null rows, with and without
      GROUP BY. (Done; see Phase 13.)
- [ ] **IN (...).** Still deferred: needs `OR`, which is not built. **BETWEEN**
      is done (desugars to two range predicates; see Phase 13).
- [x] **HAVING.** Apply the existing filter operator to aggregate output. HAVING
      on an aggregate that is not in SELECT is computed, not rejected. (Done.)
- [x] **DISTINCT.** Implemented as GROUP BY over the projected columns. (Done.)
- [ ] **Column-to-column comparisons in WHERE.** Not started.
- [ ] **LEFT / OUTER JOIN and multiple JOINs.** Engine change: join operator
      and plan composition. Gate on 11a consumer list; do not start without a
      real query. Update ARCHITECTURE.md's materialization note when done.
- [x] **LIKE.** Implemented with `%` and `_` only, as `LIKE` and `NOT LIKE`
      (added without a consumer gate, on request). Other constructs such as
      `ESCAPE` are not accepted. (Done.)

### 11c. Types

- [ ] Date/Timestamp: start only after a consumer schema names a real column
      (per v1.1-scope.md). Write the blast-radius list (Scalar, DataType,
      arrays, comparisons, format.rs inference, Parquet bridge) before
      estimating.

### 11d. Documentation and guardrails

- [ ] Update `docs/v1.1-scope.md` with a changelog entry for each item moved
      out of "deferred."
- [ ] Update the README "Constructs outside the subset" section as each
      construct lands; keep the diagnostic tests in
      `tests/diagnostics.rs` in sync.
- [ ] Add a SQL support matrix to README.md (supported / deferred / rejected)
      so users can see the subset without reading the grammar.

**Milestone:** The comparison claims are backed by data, and the SQL subset
covers the queries the three consumers actually run. Every remaining gap is a
recorded decision with a named consumer gate.

## Phase 12 — Performance (typed hot paths)

> Source: performance review, 2026-10-13. Baseline (`cargo bench -p
> tpt-strata --bench queries`, median-ish of 7 samples): GROUP BY 100k rows
> ~33 ms (~330 ns/row); filter+project+order+limit ~60 ms (~600 ns/row); SQL
> GROUP BY ~43 ms; equi-join 20k rows ~14 ms (~700 ns/row). Raw run saved to
> the session scratchpad as `bench_baseline.txt`.
>
> Constraint: the core crate stays zero-dependency. `tpt-simd` (GitHub,
> pre-release v0.1.0, MIT/Apache-2.0) has zero external dependencies in
> `tpt-simd-vector`, but it is not published and its crates depend on each
> other by path. It is not added to core. Evaluate it again (see 12e) once
> it is published and a kernel is measured to need explicit lanes.

### 12a. Aggregate (GROUP BY)

- [ ] Replace per-row `Vec<Scalar>` group keys with borrowed typed key parts
      (`KeyPart<'a>`): no allocation per row, `&str` keys borrowed from the
      input batch.
- [ ] Replace `Box<dyn Accumulator>` per group with typed state vectors
      indexed by group id (`AggState`), updated by a loop over typed arrays.
- [ ] Keep semantics: SUM/AVG skip NULL and return NULL for no input; COUNT(*)
      counts all rows; MIN/MAX skip NULL; global aggregate returns one row
      over zero rows.
- [ ] Group output order is first-seen (was HashMap order). Check tests.

### 12b. Filter

- [ ] Evaluate predicates on typed slices, producing a selection vector.
      Typed fast path for same-type comparisons; generic fallback for mixed
      types so behaviour is unchanged.

### 12c. Sort and join

- [ ] Sort: sort an index vector with a typed comparator over the merged key
      columns, then `take` once per column (no per-value `Scalar` clones).
      Keep NULLS FIRST ascending.
- [ ] Join: build the hash index on typed `KeyPart`s instead of `Scalar`
      (removes the per-probe-row `String` clone). Keep NULL keys excluded.

### 12d. Data movement

- [ ] Measure how much time `DataSource::execute` and SQL bind spend cloning
      batches (`batches().to_vec()`, `Vec<Batch>::clone`). If significant,
      share column storage behind `Arc`.

### 12e. Evaluate tpt-simd (optional, after publication)

- [ ] Once tpt-simd is published on crates.io, decide whether to add it as an
      optional `simd` feature in the core crate (default build stays
      zero-dependency). Only if the typed kernels from 12a/12b leave a
      measured hot loop that benefits from explicit lanes.

### 12f. Measurement

- [ ] Re-run `cargo bench -p tpt-strata` after each item; record before/after
      ns/row in docs/benchmarks.md with the toolchain and date.
- [ ] Profile the GROUP BY benchmark with a sampling profiler to confirm the
      remaining cost after 12a.

**Milestone:** The GROUP BY, filter, sort, and join paths work on typed data
with no per-row `Scalar` or `String` allocation, the zero-dependency promise
still holds, and every number in docs/benchmarks.md is backed by a re-run.

## Phase 13 — Review implementation log (unreleased)

> Work from the 2026-10-13 platform review. Nothing has been tagged, published,
> or uploaded. `[ ]` means not done.

### Phase A — correctness

- [x] String and boolean range predicates compare; they were always false.
- [x] `AS` aliases name output columns and work in `ORDER BY` and `HAVING`.
      Two tests that asserted the old names were corrected.
- [x] Joins: qualified field names, an ambiguity error, the ON clause accepted
      in either order, a join-key type check, a duplicate-name error for the
      builder `join()`, and self-joins rejected with a message.
- [x] Batches are shared (`Arc`), so a query no longer copies its input.
      `Table::batches()` returns `&[Arc<Batch>]`.
- [x] Float keys: `NaN` groups together and `-0.0` groups with `0.0`.
- [x] Empty inputs (`LIMIT 0`, an empty join side) produce typed empty batches.
- [x] Comparisons with `NULL` match nothing. `IS [NOT] NULL` added.
- [x] Integer and float comparisons are exact. `WHERE id > 2.5` works on an
      integer column.
- [x] `SUM` over integers returns `Int64` and fails on overflow.
- [x] Builder: `count_all()`, `project()` after aggregates, and integer literals
      against float columns. `order_by` is still checked at execute time, which
      is the only point where aggregate names are known.
- [x] Qualified `ORDER BY`, `SELECT *, col`, dotted quoted identifiers, and an
      error for an unknown table qualifier.
- [x] Lexer: `--` and `/* */` comments, exponent literals, doubled quotes in
      quoted identifiers. `COUNT(DISTINCT ...)` gets a targeted diagnostic.
- [x] `Table::concat` accepts inputs that differ only in nullability.
- [x] Parquet errors are `Io` and `Import`, with `source()` for I/O.
- [x] `print_table` measures width in characters, shows the header for zero
      rows, and escapes newlines.
- [ ] `LIMIT` still reads its whole input; there is no upstream short-circuit.
- [ ] The literal `-9223372036854775808` still fails, because the minus sign is
      lexed separately.
- [ ] `Scalar` display still prints `100.0` as `100`.
- [ ] `ArrayRef::take` still panics on an out-of-range index.

### Phase B — release readiness (nothing published)

- [x] `cargo publish --dry-run -p tpt-strata` passes. It packages and verifies
      the crate, and stops before the upload.
- [ ] `tpt-strata-parquet` dry run: needs `tpt-strata` on crates.io first.
- [x] CI: tests on Linux, Windows and macOS; a rustdoc job with warnings as errors.
- [x] `release-check.yml`: runs on `v*` tags or by hand, and never uploads.
- [x] Dependabot, issue and pull request templates, CODEOWNERS, a Code of Conduct,
      and an expanded CONTRIBUTING.
- [ ] CODEOWNERS names `@PhillipC05` as a placeholder. Replace it with the
      real maintainers.
- [ ] MSRV 1.75 is not verified here: no 1.75 toolchain is installed locally,
      and arrow/parquet 55 may need a newer compiler. The CI `msrv` job is the check.
- [ ] Exact version pins (`=1.0.1`) are unchanged. Revisit when the publish
      order is settled.
- [ ] No tags, crates.io entries, or docs.rs builds exist yet.

### Phase C — adoption and features (partly done)

- [x] `Table::from_rows`, `Table::rename_columns`.
- [x] CSV reading and writing with no dependencies (`tpt_strata::csv`).
- [x] Parquet: widened types, `read_parquet_columns`, `read_parquet_bytes`,
      `write_parquet`, with round-trip tests.
- [x] `strata` command-line tool: SQL over CSV and Parquet, joins across
      files, `--explain`, `--check`, `--csv`, and a prompt. Not published.
- [x] `SqlContext::explain` prints the physical plan.
- [x] v1.1 SQL: `IS [NOT] NULL`, `BETWEEN`, `[NOT] LIKE`, `DISTINCT`, `HAVING`,
      `OFFSET`, and several `ORDER BY` keys.
- [ ] Still deferred: `OR`, `IN`, parentheses in `WHERE`, `LEFT`/multiple joins,
      column-to-column comparisons, `CAST`, and arithmetic in `SELECT`.
- [ ] Phase 12 performance items (typed group keys, typed filter kernels,
      top-k sort, typed join keys) are not done. The measured gain came from
      shared batches and typed gathers, which is part of 12c and 12d.
- [ ] Parquet row-group streaming is not done. The whole file is read into memory.
- [ ] JSON ingest is not done.

### Phase D — innovation (a small set)

- [x] `SqlContext::check` binds a query without running it, for use in CI.
- [x] `QueryError::kind()` gives stable, machine-readable error names.
- [x] WASM build and browser playground (`tpt-strata-wasm`, `playground/`),
      with a Node smoke test. The page has not been opened in a browser.
- [x] Python bindings (`tpt-strata-python`, built with maturin): 16 tests pass.
- [x] Serde support (`tpt-strata-serde`): flat structs to tables and back.
- [ ] Not started: a derive macro and per-error explanation pages.

### Phase E — examples and templates

- [x] Examples: `csv_analytics` and `joins_and_aliases`, with sample CSVs in
      `crates/tpt-strata/examples/data`.
- [x] Docs: `docs/quickstart.md`, `docs/cookbook.md`, `docs/faq.md`.
- [x] `templates/embedded-analytics`: a starter project, excluded from the
      workspace. It builds and runs.
- [x] Comparison benchmark against DataFusion and DuckDB (`bench/compare`,
      results in `docs/benchmark-comparison.md`). tpt-strata is slower on these
      analytic queries; the write-up says by how much.
- [ ] Not done: `docs/consumer-queries.md`, which needs the consumers' own queries.

### Verification

- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --workspace`, and `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings`
  all pass.
- `cargo bench -p tpt-strata --bench queries` was re-run. The figures are in
  docs/benchmarks.md, with the caveat that they are not a same-session comparison.
