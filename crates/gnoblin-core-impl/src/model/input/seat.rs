//! What the input module reads and writes of the seats: their names, pointer and focus, the settings the compositor published and the
//! device lists. Each is the schema's default until the compositor says otherwise.

use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_tree_client::tree_mirror::{MirrorValue, NIL, TxAnimation};

use super::super::values::{self, live, nodes_of, tree_class};
use crate::session;

/// The seat nodes, in the order the tree made them.
pub(super) fn seats() -> Vec<u64> {
    nodes_of("seat")
}

fn read(seat: u64, key: &str) -> crate::values::OwnedValue {
    live(seat, tree_class("seat"), key)
}

pub(super) fn name(seat: u64) -> String {
    read(seat, "name").string()
}

/// The pointer as the compositor last published it.
pub(super) fn pointer(seat: u64) -> (f64, f64) {
    (read(seat, "pointer_x").number(), read(seat, "pointer_y").number())
}

/// The node with keyboard focus, or 0.
pub(super) fn focus(seat: u64) -> u64 {
    read(seat, "focus").node()
}

/// The seat's `drag_constraint`, `work_area` or `minimum_strip`, as the compositor configured it. Any other text is passed on as it is.
pub(super) fn drag_constraint(seat: u64) -> String {
    let text = read(seat, "drag_constraint").string();
    if text.is_empty() { "work_area".to_owned() } else { text }
}

/// The seat's `edge_tiling`: whether a drop at the edge of the screen tiles a window.
pub(super) fn edge_tiling(seat: u64) -> bool {
    read(seat, "edge_tiling").flag()
}

/// The devices of a seat, one `id:name:caps` item each.
pub(super) fn devices(seat: u64) -> Vec<String> {
    match read(seat, "device") {
        crate::values::OwnedValue::Texts(items) => items,
        _ => Vec::new(),
    }
}

/// One `id:name:caps` item of a seat's device list: the id up to the first colon, the capabilities after the last.
pub(super) fn split_device(item: &str) -> Option<(String, String, Vec<String>)> {
    let first = item.find(':')?;
    let last = item.rfind(':')?;
    if first == 0 || last <= first {
        return None;
    }
    let caps = &item[last + 1..];
    Some((
        item[..first].to_owned(),
        item[first + 1..last].to_owned(),
        if caps.is_empty() {
            Vec::new()
        } else {
            caps.split(',').map(str::to_owned).collect()
        },
    ))
}

/// The seat of the desktop's own keyboard and pointer: the first one that is not the XR program's.
pub(super) fn main_seat() -> Option<u64> {
    seats().into_iter().find(|seat| name(*seat) != "xr")
}

/// Whether the seat asks for reduced motion.
pub(super) fn reduced_motion(seat: u64) -> bool {
    read(seat, "reduced_motion").flag()
}

/// Whether the compositor accepts a shader effect from a shell, as the seat publishes it.
pub(super) fn user_shaders(seat: u64) -> bool {
    read(seat, "user_shaders").flag()
}

/// The settings record of a seat.
pub(super) fn settings(cx: &Cx<'_>, seat: u64) -> Res<Val> {
    let flag = |key: &str| Val::Bool(read(seat, key).flag());
    let word = |key: &str| Val::Text(read(seat, key).string());
    values::record(
        cx,
        "seat_settings",
        vec![
            ("edge_tiling", flag("edge_tiling")),
            ("focus_mode", word("focus_mode")),
            ("focus_new_windows", word("focus_new_windows")),
            ("raise_on_click", flag("raise_on_click")),
            ("auto_raise", flag("auto_raise")),
            ("auto_raise_delay", Val::Double(read(seat, "auto_raise_delay").number())),
            ("focus_change_on_pointer_rest", flag("focus_change_on_pointer_rest")),
            ("reduced_motion", flag("reduced_motion")),
            ("builtin_motion", flag("builtin_motion")),
        ],
    )
}

/// Whether the compositor plays its own animations on window nodes.
pub(super) fn builtin_motion(seat: u64) -> bool {
    read(seat, "builtin_motion").flag()
}

fn write(seat: u64, key: &str, value: MirrorValue) -> Res<()> {
    session::with(|session| {
        session
            .tree()
            .map_err(|(code, text)| Fail::new(code, text))?
            .set(seat, key, value, TxAnimation::default())
            .map_err(|error| Fail::new(abi::E_NO_SUCH_NODE, format!("Seat#{seat}.{key}: {error}")))
    })
}

/// Queues the seat's keyboard focus going to no window.
pub(super) fn clear_focus(seat: u64) -> Res<()> {
    write(seat, "focus", MirrorValue::Node(NIL))
}

/// Queues a write of the seat's `builtin_motion`.
pub(super) fn set_builtin_motion(seat: u64, enabled: bool) -> Res<()> {
    write(seat, "builtin_motion", MirrorValue::Bool(enabled))
}

/// The client object of the window `node`, when the model has it.
pub(super) fn client_of(cx: &Cx<'_>, node: u64) -> Option<Obj> {
    if node == 0 {
        return None;
    }
    super::super::with_world(cx, |world| world.clients.get(&node).copied())
        .ok()
        .flatten()
}

/// The windows back to front, as the tree stacks them.
pub(super) fn stack(cx: &Cx<'_>) -> Vec<Obj> {
    let nodes: Vec<u64> = session::with(|session| {
        session
            .tree_ref()
            .ok()
            .and_then(|tree| tree.children_of(tree.root()).map(<[u64]>::to_vec))
            .unwrap_or_default()
    });
    nodes.into_iter().filter_map(|node| client_of(cx, node)).collect()
}
