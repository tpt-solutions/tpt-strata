use std::collections::HashMap;
use std::sync::Arc;

use crate::array::ArrayRef;
use crate::batch::Batch;
use crate::error::QueryError;
use crate::schema::{DataType, Field, Schema};
use crate::types::{scalar_cmp, sort_cmp, Scalar};

/// A physical plan node that can be executed to produce batches.
pub trait PhysicalPlan: Send + Sync {
    fn schema(&self) -> &Schema;
    fn execute(&self) -> Result<Vec<Batch>, QueryError>;
}

// === Data source ===

/// No-op source producing pre-materialized batches.
pub struct DataSource {
    schema: Arc<Schema>,
    batches: Vec<Batch>,
}

impl DataSource {
    pub fn new(schema: Arc<Schema>, batches: Vec<Batch>) -> Self {
        Self { schema, batches }
    }

    pub fn from_batch(batch: Batch) -> Self {
        let schema = batch.schema.clone();
        Self {
            schema,
            batches: vec![batch],
        }
    }

    pub fn batches(&self) -> &[Batch] {
        &self.batches
    }
}

impl PhysicalPlan for DataSource {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<Batch>, QueryError> {
        Ok(self.batches.clone())
    }
}

// === Comparison ===

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
}

fn compare_scalars(a: &Scalar, b: &Scalar, op: Comparison) -> bool {
    match op {
        Comparison::Eq => a == b,
        Comparison::NotEq => a != b,
        Comparison::Gt => a.as_f64().zip(b.as_f64()).is_some_and(|(a, b)| a > b),
        Comparison::Gte => a.as_f64().zip(b.as_f64()).is_some_and(|(a, b)| a >= b),
        Comparison::Lt => a.as_f64().zip(b.as_f64()).is_some_and(|(a, b)| a < b),
        Comparison::Lte => a.as_f64().zip(b.as_f64()).is_some_and(|(a, b)| a <= b),
    }
}

/// A row predicate over a single column.
#[derive(Debug, Clone)]
pub struct Predicate {
    pub column: usize,
    pub op: Comparison,
    pub value: Scalar,
}

impl Predicate {
    pub fn matches(&self, batch: &Batch, row: usize) -> bool {
        match batch.columns[self.column].get(row) {
            Some(v) => compare_scalars(&v, &self.value, self.op),
            None => false,
        }
    }
}

// === Filter ===

pub struct FilterExec {
    input: Arc<dyn PhysicalPlan>,
    predicate: Predicate,
    schema: Arc<Schema>,
}

impl FilterExec {
    pub fn new(input: Arc<dyn PhysicalPlan>, predicate: Predicate) -> Self {
        let schema = Arc::new(input.schema().clone());
        Self {
            input,
            predicate,
            schema,
        }
    }
}

impl PhysicalPlan for FilterExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<Batch>, QueryError> {
        let mut out = Vec::new();
        for batch in self.input.execute()? {
            let mut keep = Vec::new();
            for row in 0..batch.row_count() {
                if self.predicate.matches(&batch, row) {
                    keep.push(row);
                }
            }
            let columns = batch
                .columns
                .iter()
                .map(|c| c.take(&keep))
                .collect::<Vec<_>>();
            out.push(Batch::new_unchecked(batch.schema.clone(), columns));
        }
        Ok(out)
    }
}

// === Project ===

/// A projection expression: a reference to an input column.
#[derive(Debug, Clone, Copy)]
pub struct Expr {
    /// Input column index.
    pub column: usize,
}

pub struct ProjectExec {
    input: Arc<dyn PhysicalPlan>,
    exprs: Vec<Expr>,
    schema: Arc<Schema>,
}

