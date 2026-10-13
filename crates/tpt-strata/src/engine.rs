use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;

use crate::array::{empty_array, ArrayRef, BoolArray, F64Array, I32Array, I64Array, StrArray};
use crate::batch::{Batch, BatchRef};
use crate::error::QueryError;
use crate::schema::{DataType, Field, Schema};
use crate::types::{numeric_cmp, Scalar};

pub use crate::aggregate::{AggFunc, Aggregate, AggregateExec};
use crate::key::{key_at, FastMap, KeyPart};

/// Batches pulled one at a time from a plan. Dropping the stream stops the
/// work upstream, which is how `LIMIT` avoids reading its whole input.
pub type BatchStream = Box<dyn Iterator<Item = Result<BatchRef, QueryError>> + Send>;

/// A physical plan node that can be executed to produce batches.
///
/// Two entry points, one of which each operator implements:
///
/// - [`open`](PhysicalPlan::open) yields batches lazily. Scan, filter,
///   project and limit implement it, so a `LIMIT` above them stops reading
///   input once it has enough rows.
/// - [`execute`](PhysicalPlan::execute) returns every batch at once. Sort,
///   join and aggregate need all their input, so they implement it. Its
///   default collects `open`, and always returns at least one batch, empty if
///   there are no rows.
pub trait PhysicalPlan: Send + Sync {
    fn schema(&self) -> &Schema;

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let mut out = self.open()?.collect::<Result<Vec<_>, _>>()?;
        if out.is_empty() {
            out.push(empty_batch(&Arc::new(self.schema().clone())));
        }
        Ok(out)
    }

    /// Pull batches lazily. The default runs [`execute`](PhysicalPlan::execute)
    /// and yields its batches, for operators that cannot stream.
    fn open(&self) -> Result<BatchStream, QueryError> {
        let batches = self.execute()?;
        Ok(Box::new(batches.into_iter().map(Ok)))
    }
    /// Write this operator and its inputs as an indented tree, one operator
    /// per line, root first.
    fn explain(&self, depth: usize, out: &mut String) {
        push_line(out, depth, "Operator");
    }
}

