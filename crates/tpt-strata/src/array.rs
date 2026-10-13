use crate::schema::DataType;
use crate::types::Scalar;
use std::fmt;

/// A typed, nullable arrow-free array of values of one type.
pub trait Array: fmt::Debug + Send + Sync {
    fn data_type(&self) -> DataType;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn is_null(&self, i: usize) -> bool;
    fn null_count(&self) -> usize;
    /// Return the value at `i` as a [`Scalar`], or `None` if null.
    fn get(&self, i: usize) -> Option<Scalar>;
}

/// An owned, dispatchable reference to a columnar array.
#[derive(Debug, Clone)]
pub enum ArrayRef {
    Boolean(BoolArray),
    Int32(I32Array),
    Int64(I64Array),
    Float64(F64Array),
    Utf8(StrArray),
    /// Days since 1970-01-01, held in an `I32Array`.
    Date(I32Array),
    /// Microseconds since the epoch, held in an `I64Array`.
    Timestamp(I64Array),
}

impl ArrayRef {
    /// The native type held by this array.
    pub fn data_type(&self) -> DataType {
        match self {
            ArrayRef::Boolean(a) => a.data_type(),
            ArrayRef::Int32(a) => a.data_type(),
            ArrayRef::Int64(a) => a.data_type(),
            ArrayRef::Float64(a) => a.data_type(),
            ArrayRef::Utf8(a) => a.data_type(),
            ArrayRef::Date(_) => DataType::Date,
            ArrayRef::Timestamp(_) => DataType::Timestamp,
        }
    }

    /// The number of elements in the array.
    pub fn len(&self) -> usize {
        match self {
            ArrayRef::Boolean(a) => a.len(),
            ArrayRef::Int32(a) => a.len(),
            ArrayRef::Int64(a) => a.len(),
            ArrayRef::Float64(a) => a.len(),
            ArrayRef::Utf8(a) => a.len(),
            ArrayRef::Date(a) => a.len(),
            ArrayRef::Timestamp(a) => a.len(),
        }
    }

    /// Whether the array has no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the element at `i` is null.
    pub fn is_null(&self, i: usize) -> bool {
        match self {
            ArrayRef::Boolean(a) => a.is_null(i),
            ArrayRef::Int32(a) => a.is_null(i),
            ArrayRef::Int64(a) => a.is_null(i),
            ArrayRef::Float64(a) => a.is_null(i),
            ArrayRef::Utf8(a) => a.is_null(i),
            ArrayRef::Date(a) => a.is_null(i),
            ArrayRef::Timestamp(a) => a.is_null(i),
        }
    }

    /// The number of null elements in the array.
    pub fn null_count(&self) -> usize {
        match self {
            ArrayRef::Boolean(a) => a.null_count(),
            ArrayRef::Int32(a) => a.null_count(),
            ArrayRef::Int64(a) => a.null_count(),
            ArrayRef::Float64(a) => a.null_count(),
            ArrayRef::Utf8(a) => a.null_count(),
            ArrayRef::Date(a) => a.null_count(),
            ArrayRef::Timestamp(a) => a.null_count(),
        }
    }

    /// The value at `i` as a [`Scalar`], or `None` if null.
    pub fn get(&self, i: usize) -> Option<Scalar> {
        match self {
            ArrayRef::Boolean(a) => a.get(i),
            ArrayRef::Int32(a) => a.get(i),
            ArrayRef::Int64(a) => a.get(i),
            ArrayRef::Float64(a) => a.get(i),
            ArrayRef::Utf8(a) => a.get(i),
            ArrayRef::Date(a) => a.value(i).map(Scalar::Date),
            ArrayRef::Timestamp(a) => a.value(i).map(Scalar::Timestamp),
        }
    }

    /// A new array of the elements at `indices`, in that order. An index past
    /// the end gives a NULL.
    pub fn take(&self, indices: &[usize]) -> Self {
        match self {
            ArrayRef::Boolean(a) => ArrayRef::Boolean(a.take(indices)),
            ArrayRef::Int32(a) => ArrayRef::Int32(a.take(indices)),
            ArrayRef::Int64(a) => ArrayRef::Int64(a.take(indices)),
            ArrayRef::Float64(a) => ArrayRef::Float64(a.take(indices)),
            ArrayRef::Utf8(a) => ArrayRef::Utf8(a.take(indices)),
            ArrayRef::Date(a) => ArrayRef::Date(a.take(indices)),
            ArrayRef::Timestamp(a) => ArrayRef::Timestamp(a.take(indices)),
        }
    }

