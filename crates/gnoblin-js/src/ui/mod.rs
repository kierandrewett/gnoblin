//! The host functions of the UI toolkit (`data/js/gnoblin/ui.ts`): `__gnoblinHost.ui`.
//!
//! The toolkit's canvases are nodes of the compositor's tree that the shell makes itself. This module makes them in
//! the frame's transaction (`create`, `insert`, `destroy`), writes their geometry and their encoded paint list
//! (`set`, `paint`), asks the compositor to measure text (`measure`) and hands the answers and the pointer events
//! that hit a canvas back to the script (`takeAnswers`, `takeEvents`). A script may only touch nodes this module
//! made: the tree's other nodes belong to the compositor.
//!
//! Like the other host functions, each is a thin closure over a free function on [`HostState`], and a refusal is a
//! short code string, so the logic is testable without an engine.
//!
//! `install` puts the functions below on the host bridge; the rest of this file is what they do.

use std::collections::HashSet;

use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NIL, NodeId, TreeEvent, TxError, class_name};
use rquickjs::{Ctx, IntoJs, Object, Value};

use crate::bindings::{HostState, NodeHandles, adopt_windows, parents_first};

mod install;

pub(crate) use self::install::install;

/// A shell makes the ids of its nodes from the epoch it attached under, so two shells never make the same id: the
/// epoch in the high bits, a counter in the low ones. The same split `the retired retired host`'s shell uses.
const EPOCH_SHIFT: u32 = 32;
const EPOCH_LIMIT: u64 = 1 << 31;

/// The class of the nodes the toolkit makes.
const CANVAS: &str = "Canvas";

/// The properties of a canvas a script may write as a number, and as a flag.
const NUMBER_KEYS: &[&str] = &["x", "y", "width", "height", "opacity"];
const FLAG_KEYS: &[&str] = &["visible", "input", "clip", "secure"];

/// The event kinds that reach a canvas: the pointer's, and a key (a canvas holds the seat's keyboard focus when
/// the script asks for it, as a menu or an entry does).
const POINTER_KINDS: &[&str] = &["press", "release", "motion", "scroll", "enter", "leave"];
const KEY_KIND: &str = "key";
/// A request the compositor makes of a window (`menu` for its window menu) is for the shell whatever the node.
const REQUEST_KIND: &str = "request";

/// The seat properties the toolkit may read: the pointer, and what the titlebar's clicks do.
const SEAT_NUMBERS: &[&str] = &["pointer_x", "pointer_y"];
const SEAT_TEXTS: &[(&str, &str)] = &[
    ("double_click", "toggle-maximize"),
    ("middle_click", "lower"),
    ("right_click", "menu"),
];

/// The compositor's answer to a measurement, as the script reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JsUiAnswer {
    token: f64,
    result: String,
    error: String,
}

impl<'js> IntoJs<'js> for JsUiAnswer {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("token", self.token)?;
        object.set("result", self.result)?;
        object.set("error", self.error)?;
        Ok(object.into_value())
    }
}

/// An event for the toolkit, as the script reads it: a pointer event that hit a canvas, a key sent to the canvas that
/// holds the keyboard focus, or a request the compositor made of a window.
#[derive(Clone, PartialEq)]
pub(crate) struct JsUiEvent {
    node: f64,
    kind: String,
    x: f64,
    y: f64,
    button: f64,
    dx: f64,
    dy: f64,
    key: String,
    text: String,
    pressed: bool,
    request: String,
}

/// What a key typed is never printed: an entry can be a passphrase's, so a log shows how long the key and the text are
/// and nothing of them.
impl std::fmt::Debug for JsUiEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsUiEvent")
            .field("node", &self.node)
            .field("kind", &self.kind)
            .field("x", &self.x)
            .field("y", &self.y)
            .field("button", &self.button)
            .field("dx", &self.dx)
            .field("dy", &self.dy)
            .field("key", &format_args!("<{} bytes>", self.key.len()))
            .field("text", &format_args!("<{} bytes>", self.text.len()))
            .field("pressed", &self.pressed)
            .field("request", &self.request)
            .finish()
    }
}

