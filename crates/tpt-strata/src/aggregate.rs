//! Grouped aggregation over typed columns.
//!
//! Groups are found with borrowed [`KeyPart`]s, and each aggregate keeps its
//! state in vectors indexed by group id. Nothing is boxed per row. Output
//! columns are built directly as typed arrays, so MIN and MAX return the value
//! at the row they remember instead of a copied scalar.
//!
//! Semantics: SUM and AVG skip NULL and return NULL for a group with no
//! non-null input. COUNT(col) skips NULL and COUNT(*) counts every row. MIN and
//! MAX skip NULL. A global aggregate over zero rows yields one row.

use std::sync::Arc;

use crate::array::{ArrayRef, F64Array, I64Array};
use crate::batch::{Batch, BatchRef};
use crate::engine::{gather_opt, names, push_line, Comparison, Condition, PhysicalPlan, Predicate};
use crate::error::QueryError;
use crate::key::{key_at, FastMap, KeyPart};
use crate::schema::{DataType, Field, Schema};
use crate::types::Scalar;

/// Whether a data type is orderable for `MIN`/`MAX` (and `ORDER BY`).
///
/// Every native v1 type is orderable, so this is a no-op guard today. It
/// exists so a future non-orderable `DataType` fails at plan-build time with a
/// named diagnostic instead of at runtime.
fn is_orderable(dt: DataType) -> bool {
    matches!(
        dt,
        DataType::Boolean
            | DataType::Int32
            | DataType::Int64
            | DataType::Float64
            | DataType::Utf8
            | DataType::Date
            | DataType::Timestamp
    )
}

/// Whether a data type is numeric for `SUM`/`AVG`.
fn is_numeric(dt: DataType) -> bool {
    matches!(dt, DataType::Int32 | DataType::Int64 | DataType::Float64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFunc {
    Sum,
    Count,
    Min,
    Max,
    Avg,
}

impl std::fmt::Display for AggFunc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AggFunc::Sum => write!(f, "SUM"),
            AggFunc::Count => write!(f, "COUNT"),
            AggFunc::Min => write!(f, "MIN"),
            AggFunc::Max => write!(f, "MAX"),
            AggFunc::Avg => write!(f, "AVG"),
        }
    }
}

/// An aggregate function applied to one input column.
#[derive(Debug, Clone)]
pub struct Aggregate {
    pub func: AggFunc,
    pub column: usize,
    pub out_name: String,
    /// Whether `COUNT(*)` semantics apply: count every row, including nulls.
    /// `false` means `COUNT(col)`, which skips null values.
    pub count_star: bool,
}

/// Resolve the output [`DataType`] of an aggregate over an input schema.
fn aggregate_output_type(input_schema: &Schema, agg: &Aggregate) -> DataType {
    let input = input_schema.field(agg.column).map(|f| f.data_type);
    match agg.func {
        AggFunc::Count => DataType::Int64,
        AggFunc::Min | AggFunc::Max => input.unwrap_or(DataType::Float64),
        AggFunc::Sum => match input {
            Some(DataType::Int32 | DataType::Int64) => DataType::Int64,
            _ => DataType::Float64,
        },
        AggFunc::Avg => DataType::Float64,
    }
}

/// State for one aggregate, across all groups. Each vector is indexed by group id.
enum AggState {
    Sum {
        col: usize,
        label: String,
        as_float: bool,
        int: Vec<i128>,
        float: Vec<f64>,
        saw: Vec<bool>,
    },
    Count {
        col: usize,
        count_star: bool,
        n: Vec<i64>,
    },
    /// Position `(batch, row)` of the current best value of each group.
    MinMax {
        col: usize,
        max: bool,
        best: Vec<Option<(usize, usize)>>,
    },
    Avg {
        col: usize,
        int: Vec<i128>,
        float: Vec<f64>,
        n: Vec<i64>,
    },
}

impl AggState {
    fn new(agg: &Aggregate, input: DataType) -> Self {
        let col = agg.column;
        match agg.func {
            AggFunc::Sum => AggState::Sum {
                col,
                label: agg.out_name.clone(),
                as_float: input == DataType::Float64,
                int: Vec::new(),
                float: Vec::new(),
                saw: Vec::new(),
            },
            AggFunc::Count => AggState::Count {
                col,
                count_star: agg.count_star,
                n: Vec::new(),
            },
            AggFunc::Min => AggState::MinMax {
                col,
                max: false,
                best: Vec::new(),
            },
            AggFunc::Max => AggState::MinMax {
                col,
                max: true,
                best: Vec::new(),
            },
            AggFunc::Avg => AggState::Avg {
                col,
                int: Vec::new(),
                float: Vec::new(),
                n: Vec::new(),
            },
        }
    }

    /// Reserve room for `n` more groups in each per-group vector, so they do not
    /// reallocate as groups are created.
    fn reserve(&mut self, n: usize) {
        match self {
            AggState::Sum {
                int,
                float,
                saw,
                as_float,
                ..
            } => {
                int.reserve(n);
                if *as_float {
                    float.reserve(n);
                }
                saw.reserve(n);
            }
            AggState::Count { n: counts, .. } => counts.reserve(n),
            AggState::MinMax { best, .. } => best.reserve(n),
            AggState::Avg {
                int,
                float,
                n: counts,
                ..
            } => {
                int.reserve(n);
                float.reserve(n);
                counts.reserve(n);
            }
        }
    }

    /// Make room for one more group, initialised to its empty state.
    fn push_group(&mut self) {
        match self {
            AggState::Sum {
                int,
                float,
                saw,
                as_float,
                ..
            } => {
                int.push(0);
                // Only a float sum reads the float total, so an integer sum keeps none.
                if *as_float {
                    float.push(0.0);
                }
                saw.push(false);
            }
            AggState::Count { n, .. } => n.push(0),
            AggState::MinMax { best, .. } => best.push(None),
            AggState::Avg { int, float, n, .. } => {
                int.push(0);
                float.push(0.0);
                n.push(0);
            }
        }
    }

