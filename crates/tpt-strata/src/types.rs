use std::cmp::Ordering;
use std::fmt;

/// A scalar value in the native columnar format.
#[derive(Debug, Clone)]
pub enum Scalar {
    /// The absence of a value (SQL NULL).
    Null,
    /// A boolean value.
    Bool(bool),
    /// A 32-bit signed integer.
    I32(i32),
    /// A 64-bit signed integer.
    I64(i64),
    /// A double-precision floating-point value.
    F64(f64),
    /// A UTF-8 string.
    Str(String),
    /// A calendar date: days since 1970-01-01.
    Date(i32),
    /// A timestamp without a time zone: microseconds since 1970-01-01 00:00:00.
    Timestamp(i64),
}

/// Canonical bit pattern for an `f64` used as a group or join key.
///
/// Every NaN collapses to one pattern and `-0.0` folds into `0.0`, so keys
/// that should group together compare equal and hash identically.
fn f64_key_bits(v: f64) -> u64 {
    if v.is_nan() {
        f64::NAN.to_bits()
    } else if v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

// Equality is a true equivalence relation (reflexive, including for NaN),
// which `HashMap` keys require. `Scalar::F64(NaN) == Scalar::F64(NaN)` is
// therefore `true` here, unlike raw IEEE `==`. Predicate evaluation does not
// use this impl for numeric ordering; see [`numeric_cmp`].
impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Scalar::Null, Scalar::Null) => true,
            (Scalar::Bool(a), Scalar::Bool(b)) => a == b,
            (Scalar::I32(a), Scalar::I32(b)) => a == b,
            (Scalar::I64(a), Scalar::I64(b)) => a == b,
            (Scalar::F64(a), Scalar::F64(b)) => f64_key_bits(*a) == f64_key_bits(*b),
            (Scalar::Str(a), Scalar::Str(b)) => a == b,
            (Scalar::Date(a), Scalar::Date(b)) => a == b,
            (Scalar::Timestamp(a), Scalar::Timestamp(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Scalar {}

impl std::hash::Hash for Scalar {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Scalar::Null => {}
            Scalar::Bool(v) => v.hash(state),
            Scalar::I32(v) => v.hash(state),
            Scalar::I64(v) => v.hash(state),
            Scalar::F64(v) => f64_key_bits(*v).hash(state),
            Scalar::Str(v) => v.hash(state),
            Scalar::Date(v) => v.hash(state),
            Scalar::Timestamp(v) => v.hash(state),
        }
    }
}

impl Scalar {
    /// Returns the type name as a human-readable string.
    pub fn type_name(&self) -> &'static str {
        match self {
            Scalar::Null => "null",
            Scalar::Bool(_) => "bool",
            Scalar::I32(_) => "i32",
            Scalar::I64(_) => "i64",
            Scalar::F64(_) => "f64",
            Scalar::Str(_) => "str",
            Scalar::Date(_) => "date",
            Scalar::Timestamp(_) => "timestamp",
        }
    }

    /// Attempt to interpret the scalar as an f64 (lossy for large `i64`).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Scalar::I32(v) => Some(*v as f64),
            Scalar::I64(v) => Some(*v as f64),
            Scalar::F64(v) => Some(*v),
            _ => None,
        }
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scalar::Null => write!(f, "NULL"),
            Scalar::Bool(v) => write!(f, "{v}"),
            Scalar::I32(v) => write!(f, "{v}"),
            Scalar::I64(v) => write!(f, "{v}"),
            Scalar::F64(v) => {
                // Keep the decimal point on whole numbers, so a float column
                // does not look like an integer column (100.0, not 100).
                if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e16 {
                    write!(f, "{v:.1}")
                } else {
                    write!(f, "{v}")
                }
            }
            Scalar::Str(v) => write!(f, "{v}"),
            Scalar::Date(v) => write!(f, "{}", crate::temporal::format_date(*v)),
            Scalar::Timestamp(v) => write!(f, "{}", crate::temporal::format_timestamp(*v)),
        }
    }
}

/// Compare two integer/float values exactly. `i` is compared to `f` without
/// rounding `i` through `f64`, so large `i64` values keep their order.
/// Returns `None` for a NaN `f`.
fn cmp_int_float(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    // 2^63 is the first float strictly above every i64 value.
    const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
    if f >= TWO_POW_63 {
        return Some(Ordering::Less);
    }
    if f < -TWO_POW_63 {
        return Some(Ordering::Greater);
    }
    let t = f.trunc();
    Some(match i.cmp(&(t as i64)) {
        Ordering::Equal => {
            if f > t {
                Ordering::Less
            } else if f < t {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        other => other,
    })
}

/// Exact numeric ordering across `I32`, `I64` and `F64` scalars.
///
/// Returns `None` if either side is not numeric, or if a NaN is involved
/// (NaN is unordered, so every ordering comparison against it is false).
pub fn numeric_cmp(a: &Scalar, b: &Scalar) -> Option<Ordering> {
    match (a, b) {
        (Scalar::I32(x), Scalar::I32(y)) => Some(x.cmp(y)),
        (Scalar::I64(x), Scalar::I64(y)) => Some(x.cmp(y)),
        (Scalar::I32(x), Scalar::I64(y)) => Some((*x as i64).cmp(y)),
        (Scalar::I64(x), Scalar::I32(y)) => Some(x.cmp(&(*y as i64))),
        (Scalar::F64(x), Scalar::F64(y)) => x.partial_cmp(y),
        (Scalar::I32(x), Scalar::F64(y)) => cmp_int_float(*x as i64, *y),
        (Scalar::I64(x), Scalar::F64(y)) => cmp_int_float(*x, *y),
        (Scalar::F64(x), Scalar::I32(y)) => cmp_int_float(*y as i64, *x).map(Ordering::reverse),
        (Scalar::F64(x), Scalar::I64(y)) => cmp_int_float(*y, *x).map(Ordering::reverse),
        _ => None,
    }
}

/// Compare two scalars for ordering. Returns Equal for incompatible types.
pub fn scalar_cmp(a: &Scalar, b: &Scalar) -> Ordering {
    match (a, b) {
        (Scalar::I32(a), Scalar::I32(b)) => a.cmp(b),
        (Scalar::I64(a), Scalar::I64(b)) => a.cmp(b),
        (Scalar::F64(a), Scalar::F64(b)) => a.total_cmp(b),
        (Scalar::Str(a), Scalar::Str(b)) => a.cmp(b),
        (Scalar::Bool(a), Scalar::Bool(b)) => a.cmp(b),
        (Scalar::Date(a), Scalar::Date(b)) => a.cmp(b),
        (Scalar::Timestamp(a), Scalar::Timestamp(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

/// Order two scalars for sort operators.
///
/// `NULL` sorts **first** (ascending) by design — it is treated as smaller
/// than every non-null value, so a descending sort reverses it to last.
/// Unlike [`scalar_cmp`], a `NULL`-vs-value pair never falls through to
/// `Equal`, so nulls always land in a deterministic position.
pub fn sort_cmp(a: &Scalar, b: &Scalar) -> Ordering {
    match (a, b) {
        (Scalar::Null, Scalar::Null) => Ordering::Equal,
        (Scalar::Null, _) => Ordering::Less,
        (_, Scalar::Null) => Ordering::Greater,
        _ => scalar_cmp(a, b),
    }
}
