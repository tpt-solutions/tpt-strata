# tpt-strata — Architecture

This document explains how the crate is split, what lives where, and why the
boundaries exist. It exists so a newcomer can see the shape of the engine
before reading code. The governing rules are the **non-negotiables** in
`spec.txt` §2; anything below that contradicts them is a bug.

## Crate layout

```
crates/
  tpt-strata/          The core crate: format + engine + builder + SQL + diagnostics
  tpt-strata-parquet/  The import bridge: reads Parquet into the native format
```

Two rules follow directly from the non-negotiables:

1. **The native format and engine are independent** (non-negotiable #1).
   `tpt-strata` has **zero external dependencies** — it is not built on, and
   is not wire-compatible with, any external in-memory columnar standard.
   The exchange shape between engine operators is tpt-strata's own `Batch`.
2. **Interchange is a boundary concern** (non-negotiable #2). The only place
   an external columnar interop dependency (`parquet`, `arrow`) may appear is
   the `tpt-strata-parquet` crate. The bridge maps *in* to the native format;
   the core never learns what Parquet or Arrow is.

```
┌────────────────────────────┐   native Batch   ┌──────────────────────────┐
│        tpt-strata          │ ───────────────► │     tpt-strata-parquet   │
│                            │ ◄─────────────── │   (reads external data   │
│  zero external deps        │    native Batch  │    into native format)   │
└────────────────────────────┘                  └──────────────────────────┘
```

## Core crate module map

| Module | Responsibility |
|---|---|
| `types.rs`    | `Scalar` — the unit value type and its ordering/comparison helpers |
| `schema.rs`   | `DataType`, `Field`, `Schema` — the caller-facing shape of columns |
| `array.rs`    | `Array` trait + typed nullable arrays (`BoolArray`… `StrArray`) + `ArrayRef` |
| `batch.rs`    | `Batch` — one schema + one array per field; the streaming unit of execution |
| `format.rs`   | `Column`/`Table` — building tables from Rust data; `print_table` |
| `error.rs`    | `QueryError` — diagnostics tied to the caller's schema, not plan nodes |
| `engine.rs`   | Physical operators: `DataSource`, `FilterExec`, `ProjectExec`, `AggregateExec`, `JoinExec`, `SortExec`, `LimitExec` |
| `query.rs`    | `QueryBuilder` + `join()` — the trait-free builder API on top of the engine |
| `sql.rs`      | Hand-written lexer/parser/binder (`SqlContext`) compiling SQL to the engine |

Dependency direction: the upper modules are layers over the lower ones.
`error.rs` is the leaf every layer reports through.

## The format / engine boundary

- **Format** (`types`, `schema`, `array`, `batch`) defines *what data looks
  like*: a `Batch` is a fixed schema plus one `Vec<Option<T>>`-backed array
  per field. Null is explicit; there is no sentinel value.
- **Engine** (`engine.rs`) defines *how it moves*: every operator implements
  the `PhysicalPlan` trait (`schema()` + `execute()`), taking an input plan
  and producing `Vec<Batch>`.
- The only contract between layers is `Batch`. No operator depends on what
  produced its input, and no operator knows about SQL, the builder, or
  Parquet.

### Execution characteristics (v1, deliberately simple)

- Operators are **pull-based and in-process**; each `execute()` call fully
  materializes its output batches.
- `AggregateExec` accumulates group state in a `HashMap` keyed by the group
  key scalars; global aggregates (no `GROUP BY`) emit a single row even over
  zero input rows.
- `JoinExec` is a hash join: it indexes the right input by key, then walks
  the left input, emitting **one contiguous batch** for all matches.
- `SortExec` materializes all rows into per-column vectors, sorts an index
  array, then rebuilds native columns.
- `LimitExec` stops as soon as its budget is exhausted (it short-circuits
  remaining input batches).
- These are single-threaded. Parallelism, vectorization, and code generation
  are deliberately out of scope for v1 (see `spec.txt` §4).

## The builder and SQL layers

Both layers compile down to the same physical operators. This is why
`builder_matches_engine_results` and the integration suite can assert that a
SQL query and the equivalent builder chain return identical tables.

- **Builder** (`query.rs`): zero trait implementations for the common case.
  `QueryBuilder` is a consuming chain that lowers each step into an operator
  stack over a `DataSource`.
- **SQL** (`sql.rs`): the v1 subset (filter, project, aggregates incl.
  `COUNT(*)`, join, group by, order by, limit). Notable v1 simplifications:
  `AS` aliases are parsed and discarded (order/project by the underlying
  expression), types are coerced from numeric literals to the compared
  column's type, and keywords are case-insensitive while identifiers are not.

## The bridge-isolation rule in practice

`crates/tpt-strata-parquet/Cargo.toml` is the *only* manifest in the
workspace that may list `parquet`/`arrow`. The bridge:

1. opens a Parquet file,
2. maps a supported subset of Arrow/Parquet types (`BOOLEAN`, `INT32`,
   `INT64`, `DOUBLE`, `STRING`) to `tpt_strata::DataType`,
3. rejects any other type with a diagnostic naming the offending column,
4. materializes columns into native arrays (including nulls), preserving
   values across row groups, and returns a `tpt_strata::Table`.

"The import/export boundary" therefore means: *external formats are converted
to the native format at the edge, and everything inside the edge speaks
`Batch`.* If a new external format needs supporting, it gets a new bridge
crate that also depends only on the native format — never the other way
around.

## Diagnostics as a cross-cutting concern

`spec.txt` §2 non-negotiable #3: every caller-facing error names the caller's
own problem in the caller's own terms. `QueryError` is that contract, and the
specific wording is pinned by snapshot tests in
`crates/tpt-strata/tests/diagnostics.rs` so it cannot drift silently. The
bridge reuses the same `QueryError` type, so a schema failure at the import
edge reads like a schema failure inside a query.

## Testing and benchmarking

- Unit suites: `tests/engine.rs` (operators), `tests/sql.rs` (SQL surface),
  `tests/diagnostics.rs` (error wording), `tests/integration.rs`
  (end-to-end scenarios from `spec.txt` §4), and the bridge's
  `crates/tpt-strata-parquet/tests/import.rs`.
- Bench: `benches/queries.rs` (zero deps), recorded in `docs/benchmarks.md`.