    /// Fold the value at `pos` = (batch, row) into group `gid`.
    /// Fold one batch into the state. `gids[row]` is the group of each row of
    /// batch `bi`. The column type is matched once per batch, and each loop is
    /// typed, so there is no per-row dispatch on state or column.
    fn update_batch(&mut self, batches: &[BatchRef], bi: usize, start: usize, gids: &[usize]) {
        let batch = &batches[bi];
        match self {
            AggState::Sum {
                col,
                int,
                float,
                saw,
                ..
            } => match &*batch.columns[*col] {
                // An integer column's SUM is read from `int` alone (see `finish`),
                // so the float total is kept only for float columns.
                ArrayRef::Int32(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.value(row) {
                            int[gid] += i128::from(v);
                            saw[gid] = true;
                        }
                    }
                }
                ArrayRef::Int64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.value(row) {
                            int[gid] += i128::from(v);
                            saw[gid] = true;
                        }
                    }
                }
                ArrayRef::Float64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.value(row) {
                            float[gid] += v;
                            saw[gid] = true;
                        }
                    }
                }
                _ => {}
            },
            AggState::Count { col, count_star, n } => {
                let column = &batch.columns[*col];
                for (k, &gid) in gids.iter().enumerate() {
                    let row = start + k;
                    if *count_star || !column.is_null(row) {
                        n[gid] += 1;
                    }
                }
            }
            AggState::MinMax { col, max, best } => {
                let value = &batch.columns[*col];
                for (k, &gid) in gids.iter().enumerate() {
                    let row = start + k;
                    if value.is_null(row) {
                        continue;
                    }
                    let pos = (bi, row);
                    let replace = match best[gid] {
                        None => true,
                        Some((cb, cr)) => {
                            let current = key_at(&batches[cb].columns[*col], cr);
                            let ord = key_at(value, row).cmp(&current);
                            if *max {
                                ord == std::cmp::Ordering::Greater
                            } else {
                                ord == std::cmp::Ordering::Less
                            }
                        }
                    };
                    if replace {
                        best[gid] = Some(pos);
                    }
                }
            }
            AggState::Avg { col, int, float, n } => match &*batch.columns[*col] {
                ArrayRef::Int32(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.value(row) {
                            int[gid] += i128::from(v);
                            n[gid] += 1;
                        }
                    }
                }
                ArrayRef::Int64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.value(row) {
                            int[gid] += i128::from(v);
                            n[gid] += 1;
                        }
                    }
                }
                ArrayRef::Float64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.value(row) {
                            float[gid] += v;
                            n[gid] += 1;
                        }
                    }
                }
                _ => {}
            },
        }
    }

    /// Fold another partial state into this one. `map[g]` is the group in
    /// `self` for group `g` of `other`. Min and max keep the earlier value on
    /// a tie, as a sequential pass does.
    fn merge(&mut self, other: &AggState, map: &[usize], batches: &[BatchRef]) {
        match (self, other) {
            (
                AggState::Sum {
                    int, float, saw, ..
                },
                AggState::Sum {
                    int: oi,
                    float: of,
                    saw: os,
                    ..
                },
            ) => {
                // An integer sum has no float total to merge.
                let has_float = !float.is_empty();
                for (lg, &gg) in map.iter().enumerate() {
                    int[gg] += oi[lg];
                    if has_float {
                        float[gg] += of[lg];
                    }
                    saw[gg] |= os[lg];
                }
            }
            (AggState::Count { n, .. }, AggState::Count { n: on, .. }) => {
                for (lg, &gg) in map.iter().enumerate() {
                    n[gg] += on[lg];
                }
            }
            (
                AggState::MinMax { col, max, best },
                AggState::MinMax {
                    best: other_best, ..
                },
            ) => {
                for (lg, &gg) in map.iter().enumerate() {
                    let Some((pb, pr)) = other_best[lg] else {
                        continue;
                    };
                    let replace = match best[gg] {
                        None => true,
                        Some((cb, cr)) => {
                            let current = key_at(&batches[cb].columns[*col], cr);
                            let candidate = key_at(&batches[pb].columns[*col], pr);
                            let ord = candidate.cmp(&current);
                            if *max {
                                ord == std::cmp::Ordering::Greater
                            } else {
                                ord == std::cmp::Ordering::Less
                            }
                        }
                    };
                    if replace {
                        best[gg] = Some((pb, pr));
                    }
                }
            }
            (
                AggState::Avg { int, float, n, .. },
                AggState::Avg {
                    int: oi,
                    float: of,
                    n: on,
                    ..
                },
            ) => {
                for (lg, &gg) in map.iter().enumerate() {
                    int[gg] += oi[lg];
                    float[gg] += of[lg];
                    n[gg] += on[lg];
                }
            }
            // Partials come from the same aggregates, so the kinds match.
            _ => unreachable!("partial states come from the same aggregates"),
        }
    }

    /// Build the output column for this aggregate, one value per group.
    /// The output column for the groups in `keep`, in that order.
    fn finish(
        &self,
        dt: DataType,
        batches: &[BatchRef],
        keep: &[usize],
    ) -> Result<ArrayRef, QueryError> {
        match self {
            AggState::Sum {
                label,
                as_float,
                int,
                float,
                saw,
                ..
            } => {
                if *as_float {
                    let vals: Vec<Option<f64>> =
                        keep.iter().map(|&g| saw[g].then_some(float[g])).collect();
                    Ok(ArrayRef::Float64(F64Array::from_nulls(vals)))
                } else {
                    let vals = keep
                        .iter()
                        .map(|&g| {
                            if !saw[g] {
                                return Ok(None);
                            }
                            i64::try_from(int[g]).map(Some).map_err(|_| {
                                QueryError::UnsupportedOperation {
                                    message: format!(
                                        "SUM overflowed i64 in '{label}'; narrow the input or sum a float column"
                                    ),
                                }
                            })
                        })
                        .collect::<Result<Vec<Option<i64>>, _>>()?;
                    Ok(ArrayRef::Int64(I64Array::from_nulls(vals)))
                }
            }
            AggState::Count { n, .. } => Ok(ArrayRef::Int64(I64Array::new(
                keep.iter().map(|&g| n[g]).collect(),
            ))),
            AggState::MinMax { col, best, .. } => {
                Ok(gather_opt(batches, *col, keep.iter().map(|&g| best[g]), dt))
            }
            AggState::Avg { int, float, n, .. } => {
                let vals: Vec<Option<f64>> = keep
                    .iter()
                    .map(|&g| (n[g] > 0).then(|| (int[g] as f64 + float[g]) / n[g] as f64))
                    .collect();
                Ok(ArrayRef::Float64(F64Array::from_nulls(vals)))
            }
        }
    }
}

