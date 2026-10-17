# Quickstart

About five minutes, from nothing installed to a query over your own data.

tpt-strata is not on crates.io yet, so depend on it by path or by git for now.
From a checkout of this repository:

```toml
# Cargo.toml of your project
[dependencies]
tpt-strata = { path = "../tpt-strata/crates/tpt-strata" }
# Only if you read or write Parquet:
tpt-strata-parquet = { path = "../tpt-strata/crates/tpt-strata-parquet" }
```

Once published, this becomes `cargo add tpt-strata`.

## 1. Query rows from Rust

```rust
use tpt_strata::csv::parse_csv;
use tpt_strata::format::print_table;
use tpt_strata::sql::SqlContext;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sales = parse_csv(
        "region,amount\nnorth,10.5\nsouth,4\nnorth,2\n",
    )?;

    let mut ctx = SqlContext::new();
    ctx.add_table("sales", &sales);

    let report = ctx.run(
        "SELECT region, SUM(amount) AS total FROM sales GROUP BY region ORDER BY total DESC",
    )?;
    print_table(&report);
    Ok(())
}
```

Output:

```text
+--------+-------+
| region | total |
+--------+-------+
| north  | 12.5  |
| south  | 4     |
+--------+-------+
```

You can also build rows in code with `Table::from_rows`:

```rust
use tpt_strata::{Scalar, Table};

let t = Table::from_rows(
    &["city", "population"],
    vec![
        vec![Scalar::Str("nyc".into()), Scalar::I64(8_000_000)],
        vec![Scalar::Str("sfo".into()), Scalar::I64(800_000)],
    ],
)?;
```

## 2. Use the command line

The `strata` binary runs SQL over CSV and Parquet files. Build it from the
workspace:

```sh
cargo build -p tpt-strata-cli
```

Each file becomes a table named after the file (without its extension):

```sh
strata crates/tpt-strata/examples/data/orders.csv \
  "SELECT region, SUM(quantity) AS units FROM orders GROUP BY region ORDER BY units DESC"
```

Join two files by passing both:

```sh
strata orders.csv customers.csv \
  "SELECT customers.name, SUM(orders.quantity) AS units FROM orders JOIN customers ON orders.customer_id = customers.customer_id GROUP BY customers.name"
```

Useful flags:

| Flag | What it does |
|---|---|
| `--csv` | print results as CSV |
| `--explain` | print the physical plan instead of running the query |
| `--check` | check the query against the file schemas without running it |

With no query argument, `strata` reads one query per line from stdin.

## 3. Check SQL in CI

`SqlContext::check` binds a query against the table schemas without reading
the data, so a misspelled column fails the build:

```rust
ctx.check("SELECT region, SUM(amount) FROM sales GROUP BY region")?;
```

The CLI does the same with `--check`.

## 4. Read and write Parquet

```rust
use tpt_strata_parquet::{read_parquet, write_parquet};

let table = read_parquet("metrics.parquet")?;
write_parquet(&table, "copy.parquet")?;
```

If a file has a column of a type tpt-strata does not import (for example
binary), read only the columns you need:

```rust
let table = tpt_strata_parquet::read_parquet_columns("events.parquet", &["id", "ts"])?;
```

## 5. Handle errors

Every error has a stable `kind()` for code that branches on the error type,
and a message for people:

```rust
match ctx.run("SELECT nope FROM sales") {
    Ok(t) => print_table(&t),
    Err(e) if e.kind() == "missing-column" => eprintln!("check your column names: {e}"),
    Err(e) => return Err(e.into()),
}
```

Kinds: `missing-column`, `type-mismatch`, `unsupported`, `sql`, `io`, `import`.

## Next

- [Cookbook](cookbook.md): recipes for the common questions
- [FAQ](faq.md): what it is for, what it is not for, and what is not supported yet
- [scope decisions](scope.md): which SQL features are deliberately deferred
