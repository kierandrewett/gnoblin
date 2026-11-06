//! Typed property values.
//!
//! A value is a plain Rust enum that mirrors [`gnoblin_schema::Type`]. The schema
//! stays the authority on types: [`Value::fits`] answers whether a value has the
//! shape a member declares, and the graph refuses a write that does not fit
//! rather than coercing it.

use std::collections::BTreeMap;

use gnoblin_schema::Type;

use gnoblin_types::ObjectId;

/// One typed value: a property, a signal field or a method argument.
///
/// `Double` is compared with IEEE equality, so this type is [`PartialEq`] and not
/// [`Eq`]; nothing in the crate hashes values.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    Int(i64),
    /// An unsigned 64-bit integer.
    Uint(u64),
    /// An IEEE 754 double.
    Double(f64),
    /// A UTF-8 string.
    Text(String),
    /// Opaque octets.
    Bytes(Vec<u8>),
    /// The identity of another object.
    Object(ObjectId),
    /// A homogeneous sequence.
    List(Vec<Value>),
}

impl Value {
    /// Whether this value has the shape the schema declares for `ty`.
    ///
    /// A reference is checked for shape only, not for whether the object exists or
    /// has the declared type: a reference is an identity, and resolving it is the
    /// reader's job.
    pub fn fits(&self, ty: &Type) -> bool {
        match (self, ty) {
            (Value::Bool(_), Type::Bool)
            | (Value::Int(_), Type::Int)
            | (Value::Uint(_), Type::Uint)
            | (Value::Double(_), Type::Double)
            | (Value::Text(_), Type::Text)
            | (Value::Bytes(_), Type::Bytes)
            | (Value::Object(_), Type::ObjectRef(_)) => true,
            (Value::List(values), Type::List(inner)) => values.iter().all(|value| value.fits(inner)),
            _ => false,
        }
    }

    /// The schema type name this value has, for an error message.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Uint(_) => "uint",
            Value::Double(_) => "double",
            Value::Text(_) => "string",
            Value::Bytes(_) => "bytes",
            Value::Object(_) => "object",
            Value::List(_) => "list",
        }
    }
}

/// A set of named values: an object's properties, a signal's fields or a method's
/// arguments.
///
/// Ordered by name so a snapshot, an encoded frame and a test assertion all see
/// the same order regardless of how the caller built the map.
pub type Fields = BTreeMap<String, Value>;