pub struct AggregateExec {
    input: Arc<dyn PhysicalPlan>,
    group_by: Vec<usize>,
    aggregates: Vec<Aggregate>,
    schema: Arc<Schema>,
    /// HAVING over the output columns (group keys, then aggregates). Groups it
    /// rejects are never built into output.
    having: Option<Condition>,
}

impl AggregateExec {
    pub fn new(
        input: Arc<dyn PhysicalPlan>,
        group_by: Vec<usize>,
        aggregates: Vec<Aggregate>,
    ) -> Result<Self, QueryError> {
        let input_schema = input.schema();
        let mut fields = Vec::new();
        for &idx in &group_by {
            let f = input_schema
                .field(idx)
                .ok_or_else(|| QueryError::MissingColumn {
                    column: format!("index {idx}"),
                    available: names(input_schema),
                })?;
            fields.push(Field::new(f.name.clone(), f.data_type, f.nullable));
        }
        for agg in &aggregates {
            let f = input_schema
                .field(agg.column)
                .ok_or_else(|| QueryError::MissingColumn {
                    column: format!("index {}", agg.column),
                    available: names(input_schema),
                })?;
            if matches!(agg.func, AggFunc::Sum | AggFunc::Avg) && !is_numeric(f.data_type) {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "aggregate '{}' requires a numeric column; column '{}' has type {}",
                        agg.out_name, f.name, f.data_type
                    ),
                });
            }
            if matches!(agg.func, AggFunc::Min | AggFunc::Max) && !is_orderable(f.data_type) {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "aggregate '{}' requires an orderable column; column '{}' has type {}",
                        agg.out_name, f.name, f.data_type
                    ),
                });
            }
            fields.push(Field::new(
                agg.out_name.clone(),
                aggregate_output_type(input_schema, agg),
                true,
            ));
        }
        let schema = Arc::new(Schema::new(fields));
        Ok(Self {
            input,
            group_by,
            aggregates,
            schema,
            having: None,
        })
    }

    /// Keep only the groups for which `condition` holds. Its column indices
    /// refer to this operator's output schema.
    pub fn with_having(mut self, condition: Condition) -> Self {
        self.having = Some(condition);
        self
    }

    /// The groups (by id, in order) for which the HAVING condition holds.
    ///
    /// Only the output columns the condition reads are built for every group,
    /// and they are read through a probe batch of just those columns. The rest
    /// of the output is built afterwards, for the kept groups only.
    fn kept_groups(
        &self,
        condition: &Condition,
        first: &[Option<(usize, usize)>],
        states: &[AggState],
        batches: &[BatchRef],
    ) -> Result<Vec<usize>, QueryError> {
        let groups = first.len();
        if let Condition::Cmp(p) = condition {
            if let Some(keep) = self.having_on_sum(p, groups, states) {
                return Ok(keep);
            }
        }
        let all: Vec<usize> = (0..groups).collect();
        let mut referenced = Vec::new();
        condition.columns(&mut referenced);
        referenced.sort_unstable();
        referenced.dedup();

        let key_count = self.group_by.len();
        let mut columns = Vec::with_capacity(referenced.len());
        for &j in &referenced {
            let dt = self.schema.fields[j].data_type;
            let column = if j < key_count {
                gather_opt(batches, self.group_by[j], first.iter().copied(), dt)
            } else {
                states[j - key_count].finish(dt, batches, &all)?
            };
            columns.push(column);
        }
        let probe_index = |j: usize| referenced.binary_search(&j).unwrap_or(0);
        let condition = condition.remap_columns(&probe_index);
        let probe = Batch::new_unchecked(self.schema.clone(), columns);
        Ok(select_condition(&condition, &probe, None, groups))
    }
}

impl AggregateExec {
    /// HAVING on an integer `SUM` against an integer literal, on the dense
    /// accumulators. `order` lists the groups in first-seen order and the result
    /// keeps that order. `None` when the condition is another shape.
    fn dense_having(
        &self,
        p: &Predicate,
        kinds: &[DenseKind],
        order: &[usize],
        int: &[i64],
        saw: &[bool],
        naggs: usize,
    ) -> Option<Vec<usize>> {
        let key_count = self.group_by.len();
        let (Scalar::I64(lit), true) = (&p.value, p.column >= key_count) else {
            return None;
        };
        let a = p.column - key_count;
        if !matches!(kinds.get(a), Some(DenseKind::Sum { .. })) {
            return None;
        }
        let lit = *lit;
        let mut keep = Vec::new();
        for &g in order {
            let idx = g * naggs + a;
            if !saw[idx] {
                continue;
            }
            let v = int[idx];
            let hit = match p.op {
                Comparison::Eq => v == lit,
                Comparison::NotEq => v != lit,
                Comparison::Gt => v > lit,
                Comparison::Gte => v >= lit,
                Comparison::Lt => v < lit,
                Comparison::Lte => v <= lit,
                _ => return None,
            };
            if hit {
                keep.push(g);
            }
        }
        Some(keep)
    }

