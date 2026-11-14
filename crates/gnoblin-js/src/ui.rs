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

use std::collections::HashSet;

use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NIL, NodeId, TreeEvent, TxError, lua_class};
use rquickjs::{Ctx, Function, IntoJs, Object, Value};

use crate::bindings::{HostState, NodeHandles, SharedState, adopt_windows, parents_first};

/// A shell makes the ids of its nodes from the epoch it attached under, so two shells never make the same id: the
/// epoch in the high bits, a counter in the low ones. The same split `gnoblin-lua`'s shell uses.
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
        Some(record) if lua_class(&record.class) == CANVAS => Ok(()),
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
            Some(record) if !state.ui.owned.contains(&node) && lua_class(&record.class) != "Window" => {
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

/// Whether the session is locked or about to be: nothing opens over the lock screen.
pub(crate) fn locked(state: &HostState) -> bool {
    parents_first(&state.mirror).into_iter().any(|id| {
        let Some(record) = state.tick.node(&state.mirror, id) else {
            return false;
        };
        if lua_class(&record.class) != "Lock" {
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
                .is_some_and(|record| lua_class(&record.class) == "Screen")
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

/// Installs `__gnoblinHost.ui`, the functions `gnoblin/ui` runs on.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let ui = Object::new(ctx.clone())?;

    let shared = state.clone();
    ui.set(
        "create",
        Function::new(ctx.clone(), move || -> Option<f64> {
            let mut state = shared.borrow_mut();
            let id = create(&mut state)?;
            Some(state.handles.to_script(id))
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "insert",
        Function::new(
            ctx.clone(),
            move |node: f64, parent: f64, index: f64| -> Option<&'static str> {
                let mut state = shared.borrow_mut();
                let (Some(node), Some(parent)) = (state.handles.resolve(node), state.handles.resolve(parent)) else {
                    return Some("unknown-node");
                };
                insert(&mut state, node, parent, index as i64)
            },
        )?,
    )?;

    let shared = state.clone();
    ui.set(
        "destroy",
        Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(node) {
                Some(node) => destroy(&mut state, node),
                None => Some("unknown-node"),
            }
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "set",
        Function::new(
            ctx.clone(),
            move |node: f64, key: String, value: Value<'js>| -> Option<&'static str> {
                let mut state = shared.borrow_mut();
                let Some(node) = state.handles.resolve(node) else {
                    return Some("unknown-node");
                };
                if let Some(flag) = value.as_bool() {
                    set_flag(&mut state, node, &key, flag)
                } else if let Some(number) = value.as_number() {
                    set_number(&mut state, node, &key, number)
                } else {
                    Some("invalid-value")
                }
            },
        )?,
    )?;

    let shared = state.clone();
    ui.set(
        "paint",
        Function::new(ctx.clone(), move |node: f64, bytes: Vec<u8>| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(node) {
                Some(node) => paint(&mut state, node, bytes),
                None => Some("unknown-node"),
            }
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "measure",
        Function::new(
            ctx.clone(),
            move |node: f64, font: String, size: f64, text: String| -> Option<f64> {
                let mut state = shared.borrow_mut();
                let node = state.handles.resolve(node)?;
                measure(&mut state, node, &font, size, &text).map(|token| token as f64)
            },
        )?,
    )?;

    let shared = state.clone();
    ui.set(
        "takeAnswers",
        Function::new(ctx.clone(), move || -> Vec<JsUiAnswer> {
            std::mem::take(&mut shared.borrow_mut().ui.answers)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "takeEvents",
        Function::new(ctx.clone(), move || -> Vec<JsUiEvent> {
            std::mem::take(&mut shared.borrow_mut().ui.events)
        })?,
    )?;

    // `focus(0)` takes the keyboard focus away.
    let shared = state.clone();
    ui.set(
        "focus",
        Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            let Some(node) = state.handles.resolve(node) else {
                return Some("unknown-node");
            };
            focus(&mut state, node)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "focused",
        Function::new(ctx.clone(), move || -> Option<f64> {
            let state = shared.borrow();
            focused(&state).map(|node| state.handles.to_script(node))
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "top",
        Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(node) {
                Some(node) => mount_top(&mut state, node),
                None => Some("unknown-node"),
            }
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "seatNumber",
        Function::new(ctx.clone(), move |key: String| -> Option<f64> {
            seat_number(&shared.borrow(), &key)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "seatText",
        Function::new(ctx.clone(), move |key: String| -> Option<String> {
            seat_text(&shared.borrow(), &key)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "locked",
        Function::new(ctx.clone(), move || -> bool { locked(&shared.borrow()) })?,
    )?;

    let shared = state.clone();
    ui.set(
        "screen",
        Function::new(ctx.clone(), move || -> Option<Vec<f64>> {
            screen(&shared.borrow()).map(|(width, height)| vec![width, height])
        })?,
    )?;

    host.set("ui", ui)
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{SnapshotNode, TreeMirror, TxOp};

    use super::*;

    fn snapshot(id: NodeId, parent: NodeId, class: &str) -> SnapshotNode {
        SnapshotNode {
            id,
            parent,
            index: 0,
            class: class.to_owned(),
            title: String::new(),
            app_id: String::new(),
            props: Vec::new(),
        }
    }

    /// A root (1) with one window (2) under it, attached under epoch 1.
    fn state() -> HostState {
        let nodes = [snapshot(1, 0, "node"), snapshot(2, 1, "window")];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    #[test]
    fn an_id_is_the_epoch_and_a_count() {
        let mut state = state();
        assert_eq!(create(&mut state), Some((1 << 32) | 1));
        assert_eq!(create(&mut state), Some((1 << 32) | 2));
    }

    #[test]
    fn only_canvases_the_toolkit_made_are_writable() {
        let mut state = state();
        let canvas = create(&mut state).expect("a canvas");
        assert_eq!(set_number(&mut state, canvas, "width", 10.0), None);
        assert_eq!(set_flag(&mut state, canvas, "visible", false), None);
        assert_eq!(paint(&mut state, canvas, vec![1, 2, 3]), None);
        assert_eq!(set_number(&mut state, 2, "width", 10.0), Some("not-owned"));
        assert_eq!(destroy(&mut state, 2), Some("not-owned"));
        assert_eq!(set_number(&mut state, canvas, "content", 1.0), Some("unknown-property"));
        assert_eq!(set_number(&mut state, canvas, "x", f64::NAN), Some("invalid-value"));
        assert_eq!(set_flag(&mut state, canvas, "width", true), Some("unknown-property"));
    }

    #[test]
    fn a_canvas_goes_under_any_node_and_destroying_it_twice_is_not_an_error() {
        let mut state = state();
        let canvas = create(&mut state).expect("a canvas");
        assert_eq!(insert(&mut state, canvas, 2, -1), None);
        assert_eq!(destroy(&mut state, canvas), None);
        assert_eq!(destroy(&mut state, canvas), None, "a node already gone is fine");
        assert_eq!(set_number(&mut state, canvas, "x", 1.0), Some("unknown-node"));
    }

    #[test]
    fn a_measurement_carries_the_size_and_the_font_and_its_answer_is_kept_once() {
        let mut state = state();
        let canvas = create(&mut state).expect("a canvas");
        let token = measure(&mut state, canvas, "sans", 10.0, "Hello").expect("a token");
        let calls: Vec<(String, Vec<u8>, String)> = state
            .tick
            .ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::Call { method, args, text, .. } => Some((method.clone(), args.clone(), text.expose().to_owned())),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            [(
                "measure".to_owned(),
                10.0_f64.to_le_bytes().to_vec(),
                "sans\nHello".to_owned()
            )]
        );
        assert!(
            !state.ui.accept_answer(token + 1, b"1 1 1", ""),
            "another call's answer"
        );
        assert!(state.ui.accept_answer(token, b"31 15 12", ""));
        assert!(
            !state.ui.accept_answer(token, b"31 15 12", ""),
            "an answer is kept once"
        );
        assert_eq!(state.ui.answers.len(), 1);
    }

    fn node_with(id: NodeId, parent: NodeId, class: &str, props: Vec<(String, MirrorValue)>) -> SnapshotNode {
        SnapshotNode {
            props,
            ..snapshot(id, parent, class)
        }
    }

    /// A root with a seat, a lock, a primary screen of 800 by 600, and two windows, one of them without a parent as the
    /// compositor makes them.
    fn session() -> HostState {
        let nodes = [
            snapshot(1, 0, "node"),
            node_with(
                2,
                1,
                "seat",
                vec![
                    ("pointer_x".into(), MirrorValue::Double(120.0)),
                    ("right_click".into(), MirrorValue::Text("none".into())),
                ],
            ),
            node_with(3, 1, "lock", Vec::new()),
            node_with(
                4,
                1,
                "screen",
                vec![
                    ("width".into(), MirrorValue::Double(800.0)),
                    ("height".into(), MirrorValue::Double(600.0)),
                    ("primary".into(), MirrorValue::Bool(true)),
                ],
            ),
            snapshot(5, 0, "window"),
            snapshot(6, 0, "window"),
        ];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    #[test]
    fn a_key_and_a_request_are_the_toolkits_and_a_key_for_a_node_that_is_not_ours_is_not() {
        let mut state = session();
        let canvas = create(&mut state).expect("a canvas");
        let event = |node: NodeId, kind: &str, key: &str| TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: kind.to_owned(),
            fields: gnoblin_tree_client::tree_mirror::EventFields {
                key: key.to_owned(),
                text: key.to_owned(),
                pressed: true,
                request: "menu".to_owned(),
                ..Default::default()
            },
        };
        let handles = NodeHandles::default();
        assert!(state.ui.accept_event(&event(canvas, "key", "a"), &handles));
        assert!(
            !state.ui.accept_event(&event(5, "key", "a"), &handles),
            "a window's key is not ours"
        );
        assert!(
            state.ui.accept_event(&event(5, "request", ""), &handles),
            "a request is, whatever the node"
        );
        assert_eq!(state.ui.events.len(), 2);
        assert_eq!(state.ui.events[0].key, "a");
        assert_eq!(state.ui.events[1].request, "menu");
        assert!(
            !format!("{:?}", state.ui.events[0]).contains("\"a\""),
            "a key typed is never printed"
        );
    }

    #[test]
    fn a_canvas_or_a_window_may_hold_the_keyboard_and_nothing_else_may() {
        let mut state = session();
        let canvas = create(&mut state).expect("a canvas");
        assert_eq!(focus(&mut state, canvas), None);
        assert_eq!(focused(&state), Some(canvas));
        assert_eq!(focus(&mut state, 5), None, "a window gets the keyboard back");
        assert_eq!(focused(&state), Some(5));
        assert_eq!(focus(&mut state, NIL), None);
        assert_eq!(focused(&state), None, "nil takes it away");
        assert_eq!(
            focus(&mut state, 4),
            Some("not-owned"),
            "a screen is not for the keyboard"
        );
        assert_eq!(focus(&mut state, 99), Some("unknown-node"));
        let mut bare = self::state();
        assert_eq!(focus(&mut bare, 2), Some("no-seat"));
    }

    #[test]
    fn the_seat_gives_the_pointer_and_what_a_titlebar_click_does_and_nothing_else() {
        let state = session();
        assert_eq!(seat_number(&state, "pointer_x"), Some(120.0));
        assert_eq!(seat_number(&state, "pointer_y"), Some(0.0));
        assert_eq!(seat_number(&state, "focus_mode"), None);
        assert_eq!(seat_text(&state, "right_click").as_deref(), Some("none"));
        assert_eq!(
            seat_text(&state, "middle_click").as_deref(),
            Some("lower"),
            "the schema's default"
        );
        assert_eq!(seat_text(&state, "double_click").as_deref(), Some("toggle-maximize"));
        assert_eq!(seat_text(&state, "name"), None);
    }

    #[test]
    fn the_session_is_locked_while_the_lock_is_engaged_or_not_not_locked() {
        let mut state = session();
        assert!(!locked(&state));
        state
            .tick
            .set(&state.mirror, 3, "engaged", MirrorValue::Bool(true), None, None)
            .expect("a write");
        assert!(locked(&state));
        state
            .tick
            .set(&state.mirror, 3, "engaged", MirrorValue::Bool(false), None, None)
            .expect("a write");
        state
            .tick
            .set(
                &state.mirror,
                3,
                "state",
                MirrorValue::Text("pending".into()),
                None,
                None,
            )
            .expect("a write");
        assert!(locked(&state));
    }

    #[test]
    fn the_screen_is_the_primary_ones_size() {
        assert_eq!(screen(&session()), Some((800.0, 600.0)));
        assert_eq!(screen(&state()), None);
    }

    #[test]
    fn a_canvas_mounted_on_top_goes_after_windows_the_compositor_left_without_a_parent() {
        let mut state = session();
        let canvas = create(&mut state).expect("a canvas");
        assert_eq!(mount_top(&mut state, canvas), None);
        let parents: Vec<(NodeId, NodeId)> = state
            .tick
            .ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::Insert { node, parent, .. } => Some((*node, *parent)),
                _ => None,
            })
            .collect();
        assert_eq!(
            parents,
            [(5, 1), (6, 1), (canvas, 1)],
            "the windows first, in their order, then the canvas"
        );
        assert_eq!(mount_top(&mut state, 5), Some("not-owned"));
    }
}
