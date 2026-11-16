//! Values across the C boundary: the owned form the core keeps, the `GnoblinValue` a callback is lent, and the conversions
//! to and from the tree's own `MirrorValue`.
//!
//! Three directions, three rules (`design/modular-framework.md` section 5.2):
//!
//! - **Into a callback**: an event's values are lent for the call. [`RawScope`] holds the arrays a list value points into, so
//!   the pointers stay valid until the callback returns.
//! - **Out of a read** (`node_get`): text and bytes are copied into core memory and lent as owned values (`value_free`); lists
//!   are lent from a per-thread arena that the next call which returns transient data replaces.
//! - **Into a write** (`write_set`, a call's arguments): the shell's value is read for the call and copied. A value of the wrong
//!   kind for the property is `E_TYPE`; integers widen to doubles and a node id may be written as an unsigned integer, because a
//!   schema type is the shell's to satisfy and not its spelling's.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::slice;

use gnoblin_core_abi::{self as abi, GnoblinNode, GnoblinNodes, GnoblinStr, GnoblinValue};
use gnoblin_tree_client::tree_mirror::MirrorValue;

use crate::catalog::DefaultDef;
use crate::memory;

/// A value the compositor side publishes with an event, or the core keeps for one. Owned, so it outlives the queue.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedValue {
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A double.
    Double(f64),
    /// Text.
    Text(String),
    /// A node id.
    Node(u64),
    /// A list of numbers.
    List(Vec<f64>),
    /// A list of strings.
    Texts(Vec<String>),
    /// Opaque bytes.
    Bytes(Vec<u8>),
}

impl OwnedValue {
    /// The value a tree property holds, `None` for a kind the ABI does not carry yet (effects).
    pub fn from_mirror(value: &MirrorValue) -> Option<OwnedValue> {
        Some(match value {
            MirrorValue::Bool(value) => OwnedValue::Bool(*value),
            MirrorValue::Int(value) => OwnedValue::Int(*value),
            MirrorValue::Uint(value) => OwnedValue::Uint(*value),
            MirrorValue::Double(value) => OwnedValue::Double(*value),
            MirrorValue::Text(value) => OwnedValue::Text(value.clone()),
            MirrorValue::Node(value) => OwnedValue::Node(*value),
            MirrorValue::List(value) => OwnedValue::List(value.clone()),
            MirrorValue::Texts(value) => OwnedValue::Texts(value.clone()),
            MirrorValue::Bytes(value) => OwnedValue::Bytes(value.clone()),
            MirrorValue::Effects(_) => return None,
        })
    }

    /// What a property of `value_kind` reads as before anything wrote it: its schema default, or the empty value of its kind.
    pub(crate) fn default_of(value_kind: u32, default: DefaultDef) -> Option<OwnedValue> {
        Some(match (value_kind, default) {
            (_, DefaultDef::Bool(value)) => OwnedValue::Bool(value),
            (_, DefaultDef::Int(value)) => OwnedValue::Int(value),
            (_, DefaultDef::Uint(value)) => OwnedValue::Uint(value),
            (_, DefaultDef::Double(value)) => OwnedValue::Double(value),
            (_, DefaultDef::Text(value)) => OwnedValue::Text(value.to_owned()),
            (_, DefaultDef::Numbers(value)) => OwnedValue::List(value.to_vec()),
            (_, DefaultDef::Texts(value)) => OwnedValue::Texts(value.iter().map(|text| (*text).to_owned()).collect()),
            (abi::VALUE_BOOL, DefaultDef::None) => OwnedValue::Bool(false),
            (abi::VALUE_INT, DefaultDef::None) => OwnedValue::Int(0),
            (abi::VALUE_UINT, DefaultDef::None) => OwnedValue::Uint(0),
            (abi::VALUE_DOUBLE, DefaultDef::None) => OwnedValue::Double(0.0),
            (abi::VALUE_TEXT, DefaultDef::None) => OwnedValue::Text(String::new()),
            (abi::VALUE_NODE, DefaultDef::None) => OwnedValue::Node(0),
            (abi::VALUE_LIST, DefaultDef::None) => OwnedValue::List(Vec::new()),
            (abi::VALUE_TEXTS, DefaultDef::None) => OwnedValue::Texts(Vec::new()),
            (abi::VALUE_BYTES, DefaultDef::None) => OwnedValue::Bytes(Vec::new()),
            _ => return None,
        })
    }
}

