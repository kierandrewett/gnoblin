//! What a script reads and writes of the seats: their names, pointer and focus, the settings the compositor
//! published, and the device lists.

use gnoblin_tree_client::tree_mirror::{MirrorValue, NIL, NodeId, class_name};
use rquickjs::{Ctx, IntoJs, Object, Value};

use crate::bindings::{HostState, parents_first};

/// The seat nodes, in tree order.
pub(crate) fn seat_nodes(state: &HostState) -> Vec<NodeId> {
    parents_first(&state.mirror)
        .into_iter()
        .filter(|id| {
            state
                .mirror
                .node(*id)
                .is_some_and(|record| class_name(&record.class) == "Seat")
        })
        .collect()
}

/// The seat nodes as the numbers a script holds.
pub(super) fn seats(state: &HostState) -> Vec<f64> {
    seat_nodes(state)
        .into_iter()
        .map(|id| state.handles.to_script(id))
        .collect()
}

fn seat_prop<'a>(state: &'a HostState, seat: NodeId, key: &str) -> Option<&'a MirrorValue> {
    let record = state.mirror.node(seat)?;
    (class_name(&record.class) == "Seat")
        .then(|| record.props.get(key))
        .flatten()
}

pub(super) fn seat_name(state: &HostState, seat: NodeId) -> String {
    match seat_prop(state, seat, "name") {
        Some(MirrorValue::Text(name)) => name.clone(),
        _ => String::new(),
    }
}

/// The pointer as the compositor last published it; the origin before it has published one.
pub(super) fn pointer(state: &HostState, seat: NodeId) -> Vec<f64> {
    let axis = |key: &str| match seat_prop(state, seat, key) {
        Some(MirrorValue::Double(value)) => *value,
        _ => 0.0,
    };
    vec![axis("pointer_x"), axis("pointer_y")]
}

/// The node with keyboard focus, or 0.
pub(super) fn focus(state: &HostState, seat: NodeId) -> f64 {
    match seat_prop(state, seat, "focus") {
        Some(MirrorValue::Node(node)) => state.handles.to_script(*node),
        _ => 0.0,
    }
}

/// The seat's drag constraint, `work_area` or `minimum_strip`, as the compositor configured it. Any other text
/// is passed on as it is, so the script that reads it can refuse it by name.
pub(super) fn drag_constraint(state: &HostState, seat: NodeId) -> String {
    match seat_prop(state, seat, "drag_constraint") {
        Some(MirrorValue::Text(text)) => text.clone(),
        _ => "work_area".to_owned(),
    }
}

/// The window management settings the compositor publishes on a seat, which the stock focus policy and edge tiling
/// read. Each is the schema's default (`crates/gnoblin-schema/schema/tree/seat.toml`) until the compositor says
/// otherwise, and the values are passed on as they are: the script's checks refuse a word it does not know, by name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JsSeatSettings {
    pub(super) edge_tiling: bool,
    pub(super) focus_mode: String,
    pub(super) focus_new_windows: String,
    pub(super) raise_on_click: bool,
    pub(super) auto_raise: bool,
    pub(super) auto_raise_delay: f64,
    pub(super) focus_change_on_pointer_rest: bool,
    /// `appearance.reduced-motion`: the shell shortens its animations to zero while it is true.
    pub(super) reduced_motion: bool,
    /// Whether the compositor plays its own window animations (`builtin_motion`).
    pub(super) builtin_motion: bool,
}

impl<'js> IntoJs<'js> for JsSeatSettings {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("edgeTiling", self.edge_tiling)?;
        object.set("focusMode", self.focus_mode)?;
        object.set("focusNewWindows", self.focus_new_windows)?;
        object.set("raiseOnClick", self.raise_on_click)?;
        object.set("autoRaise", self.auto_raise)?;
        object.set("autoRaiseDelay", self.auto_raise_delay)?;
        object.set("focusChangeOnPointerRest", self.focus_change_on_pointer_rest)?;
        object.set("reducedMotion", self.reduced_motion)?;
        object.set("builtinMotion", self.builtin_motion)?;
        Ok(object.into_value())
    }
}

