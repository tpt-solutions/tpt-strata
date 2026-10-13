# tpt-strata-wasm

The WebAssembly build of [tpt-strata](../tpt-strata), used by the browser
playground in [`playground/`](../../playground). The module is about 300 KB
after optimisation.

```sh
wasm-pack build crates/tpt-strata-wasm --target web --release
```

```js
import init, { run } from "./pkg/tpt_strata_wasm.js";
await init();

const tables = JSON.stringify({ orders: "id,amount\n1,2.5\n2,4\n" });
const result = JSON.parse(run(tables, "SELECT SUM(amount) AS total FROM orders"));
// { columns: ["total"], rows: [[6.5]], rowCount: 1 }
```

Exported functions: `run` (JSON result), `render_query` (text grid),
`explain_query` (plan), `check_query` (validate only), and `version`.

Errors are thrown as `"[kind] message"`, where the kind is the stable
`QueryError::kind()` name.

This crate is not published yet.
