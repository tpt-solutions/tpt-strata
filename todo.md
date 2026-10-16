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

- [x] `OR` in `WHERE` — done (Phase 14), with parentheses
- [x] `IS NULL` / `IS NOT NULL` — implemented (Phase 13)
- [x] `IN (...)` — done (Phase 14). `BETWEEN` is implemented (Phase 13)
- [x] Parentheses / operator precedence in `WHERE` — done (Phase 14)
- [x] `LIKE` / `NOT LIKE`, `DISTINCT`, `HAVING` — implemented (Phase 13)
- [x] Multiple `JOIN`s per query; LEFT/OUTER join — done (Phase 14). `RIGHT`
      and `FULL` joins are done too (Phase 14, after the review).
- [x] Column-to-column comparisons in `WHERE` — done (Phase 14)

### Missing data types

Decision: **evaluated and deferred pending consumer confirmation**, recorded
in `docs/v1.1-scope.md`. Adding a type touches the whole native format
(`Scalar`, `DataType`, arrays, engine comparisons, `format.rs`, bridge), so
each must be bought by a real consumer schema.

- [x] Native `Date`/`Timestamp` types — implemented 2026-10-13 at the owner's request, ahead of a consumer schema. Parquet DATE32 and
      TIMESTAMP columns import as native types. The old raw-integer import is gone.
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
- [x] Add Polars and DuckDB (`duckdb-rs`) to the Phase 0 comparison (validation-memo.md, sections 7 and 8; sketches in `bench/phase0-polars` and `bench/phase0-duckdb`): same
      "point at data" case, same broken queries; record dependency count,
      debug and release compile time, and error text. Update
      `validation-memo.md` with a dated section.
- [x] Re-check the Phase 8 benchmark item (done with DataFusion and DuckDB in
      `docs/benchmark-comparison.md`; Polars is still open above): `docs/benchmarks.md` says no direct
      comparison exists, so the "compare vs. the incumbent" claim is not met.
      Either add a DataFusion (and ideally Polars) run on the same workloads,
      or reword the item to say it is not done.
