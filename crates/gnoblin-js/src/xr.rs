//! `__gnoblinHost.xr`: the scene nodes the XR module (`gnoblin/scene`, `gnoblin/xr`) makes and finds.
//!
//! The XR program reads `View` nodes under a node named `room` (`docs/xr.md`), so the shell has to make plain nodes,
//! cameras and Views, put them in the tree and take them out. Like the UI toolkit's canvases, a node made here is
//! owned by this shell: only it may be inserted or destroyed through these functions, and ids come from the toolkit's
//! count so the two never clash. Properties are read and written through the generic host functions (`get`, `set`),
//! which know these classes (`bindings/props.rs`).

use std::collections::HashSet;

use gnoblin_tree_client::tree_mirror::{NIL, NodeId, TxError, lua_class};
use rquickjs::{Ctx, Function, Object};

use crate::bindings::{HostState, JsHostValue, SharedState, parents_first};

/// The classes a script may make: the room, a camera and a View.
const CLASSES: [&str; 3] = ["Node", "Camera", "View"];

/// What the XR module keeps on the host.
#[derive(Default)]
pub(crate) struct JsXrState {
    /// The nodes this shell made and has not destroyed.
    owned: HashSet<NodeId>,
    /// Whether the session declared XR (`JsShellConfig::xr_declared`): the `gnoblin/extensions/xr` module imports.
    declared: bool,
}

impl JsXrState {
    pub(crate) fn declared(declared: bool) -> Self {
        Self {
            owned: HashSet::new(),
            declared,
        }
    }

    /// Forgets nodes the compositor destroyed: a window's destruction takes nothing of ours, but a tree reset might.
    pub(crate) fn forget(&mut self, nodes: &[NodeId]) {
        for node in nodes {
            self.owned.remove(node);
        }
    }
}

/// Makes a node of `class` in this tick. `Err` is a refusal code: `unknown-class` or `refused` (no id left).
pub(crate) fn create(state: &mut HostState, class: &str) -> Result<NodeId, &'static str> {
    let Some(class) = CLASSES.iter().find(|known| known.eq_ignore_ascii_case(class)) else {
        return Err("unknown-class");
    };
    let HostState {
        mirror, tick, ui, xr, ..
    } = state;
    let id = ui.mint(mirror.epoch()).ok_or("refused")?;
    tick.new_node(mirror, id, class).map_err(|_| "refused")?;
    xr.owned.insert(id);
    Ok(id)
}

/// Whether `node` is one of ours that this tick still sees.
fn owned(state: &HostState, node: NodeId) -> Result<(), &'static str> {
    if state.tick.node(&state.mirror, node).is_none() {
        return Err("unknown-node");
    }
    if state.xr.owned.contains(&node) {
        Ok(())
    } else {
        Err("not-owned")
    }
}

/// Puts `node` (one of ours) under `parent` at `index`; a negative index is last. A node already in the tree moves.
pub(crate) fn insert(state: &mut HostState, node: NodeId, parent: NodeId, index: i64) -> Option<&'static str> {
    if let Err(code) = owned(state, node) {
        return Some(code);
    }
    let HostState { mirror, tick, .. } = state;
    tick.insert(mirror, node, parent, index).err().map(|error| match error {
        TxError::UnknownNode(_) => "unknown-node",
        TxError::Cycle(_) => "cycle",
        TxError::Root(_) | TxError::Exists(_) => "refused",
    })
}

/// Destroys a node of ours and its subtree. One that is already gone is not an error: the compositor may have taken
/// it with something above it.
pub(crate) fn destroy(state: &mut HostState, node: NodeId) -> Option<&'static str> {
    match owned(state, node) {
        Err("unknown-node") => {
            state.xr.owned.remove(&node);
            return None;
        }
        Err(code) => return Some(code),
        Ok(()) => {}
    }
    let HostState { mirror, tick, xr, .. } = state;
    match tick.destroy(mirror, node) {
        Ok(destroyed) => {
            xr.owned.remove(&node);
            xr.forget(&destroyed.removed);
            None
        }
        Err(TxError::UnknownNode(_)) => {
            xr.owned.remove(&node);
            None
        }
        Err(_) => Some("refused"),
    }
}

/// The nodes of `class` the mirror holds, parents before children, as the numbers a script holds.
fn nodes(state: &HostState, class: &str) -> Vec<f64> {
    parents_first(&state.mirror)
        .into_iter()
        .filter(|id| {
            state
                .mirror
                .node(*id)
                .is_some_and(|record| lua_class(&record.class).eq_ignore_ascii_case(class))
        })
        .map(|id| state.handles.to_script(id))
        .collect()
}

/// The children of `node` in order, as this tick sees them. Empty for a node that is not there.
fn children(state: &HostState, node: NodeId) -> Vec<f64> {
    state
        .tick
        .children(&state.mirror, node)
        .map(|list| list.iter().map(|id| state.handles.to_script(*id)).collect())
        .unwrap_or_default()
}