impl<'js> IntoJs<'js> for JsUiEvent {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("node", self.node)?;
        object.set("kind", self.kind)?;
        object.set("x", self.x)?;
        object.set("y", self.y)?;
        object.set("button", self.button)?;
        object.set("dx", self.dx)?;
        object.set("dy", self.dy)?;
        object.set("key", self.key)?;
        object.set("text", self.text)?;
        object.set("pressed", self.pressed)?;
        object.set("request", self.request)?;
        Ok(object.into_value())
    }
}

/// What the toolkit keeps on the host between frames.
#[derive(Default)]
pub(crate) struct JsUiState {
    /// The count of ids made, the low bits of the next one.
    counter: u64,
    /// The nodes the toolkit made and has not destroyed. Nothing else is writable through this module.
    owned: HashSet<NodeId>,
    /// The tokens of measurements asked and not yet answered.
    measures: HashSet<u64>,
    answers: Vec<JsUiAnswer>,
    events: Vec<JsUiEvent>,
}

impl JsUiState {
    /// Keeps the answer to a measurement this module asked. Returns false for any other call's answer.
    pub(crate) fn accept_answer(&mut self, token: u64, result: &[u8], error: &str) -> bool {
        if !self.measures.remove(&token) {
            return false;
        }
        self.answers.push(JsUiAnswer {
            token: token as f64,
            result: String::from_utf8_lossy(result).into_owned(),
            error: error.to_owned(),
        });
        true
    }

    /// Keeps an event for the toolkit: a pointer event or a key addressed to a canvas of ours, or a request made of any
    /// node (a window's `menu`). Any other event is not the toolkit's.
    pub(crate) fn accept_event(&mut self, event: &TreeEvent, handles: &NodeHandles) -> bool {
        let kind = event.kind.as_str();
        let wanted = if kind == REQUEST_KIND {
            true
        } else {
            self.owned.contains(&event.node) && (kind == KEY_KIND || POINTER_KINDS.contains(&kind))
        };
        if !wanted {
            return false;
        }
        self.events.push(JsUiEvent {
            node: handles.to_script(event.node),
            kind: event.kind.clone(),
            x: event.fields.x,
            y: event.fields.y,
            button: event.fields.button as f64,
            dx: event.fields.dx,
            dy: event.fields.dy,
            key: event.fields.key.clone(),
            text: event.fields.text.clone(),
            pressed: event.fields.pressed,
            request: event.fields.request.clone(),
        });
        true
    }

    /// The next id this shell may give a node it creates: the epoch and a count, so no two nodes of one shell share
    /// one, and none is the id of a node the compositor made. The XR module draws from the same count.
    pub(crate) fn mint(&mut self, epoch: u64) -> Option<NodeId> {
        if epoch >= EPOCH_LIMIT || self.counter + 1 >= 1 << EPOCH_SHIFT {
            return None;
        }
        self.counter += 1;
        Some((epoch << EPOCH_SHIFT) | self.counter)
    }

    /// Forgets nodes the compositor destroyed: a window's destruction takes its canvases with it.
    pub(crate) fn forget(&mut self, nodes: &[NodeId]) {
        for node in nodes {
            self.owned.remove(node);
        }
    }
}

/// Makes a canvas node in this tick and returns its id, or `None` when the shell has no id left.
pub(crate) fn create(state: &mut HostState) -> Option<NodeId> {
    let HostState { mirror, tick, ui, .. } = state;
    let id = ui.mint(mirror.epoch())?;
    tick.new_node(mirror, id, CANVAS).ok()?;
    ui.owned.insert(id);
    Some(id)
}

/// Whether `node` is a canvas of ours that this tick still sees.
fn owned_canvas(state: &HostState, node: NodeId) -> Result<(), &'static str> {
    if !state.ui.owned.contains(&node) {
        return Err(if state.tick.node(&state.mirror, node).is_some() {
            "not-owned"
        } else {
            "unknown-node"
        });
    }
    match state.tick.node(&state.mirror, node) {
        Some(record) if class_name(&record.class) == CANVAS => Ok(()),
        _ => Err("unknown-node"),
    }
}

/// Puts `node` (a canvas of ours) under `parent` (any node of the tree) at `index`; a negative index is last.
pub(crate) fn insert(state: &mut HostState, node: NodeId, parent: NodeId, index: i64) -> Option<&'static str> {
    if let Err(code) = owned_canvas(state, node) {
        return Some(code);
    }
    let HostState { mirror, tick, .. } = state;
    tick.insert(mirror, node, parent, index).err().map(|error| match error {
        TxError::UnknownNode(_) => "unknown-node",
        TxError::Cycle(_) => "cycle",
        TxError::Root(_) | TxError::Exists(_) => "refused",
    })
}