impl ProjectExec {
    pub fn new(
        input: Arc<dyn PhysicalPlan>,
        exprs: Vec<Expr>,
        out_names: Vec<String>,
    ) -> Result<Self, QueryError> {
        if exprs.len() != out_names.len() {
            return Err(QueryError::UnsupportedOperation {
                message: "internal error: projection expr/name count mismatch".to_string(),
            });
        }
        let input_schema = input.schema();
        let mut fields = Vec::with_capacity(exprs.len());
        for (e, name) in exprs.iter().zip(out_names.iter()) {
            let src = input_schema
                .field(e.column)
                .ok_or_else(|| QueryError::MissingColumn {
                    column: format!("index {}", e.column),
                    available: vec![],
                })?;
            fields.push(Field::new(name.clone(), src.data_type, src.nullable));
        }
        let schema = Arc::new(Schema::new(fields));
        Ok(Self {
            input,
            exprs,
            schema,
        })
    }
}

impl PhysicalPlan for ProjectExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<Batch>, QueryError> {
        let mut out = Vec::new();
        for batch in self.input.execute()? {
            let columns = self
                .exprs
                .iter()
                .map(|e| batch.columns[e.column].clone())
                .collect::<Vec<_>>();
            out.push(Batch::new_unchecked(self.schema.clone(), columns));
        }
        Ok(out)
    }
}

// === Aggregate ===

/// Whether a data type is orderable for `MIN`/`MAX` (and `ORDER BY`).
///
/// Every native v1 type is orderable, so this is a no-op guard today; it
/// exists so a future non-orderable `DataType` (e.g. binary/JSON) fails at
/// plan-build time with a named diagnostic instead of falling through to
/// `scalar_cmp`'s equal-for-unknown fallback at runtime.
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

trait Accumulator: Send + Sync {
    fn add(&mut self, value: Scalar);
    fn finish(&self) -> Scalar;
}

struct SumAcc {
    acc: f64,
    saw: bool,
}

impl Accumulator for SumAcc {
    fn add(&mut self, value: Scalar) {
        if let Some(v) = value.as_f64() {
            self.acc += v;
            self.saw = true;
        }
    }
    fn finish(&self) -> Scalar {
        if self.saw {
            Scalar::F64(self.acc)
        } else {
            Scalar::Null
        }
    }
}

struct CountAcc {
    n: i64,
    count_star: bool,
}

impl Accumulator for CountAcc {
    fn add(&mut self, value: Scalar) {
        if self.count_star || !matches!(value, Scalar::Null) {
            self.n += 1;
        }
    }
    fn finish(&self) -> Scalar {
        Scalar::I64(self.n)
    }
}

struct MinAcc {
    min: Option<Scalar>,
}

impl Accumulator for MinAcc {
    fn add(&mut self, value: Scalar) {
        match (&self.min, &value) {
            (None, v) if !matches!(v, Scalar::Null) => self.min = Some(value),
            (Some(cur), v)
                if !matches!(v, Scalar::Null) && scalar_cmp(v, cur) == std::cmp::Ordering::Less =>
            {
                self.min = Some(value);
            }
            _ => {}
        }
    }
    fn finish(&self) -> Scalar {
        self.min.clone().unwrap_or(Scalar::Null)
    }
}

struct MaxAcc {
    max: Option<Scalar>,
}

impl Accumulator for MaxAcc {
    fn add(&mut self, value: Scalar) {
        match (&self.max, &value) {
            (None, v) if !matches!(v, Scalar::Null) => self.max = Some(value),
            (Some(cur), v)
                if !matches!(v, Scalar::Null)
                    && scalar_cmp(v, cur) == std::cmp::Ordering::Greater =>
            {
                self.max = Some(value);
            }
            _ => {}
        }
    }
    fn finish(&self) -> Scalar {
        self.max.clone().unwrap_or(Scalar::Null)
    }
}

struct AvgAcc {
    sum: f64,
    n: usize,
}

impl Accumulator for AvgAcc {
    fn add(&mut self, value: Scalar) {
        if let Some(v) = value.as_f64() {
            self.sum += v;
            self.n += 1;
        }
    }
    fn finish(&self) -> Scalar {
        if self.n == 0 {
            Scalar::Null
        } else {
            Scalar::F64(self.sum / self.n as f64)
        }
    }
}