/// A `GnoblinValue` with nothing in it.
const fn blank() -> GnoblinValue {
    GnoblinValue {
        size: size_of::<GnoblinValue>() as u32,
        kind: 0,
        flags: 0,
        reserved: 0,
        bits: 0,
        ptr: core::ptr::null(),
        len: 0,
    }
}

/// The arrays the pointers of values lent to one callback point into. Build the values, call the callback, then drop it.
#[derive(Default)]
pub(crate) struct RawScope {
    lists: Vec<Vec<GnoblinValue>>,
    texts: Vec<Vec<GnoblinStr>>,
}

impl RawScope {
    /// The value as the ABI struct. Text, bytes and lists point into `value` and into this scope, both of which must outlive the
    /// callback the struct is passed to.
    pub(crate) fn raw(&mut self, value: &OwnedValue) -> GnoblinValue {
        let mut raw = blank();
        match value {
            OwnedValue::Bool(value) => {
                raw.kind = abi::VALUE_BOOL;
                raw.bits = u64::from(*value);
            }
            OwnedValue::Int(value) => {
                raw.kind = abi::VALUE_INT;
                // Two's complement, as the ABI defines the bit pattern.
                raw.bits = *value as u64;
            }
            OwnedValue::Uint(value) => {
                raw.kind = abi::VALUE_UINT;
                raw.bits = *value;
            }
            OwnedValue::Double(value) => {
                raw.kind = abi::VALUE_DOUBLE;
                raw.bits = value.to_bits();
            }
            OwnedValue::Node(value) => {
                raw.kind = abi::VALUE_NODE;
                raw.bits = *value;
            }
            OwnedValue::Text(text) => {
                raw.kind = abi::VALUE_TEXT;
                raw.ptr = text.as_ptr().cast();
                raw.len = text.len();
            }
            OwnedValue::Bytes(bytes) => {
                raw.kind = abi::VALUE_BYTES;
                raw.ptr = bytes.as_ptr().cast();
                raw.len = bytes.len();
            }
            OwnedValue::List(numbers) => {
                let items: Vec<GnoblinValue> = numbers
                    .iter()
                    .map(|number| {
                        let mut item = blank();
                        item.kind = abi::VALUE_DOUBLE;
                        item.bits = number.to_bits();
                        item
                    })
                    .collect();
                raw.kind = abi::VALUE_LIST;
                raw.len = items.len();
                self.lists.push(items);
                raw.ptr = self.lists.last().map_or(core::ptr::null(), |list| list.as_ptr().cast());
            }
            OwnedValue::Texts(texts) => {
                let items: Vec<GnoblinStr> = texts.iter().map(|text| GnoblinStr::borrowed(text)).collect();
                raw.kind = abi::VALUE_TEXTS;
                raw.len = items.len();
                self.texts.push(items);
                raw.ptr = self.texts.last().map_or(core::ptr::null(), |list| list.as_ptr().cast());
            }
        }
        raw
    }
}

// ---- values the core lends out of a read ------------------------------------------------------------------------------

/// What a call that returns transient data lent last on this thread. The next such call replaces it, which is what "valid until
/// the next call that lends transient data" means.
#[derive(Default)]
struct Arena {
    nodes: Vec<GnoblinNode>,
    name: String,
    list: Vec<GnoblinValue>,
    texts: Vec<GnoblinStr>,
    owned_texts: Vec<String>,
}

thread_local! {
    static ARENA: RefCell<Arena> = RefCell::new(Arena::default());
}