    /// Append all elements of `other` onto `self`.
    ///
    /// Returns an error if the arrays are not the same type.
    pub fn concat(&self, other: &ArrayRef) -> Result<Self, crate::error::QueryError> {
        if self.data_type() != other.data_type() {
            return Err(crate::error::QueryError::UnsupportedOperation {
                message: format!(
                    "internal error: cannot concatenate arrays of type {} and {}",
                    self.data_type(),
                    other.data_type()
                ),
            });
        }
        Ok(match (self, other) {
            (ArrayRef::Boolean(a), ArrayRef::Boolean(b)) => {
                let mut out = a.clone();
                out.extend_from(b);
                ArrayRef::Boolean(out)
            }
            (ArrayRef::Int32(a), ArrayRef::Int32(b)) => {
                let mut out = a.clone();
                out.extend_from(b);
                ArrayRef::Int32(out)
            }
            (ArrayRef::Int64(a), ArrayRef::Int64(b)) => {
                let mut out = a.clone();
                out.extend_from(b);
                ArrayRef::Int64(out)
            }
            (ArrayRef::Float64(a), ArrayRef::Float64(b)) => {
                let mut out = a.clone();
                out.extend_from(b);
                ArrayRef::Float64(out)
            }
            (ArrayRef::Utf8(a), ArrayRef::Utf8(b)) => {
                let mut out = a.clone();
                out.extend_from(b);
                ArrayRef::Utf8(out)
            }
            (ArrayRef::Date(a), ArrayRef::Date(b)) => {
                let mut out = a.clone();
                out.extend_from(b);
                ArrayRef::Date(out)
            }
            (ArrayRef::Timestamp(a), ArrayRef::Timestamp(b)) => {
                let mut out = a.clone();
                out.extend_from(b);
                ArrayRef::Timestamp(out)
            }
            _ => unreachable!("data_type equality checked above"),
        })
    }
}

/// The owned payload of an element, for building a `Scalar` from a borrowed value.
/// Strings are shared as `Arc<str>` in arrays, so cloning a row is a
/// reference-count bump rather than a heap copy.
trait OwnedValue {
    type Owned;
    fn owned(&self) -> Self::Owned;
}

macro_rules! owned_copy {
    ($($t:ty),*) => {$(
        impl OwnedValue for $t {
            type Owned = $t;
            fn owned(&self) -> $t {
                *self
            }
        }
    )*};
}
owned_copy!(bool, i32, i64, f64);

impl OwnedValue for std::sync::Arc<str> {
    type Owned = String;
    fn owned(&self) -> String {
        self.to_string()
    }
}

