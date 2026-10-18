# Publishing

How to release the tpt-strata crates to crates.io. Each release also gets a
tracking file, such as [RELEASE-0.1.0.md](RELEASE-0.1.0.md).

Written for maintainers who cut releases.

## What gets published

Two crates are published. Publish them in this order, because the Parquet
crate depends on the core crate:

| Order | Crate                  | Notes                                  |
| ----- | ---------------------- | -------------------------------------- |
| 1     | `tpt-strata`           | Core engine. Zero external dependencies |
| 2     | `tpt-strata-parquet`   | Parquet bridge. Needs `tpt-strata` on crates.io first |

The other crates set `publish = false`. `tpt-strata-cli`, `tpt-strata-wasm`
and `tpt-strata-python` are never released to crates.io: the CLI is installed
from git or GitHub releases, the WASM build serves the playground, and the
Python package is released to PyPI with maturin.

`tpt-strata-serde` is the planned follow-up. It can be published as a 0.1.x
release once a consumer team has used it. Before that, remove its
`publish = false` line, add a changelog entry, and run the dry run.

## Before you start

1. Confirm the version is the same in `[workspace.package]` in the root
   `Cargo.toml`, and in the `=<version>` pins in each crate's dependencies.
2. Move the `[Unreleased]` entries in the root `CHANGELOG.md` and in the
   per-crate changelogs to the new version and today's date.
3. Make sure the working tree is clean and CI is green on `master`.
4. Check that the minimum Rust versions in each `Cargo.toml` still match what
   CI verifies.

## Dry run

A dry run packages the crate, builds it from the package, and stops before the
upload. Run it for both crates:

```sh
cargo publish --dry-run -p tpt-strata
cargo publish --dry-run -p tpt-strata-parquet
```

Notes:

- `tpt-strata-parquet` can only pass a dry run once the matching `tpt-strata`
  version is on crates.io. Before that, it fails with `no matching package named
  tpt-strata found`. This is expected. Dry run `tpt-strata` first, publish it,
  then dry run `tpt-strata-parquet`.
- Add `--allow-dirty` only for a local check on an uncommitted tree. Do not use
  it for the real release.
- To see exactly which files go into a package, run
  `cargo package --list -p <crate>`.

## Publish

Publish one crate at a time, and wait for each one to appear in the index
before moving to the next:

```sh
cargo publish -p tpt-strata
# wait until `cargo search tpt-strata` shows the new version
cargo publish -p tpt-strata-parquet
```

You need a crates.io API token. Run `cargo login` once, or set
`CARGO_REGISTRY_TOKEN`.

## After publishing

1. Tag the release: `git tag v<version>` and `git push origin v<version>`.
2. Create a GitHub release for the tag, with the changelog entries as the notes.
3. Check the docs build on docs.rs for both crates.
4. Check the crates.io badges in the READMEs show the new version.