- [x] Reconcile CAST (decision: stays deferred, recorded as a row in
      `docs/v1.1-scope.md`; the memo's "add at the SQL layer" note is superseded): `validation-memo.md` says "add at the SQL layer," but
      `docs/v1.1-scope.md` does not list CAST. Add a row with a decision, or
      correct the memo.
- [ ] Revisit the spec §3 thesis in writing: state the ergonomic win over
      DataFusion using the consumer query list, not the five-row sample.

### 11b. Expand the SQL subset (ordered by cost)

Start with the parser change, since the cheap predicates fit into it.

- [x] **OR and parentheses in WHERE.** Done with a boolean condition tree
      (`AND` binds tighter than `OR`), used in `WHERE` and `HAVING`. Tests in
      `tests/conditions.rs`: precedence, nested parentheses, NULL inside OR,
      type mismatch in a nested condition.
- [x] **IS NULL / IS NOT NULL.** Engine already has null-aware semantics; add
      a predicate variant. Tests: null vs. non-null rows, with and without
      GROUP BY. (Done; see Phase 13.)
- [x] **IN (...).** Done: an OR of equality comparisons, with NULL entries that
      match nothing. **BETWEEN** is done too (two range predicates).
- [x] **HAVING.** Apply the existing filter operator to aggregate output. HAVING
      on an aggregate that is not in SELECT is computed, not rejected. (Done.)
- [x] **DISTINCT.** Implemented as GROUP BY over the projected columns. (Done.)
- [x] **Column-to-column comparisons in WHERE.** Done. Types must match, or
      both be numeric.
- [x] **LEFT / OUTER JOIN and multiple JOINs.** Done as equi-joins: a left join
      keeps unmatched left rows with NULLs, and a chain of joins may refer to any
      table joined before. `RIGHT` and `FULL` joins were added later (Phase 14). The
      consumer-gate note above no longer applies: it was implemented on request.
- [x] **LIKE.** Implemented with `%` and `_` only, as `LIKE` and `NOT LIKE`
      (added without a consumer gate, on request). Other constructs such as
      `ESCAPE` are not accepted. (Done.)

### 11c. Types

- [x] Date/Timestamp: done (see Phase 15). The blast-radius list was the work plan:
      (per v1.1-scope.md). Write the blast-radius list (Scalar, DataType,
      arrays, comparisons, format.rs inference, Parquet bridge) before
      estimating.

### 11d. Documentation and guardrails


- [x] Update `docs/v1.1-scope.md` with a changelog entry for each item moved
      out of "deferred."
- [x] Update the README "Constructs outside the subset" section as each
      construct lands; keep the diagnostic tests in
      `tests/diagnostics.rs` in sync.
- [x] Add a SQL support matrix to README.md (supported / deferred / rejected)
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
>
> Status (Phase 14): 12a–12d and the 12f re-run are done. The profiler run in 12f
> is open because no sampling profiler is installed here. 12e waits for
> publication.

### 12a. Aggregate (GROUP BY)

- [x] Replace per-row `Vec<Scalar>` group keys with borrowed typed key parts
      (`KeyPart<'a>`): no allocation per row, `&str` keys borrowed from the
      input batch.
- [x] Replace `Box<dyn Accumulator>` per group with typed state vectors
      indexed by group id (`AggState`), updated by a loop over typed arrays.
- [x] Keep semantics: SUM/AVG skip NULL and return NULL for no input; COUNT(*)
      counts all rows; MIN/MAX skip NULL; global aggregate returns one row
      over zero rows.
- [x] Group output order is first-seen (was HashMap order). Check tests.

### 12b. Filter

- [x] Evaluate predicates on typed slices, producing a selection vector.
      Typed fast path for same-type comparisons; generic fallback for mixed
      types so behaviour is unchanged.

### 12c. Sort and join

- [x] Sort: sort an index vector with a typed comparator over the merged key
      columns, then `take` once per column (no per-value `Scalar` clones).
      Keep NULLS FIRST ascending.
- [x] Join: build the hash index on typed `KeyPart`s instead of `Scalar`
      (removes the per-probe-row `String` clone). Keep NULL keys excluded.

### 12d. Data movement

- [x] Measure how much time `DataSource::execute` and SQL bind spend cloning
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
- [x] `LIMIT` short-circuit: operators now stream through `PhysicalPlan::open()`, so `LIMIT` stops pulling input once it has enough rows (`tests/streaming.rs`).
- [x] The literal `-9223372036854775808` is accepted (Phase 14).
- [x] `Scalar` display prints whole floats with a decimal point, `100.0` (Phase 14).
- [x] `ArrayRef::take` returns NULL for an out-of-range index (Phase 14).

### Phase B — release readiness (nothing published)

- [x] `cargo publish --dry-run -p tpt-strata` passes. It packages and verifies
      the crate, and stops before the upload.
- [ ] `tpt-strata-parquet` dry run: needs `tpt-strata` on crates.io first.
- [x] CI: tests on Linux, Windows and macOS; a rustdoc job with warnings as errors.
- [x] `release-check.yml`: runs on `v*` tags or by hand, and never uploads.
- [x] Dependabot, issue and pull request templates, CODEOWNERS, a Code of Conduct,
      and an expanded CONTRIBUTING.
- [x] CODEOWNERS team name: DROPPED (decision, 2026-10-13, per owner). Not pursued.
- [x] MSRV checked per crate (Phase 14): core and serde build and test on 1.75.
      Parquet, CLI and WASM build on 1.85, and the whole workspace needs 1.85.
      CI runs both checks.
- [x] Exact version pins (`=1.0.1`): DECIDED, keep them. Revisit only when the publish
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
- [ ] Still deferred: `CAST` and arithmetic in `SELECT`. `OR`, `IN`, parentheses,
      `LEFT`/multiple joins and column comparisons are done (Phase 14).
- [x] Phase 12 typed hot paths (12a–12d) are done (Phase 14). Only the
      profiler re-check in 12f is open.
- [ ] Parquet row-group streaming: DECIDED, deferred (2026-10-13). The whole file is read into memory. A streaming reader would be new public surface on `tpt-strata-parquet`, and no consumer has shown a table that does not fit. Gate: a consumer query over a file larger than memory.
- [x] JSON ingest is done (`tpt_strata::json::parse_json`, Phase 14).

### Phase D — innovation (a small set)

- [x] `SqlContext::check` binds a query without running it, for use in CI.
- [x] `QueryError::kind()` gives stable, machine-readable error names.
- [x] WASM build and browser playground (`tpt-strata-wasm`, `playground/`),
      with a Node smoke test. The page has not been opened in a browser.
- [x] Python bindings (`tpt-strata-python`, built with maturin): 16 tests pass.
- [x] Serde support (`tpt-strata-serde`): flat structs to tables and back.
- [x] Derive macro: DROPPED (decision, 2026-10-13). `tpt-strata-serde` covers flat structs, and a derive needs a new proc-macro crate for little gain. Revisit only if a consumer asks. Per-error explanation pages are done (`docs/errors.md`).

### Phase E — examples and templates

- [x] Examples: `csv_analytics` and `joins_and_aliases`, with sample CSVs in
      `crates/tpt-strata/examples/data`.
- [x] Docs: `docs/quickstart.md`, `docs/cookbook.md`, `docs/faq.md`.
- [x] `templates/embedded-analytics`: a starter project, excluded from the
      workspace. It builds and runs.
- [x] Comparison benchmark against DataFusion and DuckDB (`bench/compare`,
      results in `docs/benchmark-comparison.md`). tpt-strata is slower on these
      analytic queries; the write-up says by how much.
- [ ] `docs/consumer-queries.md`: the template is written. The repo holds no consumer queries (spec.txt gives use cases in prose only), so this stays open until the three teams send their SQL.

### Verification

- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --workspace`, and `cargo doc --no-deps` with `RUSTDOCFLAGS=-D warnings`
  all pass.
- `cargo bench -p tpt-strata --bench queries` was re-run. The figures are in
  docs/benchmarks.md, with the caveat that they are not a same-session comparison.

## Phase 14 — Remaining-items pass (unreleased)

> Work on the items still open after Phase 13. Nothing was tagged, published,
> or uploaded. Items marked `[ ]` are still open, with the reason.

### Phase 12 — typed hot paths

- [x] 12a: group keys are borrowed typed keys (`KeyPart`), and each aggregate
      keeps typed state indexed by group. Output is in first-seen order.
- [x] 12b: filters on same-type columns compare typed keys. Other cases use the
      general path. Floats keep the SQL NaN rules.
- [x] 12c: sort and join use typed keys. NULL keys still never match in a join.
- [x] 12d: columns are shared (`Arc`), so projecting copies no data. Joins read
      only the columns the query names.
- [x] 12f: the benchmark was re-run and recorded in `docs/benchmark-comparison.md`.
- [ ] 12f: profile the group-by with a sampling profiler. No profiler is
      installed here.
- [ ] 12e: tpt-simd. Waits for publication.

Single-thread results, 1,000,000 rows (first measurement → now): group by
107 ms → 35 ms; filter, sort, limit 68 ms → 54 ms; join 269 ms → 104 ms.

### SQL (Phase 10 and 11b)

- [x] `OR` and parentheses in `WHERE` and `HAVING` (`tests/conditions.rs`).
- [x] `IN (...)`.
- [x] Column-to-column comparisons. Types must match, or both be numeric.
- [x] `LEFT [OUTER] JOIN` and several joins in a chain (`tests/joins.rs`).
- [x] `RIGHT` and `FULL` joins (`tests/joins.rs`). Unmatched rows of the other side
      are kept with NULLs.
- [x] `NOT`, `NOT IN`, `NOT BETWEEN` (`tests/conditions.rs`). `NOT` is pushed down
      to the comparisons, so the NULL rows match SQL three-valued logic.
- [x] SQL support matrix in the README and the constructs section updated.

### Phase 13 leftovers

- [x] `-9223372036854775808` literal.
- [x] Whole floats print as `100.0`.
- [x] `ArrayRef::take` returns NULL for an out-of-range index.
- [x] JSON ingest (`tpt_strata::json::parse_json`). The CLI reads `.json`.
- [x] `LIMIT` short-circuit (done 2026-10-13, see `engine::BatchStream`). Scan, filter, project and limit stream;
      sort, join and aggregate still materialize, as they must.
- [ ] Parquet row-group streaming: DECIDED, deferred (2026-10-13), same gate as above. The whole table is held in memory.

### Phase 14 — other open items

- [x] Phase D: derive macro DROPPED (decision, 2026-10-13); see Phase 13 D.
- [x] Phase D: per-error explanation pages — `docs/errors.md`, one section per `kind()`.
- [ ] Phase 11a: consumer queries. These need the consumers. The Polars
      comparison is done (validation-memo.md, section 7).
- [ ] Phase 12f profiler: blocked on elevation. `samply` is installed, but on
      Windows it needs `xperf` from the Windows Performance Toolkit, which needs
      an elevated install. From an elevated shell, run
      `winget install -e --id Microsoft.WindowsADK --override "/quiet /features OptionId.WindowsPerformanceToolkit"`,
      then `samply record --save-only -o profile.json.gz -- target/release/deps/queries-*.exe`.
- [x] Phase 11c: native Date and Timestamp types, done (Phase 15).
- [ ] Phase 9 and 12e: publishing, tags, and the tpt-simd evaluation. Out of
      scope until a release is decided.

### Verification

- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets
  --all-features -- -D warnings` pass.
- `cargo test --workspace`: 22 suites, all passing.
- MSRV: `tpt-strata` and `tpt-strata-serde` build and test on 1.75.
  `tpt-strata-parquet`, `tpt-strata-cli` and `tpt-strata-wasm` build on 1.85,
  and so does the whole workspace.

## Phase 15 — Native dates and performance pass (unreleased)

> Work on 2026-10-13, at the owner's request. Date and Timestamp were deferred
> until a consumer schema existed; the owner asked to go ahead. Profiling used
> per-stage timing, because `samply` still needs an elevated `xperf` install.

### Native dates

- [x] `DATE` (`DataType::Date`, `Scalar::Date`, days since 1970-01-01) and
      `TIMESTAMP` (`DataType::Timestamp`, microseconds, no time zone).
- [x] Compare, sort, group, join, and MIN/MAX. SUM and AVG reject them.
- [x] SQL `DATE '…'` and `TIMESTAMP '…'` literals. A plain string against a
      date column is parsed; an invalid one gets a diagnostic naming the format.
- [x] Parquet DATE32 and TIMESTAMP (any unit or zone, read as UTC micros) import
      and write as native types.
- [x] Serde and WASM render them as ISO text.
- [ ] CSV and JSON do not infer dates. Text stays text. Deliberate for now;
      revisit with a consumer.

### Performance (measured; see docs/benchmark-comparison.md, Phase 15 run)

- [x] Strings as `Arc<str>`: a stage breakdown showed filters spending most of
      their time copying `String`s. Two filters: 19.7 ms → 3–4 ms per 100k rows.
- [x] Fused predicates: one filter pass, columns gathered once.
- [x] Float fast path for `Float64` columns against `Float64` literals.
- [x] Top-k: `ORDER BY … LIMIT` keeps `offset + n` rows at the sort. Ties keep
      input order. Tested against a full sort.
- [x] Single-key GROUP BY indexes on the key, with no per-row `Vec`.
- [x] Q2 (filter, sort, limit): 54 ms → 18 ms on 1M rows.
- [x] Q1 (GROUP BY): 34.7 → 29.2 ms. Aggregates fold each batch with typed loops.
- [ ] Q3 (join): 104 → 59 ms, still about 6× DataFusion on one thread. Probe and
      output gather are the costs (the position vectors are 24 bytes per row).
- [ ] Parity is not reached. Remaining gaps: join, per-row aggregate dispatch,
      `Vec<Option<T>>` array layout, and single-threaded execution.
- [ ] Profiler re-run (12f) is still blocked on elevation.
- [x] Ceiling and distance: `bench/compare` now runs a hand-written loop floor
      and a bandwidth bound beside the engines (results in
      docs/benchmark-comparison.md). tpt-strata is 28× (Q1), 3.5× (Q2) and 75×
      (Q3) the hand loop. Next: a branchless or vectorised Q2 filter, and a
      join probe that does not copy row positions.
- [x] Join probe: direct layout for small `Int64` key ranges (no hashing),
      tested in `tests/join_index.rs`. Probe 14.5 ms → ~10 ms on 1M rows.
      Gather (~16 ms, `Vec<Option<T>>` per column) is the next join cost.
- [x] Single-key GROUP BY on strings: a 256-slot cache keyed by each string's
      `Arc` address sits in front of the content hash map (`aggregate.rs`,
      `PTR_CACHE`). Q1 24.9 → 6.2 ms on 1M rows, now faster than DataFusion on
      one thread. High-cardinality GROUP BY is not yet measured.
- [x] Filter on `Int64` against an `Int64` literal reads values directly
      (`engine.rs`, `Predicate::select_from`). Q2 14.0 → 8.1 ms on 1M rows.
- [x] Decision: string columns keep `Arc<str>`. An offset-buffer layout is a
      breaking change to `StrArray`, and it would only help the join's string
      gather. Revisit if that gather stays the main join cost.
- [x] Join pushdown: a grouped inner join with right-side group keys and
      integer SUM / COUNT / MIN / MAX over left columns aggregates the left side
      first (`sql.rs`, `pushdown_join`). Tests in `tests/pushdown.rs`. Q3
      38 → 7.1 ms on 1M rows, now faster than DataFusion on one thread.
- [x] Branch-free row selection in `for_rows` (`engine.rs`): every row index
      is written, and the length advances only on a match. Q2 9.1 → 3.0 ms on
      1M rows, now faster than the hand loop.
- [x] LIKE: allocation-free byte matcher, and an in-place path for `Utf8`
      columns with the common pattern shapes (`engine.rs`, `like_kind`). Q5
      179 → 14 ms on 1M rows. Tests in `tests/like.rs`.
- [x] Q5 filter copies only the columns the query reads above WHERE
      (`sql.rs`, `filter_keep`). Already in place; its before/after time was
      not measured separately.
- [x] Q4 (1M groups): the dense worker reserves its group vectors up front
      (`aggregate.rs`, `dense_worker`). Parallel 56 → 41 ms, one run each. Still
      about 12× the hand loop. Stage profile (docs/benchmark-comparison.md,
      Q4 stage breakdown): the cross-worker merge is 36% and output-vector
      building about 32%. Key-range partitioning (no merge) was tried and is not
      faster: every thread rescans the input (docs/benchmark-comparison.md). Open:
      a per-group state of 16 bytes or less, and output built from the workers'
      ranges. `samply` needs Administrator on Windows, so it was not used.
      Done since: a single `HAVING SUM` is decided on the accumulators before
      the output is built (Q4 37–44 → 27–28 ms, min of 15 runs).
- [ ] tpt-simd (12e): published to crates.io, but still changing on GitHub.
      Not adopted. Revisit once its API is stable, and only if a measured
      kernel needs explicit lanes.