/// One typed column: `values[i]` is the value at `i`, and `valid` says which
/// values are real. `valid` is `None` when every value is valid, which is
/// the common case, so such a column stores no validity at all. A NULL keeps
/// a placeholder in `values`, which is never read.
macro_rules! impl_array {
    ($name:ident, $variant:ident, $data_type:expr, $ty:ty, $scalar:ident, $null:expr) => {
        #[derive(Debug, Clone, Default)]
        pub struct $name {
            values: Vec<$ty>,
            valid: Option<Vec<bool>>,
        }

        impl $name {
            /// An array with no NULLs.
            pub fn new(values: Vec<$ty>) -> Self {
                Self {
                    values,
                    valid: None,
                }
            }

            /// An array from values and an optional validity vector (`None` means
            /// every value is valid). `valid`, when given, is as long as `values`.
            pub fn from_parts(values: Vec<$ty>, valid: Option<Vec<bool>>) -> Self {
                debug_assert!(valid.as_ref().map_or(true, |v| v.len() == values.len()));
                Self { values, valid }
            }

            /// An array from optional values; `None` is NULL.
            pub fn from_nulls(values: Vec<Option<$ty>>) -> Self {
                // One pass. The validity vector appears at the first NULL, and
                // every value before it was valid.
                let mut out = Vec::with_capacity(values.len());
                let mut valid: Option<Vec<bool>> = None;
                for v in values {
                    match v {
                        Some(x) => {
                            out.push(x);
                            if let Some(bits) = valid.as_mut() {
                                bits.push(true);
                            }
                        }
                        None => {
                            let bits = valid.get_or_insert_with(|| vec![true; out.len()]);
                            bits.push(false);
                            out.push($null);
                        }
                    }
                }
                Self { values: out, valid }
            }

            /// The value at `i`, or `None` if it is NULL or out of range.
            #[inline]
            pub fn value_ref(&self, i: usize) -> Option<&$ty> {
                if self.is_valid(i) {
                    self.values.get(i)
                } else {
                    None
                }
            }

            /// The value at `i` by copy (or `Arc` clone), or `None` if NULL.
            #[inline]
            pub fn value(&self, i: usize) -> Option<$ty> {
                self.value_ref(i).cloned()
            }

            /// Every element, with `None` for NULL.
            pub fn iter_values(&self) -> impl Iterator<Item = Option<&$ty>> {
                (0..self.values.len()).map(move |i| self.value_ref(i))
            }

            /// Every element as an owned option, for handing to other libraries.
            pub fn to_options(&self) -> Vec<Option<$ty>> {
                self.iter_values().map(|v| v.cloned()).collect()
            }

            /// A new array of the elements at `indices`. An index past the end
            /// gives a NULL.
            pub fn take(&self, indices: &[usize]) -> Self {
                // Common case: no NULLs and every index in range. Gather the values
                // and skip building a validity vector.
                if self.valid.is_none() && indices.iter().all(|&i| i < self.values.len()) {
                    return Self {
                        values: indices.iter().map(|&i| self.values[i].clone()).collect(),
                        valid: None,
                    };
                }
                let mut values = Vec::with_capacity(indices.len());
                let mut valid = Vec::with_capacity(indices.len());
                for &i in indices {
                    match self.value_ref(i) {
                        Some(v) => {
                            values.push(v.clone());
                            valid.push(true);
                        }
                        None => {
                            values.push($null);
                            valid.push(false);
                        }
                    }
                }
                let valid = if valid.iter().all(|&v| v) {
                    None
                } else {
                    Some(valid)
                };
                Self { values, valid }
            }

            /// Append `other` onto `self`.
            pub fn extend_from(&mut self, other: &Self) {
                let n = self.values.len();
                self.values.extend(other.values.iter().cloned());
                self.valid = match (self.valid.take(), &other.valid) {
                    (None, None) => None,
                    (current, theirs) => {
                        let mut valid = current.unwrap_or_else(|| vec![true; n]);
                        match theirs {
                            None => valid.extend(std::iter::repeat(true).take(other.values.len())),
                            Some(v) => valid.extend_from_slice(v),
                        }
                        Some(valid)
                    }
                };
            }

            /// Whether `i` is in range and not NULL.
            #[inline]
            fn is_valid(&self, i: usize) -> bool {
                i < self.values.len() && self.valid.as_ref().map_or(true, |v| v[i])
            }
        }

        impl Array for $name {
            fn data_type(&self) -> DataType {
                $data_type
            }

            fn len(&self) -> usize {
                self.values.len()
            }

            fn is_empty(&self) -> bool {
                self.values.is_empty()
            }

            /// Out-of-range indices count as null rather than panicking.
            fn is_null(&self, i: usize) -> bool {
                !self.is_valid(i)
            }

            fn null_count(&self) -> usize {
                match &self.valid {
                    None => 0,
                    Some(v) => v.iter().filter(|&&b| !b).count(),
                }
            }

            /// Returns `None` for a null value and for an out-of-range index.
            fn get(&self, i: usize) -> Option<Scalar> {
                self.value_ref(i).map(|v| Scalar::$scalar(v.owned()))
            }
        }

        impl From<$name> for ArrayRef {
            fn from(v: $name) -> Self {
                ArrayRef::$variant(v)
            }
        }

        impl<'a> TryFrom<&'a ArrayRef> for &'a $name {
            type Error = crate::error::QueryError;

            fn try_from(value: &'a ArrayRef) -> Result<Self, Self::Error> {
                match value {
                    ArrayRef::$variant(a) => Ok(a),
                    other => Err(crate::error::QueryError::internal_downcast(other)),
                }
            }
        }
    };
}

impl_array!(BoolArray, Boolean, DataType::Boolean, bool, Bool, false);
impl_array!(I32Array, Int32, DataType::Int32, i32, I32, 0);
impl_array!(I64Array, Int64, DataType::Int64, i64, I64, 0);
impl_array!(F64Array, Float64, DataType::Float64, f64, F64, 0.0);
impl_array!(
    StrArray,
    Utf8,
    DataType::Utf8,
    std::sync::Arc<str>,
    Str,
    std::sync::Arc::from("")
);

/// A zero-length array of the given type.
pub(crate) fn empty_array(data_type: DataType) -> ArrayRef {
    match data_type {
        DataType::Boolean => ArrayRef::Boolean(BoolArray::default()),
        DataType::Int32 => ArrayRef::Int32(I32Array::default()),
        DataType::Int64 => ArrayRef::Int64(I64Array::default()),
        DataType::Float64 => ArrayRef::Float64(F64Array::default()),
        DataType::Utf8 => ArrayRef::Utf8(StrArray::default()),
        DataType::Date => ArrayRef::Date(I32Array::default()),
        DataType::Timestamp => ArrayRef::Timestamp(I64Array::default()),
    }
}
