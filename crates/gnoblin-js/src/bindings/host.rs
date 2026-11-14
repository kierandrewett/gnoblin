//! The functions of `globalThis.__gnoblinHost`.
//!
//! Each is a thin closure over a free function that works on [`HostState`] and plain Rust values, so the
//! logic is testable without an engine. A function that can fail returns `undefined` for success and a
//! short code for a refusal (`unknown-node`, `unknown-property`, `read-only`, `invalid-value`,
//! `unknown-method`), which `js/gnoblin.ts` turns into an error naming the member.

use gnoblin_tree_client::tree_mirror::{MirrorValue, NodeId, TreeMirror, lua_class};
use rquickjs::{Ctx, Function, Object, Value};

use super::props::{JsPropKind, default_value, defs_for, may_call};
use super::{HostState, JsHostValue, JsSpawnRequest, SharedState};

/// The largest id a script number holds exactly.
const MAX_EXACT_ID: f64 = 9_007_199_254_740_991.0;

/// The node a script number names, or `None` when it cannot be an id.
fn node_id(raw: f64) -> Option<NodeId> {
    (raw.is_finite() && (0.0..=MAX_EXACT_ID).contains(&raw) && raw.fract() == 0.0).then_some(raw as NodeId)
}

/// Every node of the mirror, parents before their children, in tree order.
pub(crate) fn parents_first(mirror: &TreeMirror) -> Vec<NodeId> {
    let mut order = Vec::new();
    let mut stack = vec![mirror.root()];
    while let Some(id) = stack.pop() {
        let Some(record) = mirror.node(id) else { continue };
        order.push(id);
        stack.extend(record.children.iter().rev().copied());
    }
    order
}

fn ids_of_class(state: &HostState, class: &str) -> Vec<f64> {
    parents_first(&state.mirror)
        .into_iter()
        .filter(|id| {
            state
                .mirror
                .node(*id)
                .is_some_and(|record| lua_class(&record.class) == class)
        })
        .map(|id| id as f64)
        .collect()
}

/// The lowercase class of a node a script can see, live or destroyed in this frame's batches.
fn kind_of(state: &HostState, node: NodeId) -> Option<String> {
    let record = state
        .tick
        .node(&state.mirror, node)
        .or_else(|| state.graveyard.get(&node))?;
    Some(record.class.to_ascii_lowercase())
}

/// A property as a script reads it. `published` reads the mirror alone: what the compositor last said, not
/// the writes this shell has queued on top. A node destroyed in the batches being planned answers from the
/// record it had.
fn read(state: &HostState, node: NodeId, key: &str, published: bool) -> Option<JsHostValue> {
    let live = if published {
        state.mirror.node(node)
    } else {
        state.tick.node(&state.mirror, node)
    };
    let (record, value) = match live {
        Some(record) if published => (record, record.props.get(key)),
        Some(record) => (record, state.tick.get(&state.mirror, node, key)),
        None => {
            let record = state.graveyard.get(&node)?;
            (record, record.props.get(key))
        }
    };
    let def = defs_for(&record.class)?.iter().find(|def| def.name == key)?;
    match value {
        Some(value) => JsHostValue::from_mirror(value),
        None => JsHostValue::from_mirror(&default_value(def.kind)),
    }
}

/// A script value, before it is checked against the property's kind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsScriptValue {
    Flag(bool),
    Number(f64),
    Text(String),
    Other,
}

fn convert(kind: JsPropKind, value: &JsScriptValue) -> Option<MirrorValue> {
    match (kind, value) {
        (JsPropKind::Double, JsScriptValue::Number(n)) if n.is_finite() => Some(MirrorValue::Double(*n)),
        (JsPropKind::Flag, JsScriptValue::Flag(flag)) => Some(MirrorValue::Bool(*flag)),
        (JsPropKind::Text, JsScriptValue::Text(text)) => Some(MirrorValue::Text(text.clone())),
        (JsPropKind::Node, JsScriptValue::Number(n)) => node_id(*n).map(MirrorValue::Node),
        _ => None,
    }
}