    /// HAVING on an integer `SUM` against an integer literal, decided from the
    /// accumulators directly, so no output column is built for every group.
    /// Returns `None` when the condition is another shape, or when a sum is
    /// outside `i64`: the general path reports that overflow as an error.
    fn having_on_sum(
        &self,
        p: &Predicate,
        groups: usize,
        states: &[AggState],
    ) -> Option<Vec<usize>> {
        let key_count = self.group_by.len();
        let (Scalar::I64(lit), true) = (&p.value, p.column >= key_count) else {
            return None;
        };
        let AggState::Sum {
            as_float: false,
            int,
            saw,
            ..
        } = &states[p.column - key_count]
        else {
            return None;
        };
        let lit = i128::from(*lit);
        let mut keep = Vec::new();
        for g in 0..groups {
            if !saw[g] {
                continue;
            }
            let v = int[g];
            if i64::try_from(v).is_err() {
                return None;
            }
            let hit = match p.op {
                Comparison::Eq => v == lit,
                Comparison::NotEq => v != lit,
                Comparison::Gt => v > lit,
                Comparison::Gte => v >= lit,
                Comparison::Lt => v < lit,
                Comparison::Lte => v <= lit,
                _ => return None,
            };
            if hit {
                keep.push(g);
            }
        }
        Some(keep)
    }
}

/// The rows of `batch` (or of `rows`, when given) that satisfy `condition`, in
/// order. A comparison and an `AND` of comparisons use the predicate's typed
/// path, which reads the column directly. Other conditions check each row.
fn select_condition(
    condition: &Condition,
    batch: &Batch,
    rows: Option<&[usize]>,
    n: usize,
) -> Vec<usize> {
    match condition {
        Condition::Cmp(p) => p.select_from(batch, rows),
        Condition::And(parts) => {
            let mut current: Option<Vec<usize>> = None;
            for part in parts {
                let next = select_condition(part, batch, current.as_deref().or(rows), n);
                current = Some(next);
            }
            current.unwrap_or_else(|| match rows {
                Some(r) => r.to_vec(),
                None => (0..n).collect(),
            })
        }
        _ => match rows {
            Some(r) => r
                .iter()
                .copied()
                .filter(|&g| condition.matches(batch, g))
                .collect(),
            None => (0..n).filter(|&g| condition.matches(batch, g)).collect(),
        },
    }
}

/// The groups and states an aggregate produces: first rows, then states.
type AggOutput = (Vec<Option<(usize, usize)>>, Vec<AggState>);

/// Accumulators for a dense single-key aggregate: groups in first-seen order,
/// with `naggs` values per group in `int`, `saw` and `count`. The records stay
/// narrow, because the merge reads every group of every worker.
#[derive(Default)]
struct DenseGroups {
    /// The key of each group. The NULL group's entry is not used.
    keys: Vec<i64>,
    /// The index of the NULL group, or `NO_GROUP`.
    null_group: usize,
    /// The first row of each group, packed with `pack`.
    first: Vec<u64>,
    int: Vec<i64>,
    saw: Vec<bool>,
    /// Row counts per group. Kept only when a COUNT needs them (`counts`).
    count: Vec<i64>,
    counts: bool,
    /// Set when an integer sum leaves `i64`. The general path then reports it.
    overflow: bool,
}

impl DenseGroups {
    fn new(counts: bool) -> Self {
        DenseGroups {
            null_group: NO_GROUP,
            counts,
            ..Default::default()
        }
    }

    /// Add a group, with its accumulators at zero, and return its index.
    fn push(&mut self, key: i64, first: u64, naggs: usize) -> usize {
        self.keys.push(key);
        self.first.push(first);
        for _ in 0..naggs {
            self.int.push(0);
            self.saw.push(false);
            if self.counts {
                self.count.push(0);
            }
        }
        self.first.len() - 1
    }

    /// Add the NULL group, with its accumulators at zero, and return its index.
    fn push_null(&mut self, first: u64, naggs: usize) -> usize {
        let g = self.push(0, first, naggs);
        self.null_group = g;
        g
    }
}

/// `(batch, row)` packed into one word. Comparing packed words compares the
/// pairs, so the earliest first row is the smallest word.
fn pack(batch: usize, row: usize) -> u64 {
    ((batch as u64) << 32) | row as u64
}

fn unpack(word: u64) -> (usize, usize) {
    ((word >> 32) as usize, (word & 0xFFFF_FFFF) as usize)
}

/// The aggregates a dense single-key pass can compute: integer sums and counts.
#[derive(Clone, Copy)]
enum DenseKind {
    Sum { col: usize },
    Count { col: usize, count_star: bool },
}

/// Add the integer values of one aggregate into the groups of one segment.
/// `value(k)` is the value of row `k` of the segment, `None` for NULL.
fn sum_into(
    groups: &mut DenseGroups,
    a: usize,
    naggs: usize,
    gids: &[usize],
    value: impl Fn(usize) -> Option<i64>,
) {
    for (k, &g) in gids.iter().enumerate() {
        if let Some(v) = value(k) {
            let idx = g * naggs + a;
            match groups.int[idx].checked_add(v) {
                Some(sum) => groups.int[idx] = sum,
                None => groups.overflow = true,
            }
            groups.saw[idx] = true;
        }
    }
}

