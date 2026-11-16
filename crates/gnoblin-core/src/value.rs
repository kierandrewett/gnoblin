//! Values, events and catalog members: the safe view of what the core passes to a shell.
//!
//! Everything the core hands a callback is borrowed for the duration of that callback, so the types here carry a
//! lifetime and cannot be kept. A shell that wants to keep a value copies it out.
#![allow(unsafe_code)]

use gnoblin_core_abi::{self as abi, GnoblinEvent, GnoblinMember, GnoblinNode, GnoblinStr, GnoblinValue};

/// A node of the compositor's tree, by the schema's id. The id is never reused within one compositor run, and a
/// destroyed node's id returns `NoSuchNode`, never another node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u64);

impl NodeId {
    /// The nil id: "no node".
    pub const NIL: NodeId = NodeId(0);
}

impl From<GnoblinNode> for NodeId {
    fn from(raw: GnoblinNode) -> NodeId {
        NodeId(raw)
    }
}

/// A catalog class (`window`, `seat`, ...), interned by name. An id is only meaningful against the core that issued it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Class(pub u32);

/// What kind of member of a class a [`CatalogMember`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemberKind {
    /// A typed value the compositor keeps and publishes changes of.
    Property,
    /// A call the shell can make.
    Method,
    /// A notification the compositor sends.
    Signal,
}

impl MemberKind {
    pub(crate) fn raw(self) -> u32 {
        match self {
            MemberKind::Property => abi::MEMBER_PROPERTY,
            MemberKind::Method => abi::MEMBER_METHOD,
            MemberKind::Signal => abi::MEMBER_SIGNAL,
        }
    }
}

/// One property, method or signal of a class, as the catalog describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogMember {
    /// The member's id within its class and kind.
    pub id: u32,
    /// Property, method or signal.
    pub kind: MemberKind,
    /// The `VALUE_*` kind of a property (`VALUE_SECRET` for a secret one); zero for methods and signals.
    pub value_kind: u32,
    /// Whether a write may animate the property.
    pub animated: bool,
    /// Whether the property cannot be written.
    pub readonly: bool,
    /// Whether the member's values are secret and never logged.
    pub secret: bool,
    /// The `VALUE_*` kinds of a method's parameters or a signal's fields.
    pub args: Vec<u32>,
}

impl CatalogMember {
    /// Copies a member out of the core's borrowed description.
    pub(crate) fn from_raw(raw: &GnoblinMember, kind: MemberKind) -> CatalogMember {
        let args = if raw.arg_kinds.is_null() || raw.arg_count == 0 {
            Vec::new()
        } else {
            // SAFETY: the core documents `arg_kinds` as `arg_count` `u32`s in static storage that lives for the whole
            // process, and `arg_count` is not zero.
            unsafe { std::slice::from_raw_parts(raw.arg_kinds, raw.arg_count as usize) }.to_vec()
        };
        CatalogMember {
            id: raw.id,
            kind,
            value_kind: raw.value_kind,
            animated: raw.flags & abi::MEMBER_ANIMATED != 0,
            readonly: raw.flags & abi::MEMBER_READONLY != 0,
            secret: raw.flags & abi::MEMBER_SECRET != 0,
            args,
        }
    }
}