fn new_accumulator(agg: &Aggregate) -> Box<dyn Accumulator> {
    match agg.func {
        AggFunc::Sum => Box::new(SumAcc {
            acc: 0.0,
            saw: false,
        }),
        AggFunc::Count => Box::new(CountAcc {
            n: 0,
            count_star: agg.count_star,
        }),
        AggFunc::Min => Box::new(MinAcc { min: None }),
        AggFunc::Max => Box::new(MaxAcc { max: None }),
        AggFunc::Avg => Box::new(AvgAcc { sum: 0.0, n: 0 }),
    }
}

/// Resolve the output [`DataType`] of an aggregate over an input schema.
fn aggregate_output_type(input_schema: &Schema, agg: &Aggregate) -> DataType {
    match agg.func {
        AggFunc::Count => DataType::Int64,
        AggFunc::Min | AggFunc::Max => input_schema
            .field(agg.column)
            .map(|f| f.data_type)
            .unwrap_or(DataType::Float64),
        AggFunc::Sum | AggFunc::Avg => DataType::Float64,
    }
}

/// Build an array of finished aggregate values with the correct type.
fn make_agg_array(dt: DataType, vals: &[Scalar]) -> ArrayRef {
    match dt {
        DataType::Boolean => {
            let v = vals
                .iter()
                .map(|s| match s {
                    Scalar::Bool(b) => Some(*b),
                    _ => None,
                })
                .collect();
            ArrayRef::Boolean(crate::array::BoolArray(v))
        }
        DataType::Int32 => {
            let v = vals
                .iter()
                .map(|s| match s {
                    Scalar::I32(b) => Some(*b),
                    _ => None,
                })
                .collect();
            ArrayRef::Int32(crate::array::I32Array(v))
        }
        DataType::Int64 => {
            let v = vals
                .iter()
                .map(|s| match s {
                    Scalar::I64(b) => Some(*b),
                    _ => None,
                })
                .collect();
            ArrayRef::Int64(crate::array::I64Array(v))
        }
        DataType::Float64 => {
            let v = vals
                .iter()
                .map(|s| match s {
                    Scalar::F64(b) => Some(*b),
                    _ => None,
                })
                .collect();
            ArrayRef::Float64(crate::array::F64Array(v))
        }
        DataType::Utf8 => {
            let v = vals
                .iter()
                .map(|s| match s {
                    Scalar::Str(b) => Some(b.clone()),
                    _ => None,
                })
                .collect();
            ArrayRef::Utf8(crate::array::StrArray(v))
        }
    }
}