/// Aggregate one worker's segments. The key range covered is taken from these
/// segments alone, so the slot table is small even when the whole input's key
/// range is large. Groups are created in first-seen order.
fn dense_worker(
    batches: &[BatchRef],
    segs: &[Segment],
    key: usize,
    kinds: &[DenseKind],
) -> DenseGroups {
    let naggs = kinds.len();
    let (mut lo, mut hi) = (i64::MAX, i64::MIN);
    for &(bi, start, end) in segs {
        if let ArrayRef::Int64(a) = &*batches[bi].columns[key] {
            for row in start..end {
                if let Some(v) = a.value(row) {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
        }
    }
    let span = if lo > hi {
        0
    } else {
        (i128::from(hi) - i128::from(lo) + 1) as usize
    };
    let mut slot = vec![NO_GROUP; span];
    let has_count = kinds.iter().any(|k| matches!(k, DenseKind::Count { .. }));
    let mut groups = DenseGroups::new(has_count);
    // A worker creates at most one group per row and at most one per key in
    // its range. Reserving that bound up front avoids repeated regrowth of the
    // five per-group vectors when every key is a group.
    let rows: usize = segs.iter().map(|&(_, start, end)| end - start).sum();
    let groups_bound = span.min(rows);
    groups.keys.reserve(groups_bound);
    groups.first.reserve(groups_bound);
    groups.int.reserve(groups_bound * naggs);
    groups.saw.reserve(groups_bound * naggs);
    if has_count {
        groups.count.reserve(groups_bound * naggs);
    }
    let mut gids: Vec<usize> = Vec::with_capacity(SEGMENT_ROWS);
    for &(bi, start, end) in segs {
        let batch = &batches[bi];
        gids.clear();
        // The key column's type is fixed for the segment, so it is matched once,
        // not once per row.
        let keys = match &*batch.columns[key] {
            ArrayRef::Int64(a) => Some(a),
            _ => None,
        };
        // One group id per row, written in a single pass over the segment.
        gids.extend(
            (start..end).map(|row| match keys.and_then(|a| a.value(row)) {
                Some(v) => {
                    let off = (v - lo) as usize;
                    if slot[off] == NO_GROUP {
                        slot[off] = groups.push(v, pack(bi, row), naggs);
                    }
                    slot[off]
                }
                None => {
                    if groups.null_group == NO_GROUP {
                        groups.push_null(pack(bi, row), naggs);
                    }
                    groups.null_group
                }
            }),
        );
        for (a, kind) in kinds.iter().enumerate() {
            match *kind {
                DenseKind::Sum { col } => match &*batch.columns[col] {
                    ArrayRef::Int32(c) => {
                        sum_into(&mut groups, a, naggs, &gids, |k| {
                            c.value(start + k).map(i64::from)
                        });
                    }
                    ArrayRef::Int64(c) => {
                        sum_into(&mut groups, a, naggs, &gids, |k| c.value(start + k));
                    }
                    _ => {}
                },
                DenseKind::Count { col, count_star } => {
                    let column = &batch.columns[col];
                    for (k, &g) in gids.iter().enumerate() {
                        if count_star || !column.is_null(start + k) {
                            groups.count[g * naggs + a] += 1;
                        }
                    }
                }
            }
        }
    }
    groups
}

/// Run `dense_worker` over the segments: on one thread, or split into
/// contiguous runs across threads when the `parallel` feature is on.
fn dense_workers(
    batches: &[BatchRef],
    segs: &[Segment],
    key: usize,
    kinds: &[DenseKind],
) -> Vec<DenseGroups> {
    #[cfg(feature = "parallel")]
    {
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(segs.len());
        if threads > 1 {
            let per = segs.len().div_ceil(threads);
            return std::thread::scope(|scope| {
                let workers: Vec<_> = segs
                    .chunks(per)
                    .map(|chunk| scope.spawn(move || dense_worker(batches, chunk, key, kinds)))
                    .collect();
                workers
                    .into_iter()
                    .map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                    .collect()
            });
        }
    }
    vec![dense_worker(batches, segs, key, kinds)]
}

/// Rows per unit of aggregation work. Segments are the unit that threads
/// share, so a large batch splits across them.
pub(crate) const SEGMENT_ROWS: usize = 64 * 1024;

/// `(batch, start, end)`: rows `start..end` of batch `batch`, in input order.
pub(crate) type Segment = (usize, usize, usize);

/// Slots in the string-address cache used by single-key GROUP BY. A power of two.
const PTR_CACHE: usize = 256;

/// Marks an unused dense slot.
const NO_GROUP: usize = usize::MAX;

/// The smallest key and the number of values in its range, when column `col`
/// is `Int64` and its keys fit a slot each: the range is at most four slots per
/// input row (plus a fixed allowance for small inputs). `None` otherwise, or when
/// every key is NULL.
fn int_key_range(batches: &[BatchRef], col: usize) -> Option<(i64, usize)> {
    let rows: usize = batches.iter().map(|b| b.row_count()).sum();
    let (mut lo, mut hi) = (i64::MAX, i64::MIN);
    for b in batches {
        match &*b.columns[col] {
            ArrayRef::Int64(a) => {
                for row in 0..b.row_count() {
                    if let Some(v) = a.value(row) {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
            }
            _ => return None,
        }
    }
    if lo > hi {
        return None;
    }
    let range = i128::from(hi) - i128::from(lo) + 1;
    if range > (1 << 22) || range > 4 * rows as i128 + 1024 {
        return None;
    }
    Some((lo, range as usize))
}

pub(crate) fn segments(batches: &[BatchRef]) -> Vec<Segment> {
    let mut segs = Vec::new();
    for (bi, batch) in batches.iter().enumerate() {
        let rows = batch.row_count();
        let mut start = 0;
        while start < rows {
            let end = (start + SEGMENT_ROWS).min(rows);
            segs.push((bi, start, end));
            start = end;
        }
    }
    segs
}

/// Groups in first-seen order, with one state per aggregate, for some
/// segments. Partials over contiguous segments merge, in order, into the same
/// result as one sequential pass.
struct Partial {
    first: Vec<Option<(usize, usize)>>,
    states: Vec<AggState>,
}

impl AggregateExec {
    fn new_states(&self) -> Vec<AggState> {
        let input = self.input.schema();
        self.aggregates
            .iter()
            .map(|a| {
                let dt = input
                    .field(a.column)
                    .map_or(DataType::Float64, |f| f.data_type);
                AggState::new(a, dt)
            })
            .collect()
    }

    /// Aggregate the given segments on the calling thread.
    fn run_segments(
        &self,
        batches: &[BatchRef],
        segs: &[Segment],
        dense: Option<(i64, usize)>,
    ) -> Partial {
        let global = self.group_by.is_empty();
        let mut states = self.new_states();
        // Representative row of each group, in first-seen order. A global
        // aggregate has one group, which has no representative row when the
        // input is empty.
        let mut first: Vec<Option<(usize, usize)>> = Vec::new();
        let mut index: FastMap<Vec<KeyPart<'_>>, usize> = FastMap::default();
        // With one group column, index on the key itself: no per-row Vec.
        let mut single_index: FastMap<KeyPart<'_>, usize> = FastMap::default();
        if global {
            first.push(None);
            for s in &mut states {
                s.push_group();
            }
        }

        let mut ptr_cache: [Option<(usize, usize)>; PTR_CACHE] = [None; PTR_CACHE];
        let mut scratch: Vec<KeyPart<'_>> = Vec::with_capacity(self.group_by.len());
        let mut gids: Vec<usize> = Vec::new();
        // Direct slots for a dense `Int64` key: the slot is the key's offset from
        // the smallest key, so no hashing. The NULL group has its own slot.
        let mut dense_slots: Vec<usize> =
            dense.map_or(Vec::new(), |(_, range)| vec![NO_GROUP; range]);
        let mut null_group = NO_GROUP;
        if let Some((_, range)) = dense {
            // A group needs at least one row, so the rows here bound the groups.
            let rows: usize = segs.iter().map(|&(_, start, end)| end - start).sum();
            let expected = range.min(rows);
            for s in &mut states {
                s.reserve(expected);
            }
            first.reserve(expected);
        }
        for &(bi, start, end) in segs {
            let batch = &batches[bi];
            gids.clear();
            for row in start..end {
                let gid = if global {
                    0
                } else if let (Some((min, _)), [c]) = (dense, self.group_by.as_slice()) {
                    let value = match &*batch.columns[*c] {
                        ArrayRef::Int64(a) => a.value(row),
                        _ => None,
                    };
                    match value {
                        Some(v) => {
                            let off = (v - min) as usize;
                            if dense_slots[off] == NO_GROUP {
                                dense_slots[off] = first.len();
                                first.push(Some((bi, row)));
                                for s in &mut states {
                                    s.push_group();
                                }
                            }
                            dense_slots[off]
                        }
                        None => {
                            if null_group == NO_GROUP {
                                null_group = first.len();
                                first.push(Some((bi, row)));
                                for s in &mut states {
                                    s.push_group();
                                }
                            }
                            null_group
                        }
                    }
                } else if let [c] = self.group_by.as_slice() {
                    // Shared strings: a direct-mapped cache keyed by the string's
                    // allocation answers without hashing its bytes. Batches are
                    // alive for the whole run, so equal addresses mean the same
                    // string. A miss falls through to the content-keyed map.
                    let address = match &*batch.columns[*c] {
                        ArrayRef::Utf8(a) => a
                            .value_ref(row)
                            .map(|v| Arc::as_ptr(v) as *const u8 as usize),
                        _ => None,
                    };
                    let slot = address.map_or(0, |p| (p >> 4) & (PTR_CACHE - 1));
                    if let (Some(p), Some((cp, cg))) = (address, ptr_cache[slot]) {
                        if cp == p {
                            gids.push(cg);
                            continue;
                        }
                    }
                    let key = key_at(&batch.columns[*c], row);
                    let g = match single_index.get(&key) {
                        Some(&g) => g,
                        None => {
                            let g = first.len();
                            first.push(Some((bi, row)));
                            single_index.insert(key, g);
                            for s in &mut states {
                                s.push_group();
                            }
                            g
                        }
                    };
                    if let Some(p) = address {
                        ptr_cache[slot] = Some((p, g));
                    }
                    g
                } else {
                    scratch.clear();
                    scratch.extend(
                        self.group_by
                            .iter()
                            .map(|&c| key_at(&batch.columns[c], row)),
                    );
                    match index.get(scratch.as_slice()) {
                        Some(&g) => g,
                        None => {
                            let g = first.len();
                            first.push(Some((bi, row)));
                            index.insert(scratch.clone(), g);
                            for s in &mut states {
                                s.push_group();
                            }
                            g
                        }
                    }
                };
                gids.push(gid);
            }
            for s in &mut states {
                s.update_batch(batches, bi, start, &gids);
            }
        }
        Partial { first, states }
    }

    /// Run the segments: on one thread, or split across threads when the
    /// `parallel` feature is on. Partials come back in segment order.
    fn run_partials(
        &self,
        batches: &[BatchRef],
        segs: &[Segment],
        dense: Option<(i64, usize)>,
    ) -> Vec<Partial> {
        #[cfg(feature = "parallel")]
        {
            let threads = std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .min(segs.len());
            if threads > 1 {
                let per = segs.len().div_ceil(threads);
                return std::thread::scope(|scope| {
                    let workers: Vec<_> = segs
                        .chunks(per)
                        .map(|chunk| scope.spawn(move || self.run_segments(batches, chunk, dense)))
                        .collect();
                    workers
                        .into_iter()
                        .map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                        .collect()
                });
            }
        }
        vec![self.run_segments(batches, segs, dense)]
    }

    /// The dense path for a single integer key, when every aggregate is an
    /// integer `SUM` or a `COUNT`. Returns the same groups, states and order as
    /// the general path, or `None` when the aggregates do not qualify, or when
    /// an integer sum leaves `i64` (the general path then reports it).
    fn dense_aggregate(
        &self,
        batches: &[BatchRef],
        segs: &[Segment],
        key: usize,
        min: i64,
        range: usize,
    ) -> Option<AggOutput> {
        let input = self.input.schema();
        let kinds: Vec<DenseKind> = self
            .aggregates
            .iter()
            .map(|a| match a.func {
                AggFunc::Sum => match input.field(a.column).map(|f| f.data_type) {
                    Some(DataType::Int32 | DataType::Int64) => {
                        Some(DenseKind::Sum { col: a.column })
                    }
                    _ => None,
                },
                AggFunc::Count => Some(DenseKind::Count {
                    col: a.column,
                    count_star: a.count_star,
                }),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let naggs = kinds.len();
        let has_count = kinds.iter().any(|k| matches!(k, DenseKind::Count { .. }));

        // Packed first rows need the batch and the row each to fit in 32 bits.
        let mut offsets = Vec::with_capacity(batches.len());
        let mut total_rows = 0;
        for batch in batches {
            offsets.push(total_rows);
            total_rows += batch.row_count();
        }
        if total_rows > u32::MAX as usize || batches.len() > u32::MAX as usize {
            return None;
        }

        // Merge the workers in order. Each group keeps the earliest first row
        // seen across workers, which is the row the general path would keep.
        let workers_out = dense_workers(batches, segs, key, &kinds);
        // Each worker's groups are merged in, so their total bounds the output.
        // The vectors are sized for that bound and filled by index.
        let cap: usize = workers_out.iter().map(|w| w.first.len()).sum();
        let mut first = vec![u64::MAX; cap];
        let mut int = vec![0i64; cap * naggs];
        let mut saw = vec![false; cap * naggs];
        let mut count = vec![0i64; if has_count { cap * naggs } else { 0 }];
        let mut groups = 0;
        let mut slot = vec![NO_GROUP; range];
        let mut null_group = NO_GROUP;
        let mut overflow = false;
        for worker in workers_out {
            overflow |= worker.overflow;
            for (lg, &key_value) in worker.keys.iter().enumerate() {
                let g = if lg == worker.null_group {
                    if null_group == NO_GROUP {
                        null_group = groups;
                        groups += 1;
                    }
                    null_group
                } else {
                    let off = (key_value - min) as usize;
                    if slot[off] == NO_GROUP {
                        slot[off] = groups;
                        groups += 1;
                    }
                    slot[off]
                };
                first[g] = first[g].min(worker.first[lg]);
                let (src, dst) = (lg * naggs, g * naggs);
                for a in 0..naggs {
                    match int[dst + a].checked_add(worker.int[src + a]) {
                        Some(sum) => int[dst + a] = sum,
                        None => overflow = true,
                    }
                    saw[dst + a] |= worker.saw[src + a];
                    if has_count {
                        count[dst + a] += worker.count[src + a];
                    }
                }
            }
        }
        if overflow {
            return None;
        }
        first.truncate(groups);
        int.truncate(groups * naggs);
        saw.truncate(groups * naggs);
        count.truncate(groups * naggs);

        // First-seen order: each group's first row is unique, so the groups
        // sorted by their packed first row (batch, then row) are in that order.
        // With few groups, sorting them is much cheaper than a table over every
        // input row, so the table is used only when groups are a large share.
        let order: Vec<usize> = if first.len().saturating_mul(16) < total_rows {
            let mut order: Vec<usize> = (0..first.len()).collect();
            order.sort_unstable_by_key(|&g| first[g]);
            order
        } else {
            // A table over all rows, indexed by row, lists the groups in order.
            let mut by_row = vec![NO_GROUP; total_rows];
            for (g, &word) in first.iter().enumerate() {
                let (bi, row) = unpack(word);
                by_row[offsets[bi] + row] = g;
            }
            by_row.into_iter().filter(|&g| g != NO_GROUP).collect()
        };
        // A HAVING on a sum is decided here, so the output below is built only
        // for the groups it keeps. Execution's own HAVING pass then keeps them all.
        let order = match &self.having {
            Some(Condition::Cmp(p)) => self
                .dense_having(p, &kinds, &order, &int, &saw, naggs)
                .unwrap_or(order),
            _ => order,
        };

        let out_first = order.iter().map(|&g| Some(unpack(first[g]))).collect();
        let states = kinds
            .iter()
            .enumerate()
            .map(|(a, kind)| match *kind {
                DenseKind::Sum { col } => AggState::Sum {
                    col,
                    label: self.aggregates[a].out_name.clone(),
                    as_float: false,
                    int: order
                        .iter()
                        .map(|&g| i128::from(int[g * naggs + a]))
                        .collect(),
                    float: Vec::new(),
                    saw: order.iter().map(|&g| saw[g * naggs + a]).collect(),
                },
                DenseKind::Count { col, count_star } => AggState::Count {
                    col,
                    count_star,
                    n: order.iter().map(|&g| count[g * naggs + a]).collect(),
                },
            })
            .collect();
        Some((out_first, states))
    }

    /// Merge partials, in order, into one set of groups and states. A group
    /// keeps the position where the sequential pass first saw it.
    fn merge_partials(
        &self,
        batches: &[BatchRef],
        partials: Vec<Partial>,
        dense: Option<(i64, usize)>,
    ) -> (Vec<Option<(usize, usize)>>, Vec<AggState>) {
        let global = self.group_by.is_empty();
        let mut states = self.new_states();
        // Every partial group is merged once, so the total bounds the output:
        // reserve up front instead of growing each vector as groups arrive.
        let total: usize = partials.iter().map(|p| p.first.len()).sum();
        let mut first: Vec<Option<(usize, usize)>> = Vec::with_capacity(total);
        for s in &mut states {
            s.reserve(total);
        }
        let mut index: FastMap<Vec<KeyPart<'_>>, usize> = FastMap::default();
        // With one group column, index on the key itself, as the segment pass
        // does: no per-group Vec.
        let mut single_index: FastMap<KeyPart<'_>, usize> = FastMap::default();
        let single = match self.group_by.as_slice() {
            [c] => Some(*c),
            _ => None,
        };
        // A dense `Int64` key: the slot is the key's offset from the smallest key,
        // as in the segment pass. The NULL group has its own slot.
        let mut dense_slots: Vec<usize> =
            dense.map_or(Vec::new(), |(_, range)| vec![NO_GROUP; range]);
        let mut null_group = NO_GROUP;
        if global {
            first.push(None);
            index.insert(Vec::new(), 0);
            for s in &mut states {
                s.push_group();
            }
        }
        for partial in &partials {
            let mut map = Vec::with_capacity(partial.first.len());
            for rep in &partial.first {
                let gid = match (dense, single, rep) {
                    (Some((min, _)), Some(c), Some((bi, row))) => {
                        let key = key_at(&batches[*bi].columns[c], *row);
                        let known = match key {
                            KeyPart::I64(v) => dense_slots[(v - min) as usize],
                            _ => null_group,
                        };
                        if known != NO_GROUP {
                            known
                        } else {
                            let g = first.len();
                            first.push(*rep);
                            match key {
                                KeyPart::I64(v) => dense_slots[(v - min) as usize] = g,
                                _ => null_group = g,
                            }
                            for s in &mut states {
                                s.push_group();
                            }
                            g
                        }
                    }
                    (_, Some(c), Some((bi, row))) => {
                        let key = key_at(&batches[*bi].columns[c], *row);
                        match single_index.get(&key) {
                            Some(&g) => g,
                            None => {
                                let g = first.len();
                                first.push(*rep);
                                single_index.insert(key, g);
                                for s in &mut states {
                                    s.push_group();
                                }
                                g
                            }
                        }
                    }
                    _ => {
                        let key: Vec<KeyPart<'_>> = match rep {
                            Some((bi, row)) => self
                                .group_by
                                .iter()
                                .map(|&c| key_at(&batches[*bi].columns[c], *row))
                                .collect(),
                            None => Vec::new(),
                        };
                        match index.get(&key) {
                            Some(&g) => g,
                            None => {
                                let g = first.len();
                                first.push(*rep);
                                index.insert(key, g);
                                for s in &mut states {
                                    s.push_group();
                                }
                                g
                            }
                        }
                    }
                };
                map.push(gid);
            }
            for (state, other) in states.iter_mut().zip(&partial.states) {
                state.merge(other, &map, batches);
            }
        }
        (first, states)
    }
}

impl PhysicalPlan for AggregateExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let batches = self.input.execute()?;
        let segs = segments(&batches);
        let dense = match self.group_by.as_slice() {
            [c] => int_key_range(&batches, *c),
            _ => None,
        };
        let fast = match (self.group_by.as_slice(), dense) {
            ([c], Some((min, range))) => self.dense_aggregate(&batches, &segs, *c, min, range),
            _ => None,
        };
        let (first, states) = match fast {
            Some(out) => out,
            None => {
                let mut partials = self.run_partials(&batches, &segs, dense);
                if partials.len() == 1 {
                    let only = partials.remove(0);
                    (only.first, only.states)
                } else {
                    self.merge_partials(&batches, partials, dense)
                }
            }
        };

        // The groups to output, in order: all of them, or those HAVING keeps.
        let keep: Vec<usize> = match &self.having {
            Some(condition) => self.kept_groups(condition, &first, &states, &batches)?,
            None => (0..first.len()).collect(),
        };
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(self.schema.len());
        for (gi, &c) in self.group_by.iter().enumerate() {
            let dt = self.schema.fields[gi].data_type;
            columns.push(gather_opt(&batches, c, keep.iter().map(|&g| first[g]), dt));
        }
        for (i, state) in states.iter().enumerate() {
            let dt = self.schema.fields[self.group_by.len() + i].data_type;
            columns.push(state.finish(dt, &batches, &keep)?);
        }

        Ok(vec![Arc::new(Batch::new_unchecked(
            self.schema.clone(),
            columns,
        ))])
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let input = self.input.schema();
        let keys: Vec<&str> = self
            .group_by
            .iter()
            .map(|&i| input.field(i).map_or("?", |f| f.name.as_str()))
            .collect();
        let aggs: Vec<&str> = self
            .aggregates
            .iter()
            .map(|a| a.out_name.as_str())
            .collect();
        push_line(
            out,
            depth,
            &format!(
                "Aggregate group_by=[{}] aggs=[{}]",
                keys.join(", "),
                aggs.join(", ")
            ),
        );
        self.input.explain(depth + 1, out);
    }
}
