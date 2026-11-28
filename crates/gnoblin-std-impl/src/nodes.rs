//! Reads and writes of the tree's nodes the library makes through the core SDK.
//!
//! A read of a node that is gone, or of a property the schema lacks, yields the empty value of its kind instead of a failure: the
//! library asks about windows and outputs that may close in the same batch, and a layout pass must not abort on one of them. A
//! write that fails is logged by the caller's guard; the next pass tries again.

use gnoblin_core::host::{Host, NodeId, Rect, Value};
#[cfg(feature = "testing")]
use std::cell::Cell;

use crate::animation::staging::{self, Prop};
use crate::state::NodeId as Id;

#[cfg(feature = "testing")]
thread_local! {
    static ACCESSES: Cell<u64> = const { Cell::new(0) };
}

/// How many times this thread's library has asked the core for a node since it started. A test that says a turn reads no window
/// compares it before and after the turn (`gnoblin_std_impl::native_accesses`).
#[cfg(feature = "testing")]
pub(crate) fn accesses() -> u64 {
    ACCESSES.with(Cell::get)
}

/// Notes one trip to the core, for [`accesses`]; nothing without the testing feature.
#[inline]
fn counted() {
    #[cfg(feature = "testing")]
    ACCESSES.with(|count| count.set(count.get() + 1));
}

/// A handle on the node `id`.
fn node(g: &Host, id: Id) -> gnoblin_core::host::Node {
    counted();
    g.node(NodeId(id))
}

/// Whether the node is in the tree as this turn sees it.
pub(crate) fn alive(g: &Host, id: Id) -> bool {
    node(g, id).is_alive()
}

/// A number property, `0` when it cannot be read.
pub(crate) fn number(g: &Host, id: Id, key: &str) -> f64 {
    if let Some(Prop::Number(staged)) = staging::peek(id, key) {
        return staged;
    }
    node(g, id).get_f64(key).unwrap_or(0.0)
}

/// A boolean property, `false` when it cannot be read.
pub(crate) fn flag(g: &Host, id: Id, key: &str) -> bool {
    flag_or(g, id, key, false)
}

/// A boolean property, `default` when it cannot be read.
pub(crate) fn flag_or(g: &Host, id: Id, key: &str, default: bool) -> bool {
    if let Some(Prop::Bool(staged)) = staging::peek(id, key) {
        return staged;
    }
    node(g, id).get_bool(key).unwrap_or(default)
}

/// A text property, empty when it cannot be read.
pub(crate) fn text(g: &Host, id: Id, key: &str) -> String {
    node(g, id).get_text(key).unwrap_or_default()
}

/// A property that refers to another node: `None` for nil and for what cannot be read.
pub(crate) fn node_ref(g: &Host, id: Id, key: &str) -> Option<Id> {
    node(g, id).get_node(key).ok().flatten().map(|other| other.id().0)
}

/// Whether the window is drawn: a window nothing has hidden is.
pub(crate) fn visible(g: &Host, id: Id) -> bool {
    flag_or(g, id, "visible", true)
}

/// The window's rectangle: what the shell wrote, or what the client committed until it did.
pub(crate) fn rect_of(g: &Host, id: Id) -> Rect {
    Rect::new(
        number(g, id, "x"),
        number(g, id, "y"),
        number(g, id, "width"),
        number(g, id, "height"),
    )
}

/// A list of numbers (`rotation`, `scale`) as three components, `fallback` when it is not three numbers.
pub(crate) fn triple(g: &Host, id: Id, key: &str, fallback: [f64; 3]) -> [f64; 3] {
    if let Some(Prop::Triple(staged)) = staging::peek(id, key) {
        return staged;
    }
    match node(g, id).get(key) {
        Ok(Value::List(list)) if list.len() == 3 => [list[0], list[1], list[2]],
        _ => fallback,
    }
}

/// A write of the library's own: held while a transition is staging, else made at once (and a transition that played the property lets
/// it go).
fn put(g: &Host, id: Id, key: &str, value: Prop) {
    if staging::capture(id, key, &value) {
        return;
    }
    staging::released(id, key);
    let _ = node(g, id).set(key, value.to_value());
}

/// Writes a number property when it differs from what the node holds.
pub(crate) fn write_num(g: &Host, id: Id, key: &str, value: f64) {
    if number(g, id, key) != value {
        put(g, id, key, Prop::Number(value));
    }
}

/// Writes the four rectangle properties whether or not they differ from what the window holds: a window the compositor has not given
/// a place reads as its committed size, and a placement that only repeats it would leave the place unset.
pub(crate) fn write_rect_all(g: &Host, id: Id, rect: &Rect) {
    for (key, value) in [
        ("x", rect.x),
        ("y", rect.y),
        ("width", rect.width),
        ("height", rect.height),
    ] {
        put(g, id, key, Prop::Number(value));
    }
}

/// Writes a boolean property when it differs from what the node holds.
pub(crate) fn write_flag(g: &Host, id: Id, key: &str, value: bool) {
    let held = match staging::peek(id, key) {
        Some(Prop::Bool(staged)) => Some(staged),
        _ => node(g, id).get_bool(key).ok(),
    };
    if held != Some(value) {
        put(g, id, key, Prop::Bool(value));
    }
}

/// Writes a three-component property when it differs from what the node holds.
pub(crate) fn write_triple(g: &Host, id: Id, key: &str, value: [f64; 3], default: [f64; 3]) {
    if triple(g, id, key, default) != value {
        put(g, id, key, Prop::Triple(value));
    }
}

/// The first seat of the tree, or `None` for a tree with no seat (nothing then has the keyboard, and nothing is asked to).
pub(crate) fn seat(g: &Host) -> Option<gnoblin_core::host::Node> {
    counted();
    let seat = g.class("seat").ok()?;
    g.root()
        .ok()?
        .children()
        .ok()?
        .into_iter()
        .find(|child| child.class().ok() == Some(seat))
}

/// The window that has the keyboard, among `clients`: the one the seat's focus names, as a shell gives focus by writing the seat's
/// `focus` and a compositor was seen not to keep a window's own `focused` current while a shell holds the tree. A tree with no seat
/// has no seat focus to read, so the window whose own `focused` is set is the answer.
pub(crate) fn focused_window(g: &Host, clients: &[Id]) -> Option<Id> {
    match seat(g) {
        Some(seat) => seat
            .get_node("focus")
            .ok()
            .flatten()
            .map(|focus| focus.id().0)
            .filter(|focus| clients.contains(focus)),
        None => clients.iter().copied().find(|client| flag(g, *client, "focused")),
    }
}

/// Writes the four rectangle properties that differ from the window's own.
pub(crate) fn write_rect(g: &Host, id: Id, rect: &Rect) {
    write_num(g, id, "x", rect.x);
    write_num(g, id, "y", rect.y);
    write_num(g, id, "width", rect.width);
    write_num(g, id, "height", rect.height);
}

/// The seat's pointer, as the seat reports it (`pointer_x` and `pointer_y`), or `None` for a tree with no seat.
pub(crate) fn pointer(g: &Host) -> Option<(f64, f64)> {
    let seat = seat(g)?;
    Some((seat.get_f64("pointer_x").ok()?, seat.get_f64("pointer_y").ok()?))
}
