## What this changes

<!-- One or two sentences. Link the todo.md item or issue if there is one. -->

## How it was checked

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all --all-features`
- [ ] New behaviour has a regression test (name the file)
- [ ] Changed error text is reflected in the diagnostic tests

## Checklist

- [ ] The core crate (`crates/tpt-strata`) still has no external dependencies
- [ ] User-facing changes are in the relevant CHANGELOG under *Unreleased*
- [ ] Docs (README, `docs/`) are updated if behaviour or supported SQL changed