/// Lends `ids` as a node array.
pub(crate) fn lend_nodes(ids: &[GnoblinNode]) -> GnoblinNodes {
    ARENA.with(|arena| {
        let mut arena = arena.borrow_mut();
        arena.nodes = ids.to_vec();
        GnoblinNodes {
            ptr: if arena.nodes.is_empty() {
                core::ptr::null()
            } else {
                arena.nodes.as_ptr()
            },
            len: arena.nodes.len(),
        }
    })
}

/// Lends `text` as a string.
pub(crate) fn lend_name(text: &str) -> GnoblinStr {
    ARENA.with(|arena| {
        let mut arena = arena.borrow_mut();
        text.clone_into(&mut arena.name);
        GnoblinStr {
            ptr: if arena.name.is_empty() {
                core::ptr::null()
            } else {
                arena.name.as_ptr()
            },
            len: arena.name.len(),
        }
    })
}

/// Lends `value` to the shell as the result of a read: text and bytes are owned by the shell (freed with `value_free`), a list is
/// transient.
pub(crate) fn lend_value(value: OwnedValue) -> GnoblinValue {
    match value {
        OwnedValue::Text(text) => memory::lend_text(text),
        OwnedValue::Bytes(bytes) => {
            let buf = memory::lend_bytes(bytes);
            let mut raw = blank();
            raw.kind = abi::VALUE_BYTES;
            raw.flags = if buf.ptr.is_null() { 0 } else { abi::VALUE_OWNED };
            raw.ptr = buf.ptr.cast_const().cast();
            raw.len = buf.len;
            raw
        }
        OwnedValue::List(numbers) => ARENA.with(|arena| {
            let mut arena = arena.borrow_mut();
            arena.list = numbers
                .iter()
                .map(|number| {
                    let mut item = blank();
                    item.kind = abi::VALUE_DOUBLE;
                    item.bits = number.to_bits();
                    item
                })
                .collect();
            let mut raw = blank();
            raw.kind = abi::VALUE_LIST;
            raw.len = arena.list.len();
            raw.ptr = arena.list.as_ptr().cast();
            raw
        }),
        OwnedValue::Texts(texts) => ARENA.with(|arena| {
            let mut arena = arena.borrow_mut();
            arena.owned_texts = texts;
            arena.texts = arena
                .owned_texts
                .iter()
                .map(|text| GnoblinStr::borrowed(text))
                .collect();
            let mut raw = blank();
            raw.kind = abi::VALUE_TEXTS;
            raw.len = arena.texts.len();
            raw.ptr = arena.texts.as_ptr().cast();
            raw
        }),
        other => {
            let mut scope = RawScope::default();
            scope.raw(&other)
        }
    }
}

// ---- values the shell writes ------------------------------------------------------------------------------------------

/// Why a value the shell passed could not be used.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ValueError {
    /// The value's kind is not the property's.
    Type(String),
    /// The value is malformed: a null pointer with a length, text that is not UTF-8, a size too small to read.
    Malformed(String),
    /// A kind the ABI does not carry yet.
    Unsupported(String),
}

impl ValueError {
    /// The ABI code for this failure.
    pub(crate) fn code(&self) -> i32 {
        match self {
            Self::Type(_) => abi::E_TYPE,
            Self::Malformed(_) => abi::E_ERR,
            Self::Unsupported(_) => abi::E_UNSUPPORTED,
        }
    }

    /// The text of the failure.
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Type(text) | Self::Malformed(text) | Self::Unsupported(text) => text,
        }
    }
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        abi::VALUE_BOOL => "bool",
        abi::VALUE_INT => "int",
        abi::VALUE_UINT => "uint",
        abi::VALUE_DOUBLE => "double",
        abi::VALUE_TEXT => "text",
        abi::VALUE_NODE => "node",
        abi::VALUE_LIST => "list",
        abi::VALUE_TEXTS => "texts",
        abi::VALUE_EFFECTS => "effects",
        abi::VALUE_BYTES => "bytes",
        abi::VALUE_SECRET => "secret",
        abi::VALUE_RECORD => "record",
        _ => "an unknown kind",
    }
}

