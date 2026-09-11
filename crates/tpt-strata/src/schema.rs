use std::fmt;

/// The data types supported by tpt-strata's native columnar format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataType {
    Boolean,
    Int32,
    Int64,
    Float64,
    Utf8,
}

impl DataType {
    /// Human-readable type name used in caller-facing diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            DataType::Boolean => "bool",
            DataType::Int32 => "i32",
            DataType::Int64 => "i64",
            DataType::Float64 => "f64",
            DataType::Utf8 => "str",
        }
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A named, typed column in a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub data_type: DataType,
    pub nullable: bool,
}

impl Field {
    /// Create a field with the given name, type, and nullability.
    pub fn new(name: impl Into<String>, data_type: DataType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

/// The set of fields describing a table or batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    pub fields: Vec<Field>,
}

impl Schema {
    /// Build a schema from its fields, in column order.
    pub fn new(fields: Vec<Field>) -> Self {
        Self { fields }
    }

    /// The number of fields in the schema.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether the schema has no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Return the index of the named field, if present.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }

    /// Return the [`DataType`] of the field at `index`.
    pub fn field(&self, index: usize) -> Option<&Field> {
        self.fields.get(index)
    }
}

impl fmt::Display for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|fd| match fd.nullable {
                true => format!("{}: {} (nullable)", fd.name, fd.data_type),
                false => format!("{}: {}", fd.name, fd.data_type),
            })
            .collect();
        write!(f, "{}", fields.join(", "))
    }
}
