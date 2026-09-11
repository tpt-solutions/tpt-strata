use crate::array::ArrayRef;
use crate::error::QueryError;
use crate::schema::Schema;
use std::sync::Arc;

/// A chunk of columnar data: a schema plus one array per field.
///
/// Batches are the unit of streaming execution through the physical engine.
#[derive(Debug, Clone)]
pub struct Batch {
    pub schema: Arc<Schema>,
    pub columns: Vec<ArrayRef>,
}

impl Batch {
    /// Create a batch, validating that the column count matches the schema.
    pub fn try_new(schema: Arc<Schema>, columns: Vec<ArrayRef>) -> Result<Self, QueryError> {
        if columns.len() != schema.len() {
            return Err(QueryError::UnsupportedOperation {
                message: format!(
                    "internal error: batch has {} columns but schema has {} fields",
                    columns.len(),
                    schema.len()
                ),
            });
        }
        if let Some(f) = columns.first() {
            let len = f.len();
            for c in &columns {
                if c.len() != len {
                    return Err(QueryError::UnsupportedOperation {
                        message: "internal error: columns in a batch have differing lengths"
                            .to_string(),
                    });
                }
            }
        }
        for (c, field) in columns.iter().zip(schema.fields.iter()) {
            if c.data_type() != field.data_type {
                return Err(QueryError::UnsupportedOperation {
                    message: format!(
                        "internal error: column '{}' has type {} but schema declares {}",
                        field.name,
                        c.data_type(),
                        field.data_type
                    ),
                });
            }
        }
        Ok(Self { schema, columns })
    }

    /// Create a batch without validation, for callers that already hold
    /// type-correct arrays (e.g. the Parquet import bridge).
    pub fn new_unchecked(schema: Arc<Schema>, columns: Vec<ArrayRef>) -> Self {
        Self { schema, columns }
    }

    /// The number of rows in the batch (0 when there are no columns).
    pub fn row_count(&self) -> usize {
        self.columns.first().map_or(0, |c| c.len())
    }

    /// The number of columns in the batch.
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }
}