pub(crate) fn push_line(out: &mut String, depth: usize, text: &str) {
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

pub(crate) fn names(schema: &Schema) -> Vec<String> {
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
pub(crate) fn gather(
    batches: &[BatchRef],
    col: usize,
    pos: &[(usize, usize)],
    dt: DataType,
) -> ArrayRef {
    gather_opt(batches, col, pos.iter().map(|&p| Some(p)), dt)
}

/// As [`gather`], but a `None` position yields a NULL row.
pub(crate) fn gather_opt<I>(batches: &[BatchRef], col: usize, pos: I, dt: DataType) -> ArrayRef
where
    I: IntoIterator<Item = Option<(usize, usize)>>,
{
    macro_rules! take_typed {
        ($variant:ident, $arr:ident) => {{
            // Resolve each batch's typed array once, not once per element.
            let parts: Vec<Option<&$arr>> = batches
                .iter()
                .map(|b| match &*b.columns[col] {
                    ArrayRef::$variant(a) => Some(a),
                    _ => None,
                })
                .collect();
            let vals = pos
                .into_iter()
                .map(|p| match p {
                    Some((b, r)) => parts[b].and_then(|a| a.0.get(r).cloned().flatten()),
                    None => None,
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
        DataType::Date => take_typed!(Date, I32Array),
        DataType::Timestamp => take_typed!(Timestamp, I64Array),
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

    fn open(&self) -> Result<BatchStream, QueryError> {
        Ok(Box::new(self.batches.clone().into_iter().map(Ok)))
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
        (Scalar::Date(x), Scalar::Date(y)) => Some(x.cmp(y)),
        (Scalar::Timestamp(x), Scalar::Timestamp(y)) => Some(x.cmp(y)),
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

/// The literal as a typed key, for the fast filter path. Floats and NULL are
/// left out: NaN is unordered in SQL but totally ordered as a key, so floats
/// keep the general path.
fn literal_key(v: &Scalar) -> Option<KeyPart<'_>> {
    match v {
        Scalar::Bool(b) => Some(KeyPart::Bool(*b)),
        Scalar::I32(n) => Some(KeyPart::I32(*n)),
        Scalar::I64(n) => Some(KeyPart::I64(*n)),
        Scalar::Str(s) => Some(KeyPart::Str(s)),
        Scalar::Date(n) => Some(KeyPart::I32(*n)),
        Scalar::Timestamp(n) => Some(KeyPart::I64(*n)),
        Scalar::F64(_) | Scalar::Null => None,
    }
}

/// The rows in `rows` (or every row below `n`) for which `keep` holds, in order.
fn for_rows(n: usize, rows: Option<&[usize]>, mut keep: impl FnMut(usize) -> bool) -> Vec<usize> {
    match rows {
        None => (0..n).filter(|&row| keep(row)).collect(),
        Some(rows) => rows.iter().copied().filter(|&row| keep(row)).collect(),
    }
}

impl Predicate {
    /// The rows of `batch` that pass, in order.
    ///
    /// When the column and the literal have the same type, comparisons run on
    /// typed keys with no per-row `Scalar`. Otherwise each row goes through
    /// [`Predicate::matches`], which has the full SQL rules.
    pub fn select(&self, batch: &Batch) -> Vec<usize> {
        self.select_from(batch, None)
    }

    /// As [`Predicate::select`], over only the candidate rows `rows`, in order.
    /// `None` means every row. This lets a chain of predicates narrow one
    /// selection without copying the batch between them.
    pub fn select_from(&self, batch: &Batch, rows: Option<&[usize]>) -> Vec<usize> {
        let column = batch.columns.get(self.column);

        // A Float64 column against a Float64 literal compares the values
        // directly. IEEE ordering already makes NaN unordered (false), and
        // `!=` is made to agree with `compare_values`, which is also false.
        let float_column = match column.map(|c| &**c) {
            Some(ArrayRef::Float64(arr)) => Some(arr),
            _ => None,
        };
        if let (Some(arr), Scalar::F64(lit)) = (float_column, &self.value) {
            let op = self.op;
            if matches!(
                op,
                Comparison::Eq
                    | Comparison::NotEq
                    | Comparison::Gt
                    | Comparison::Gte
                    | Comparison::Lt
                    | Comparison::Lte
            ) {
                let lit = *lit;
                return for_rows(batch.row_count(), rows, |row| {
                    match arr.0.get(row).copied().flatten() {
                        Some(x) if !x.is_nan() && !lit.is_nan() => match op {
                            Comparison::Eq => x == lit,
                            Comparison::NotEq => x != lit,
                            Comparison::Gt => x > lit,
                            Comparison::Gte => x >= lit,
                            Comparison::Lt => x < lit,
                            _ => x <= lit,
                        },
                        _ => false,
                    }
                });
            }
        }

        let typed = match self.op {
            Comparison::IsNull | Comparison::IsNotNull | Comparison::Like | Comparison::NotLike => {
                None
            }
            _ => literal_key(&self.value),
        };
        for_rows(batch.row_count(), rows, |row| {
            if let (Some(lit), Some(col)) = (typed, column) {
                let key = key_at(col, row);
                // Same discriminant means same column type. A NULL key never
                // matches, which the fallback path also returns.
                if std::mem::discriminant(&key) == std::mem::discriminant(&lit) {
                    let ord = key.cmp(&lit);
                    return match self.op {
                        Comparison::Eq => ord == Ordering::Equal,
                        Comparison::NotEq => ord != Ordering::Equal,
                        Comparison::Gt => ord == Ordering::Greater,
                        Comparison::Gte => ord != Ordering::Less,
                        Comparison::Lt => ord == Ordering::Less,
                        Comparison::Lte => ord != Ordering::Greater,
                        _ => self.matches(batch, row),
                    };
                }
            }
            self.matches(batch, row)
        })
    }

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

/// Keep the rows of its input that satisfy every predicate.
///
/// The predicates are applied in one pass: each one narrows the selection left
/// by the previous one, and the columns are gathered once at the end.
pub struct FilterExec {
    input: Arc<dyn PhysicalPlan>,
    predicates: Vec<Predicate>,
    schema: Arc<Schema>,
}

impl FilterExec {
    pub fn new(input: Arc<dyn PhysicalPlan>, predicate: Predicate) -> Self {
        Self::new_all(input, vec![predicate])
    }

    /// A filter that keeps rows passing every predicate in `predicates`. With
    /// no predicates every row passes.
    pub fn new_all(input: Arc<dyn PhysicalPlan>, predicates: Vec<Predicate>) -> Self {
        let schema = Arc::new(input.schema().clone());
        Self {
            input,
            predicates,
            schema,
        }
    }
}

impl PhysicalPlan for FilterExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn open(&self) -> Result<BatchStream, QueryError> {
        let predicates = self.predicates.clone();
        let schema = self.schema.clone();
        Ok(Box::new(self.input.open()?.map(move |batch| {
            let batch = batch?;
            let mut selection: Option<Vec<usize>> = None;
            for predicate in &predicates {
                selection = Some(predicate.select_from(&batch, selection.as_deref()));
            }
            let keep = selection.unwrap_or_else(|| (0..batch.row_count()).collect());
            let columns: Vec<ArrayRef> = batch.columns.iter().map(|c| c.take(&keep)).collect();
            Ok(Arc::new(Batch::new_unchecked(schema.clone(), columns)))
        })))
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let parts: Vec<String> = self
            .predicates
            .iter()
            .map(|p| {
                let column = self.schema.field(p.column).map_or("?", |f| f.name.as_str());
                match p.op {
                    Comparison::IsNull | Comparison::IsNotNull => {
                        format!("{column} {}", p.op.symbol())
                    }
                    _ => format!("{column} {} {}", p.op.symbol(), p.value),
                }
            })
            .collect();
        let text = if parts.is_empty() {
            "Filter (none)".to_string()
        } else {
            format!("Filter {}", parts.join(" AND "))
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

    fn open(&self) -> Result<BatchStream, QueryError> {
        let exprs = self.exprs.clone();
        let schema = self.schema.clone();
        Ok(Box::new(self.input.open()?.map(move |batch| {
            let batch = batch?;
            let columns = exprs
                .iter()
                .map(|e| {
                    batch
                        .columns
                        .get(e.column)
                        .cloned()
                        .ok_or_else(|| internal("projection column out of range"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Arc::new(Batch::new_shared(schema.clone(), columns)))
        })))
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let cols: Vec<&str> = self.schema.fields.iter().map(|f| f.name.as_str()).collect();
        push_line(out, depth, &format!("Project [{}]", cols.join(", ")));
        self.input.explain(depth + 1, out);
    }
}

// === Join ===

/// Which rows of an equi-join are kept when they have no match on the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// Only matched pairs.
    Inner,
    /// Every left row; unmatched ones get NULLs on the right.
    Left,
    /// Every right row; unmatched ones get NULLs on the left.
    Right,
    /// Every row from both sides; unmatched ones get NULLs on the other side.
    Full,
}

impl JoinKind {
    fn keeps_left(self) -> bool {
        matches!(self, JoinKind::Left | JoinKind::Full)
    }

    fn keeps_right(self) -> bool {
        matches!(self, JoinKind::Right | JoinKind::Full)
    }
}

/// Equi-join (hash join) on one key column per side.
///
/// Output columns are the left schema followed by the right schema. The two
/// sides must not share a column name, because the output names must be
/// unambiguous. Null keys never match. Matched rows come first, in left-row
/// order; unmatched rows of an outer join follow, so output order is not
/// the input order.
pub struct JoinExec {
    left: Arc<dyn PhysicalPlan>,
    right: Arc<dyn PhysicalPlan>,
    left_key: usize,
    right_key: usize,
    schema: Arc<Schema>,
    kind: JoinKind,
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
            kind: JoinKind::Inner,
        })
    }

    /// As [`JoinExec::try_new`], but with the join kind chosen: `outer` keeps
    /// unmatched left rows (LEFT JOIN). See [`JoinExec::try_new_join`].
    pub fn try_new_kind(
        left: Arc<dyn PhysicalPlan>,
        right: Arc<dyn PhysicalPlan>,
        left_key: usize,
        right_key: usize,
        outer: bool,
    ) -> Result<Self, QueryError> {
        let kind = if outer {
            JoinKind::Left
        } else {
            JoinKind::Inner
        };
        Self::try_new_join(left, right, left_key, right_key, kind)
    }

    /// As [`JoinExec::try_new`], with any [`JoinKind`]. The columns that can be
    /// NULL-padded for the chosen kind become nullable in the output schema.
    pub fn try_new_join(
        left: Arc<dyn PhysicalPlan>,
        right: Arc<dyn PhysicalPlan>,
        left_key: usize,
        right_key: usize,
        kind: JoinKind,
    ) -> Result<Self, QueryError> {
        let mut join = Self::try_new(left, right, left_key, right_key)?;
        join.kind = kind;
        let left_width = join.left.schema().len();
        let mut fields = join.schema.fields.clone();
        if kind.keeps_left() {
            for f in fields.iter_mut().skip(left_width) {
                f.nullable = true;
            }
        }
        if kind.keeps_right() {
            for f in fields.iter_mut().take(left_width) {
                f.nullable = true;
            }
        }
        join.schema = Arc::new(Schema::new(fields));
        Ok(join)
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
        // Keys are borrowed from the batches, so string keys are not copied.
        let mut index: FastMap<KeyPart<'_>, Vec<(usize, usize)>> = FastMap::default();
        for (bi, batch) in right_batches.iter().enumerate() {
            for row in 0..batch.row_count() {
                let key = key_at(&batch.columns[self.right_key], row);
                if key == KeyPart::Null {
                    continue;
                }
                index.entry(key).or_default().push((bi, row));
            }
        }

        // Probe with the left side, collecting matched positions on both sides.
        // A `None` position is a row with no match on that side (outer joins).
        // `right_matched[bi][row]` records which right rows found a partner, so
        // a RIGHT or FULL join can add the rest afterwards.
        let mut right_matched: Vec<Vec<bool>> = right_batches
            .iter()
            .map(|b| vec![false; b.row_count()])
            .collect();
        // Sized for one output row per left row: exact for key joins, and a
        // good start for the rest.
        let mut left_pos: Vec<Option<(usize, usize)>> =
            Vec::with_capacity(left_batches.iter().map(|b| b.row_count()).sum());
        let mut right_pos: Vec<Option<(usize, usize)>> =
            Vec::with_capacity(left_batches.iter().map(|b| b.row_count()).sum());
        for (lbi, lb) in left_batches.iter().enumerate() {
            for row in 0..lb.row_count() {
                let key = key_at(&lb.columns[self.left_key], row);
                let hits = if key == KeyPart::Null {
                    None
                } else {
                    index.get(&key)
                };
                match hits {
                    Some(hits) if !hits.is_empty() => {
                        for &(rbi, rrow) in hits {
                            right_matched[rbi][rrow] = true;
                            left_pos.push(Some((lbi, row)));
                            right_pos.push(Some((rbi, rrow)));
                        }
                    }
                    _ if self.kind.keeps_left() => {
                        left_pos.push(Some((lbi, row)));
                        right_pos.push(None);
                    }
                    _ => {}
                }
            }
        }
        if self.kind.keeps_right() {
            for (rbi, matched_rows) in right_matched.iter().enumerate() {
                for (row, &matched) in matched_rows.iter().enumerate() {
                    if !matched {
                        left_pos.push(None);
                        right_pos.push(Some((rbi, row)));
                    }
                }
            }
        }

        let left_width = self.left.schema().len();
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(self.schema.len());
        for c in 0..left_width {
            let dt = self.schema.fields[c].data_type;
            columns.push(gather_opt(&left_batches, c, left_pos.iter().copied(), dt));
        }
        for c in 0..self.right.schema().len() {
            let dt = self.schema.fields[left_width + c].data_type;
            columns.push(gather_opt(&right_batches, c, right_pos.iter().copied(), dt));
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
        let kind = match self.kind {
            JoinKind::Inner => "",
            JoinKind::Left => "Left ",
            JoinKind::Right => "Right ",
            JoinKind::Full => "Full ",
        };
        push_line(out, depth, &format!("{kind}HashJoin on {lname} = {rname}"));
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
    /// Keep only the first `keep` rows in sort order (top-k), when a `LIMIT`
    /// sits above the sort. `None` keeps every row.
    keep: Option<usize>,
}

impl SortExec {
    pub fn new(input: Arc<dyn PhysicalPlan>, sort_exprs: Vec<SortExpr>) -> Self {
        let schema = Arc::new(input.schema().clone());
        Self {
            input,
            sort_exprs,
            schema,
            keep: None,
        }
    }

    /// Keep only the first `keep` rows in sort order. A `LIMIT` above this
    /// sort with `n` rows and `offset` skipped passes `Some(offset + n)`.
    /// Ties keep their input order, exactly as a full sort would.
    pub fn with_keep(mut self, keep: Option<usize>) -> Self {
        self.keep = keep;
        self
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
        // Typed keys borrowed from the batches; NULL sorts first, as before.
        let keys: Vec<Vec<KeyPart<'_>>> = self
            .sort_exprs
            .iter()
            .map(|se| {
                positions
                    .iter()
                    .map(|&(b, r)| key_at(&batches[b].columns[se.column], r))
                    .collect()
            })
            .collect();

        // Ties fall back to input position, so the order is total and equal to
        // a stable sort. That also makes the top-k selection below exact.
        let cmp = |a: &usize, b: &usize| {
            for (k, se) in self.sort_exprs.iter().enumerate() {
                // `sort_cmp` gives NULL a deterministic position (first when
                // ascending) instead of comparing it equal to non-null values.
                let ord = keys[k][*a].cmp(&keys[k][*b]);
                let ord = if se.desc { ord.reverse() } else { ord };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            a.cmp(b)
        };
        let mut order: Vec<usize> = (0..positions.len()).collect();
        match self.keep {
            // Top-k: partition out the first `k` rows in O(n), then sort only those.
            Some(k) if k < order.len() => {
                if k == 0 {
                    order.clear();
                } else {
                    order.select_nth_unstable_by(k - 1, cmp);
                    order.truncate(k);
                    order.sort_by(cmp);
                }
            }
            _ => order.sort_by(cmp),
        }

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

    fn open(&self) -> Result<BatchStream, QueryError> {
        let mut input = self.input.open()?;
        let schema = self.schema.clone();
        let mut to_skip = self.offset;
        let mut remaining = self.n;
        Ok(Box::new(std::iter::from_fn(move || loop {
            // Once the limit is met, stop without pulling from the input again.
            if remaining == 0 {
                return None;
            }
            let batch = match input.next()? {
                Ok(batch) => batch,
                Err(e) => return Some(Err(e)),
            };
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
                return Some(Ok(batch));
            }
            let indices: Vec<usize> = (skip..skip + take).collect();
            let columns: Vec<ArrayRef> = batch.columns.iter().map(|c| c.take(&indices)).collect();
            return Some(Ok(Arc::new(Batch::new_unchecked(schema.clone(), columns))));
        })))
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

// === Conditions (AND / OR over comparisons) ===

/// A boolean condition over the columns of one batch: a comparison with a
/// literal, a comparison between two columns, or an AND/OR of conditions.
///
/// There is no NOT, so a comparison that is unknown (NULL) can be treated as
/// false when filtering. That gives the same rows as SQL's three-valued logic.
#[derive(Debug, Clone)]
pub enum Condition {
    Cmp(Predicate),
    ColCmp {
        left: usize,
        op: Comparison,
        right: usize,
    },
    And(Vec<Condition>),
    Or(Vec<Condition>),
}

impl Condition {
    /// Whether `row` of `batch` satisfies the condition.
    pub fn matches(&self, batch: &Batch, row: usize) -> bool {
        match self {
            Condition::Cmp(p) => p.matches(batch, row),
            Condition::ColCmp { left, op, right } => {
                let a = batch.columns.get(*left).and_then(|c| c.get(row));
                let b = batch.columns.get(*right).and_then(|c| c.get(row));
                match (a, b) {
                    (Some(a), Some(b)) => compare_values(&a, &b, *op),
                    _ => false,
                }
            }
            Condition::And(parts) => parts.iter().all(|c| c.matches(batch, row)),
            Condition::Or(parts) => parts.iter().any(|c| c.matches(batch, row)),
        }
    }
}

/// Keep the rows of its input that satisfy a [`Condition`].
pub struct CondFilterExec {
    input: Arc<dyn PhysicalPlan>,
    condition: Condition,
    schema: Arc<Schema>,
}

impl CondFilterExec {
    pub fn new(input: Arc<dyn PhysicalPlan>, condition: Condition) -> Self {
        let schema = Arc::new(input.schema().clone());
        Self {
            input,
            condition,
            schema,
        }
    }
}

impl PhysicalPlan for CondFilterExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn open(&self) -> Result<BatchStream, QueryError> {
        let condition = self.condition.clone();
        let schema = self.schema.clone();
        Ok(Box::new(self.input.open()?.map(move |batch| {
            let batch = batch?;
            let keep: Vec<usize> = (0..batch.row_count())
                .filter(|&row| condition.matches(&batch, row))
                .collect();
            let columns: Vec<ArrayRef> = batch.columns.iter().map(|c| c.take(&keep)).collect();
            Ok(Arc::new(Batch::new_unchecked(schema.clone(), columns)))
        })))
    }

    fn explain(&self, depth: usize, out: &mut String) {
        push_line(out, depth, "Filter (AND/OR condition)");
        self.input.explain(depth + 1, out);
    }
}