/// Queues a write of `node.key`. Returns the refusal code, or `None` when the write is queued.
pub(crate) fn write(state: &mut HostState, node: NodeId, key: &str, value: &JsScriptValue) -> Option<&'static str> {
    let HostState { mirror, tick, .. } = state;
    let Some(record) = tick.node(mirror, node) else {
        return Some("unknown-node");
    };
    let Some(def) = defs_for(&record.class).and_then(|defs| defs.iter().find(|def| def.name == key)) else {
        return Some("unknown-property");
    };
    if !def.writable {
        return Some("read-only");
    }
    let Some(value) = convert(def.kind, value) else {
        return Some("invalid-value");
    };
    tick.set(mirror, node, key, value, None, None)
        .err()
        .map(|_| "unknown-node")
}

/// Queues a call of `method` on `node`.
pub(crate) fn call(state: &mut HostState, node: NodeId, method: &str) -> Option<&'static str> {
    let HostState {
        mirror, tick, tokens, ..
    } = state;
    let Some(record) = tick.node(mirror, node) else {
        return Some("unknown-node");
    };
    if !may_call(&record.class, method) {
        return Some("unknown-method");
    }
    *tokens += 1;
    tick.call(mirror, node, method, Vec::new(), *tokens, Default::default())
        .err()
        .map(|_| "unknown-node")
}

/// Hands a spawn request to the host's spawner, and returns its refusal, if any.
pub(crate) fn spawn(state: &mut HostState, request: &JsSpawnRequest) -> Option<String> {
    let Some(spawner) = state.spawner.as_mut() else {
        return Some("no spawner is attached to this shell".to_owned());
    };
    spawner(request).err().map(|error| error.0)
}

/// The request a script's `spawn` argument names: a string, or an array of strings.
fn request_of<'js>(value: &Value<'js>) -> Option<JsSpawnRequest> {
    if let Some(text) = value.as_string() {
        return text.to_string().ok().map(JsSpawnRequest::Command);
    }
    let array = value.as_array()?;
    let argv: Option<Vec<String>> = array.iter::<String>().map(Result::ok).collect();
    argv.map(JsSpawnRequest::Argv)
}

fn script_value<'js>(value: &Value<'js>) -> JsScriptValue {
    if let Some(flag) = value.as_bool() {
        JsScriptValue::Flag(flag)
    } else if let Some(n) = value.as_number() {
        JsScriptValue::Number(n)
    } else if let Some(text) = value.as_string().and_then(|text| text.to_string().ok()) {
        JsScriptValue::Text(text)
    } else {
        JsScriptValue::Other
    }
}

