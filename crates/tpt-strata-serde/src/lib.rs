//! # tpt-strata-serde
//!
//! Serde support for tpt-strata. Convert your structs into a [`Table`] and
//! read query results back into structs, with no hand-written column lists.
//!
//! ```rust
//! use serde::{Deserialize, Serialize};
//! use tpt_strata::sql::SqlContext;
//! use tpt_strata_serde::{from_table, to_table};
//!
//! #[derive(Serialize, Deserialize, Debug, PartialEq)]
//! struct Order { region: String, quantity: i64, price: Option<f64> }
//!
//! let orders = vec![
//!     Order { region: "north".into(), quantity: 4, price: Some(2.5) },
//!     Order { region: "north".into(), quantity: 6, price: None },
//! ];
//! let table = to_table(&orders)?;
//!
//! let mut ctx = SqlContext::new();
//! ctx.add_table("orders", &table);
//! let totals = ctx.run("SELECT region, SUM(quantity) AS quantity FROM orders GROUP BY region")?;
//!
//! #[derive(Deserialize, Debug, PartialEq)]
//! struct Total { region: String, quantity: i64 }
//! let rows: Vec<Total> = from_table(&totals)?;
//! assert_eq!(rows, vec![Total { region: "north".into(), quantity: 10 }]);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Mapping
//!
//! | Rust field type | tpt-strata type |
//! |---|---|
//! | `bool` | `Boolean` |
//! | `i8`, `i16`, `i32`, `u8`, `u16` | `Int32` if every value fits, else `Int64` |
//! | `i64`, `u32`, `u64` (if it fits `i64`) | `Int64` |
//! | `f32`, `f64` | `Float64` |
//! | `String`, `&str`, `char` | `Utf8` |
//! | `Option<T>` | the type of `T`, with `None` as NULL |
//!
//! Only flat structs (fields that are scalars or options of scalars) can be
//! converted. A nested struct, list or map is an error that names the field.
//! Every column needs at least one non-null value, because the column type
//! is inferred from the data (see [`Table::try_new`]).

use std::fmt;

use serde::de::{self, DeserializeOwned, IntoDeserializer, MapAccess};
use serde::ser::{self, Serialize};
use tpt_strata::{Scalar, Table};

/// An error converting between Rust values and a table.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "serde: {}", self.message)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::new(msg.to_string())
    }
}

impl de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::new(msg.to_string())
    }
}

impl From<tpt_strata::QueryError> for Error {
    fn from(e: tpt_strata::QueryError) -> Self {
        Error::new(e.to_string())
    }
}

/// Convert a slice of flat structs into a [`Table`].
///
/// Column names come from the struct field names, in declaration order. An
/// empty slice is an error, because the column types cannot be inferred.
pub fn to_table<T: Serialize>(rows: &[T]) -> Result<Table, Error> {
    if rows.is_empty() {
        return Err(Error::new(
            "cannot infer a schema from zero rows; build the Table with Table::empty or from_rows",
        ));
    }
    let mut names: Option<Vec<&'static str>> = None;
    let mut values: Vec<Vec<Scalar>> = Vec::with_capacity(rows.len());
    for row in rows {
        let fields = row.serialize(RowSerializer)?;
        match &names {
            None => names = Some(fields.iter().map(|(n, _)| *n).collect()),
            Some(expected) => {
                if expected.len() != fields.len()
                    || expected.iter().zip(&fields).any(|(a, (b, _))| a != b)
                {
                    return Err(Error::new("rows do not all have the same fields"));
                }
            }
        }
        values.push(fields.into_iter().map(|(_, v)| v).collect());
    }
    let names = names.unwrap_or_default();
    let names_ref: Vec<&str> = names.to_vec();
    Ok(Table::from_rows(&names_ref, values)?)
}

/// Read every row of a table into a struct.
///
/// Column names must match the struct's field names. Extra columns are
/// ignored. Missing `Option` fields become `None`. A missing non-optional
/// field is an error that names it.
pub fn from_table<T: DeserializeOwned>(table: &Table) -> Result<Vec<T>, Error> {
    let fields = &table.schema().fields;
    let mut out = Vec::with_capacity(table.row_count());
    for batch in table.batches() {
        for r in 0..batch.row_count() {
            let row: Vec<(&str, Value)> = fields
                .iter()
                .enumerate()
                .map(|(c, f)| {
                    let v = batch.columns[c].get(r).unwrap_or(Scalar::Null);
                    (f.name.as_str(), Value(v))
                })
                .collect();
            let de = MapDeserializer::new(row.into_iter());
            out.push(T::deserialize(de)?);
        }
    }
    Ok(out)
}

