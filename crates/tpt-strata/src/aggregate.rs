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
        DataType::Boolean | DataType::Int32 | DataType::Int64 | DataType::Float64 | DataType::Utf8
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
    fn update(&mut self, gid: usize, batches: &[BatchRef], pos: (usize, usize)) {
        let (bi, row) = pos;
        match self {
            AggState::Sum {
                col,
                int,
                float,
                saw,
                ..
            } => match &*batches[bi].columns[*col] {
                ArrayRef::Int32(a) => {
                    if let Some(v) = a.0[row] {
                        int[gid] += i128::from(v);
                        float[gid] += f64::from(v);
                        saw[gid] = true;
                    }
                }
                ArrayRef::Int64(a) => {
                    if let Some(v) = a.0[row] {
                        int[gid] += i128::from(v);
                        float[gid] += v as f64;
                        saw[gid] = true;
                    }
                }
                ArrayRef::Float64(a) => {
                    if let Some(v) = a.0[row] {
                        float[gid] += v;
                        saw[gid] = true;
                    }
                }
                _ => {}
            },
            AggState::Count { col, count_star, n } => {
                if *count_star || !batches[bi].columns[*col].is_null(row) {
                    n[gid] += 1;
                }
            }
            AggState::MinMax { col, max, best } => {
                let value = &batches[bi].columns[*col];
                if value.is_null(row) {
                    return;
                }
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
            AggState::Avg { col, int, float, n } => match &*batches[bi].columns[*col] {
                ArrayRef::Int32(a) => {
                    if let Some(v) = a.0[row] {
                        int[gid] += i128::from(v);
                        n[gid] += 1;
                    }
                }
                ArrayRef::Int64(a) => {
                    if let Some(v) = a.0[row] {
                        int[gid] += i128::from(v);
                        n[gid] += 1;
                    }
                }
                ArrayRef::Float64(a) => {
                    if let Some(v) = a.0[row] {
                        float[gid] += v;
                        n[gid] += 1;
                    }
                }
                _ => {}
            },
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

impl PhysicalPlan for AggregateExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let batches = self.input.execute()?;
        let input = self.input.schema();
        let global = self.group_by.is_empty();

        let mut states: Vec<AggState> = self
            .aggregates
            .iter()
            .map(|a| {
                let dt = input
                    .field(a.column)
                    .map_or(DataType::Float64, |f| f.data_type);
                AggState::new(a, dt)
            })
            .collect();

        // Representative row of each group, in first-seen order. A global
        // aggregate has one group, which has no representative row when the
        // input is empty.
        let mut first: Vec<Option<(usize, usize)>> = Vec::new();
        let mut index: FastMap<Vec<KeyPart<'_>>, usize> = FastMap::default();
        if global {
            first.push(None);
            for s in &mut states {
                s.push_group();
            }
        }

        let mut scratch: Vec<KeyPart<'_>> = Vec::with_capacity(self.group_by.len());
        for (bi, batch) in batches.iter().enumerate() {
            for row in 0..batch.row_count() {
                let gid = if global {
                    0
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
                for s in &mut states {
                    s.update(gid, &batches, (bi, row));
                }
            }
        }

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
