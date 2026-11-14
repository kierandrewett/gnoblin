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
use gnoblin_tree_client::tree_mirror::{MirrorValue, NodeId, TreeEvent, TxError, lua_class};
use rquickjs::{Ctx, Function, IntoJs, Object, Value};

use crate::bindings::{HostState, NodeHandles, SharedState};

/// A shell makes the ids of its nodes from the epoch it attached under, so two shells never make the same id: the
/// epoch in the high bits, a counter in the low ones. The same split `gnoblin-lua`'s shell uses.
const EPOCH_SHIFT: u32 = 32;
const EPOCH_LIMIT: u64 = 1 << 31;

/// The class of the nodes the toolkit makes.
const CANVAS: &str = "Canvas";

/// The properties of a canvas a script may write as a number, and as a flag.
const NUMBER_KEYS: &[&str] = &["x", "y", "width", "height", "opacity"];
const FLAG_KEYS: &[&str] = &["visible", "input", "clip"];

/// The event kinds that reach a canvas. A key event carries what the user typed, so it is not one of them.
const POINTER_KINDS: &[&str] = &["press", "release", "motion", "scroll", "enter", "leave"];

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

/// A pointer event that hit a canvas, as the script reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JsUiEvent {
    node: f64,
    kind: String,
    x: f64,
    y: f64,
    button: f64,
    dx: f64,
    dy: f64,
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

    /// Keeps a pointer event addressed to a canvas of ours; any other event is not the toolkit's.
    pub(crate) fn accept_event(&mut self, event: &TreeEvent, handles: &NodeHandles) -> bool {
        if !self.owned.contains(&event.node) || !POINTER_KINDS.contains(&event.kind.as_str()) {
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
        });
        true
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
    let epoch = mirror.epoch();
    if epoch >= EPOCH_LIMIT || ui.counter + 1 >= 1 << EPOCH_SHIFT {
        return None;
    }
    ui.counter += 1;
    let id = (epoch << EPOCH_SHIFT) | ui.counter;
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
}
