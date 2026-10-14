# tpt-strata-cli

`strata` runs SQL over Parquet and CSV files from the command line, powered by
[tpt-strata](../tpt-strata) and [tpt-strata-parquet](../tpt-strata-parquet).

```sh
strata sales.parquet "SELECT region, SUM(amount) AS total FROM sales GROUP BY region"
```

Each file becomes a table named after the file name without its extension.
Characters other than letters, digits and `_` become `_`, so `my-orders.csv`
is queried as `my_orders`.

## Usage

```text
strata <FILE>... [SQL] [OPTIONS]
```

With no SQL argument, `strata` starts a prompt that reads one query per line.

| Option      | Effect                                              |
| ----------- | --------------------------------------------------- |
| `--csv`     | Print results as CSV instead of a grid              |
| `--explain` | Print the physical plan instead of running the query |
| `--check`   | Bind the query against the files without running it |
| `-h, --help` | Show help                                          |

## Examples

Join two CSV files:

```sh
strata orders.csv customers.csv "SELECT orders.id, customers.name FROM orders JOIN customers ON orders.cust = customers.id"
```

Export a result as CSV:

```sh
strata sales.parquet --csv "SELECT region, SUM(amount) AS total FROM sales GROUP BY region" > totals.csv
```

Check a query without running it, for example in a script:

```sh
strata sales.parquet --check "SELECT region FROM sales" && echo ok
```

## Exit codes

- `0`: the query ran, or the check passed.
- `2`: a usage problem, a file that could not be read, or a query error.

## Installation

This crate is not published yet. Build it from the workspace:

```sh
cargo build --release -p tpt-strata-cli
./target/release/strata --help
```

## License

Licensed under either of [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE), at your option.