/// Destroys a canvas of ours and its subtree. A node that is already gone is not an error: the compositor may have
/// destroyed it with its window.
pub(crate) fn destroy(state: &mut HostState, node: NodeId) -> Option<&'static str> {
    match owned_canvas(state, node) {
        Err("unknown-node") => {
            state.ui.owned.remove(&node);
            return None;
        }
        Err(code) => return Some(code),
        Ok(()) => {}
    }
    let HostState { mirror, tick, ui, .. } = state;
    match tick.destroy(mirror, node) {
        Ok(destroyed) => {
            ui.owned.remove(&node);
            ui.forget(&destroyed.removed);
            None
        }
        Err(TxError::UnknownNode(_)) => {
            ui.owned.remove(&node);
            None
        }
        Err(_) => Some("refused"),
    }
}

/// Queues a write of a numeric property of a canvas of ours.
pub(crate) fn set_number(state: &mut HostState, node: NodeId, key: &str, value: f64) -> Option<&'static str> {
    if !NUMBER_KEYS.contains(&key) {
        return Some("unknown-property");
    }
    if !value.is_finite() {
        return Some("invalid-value");
    }
    write(state, node, key, MirrorValue::Double(value))
}

/// Queues a write of a flag property of a canvas of ours.
pub(crate) fn set_flag(state: &mut HostState, node: NodeId, key: &str, value: bool) -> Option<&'static str> {
    if !FLAG_KEYS.contains(&key) {
        return Some("unknown-property");
    }
    write(state, node, key, MirrorValue::Bool(value))
}

/// Queues a write of a canvas's encoded paint list. The compositor decodes it and refuses one that is malformed.
pub(crate) fn paint(state: &mut HostState, node: NodeId, bytes: Vec<u8>) -> Option<&'static str> {
    write(state, node, "content", MirrorValue::Bytes(bytes))
}

fn write(state: &mut HostState, node: NodeId, key: &str, value: MirrorValue) -> Option<&'static str> {
    if let Err(code) = owned_canvas(state, node) {
        return Some(code);
    }
    let HostState { mirror, tick, .. } = state;
    tick.set(mirror, node, key, value, None, None)
        .err()
        .map(|_| "unknown-node")
}

/// Asks the compositor to measure `text` in `font` at `size`. The answer comes back under the token this returns.
/// The call's argument is the size as the little-endian bits of an `f64`, its text the font, a newline and the string
/// (`crates/gnoblin-schema/schema/tree/canvas.toml`).
pub(crate) fn measure(state: &mut HostState, node: NodeId, font: &str, size: f64, text: &str) -> Option<u64> {
    owned_canvas(state, node).ok()?;
    let HostState {
        mirror,
        tick,
        tokens,
        ui,
        ..
    } = state;
    *tokens += 1;
    let token = *tokens;
    tick.call(
        mirror,
        node,
        "measure",
        size.to_le_bytes().to_vec(),
        token,
        SecretText::new(format!("{font}\n{text}")),
    )
    .ok()?;
    ui.measures.insert(token);
    Some(token)
}

/// The first seat of the tree, the one the shell's keyboard is.
fn first_seat(state: &HostState) -> Option<NodeId> {
    crate::input::seat_nodes(state).first().copied()
}

/// Gives the seat's keyboard focus to `node`, or takes it away with `NIL`. A canvas of ours may hold it (a menu, an
/// entry), and so may a window: that is how a menu hands the keyboard back to the window it came from.
pub(crate) fn focus(state: &mut HostState, node: NodeId) -> Option<&'static str> {
    if node != NIL {
        match state.tick.node(&state.mirror, node) {
            None => return Some("unknown-node"),
            Some(record) if !state.ui.owned.contains(&node) && class_name(&record.class) != "Window" => {
                return Some("not-owned");
            }
            Some(_) => {}
        }
    }
    crate::input::focus_window(state, node)
}

