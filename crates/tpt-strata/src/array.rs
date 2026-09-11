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
        }
    }

    /// Select the rows at `indices`, preserving order.
    pub fn take(&self, indices: &[usize]) -> Self {
        match self {
            ArrayRef::Boolean(a) => {
                let vals = indices.iter().map(|&i| a.0[i]).collect();
                ArrayRef::Boolean(BoolArray(vals))
            }
            ArrayRef::Int32(a) => {
                let vals = indices.iter().map(|&i| a.0[i]).collect();
                ArrayRef::Int32(I32Array(vals))
            }
            ArrayRef::Int64(a) => {
                let vals = indices.iter().map(|&i| a.0[i]).collect();
                ArrayRef::Int64(I64Array(vals))
            }
            ArrayRef::Float64(a) => {
                let vals = indices.iter().map(|&i| a.0[i]).collect();
                ArrayRef::Float64(F64Array(vals))
            }
            ArrayRef::Utf8(a) => {
                let vals = indices.iter().map(|&i| a.0[i].clone()).collect();
                ArrayRef::Utf8(StrArray(vals))
            }
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
                let mut vals = a.0.clone();
                vals.extend_from_slice(&b.0);
                ArrayRef::Boolean(BoolArray(vals))
            }
            (ArrayRef::Int32(a), ArrayRef::Int32(b)) => {
                let mut vals = a.0.clone();
                vals.extend_from_slice(&b.0);
                ArrayRef::Int32(I32Array(vals))
            }
            (ArrayRef::Int64(a), ArrayRef::Int64(b)) => {
                let mut vals = a.0.clone();
                vals.extend_from_slice(&b.0);
                ArrayRef::Int64(I64Array(vals))
            }
            (ArrayRef::Float64(a), ArrayRef::Float64(b)) => {
                let mut vals = a.0.clone();
                vals.extend_from_slice(&b.0);
                ArrayRef::Float64(F64Array(vals))
            }
            (ArrayRef::Utf8(a), ArrayRef::Utf8(b)) => {
                let mut vals = a.0.clone();
                vals.extend_from_slice(&b.0);
                ArrayRef::Utf8(StrArray(vals))
            }
            _ => unreachable!("data_type equality checked above"),
        })
    }
}

macro_rules! impl_array {
    ($name:ident, $variant:ident, $data_type:expr, $ty:ty, $scalar:ident) => {
        #[derive(Debug, Clone, Default)]
        pub struct $name(pub Vec<Option<$ty>>);

        impl $name {
            pub fn new(values: Vec<$ty>) -> Self {
                Self(values.into_iter().map(Some).collect())
            }

            pub fn from_nulls(values: Vec<Option<$ty>>) -> Self {
                Self(values)
            }

            pub fn values(&self) -> &[Option<$ty>] {
                &self.0
            }

            pub fn iter_values(&self) -> impl Iterator<Item = Option<&$ty>> {
                self.0.iter().map(|v| v.as_ref())
            }
        }

        impl Array for $name {
            fn data_type(&self) -> DataType {
                $data_type
            }

            fn len(&self) -> usize {
                self.0.len()
            }

            fn is_empty(&self) -> bool {
                self.0.is_empty()
            }

            fn is_null(&self, i: usize) -> bool {
                self.0[i].is_none()
            }

            fn null_count(&self) -> usize {
                self.0.iter().filter(|v| v.is_none()).count()
            }

            fn get(&self, i: usize) -> Option<Scalar> {
                self.0[i].as_ref().map(|v| Scalar::$scalar(v.clone()))
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

impl_array!(BoolArray, Boolean, DataType::Boolean, bool, Bool);
impl_array!(I32Array, Int32, DataType::Int32, i32, I32);
impl_array!(I64Array, Int64, DataType::Int64, i64, I64);
impl_array!(F64Array, Float64, DataType::Float64, f64, F64);
impl_array!(StrArray, Utf8, DataType::Utf8, String, Str);
