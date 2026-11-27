//! The value that crosses the object protocol, owned and in Rust's terms.
//!
//! [`Val`] is what a property holds, a call takes and returns, and a signal carries. The C ABI's `GnoblinValue` is the borrowed,
//! pointer-and-length form of the same thing (`wire`). One value type serves both ends of the protocol: the server reads the
//! arguments of a call as `Val`s and answers with one, and the client (an SDK, a script runtime) builds the arguments and reads
//! the answer.

use std::rc::Rc;

use crate::error::{Fail, Res};

/// An object of the protocol: the `GnoblinObj` a library issues. Zero is nil.
pub type Obj = u64;
/// A class of a catalog, by the id its library issues. Zero is nil.
pub type ClassId = u32;
/// A member of a class, by id within its class. Zero is nil.
pub type MemberId = u32;

/// How a timed write plays (`set_timed`): the duration, a delay, the `tween` node that drives it and the curve.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timing {
    /// The duration in seconds.
    pub seconds: f64,
    /// Seconds the start value is held before the change begins.
    pub delay: f64,
    /// The `tween` node whose progress drives the track, or zero for none.
    pub tween: u64,
    /// The curve: progress values from 0 to 1 spread evenly over the duration.
    pub samples: Vec<f64>,
}

/// A function the other side of the protocol can run: a closure a shell passed in, a closure the Rust side made, or a method of an
/// interface a shell implements. Calling it from the library happens on the shell thread, from the turn, never inside a call.
pub trait Callable {
    /// Runs it. `member` is zero for a plain closure, the method or property id for an implemented interface, and the signal or
    /// property id for a listener.
    ///
    /// # Errors
    ///
    /// The callee failed (a script raised, a panic was caught).
    fn call(&self, member: MemberId, args: &[Val]) -> Res<Val>;

    /// Marks the callable adopted: the call that carried it succeeded, so the library now owns what stands behind it and frees it when
    /// it lets go. A callable that was never adopted frees nothing, because the side that made it still owns it.
    fn adopt(&self) {}
}

/// A closure value. The server holds the shell's; the client holds its own until the library adopts it.
pub type Closure = Rc<dyn Callable>;

/// A value of the object protocol.
#[derive(Clone)]
pub enum Val {
    /// Nothing: an optional value that is absent, or an answer that is none.
    None,
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A floating-point number.
    Double(f64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// Secret text: never logged.
    Secret(String),
    /// A node of the compositor's tree, by id.
    Node(u64),
    /// A list of values of any kind.
    List(Vec<Val>),
    /// A record: the fields of the class `class`, in the order of the catalog's properties (zero when the sender names no class).
    Record {
        /// The record's class.
        class: ClassId,
        /// The fields, positional.
        fields: Vec<Val>,
    },
    /// An object: its handle and its most derived class, when the sender says (zero when not).
    Object {
        /// The object's class.
        class: ClassId,
        /// The handle.
        obj: Obj,
    },
    /// An object another library serves: the namespace of that library (`gnoblin.model`) and the handle there. Its reference, when
    /// a call hands one out, is that library's, and the receiver reads the class from it.
    Foreign {
        /// The namespace of the library that serves it.
        namespace: String,
        /// The handle in that library.
        obj: Obj,
    },
    /// A member of an enum: the enum's class (zero when not said) and the member's id, the first being 1.
    Enum {
        /// The enum's class.
        class: ClassId,
        /// The member's id within the enum.
        value: u32,
    },
    /// A closure.
    Closure(Closure),
}

impl std::fmt::Debug for Val {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Val::None => f.write_str("none"),
            Val::Bool(value) => write!(f, "{value}"),
            Val::Int(value) => write!(f, "{value}"),
            Val::Uint(value) => write!(f, "{value}u"),
            Val::Double(value) => write!(f, "{value:?}"),
            Val::Text(value) => write!(f, "{value:?}"),
            Val::Bytes(value) => write!(f, "bytes({})", value.len()),
            Val::Secret(_) => f.write_str("secret"),
            Val::Node(value) => write!(f, "node({value})"),
            Val::List(items) => f.debug_list().entries(items).finish(),
            Val::Record { class, fields } => f.debug_tuple(&format!("record#{class}")).field(fields).finish(),
            Val::Object { class, obj } => write!(f, "object#{class}({obj:#x})"),
            Val::Foreign { namespace, obj } => write!(f, "object@{namespace}({obj:#x})"),
            Val::Enum { class, value } => write!(f, "enum#{class}({value})"),
            Val::Closure(_) => f.write_str("closure"),
        }
    }
}

