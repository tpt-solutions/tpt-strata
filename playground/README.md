# tpt-strata playground

A browser page that runs tpt-strata's SQL engine as WebAssembly. Edit tables as
CSV, write a query, and run it. Nothing leaves the page.

## Build and run

Install [wasm-pack](https://rustwasm.github.io/wasm-pack/) once, then from the
repository root:

```sh
wasm-pack build crates/tpt-strata-wasm --target web --out-dir ../../playground/pkg --release
wasm-pack build crates/tpt-strata-wasm --target nodejs --out-dir ../../playground/pkg-node --release

# Browser: serve this folder over HTTP (modules do not load from file://)
cd playground && python -m http.server 8000
# then open http://localhost:8000
```

The `pkg/` and `pkg-node/` folders are build output and are not committed.

## Smoke test under Node

```sh
node playground/test/smoke.cjs
```

## What the page can do

- **Run** the query and show the result as a table, with NULLs marked.
- **Explain** prints the physical plan without running it.
- **Check** binds the query against the tables and reports the first problem.
- Errors show the stable kind, for example `[missing-column] ...`.

## Limits

- The whole input is parsed in the page. Keep tables to a few hundred thousand
  rows.
- The engine runs on one thread. A long query freezes the page until it
  finishes, so it is best run in a worker for large inputs.