/// The bytes a text, bytes or secret value points at.
///
/// # Safety
///
/// `raw.ptr` must be null or point at `raw.len` readable bytes.
unsafe fn bytes_of(raw: &GnoblinValue) -> Result<&[u8], ValueError> {
    if raw.len == 0 {
        return Ok(&[]);
    }
    if raw.ptr.is_null() {
        return Err(ValueError::Malformed(
            "a value with a nonzero length and no data".to_owned(),
        ));
    }
    // SAFETY: the caller guarantees `ptr` points at `len` readable bytes.
    Ok(unsafe { slice::from_raw_parts(raw.ptr.cast::<u8>(), raw.len) })
}

/// A text value's text.
///
/// # Safety
///
/// As for [`bytes_of`].
unsafe fn text_of(raw: &GnoblinValue) -> Result<String, ValueError> {
    // SAFETY: forwarded to the caller's guarantee.
    let bytes = unsafe { bytes_of(raw)? };
    String::from_utf8(bytes.to_vec())
        .map_err(|error| ValueError::Malformed(format!("text that is not UTF-8 ({error})")))
}

/// Reads the shell's value as a tree value of kind `want` (a `VALUE_*` kind of the catalog), widening a number and accepting a
/// node id as an unsigned integer.
///
/// # Safety
///
/// `raw` must point at a `GnoblinValue` whose `size` is what the shell built, and whose pointers are valid for the call.
pub(crate) unsafe fn read_value(raw: *const GnoblinValue, want: u32) -> Result<MirrorValue, ValueError> {
    // SAFETY: the caller guarantees `raw` is null or a readable value.
    let Some(raw) = (unsafe { raw.as_ref() }) else {
        return Err(ValueError::Malformed("a null value".to_owned()));
    };
    if (raw.size as usize) < size_of::<u32>() * 4 + size_of::<u64>() {
        return Err(ValueError::Malformed(format!(
            "a value of {} bytes is too small to read",
            raw.size
        )));
    }
    let mismatch = || {
        ValueError::Type(format!(
            "a {} value for a {} property",
            kind_name(raw.kind),
            kind_name(want)
        ))
    };
    let number = || -> Option<f64> {
        match raw.kind {
            abi::VALUE_DOUBLE => Some(f64::from_bits(raw.bits)),
            abi::VALUE_INT => Some(raw.bits as i64 as f64),
            abi::VALUE_UINT => Some(raw.bits as f64),
            _ => None,
        }
    };
    match want {
        abi::VALUE_BOOL if raw.kind == abi::VALUE_BOOL => Ok(MirrorValue::Bool(raw.bits != 0)),
        abi::VALUE_INT => match raw.kind {
            abi::VALUE_INT => Ok(MirrorValue::Int(raw.bits as i64)),
            abi::VALUE_UINT => i64::try_from(raw.bits)
                .map(MirrorValue::Int)
                .map_err(|_| ValueError::Type(format!("{} does not fit a signed integer", raw.bits))),
            _ => Err(mismatch()),
        },
        abi::VALUE_UINT => match raw.kind {
            abi::VALUE_UINT => Ok(MirrorValue::Uint(raw.bits)),
            abi::VALUE_INT => u64::try_from(raw.bits as i64)
                .map(MirrorValue::Uint)
                .map_err(|_| ValueError::Type(format!("{} is negative, for an unsigned property", raw.bits as i64))),
            _ => Err(mismatch()),
        },
        abi::VALUE_DOUBLE => number().map(MirrorValue::Double).ok_or_else(mismatch),
        abi::VALUE_NODE => match raw.kind {
            abi::VALUE_NODE | abi::VALUE_UINT => Ok(MirrorValue::Node(raw.bits)),
            _ => Err(mismatch()),
        },
        // SAFETY (text and bytes arms): the caller guarantees the pointer and length are valid for the call.
        abi::VALUE_TEXT if raw.kind == abi::VALUE_TEXT => Ok(MirrorValue::Text(unsafe { text_of(raw)? })),
        abi::VALUE_BYTES if raw.kind == abi::VALUE_BYTES => Ok(MirrorValue::Bytes(unsafe { bytes_of(raw)? }.to_vec())),
        abi::VALUE_LIST if raw.kind == abi::VALUE_LIST => {
            if raw.len == 0 {
                return Ok(MirrorValue::List(Vec::new()));
            }
            if raw.ptr.is_null() {
                return Err(ValueError::Malformed("a list with a length and no data".to_owned()));
            }
            // SAFETY: the caller guarantees a list value points at `len` values.
            let items = unsafe { slice::from_raw_parts(raw.ptr.cast::<GnoblinValue>(), raw.len) };
            items
                .iter()
                .map(|item| match item.kind {
                    abi::VALUE_DOUBLE => Ok(f64::from_bits(item.bits)),
                    abi::VALUE_INT => Ok(item.bits as i64 as f64),
                    abi::VALUE_UINT => Ok(item.bits as f64),
                    other => Err(ValueError::Type(format!(
                        "a {} item in a list of numbers",
                        kind_name(other)
                    ))),
                })
                .collect::<Result<Vec<f64>, _>>()
                .map(MirrorValue::List)
        }
        abi::VALUE_TEXTS if raw.kind == abi::VALUE_TEXTS => {
            if raw.len == 0 {
                return Ok(MirrorValue::Texts(Vec::new()));
            }
            if raw.ptr.is_null() {
                return Err(ValueError::Malformed(
                    "a list of texts with a length and no data".to_owned(),
                ));
            }
            // SAFETY: the caller guarantees a texts value points at `len` strings.
            let items = unsafe { slice::from_raw_parts(raw.ptr.cast::<GnoblinStr>(), raw.len) };
            items
                .iter()
                .map(|item| {
                    if item.len == 0 {
                        return Ok(String::new());
                    }
                    if item.ptr.is_null() {
                        return Err(ValueError::Malformed("a string with a length and no data".to_owned()));
                    }
                    // SAFETY: the caller guarantees each string is `len` readable bytes.
                    let bytes = unsafe { slice::from_raw_parts(item.ptr, item.len) };
                    String::from_utf8(bytes.to_vec())
                        .map_err(|error| ValueError::Malformed(format!("text that is not UTF-8 ({error})")))
                })
                .collect::<Result<Vec<String>, _>>()
                .map(MirrorValue::Texts)
        }
        abi::VALUE_EFFECTS => Err(ValueError::Unsupported(
            "effects are not carried across the ABI yet (a later minor defines the record)".to_owned(),
        )),
        abi::VALUE_SECRET | abi::VALUE_RECORD => Err(ValueError::Unsupported(format!(
            "a {} value cannot be written to a tree property",
            kind_name(want)
        ))),
        _ => Err(mismatch()),
    }
}

