use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;

use crate::array::{empty_array, ArrayRef, BoolArray, F64Array, I32Array, I64Array, StrArray};
use crate::batch::{Batch, BatchRef};
use crate::error::QueryError;
use crate::schema::{DataType, Field, Schema};
use crate::types::{numeric_cmp, Scalar};

use crate::aggregate::{segments, Segment};
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
                    Some((b, r)) => parts[b].and_then(|a| a.value(r)),
                    None => None,
                })
                .collect();
            ArrayRef::$variant($arr::from_nulls(vals))
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

/// A `(batch, row)` position packed into one word, for join output. `NO_ROW`
/// marks a row with no partner on that side. A packed position is 8 bytes,
/// where `Option<(usize, usize)>` is 24.
const NO_ROW: u64 = u64::MAX;

fn pack(batch: usize, row: usize) -> u64 {
    ((batch as u64) << 32) | row as u64
}

/// Like [`gather_opt`], over packed positions. `NO_ROW` gives a NULL.
fn gather_packed(batches: &[BatchRef], col: usize, pos: &[u64], dt: DataType) -> ArrayRef {
    macro_rules! take_typed {
        ($variant:ident, $arr:ident) => {{
            let parts: Vec<Option<&$arr>> = batches
                .iter()
                .map(|b| match &*b.columns[col] {
                    ArrayRef::$variant(a) => Some(a),
                    _ => None,
                })
                .collect();
            ArrayRef::$variant($arr::gather_positions(&parts, pos, NO_ROW))
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

/// A `LIKE` pattern classified once, so the common shapes skip the general
/// matcher: `abc`, `abc%`, `%abc` and `%abc%`. Anything with `_` or an inner `%`
/// is `General`.
enum LikeKind<'a> {
    Exact(&'a str),
    Prefix(&'a str),
    Suffix(&'a str),
    Contains(&'a str),
    General,
}

fn like_kind(pattern: &str) -> LikeKind<'_> {
    let body = pattern.trim_matches('%');
    if body.contains(['%', '_']) {
        return LikeKind::General;
    }
    match (pattern.starts_with('%'), pattern.ends_with('%')) {
        (true, true) => LikeKind::Contains(body),
        (false, true) => LikeKind::Prefix(body),
        (true, false) => LikeKind::Suffix(body),
        (false, false) => LikeKind::Exact(body),
    }
}

/// Whether `text` matches a pattern already classified by [`like_kind`].
fn like_matches(kind: &LikeKind<'_>, text: &str, pattern: &str) -> bool {
    match kind {
        LikeKind::Exact(b) => text == *b,
        LikeKind::Prefix(b) => text.starts_with(b),
        LikeKind::Suffix(b) => text.ends_with(b),
        LikeKind::Contains(b) => text.contains(b),
        LikeKind::General => like_match(text, pattern),
    }
}

/// Match `text` against a SQL `LIKE` pattern (`%` any run, `_` one char).
///
/// Works on UTF-8 bytes, so it allocates nothing. A `_` or a `%` step moves
/// over whole characters. A literal pattern character is matched byte by byte,
/// which is exact because UTF-8 is self-synchronising.
pub(crate) fn like_match(text: &str, pattern: &str) -> bool {
    let (t, p) = (text.as_bytes(), pattern.as_bytes());
    // Byte width of the character that starts at `i` in `t`.
    let width = |i: usize| match t[i] {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    };
    let (mut ti, mut pi) = (0usize, 0usize);
    // Position of the last `%` seen, and the text index it currently absorbs up to.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == b'%' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == b'_' {
            ti += width(ti);
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            ti += 1;
            pi += 1;
        } else if let Some((sp, st)) = star {
            // Let the last `%` absorb one more character and retry.
            let next = st + width(st);
            pi = sp + 1;
            star = Some((sp, next));
            ti = next;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'%' {
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
fn for_rows(n: usize, rows: Option<&[usize]>, keep: impl Fn(usize) -> bool + Sync) -> Vec<usize> {
    match rows {
        None => {
            #[cfg(feature = "parallel")]
            if let Some(out) = for_all_rows_parallel(n, &keep) {
                return out;
            }
            compact(0..n, &keep)
        }
        Some(rows) => rows.iter().copied().filter(|&row| keep(row)).collect(),
    }
}

/// Branch-free compaction over `range`: every row index is written, and the
/// output length advances only on a match. A selective filter over random data
/// mispredicts a branch on about half the rows, so this is much faster.
fn compact(range: std::ops::Range<usize>, keep: &impl Fn(usize) -> bool) -> Vec<usize> {
    let mut out = vec![0usize; range.len()];
    let mut k = 0;
    for row in range {
        out[k] = row;
        k += usize::from(keep(row));
    }
    out.truncate(k);
    out
}

/// Compact all rows below `n` on worker threads, one contiguous chunk each,
/// and join the chunks in order. Small inputs stay on one thread, where the
/// thread start-up cost would outweigh the work.
#[cfg(feature = "parallel")]
fn for_all_rows_parallel(n: usize, keep: &(impl Fn(usize) -> bool + Sync)) -> Option<Vec<usize>> {
    const MIN_ROWS: usize = 1 << 18;
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get());
    if threads < 2 || n < MIN_ROWS {
        return None;
    }
    let per = n.div_ceil(threads);
    let parts: Vec<Vec<usize>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..n)
            .step_by(per)
            .map(|start| {
                let end = (start + per).min(n);
                scope.spawn(move || compact(start..end, keep))
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    Some(parts.concat())
}

/// The rows of a batch that pass every predicate, in order. Large batches are
/// split across the pool; each job owns its input, so it can outlive the call.
#[cfg(feature = "parallel")]
fn filter_rows(batch: &BatchRef, predicates: &Arc<Vec<Predicate>>) -> Vec<usize> {
    const MIN_ROWS: usize = 1 << 18;
    let n = batch.row_count();
    let threads = crate::pool::threads();
    if threads < 2 || n < MIN_ROWS {
        return filter_rows_here(batch, predicates);
    }
    let per = n.div_ceil(threads);
    let jobs: Vec<Box<dyn FnOnce() -> Vec<usize> + Send>> = (0..n)
        .step_by(per)
        .map(|start| {
            let end = (start + per).min(n);
            let batch = Arc::clone(batch);
            let predicates = Arc::clone(predicates);
            Box::new(move || {
                let tests: Vec<RowTest<'_>> =
                    predicates.iter().map(|p| RowTest::new(p, &batch)).collect();
                (start..end)
                    .filter(|&row| tests.iter().all(|t| t.test(row)))
                    .collect()
            }) as Box<dyn FnOnce() -> Vec<usize> + Send>
        })
        .collect();
    crate::pool::run(jobs).concat()
}

/// The rows of a batch that pass every predicate, on the calling thread.
#[cfg(not(feature = "parallel"))]
fn filter_rows(batch: &BatchRef, predicates: &Arc<Vec<Predicate>>) -> Vec<usize> {
    filter_rows_here(batch, predicates)
}

/// The rows of a batch that pass every predicate, on the calling thread.
fn filter_rows_here(batch: &BatchRef, predicates: &Arc<Vec<Predicate>>) -> Vec<usize> {
    let tests: Vec<RowTest<'_>> = predicates.iter().map(|p| RowTest::new(p, batch)).collect();
    sparse_rows(batch.row_count(), |row| tests.iter().all(|t| t.test(row)))
}

/// The rows of `n` for which `keep` holds, in order. Built by pushing, so the
/// output is only as large as the matches. A conjunction of filters is sparse,
/// so this is cheaper than writing every row index.
fn sparse_rows(n: usize, keep: impl Fn(usize) -> bool + Sync) -> Vec<usize> {
    #[cfg(feature = "parallel")]
    if let Some(out) = sparse_rows_parallel(n, &keep) {
        return out;
    }
    (0..n).filter(|&row| keep(row)).collect()
}

/// As [`sparse_rows`], on worker threads over contiguous chunks, joined in order.
#[cfg(feature = "parallel")]
fn sparse_rows_parallel(n: usize, keep: &(impl Fn(usize) -> bool + Sync)) -> Option<Vec<usize>> {
    const MIN_ROWS: usize = 1 << 18;
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get());
    if threads < 2 || n < MIN_ROWS {
        return None;
    }
    let per = n.div_ceil(threads);
    let parts: Vec<Vec<usize>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..n)
            .step_by(per)
            .map(|start| {
                let end = (start + per).min(n);
                scope.spawn(move || (start..end).filter(|&row| keep(row)).collect::<Vec<_>>())
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    Some(parts.concat())
}

/// One predicate's test over one batch. The typed fast path is chosen once,
/// from the column type and the literal, so `test` does no per-row choice
/// between them. It gives the same answer as [`Predicate::matches`].
pub(crate) struct RowTest<'a> {
    predicate: &'a Predicate,
    batch: &'a Batch,
    plan: Plan<'a>,
}

enum Plan<'a> {
    /// A Float64 column against a Float64 literal.
    Float(&'a F64Array, f64),
    /// A Utf8 column against a LIKE pattern: the pattern, its kind, and whether
    /// the test is LIKE (`true`) or NOT LIKE.
    Like(&'a StrArray, LikeKind<'a>, &'a str, bool),
    /// An Int64 column against an Int64 literal.
    Int(&'a I64Array, i64),
    /// Anything else: a typed key comparison, or the full SQL rules.
    Key(Option<&'a ArrayRef>, Option<KeyPart<'a>>),
}

impl<'a> RowTest<'a> {
    pub(crate) fn new(predicate: &'a Predicate, batch: &'a Batch) -> Self {
        let op = predicate.op;
        let column = batch.columns.get(predicate.column).map(|c| &**c);
        let numeric = matches!(
            op,
            Comparison::Eq
                | Comparison::NotEq
                | Comparison::Gt
                | Comparison::Gte
                | Comparison::Lt
                | Comparison::Lte
        );
        let plan = match (column, &predicate.value) {
            (Some(ArrayRef::Float64(arr)), Scalar::F64(lit)) if numeric => Plan::Float(arr, *lit),
            (Some(ArrayRef::Utf8(arr)), Scalar::Str(pattern))
                if matches!(op, Comparison::Like | Comparison::NotLike) =>
            {
                let pattern: &str = pattern;
                Plan::Like(arr, like_kind(pattern), pattern, op == Comparison::Like)
            }
            (Some(ArrayRef::Int64(arr)), Scalar::I64(lit)) if numeric => Plan::Int(arr, *lit),
            _ => {
                let typed = match op {
                    Comparison::IsNull
                    | Comparison::IsNotNull
                    | Comparison::Like
                    | Comparison::NotLike => None,
                    _ => literal_key(&predicate.value),
                };
                Plan::Key(column, typed)
            }
        };
        RowTest {
            predicate,
            batch,
            plan,
        }
    }

    /// Whether `row` passes the predicate.
    #[inline]
    pub(crate) fn test(&self, row: usize) -> bool {
        match &self.plan {
            Plan::Float(arr, lit) => {
                let lit = *lit;
                match arr.value(row) {
                    Some(x) if !x.is_nan() && !lit.is_nan() => match self.predicate.op {
                        Comparison::Eq => x == lit,
                        Comparison::NotEq => x != lit,
                        Comparison::Gt => x > lit,
                        Comparison::Gte => x >= lit,
                        Comparison::Lt => x < lit,
                        _ => x <= lit,
                    },
                    _ => false,
                }
            }
            Plan::Like(arr, kind, pattern, want) => match arr.value_ref(row) {
                Some(s) => like_matches(kind, s, pattern) == *want,
                None => false,
            },
            Plan::Int(arr, lit) => {
                let lit = *lit;
                match arr.value(row) {
                    Some(x) => match self.predicate.op {
                        Comparison::Eq => x == lit,
                        Comparison::NotEq => x != lit,
                        Comparison::Gt => x > lit,
                        Comparison::Gte => x >= lit,
                        Comparison::Lt => x < lit,
                        _ => x <= lit,
                    },
                    None => false,
                }
            }
            Plan::Key(column, typed) => {
                if let (Some(lit), Some(col)) = (*typed, *column) {
                    let key = key_at(col, row);
                    // Same discriminant means same column type. A NULL key never
                    // matches, which the fallback path also returns.
                    if std::mem::discriminant(&key) == std::mem::discriminant(&lit) {
                        let ord = key.cmp(&lit);
                        return match self.predicate.op {
                            Comparison::Eq => ord == Ordering::Equal,
                            Comparison::NotEq => ord != Ordering::Equal,
                            Comparison::Gt => ord == Ordering::Greater,
                            Comparison::Gte => ord != Ordering::Less,
                            Comparison::Lt => ord == Ordering::Less,
                            Comparison::Lte => ord != Ordering::Greater,
                            _ => self.predicate.matches(self.batch, row),
                        };
                    }
                }
                self.predicate.matches(self.batch, row)
            }
        }
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
        let test = RowTest::new(self, batch);
        for_rows(batch.row_count(), rows, |row| test.test(row))
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
    /// The input columns to output, in order. `None` outputs every column.
    keep: Option<Vec<usize>>,
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
            keep: None,
        }
    }

    /// Output only the input columns at `keep`, in that order. A column that
    /// only the predicates read is then not copied for the kept rows.
    pub fn with_keep(mut self, keep: Vec<usize>) -> Self {
        // No column read above the filter: one placeholder column carries the
        // row count, and it is never read.
        let fields = if keep.is_empty() {
            vec![Field::new("$rows", DataType::Int64, false)]
        } else {
            keep.iter()
                .map(|&c| self.input.schema().fields[c].clone())
                .collect()
        };
        self.schema = Arc::new(Schema::new(fields));
        self.keep = Some(keep);
        self
    }
}

impl PhysicalPlan for FilterExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn open(&self) -> Result<BatchStream, QueryError> {
        let predicates = self.predicates.clone();
        let schema = self.schema.clone();
        let keep_cols = self.keep.clone();
        let predicates = Arc::new(predicates);
        Ok(Box::new(self.input.open()?.map(move |batch| {
            let batch = batch?;
            // Every predicate is tested on each row in one pass, so no row list
            // is built for the first predicate and then narrowed by the second.
            let rows = if predicates.is_empty() {
                (0..batch.row_count()).collect()
            } else {
                filter_rows(&batch, &predicates)
            };
            let columns: Vec<ArrayRef> = match &keep_cols {
                Some(cols) if cols.is_empty() => {
                    vec![ArrayRef::Int64(I64Array::new(vec![0; rows.len()]))]
                }
                Some(cols) => cols.iter().map(|&c| batch.columns[c].take(&rows)).collect(),
                None => batch.columns.iter().map(|c| c.take(&rows)).collect(),
            };
            Ok(Arc::new(Batch::new_unchecked(schema.clone(), columns)))
        })))
    }

    fn explain(&self, depth: usize, out: &mut String) {
        let parts: Vec<String> = self
            .predicates
            .iter()
            .map(|p| {
                let column = self
                    .input
                    .schema()
                    .field(p.column)
                    .map_or("?", |f| f.name.as_str());
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

/// The result of probing some left segments: output positions in left order,
/// and which right rows found a partner.
struct Probe {
    left_pos: Vec<u64>,
    right_pos: Vec<u64>,
    right_matched: Vec<Vec<bool>>,
}

/// The right side of a join, indexed by key: each key maps to the
/// `(batch, row)` positions that carry it. Null keys are never stored.
enum JoinIndex<'a> {
    /// Any key type: a hash map from the borrowed key.
    Hash(FastMap<KeyPart<'a>, Vec<(usize, usize)>>),
    /// `Int64` keys in a small range (typical surrogate keys): the key is an
    /// offset into `slots`, so a probe does no hashing.
    Dense {
        min: i64,
        slots: Vec<Vec<(usize, usize)>>,
    },
}

impl JoinIndex<'_> {
    /// The right positions that match the left key at `row` of `col`. Empty
    /// for a NULL key or no match.
    #[inline]
    fn hits<'s>(&'s self, col: &'s ArrayRef, row: usize) -> &'s [(usize, usize)] {
        match self {
            JoinIndex::Hash(map) => {
                let key = key_at(col, row);
                if key == KeyPart::Null {
                    return &[];
                }
                match map.get(&key) {
                    Some(v) => v.as_slice(),
                    None => &[],
                }
            }
            JoinIndex::Dense { min, slots } => match col {
                ArrayRef::Int64(a) => match a.value(row) {
                    Some(v) => {
                        let off = v.wrapping_sub(*min) as u64;
                        if off < slots.len() as u64 {
                            slots[off as usize].as_slice()
                        } else {
                            &[]
                        }
                    }
                    None => &[],
                },
                _ => &[],
            },
        }
    }
}

/// Index the right side on `key`. Uses the direct layout when the key is an
/// `Int64` column whose values fit in a small range; otherwise hashes.
fn build_index(batches: &[BatchRef], key: usize) -> JoinIndex<'_> {
    if let Some(dense) = dense_index(batches, key) {
        return dense;
    }
    let mut index: FastMap<KeyPart<'_>, Vec<(usize, usize)>> = FastMap::default();
    for (bi, batch) in batches.iter().enumerate() {
        for row in 0..batch.row_count() {
            let k = key_at(&batch.columns[key], row);
            if k == KeyPart::Null {
                continue;
            }
            index.entry(k).or_default().push((bi, row));
        }
    }
    JoinIndex::Hash(index)
}

/// The direct layout, or `None` when the key is not `Int64` or its range is
/// too wide for a slot per value (more than four slots per right row, plus a
/// fixed allowance for small inputs).
fn dense_index(batches: &[BatchRef], key: usize) -> Option<JoinIndex<'_>> {
    let rows: usize = batches.iter().map(|b| b.row_count()).sum();
    let (mut lo, mut hi) = (i64::MAX, i64::MIN);
    for b in batches {
        match &*b.columns[key] {
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
    if range > (1 << 20) || range > 4 * rows as i128 + 1024 {
        return None;
    }
    let mut slots: Vec<Vec<(usize, usize)>> = vec![Vec::new(); range as usize];
    for (bi, b) in batches.iter().enumerate() {
        if let ArrayRef::Int64(a) = &*b.columns[key] {
            for row in 0..b.row_count() {
                if let Some(v) = a.value(row) {
                    slots[(v - lo) as usize].push((bi, row));
                }
            }
        }
    }
    Some(JoinIndex::Dense { min: lo, slots })
}

/// Probe the left rows in `segs` against the right-side `index`.
fn probe_segments(
    segs: &[Segment],
    left_batches: &[BatchRef],
    left_key: usize,
    index: &JoinIndex<'_>,
    right_rows: &[usize],
    keeps_left: bool,
) -> Probe {
    let mut p = Probe {
        left_pos: Vec::new(),
        right_pos: Vec::new(),
        right_matched: right_rows.iter().map(|&n| vec![false; n]).collect(),
    };
    for &(lbi, start, end) in segs {
        let lb = &left_batches[lbi];
        p.left_pos.reserve(end - start);
        p.right_pos.reserve(end - start);
        for row in start..end {
            let hits = index.hits(&lb.columns[left_key], row);
            if !hits.is_empty() {
                for &(rbi, rrow) in hits {
                    p.right_matched[rbi][rrow] = true;
                    p.left_pos.push(pack(lbi, row));
                    p.right_pos.push(pack(rbi, rrow));
                }
            } else if keeps_left {
                p.left_pos.push(pack(lbi, row));
                p.right_pos.push(NO_ROW);
            }
        }
    }
    p
}

/// Probe all left segments. With the `parallel` feature, contiguous runs of
/// segments are probed on worker threads and merged in order, so the output
/// matches a single pass.
fn probe_all(
    left_batches: &[BatchRef],
    left_key: usize,
    index: &JoinIndex<'_>,
    right_rows: &[usize],
    keeps_left: bool,
) -> Probe {
    let segs = segments(left_batches);
    #[cfg(feature = "parallel")]
    {
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(segs.len());
        if threads > 1 {
            let per = segs.len().div_ceil(threads);
            let parts: Vec<Probe> = std::thread::scope(|scope| {
                let workers: Vec<_> = segs
                    .chunks(per)
                    .map(|chunk| {
                        scope.spawn(move || {
                            probe_segments(
                                chunk,
                                left_batches,
                                left_key,
                                index,
                                right_rows,
                                keeps_left,
                            )
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                    .collect()
            });
            return merge_probes(parts);
        }
    }
    probe_segments(&segs, left_batches, left_key, index, right_rows, keeps_left)
}

/// Concatenate probes in order, and OR their right-match flags.
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
fn merge_probes(parts: Vec<Probe>) -> Probe {
    let mut merged = Probe {
        left_pos: Vec::new(),
        right_pos: Vec::new(),
        right_matched: Vec::new(),
    };
    for part in parts {
        merged.left_pos.extend(part.left_pos);
        merged.right_pos.extend(part.right_pos);
        if merged.right_matched.is_empty() {
            merged.right_matched = part.right_matched;
        } else {
            for (acc, mine) in merged.right_matched.iter_mut().zip(part.right_matched) {
                for (a, m) in acc.iter_mut().zip(mine) {
                    *a |= m;
                }
            }
        }
    }
    merged
}

impl PhysicalPlan for JoinExec {
    fn schema(&self) -> &Schema {
        &self.schema
    }

    fn execute(&self) -> Result<Vec<BatchRef>, QueryError> {
        let left_batches = self.left.execute()?;
        let right_batches = self.right.execute()?;

        // Index the right side by key, recording (batch, row) positions.
        // Keys are borrowed from the batches, so string keys are not copied.
        let index = build_index(&right_batches, self.right_key);

        let right_rows: Vec<usize> = right_batches.iter().map(|b| b.row_count()).collect();
        let probe = probe_all(
            &left_batches,
            self.left_key,
            &index,
            &right_rows,
            self.kind.keeps_left(),
        );
        let Probe {
            mut left_pos,
            mut right_pos,
            right_matched,
        } = probe;
        if self.kind.keeps_right() {
            for (rbi, matched_rows) in right_matched.iter().enumerate() {
                for (row, &matched) in matched_rows.iter().enumerate() {
                    if !matched {
                        left_pos.push(NO_ROW);
                        right_pos.push(pack(rbi, row));
                    }
                }
            }
        }

        let left_width = self.left.schema().len();
        let mut columns: Vec<ArrayRef> = Vec::with_capacity(self.schema.len());
        for c in 0..left_width {
            let dt = self.schema.fields[c].data_type;
            columns.push(gather_packed(&left_batches, c, &left_pos, dt));
        }
        for c in 0..self.right.schema().len() {
            let dt = self.schema.fields[left_width + c].data_type;
            columns.push(gather_packed(&right_batches, c, &right_pos, dt));
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
impl Condition {
    /// Add every column index the condition reads to `out`.
    pub fn columns(&self, out: &mut Vec<usize>) {
        match self {
            Condition::Cmp(p) => out.push(p.column),
            Condition::ColCmp { left, right, .. } => {
                out.push(*left);
                out.push(*right);
            }
            Condition::And(parts) | Condition::Or(parts) => {
                for part in parts {
                    part.columns(out);
                }
            }
        }
    }

    /// The same condition with every column index mapped through `map`.
    pub fn remap_columns(&self, map: &dyn Fn(usize) -> usize) -> Condition {
        match self {
            Condition::Cmp(p) => Condition::Cmp(Predicate {
                column: map(p.column),
                op: p.op,
                value: p.value.clone(),
            }),
            Condition::ColCmp { left, op, right } => Condition::ColCmp {
                left: map(*left),
                op: *op,
                right: map(*right),
            },
            Condition::And(parts) => {
                Condition::And(parts.iter().map(|c| c.remap_columns(map)).collect())
            }
            Condition::Or(parts) => {
                Condition::Or(parts.iter().map(|c| c.remap_columns(map)).collect())
            }
        }
    }
}

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
