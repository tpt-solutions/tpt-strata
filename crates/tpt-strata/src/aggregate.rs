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
use crate::engine::{gather_opt, names, push_line, PhysicalPlan};
use crate::error::QueryError;
use crate::key::{key_at, FastMap, KeyPart};
use crate::schema::{DataType, Field, Schema};

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

    /// Make room for one more group, initialised to its empty state.
    fn push_group(&mut self) {
        match self {
            AggState::Sum {
                int, float, saw, ..
            } => {
                int.push(0);
                float.push(0.0);
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
                ArrayRef::Int32(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.0[row] {
                            int[gid] += i128::from(v);
                            float[gid] += f64::from(v);
                            saw[gid] = true;
                        }
                    }
                }
                ArrayRef::Int64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.0[row] {
                            int[gid] += i128::from(v);
                            float[gid] += v as f64;
                            saw[gid] = true;
                        }
                    }
                }
                ArrayRef::Float64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.0[row] {
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
                        if let Some(v) = a.0[row] {
                            int[gid] += i128::from(v);
                            n[gid] += 1;
                        }
                    }
                }
                ArrayRef::Int64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.0[row] {
                            int[gid] += i128::from(v);
                            n[gid] += 1;
                        }
                    }
                }
                ArrayRef::Float64(a) => {
                    for (k, &gid) in gids.iter().enumerate() {
                        let row = start + k;
                        if let Some(v) = a.0[row] {
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
                for (lg, &gg) in map.iter().enumerate() {
                    int[gg] += oi[lg];
                    float[gg] += of[lg];
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
    fn finish(
        &self,
        dt: DataType,
        batches: &[BatchRef],
        groups: usize,
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
                        (0..groups).map(|g| saw[g].then_some(float[g])).collect();
                    Ok(ArrayRef::Float64(F64Array::from_nulls(vals)))
                } else {
                    let vals = (0..groups)
                        .map(|g| {
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
            AggState::Count { n, .. } => Ok(ArrayRef::Int64(I64Array::new(n[..groups].to_vec()))),
            AggState::MinMax { col, best, .. } => Ok(gather_opt(
                batches,
                *col,
                best[..groups].iter().copied(),
                dt,
            )),
            AggState::Avg { int, float, n, .. } => {
                let vals: Vec<Option<f64>> = (0..groups)
                    .map(|g| (n[g] > 0).then(|| (int[g] as f64 + float[g]) / n[g] as f64))
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
        })
    }
}

/// Rows per unit of aggregation work. Segments are the unit that threads
/// share, so a large batch splits across them.
const SEGMENT_ROWS: usize = 64 * 1024;

/// `(batch, start, end)`: rows `start..end` of batch `batch`, in input order.
type Segment = (usize, usize, usize);

fn segments(batches: &[BatchRef]) -> Vec<Segment> {
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
    fn run_segments(&self, batches: &[BatchRef], segs: &[Segment]) -> Partial {
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

        let mut scratch: Vec<KeyPart<'_>> = Vec::with_capacity(self.group_by.len());
        let mut gids: Vec<usize> = Vec::new();
        for &(bi, start, end) in segs {
            let batch = &batches[bi];
            gids.clear();
            for row in start..end {
                let gid = if global {
                    0
                } else if let [c] = self.group_by.as_slice() {
                    let key = key_at(&batch.columns[*c], row);
                    match single_index.get(&key) {
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
                    }
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
    fn run_partials(&self, batches: &[BatchRef], segs: &[Segment]) -> Vec<Partial> {
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
                        .map(|chunk| scope.spawn(move || self.run_segments(batches, chunk)))
                        .collect();
                    workers
                        .into_iter()
                        .map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                        .collect()
                });
            }
        }
        vec![self.run_segments(batches, segs)]
    }

    /// Merge partials, in order, into one set of groups and states. A group
    /// keeps the position where the sequential pass first saw it.
    fn merge_partials(
        &self,
        batches: &[BatchRef],
        partials: Vec<Partial>,
    ) -> (Vec<Option<(usize, usize)>>, Vec<AggState>) {
        let global = self.group_by.is_empty();
        let mut states = self.new_states();
        let mut first: Vec<Option<(usize, usize)>> = Vec::new();
        let mut index: FastMap<Vec<KeyPart<'_>>, usize> = FastMap::default();
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
                let key: Vec<KeyPart<'_>> = match rep {
                    Some((bi, row)) => self
                        .group_by
                        .iter()
                        .map(|&c| key_at(&batches[*bi].columns[c], *row))
                        .collect(),
                    None => Vec::new(),
                };
                let gid = match index.get(&key) {
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
        let mut partials = self.run_partials(&batches, &segs);
        let (first, states) = if partials.len() == 1 {
            let only = partials.remove(0);
            (only.first, only.states)
        } else {
            self.merge_partials(&batches, partials)
        };

        let groups = first.len();
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(self.schema.len());
        for (gi, &c) in self.group_by.iter().enumerate() {
            let dt = self.schema.fields[gi].data_type;
            columns.push(gather_opt(&batches, c, first.iter().copied(), dt));
        }
        for (i, state) in states.iter().enumerate() {
            let dt = self.schema.fields[self.group_by.len() + i].data_type;
            columns.push(state.finish(dt, &batches, groups)?);
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