/// The parent of `node`, or `None` for a node with none (the root, a window the compositor made unparented).
fn parent(state: &HostState, node: NodeId) -> Option<f64> {
    let record = state.tick.node(&state.mirror, node)?;
    (record.parent != NIL).then(|| state.handles.to_script(record.parent))
}

/// Installs `__gnoblinHost.xr`.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let xr = Object::new(ctx.clone())?;

    // `create(class)`: the node's number, or a refusal code as text.
    let shared = state.clone();
    xr.set(
        "create",
        Function::new(ctx.clone(), move |class: String| -> JsHostValue {
            let mut state = shared.borrow_mut();
            match create(&mut state, &class) {
                Ok(id) => JsHostValue::Number(state.handles.to_script(id)),
                Err(code) => JsHostValue::Text(code.to_owned()),
            }
        })?,
    )?;

    let shared = state.clone();
    xr.set(
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
    xr.set(
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
    xr.set(
        "nodes",
        Function::new(ctx.clone(), move |class: String| -> Vec<f64> {
            nodes(&shared.borrow(), &class)
        })?,
    )?;

    let shared = state.clone();
    xr.set(
        "children",
        Function::new(ctx.clone(), move |node: f64| -> Vec<f64> {
            let state = shared.borrow();
            state
                .handles
                .resolve(node)
                .map_or_else(Vec::new, |node| children(&state, node))
        })?,
    )?;

    let shared = state.clone();
    xr.set(
        "parent",
        Function::new(ctx.clone(), move |node: f64| -> Option<f64> {
            let state = shared.borrow();
            parent(&state, state.handles.resolve(node)?)
        })?,
    )?;

    let shared = state.clone();
    xr.set(
        "root",
        Function::new(ctx.clone(), move || -> f64 {
            let state = shared.borrow();
            state.handles.to_script(state.mirror.root())
        })?,
    )?;

    let shared = state.clone();
    xr.set(
        "declared",
        Function::new(ctx.clone(), move || -> bool { shared.borrow().xr.declared })?,
    )?;

    host.set("xr", xr)
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

    fn state() -> HostState {
        let nodes = [snapshot(1, 0, "node"), snapshot(2, 1, "window")];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    #[test]
    fn only_a_node_a_camera_and_a_view_can_be_made_and_their_ids_never_repeat() {
        let mut state = state();
        assert_eq!(create(&mut state, "canvas"), Err("unknown-class"));
        assert_eq!(create(&mut state, "window"), Err("unknown-class"));
        let room = create(&mut state, "Node").expect("a node");
        let view = create(&mut state, "view").expect("a view");
        let canvas = crate::ui::create(&mut state).expect("a canvas");
        assert!(room != view && view != canvas && room != canvas);
        let classes: Vec<String> = state
            .tick
            .ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::New { class, .. } => Some(class.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(classes, ["node", "view", "canvas"], "the wire carries the schema names");
    }

    #[test]
    fn a_node_that_is_not_ours_is_neither_inserted_nor_destroyed() {
        let mut state = state();
        assert_eq!(insert(&mut state, 2, 1, -1), Some("not-owned"));
        assert_eq!(destroy(&mut state, 2), Some("not-owned"));
        assert_eq!(insert(&mut state, 99, 1, -1), Some("unknown-node"));
    }

    #[test]
    fn a_view_goes_under_the_room_moves_to_the_end_and_destroying_twice_is_not_an_error() {
        let mut state = state();
        let room = create(&mut state, "Node").expect("room");
        let a = create(&mut state, "View").expect("a");
        let b = create(&mut state, "View").expect("b");
        assert_eq!(insert(&mut state, room, 1, -1), None);
        assert_eq!(insert(&mut state, a, room, -1), None);
        assert_eq!(insert(&mut state, b, room, -1), None);
        assert_eq!(
            insert(&mut state, a, room, -1),
            None,
            "a node already in the tree moves"
        );
        let listed = children(&state, room);
        assert_eq!(
            listed,
            [state.handles.to_script(b), state.handles.to_script(a)],
            "a moved to the end"
        );
        assert_eq!(insert(&mut state, room, a, -1), Some("cycle"));
        assert_eq!(destroy(&mut state, room), None);
        assert_eq!(destroy(&mut state, room), None);
        assert_eq!(parent(&state, a), None, "the subtree went with the room");
    }

    #[test]
    fn nodes_lists_a_class_from_the_mirror_and_a_parent_is_none_for_a_root() {
        let state = state();
        assert_eq!(nodes(&state, "window"), [2.0]);
        assert_eq!(nodes(&state, "View"), Vec::<f64>::new());
        assert_eq!(parent(&state, 2), Some(1.0));
        assert_eq!(parent(&state, 1), None);
    }
}