fn make_grouped_column(dt: DataType, vals: Vec<Scalar>) -> ArrayRef {
    make_agg_array(dt, &vals)
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
                    available: vec![],
                })?;
            fields.push(Field::new(f.name.clone(), f.data_type, f.nullable));
        }
        for agg in &aggregates {
            let f = input_schema
                .field(agg.column)
                .ok_or_else(|| QueryError::MissingColumn {
                    column: format!("index {}", agg.column),
                    available: vec![],
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

    fn group_key(&self, batch: &Batch, row: usize) -> Vec<Scalar> {
        self.group_by
            .iter()
            .map(|&idx| batch.columns[idx].get(row).unwrap_or(Scalar::Null))
            .collect()
    }
}

impl PhysicalPlan for AggregateExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<Batch>, QueryError> {
        let batches = self.input.execute()?;

        let mut groups: HashMap<Vec<Scalar>, Vec<Box<dyn Accumulator>>> = HashMap::new();

        for batch in &batches {
            for row in 0..batch.row_count() {
                let key = self.group_key(batch, row);
                let accs = groups.entry(key).or_insert_with(|| {
                    self.aggregates.iter().map(|a| new_accumulator(a)).collect()
                });
                for (i, agg) in self.aggregates.iter().enumerate() {
                    let value = batch.columns[agg.column].get(row).unwrap_or(Scalar::Null);
                    accs[i].add(value);
                }
            }
        }

        if self.group_by.is_empty() {
            // Global aggregation: a single output row (even over zero rows).
            let key = Vec::new();
            let accs = groups
                .entry(key)
                .or_insert_with(|| self.aggregates.iter().map(|a| new_accumulator(a)).collect());
            let mut columns: Vec<ArrayRef> = Vec::new();
            for (i, agg) in self.aggregates.iter().enumerate() {
                let dt = aggregate_output_type(self.input.schema(), agg);
                columns.push(make_agg_array(dt, &[accs[i].finish()]));
            }
            return Ok(vec![Batch::new_unchecked(self.schema.clone(), columns)]);
        }

        // Grouped: one output row per distinct key.
        let mut columns: Vec<ArrayRef> = Vec::new();
        for (gi, &idx) in self.group_by.iter().enumerate() {
            let dt = self
                .input
                .schema()
                .field(idx)
                .map(|f| f.data_type)
                .unwrap_or(DataType::Int64);
            let vals: Vec<Scalar> = groups.keys().map(|key| key[gi].clone()).collect();
            columns.push(make_grouped_column(dt, vals));
        }
        for (i, agg) in self.aggregates.iter().enumerate() {
            let dt = aggregate_output_type(self.input.schema(), agg);
            let finished: Vec<Scalar> = groups.values().map(|accs| accs[i].finish()).collect();
            columns.push(make_agg_array(dt, &finished));
        }

        Ok(vec![Batch::new_unchecked(self.schema.clone(), columns)])
    }
}

// === Join ===

pub struct JoinExec {
    left: Arc<dyn PhysicalPlan>,
    right: Arc<dyn PhysicalPlan>,
    left_key: usize,
    right_key: usize,
    schema: Arc<Schema>,
}

impl JoinExec {
    pub fn new(
        left: Arc<dyn PhysicalPlan>,
        right: Arc<dyn PhysicalPlan>,
        left_key: usize,
        right_key: usize,
    ) -> Self {
        let left_schema = left.schema();
        let right_schema = right.schema();
        let mut fields = left_schema.fields.clone();
        fields.extend(right_schema.fields.iter().cloned());
        let schema = Arc::new(Schema::new(fields));
        Self {
            left,
            right,
            left_key,
            right_key,
            schema,
        }
    }
}

impl PhysicalPlan for JoinExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<Batch>, QueryError> {
        let left_batches = self.left.execute()?;
        let right_batches = self.right.execute()?;

        if left_batches.is_empty() || right_batches.is_empty() {
            return Ok(Vec::new());
        }

        // Build a hash map from the right key to matching (batch, row) positions.
        let mut right_index: HashMap<Scalar, Vec<(usize, usize)>> = HashMap::new();
        for (bi, batch) in right_batches.iter().enumerate() {
            if batch.columns.is_empty() {
                continue;
            }
            for row in 0..batch.row_count() {
                let key = batch.columns[self.right_key]
                    .get(row)
                    .unwrap_or(Scalar::Null);
                if matches!(key, Scalar::Null) {
                    continue;
                }
                right_index.entry(key).or_default().push((bi, row));
            }
        }

        let mut left_rows: Vec<(usize, usize)> = Vec::new();
        let mut right_rows: Vec<(usize, usize)> = Vec::new();
        for (lbi, lb) in left_batches.iter().enumerate() {
            if lb.columns.is_empty() {
                continue;
            }
            for row in 0..lb.row_count() {
                let key = lb.columns[self.left_key].get(row).unwrap_or(Scalar::Null);
                if matches!(key, Scalar::Null) {
                    continue;
                }
                if let Some(matches) = right_index.get(&key) {
                    for &(rbi, rrow) in matches {
                        left_rows.push((lbi, row));
                        right_rows.push((rbi, rrow));
                    }
                }
            }
        }

        // Materialize matched rows as a single output batch.
        let left_width = left_batches[0].columns.len();
        let total_width = left_width + right_batches[0].columns.len();
        let mut out_cols: Vec<Vec<Scalar>> = (0..total_width).map(|_| Vec::new()).collect();

        for (lbi, lrow) in &left_rows {
            let batch = &left_batches[*lbi];
            for (ci, col) in batch.columns.iter().enumerate() {
                out_cols[ci].push(col.get(*lrow).unwrap_or(Scalar::Null));
            }
        }
        for (rbi, rrow) in &right_rows {
            let batch = &right_batches[*rbi];
            for (ci, col) in batch.columns.iter().enumerate() {
                out_cols[left_width + ci].push(col.get(*rrow).unwrap_or(Scalar::Null));
            }
        }

        let columns: Vec<ArrayRef> = out_cols
            .into_iter()
            .enumerate()
            .map(|(ci, vals)| {
                let dt = self
                    .schema
                    .field(ci)
                    .map(|f| f.data_type)
                    .unwrap_or(DataType::Int64);
                make_grouped_column(dt, vals)
            })
            .collect();

        Ok(vec![Batch::new_unchecked(self.schema.clone(), columns)])
    }
}

