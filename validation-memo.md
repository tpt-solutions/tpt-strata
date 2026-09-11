# Phase 0 — Validation & Go/No-Go Memo

**Date:** 2026-09-10
**Author:** tpt-strata validation
**Status:** GO

---

## 1. Validation Method

Two prototypes were built and run against the same workload:

- **Incumbent:** Apache DataFusion v55.0.0 (latest stable as of 2026-09-10)
- **tpt-strata:** Zero-dependency sketch implementing native columnar format + builder API

Both solved the same "point at data, get an answer" case: in-memory employee data (5 rows, 3 columns) with GROUP BY aggregation and filtered queries.

---

## 2. Comparison Results

### Basic Query (GROUP BY + aggregate)

| Metric | DataFusion v55 | tpt-strata sketch |
|---|---|---|
| Total dependencies | **282 packages** | **0 packages** |
| First-build compile time | **~2 min (debug)** | **~1.2 sec (debug)** |
| Caller code (basic query) | ~40 lines (schema + data + ctx + register + SQL) | ~15 lines (columns + builder chain) |
| Trait implementations needed | 0 | 0 |
| Async runtime required | Yes (tokio) | No |
| In-memory data registration | `MemTable::try_new(schema, vec![vec![batch]])` + `register_table` | `Table::new(vec![Column { ... }])` |

### Diagnostic Comparison (same broken queries)

**Missing column (`nonexistent`):**

| Engine | Error text |
|---|---|
| DataFusion | `Schema error: No field named nonexistent. Valid fields are employees.department, employees.salary, employees.years.` |
| tpt-strata | `No column named 'nonexistent'. Available columns: department, salary, years` |

**Type mismatch (`department > 42`):**

| Engine | Error text |
|---|---|
| DataFusion | `Arrow error: Cast error: Cannot cast string 'eng' to value of Int64 type` |
| tpt-strata | `Type mismatch in 'department': expected str, found i32. Cannot compare department (str) with value of i32 type` |

**Unsupported construct (`::INT` cast):**

| Engine | Error text |
|---|---|
| DataFusion | `Arrow error: Cast error: Cannot cast string 'eng' to value of Int32 type` |
| tpt-strata | `Unsupported operation: CAST not available in builder API (add at SQL layer)` |

---

## 3. Analysis

### Compile times / dependency surface
DataFusion pulls 282 transitive crates. For an embedded library used by three TPT projects, this is a meaningful tax on every CI run, every contributor onboarding, and every downstream binary size. tpt-strata with zero external dependencies compiles in ~1 second — a >100x improvement.

### Error messages
DataFusion's errors are technically correct but leak implementation details (`Arrow error: Cast error: Cannot cast string 'eng' to value of Int64 type`). A caller who doesn't know about Arrow's type names has to decode what "Int64 type" means in their context. tpt-strata's errors name the caller's column and the actual types in terms the caller used. This matches the spec's non-negotiable #3.

### Trait burden
Both engines require zero trait implementations for the basic case. The incumbent's friction appears at extension points (custom data sources, custom functions), not the happy path. tpt-strata's builder API deliberately avoids exposing trait-based extension for v1.

### Integration with tpt-keystone-db
DataFusion requires async (tokio), an Arrow schema mirror, and `MemTable` registration. tpt-strata's sketch uses plain structs and synchronous calls — closer to "boilerplate-free integration with tpt-keystone-db's own types."

---

## 4. Bridge Surface Decision (Phase 0.5)

**V1 scope:**
- **Import bridge:** Read Parquet into tpt-strata's native format. This is needed because tpt-keystone-db and tpt-cloud-observability likely store data in Parquet.
- **Export bridge:** Defer. None of the three consumers currently require PyArrow-readable output from the query engine. Revisit when a consumer asks for it.

**Bridge isolation rule confirmed:** The Parquet dependency (`parquet` crate) appears only in the bridge crate, never in core.

---

## 5. Decision

**GO.** The ease-of-use validation in Section 3 is positive:

1. **Compile times:** >100x faster (0 deps vs 282).
2. **Error messages:** Caller-facing, naming the caller's own columns and types.
3. **Integration boilerplate:** Synchronous, zero-async, no Arrow schema mirroring.
4. **License:** MIT/Apache-2.0 dual (vs DataFusion's Apache-2.0 only).

The thesis that tpt-strata is a meaningful usability win over the incumbent is confirmed by this validation. Proceed to Phase 1.

---

*This memo satisfies Phase 0 tasks 0.5, 0.6, and 0.7.*

---

## 6. Phase 5 Re-Run — Finished Implementation vs. Phase 0 Records

**Date:** 2026-09-11

The three Phase 0 diagnostic cases were re-run against the finished
implementation. The exact wording below is pinned by snapshot tests in
`crates/tpt-strata/tests/diagnostics.rs` (every assertion passes).

**Missing column (`nonexistent`):**

| Engine | Error text |
|---|---|
| DataFusion (recorded Phase 0) | `Schema error: No field named nonexistent. Valid fields are employees.department, employees.salary, employees.years.` |
| tpt-strata (builder + SQL) | `No column named 'nonexistent'. Available columns: department, salary, years` |

**Type mismatch (`department > 42`):**

| Engine | Error text |
|---|---|
| DataFusion (recorded Phase 0) | `Arrow error: Cast error: Cannot cast string 'eng' to value of Int64 type` |
| tpt-strata (builder) | `Type mismatch in 'department': expected str, found i32. Cannot compare 'department' (str) with a value of i32 type` |
| tpt-strata (SQL) | `Type mismatch in 'salary': expected f64, found str. Cannot compare 'salary' (f64) with a literal of str type` |

**Unsupported construct (`CAST`/`::INT`):**

| Engine | Error text |
|---|---|
| DataFusion (recorded Phase 0) | `Arrow error: Cast error: Cannot cast string 'eng' to value of Int32 type` |
| tpt-strata (SQL) | `SQL error: unsupported cast: tpt-strata v1 has no CAST support; provide values in the target type directly` |

### Verdict

The ease-of-use thesis holds in the finished implementation:

1. **Missing column** — tpt-strata names the caller's column and lists the
   schema's actual columns (same as Phase 0, unchanged through phases 2-5).
2. **Type mismatch** — tpt-strata names the column, the expected type (in
   `str`/`f64`/`i32` terms), the actual type, and a plain-language explanation;
   DataFusion still leaks `Arrow ... Int64 type` internals.
3. **Unsupported construct** — tpt-strata says exactly what is unsupported and
   what to do instead; DataFusion reports a cast failure that only makes sense
   if you know Arrow's type system.

No regression in wording from the Phase 0 sketch. Diagnostic wording is now
locked by regression tests, so it cannot silently drift in future phases.
