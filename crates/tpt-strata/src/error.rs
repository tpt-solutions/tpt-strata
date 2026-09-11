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
                )?;
                if let Some(s) = suggest_closest(column, available) {
                    write!(f, " Did you mean '{s}'?")?;
                }
                Ok(())
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

/// Return the closest edit-distance match (Levenshtein distance <= 2) for a
/// misspelled name, used to suggest corrections in diagnostics.
pub(crate) fn suggest_closest(candidate: &str, available: &[String]) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for a in available {
        if a == candidate {
            continue;
        }
        let d = edit_distance(candidate, a);
        if d <= 2 && best.map_or(true, |(_, bd)| d < bd) {
            best = Some((a, d));
        }
    }
    best.map(|(s, _)| s.to_string())
}

fn edit_distance(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (cur[j] + 1).min(prev[j + 1] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}