impl Val {
    /// The name of the value's kind, for a message.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Val::None => "none",
            Val::Bool(_) => "a boolean",
            Val::Int(_) => "an integer",
            Val::Uint(_) => "an unsigned integer",
            Val::Double(_) => "a number",
            Val::Text(_) => "text",
            Val::Bytes(_) => "bytes",
            Val::Secret(_) => "secret text",
            Val::Node(_) => "a node",
            Val::List(_) => "a list",
            Val::Record { .. } => "a record",
            Val::Object { .. } => "an object",
            Val::Foreign { .. } => "an object of another library",
            Val::Enum { .. } => "an enum value",
            Val::Closure(_) => "a closure",
        }
    }

    fn wrong(&self, wanted: &str) -> Fail {
        Fail::type_error(format!("expected {wanted}, got {}", self.kind_name()))
    }

    /// True for [`Val::None`].
    pub fn is_none(&self) -> bool {
        matches!(self, Val::None)
    }

    /// True for a value `any` carries: none, a boolean, a number, text, or a list of those. Nothing in it is an object, a record, a
    /// closure or bytes, so it holds no reference and means the same to every receiver.
    pub fn is_plain(&self) -> bool {
        match self {
            Val::None | Val::Bool(_) | Val::Int(_) | Val::Uint(_) | Val::Double(_) | Val::Text(_) => true,
            Val::List(items) => items.iter().all(Val::is_plain),
            _ => false,
        }
    }

    /// The boolean.
    ///
    /// # Errors
    ///
    /// The value is not a boolean.
    pub fn bool(&self) -> Res<bool> {
        match self {
            Val::Bool(value) => Ok(*value),
            other => Err(other.wrong("a boolean")),
        }
    }

    /// The integer: an integer, or a number that is whole and fits.
    ///
    /// # Errors
    ///
    /// The value is not a whole number in range.
    pub fn int(&self) -> Res<i64> {
        match self {
            Val::Int(value) => Ok(*value),
            Val::Uint(value) => i64::try_from(*value).map_err(|_| Fail::type_error("the integer is out of range")),
            Val::Double(value) if value.fract() == 0.0 && value.abs() < 9.0e15 => Ok(*value as i64),
            other => Err(other.wrong("an integer")),
        }
    }

    /// The unsigned integer: a non-negative integer, or a number that is whole and fits.
    ///
    /// # Errors
    ///
    /// The value is not a whole, non-negative number in range.
    pub fn uint(&self) -> Res<u64> {
        match self {
            Val::Uint(value) | Val::Node(value) => Ok(*value),
            Val::Int(value) => u64::try_from(*value).map_err(|_| Fail::type_error("the integer is negative")),
            Val::Double(value) if value.fract() == 0.0 && *value >= 0.0 && *value < 9.0e15 => Ok(*value as u64),
            other => Err(other.wrong("an unsigned integer")),
        }
    }

    /// The number, from any numeric value.
    ///
    /// # Errors
    ///
    /// The value is not a number.
    pub fn double(&self) -> Res<f64> {
        match self {
            Val::Double(value) => Ok(*value),
            Val::Int(value) => Ok(*value as f64),
            Val::Uint(value) => Ok(*value as f64),
            other => Err(other.wrong("a number")),
        }
    }

    /// The text (secret text reads as text).
    ///
    /// # Errors
    ///
    /// The value is not text.
    pub fn text(&self) -> Res<&str> {
        match self {
            Val::Text(value) | Val::Secret(value) => Ok(value),
            other => Err(other.wrong("text")),
        }
    }

    /// The elements of a list.
    ///
    /// # Errors
    ///
    /// The value is not a list.
    pub fn list(&self) -> Res<&[Val]> {
        match self {
            Val::List(items) => Ok(items),
            other => Err(other.wrong("a list")),
        }
    }

    /// The fields of a record, positional.
    ///
    /// # Errors
    ///
    /// The value is not a record.
    pub fn record(&self) -> Res<&[Val]> {
        match self {
            Val::Record { fields, .. } => Ok(fields),
            other => Err(other.wrong("a record")),
        }
    }

    /// The handle of an object.
    ///
    /// # Errors
    ///
    /// The value is not an object.
    pub fn object(&self) -> Res<Obj> {
        match self {
            Val::Object { obj, .. } => Ok(*obj),
            other => Err(other.wrong("an object")),
        }
    }

    /// The member id of an enum value.
    ///
    /// # Errors
    ///
    /// The value is not an enum value.
    pub fn enum_value(&self) -> Res<u32> {
        match self {
            Val::Enum { value, .. } => Ok(*value),
            other => Err(other.wrong("an enum value")),
        }
    }

    /// The closure.
    ///
    /// # Errors
    ///
    /// The value is not a closure.
    pub fn closure(&self) -> Res<&Closure> {
        match self {
            Val::Closure(closure) => Ok(closure),
            other => Err(other.wrong("a closure")),
        }
    }

    /// Marks every closure inside the value adopted (see [`Callable::adopt`]).
    pub fn adopt_closures(&self) {
        match self {
            Val::Closure(closure) => closure.adopt(),
            Val::List(items) => items.iter().for_each(Val::adopt_closures),
            Val::Record { fields, .. } => fields.iter().for_each(Val::adopt_closures),
            _ => {}
        }
    }
}

impl From<bool> for Val {
    fn from(value: bool) -> Val {
        Val::Bool(value)
    }
}

impl From<i64> for Val {
    fn from(value: i64) -> Val {
        Val::Int(value)
    }
}

impl From<u64> for Val {
    fn from(value: u64) -> Val {
        Val::Uint(value)
    }
}

impl From<f64> for Val {
    fn from(value: f64) -> Val {
        Val::Double(value)
    }
}

impl From<&str> for Val {
    fn from(value: &str) -> Val {
        Val::Text(value.to_owned())
    }
}

impl From<String> for Val {
    fn from(value: String) -> Val {
        Val::Text(value)
    }
}
