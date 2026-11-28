//! Owned values: what a shell reads from a node and writes to one.
//!
//! [`ShellValue`](crate::host::ShellValue) borrows from a callback's arguments and cannot be kept. [`Value`] owns its data, converts from
//! the Rust types a shell has (`10.0`, `"title"`, `true`, a [`NodeId`]) and crosses the ABI as a `GnoblinValue` that borrows from
//! it only for the call.
#![allow(unsafe_code)]

use core::ffi::c_void;

use gnoblin_core_abi::{self as abi, GnoblinStr, GnoblinValue};

use crate::value::{NodeId, ShellValue};

/// A property value, one variant per kind the tree carries.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// A boolean (`visible`, `maximised`).
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A double (`x`, `width`, `opacity`).
    Double(f64),
    /// Text (`title`, `app_id`).
    Text(String),
    /// A reference to another node (a seat's `focus`).
    Node(NodeId),
    /// A list of numbers.
    List(Vec<f64>),
    /// A list of strings.
    Texts(Vec<String>),
    /// Opaque bytes.
    Bytes(Vec<u8>),
}

impl Value {
    /// The value as a double, widening an integer; `None` for any other kind.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(value) => Some(*value),
            Value::Int(value) => Some(*value as f64),
            Value::Uint(value) => Some(*value as f64),
            _ => None,
        }
    }

    /// The value as a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(value) => Some(value),
            _ => None,
        }
    }

    /// The value as a node reference; the nil node for a reference to nothing.
    pub fn as_node(&self) -> Option<NodeId> {
        match self {
            Value::Node(value) => Some(*value),
            _ => None,
        }
    }

    /// Copies a borrowed value out of a callback.
    pub fn from_shell(value: &ShellValue<'_>) -> Option<Value> {
        Some(match value {
            ShellValue::Bool(value) => Value::Bool(*value),
            ShellValue::Int(value) => Value::Int(*value),
            ShellValue::Uint(value) => Value::Uint(*value),
            ShellValue::Double(value) => Value::Double(*value),
            ShellValue::Text(value) => Value::Text((*value).to_owned()),
            ShellValue::Node(value) => Value::Node(*value),
            ShellValue::List(items) => Value::List(
                items
                    .iter()
                    .filter_map(|item| Value::from_shell(item)?.as_f64())
                    .collect(),
            ),
            ShellValue::Texts(items) => Value::Texts(items.iter().map(|item| (*item).to_owned()).collect()),
            ShellValue::Bytes(value) => Value::Bytes((*value).to_vec()),
            _ => return None,
        })
    }

    /// Copies a value the core lent as the result of a read. Does not free it.
    ///
    /// # Safety
    ///
    /// `raw` must be a value the core filled in, whose pointers are valid for the call.
    pub(crate) unsafe fn from_raw(raw: &GnoblinValue) -> Option<Value> {
        // SAFETY: the caller guarantees the pointers match the kind, which is what `ShellValue::from_raw` needs.
        Value::from_shell(&unsafe { ShellValue::from_raw(raw) })
    }

    /// Calls `f` with the value as the ABI struct, borrowing from `self` for the call.
    pub(crate) fn with_raw<R>(&self, f: impl FnOnce(&GnoblinValue) -> R) -> R {
        // The arrays a list points into live in these locals until `f` returns.
        let (mut numbers, mut strings) = (Vec::new(), Vec::new());
        let raw = self.fill(&mut numbers, &mut strings);
        f(&raw)
    }

    /// The value as the ABI struct. A list points into `numbers` or `strings`, and text, bytes and the strings of a list of texts
    /// into `self`: the struct is valid while all three stay where they are.
    fn fill(&self, numbers: &mut Vec<GnoblinValue>, strings: &mut Vec<GnoblinStr>) -> GnoblinValue {
        let mut raw = GnoblinValue {
            size: size_of::<GnoblinValue>() as u32,
            kind: 0,
            flags: 0,
            reserved: 0,
            bits: 0,
            ptr: core::ptr::null::<c_void>(),
            len: 0,
        };
        match self {
            Value::Bool(value) => {
                raw.kind = abi::VALUE_BOOL;
                raw.bits = u64::from(*value);
            }
            Value::Int(value) => {
                raw.kind = abi::VALUE_INT;
                raw.bits = *value as u64;
            }
            Value::Uint(value) => {
                raw.kind = abi::VALUE_UINT;
                raw.bits = *value;
            }
            Value::Double(value) => {
                raw.kind = abi::VALUE_DOUBLE;
                raw.bits = value.to_bits();
            }
            Value::Node(value) => {
                raw.kind = abi::VALUE_NODE;
                raw.bits = value.0;
            }
            Value::Text(text) => {
                raw.kind = abi::VALUE_TEXT;
                raw.ptr = text.as_ptr().cast();
                raw.len = text.len();
            }
            Value::Bytes(bytes) => {
                raw.kind = abi::VALUE_BYTES;
                raw.ptr = bytes.as_ptr().cast();
                raw.len = bytes.len();
            }
            Value::List(list) => {
                *numbers = list
                    .iter()
                    .map(|number| GnoblinValue {
                        size: size_of::<GnoblinValue>() as u32,
                        kind: abi::VALUE_DOUBLE,
                        flags: 0,
                        reserved: 0,
                        bits: number.to_bits(),
                        ptr: core::ptr::null::<c_void>(),
                        len: 0,
                    })
                    .collect();
                raw.kind = abi::VALUE_LIST;
                raw.ptr = numbers.as_ptr().cast();
                raw.len = numbers.len();
            }
            Value::Texts(list) => {
                *strings = list.iter().map(|text| GnoblinStr::borrowed(text)).collect();
                raw.kind = abi::VALUE_TEXTS;
                raw.ptr = strings.as_ptr().cast();
                raw.len = strings.len();
            }
        }
        raw
    }
}

