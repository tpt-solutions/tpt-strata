use std::fmt;

/// Errors surfaced to callers, named in terms of the caller's own schema.
#[derive(Debug)]
pub enum QueryError {
    /// A column name provided by the caller does not exist in the table.
    MissingColumn {
        column: String,
        available: Vec<String>,
    },
    /// A type comparison was attempted between incompatible types.
    TypeMismatch {
        column: String,
        expected: &'static str,
        actual: &'static str,
        context: String,
    },
    /// An operation is not supported at this layer.
    UnsupportedOperation { message: String },
    /// The SQL string could not be parsed.
    Sql { message: String },
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::MissingColumn { column, available } => {
                write!(
                    f,
                    "No column named '{column}'. Available columns: {}",
                    available.join(", ")
                )
            }
            QueryError::TypeMismatch {
                column,
                expected,
                actual,
                context,
            } => {
                write!(
                    f,
                    "Type mismatch in '{column}': expected {expected}, found {actual}. {context}"
                )
            }
            QueryError::UnsupportedOperation { message } => {
                write!(f, "Unsupported operation: {message}")
            }
            QueryError::Sql { message } => write!(f, "SQL error: {message}"),
        }
    }
}

impl std::error::Error for QueryError {}

impl QueryError {
    /// Internal helper for an array downcast that hit the wrong variant.
    pub(crate) fn unsupported_format(array: &dyn std::fmt::Debug) -> Self {
        QueryError::UnsupportedOperation {
            message: format!("internal type mismatch during conversion: {array:?}"),
        }
    }
}