// === Sort ===

#[derive(Debug, Clone)]
pub struct SortExpr {
    pub column: usize,
    pub desc: bool,
}

pub struct SortExec {
    input: Arc<dyn PhysicalPlan>,
    sort_exprs: Vec<SortExpr>,
    schema: Arc<Schema>,
}

impl SortExec {
    pub fn new(input: Arc<dyn PhysicalPlan>, sort_exprs: Vec<SortExpr>) -> Self {
        let schema = Arc::new(input.schema().clone());
        Self {
            input,
            sort_exprs,
            schema,
        }
    }
}

impl PhysicalPlan for SortExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<Batch>, QueryError> {
        let batches = self.input.execute()?;

        // Materialize the full row set into per-column scalar vectors.
        let num_cols = batches.first().map_or(0, |b| b.columns.len());
        let mut cols: Vec<Vec<Scalar>> = (0..num_cols).map(|_| Vec::new()).collect();

        for batch in &batches {
            for row in 0..batch.row_count() {
                for (ci, col) in batch.columns.iter().enumerate() {
                    cols[ci].push(col.get(row).unwrap_or(Scalar::Null));
                }
            }
        }

        let n = cols.first().map_or(0, |c| c.len());
        let mut indices: Vec<usize> = (0..n).collect();
        indices.sort_by(|&a, &b| {
            for se in &self.sort_exprs {
                // `sort_cmp` (not `scalar_cmp`) so NULL gets a deterministic
                // position (first ascending) instead of silently comparing
                // equal to non-null values.
                let ord = sort_cmp(&cols[se.column][a], &cols[se.column][b]);
                let ord = if se.desc { ord.reverse() } else { ord };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });

        let columns: Vec<ArrayRef> = (0..num_cols)
            .map(|ci| {
                let dt = self
                    .schema
                    .field(ci)
                    .map(|f| f.data_type)
                    .unwrap_or(DataType::Int64);
                let ordered: Vec<Scalar> = indices.iter().map(|&i| cols[ci][i].clone()).collect();
                make_grouped_column(dt, ordered)
            })
            .collect();

        Ok(vec![Batch::new_unchecked(self.schema.clone(), columns)])
    }
}

// === Limit ===

pub struct LimitExec {
    input: Arc<dyn PhysicalPlan>,
    n: usize,
    schema: Arc<Schema>,
}

impl LimitExec {
    pub fn new(input: Arc<dyn PhysicalPlan>, n: usize) -> Self {
        let schema = Arc::new(input.schema().clone());
        Self { input, n, schema }
    }
}

impl PhysicalPlan for LimitExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<Batch>, QueryError> {
        let batches = self.input.execute()?;
        let mut out = Vec::new();
        let mut remaining = self.n;
        for batch in batches {
            if remaining == 0 {
                break;
            }
            let rows = batch.row_count();
            if rows <= remaining {
                remaining -= rows;
                out.push(batch);
            } else {
                let indices: Vec<usize> = (0..remaining).collect();
                let columns = batch
                    .columns
                    .iter()
                    .map(|c| c.take(&indices))
                    .collect::<Vec<_>>();
                out.push(Batch::new_unchecked(batch.schema.clone(), columns));
                remaining = 0;
            }
        }
        Ok(out)
    }
}