/// How a method's arguments reach the wire: the integers (bools, ints, uints and node ids) as little-endian `i64`s packed back to
/// back, in order, and the text arguments joined into the one text a call carries. A seat claim is the example: `claim(id, chord,
/// grab, device)` sends `id` and `grab` as integers and `device`, a line feed and `chord` as its text (just the chord when there
/// is no device). A secret argument is a text argument whose bytes are wiped once the transaction is sent.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct WireArgs {
    /// The packed integers.
    pub args: Vec<u8>,
    /// The call's text.
    pub text: String,
}

/// Packs the arguments `values` (checked against the method's `kinds`) as the wire carries them.
///
/// # Safety
///
/// `values` must be null or point at `n` values the shell built, valid for the call.
pub(crate) unsafe fn read_args(
    values: *const GnoblinValue,
    n: usize,
    kinds: &[u32],
    method: &str,
) -> Result<WireArgs, ValueError> {
    if n != kinds.len() {
        return Err(ValueError::Type(format!(
            "`{method}` takes {} arguments, not {n}",
            kinds.len()
        )));
    }
    if n == 0 {
        return Ok(WireArgs::default());
    }
    if values.is_null() {
        return Err(ValueError::Malformed("arguments with a count and no data".to_owned()));
    }
    // SAFETY: the caller guarantees `values` points at `n` values.
    let items = unsafe { slice::from_raw_parts(values, n) };
    let mut out = WireArgs::default();
    let mut texts: Vec<String> = Vec::new();
    for (item, kind) in items.iter().zip(kinds) {
        match *kind {
            abi::VALUE_TEXT | abi::VALUE_SECRET => {
                if item.kind != abi::VALUE_TEXT && item.kind != abi::VALUE_SECRET {
                    return Err(ValueError::Type(format!(
                        "a {} argument of `{method}` where text belongs",
                        kind_name(item.kind)
                    )));
                }
                // SAFETY: the caller guarantees the argument's pointer and length are valid for the call.
                texts.push(unsafe { text_of(item)? });
            }
            abi::VALUE_BOOL | abi::VALUE_INT | abi::VALUE_UINT | abi::VALUE_NODE => {
                // SAFETY: a scalar kind reads only `bits`.
                let value = unsafe { read_value(item, *kind)? };
                let bits: i64 = match value {
                    MirrorValue::Bool(value) => i64::from(value),
                    MirrorValue::Int(value) => value,
                    MirrorValue::Uint(value) | MirrorValue::Node(value) => i64::try_from(value)
                        .map_err(|_| ValueError::Type(format!("{value} does not fit an argument of `{method}`")))?,
                    _ => 0,
                };
                out.args.extend_from_slice(&bits.to_le_bytes());
            }
            abi::VALUE_DOUBLE => {
                // SAFETY: as above.
                let MirrorValue::Double(value) = (unsafe { read_value(item, *kind)? }) else {
                    return Err(ValueError::Type(format!(
                        "a double argument of `{method}` is malformed"
                    )));
                };
                out.args.extend_from_slice(&value.to_le_bytes());
            }
            other => {
                return Err(ValueError::Unsupported(format!(
                    "`{method}` has an argument of kind {} that cannot be sent as a call argument",
                    kind_name(other)
                )));
            }
        }
    }
    out.text = match texts.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        // The subject and a device: `device`, a line feed, `subject`; the subject alone when there is no device.
        [subject, device] if device.is_empty() => subject.clone(),
        [subject, device] => format!("{device}\n{subject}"),
        more => {
            return Err(ValueError::Unsupported(format!(
                "`{method}` has {} text arguments; a call carries at most two",
                more.len()
            )));
        }
    };
    Ok(out)
}