/// A value the core passed in a callback. It borrows from the callback's arguments.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ShellValue<'a> {
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A double.
    Double(f64),
    /// UTF-8 text.
    Text(&'a str),
    /// A node id.
    Node(NodeId),
    /// A list of values.
    List(Vec<ShellValue<'a>>),
    /// A list of strings.
    Texts(Vec<&'a str>),
    /// Opaque bytes, and text that was not valid UTF-8.
    Bytes(&'a [u8]),
    /// Secret text. Copy it out before the callback returns; never log it.
    Secret(&'a [u8]),
    /// A record: its properties in schema order.
    Record(Vec<ShellValue<'a>>),
    /// A kind a newer core sends that this SDK does not know.
    Unsupported(u32),
}

impl<'a> ShellValue<'a> {
    /// Decodes a raw value.
    ///
    /// # Safety
    ///
    /// `raw.ptr` and `raw.len` must describe what `raw.kind` says they do, for the lifetime `'a`. A value the core passes
    /// in a callback satisfies this until the callback returns.
    pub(crate) unsafe fn from_raw(raw: &GnoblinValue) -> ShellValue<'a> {
        // SAFETY (all arms): the caller guarantees the pointer and length match the kind, as documented on the function.
        match raw.kind {
            abi::VALUE_BOOL => ShellValue::Bool(raw.bits != 0),
            // The bit pattern is two's complement by the ABI's definition.
            abi::VALUE_INT => ShellValue::Int(raw.bits as i64),
            abi::VALUE_UINT => ShellValue::Uint(raw.bits),
            abi::VALUE_DOUBLE => ShellValue::Double(f64::from_bits(raw.bits)),
            abi::VALUE_NODE => ShellValue::Node(NodeId(raw.bits)),
            abi::VALUE_TEXT => {
                let bytes = unsafe { bytes_of(raw) };
                match std::str::from_utf8(bytes) {
                    Ok(text) => ShellValue::Text(text),
                    Err(_) => ShellValue::Bytes(bytes),
                }
            }
            abi::VALUE_BYTES => ShellValue::Bytes(unsafe { bytes_of(raw) }),
            abi::VALUE_SECRET => ShellValue::Secret(unsafe { bytes_of(raw) }),
            abi::VALUE_LIST | abi::VALUE_RECORD => {
                let items = if raw.ptr.is_null() || raw.len == 0 {
                    &[][..]
                } else {
                    unsafe { std::slice::from_raw_parts(raw.ptr.cast::<GnoblinValue>(), raw.len) }
                };
                let values = items.iter().map(|item| unsafe { ShellValue::from_raw(item) }).collect();
                if raw.kind == abi::VALUE_LIST {
                    ShellValue::List(values)
                } else {
                    ShellValue::Record(values)
                }
            }
            abi::VALUE_TEXTS => {
                let items = if raw.ptr.is_null() || raw.len == 0 {
                    &[][..]
                } else {
                    unsafe { std::slice::from_raw_parts(raw.ptr.cast::<GnoblinStr>(), raw.len) }
                };
                ShellValue::Texts(
                    items
                        .iter()
                        .map(|item| {
                            let bytes = if item.ptr.is_null() {
                                &[][..]
                            } else {
                                unsafe { std::slice::from_raw_parts(item.ptr, item.len) }
                            };
                            std::str::from_utf8(bytes).unwrap_or("")
                        })
                        .collect(),
                )
            }
            other => ShellValue::Unsupported(other),
        }
    }
}

/// The bytes a text, bytes or secret value points at.
///
/// # Safety
///
/// `raw.ptr` must be null or point at `raw.len` readable bytes that outlive `'a`.
unsafe fn bytes_of<'a>(raw: &GnoblinValue) -> &'a [u8] {
    if raw.ptr.is_null() || raw.len == 0 {
        &[]
    } else {
        // SAFETY: the caller guarantees `ptr` points at `len` readable bytes that live for `'a`.
        unsafe { std::slice::from_raw_parts(raw.ptr.cast::<u8>(), raw.len) }
    }
}

/// One signal delivery, passed to the callback registered with `Gnoblin::on_signal`.
#[derive(Clone, Copy)]
pub struct SignalEvent<'a> {
    raw: &'a GnoblinEvent,
}

impl<'a> SignalEvent<'a> {
    pub(crate) fn new(raw: &'a GnoblinEvent) -> SignalEvent<'a> {
        SignalEvent { raw }
    }

    /// The node the signal came from, or [`NodeId::NIL`] for a signal that has none.
    pub fn node(&self) -> NodeId {
        NodeId(self.raw.node)
    }

    /// The class of that node.
    pub fn class(&self) -> Class {
        Class(self.raw.class)
    }

    /// The signal's id within its class.
    pub fn signal(&self) -> u32 {
        self.raw.signal
    }

    /// How many payload fields the signal carries.
    pub fn len(&self) -> usize {
        self.raw.n_values
    }

    /// True when the signal carries no payload.
    pub fn is_empty(&self) -> bool {
        self.raw.n_values == 0
    }

    /// The payload field at `index`, in schema order.
    pub fn value(&self, index: usize) -> Option<ShellValue<'a>> {
        if index >= self.raw.n_values || self.raw.values.is_null() {
            return None;
        }
        // SAFETY: the core passes `n_values` values at `values` that stay valid until the callback returns, and
        // `index` is below `n_values`. The event borrows from the callback's arguments, which is what `'a` is.
        Some(unsafe { ShellValue::from_raw(&*self.raw.values.add(index)) })
    }

    /// Every payload field, in schema order.
    pub fn values(&self) -> Vec<ShellValue<'a>> {
        (0..self.len()).filter_map(|index| self.value(index)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::c_void;

    fn scalar(kind: u32, bits: u64) -> GnoblinValue {
        GnoblinValue {
            size: size_of::<GnoblinValue>() as u32,
            kind,
            flags: 0,
            reserved: 0,
            bits,
            ptr: core::ptr::null::<c_void>(),
            len: 0,
        }
    }

    #[test]
    fn scalars_decode_by_kind() {
        // SAFETY: scalar kinds read only `bits`.
        unsafe {
            assert_eq!(
                ShellValue::from_raw(&scalar(abi::VALUE_BOOL, 1)),
                ShellValue::Bool(true)
            );
            assert_eq!(
                ShellValue::from_raw(&scalar(abi::VALUE_INT, (-5i64) as u64)),
                ShellValue::Int(-5)
            );
            assert_eq!(ShellValue::from_raw(&scalar(abi::VALUE_UINT, 7)), ShellValue::Uint(7));
            assert_eq!(
                ShellValue::from_raw(&scalar(abi::VALUE_DOUBLE, 1.5f64.to_bits())),
                ShellValue::Double(1.5)
            );
            assert_eq!(
                ShellValue::from_raw(&scalar(abi::VALUE_NODE, 9)),
                ShellValue::Node(NodeId(9))
            );
            assert_eq!(ShellValue::from_raw(&scalar(999, 0)), ShellValue::Unsupported(999));
        }
    }

    #[test]
    fn text_decodes_and_invalid_utf8_falls_back_to_bytes() {
        let good = b"hello";
        let bad = [0xffu8, 0xfe];
        let mut value = scalar(abi::VALUE_TEXT, 0);
        value.ptr = good.as_ptr().cast();
        value.len = good.len();
        // SAFETY: `ptr` and `len` describe the live array `good`.
        assert_eq!(unsafe { ShellValue::from_raw(&value) }, ShellValue::Text("hello"));
        value.ptr = bad.as_ptr().cast();
        value.len = bad.len();
        // SAFETY: `ptr` and `len` describe the live array `bad`.
        assert_eq!(unsafe { ShellValue::from_raw(&value) }, ShellValue::Bytes(&bad));
    }

    #[test]
    fn lists_decode_recursively() {
        let items = [scalar(abi::VALUE_UINT, 1), scalar(abi::VALUE_BOOL, 0)];
        let mut list = scalar(abi::VALUE_LIST, 0);
        list.ptr = items.as_ptr().cast();
        list.len = items.len();
        // SAFETY: `ptr` and `len` describe the live array `items` of `GnoblinValue`.
        assert_eq!(
            unsafe { ShellValue::from_raw(&list) },
            ShellValue::List(vec![ShellValue::Uint(1), ShellValue::Bool(false)])
        );
    }
}
