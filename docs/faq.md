# FAQ

## What is tpt-strata for?

Embedded analytics in your own process: "sum this column grouped by that
one," over data your program already holds, or over a CSV or Parquet file. It
is a library first, with a small command-line tool on top. It is not a server
and it does not replace a data warehouse.

## Why not DataFusion or DuckDB?

Those are excellent, and for large data, distributed work, or a rich SQL
dialect they are the right choice. tpt-strata makes a different trade:

- **No dependencies in the core crate.** The core is standard library only. It
  adds almost nothing to build time, and it is easy to audit.
- **Errors that name your problem.** A missing column lists the columns you
  have. A type mismatch names your column and both types. A rejected construct
  says why, and where the decision is recorded.
- **Small surface.** A builder API and a SQL subset that covers the common
  aggregation and join queries.

The trade-off is scale and breadth. Everything is held in memory, and the SQL
subset is deliberately small. See the next answers.

## How much data can it handle?

Everything is in memory and every operator materializes its output. Plan for
data that fits comfortably in RAM. The benchmarks in
[docs/benchmarks.md](benchmarks.md) give per-row costs; they do not show limits.
There is no spill to disk and no streaming aggregation yet.

## Is it fast?

It is reasonable for in-process analytics on moderate data. The current hot
paths allocate per row in places. Phase 12 of the [todo list](../todo.md) is
the plan for typed hot paths. Measure your own workload before relying on any
number.

## What SQL is supported?

`SELECT` with columns, `*`, and `SUM`, `COUNT`, `MIN`, `MAX`, `AVG` (including
`COUNT(*)`). `FROM` with equi-joins (inner, `LEFT`, `RIGHT`, `FULL`, chained).
`WHERE` with `AND`, `OR`, `NOT`, parentheses, comparisons, `IN`, `BETWEEN`,
`IS [NOT] NULL`, `[NOT] LIKE`. `GROUP BY`, `HAVING`, `DISTINCT`,
`ORDER BY` (several keys, aliases allowed), `LIMIT`, `OFFSET`. Comments with
`--` and `/* */`. Full list and the rejected constructs:
[v1.1 scope](v1.1-scope.md).

## What SQL is not supported?

Expressions in SELECT (`a + b`, `CASE`, functions other than the aggregates),
subqueries, `CTE`s, window functions, `CAST`, `COUNT(DISTINCT ...)`,
non-equality joins, and table aliases. Each rejected construct gets a
diagnostic that says so.

## How are NULLs handled?

SQL style, with one practical choice. A comparison with a NULL on either side
is unknown and filters the row out. So `col = NULL` and `col != NULL` match
nothing. Use `IS NULL`. Aggregates skip NULLs, except `COUNT(*)`, which counts
rows. `SUM` and `AVG` over no non-null values return NULL. ORDER BY puts NULLs
first in ascending order.

## How do numbers behave?

- Integers compare exactly, including values above 2^53. Mixed integer and
  float comparisons are exact too.
- `SUM` over an integer column returns an integer. Overflow is an error, not a
  silent wrap.
- `AVG` returns a float.
- `NaN` groups with other `NaN`s, and `-0.0` groups with `0.0`. In a
  comparison, `NaN` is unordered, so `<`, `>` and friends are false for it.

## Are strings compared by bytes?

Yes. `<` and `>` on strings compare UTF-8 byte order. `LIKE` works on
characters, so `_` matches one character, including a non-ASCII one.

## Can I join two tables with the same column names?

Yes, in SQL: qualify the names (`orders.id`). Output names drop the qualifier
unless two outputs would collide. From the builder API, `join()` refuses
duplicate output names, so rename one side with `Table::rename_columns` first.

## Does it read Parquet directly?

Yes, through `tpt-strata-parquet`, which is the only crate that depends on
Parquet and Arrow. Types are widened to tpt-strata's four native types. Date
and timestamp columns come in as their raw numbers. Columns of other types can
be skipped with `read_parquet_columns`. See the table in the crate README.

## Can it write files?

CSV, yes (`csv::to_csv`). Parquet, yes (`write_parquet`), with the same types
it reads.

## Is it thread-safe?

Tables and plans are `Send` and `Sync`, and a query does not mutate its input.
Use one `SqlContext` per thread or share it read-only. Nothing in the engine
uses interior mutability.

## Is the output order stable?

Grouped results come out sorted by group key unless you ask for another order.
Use `ORDER BY` whenever the order matters. Ties in `ORDER BY` keep the order
they had before the sort.

## Can I use it from Python?

Not yet. A Python binding is an idea on the roadmap, not something that exists.

## Is it ready for production?

It is pre-release. It has a test suite and regression tests for the fixes in
[todo.md](../todo.md), but it has not been published or run against a real
consumer workload. Pin the exact version you test.

## Where do I report a bug or ask for a feature?

Open an issue on the GitHub repository, using the templates. For a security
issue, use GitHub's private vulnerability reporting (see SECURITY.md), not a
public issue.