/// Reads `len` bytes at `ptr` as UTF-8 for the call.
///
/// # Safety
///
/// `ptr` must be null or point at `len` readable bytes.
pub(crate) unsafe fn read_text(ptr: *const u8, len: usize, what: &str) -> Result<String, ValueError> {
    if len == 0 {
        return Ok(String::new());
    }
    if ptr.is_null() {
        return Err(ValueError::Malformed(format!(
            "{what}: a null string with a nonzero length"
        )));
    }
    // SAFETY: the caller guarantees `ptr` points at `len` readable bytes.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    String::from_utf8(bytes.to_vec())
        .map_err(|error| ValueError::Malformed(format!("{what}: not valid UTF-8 ({error})")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(kind: u32, bits: u64) -> GnoblinValue {
        let mut raw = blank();
        raw.kind = kind;
        raw.bits = bits;
        raw
    }

    fn text(value: &str) -> GnoblinValue {
        let mut raw = blank();
        raw.kind = abi::VALUE_TEXT;
        raw.ptr = value.as_ptr().cast();
        raw.len = value.len();
        raw
    }

    #[test]
    fn a_number_widens_and_a_wrong_kind_is_a_type_error() {
        // SAFETY: scalar values read only `bits`.
        unsafe {
            assert_eq!(
                read_value(&scalar(abi::VALUE_INT, (-3i64) as u64), abi::VALUE_DOUBLE),
                Ok(MirrorValue::Double(-3.0))
            );
            assert_eq!(
                read_value(&scalar(abi::VALUE_UINT, 7), abi::VALUE_INT),
                Ok(MirrorValue::Int(7))
            );
            assert_eq!(
                read_value(&scalar(abi::VALUE_INT, (-1i64) as u64), abi::VALUE_UINT)
                    .unwrap_err()
                    .code(),
                abi::E_TYPE
            );
            assert_eq!(
                read_value(&scalar(abi::VALUE_BOOL, 1), abi::VALUE_DOUBLE)
                    .unwrap_err()
                    .code(),
                abi::E_TYPE
            );
            assert_eq!(
                read_value(&scalar(abi::VALUE_UINT, 5), abi::VALUE_NODE),
                Ok(MirrorValue::Node(5))
            );
        }
    }

    #[test]
    fn text_and_lists_are_copied_out_of_the_shells_memory() {
        let words = [GnoblinStr::borrowed("a"), GnoblinStr::borrowed("bc")];
        let mut list = blank();
        list.kind = abi::VALUE_TEXTS;
        list.ptr = words.as_ptr().cast();
        list.len = words.len();
        // SAFETY: the pointers describe live locals.
        unsafe {
            assert_eq!(
                read_value(&text("title"), abi::VALUE_TEXT),
                Ok(MirrorValue::Text("title".to_owned()))
            );
            assert_eq!(
                read_value(&list, abi::VALUE_TEXTS),
                Ok(MirrorValue::Texts(vec!["a".to_owned(), "bc".to_owned()]))
            );
            assert!(matches!(
                read_value(&scalar(abi::VALUE_EFFECTS, 0), abi::VALUE_EFFECTS),
                Err(ValueError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn a_seat_claims_arguments_become_integers_and_one_text() {
        let items = [
            scalar(abi::VALUE_INT, 3),
            text("Super+Return"),
            scalar(abi::VALUE_BOOL, 1),
            text("kbd0"),
        ];
        let kinds = [abi::VALUE_INT, abi::VALUE_TEXT, abi::VALUE_BOOL, abi::VALUE_TEXT];
        // SAFETY: `items` is a live array of four values.
        let wire = unsafe { read_args(items.as_ptr(), 4, &kinds, "claim") }.unwrap();
        let mut packed = 3i64.to_le_bytes().to_vec();
        packed.extend_from_slice(&1i64.to_le_bytes());
        assert_eq!(wire.args, packed);
        assert_eq!(wire.text, "kbd0\nSuper+Return");

        let no_device = [
            scalar(abi::VALUE_INT, 3),
            text("Super+Return"),
            scalar(abi::VALUE_BOOL, 0),
            text(""),
        ];
        // SAFETY: as above.
        let wire = unsafe { read_args(no_device.as_ptr(), 4, &kinds, "claim") }.unwrap();
        assert_eq!(wire.text, "Super+Return");
        // SAFETY: as above.
        let short = unsafe { read_args(no_device.as_ptr(), 3, &kinds, "claim") };
        assert_eq!(short.unwrap_err().code(), abi::E_TYPE);
    }

    #[test]
    fn lent_text_is_owned_and_a_list_is_transient() {
        let mut value = lend_value(OwnedValue::Text("owned".to_owned()));
        assert_eq!(value.flags & abi::VALUE_OWNED, abi::VALUE_OWNED);
        assert_eq!(memory::free_value(&mut value), abi::E_OK);
        let list = lend_value(OwnedValue::List(vec![1.0, 2.5]));
        assert_eq!(list.kind, abi::VALUE_LIST);
        assert_eq!(list.len, 2);
        assert_eq!(list.flags & abi::VALUE_OWNED, 0);
    }

    #[test]
    fn a_default_is_the_schemas_or_the_empty_value_of_the_kind() {
        assert_eq!(
            OwnedValue::default_of(abi::VALUE_DOUBLE, DefaultDef::Double(1.0)),
            Some(OwnedValue::Double(1.0))
        );
        assert_eq!(
            OwnedValue::default_of(abi::VALUE_TEXT, DefaultDef::None),
            Some(OwnedValue::Text(String::new()))
        );
        assert_eq!(OwnedValue::default_of(abi::VALUE_EFFECTS, DefaultDef::None), None);
    }
}
