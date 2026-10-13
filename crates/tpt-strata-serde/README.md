# tpt-strata-serde

Serde support for [tpt-strata](../tpt-strata). Turn a slice of your structs
into a table, query it with SQL, and read the results back into structs.

```rust
use serde::{Deserialize, Serialize};
use tpt_strata_serde::{from_table, to_table};

#[derive(Serialize, Deserialize)]
struct Order { region: String, quantity: i64, price: Option<f64> }

let table = to_table(&orders)?;         // Vec<Order> -> Table
let back: Vec<Order> = from_table(&table)?; // Table -> Vec<Order>
```

Column names come from the struct's field names. Column types come from the
Rust types, and `Option` fields become NULL when they are `None`. See the crate
documentation for the full mapping.

## Why a separate crate

The core `tpt-strata` crate has no dependencies, and that is a hard rule.
Serde is an external dependency, so it lives here.

## Limits

- Only flat structs convert. A nested struct, a list, or a map is an error that
  names the field.
- A column that is entirely `None` cannot be typed from the data, so it is an
  error.
- `to_table` on an empty slice is an error, because there is nothing to infer
  types from. Build an empty table with `Table::empty` instead.

This crate is not published yet.
