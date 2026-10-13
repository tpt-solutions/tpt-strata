use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::array::{empty_array, ArrayRef, BoolArray, F64Array, I32Array, I64Array, StrArray};
use crate::batch::{Batch, BatchRef};
use crate::error::QueryError;
use crate::schema::{DataType, Field, Schema};
use crate::types::{numeric_cmp, sort_cmp, Scalar};

/// A physical plan node that can be executed to produce batches.
pub trait PhysicalPlan: Send + Sync {
    fn schema(&self) -> &Schema;
    fn execute(&self) -> Result<Vec<BatchRef>, QueryError>;
    /// Write this operator and its inputs as an indented tree, one operator
    /// per line, root first.
    fn explain(&self, depth: usize, out: &mut String) {
        push_line(out, depth, "Operator");
    }
}

fn push_line(out: &mut String, depth: usize, text: &str) {
    for _ in 0..depth {
        out.push_str("  ");
    }
    out.push_str(text);
    out.push('\n');
}

fn internal(message: &str) -> QueryError {
    QueryError::UnsupportedOperation {
        message: format!("internal error: {message}"),
    }
}

fn names(schema: &Schema) -> Vec<String> {
    schema.fields.iter().map(|f| f.name.clone()).collect()
}

/// A zero-row batch with the given schema, so a result always has a batch.
fn empty_batch(schema: &Arc<Schema>) -> BatchRef {
    let columns = schema
        .fields
        .iter()
        .map(|f| empty_array(f.data_type))
        .collect();
    Arc::new(Batch::new_unchecked(schema.clone(), columns))
}

/// Build a new array for column `col` by reading the `(batch, row)` positions
/// in `pos`, in order. Typed end to end: no per-value `Scalar` boxing.
fn gather(batches: &[BatchRef], col: usize, pos: &[(usize, usize)], dt: DataType) -> ArrayRef {
    macro_rules! take_typed {
        ($variant:ident, $arr:ident) => {{
            let vals = pos
                .iter()
                .map(|&(b, r)| match &batches[b].columns[col] {
                    ArrayRef::$variant(a) => a.0.get(r).cloned().flatten(),
                    _ => None,
                })
                .collect();
            ArrayRef::$variant($arr(vals))
        }};
    }
    match dt {
        DataType::Boolean => take_typed!(Boolean, BoolArray),
        DataType::Int32 => take_typed!(Int32, I32Array),
        DataType::Int64 => take_typed!(Int64, I64Array),
        DataType::Float64 => take_typed!(Float64, F64Array),
        DataType::Utf8 => take_typed!(Utf8, StrArray),
    }
}

/// Build a typed array from finished scalars. Values of another type become
/// nulls, which only happens for the all-null result of an empty group.
fn array_from_scalars(dt: DataType, vals: &[Scalar]) -> ArrayRef {
    match dt {
        DataType::Boolean => ArrayRef::Boolean(BoolArray(
            vals.iter()
                .map(|s| match s {
                    Scalar::Bool(b) => Some(*b),
                    _ => None,
                })
                .collect(),
        )),
        DataType::Int32 => ArrayRef::Int32(I32Array(
            vals.iter()
                .map(|s| match s {
                    Scalar::I32(b) => Some(*b),
                    _ => None,
                })
                .collect(),
        )),
        DataType::Int64 => ArrayRef::Int64(I64Array(
            vals.iter()
                .map(|s| match s {
                    Scalar::I64(b) => Some(*b),
                    _ => None,
                })
                .collect(),
        )),
        DataType::Float64 => ArrayRef::Float64(F64Array(
            vals.iter()
                .map(|s| match s {
                    Scalar::F64(b) => Some(*b),
                    _ => None,
                })
                .collect(),
        )),
        DataType::Utf8 => ArrayRef::Utf8(StrArray(
            vals.iter()
                .map(|s| match s {
                    Scalar::Str(b) => Some(b.clone()),
                    _ => None,
                })
                .collect(),
        )),
    }
}

// === Data source ===

/// Source operator over pre-materialized batches.
///
/// Batches are shared, not copied: `execute` hands out `Arc` clones.
pub struct DataSource {
    schema: Arc<Schema>,
    batches: Vec<BatchRef>,
}

