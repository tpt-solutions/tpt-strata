# Release 0.1.0

Tracks the first public release of tpt-strata. The general procedure is in
[PUBLISHING.md](PUBLISHING.md); this file records where this release stands.

**Status:** prep done and checked locally. Next: push, CI, Release check
workflow. Nothing is tagged or published.

## Decisions

- **Version 0.1.0, not 1.1.0** (2026-10-18). The 1.0.0 and 1.0.1 versions in
  the changelogs were internal milestones and never reached crates.io. A 0.x
  version says the API may still change, and the consumer teams have not yet
  run their real queries against it. Their feedback may change public surface
  (CAST, Parquet streaming, the `StrArray` layout). Move to 1.0 once a consumer
  depends on it in production.
- **Two crates are published:** `tpt-strata`, then `tpt-strata-parquet`. The
  CLI, serde, WASM and Python crates stay `publish = false`.
- **Exact version pins stay** (`=0.1.0` between the workspace crates).
- **tpt-simd is not part of 0.1.0** (2026-10-18). The core crate's zero
  dependencies are a selling point, and Phase 12e adopts tpt-simd only once its
  API is stable on crates.io and a measured kernel needs explicit lanes.
  Evaluate it after this release.

## 1. Prep

- [x] Version `0.1.0` in `[workspace.package]` in the root `Cargo.toml`.
- [x] `=0.1.0` pins in the cli, parquet, python, serde and wasm manifests.
- [x] Python package: `version` in `crates/tpt-strata-python/Cargo.toml` and
      `pyproject.toml`, and the `__version__` test in
      `crates/tpt-strata-python/tests/test_tpt_strata.py`.
- [x] Changelogs (root and each crate): `[Unreleased]` is now
      `[0.1.0] - unreleased`; put the date in on publish day. The old 1.0.0 and
      1.0.1 sections are relabelled as pre-release history that was never
      published.
- [x] User-visible "v1" / "v1.1" wording. Error messages in
      `crates/tpt-strata/src/sql.rs` say "not supported in tpt-strata v1.1" and
      "tpt-strata v1 has no CAST support". Reword them without a version, and
      update the snapshot tests that pin the text.
- [x] Rename `docs/v1.1-scope.md` (for example to `docs/scope.md`) and fix the
      links to it: README, `sql.rs`, error text, other docs.
- [x] `PUBLISHING.md`: no hard-coded version left.
- [x] `todo.md`: Phase B notes still say the workspace needs Rust 1.85; it is
      1.88 since the Arrow 60 update.
- [x] Leave `bench/compare/results.md` and the benchmark docs as they are.
      They record what was measured at the time. Same for
      `validation-memo.md` and `spec.txt`.
- [x] License files in each published crate. `LICENSE-MIT` and
      `LICENSE-APACHE` are copied into `crates/tpt-strata` and
      `crates/tpt-strata-parquet`, so the packages carry the license text.
- [x] README install snippets say `"0.1"`.

## 2. Verify

- [x] CI green on `master` after the prep commits (pushed as `d85079b`).
- [x] `cargo publish --dry-run -p tpt-strata` passes (locally, 2026-10-18).
- [x] `cargo package --list -p tpt-strata` and `-p tpt-strata-parquet`: no
      stray files. Sources, tests, examples, benches, README, CHANGELOG and
      licenses only.
- [x] `cargo doc` builds with warnings as errors.
- [ ] No "v1" wording left in the published sources or READMEs. Still open:
      `ARCHITECTURE.md`, `docs/scope.md`, and the doc comments in
      `crates/tpt-strata/tests/integration.rs` and `tests/regressions.rs`.
      The changelogs and READMEs are clean; the pre-release history keeps its
      old wording on purpose.
- [ ] Run the Release check workflow by hand (Actions, `workflow_dispatch`).

## 3. Publish (permanent: a published version cannot be deleted)

- [ ] `cargo login` with a crates.io token from an account that should own
      the crates.
- [ ] `cargo publish -p tpt-strata`
- [ ] Wait until `cargo search tpt-strata` shows 0.1.0.
- [ ] `cargo publish --dry-run -p tpt-strata-parquet`, then
      `cargo publish -p tpt-strata-parquet`.

## 4. After publishing

- [ ] Tag: `git tag v0.1.0` and `git push origin v0.1.0`.
- [ ] GitHub release for `v0.1.0`, with the changelog entries as notes.
- [ ] docs.rs builds for both crates.
- [ ] README badges show 0.1.0.
- [ ] `docs/quickstart.md`: drop "not on crates.io yet" and the path
      dependency; use `cargo add tpt-strata`.
- [ ] Changelogs: replace `unreleased` with the publish date.
- [ ] `todo.md` Phase 9 and Phase B: mark publishing done.

## 5. Announce

- [ ] Tell the consumer teams (tpt-keystone-db, tpt-aion,
      tpt-cloud-observability) that 0.1.0 is on crates.io.
- [ ] Ask each team for its real SQL, for `docs/consumer-queries.md`. This
      unblocks Phase 11a and the decisions waiting on a consumer: Parquet
      streaming, CSV date inference, CAST and arithmetic.

## Log

- 2026-10-18: CI fixed (Arrow and Parquet 60, Rust 1.88 for the Arrow
  crates, cargo-deny config, Python date types). Version set to 0.1.0.
  Both crate names are free on crates.io.
- 2026-10-18: prep done. Local checks: fmt, clippy (`-D warnings`), 260
  workspace tests, Rust 1.75 core and serde tests, Rust 1.88 workspace build,
  rustdoc, Python tests (18), `cargo publish --dry-run -p tpt-strata`.
- 2026-10-18: pre-publish pass. `cargo publish --dry-run -p tpt-strata` passes
  (with `--allow-dirty`, since the tree is uncommitted; the real run needs a
  clean tree). `tpt-strata-parquet` fails as expected: no `tpt-strata` on
  crates.io yet. Changelogs: one `0.1.0` section per file (duplicate headers
  merged), the diagnostics entry now lists the constructs still rejected
  (`CAST`, `COUNT(DISTINCT)`, other functions), and the Parquet changelog says
  dates import as native types. READMEs: data model and error table cover
  Date, Timestamp, Io and Import; Parquet MSRV is 1.88. Still open: CI status
  (not checked, `gh` is not installed here), and `examples/profile_q4.rs` plus
  `examples/data/*.csv` ship in the core package. Fixed: `profile_q4.rs` is
  now excluded from the package (`exclude` in the core `Cargo.toml`); the CSV
  data stays because a shipped example uses it. Packaged core is 55 files.