/// The node with the keyboard focus as this frame's writes leave it, or `None`.
pub(crate) fn focused(state: &HostState) -> Option<NodeId> {
    let seat = first_seat(state)?;
    match state.tick.get(&state.mirror, seat, "focus") {
        Some(MirrorValue::Node(node)) if *node != NIL => Some(*node),
        _ => None,
    }
}

/// A number the seat publishes that the toolkit reads: the pointer's place.
pub(crate) fn seat_number(state: &HostState, key: &str) -> Option<f64> {
    if !SEAT_NUMBERS.contains(&key) {
        return None;
    }
    let seat = first_seat(state)?;
    match state.tick.get(&state.mirror, seat, key) {
        Some(MirrorValue::Double(value)) => Some(*value),
        _ => Some(0.0),
    }
}

/// A text the seat publishes that the toolkit reads: what each titlebar click does. The mirror holds only what
/// differs from the schema's default, so the default is what an absent value reads as.
pub(crate) fn seat_text(state: &HostState, key: &str) -> Option<String> {
    let (_, default) = SEAT_TEXTS.iter().find(|(name, _)| *name == key)?;
    let seat = first_seat(state)?;
    match state.tick.get(&state.mirror, seat, key) {
        Some(MirrorValue::Text(text)) => Some(text.clone()),
        _ => Some((*default).to_owned()),
    }
}

/// The screen the first seat's pointer is on (the seat's `screen`), as the number a script reads, or `None` while the
/// tree has no seat or the seat names no screen.
pub(crate) fn pointer_screen(state: &HostState) -> Option<f64> {
    let seat = first_seat(state)?;
    match state.tick.get(&state.mirror, seat, "screen") {
        Some(MirrorValue::Node(id)) if *id != NIL => Some(state.handles.to_script(*id)),
        _ => None,
    }
}

/// Whether the session is locked or about to be: nothing opens over the lock screen.
pub(crate) fn locked(state: &HostState) -> bool {
    parents_first(&state.mirror).into_iter().any(|id| {
        let Some(record) = state.tick.node(&state.mirror, id) else {
            return false;
        };
        if class_name(&record.class) != "Lock" {
            return false;
        }
        let engaged = matches!(
            state.tick.get(&state.mirror, id, "engaged"),
            Some(MirrorValue::Bool(true))
        );
        let held = match state.tick.get(&state.mirror, id, "state") {
            Some(MirrorValue::Text(text)) => text != "not-locked",
            _ => false,
        };
        engaged || held
    })
}

/// The size of the primary screen (the first when none is primary), or `None` while the tree has no sized screen.
pub(crate) fn screen(state: &HostState) -> Option<(f64, f64)> {
    let screens: Vec<NodeId> = parents_first(&state.mirror)
        .into_iter()
        .filter(|id| {
            state
                .tick
                .node(&state.mirror, *id)
                .is_some_and(|record| class_name(&record.class) == "Screen")
        })
        .collect();
    let primary = screens
        .iter()
        .copied()
        .find(|id| {
            matches!(
                state.tick.get(&state.mirror, *id, "primary"),
                Some(MirrorValue::Bool(true))
            )
        })
        .or_else(|| screens.first().copied())?;
    let size = |key: &str| match state.tick.get(&state.mirror, primary, key) {
        Some(MirrorValue::Double(value)) => *value,
        _ => 0.0,
    };
    let (width, height) = (size("width"), size("height"));
    (width > 0.0 && height > 0.0).then_some((width, height))
}

/// Puts `node` (a canvas of ours) last under the root, over every window. The compositor makes a window with no
/// parent, and the tree puts such a window after everything under the root, so a canvas given the root as its parent
/// would draw under it: the windows that have none are parented to the root first, in the order they already have.
pub(crate) fn mount_top(state: &mut HostState, node: NodeId) -> Option<&'static str> {
    if let Err(code) = owned_canvas(state, node) {
        return Some(code);
    }
    if let Some(code) = adopt_windows(state, NIL) {
        return Some(code);
    }
    let HostState { mirror, tick, .. } = state;
    let root = mirror.root();
    tick.insert(mirror, node, root, -1).err().map(|error| match error {
        TxError::UnknownNode(_) => "unknown-node",
        TxError::Cycle(_) => "cycle",
        TxError::Root(_) | TxError::Exists(_) => "refused",
    })
}

#[cfg(test)]
mod tests;