// === Serialization: struct -> ordered (name, scalar) pairs ===

struct RowSerializer;

type Fields = Vec<(&'static str, Scalar)>;

impl ser::Serializer for RowSerializer {
    type Ok = Fields;
    type Error = Error;
    type SerializeSeq = ser::Impossible<Fields, Error>;
    type SerializeTuple = ser::Impossible<Fields, Error>;
    type SerializeTupleStruct = ser::Impossible<Fields, Error>;
    type SerializeTupleVariant = ser::Impossible<Fields, Error>;
    type SerializeMap = ser::Impossible<Fields, Error>;
    type SerializeStruct = StructFields;
    type SerializeStructVariant = ser::Impossible<Fields, Error>;

    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<StructFields, Error> {
        Ok(StructFields {
            fields: Vec::with_capacity(len),
        })
    }

    fn serialize_bool(self, _: bool) -> Result<Fields, Error> {
        Err(not_a_struct("a bool"))
    }
    fn serialize_i8(self, _: i8) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_i16(self, _: i16) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_i32(self, _: i32) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_i64(self, _: i64) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_u8(self, _: u8) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_u16(self, _: u16) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_u32(self, _: u32) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_u64(self, _: u64) -> Result<Fields, Error> {
        Err(not_a_struct("an integer"))
    }
    fn serialize_f32(self, _: f32) -> Result<Fields, Error> {
        Err(not_a_struct("a float"))
    }
    fn serialize_f64(self, _: f64) -> Result<Fields, Error> {
        Err(not_a_struct("a float"))
    }
    fn serialize_char(self, _: char) -> Result<Fields, Error> {
        Err(not_a_struct("a char"))
    }
    fn serialize_str(self, _: &str) -> Result<Fields, Error> {
        Err(not_a_struct("a string"))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Fields, Error> {
        Err(not_a_struct("bytes"))
    }
    fn serialize_none(self) -> Result<Fields, Error> {
        Err(not_a_struct("None"))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<Fields, Error> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<Fields, Error> {
        Err(not_a_struct("unit"))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Fields, Error> {
        Err(not_a_struct("a unit struct"))
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<Fields, Error> {
        Err(not_a_struct("an enum variant"))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<Fields, Error> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Fields, Error> {
        Err(not_a_struct("an enum variant"))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(not_a_struct("a sequence"))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        Err(not_a_struct("a tuple"))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(not_a_struct("a tuple struct"))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(not_a_struct("an enum variant"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(not_a_struct("a map"))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(not_a_struct("an enum variant"))
    }
}

fn not_a_struct(what: &str) -> Error {
    Error::new(format!("expected a struct, found {what}"))
}

struct StructFields {
    fields: Fields,
}

impl ser::SerializeStruct for StructFields {
    type Ok = Fields;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        let scalar = value.serialize(ScalarSerializer { field: key })?;
        self.fields.push((key, scalar));
        Ok(())
    }

    fn end(self) -> Result<Fields, Error> {
        Ok(self.fields)
    }
}

/// Serializes one field value into a [`Scalar`]. Only scalars and options
/// of scalars are accepted; anything else names the field in the error.
struct ScalarSerializer {
    field: &'static str,
}

impl ScalarSerializer {
    fn unsupported(&self, what: &str) -> Error {
        Error::new(format!(
            "field '{}' is {what}; only scalars and Option<scalar> can be stored in a table",
            self.field
        ))
    }
}

impl ser::Serializer for ScalarSerializer {
    type Ok = Scalar;
    type Error = Error;
    type SerializeSeq = ser::Impossible<Scalar, Error>;
    type SerializeTuple = ser::Impossible<Scalar, Error>;
    type SerializeTupleStruct = ser::Impossible<Scalar, Error>;
    type SerializeTupleVariant = ser::Impossible<Scalar, Error>;
    type SerializeMap = ser::Impossible<Scalar, Error>;
    type SerializeStruct = ser::Impossible<Scalar, Error>;
    type SerializeStructVariant = ser::Impossible<Scalar, Error>;

    fn serialize_bool(self, v: bool) -> Result<Scalar, Error> {
        Ok(Scalar::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Scalar, Error> {
        Ok(Scalar::I32(i32::from(v)))
    }
    fn serialize_i16(self, v: i16) -> Result<Scalar, Error> {
        Ok(Scalar::I32(i32::from(v)))
    }
    fn serialize_i32(self, v: i32) -> Result<Scalar, Error> {
        Ok(Scalar::I32(v))
    }
    fn serialize_i64(self, v: i64) -> Result<Scalar, Error> {
        Ok(Scalar::I64(v))
    }
    fn serialize_u8(self, v: u8) -> Result<Scalar, Error> {
        Ok(Scalar::I32(i32::from(v)))
    }
    fn serialize_u16(self, v: u16) -> Result<Scalar, Error> {
        Ok(Scalar::I32(i32::from(v)))
    }
    fn serialize_u32(self, v: u32) -> Result<Scalar, Error> {
        Ok(Scalar::I64(i64::from(v)))
    }
    fn serialize_u64(self, v: u64) -> Result<Scalar, Error> {
        i64::try_from(v).map(Scalar::I64).map_err(|_| {
            Error::new(format!(
                "field '{}' is {v}, which does not fit in i64",
                self.field
            ))
        })
    }
    fn serialize_f32(self, v: f32) -> Result<Scalar, Error> {
        Ok(Scalar::F64(f64::from(v)))
    }
    fn serialize_f64(self, v: f64) -> Result<Scalar, Error> {
        Ok(Scalar::F64(v))
    }
    fn serialize_char(self, v: char) -> Result<Scalar, Error> {
        Ok(Scalar::Str(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<Scalar, Error> {
        Ok(Scalar::Str(v.to_string()))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Scalar, Error> {
        Err(self.unsupported("bytes"))
    }
    fn serialize_none(self) -> Result<Scalar, Error> {
        Ok(Scalar::Null)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<Scalar, Error> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<Scalar, Error> {
        Ok(Scalar::Null)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Scalar, Error> {
        Ok(Scalar::Null)
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Scalar, Error> {
        // Unit enum variants are stored as their name, the common choice for
        // string-valued enums.
        Ok(Scalar::Str(variant.to_string()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<Scalar, Error> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Scalar, Error> {
        Err(self.unsupported("an enum variant with data"))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(self.unsupported("a list"))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
        Err(self.unsupported("a tuple"))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(self.unsupported("a tuple struct"))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(self.unsupported("an enum variant with data"))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(self.unsupported("a map"))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Error> {
        Err(self.unsupported("a nested struct"))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(self.unsupported("an enum variant with data"))
    }
}

// === Deserialization: (name, scalar) pairs -> struct ===

/// A cell value that can be handed to serde's map deserializer.
struct Value(Scalar);

impl<'de> IntoDeserializer<'de, Error> for Value {
    type Deserializer = ValueDeserializer;
    fn into_deserializer(self) -> ValueDeserializer {
        ValueDeserializer(self.0)
    }
}

struct ValueDeserializer(Scalar);

impl<'de> de::Deserializer<'de> for ValueDeserializer {
    type Error = Error;

    fn deserialize_any<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Scalar::Null => visitor.visit_none(),
            Scalar::Bool(b) => visitor.visit_bool(b),
            Scalar::I32(v) => visitor.visit_i32(v),
            Scalar::I64(v) => visitor.visit_i64(v),
            Scalar::F64(v) => visitor.visit_f64(v),
            Scalar::Str(s) => visitor.visit_string(s),
            // Dates and timestamps cross the serde boundary as ISO-8601 text.
            Scalar::Date(d) => visitor.visit_string(tpt_strata::temporal::format_date(d)),
            Scalar::Timestamp(t) => visitor.visit_string(tpt_strata::temporal::format_timestamp(t)),
        }
    }

    fn deserialize_option<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Scalar::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum identifier ignored_any
    }
}

/// A map of field name to value, one entry per column.
struct MapDeserializer<I> {
    iter: I,
    pending: Option<Value>,
}

impl<'a, I: Iterator<Item = (&'a str, Value)>> MapDeserializer<I> {
    fn new(iter: I) -> Self {
        Self {
            iter,
            pending: None,
        }
    }
}

impl<'de, 'a, I: Iterator<Item = (&'a str, Value)>> de::Deserializer<'de> for MapDeserializer<I> {
    type Error = Error;

    fn deserialize_any<V: de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_map(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de, 'a, I: Iterator<Item = (&'a str, Value)>> MapAccess<'de> for MapDeserializer<I> {
    type Error = Error;

    fn next_key_seed<K: de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        match self.iter.next() {
            Some((name, value)) => {
                self.pending = Some(value);
                seed.deserialize(de::value::StrDeserializer::<Error>::new(name))
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        match self.pending.take() {
            Some(value) => seed.deserialize(value.into_deserializer()),
            None => Err(Error::new("value requested before its key")),
        }
    }
}
