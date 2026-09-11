# tpt-strata — Integration Recipe for a Struct-backed Store

**Audience:** the three known consumers (tpt-keystone-db, tpt-aion,
tpt-cloud-observability) and anyone embedding tpt-strata over their own Rust
data structures.

This is a template, not a library. tpt-strata has zero external dependencies
and lets you choose how much of your own store it sees. The binding surface
is three public types: `Table` (input), `QueryBuilder`/`SqlContext` (query),
and `QueryError` (diagnostics).

## 1. Map your row store to a `Table`

Two supported strategies:

### 1a. Value columns — simplest, infers the schema

For stores that are small or already materialized, build columns of
`Scalar` values and let `Table::try_new` infer types and nullability:

```rust
use tpt_strata::{Column, Scalar, Table};

fn from_records(records: &[YourRow]) -> Result<Table, tpt_strata::QueryError> {
    Table::try_new(vec![
        Column::new("service", records.iter().map(|r| Scalar::Str(r.service.clone())).collect()),
        Column::new("latency_ms", records.iter().map(|r| match r.latency {
            Some(v) => Scalar::F64(v),
            None => Scalar::Null,
        }).collect()),
    ])
}
```

`try_new` rejects mixed-type columns and all-null columns with a named
diagnostic, so the mapping is checked, not silent.

### 1b. Explicit schema + batches — avoids the value round-trip

For stores that already hold typed columnar buffers, declare the schema once
and wrap your buffers in native arrays:

```rust
use std::sync::Arc;
use tpt_strata::{ArrayRef, Batch, DataType, Field, Schema, Table};

fn table_over_buffers(
    arrival_ts: Vec<Option<i64>>,
    latency: Vec<Option<f64>>,
) -> Table {
    let schema = Arc::new(Schema::new(vec![
        Field::new("arrival_ts", DataType::Int64, true),
        Field::new("latency_ms", DataType::Float64, true),
    ]));
    let batch = Batch::new_unchecked(
        schema.clone(),
        vec![
            ArrayRef::Int64(tpt_strata::I64Array::from_nulls(arrival_ts)),
            ArrayRef::Float64(tpt_strata::F64Array::from_nulls(latency)),
        ],
    );
    Table::from_batches(schema, vec![batch])
}
```

Pro tip for the memo/observability consumers: `Table::concat` appends batches
with an identical schema, so you can keep appending ingests without rebuilding
the table.

## 2. Query it

Either surface is fine and they produce identical results.

```rust
use tpt_strata::{AggFunc, QueryBuilder};

let avg_by_service = QueryBuilder::new(&table)
    .group_by(vec!["service"])?
    .aggregate("latency_ms", AggFunc::Avg)?
    .order_by("service", false)?
    .execute()?;
```

SQL, when the query is user-authored:

```rust
use tpt_strata::sql::SqlContext;

let mut ctx = SqlContext::new();
ctx.add_table("metrics", &table);
let out = ctx.run("SELECT service, AVG(latency_ms) FROM metrics GROUP BY service")?;
```

## 3. Surface diagnostics to your callers

`QueryError` is the only error type that crosses this boundary, and it already
speaks the caller's language. Route it straight to your logging/UI:

```rust
match query_result {
    Ok(t) => render(t),
    Err(e) => eprintln!("query failed: {e}"),
}
```

Examples of what callers see (wording pinned by tests):

```
No column named 'nonexistent'. Available columns: service, region, latency_ms
Type mismatch in 'latency_ms': expected f64, found str. Cannot compare 'latency_ms' (f64) with a literal of str type
SQL error: 'OR in WHERE' is not supported in tpt-strata v1.1 (deliberate scope decision); combine predicates with AND only. See docs/v1.1-scope.md at line 1, column 37
```

Parse errors also carry the line, the offending text, and a caret.

## 4. Parquet (tpt-keystone-db / tpt-cloud-observability)

If your store ships data as Parquet, the bridge crate converts it for you —
the core engine never learns Parquet exists:

```rust
use tpt_strata_parquet::read_parquet;
let table = read_parquet("metrics.parquet")?;
```

Supported columns: `BOOLEAN`, `INT32`, `INT64`, `DOUBLE`, `STRING`. Other
types fail with a diagnostic naming the column.

## 5. Performance notes

- `ORDER BY` and `JOIN` fully materialize their inputs in memory — keep sorts
  and joins over bounded ranges, or push the sort/join out of the hot path.
- Operators are single-threaded and pull-based; intermediate columns are
  re-materialized per operator.
- For steady per-ingest throughput prefer `Table::concat` over rebuilding
  from records each tick.

## 6. Getting started checklist

1. Pick strategy 1a or 1b for your store.
2. Wrap your data in a `Table` (keep the schema around for reuse).
3. Run one `QueryBuilder` or `SqlContext` query against it.
4. Wire `QueryError` straight into your existing error/UX layer.
5. Add Parquet import at the edge only if your store already speaks it.