impl DataSource {
    pub fn new(schema: Arc<Schema>, batches: Vec<Batch>) -> Self {
        Self {
            schema,
            batches: batches.into_iter().map(Arc::new).collect(),
        }
    }

    /// Build a source from batches that are already shared.
    pub fn from_shared(schema: Arc<Schema>, batches: Vec<BatchRef>) -> Self {
        Self { schema, batches }
    }

    pub fn from_batch(batch: Batch) -> Self {
        let schema = batch.schema.clone();
        Self::new(schema, vec![batch])
    }

    pub fn batches(&self) -> &[BatchRef] {
        &self.batches
    }
}

impl PhysicalPlan for DataSource {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        Ok(self.batches.clone())
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let rows: usize = self.batches.iter().map(|b| b.row_count()).sum();
        push_line(
            out,
            depth,
            &format!(
                "Scan rows={rows} columns=[{}]",
                names(&self.schema).join(", ")
            ),
        );
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
    /// `col IS NULL`. The predicate's value is ignored.
    IsNull,
    /// `col IS NOT NULL`. The predicate's value is ignored.
    IsNotNull,
    /// `col LIKE pattern`, where `%` matches any run and `_` one character.
    Like,
    /// `col NOT LIKE pattern`.
    NotLike,
}

impl Comparison {
    /// The operator as written in SQL.
    pub fn symbol(self) -> &'static str {
        match self {
            Comparison::Eq => "=",
            Comparison::NotEq => "!=",
            Comparison::Gt => ">",
            Comparison::Gte => ">=",
            Comparison::Lt => "<",
            Comparison::Lte => "<=",
            Comparison::IsNull => "IS NULL",
            Comparison::IsNotNull => "IS NOT NULL",
            Comparison::Like => "LIKE",
            Comparison::NotLike => "NOT LIKE",
        }
    }
}

/// Match `text` against a SQL `LIKE` pattern (`%` any run, `_` one char).
pub(crate) fn like_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    let (mut ti, mut pi) = (0usize, 0usize);
    // Position of the last `%` seen, and the text index it currently absorbs up to.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '%' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && (p[pi] == '_' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if let Some((sp, st)) = star {
            // Let the last `%` absorb one more character and retry.
            pi = sp + 1;
            star = Some((sp, st + 1));
            ti = st + 1;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '%' {
        pi += 1;
    }
    pi == p.len()
}