/// Queues a write of the seat's keyboard focus: `target` is a node, or `NIL` for none. Refused when `seat` is not a
/// live seat node.
fn write_focus(state: &mut HostState, seat: NodeId, target: NodeId) -> Option<&'static str> {
    let HostState { mirror, tick, .. } = state;
    let is_seat = tick
        .node(mirror, seat)
        .is_some_and(|record| class_name(&record.class) == "Seat");
    if !is_seat {
        return Some("unknown-node");
    }
    tick.set(mirror, seat, "focus", MirrorValue::Node(target), None, None)
        .err()
        .map(|_| "unknown-node")
}

/// Queues the seat's keyboard focus going to no node: the stock follow-mouse policy does it when the pointer leaves
/// the focused window for the desktop.
pub(super) fn clear_focus(state: &mut HostState, seat: NodeId) -> Option<&'static str> {
    write_focus(state, seat, NIL)
}

/// Queues a write of the flag `key` of `seat`. Refused when `seat` is not a live seat node.
fn write_flag(state: &mut HostState, seat: NodeId, key: &str, value: bool) -> Option<&'static str> {
    let HostState { mirror, tick, .. } = state;
    let is_seat = tick
        .node(mirror, seat)
        .is_some_and(|record| class_name(&record.class) == "Seat");
    if !is_seat {
        return Some("unknown-node");
    }
    tick.set(mirror, seat, key, MirrorValue::Bool(value), None, None)
        .err()
        .map(|_| "unknown-node")
}

/// Queues a write of the seat's `decorations_available`: whether this shell draws window decorations. The compositor
/// answers a client's server-side request with server-side only while it is true, and clears it when the shell's
/// connection drops. Refused when `seat` is not a live seat node.
pub(super) fn declare_decorations(state: &mut HostState, seat: NodeId, available: bool) -> Option<&'static str> {
    write_flag(state, seat, "decorations_available", available)
}

/// Queues a write of the seat's `builtin_motion`: whether the compositor plays its own animations on window nodes. The
/// compositor sets it back to true when the shell lets go of the tree. Refused when `seat` is not a live seat node.
pub(super) fn set_builtin_motion(state: &mut HostState, seat: NodeId, enabled: bool) -> Option<&'static str> {
    write_flag(state, seat, "builtin_motion", enabled)
}

/// Queues the first seat's keyboard focus going to `window`. The compositor answers no `focus` call on a window; the
/// focus is the seat's, and a shell gives it by writing the seat's `focus` (the retired shell's `seat.focus = window`).
/// Refused with `no-seat` when the tree has no seat.
pub(crate) fn focus_window(state: &mut HostState, window: NodeId) -> Option<&'static str> {
    let Some(seat) = seat_nodes(state).first().copied() else {
        return Some("no-seat");
    };
    write_focus(state, seat, window)
}

pub(super) fn devices(state: &HostState, seat: NodeId) -> Vec<String> {
    match seat_prop(state, seat, "device") {
        Some(MirrorValue::Texts(items)) => items.clone(),
        _ => Vec::new(),
    }
}

impl JsSeatSettings {
    pub(super) fn of(state: &HostState, seat: NodeId) -> Self {
        let flag = |key: &str, default: bool| match seat_prop(state, seat, key) {
            Some(MirrorValue::Bool(value)) => *value,
            _ => default,
        };
        let word = |key: &str, default: &str| match seat_prop(state, seat, key) {
            Some(MirrorValue::Text(text)) => text.clone(),
            _ => default.to_owned(),
        };
        let delay = match seat_prop(state, seat, "auto_raise_delay") {
            Some(MirrorValue::Int(ms)) => *ms as f64,
            Some(MirrorValue::Uint(ms)) => *ms as f64,
            Some(MirrorValue::Double(ms)) => *ms,
            _ => 500.0,
        };
        Self {
            edge_tiling: flag("edge_tiling", false),
            focus_mode: word("focus_mode", "click"),
            focus_new_windows: word("focus_new_windows", "prevent"),
            raise_on_click: flag("raise_on_click", true),
            auto_raise: flag("auto_raise", false),
            auto_raise_delay: delay,
            focus_change_on_pointer_rest: flag("focus_change_on_pointer_rest", false),
            reduced_motion: flag("reduced_motion", false),
            builtin_motion: flag("builtin_motion", true),
        }
    }
}