/// Installs `globalThis.__gnoblinHost`, which the embedded `gnoblin` module reads once when it loads.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;

    let shared = state.clone();
    host.set(
        "kind",
        Function::new(ctx.clone(), move |node: f64| -> Option<String> {
            kind_of(&shared.borrow(), node_id(node)?)
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "get",
        Function::new(ctx.clone(), move |node: f64, key: String| -> Option<JsHostValue> {
            read(&shared.borrow(), node_id(node)?, &key, false)
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "published",
        Function::new(ctx.clone(), move |node: f64, key: String| -> Option<JsHostValue> {
            read(&shared.borrow(), node_id(node)?, &key, true)
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "set",
        Function::new(
            ctx.clone(),
            move |node: f64, key: String, value: Value<'js>| -> Option<&'static str> {
                let Some(node) = node_id(node) else {
                    return Some("unknown-node");
                };
                write(&mut shared.borrow_mut(), node, &key, &script_value(&value))
            },
        )?,
    )?;

    let shared = state.clone();
    host.set(
        "call",
        Function::new(ctx.clone(), move |node: f64, method: String| -> Option<&'static str> {
            let Some(node) = node_id(node) else {
                return Some("unknown-node");
            };
            call(&mut shared.borrow_mut(), node, &method)
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "windows",
        Function::new(ctx.clone(), move || -> Vec<f64> {
            ids_of_class(&shared.borrow(), "Window")
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "screens",
        Function::new(ctx.clone(), move || -> Vec<f64> {
            ids_of_class(&shared.borrow(), "Screen")
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "spawn",
        Function::new(ctx.clone(), move |command: Value<'js>| -> Option<String> {
            let Some(request) = request_of(&command) else {
                return Some("the command must be a string or an array of strings".to_owned());
            };
            spawn(&mut shared.borrow_mut(), &request)
        })?,
    )?;

    crate::input::install(ctx, &host, state)?;

    ctx.globals().set("__gnoblinHost", host)
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{MirrorValue, SnapshotNode, TreeMirror};

    use super::{JsScriptValue, call, ids_of_class, node_id, parents_first, read, write};
    use crate::bindings::{HostState, JsHostValue};

    fn node(id: u64, parent: u64, class: &str, props: Vec<(String, MirrorValue)>) -> SnapshotNode {
        SnapshotNode {
            id,
            parent,
            index: 0,
            class: class.into(),
            title: String::new(),
            app_id: String::new(),
            props,
        }
    }

    fn state() -> HostState {
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(3, 2, "window", vec![("x".into(), MirrorValue::Double(5.0))]),
            node(2, 1, "workspace", Vec::new()),
            node(4, 1, "seat", Vec::new()),
        ];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    #[test]
    fn ids_are_whole_non_negative_numbers() {
        assert_eq!(node_id(7.0), Some(7));
        assert_eq!(node_id(-1.0), None);
        assert_eq!(node_id(1.5), None);
        assert_eq!(node_id(f64::NAN), None);
        assert_eq!(node_id(1e300), None);
    }

    #[test]
    fn parents_come_before_children_whatever_the_snapshot_order() {
        assert_eq!(parents_first(&state().mirror), [1, 2, 3, 4]);
    }

    #[test]
    fn an_absent_property_reads_as_its_default_and_a_seat_exposes_none() {
        let state = state();
        assert_eq!(read(&state, 3, "x", false), Some(JsHostValue::Number(5.0)));
        assert_eq!(read(&state, 3, "title", false), Some(JsHostValue::Text(String::new())));
        assert_eq!(read(&state, 3, "focused", true), Some(JsHostValue::Flag(false)));
        assert_eq!(read(&state, 4, "x", false), None);
        assert_eq!(read(&state, 99, "x", false), None);
    }

    #[test]
    fn a_write_is_read_back_unpublished_and_the_published_value_stays() {
        let mut state = state();
        assert_eq!(write(&mut state, 3, "x", &JsScriptValue::Number(10.0)), None);
        assert_eq!(read(&state, 3, "x", false), Some(JsHostValue::Number(10.0)));
        assert_eq!(read(&state, 3, "x", true), Some(JsHostValue::Number(5.0)));
        assert_eq!(state.tick.ops().len(), 1);
    }

    #[test]
    fn a_write_that_is_not_allowed_is_refused_with_a_code_and_queues_nothing() {
        let mut state = state();
        assert_eq!(
            write(&mut state, 3, "x", &JsScriptValue::Number(f64::NAN)),
            Some("invalid-value")
        );
        assert_eq!(
            write(&mut state, 3, "x", &JsScriptValue::Text("1".into())),
            Some("invalid-value")
        );
        assert_eq!(
            write(&mut state, 3, "title", &JsScriptValue::Text("t".into())),
            Some("read-only")
        );
        assert_eq!(
            write(&mut state, 3, "nope", &JsScriptValue::Number(1.0)),
            Some("unknown-property")
        );
        assert_eq!(
            write(&mut state, 99, "x", &JsScriptValue::Number(1.0)),
            Some("unknown-node")
        );
        assert!(state.tick.ops().is_empty());
    }

    #[test]
    fn a_screen_exposes_its_size_and_primary_flag_read_only() {
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(
                5,
                1,
                "screen",
                vec![
                    ("width".into(), MirrorValue::Double(1920.0)),
                    ("primary".into(), MirrorValue::Bool(true)),
                ],
            ),
        ];
        let mut state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        assert_eq!(ids_of_class(&state, "Screen"), [5.0]);
        assert_eq!(read(&state, 5, "width", false), Some(JsHostValue::Number(1920.0)));
        assert_eq!(read(&state, 5, "height", false), Some(JsHostValue::Number(0.0)));
        assert_eq!(read(&state, 5, "primary", false), Some(JsHostValue::Flag(true)));
        assert_eq!(
            write(&mut state, 5, "width", &JsScriptValue::Number(1.0)),
            Some("read-only")
        );
    }

    #[test]
    fn only_close_and_focus_can_be_called_on_a_window() {
        let mut state = state();
        assert_eq!(call(&mut state, 3, "close"), None);
        assert_eq!(call(&mut state, 3, "kill"), Some("unknown-method"));
        assert_eq!(call(&mut state, 99, "close"), Some("unknown-node"));
        assert_eq!(state.tick.ops().len(), 1);
        assert_eq!(state.tokens, 1);
    }
}
