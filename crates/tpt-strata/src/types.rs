use std::fmt;

/// A scalar value in the native columnar format.
#[derive(Debug, Clone, PartialEq)]
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
            Scalar::F64(v) => v.to_bits().hash(state),
            Scalar::Str(v) => v.hash(state),
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
        }
    }

    /// Attempt to interpret the scalar as an f64 for numeric comparisons.
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
            Scalar::F64(v) => write!(f, "{v:.1}"),
            Scalar::Str(v) => write!(f, "{v}"),
        }
    }
}

/// Compare two scalars for ordering. Returns Equal for incompatible types.
pub fn scalar_cmp(a: &Scalar, b: &Scalar) -> std::cmp::Ordering {
    match (a, b) {
        (Scalar::I32(a), Scalar::I32(b)) => a.cmp(b),
        (Scalar::I64(a), Scalar::I64(b)) => a.cmp(b),
        (Scalar::F64(a), Scalar::F64(b)) => a.total_cmp(b),
        (Scalar::Str(a), Scalar::Str(b)) => a.cmp(b),
        (Scalar::Bool(a), Scalar::Bool(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    }
}
