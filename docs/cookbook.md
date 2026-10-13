# Cookbook

Short, runnable-in-spirit recipes. Each SQL snippet runs against a `SqlContext`
where the named tables are registered. The sample data is in
[`crates/tpt-strata/examples/data`](../crates/tpt-strata/examples/data), and
[`examples/joins_and_aliases.rs`](../crates/tpt-strata/examples/joins_and_aliases.rs)
runs most of these.

## Filtering

```sql
-- Comparisons: = != <> < <= > >=
SELECT order_id FROM orders WHERE quantity > 2 AND region = 'north'

-- Ranges, inclusive on both ends
SELECT order_id FROM orders WHERE unit_price BETWEEN 2 AND 10

-- Pattern match: % is any run, _ is one character
SELECT name FROM customers WHERE name LIKE 'A%'
SELECT name FROM customers WHERE name NOT LIKE '%o_'

-- NULL tests. "= NULL" never matches, so use IS NULL.
SELECT order_id FROM orders WHERE quantity IS NULL
SELECT order_id FROM orders WHERE quantity IS NOT NULL
```

Numbers compare exactly, including large integers and mixed integer and float
literals (`WHERE id > 2.5` works on an integer column).

## Aggregation

```sql
-- Group and summarise. COUNT(*) counts every row. COUNT(col) skips NULLs.
SELECT region, COUNT(*) AS orders, SUM(quantity) AS units
FROM orders GROUP BY region

-- Filter groups with HAVING. The aggregate need not appear in SELECT.
SELECT region FROM orders GROUP BY region HAVING SUM(quantity) >= 5

-- Distinct values. This is GROUP BY over the listed columns.
SELECT DISTINCT product, region FROM orders
```

`SUM` over an integer column returns an integer and fails with an error if it
overflows `i64`. `AVG` always returns a float.

## Sorting and paging

```sql
-- Several keys, each with its own direction. ORDER BY may use an alias.
SELECT product, SUM(quantity) AS units FROM orders
GROUP BY product ORDER BY units DESC, product ASC

-- Page two of ten rows
SELECT order_id FROM orders ORDER BY order_id LIMIT 10 OFFSET 10
```

Nulls sort first in ascending order and last in descending order.

## Joins

```sql
-- Qualify columns that exist in both tables. Output names drop the qualifier
-- unless two outputs would then collide.
SELECT orders.order_id, customers.name
FROM orders JOIN customers ON orders.customer_id = customers.customer_id

-- The ON clause can name either side first.
SELECT orders.order_id, customers.name
FROM orders JOIN customers ON customers.customer_id = orders.customer_id
```

An unqualified name that exists in both tables is an error that lists the
qualified choices. Joins are inner equi-joins on one column per side. Left
joins, multiple joins, and self-joins are not supported yet.

To join two tables that share a column name from code, rename one side first:

```rust
let customers = customers.rename_columns(&["cust_id", "name", "tier"])?;
```

## Aliases

```sql
-- AS names an output column. Use the alias in ORDER BY and HAVING.
SELECT region, SUM(quantity) AS units FROM orders
GROUP BY region HAVING units > 3 ORDER BY units DESC
```

## Reading and writing data

```rust
use tpt_strata::csv::{parse_csv, to_csv};

// CSV in: the first row is the header. Types are inferred (i64, f64, bool, str).
// Empty unquoted fields are NULL.
let t = parse_csv(&std::fs::read_to_string("orders.csv")?)?;

// CSV out
std::fs::write("report.csv", to_csv(&result))?;
```

```rust
use tpt_strata_parquet::{read_parquet, read_parquet_bytes, write_parquet};

let t = read_parquet("events.parquet")?;
write_parquet(&t, "events-copy.parquet")?;

// From memory, for example after an HTTP download
let t = read_parquet_bytes(bytes::Bytes::from(downloaded))?;
```

## Reading part of a file

```rust
// Read only the named columns. Unsupported types in other columns are not decoded.
let t = tpt_strata_parquet::read_parquet_columns("events.parquet", &["id", "amount"])?;
```

## Builder API

When queries are built in code, the builder checks names and types as you go:

```rust
use tpt_strata::{AggFunc, QueryBuilder, Scalar};

let result = QueryBuilder::new(&orders)
    .filter("quantity", ">", Scalar::I64(1))?
    .group_by(vec!["region"])?
    .count_all()                       // COUNT(*)
    .aggregate("quantity", AggFunc::Sum)?
    .project(vec!["region", "SUM(quantity)"])?
    .order_by("SUM(quantity)", true)?
    .limit(5)
    .execute()?;
```

## Seeing the plan

```rust
println!("{}", ctx.explain("SELECT region FROM orders WHERE quantity > 2 LIMIT 5")?);
// Limit 5 offset 0
//   Project [region]
//     Filter quantity > 2
//       Scan rows=10 columns=[...]
// Read it from the root down: each line feeds the line below it.
```

## Checking queries in CI

```rust
// Fails if the SQL does not parse or names a column that does not exist.
// It does not read any data.
ctx.check("SELECT region, SUM(quantity) FROM orders GROUP BY region")?;
```

## Handling errors

```rust
match ctx.run(sql) {
    Ok(table) => print_table(&table),
    Err(e) => match e.kind() {
        "missing-column" => eprintln!("unknown column: {e}"),
        "sql" => eprintln!("could not parse the query: {e}"),
        "unsupported" => eprintln!("not supported yet: {e}"),
        _ => eprintln!("{e}"),
    },
}
```

## Building a table from rows

```rust
use tpt_strata::{Scalar, Table};

let t = Table::from_rows(
    &["sensor", "reading"],
    vec![
        vec![Scalar::Str("a".into()), Scalar::F64(0.5)],
        vec![Scalar::Str("b".into()), Scalar::Null],
    ],
)?;
```

A column that is all NULL cannot have its type inferred by `Table::try_new`.
Build it through `Batch` with an explicit `Schema` instead.