/// A value lent across the ABI for longer than one call: it owns the value and the arrays the struct points into, and the struct is
/// valid until the `Lent` is dropped or replaced. The vectors' buffers stay where they are when it moves.
pub(crate) struct Lent {
    raw: GnoblinValue,
    _value: Value,
    _numbers: Vec<GnoblinValue>,
    _strings: Vec<GnoblinStr>,
}

impl Lent {
    pub(crate) fn new(value: Value) -> Lent {
        let (mut numbers, mut strings) = (Vec::new(), Vec::new());
        let raw = value.fill(&mut numbers, &mut strings);
        Lent {
            raw,
            _value: value,
            _numbers: numbers,
            _strings: strings,
        }
    }

    /// The value as the ABI struct.
    pub(crate) fn raw(&self) -> GnoblinValue {
        GnoblinValue { ..self.raw }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Value {
        Value::Bool(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Value {
        Value::Double(value)
    }
}

impl From<f32> for Value {
    fn from(value: f32) -> Value {
        Value::Double(f64::from(value))
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Value {
        Value::Int(i64::from(value))
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Value {
        Value::Int(value)
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Value {
        Value::Uint(u64::from(value))
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Value {
        Value::Uint(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Value {
        Value::Text(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Value {
        Value::Text(value)
    }
}

impl From<NodeId> for Value {
    fn from(value: NodeId) -> Value {
        Value::Node(value)
    }
}

impl From<Vec<f64>> for Value {
    fn from(value: Vec<f64>) -> Value {
        Value::List(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_crosses_as_the_abi_struct_and_back() {
        for value in [
            Value::Bool(true),
            Value::Int(-4),
            Value::Uint(9),
            Value::Double(2.5),
            Value::Text("title".to_owned()),
            Value::Node(NodeId(7)),
            Value::List(vec![1.0, 2.0]),
            Value::Texts(vec!["a".to_owned(), "b".to_owned()]),
            Value::Bytes(vec![1, 2, 3]),
        ] {
            // SAFETY: the raw value borrows from `value` for the closure, which is all `from_raw` needs.
            let back = value.with_raw(|raw| unsafe { Value::from_raw(raw) });
            assert_eq!(back, Some(value));
        }
    }

    #[test]
    fn rust_types_convert_to_the_matching_kind() {
        assert_eq!(Value::from(10.0), Value::Double(10.0));
        assert_eq!(Value::from("x"), Value::Text("x".to_owned()));
        assert_eq!(Value::from(true).as_bool(), Some(true));
        assert_eq!(Value::from(3).as_f64(), Some(3.0));
        assert_eq!(Value::from(NodeId(5)).as_node(), Some(NodeId(5)));
        assert_eq!(Value::from("x").as_f64(), None);
    }
}
