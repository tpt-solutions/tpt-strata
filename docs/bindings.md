# Bindings: Python, WebAssembly, and serde

tpt-strata's core is Rust with no dependencies. Three companion crates let you
use it from other places. Each is separate, so you only pay for what you use.

| Use it from | Crate | Built with | Published |
|---|---|---|---|
| Rust structs | `tpt-strata-serde` | `cargo` (workspace) | not yet |
| Browser, Node.js | `tpt-strata-wasm` and [`playground/`](../playground) | `wasm-pack` | not yet |
| Python | `tpt-strata-python` | `maturin` | not yet |

## Serde (Rust)

```rust
use serde::{Deserialize, Serialize};
use tpt_strata::sql::SqlContext;
use tpt_strata_serde::{from_table, to_table};

#[derive(Serialize, Deserialize)]
struct Order { region: String, quantity: i64, price: Option<f64> }

let table = to_table(&orders)?;
let mut ctx = SqlContext::new();
ctx.add_table("orders", &table);
let totals = ctx.run("SELECT region, SUM(quantity) AS quantity FROM orders GROUP BY region")?;
let rows: Vec<Total> = from_table(&totals)?;
```

Flat structs only. Nested structs, lists and maps are rejected with the field
name in the message.

## WebAssembly and the playground

```sh
wasm-pack build crates/tpt-strata-wasm --target web --out-dir ../../playground/pkg --release
cd playground && python -m http.server 8000
```

Open `http://localhost:8000`. The page edits tables as CSV, runs SQL, and shows
the plan (Explain) or a check result. Everything runs in the browser.

From JavaScript, the functions take the tables as one JSON object of name to
CSV text:

```js
import init, { run, check_query } from "./pkg/tpt_strata_wasm.js";
await init();
const json = run(JSON.stringify({ orders: csvText }), "SELECT ... FROM orders");
```

Node.js can use the same build (`--target nodejs`). The smoke test is
`node playground/test/smoke.cjs`.

## Python

Build a wheel with maturin (Python 3.9 or later, a Rust toolchain):

```sh
pip install maturin
cd crates/tpt-strata-python
maturin build --release -o dist
pip install dist/tpt_strata-*.whl
```

```python
import tpt_strata as strata

orders = strata.Table.from_csv("orders.csv")
customers = strata.Table.from_rows(["id", "name"], [[1, "ann"], [2, "ben"]])

totals = strata.sql(
    {"orders": orders, "customers": customers},
    "SELECT customers.name, SUM(orders.quantity) AS units FROM orders "
    "JOIN customers ON orders.customer_id = customers.id GROUP BY customers.name",
)
print(totals)               # a text grid
print(totals.to_rows())     # [(name, units), ...], one tuple per row
totals.to_parquet("totals.parquet")
```

- `Table` can be built from CSV (`from_csv`, `from_csv_text`), Parquet
  (`from_parquet`, optionally with `columns=`), or Python rows (`from_rows`).
  `None` is NULL. `bool`, `int`, `float` and `str` are the supported value types.
- `sql`, `explain` and `check` take a dict of name to `Table`.
- Errors raise `tpt_strata.StrataError`. Its `kind` attribute is the same stable
  name the Rust API returns, for example `missing-column` or `sql`.
- Queries release the GIL while they run, so other Python threads keep going.
- Python ints outside the `i64` range are rejected rather than rounded.

The unit tests are in `crates/tpt-strata-python/tests`:

```sh
python -m unittest discover -s crates/tpt-strata-python/tests -v
```

## Benchmark against DataFusion and DuckDB

See [benchmark-comparison.md](benchmark-comparison.md). The benchmark is in
`bench/compare`, outside the workspace, because DataFusion and DuckDB are large
dependencies.
