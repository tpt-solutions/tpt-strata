# Changelog

All notable changes to the `tpt-strata-python` package will be documented in
this file.

This file covers the Python bindings only. Workspace-wide changes are recorded
in the [root changelog](../../CHANGELOG.md), and engine changes are in the
[`tpt-strata` changelog](../tpt-strata/CHANGELOG.md).

The format is based on [Keep a Changelog](https://keepachangelog.com/), and
this package follows [Semantic Versioning](https://semver.org/).

## [0.1.0] - unreleased

### Added

- `DATE` and `TIMESTAMP` values come back from `to_rows()` as `datetime.date`
  and naive `datetime.datetime`. `from_rows()` accepts them too. A datetime with
  a time zone is rejected, because tpt-strata timestamps have none. Before
  this, the package did not build against the current engine.
- Per-crate changelog. Earlier history is in the root changelog.

### Changed

- Minimum Rust version is 1.88 (was 1.85), following the Arrow 60 update.
