# String layout: design note

Status: proposal, for decision. Nothing here is implemented.

Written for: the maintainers of tpt-strata, who decide the public API and the
format. It follows the measurements in [benchmark-comparison.md](benchmark-comparison.md).

## The problem

`StrArray` stores one `Arc<str>` per row (`crates/tpt-strata/src/array.rs`,
the `impl_array!` for `StrArray`). Costs that follow from that:

- **Size.** A fat pointer is 16 bytes per row, before the string bytes.
- **Scattered reads.** A filter reads the pointer, then the string it points to.
  Strings built from separate allocations are spread across the heap.
- **Clone cost.** Gathering rows (joins, sorts, filters that keep a column)
  clones an `Arc`, which is an atomic increment per row.

Measured (Windows, 1M rows, Q5 `LIKE 'c%'`, 60 runs each): one `Arc` per row
gave 6.2–6.6 ms, and one shared `Arc` per region gave 4.5–5.5 ms. The benchmark
path already shares strings (the Parquet reader interns them), so the 4.1 ms
Q5 figure in the benchmark is the shared case. The remaining cost is the
pointer and the pointer chase. That is measured for the shared case only.

Q3 (join) is limited by the join's gather, which copies a column's rows through
`Vec<Option<T>>`. Its string part clones an `Arc` per row. This is the cost the
earlier decision in the benchmark doc was about.

## Options

### A. Offset buffer (Arrow-style)

`values: Vec<u8>` holds all bytes. `offsets: Vec<u64>` (or `u32`) holds the
start of each row. `value(i)` returns `&str` from the buffer, with no `Arc`.

- Per row: 8 bytes of offset, plus the bytes once. A gather copies bytes, and
  never touches a refcount.
- Breaking: `value_ref` returns `&Arc<str>` today. It has no equivalent, and
  `new`/`from_parts` take `Vec<Arc<str>>`. Nine call sites use `value_ref`, and
  the Parquet test does too.
- Expected effect: a scan reads offsets and bytes sequentially. Gains on Q5 are
  plausible, but unmeasured. Long strings make gathers copy more bytes.

### B. Dictionary inside `StrArray` (no change to the public API)

`StrArray` stores either the current plain `Vec<Arc<str>>`, or a dictionary:
`keys: Vec<u32>` (one per row) and `dict: Vec<Arc<str>>` (one per distinct
value). `value_ref(i)` returns `&dict[keys[i]]` in both cases, so the public
signatures do not change.

- Per row: 4 bytes. A gather copies `u32` keys, with no `Arc` clone.
- Beyond the layout, the engine can evaluate `LIKE` and equality once per
  distinct value and then look up a result per row. For Q5 that is 8 pattern
  tests instead of 1M. This is the larger expected gain.
- Build cost: the Parquet reader already deduplicates values per batch, so it
  can produce a dictionary at no extra hash cost. CSV and JSON would need the
  same, and a column with no repeats should stay plain.
- Breaking: none in the signatures. Code that matches on the representation
  (none outside `array.rs`) would need changing. The internal enum is new.

### C. Leave it as it is

Keep `Arc<str>` per row. Q5 stays about 1.5× DataFusion, and Q3 keeps its
current gather cost. No work, no risk.

## Recommendation

Prototype B first, behind a measurement, before committing to A.

1. Build the dictionary inside `StrArray` in the Parquet reader only, and check
   that all tests pass with no API change.
2. Add the per-distinct-value `LIKE` path in `RowTest` for dictionary columns.
3. Measure Q5 and Q3 against the numbers above, on the same machine.

B is the only option that is non-breaking and that also cuts Q5's per-row work.
A is worth revisiting only if high-cardinality string columns become a
bottleneck, because it's the only option that helps those.

## Decisions needed

1. Is an internal dictionary representation in `StrArray` acceptable? It adds a
   second code path to every string operation.
2. Should the dictionary be built only for low-cardinality columns, with a
   cut-off? If so, what is the cut-off, and is it set at build time or per batch?
3. Is A (a breaking change to `value_ref` and the constructors) acceptable, or
   should the crate keep `Arc<str>` in its public API?
4. Should the join's gather get its own measurement before any of this, since
   it is Q3's cost?

## What would change my mind

- B gives under 10% on Q5 once the per-value `LIKE` is in place. Then the layout
  is not the bottleneck, and option C is right.
- Gathers on real high-cardinality data dominate. Then A is worth the break.

## Measured (2026-10-17): decision is option C

A loop-level probe, 1M rows, 8 regions, `LIKE 'c%'` style prefix test, summing
a quantity column, Windows, 60 runs:

| variant | min ms | median ms |
|---|---|---|
| per-row string test (shared `Arc` per row) | 0.98 | 1.14–1.28 |
| dictionary: test each distinct value, then a `u32` key lookup per row | 0.52 | 0.59–0.67 |

The probe saves about 0.5 ms per query. Q5 takes about 4.1 ms in the engine, so
the saving is at most about 12%, and the engine's gain is likely smaller, because
the engine's per-row work includes more than this loop. That is at the threshold
set above, so the layout change is not justified now.

Decision, following the recommendation: option C. Keep `Arc<str>`. Option B
would be the next step only if a larger string workload shows a bigger gain. The
probe was not kept.