/// Evaluate `a op b` for two non-null values.
///
/// Ordering comparisons use [`numeric_cmp`] (exact across integer and float
/// types) or same-type string/bool order. Incomparable pairs and NaN compare
/// false for every operator.
fn compare_values(a: &Scalar, b: &Scalar, op: Comparison) -> bool {
    let ord = numeric_cmp(a, b).or_else(|| match (a, b) {
        (Scalar::Str(x), Scalar::Str(y)) => Some(x.cmp(y)),
        (Scalar::Bool(x), Scalar::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    });
    match op {
        Comparison::Eq => ord == Some(Ordering::Equal),
        Comparison::NotEq => matches!(ord, Some(o) if o != Ordering::Equal),
        Comparison::Gt => ord == Some(Ordering::Greater),
        Comparison::Gte => matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
        Comparison::Lt => ord == Some(Ordering::Less),
        Comparison::Lte => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
        Comparison::Like | Comparison::NotLike => match (a, b) {
            (Scalar::Str(text), Scalar::Str(pattern)) => {
                like_match(text, pattern) == (op == Comparison::Like)
            }
            _ => false,
        },
        // Handled by `Predicate::matches`, which sees NULL rows.
        Comparison::IsNull | Comparison::IsNotNull => false,
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
    /// Whether `row` passes the predicate.
    ///
    /// A NULL on either side of a comparison makes it unknown, and unknown
    /// rows are filtered out (SQL three-valued logic). So `col = NULL` and
    /// `col != NULL` both match nothing; use `IS NULL` to test for nulls.
    pub fn matches(&self, batch: &Batch, row: usize) -> bool {
        let value = batch.columns.get(self.column).and_then(|c| c.get(row));
        match self.op {
            Comparison::IsNull => value.is_none(),
            Comparison::IsNotNull => value.is_some(),
            op => match value {
                Some(v) if !matches!(self.value, Scalar::Null) => {
                    compare_values(&v, &self.value, op)
                }
                _ => false,
            },
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

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let mut out = Vec::new();
        for batch in self.input.execute()? {
            let keep: Vec<usize> = (0..batch.row_count())
                .filter(|&row| self.predicate.matches(&batch, row))
                .collect();
            let columns = batch.columns.iter().map(|c| c.take(&keep)).collect();
            out.push(Arc::new(Batch::new_unchecked(self.schema.clone(), columns)));
        }
        if out.is_empty() {
            out.push(empty_batch(&self.schema));
        }
        Ok(out)
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let column = self
            .schema
            .field(self.predicate.column)
            .map_or("?", |f| f.name.as_str());
        let op = self.predicate.op;
        let text = match op {
            Comparison::IsNull | Comparison::IsNotNull => {
                format!("Filter {column} {}", op.symbol())
            }
            _ => format!("Filter {column} {} {}", op.symbol(), self.predicate.value),
        };
        push_line(out, depth, &text);
        self.input.explain(depth + 1, out);
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
            return Err(internal("projection expr/name count mismatch"));
        }
        let input_schema = input.schema();
        let mut fields = Vec::with_capacity(exprs.len());
        for (e, name) in exprs.iter().zip(out_names.iter()) {
            let src = input_schema
                .field(e.column)
                .ok_or_else(|| QueryError::MissingColumn {
                    column: format!("index {}", e.column),
                    available: names(input_schema),
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

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let mut out = Vec::new();
        for batch in self.input.execute()? {
            let columns = self
                .exprs
                .iter()
                .map(|e| {
                    batch
                        .columns
                        .get(e.column)
                        .cloned()
                        .ok_or_else(|| internal("projection column out of range"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            out.push(Arc::new(Batch::new_unchecked(self.schema.clone(), columns)));
        }
        if out.is_empty() {
            out.push(empty_batch(&self.schema));
        }
        Ok(out)
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let cols: Vec<&str> = self.schema.fields.iter().map(|f| f.name.as_str()).collect();
        push_line(out, depth, &format!("Project [{}]", cols.join(", ")));
        self.input.explain(depth + 1, out);
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
    fn finish(&self) -> Result<Scalar, QueryError>;
}

/// `SUM`. Integer input sums in `i128` and yields `Int64`, failing loudly on
/// overflow. Float input sums in `f64` and yields `Float64`.
struct SumAcc {
    label: String,
    as_float: bool,
    int: i128,
    float: f64,
    saw: bool,
}

impl Accumulator for SumAcc {
    fn add(&mut self, value: Scalar) {
        match value {
            Scalar::I32(v) => {
                self.int += i128::from(v);
                self.float += f64::from(v);
                self.saw = true;
            }
            Scalar::I64(v) => {
                self.int += i128::from(v);
                self.float += v as f64;
                self.saw = true;
            }
            Scalar::F64(v) => {
                self.float += v;
                self.saw = true;
            }
            _ => {}
        }
    }

    fn finish(&self) -> Result<Scalar, QueryError> {
        if !self.saw {
            return Ok(Scalar::Null);
        }
        if self.as_float {
            return Ok(Scalar::F64(self.float));
        }
        i64::try_from(self.int)
            .map(Scalar::I64)
            .map_err(|_| QueryError::UnsupportedOperation {
                message: format!(
                    "SUM overflowed i64 in '{}'; narrow the input or sum a float column",
                    self.label
                ),
            })
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
    fn finish(&self) -> Result<Scalar, QueryError> {
        Ok(Scalar::I64(self.n))
    }
}

struct MinAcc {
    min: Option<Scalar>,
}

impl Accumulator for MinAcc {
    fn add(&mut self, value: Scalar) {
        match (&self.min, &value) {
            (_, Scalar::Null) => {}
            (None, _) => self.min = Some(value),
            (Some(cur), v) if sort_cmp(v, cur) == Ordering::Less => self.min = Some(value),
            _ => {}
        }
    }
    fn finish(&self) -> Result<Scalar, QueryError> {
        Ok(self.min.clone().unwrap_or(Scalar::Null))
    }
}

struct MaxAcc {
    max: Option<Scalar>,
}

impl Accumulator for MaxAcc {
    fn add(&mut self, value: Scalar) {
        match (&self.max, &value) {
            (_, Scalar::Null) => {}
            (None, _) => self.max = Some(value),
            (Some(cur), v) if sort_cmp(v, cur) == Ordering::Greater => self.max = Some(value),
            _ => {}
        }
    }
    fn finish(&self) -> Result<Scalar, QueryError> {
        Ok(self.max.clone().unwrap_or(Scalar::Null))
    }
}

/// `AVG`. Integer input is summed exactly in `i128` before dividing.
struct AvgAcc {
    int: i128,
    float: f64,
    n: usize,
}

impl Accumulator for AvgAcc {
    fn add(&mut self, value: Scalar) {
        match value {
            Scalar::I32(v) => {
                self.int += i128::from(v);
                self.n += 1;
            }
            Scalar::I64(v) => {
                self.int += i128::from(v);
                self.n += 1;
            }
            Scalar::F64(v) => {
                self.float += v;
                self.n += 1;
            }
            _ => {}
        }
    }
    fn finish(&self) -> Result<Scalar, QueryError> {
        if self.n == 0 {
            Ok(Scalar::Null)
        } else {
            let total = self.int as f64 + self.float;
            Ok(Scalar::F64(total / self.n as f64))
        }
    }
}

fn new_accumulator(agg: &Aggregate, input: DataType) -> Box<dyn Accumulator> {
    match agg.func {
        AggFunc::Sum => Box::new(SumAcc {
            label: agg.out_name.clone(),
            as_float: input == DataType::Float64,
            int: 0,
            float: 0.0,
            saw: false,
        }),
        AggFunc::Count => Box::new(CountAcc {
            n: 0,
            count_star: agg.count_star,
        }),
        AggFunc::Min => Box::new(MinAcc { min: None }),
        AggFunc::Max => Box::new(MaxAcc { max: None }),
        AggFunc::Avg => Box::new(AvgAcc {
            int: 0,
            float: 0.0,
            n: 0,
        }),
    }
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

fn cmp_keys(a: &[Scalar], b: &[Scalar]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let ord = sort_cmp(x, y);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// One group: its key values and one accumulator per aggregate.
type GroupEntry = (Vec<Scalar>, Vec<Box<dyn Accumulator>>);

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

    fn group_key(&self, batch: &Batch, row: usize) -> Vec<Scalar> {
        self.group_by
            .iter()
            .map(|&idx| batch.columns[idx].get(row).unwrap_or(Scalar::Null))
            .collect()
    }

    fn new_accumulators(&self) -> Vec<Box<dyn Accumulator>> {
        let input = self.input.schema();
        self.aggregates
            .iter()
            .map(|a| {
                let dt = input
                    .field(a.column)
                    .map_or(DataType::Float64, |f| f.data_type);
                new_accumulator(a, dt)
            })
            .collect()
    }
}

impl PhysicalPlan for AggregateExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let batches = self.input.execute()?;

        let mut groups: HashMap<Vec<Scalar>, Vec<Box<dyn Accumulator>>> = HashMap::new();
        for batch in &batches {
            for row in 0..batch.row_count() {
                let key = self.group_key(batch, row);
                let accs = groups.entry(key).or_insert_with(|| self.new_accumulators());
                for (i, agg) in self.aggregates.iter().enumerate() {
                    let value = batch.columns[agg.column].get(row).unwrap_or(Scalar::Null);
                    accs[i].add(value);
                }
            }
        }

        let mut entries: Vec<GroupEntry> = groups.into_iter().collect();
        if self.group_by.is_empty() && entries.is_empty() {
            // Global aggregation: a single output row, even over zero rows.
            entries.push((Vec::new(), self.new_accumulators()));
        }
        // Stable, deterministic output order: sorted by group key.
        entries.sort_by(|(a, _), (b, _)| cmp_keys(a, b));

        let mut columns: Vec<ArrayRef> = Vec::with_capacity(self.schema.len());
        for (gi, _) in self.group_by.iter().enumerate() {
            let dt = self.schema.fields[gi].data_type;
            let vals: Vec<Scalar> = entries.iter().map(|(k, _)| k[gi].clone()).collect();
            columns.push(array_from_scalars(dt, &vals));
        }
        for (i, _) in self.aggregates.iter().enumerate() {
            let dt = self.schema.fields[self.group_by.len() + i].data_type;
            let finished = entries
                .iter()
                .map(|(_, accs)| accs[i].finish())
                .collect::<Result<Vec<_>, _>>()?;
            columns.push(array_from_scalars(dt, &finished));
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

// === Join ===

/// Inner equi-join (hash join) on one key column per side.
///
/// Output columns are the left schema followed by the right schema. The two
/// sides must not share a column name, because the output names must be
/// unambiguous. Null keys never match.
pub struct JoinExec {
    left: Arc<dyn PhysicalPlan>,
    right: Arc<dyn PhysicalPlan>,
    left_key: usize,
    right_key: usize,
    schema: Arc<Schema>,
}

impl JoinExec {
    /// Build a join, validating key indices, key types and output names.
    pub fn try_new(
        left: Arc<dyn PhysicalPlan>,
        right: Arc<dyn PhysicalPlan>,
        left_key: usize,
        right_key: usize,
    ) -> Result<Self, QueryError> {
        let ls = left.schema();
        let rs = right.schema();
        let lf = ls
            .field(left_key)
            .ok_or_else(|| QueryError::MissingColumn {
                column: format!("index {left_key}"),
                available: names(ls),
            })?;
        let rf = rs
            .field(right_key)
            .ok_or_else(|| QueryError::MissingColumn {
                column: format!("index {right_key}"),
                available: names(rs),
            })?;
        if lf.data_type != rf.data_type {
            return Err(QueryError::TypeMismatch {
                column: lf.name.clone(),
                expected: lf.data_type.name(),
                actual: rf.data_type.name(),
                context: format!(
                    "join keys '{}' ({}) and '{}' ({}) must have the same type",
                    lf.name, lf.data_type, rf.name, rf.data_type
                ),
            });
        }

        let mut fields = ls.fields.clone();
        fields.extend(rs.fields.iter().cloned());
        let mut seen = HashSet::new();
        for f in &fields {
            if !seen.insert(f.name.as_str()) {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "join output would contain two columns named '{}'; rename one side or qualify the columns",
                        f.name
                    ),
                });
            }
        }

        Ok(Self {
            left,
            right,
            left_key,
            right_key,
            schema: Arc::new(Schema::new(fields)),
        })
    }
}

impl PhysicalPlan for JoinExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let left_batches = self.left.execute()?;
        let right_batches = self.right.execute()?;

        // Hash the right side by key, recording (batch, row) positions.
        let mut index: HashMap<Scalar, Vec<(usize, usize)>> = HashMap::new();
        for (bi, batch) in right_batches.iter().enumerate() {
            for row in 0..batch.row_count() {
                let key = batch.columns[self.right_key]
                    .get(row)
                    .unwrap_or(Scalar::Null);
                if matches!(key, Scalar::Null) {
                    continue;
                }
                index.entry(key).or_default().push((bi, row));
            }
        }

        // Probe with the left side, collecting matched positions on both sides.
        let mut left_pos: Vec<(usize, usize)> = Vec::new();
        let mut right_pos: Vec<(usize, usize)> = Vec::new();
        for (lbi, lb) in left_batches.iter().enumerate() {
            for row in 0..lb.row_count() {
                let key = lb.columns[self.left_key].get(row).unwrap_or(Scalar::Null);
                if matches!(key, Scalar::Null) {
                    continue;
                }
                if let Some(hits) = index.get(&key) {
                    for &(rbi, rrow) in hits {
                        left_pos.push((lbi, row));
                        right_pos.push((rbi, rrow));
                    }
                }
            }
        }

        let left_width = self.left.schema().len();
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(self.schema.len());
        for c in 0..left_width {
            let dt = self.schema.fields[c].data_type;
            columns.push(gather(&left_batches, c, &left_pos, dt));
        }
        for c in 0..self.right.schema().len() {
            let dt = self.schema.fields[left_width + c].data_type;
            columns.push(gather(&right_batches, c, &right_pos, dt));
        }

        Ok(vec![Arc::new(Batch::new_unchecked(
            self.schema.clone(),
            columns,
        ))])
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let lname = self
            .left
            .schema()
            .field(self.left_key)
            .map_or("?", |f| f.name.as_str());
        let rname = self
            .right
            .schema()
            .field(self.right_key)
            .map_or("?", |f| f.name.as_str());
        push_line(out, depth, &format!("HashJoin on {lname} = {rname}"));
        self.left.explain(depth + 1, out);
        self.right.explain(depth + 1, out);
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

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let batches = self.input.execute()?;
        for se in &self.sort_exprs {
            if self.schema.field(se.column).is_none() {
                return Err(internal("sort column out of range"));
            }
        }

        let positions: Vec<(usize, usize)> = batches
            .iter()
            .enumerate()
            .flat_map(|(bi, b)| (0..b.row_count()).map(move |r| (bi, r)))
            .collect();

        // Extract each sort key once, so comparisons do not re-read arrays.
        let keys: Vec<Vec<Scalar>> = self
            .sort_exprs
            .iter()
            .map(|se| {
                positions
                    .iter()
                    .map(|&(b, r)| batches[b].columns[se.column].get(r).unwrap_or(Scalar::Null))
                    .collect()
            })
            .collect();

        let mut order: Vec<usize> = (0..positions.len()).collect();
        order.sort_by(|&a, &b| {
            for (k, se) in self.sort_exprs.iter().enumerate() {
                // `sort_cmp` gives NULL a deterministic position (first when
                // ascending) instead of comparing it equal to non-null values.
                let ord = sort_cmp(&keys[k][a], &keys[k][b]);
                let ord = if se.desc { ord.reverse() } else { ord };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        });

        let sorted: Vec<(usize, usize)> = order.iter().map(|&i| positions[i]).collect();
        let columns: Vec<ArrayRef> = self
            .schema
            .fields
            .iter()
            .enumerate()
            .map(|(c, f)| gather(&batches, c, &sorted, f.data_type))
            .collect();

        Ok(vec![Arc::new(Batch::new_unchecked(
            self.schema.clone(),
            columns,
        ))])
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let parts: Vec<String> = self
            .sort_exprs
            .iter()
            .map(|se| {
                let name = self
                    .schema
                    .field(se.column)
                    .map_or("?", |f| f.name.as_str());
                format!("{name} {}", if se.desc { "DESC" } else { "ASC" })
            })
            .collect();
        push_line(out, depth, &format!("Sort [{}]", parts.join(", ")));
        self.input.explain(depth + 1, out);
    }
}

// === Limit ===

/// Skip `offset` rows, then keep at most `n`.
pub struct LimitExec {
    input: Arc<dyn PhysicalPlan>,
    n: usize,
    offset: usize,
    schema: Arc<Schema>,
}

impl LimitExec {
    pub fn new(input: Arc<dyn PhysicalPlan>, n: usize) -> Self {
        Self::with_offset(input, 0, n)
    }

    pub fn with_offset(input: Arc<dyn PhysicalPlan>, offset: usize, n: usize) -> Self {
        let schema = Arc::new(input.schema().clone());
        Self {
            input,
            n,
            offset,
            schema,
        }
    }
}

impl PhysicalPlan for LimitExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let batches = self.input.execute()?;
        let mut out = Vec::new();
        let mut to_skip = self.offset;
        let mut remaining = self.n;
        for batch in batches {
            if remaining == 0 {
                break;
            }
            let rows = batch.row_count();
            let skip = to_skip.min(rows);
            to_skip -= skip;
            let available = rows - skip;
            if available == 0 {
                continue;
            }
            let take = available.min(remaining);
            remaining -= take;
            if skip == 0 && take == rows {
                out.push(batch);
                continue;
            }
            let indices: Vec<usize> = (skip..skip + take).collect();
            let columns = batch.columns.iter().map(|c| c.take(&indices)).collect();
            out.push(Arc::new(Batch::new_unchecked(self.schema.clone(), columns)));
        }
        if out.is_empty() {
            out.push(empty_batch(&self.schema));
        }
        Ok(out)
    }

    fn explain(&self, depth: usize, out: &mut String) {
        push_line(
            out,
            depth,
            &format!("Limit {} offset {}", self.n, self.offset),
        );
        self.input.explain(depth + 1, out);
    }
}
