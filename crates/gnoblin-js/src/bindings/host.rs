//! The functions of `globalThis.__gnoblinHost`.
//!
//! Each is a thin closure over a free function that works on [`HostState`] and plain Rust values, so the
//! logic is testable without an engine. A function that can fail returns `undefined` for success and a
//! short code for a refusal (`unknown-node`, `unknown-property`, `read-only`, `invalid-value`,
//! `unknown-method`), which `js/gnoblin.ts` turns into an error naming the member.

use gnoblin_proto::tree::effect;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NIL, NodeId, TreeMirror, TxAnimation, lua_class};
use rquickjs::function::Opt;
use rquickjs::{Ctx, Function, Object, Value};

use super::props::{JsPropDef, JsPropKind, default_value, defs_for, may_call};
use super::{HostState, JsHostValue, JsSpawnRequest, NodeHandles, SharedState, effect_from_js};

/// Every node of the mirror, parents before their children. The root's tree comes first, in tree order; then
/// each tree that has no parent (the compositor makes a window unparented, and a shell parents it), by id.
pub(crate) fn parents_first(mirror: &TreeMirror) -> Vec<NodeId> {
    let mut roots: Vec<NodeId> = mirror
        .ids()
        .filter(|id| mirror.node(*id).is_some_and(|record| record.parent == NIL))
        .collect();
    roots.sort_by_key(|id| (*id != mirror.root(), *id));
    let mut order = Vec::new();
    let mut stack: Vec<NodeId> = roots.into_iter().rev().collect();
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
        .map(|id| state.handles.to_script(id))
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
        Some(value) => JsHostValue::from_mirror(value, &state.handles),
        None => JsHostValue::from_mirror(&default_value(def), &state.handles),
    }
}

/// A script value, before it is checked against the property's kind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsScriptValue {
    Flag(bool),
    Number(f64),
    Text(String),
    Numbers(Vec<f64>),
    /// An array of strings. An empty array is `Numbers`, and a `list<string>` write takes it as the empty list.
    Texts(Vec<String>),
    /// An array of effect objects. An empty array is `Numbers`, and an `effects` write takes it as the empty list.
    Effects(Vec<effect::Properties>),
    Other,
}

/// How many components a list property (`rotation`, `scale`) has.
const VECTOR_LEN: usize = 3;

fn convert(def: &JsPropDef, value: &JsScriptValue, handles: &NodeHandles) -> Option<MirrorValue> {
    match (def.kind, value) {
        (JsPropKind::Double, JsScriptValue::Number(n)) if n.is_finite() => Some(MirrorValue::Double(*n)),
        (JsPropKind::Flag, JsScriptValue::Flag(flag)) => Some(MirrorValue::Bool(*flag)),
        (JsPropKind::Text, JsScriptValue::Text(text)) => Some(MirrorValue::Text(text.clone())),
        (JsPropKind::Node, JsScriptValue::Number(n)) => handles.resolve(*n).map(MirrorValue::Node),
        (JsPropKind::Doubles, JsScriptValue::Numbers(list))
            if list.len() == VECTOR_LEN && list.iter().all(|n| n.is_finite()) =>
        {
            Some(MirrorValue::List(list.clone()))
        }
        (JsPropKind::Texts, JsScriptValue::Texts(list))
            if def.choices.is_empty() || list.iter().all(|item| def.choices.contains(&item.as_str())) =>
        {
            Some(MirrorValue::Texts(list.clone()))
        }
        (JsPropKind::Texts, JsScriptValue::Numbers(list)) if list.is_empty() => Some(MirrorValue::Texts(Vec::new())),
        (JsPropKind::Effects, JsScriptValue::Effects(list)) => Some(MirrorValue::Effects(list.clone())),
        (JsPropKind::Effects, JsScriptValue::Numbers(list)) if list.is_empty() => {
            Some(MirrorValue::Effects(Vec::new()))
        }
        _ => None,
    }
}

/// The curve of an animated write: the seconds it takes, and samples of progress from 0 to 1 it follows.
pub(crate) type JsCurve<'a> = (f64, &'a [f64]);

/// A curve the tree accepts: a finite, non-negative duration and at least two samples in `[0, 1]`.
fn curve_is_valid((seconds, samples): JsCurve<'_>) -> bool {
    seconds.is_finite()
        && seconds >= 0.0
        && samples.len() >= 2
        && samples.iter().all(|sample| (0.0..=1.0).contains(sample))
}

/// What a write of `node.key` would be, checked as a queued write is and not yet queued: the value in the tree's own
/// type, or the refusal code. `animation` is the curve a timed write carries.
pub(crate) fn prepare(
    state: &HostState,
    node: NodeId,
    key: &str,
    value: &JsScriptValue,
    animation: Option<JsCurve<'_>>,
) -> Result<MirrorValue, &'static str> {
    let HostState {
        mirror, tick, handles, ..
    } = state;
    let Some(record) = tick.node(mirror, node) else {
        return Err("unknown-node");
    };
    let Some(def) = defs_for(&record.class).and_then(|defs| defs.iter().find(|def| def.name == key)) else {
        return Err("unknown-property");
    };
    if !def.writable {
        return Err("read-only");
    }
    let Some(value) = convert(def, value, handles) else {
        return Err("invalid-value");
    };
    if let Some(curve) = animation {
        if !curve_is_valid(curve) {
            return Err("invalid-value");
        }
        if !def.animated {
            return Err("not-animated");
        }
    }
    Ok(value)
}

/// Queues a write of `node.key`, animated when `animation` is given. Returns the refusal code, or `None` when the
/// write is queued.
pub(crate) fn write(
    state: &mut HostState,
    node: NodeId,
    key: &str,
    value: &JsScriptValue,
    animation: Option<JsCurve<'_>>,
) -> Option<&'static str> {
    write_play(state, node, key, value, animation, 0.0, None)
}

/// [`write`] for an animation that waits `delay` seconds before it starts and joins `tween`, the `tween` node whose
/// `progress`, `paused` and `reverse` drive it. A delay or a `tween` without a curve is a write that cannot be.
pub(crate) fn write_play(
    state: &mut HostState,
    node: NodeId,
    key: &str,
    value: &JsScriptValue,
    animation: Option<JsCurve<'_>>,
    delay: f64,
    tween: Option<NodeId>,
) -> Option<&'static str> {
    if !delay.is_finite() || delay < 0.0 || (animation.is_none() && (delay > 0.0 || tween.is_some())) {
        return Some("invalid-value");
    }
    if let Some(tween) = tween {
        match state.tick.node(&state.mirror, tween) {
            None => return Some("unknown-node"),
            Some(record) if lua_class(&record.class) != "Tween" => return Some("invalid-value"),
            Some(_) => {}
        }
    }
    let value = match prepare(state, node, key, value, animation) {
        Ok(value) => value,
        Err(code) => return Some(code),
    };
    let HostState { mirror, tick, .. } = state;
    let (seconds, samples) = match animation {
        Some((seconds, samples)) => (Some(seconds), Some(samples.to_vec())),
        None => (None, None),
    };
    let animation = TxAnimation {
        seconds,
        delay: (delay > 0.0).then_some(delay),
        tween,
        samples,
    };
    tick.set_animated(mirror, node, key, value, animation)
        .err()
        .map(|_| "unknown-node")
}

/// The windows back to front as the tree, with this frame's writes, stacks them: those under the root in tree order,
/// then the ones the compositor made without a parent, by id (the order `parents_first` gives them, which is the order
/// they were mapped in).
fn stacked_windows(state: &HostState) -> Vec<NodeId> {
    let HostState { mirror, tick, .. } = state;
    let is_window = |class: &str| lua_class(class) == "Window";
    let mut order = Vec::new();
    let mut walk = vec![mirror.root()];
    while let Some(id) = walk.pop() {
        let Some(record) = tick.node(mirror, id) else { continue };
        if is_window(&record.class) {
            order.push(id);
        }
        walk.extend(record.children.iter().rev().copied());
    }
    for id in parents_first(mirror) {
        let orphan = tick
            .node(mirror, id)
            .is_some_and(|record| is_window(&record.class) && record.parent == NIL);
        if orphan && !order.contains(&id) {
            order.push(id);
        }
    }
    order
}

/// Parents every unparented window but `skip` to the root, in the order the tree stacks them. The tree's order is the
/// stacking order, but a window the compositor made has no parent and sorts after the root's tree whatever the root
/// holds, so whatever is to go on top of them is put there only once they have the root as their parent.
pub(crate) fn adopt_windows(state: &mut HostState, skip: NodeId) -> Option<&'static str> {
    let order = stacked_windows(state);
    let HostState { mirror, tick, .. } = state;
    let root = mirror.root();
    for id in order.into_iter().filter(|id| *id != skip) {
        let orphan = tick.node(mirror, id).is_some_and(|record| record.parent == NIL);
        if orphan && tick.insert(mirror, id, root, -1).is_err() {
            return Some("unknown-node");
        }
    }
    None
}

/// Queues the moves that put `node` on top of every other window: the unparented windows are first given the root as
/// their parent, in the order they already have, and then `node` goes last. A window that is already on top queues
/// nothing.
pub(crate) fn raise(state: &mut HostState, node: NodeId) -> Option<&'static str> {
    let Some(record) = state.tick.node(&state.mirror, node) else {
        return Some("unknown-node");
    };
    if lua_class(&record.class) != "Window" {
        return Some("unknown-method");
    }
    if stacked_windows(state).last() == Some(&node) {
        return None;
    }
    if let Some(code) = adopt_windows(state, node) {
        return Some(code);
    }
    let HostState { mirror, tick, .. } = state;
    let root = mirror.root();
    tick.insert(mirror, node, root, -1).err().map(|_| "unknown-node")
}

/// Queues the moves that put `node` under every other window: the unparented windows are first given the root as their
/// parent, in the order they already have, and then `node` goes first. A window that is already at the bottom queues
/// nothing.
pub(crate) fn lower(state: &mut HostState, node: NodeId) -> Option<&'static str> {
    let Some(record) = state.tick.node(&state.mirror, node) else {
        return Some("unknown-node");
    };
    if lua_class(&record.class) != "Window" {
        return Some("unknown-method");
    }
    if stacked_windows(state).first() == Some(&node) {
        return None;
    }
    if let Some(code) = adopt_windows(state, node) {
        return Some(code);
    }
    let HostState { mirror, tick, .. } = state;
    let root = mirror.root();
    tick.insert(mirror, node, root, 0).err().map(|_| "unknown-node")
}

/// Queues a call of `method` on `node`. `focus` is not a call the compositor answers: it becomes a write of the seat's
/// `focus`, which is how a shell gives the keyboard focus.
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
    if method == "focus" {
        return crate::input::focus_window(state, node);
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

pub(crate) fn script_value<'js>(value: &Value<'js>) -> JsScriptValue {
    if let Some(flag) = value.as_bool() {
        JsScriptValue::Flag(flag)
    } else if let Some(n) = value.as_number() {
        JsScriptValue::Number(n)
    } else if let Some(text) = value.as_string().and_then(|text| text.to_string().ok()) {
        JsScriptValue::Text(text)
    } else if let Some(array) = value.as_array() {
        if let Ok(list) = array.iter::<f64>().collect::<Result<Vec<_>, _>>() {
            return JsScriptValue::Numbers(list);
        }
        // Read by value so that nothing is coerced: a number is not a name.
        let texts: Option<Vec<String>> = array
            .iter::<Value<'js>>()
            .map(|item| {
                item.ok()
                    .and_then(|item| item.as_string().and_then(|text| text.to_string().ok()))
            })
            .collect();
        if let Some(texts) = texts {
            return JsScriptValue::Texts(texts);
        }
        let effects: Option<Vec<effect::Properties>> = array
            .iter::<Object<'js>>()
            .map(|item| item.ok().and_then(|object| effect_from_js(&object)))
            .collect();
        effects.map_or(JsScriptValue::Other, JsScriptValue::Effects)
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
            let state = shared.borrow();
            kind_of(&state, state.handles.resolve(node)?)
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "get",
        Function::new(ctx.clone(), move |node: f64, key: String| -> Option<JsHostValue> {
            let state = shared.borrow();
            read(&state, state.handles.resolve(node)?, &key, false)
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "published",
        Function::new(ctx.clone(), move |node: f64, key: String| -> Option<JsHostValue> {
            let state = shared.borrow();
            read(&state, state.handles.resolve(node)?, &key, true)
        })?,
    )?;

    let shared = state.clone();
    host.set(
        "set",
        Function::new(
            ctx.clone(),
            move |node: f64, key: String, value: Value<'js>| -> Option<&'static str> {
                let mut state = shared.borrow_mut();
                let Some(node) = state.handles.resolve(node) else {
                    return Some("unknown-node");
                };
                write(&mut state, node, &key, &script_value(&value), None)
            },
        )?,
    )?;

    // `animate(node, key, value, seconds, samples, delay?, tween?)`: the same write, carried out over `seconds` along
    // `samples`, after `delay` seconds, and driven by the `tween` node when one is named (0 is none).
    let shared = state.clone();
    host.set(
        "animate",
        Function::new(
            ctx.clone(),
            move |node: f64,
                  key: String,
                  value: Value<'js>,
                  seconds: f64,
                  samples: Vec<f64>,
                  delay: Opt<f64>,
                  tween: Opt<f64>|
                  -> Option<&'static str> {
                let mut state = shared.borrow_mut();
                let Some(node) = state.handles.resolve(node) else {
                    return Some("unknown-node");
                };
                let tween = match tween.0.filter(|script| *script != 0.0) {
                    Some(script) => match state.handles.resolve(script) {
                        Some(tween) => Some(tween),
                        None => return Some("unknown-node"),
                    },
                    None => None,
                };
                write_play(
                    &mut state,
                    node,
                    &key,
                    &script_value(&value),
                    Some((seconds, &samples[..])),
                    delay.0.unwrap_or(0.0),
                    tween,
                )
            },
        )?,
    )?;

    let shared = state.clone();
    host.set(
        "call",
        Function::new(ctx.clone(), move |node: f64, method: String| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            let Some(node) = state.handles.resolve(node) else {
                return Some("unknown-node");
            };
            call(&mut state, node, &method)
        })?,
    )?;

    // The two stacking moves have one shape: a window node in, a refusal code out.
    type Move = fn(&mut HostState, NodeId) -> Option<&'static str>;
    for (name, queue) in [("raise", raise as Move), ("lower", lower as Move)] {
        let shared = state.clone();
        host.set(
            name,
            Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
                let mut state = shared.borrow_mut();
                let Some(node) = state.handles.resolve(node) else {
                    return Some("unknown-node");
                };
                queue(&mut state, node)
            })?,
        )?;
    }

    let shared = state.clone();
    host.set(
        "label",
        Function::new(ctx.clone(), move |node: f64| -> Option<String> {
            shared.borrow().handles.label(node)
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
        "pointerScreen",
        Function::new(ctx.clone(), move || -> Option<f64> {
            crate::ui::pointer_screen(&shared.borrow())
        })?,
    )?;

    // `gnoblin.spawn` and `gnoblin.process` are the module's own (`data/js/gnoblin/session.ts`) over `host.api`.

    crate::transition::install(ctx, &host, state)?;
    crate::input::install(ctx, &host, state)?;
    crate::ui::install(ctx, &host, state)?;
    crate::xr::install(ctx, &host, state)?;
    crate::config::install(ctx, &host, state)?;
    crate::host_api::install(ctx, &host, state)?;
    crate::lock::install(ctx, &host, state)?;

    ctx.globals().set("__gnoblinHost", host)
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{MirrorValue, SnapshotNode, TreeMirror, TxOp};

    use super::{
        JsScriptValue, call, ids_of_class, lower, parents_first, prepare, raise, read, stacked_windows, write,
    };
    use crate::bindings::{HostState, JsHostValue, NodeHandles};

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
    fn ids_a_script_number_cannot_hold_cross_as_stable_handles_and_come_back() {
        let handles = NodeHandles::default();
        let native = 0x8000_0000_0000_0004_u64;
        let other = 0x8000_0000_0000_0005_u64;
        let a = handles.to_script(native);
        assert!(a < 9_007_199_254_740_992.0, "a handle is an exact script number");
        assert_eq!(
            handles.to_script(native),
            a,
            "the same node is the same number every time"
        );
        assert_ne!(handles.to_script(other), a);
        assert_eq!(handles.resolve(a), Some(native));
        assert_eq!(handles.label(a).as_deref(), Some("9223372036854775812"));
        assert_eq!(handles.to_script(7), 7.0, "a small id is its own number");
        assert_eq!(handles.resolve(7.0), Some(7));
        assert_eq!(
            handles.resolve(a + 1000.0),
            None,
            "a handle never given out names nothing"
        );
        for bad in [-1.0, 1.5, f64::NAN, f64::INFINITY, 1e300] {
            assert_eq!(handles.resolve(bad), None, "{bad}");
        }
    }

    #[test]
    fn parents_come_before_children_whatever_the_snapshot_order() {
        assert_eq!(parents_first(&state().mirror), [1, 2, 3, 4]);
    }

    #[test]
    fn a_node_with_no_parent_is_listed_after_the_roots_tree() {
        // The compositor makes a window unparented; the shell parents it later.
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(2, 1, "seat", Vec::new()),
            node(9, 0, "window", Vec::new()),
            node(5, 0, "window", Vec::new()),
        ];
        let mirror = TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror");
        assert_eq!(parents_first(&mirror), [1, 2, 5, 9]);
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
        assert_eq!(write(&mut state, 3, "x", &JsScriptValue::Number(10.0), None), None);
        assert_eq!(read(&state, 3, "x", false), Some(JsHostValue::Number(10.0)));
        assert_eq!(read(&state, 3, "x", true), Some(JsHostValue::Number(5.0)));
        assert_eq!(state.tick.ops().len(), 1);
    }

    #[test]
    fn a_write_that_is_not_allowed_is_refused_with_a_code_and_queues_nothing() {
        let mut state = state();
        assert_eq!(
            write(&mut state, 3, "x", &JsScriptValue::Number(f64::NAN), None),
            Some("invalid-value")
        );
        assert_eq!(
            write(&mut state, 3, "x", &JsScriptValue::Text("1".into()), None),
            Some("invalid-value")
        );
        assert_eq!(
            write(&mut state, 3, "title", &JsScriptValue::Text("t".into()), None),
            Some("read-only")
        );
        assert_eq!(
            write(&mut state, 3, "nope", &JsScriptValue::Number(1.0), None),
            Some("unknown-property")
        );
        assert_eq!(
            write(&mut state, 99, "x", &JsScriptValue::Number(1.0), None),
            Some("unknown-node")
        );
        assert!(state.tick.ops().is_empty());
    }

    /// A shell with one XR hand, as the program makes it, next to the window of `state`.
    fn hand_state() -> HostState {
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(
                6,
                0,
                "controller",
                vec![("hand".into(), MirrorValue::Text("right".into()))],
            ),
        ];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    fn names(names: &[&str]) -> JsScriptValue {
        JsScriptValue::Texts(names.iter().map(|name| (*name).to_owned()).collect())
    }

    #[test]
    fn a_list_of_names_is_written_to_a_controllers_claims_and_read_back_in_order() {
        let mut state = hand_state();
        assert_eq!(
            read(&state, 6, "claimed", false),
            Some(JsHostValue::Texts(Vec::new())),
            "nothing is claimed by default"
        );
        assert_eq!(
            write(&mut state, 6, "claimed", &names(&["trigger", "grip"]), None),
            None
        );
        assert_eq!(
            read(&state, 6, "claimed", false),
            Some(JsHostValue::Texts(vec!["trigger".into(), "grip".into()]))
        );
        assert!(
            matches!(
                state.tick.ops(),
                [TxOp::Set { node: 6, key, value: MirrorValue::Texts(list), .. }]
                    if key == "claimed" && list == &["trigger", "grip"]
            ),
            "{:?}",
            state.tick.ops()
        );
        // The empty array reaches the host as an empty `Numbers` and still releases every claim.
        assert_eq!(
            write(&mut state, 6, "claimed", &JsScriptValue::Numbers(Vec::new()), None),
            None
        );
        assert_eq!(read(&state, 6, "claimed", false), Some(JsHostValue::Texts(Vec::new())));
    }

    #[test]
    fn a_claim_that_is_not_a_list_of_the_five_lowercase_names_is_refused_and_queues_nothing() {
        let mut state = hand_state();
        for bad in [
            JsScriptValue::Text("grip".into()),
            JsScriptValue::Number(1.0),
            JsScriptValue::Flag(true),
            JsScriptValue::Numbers(vec![1.0]),
            names(&["Grip"]),
            names(&["GRIP"]),
            names(&["grip", "bumper"]),
            names(&[""]),
            JsScriptValue::Other,
        ] {
            assert_eq!(
                write(&mut state, 6, "claimed", &bad, None),
                Some("invalid-value"),
                "{bad:?}"
            );
        }
        assert!(state.tick.ops().is_empty());
        // A controller's other values are the program's, whatever a script writes.
        assert_eq!(
            write(&mut state, 6, "grip", &JsScriptValue::Number(1.0), None),
            Some("read-only")
        );
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
            write(&mut state, 5, "width", &JsScriptValue::Number(1.0), None),
            Some("read-only")
        );
    }

    #[test]
    fn a_screen_reads_its_identity_and_a_window_its_dialog_relation_with_schema_defaults() {
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(
                5,
                1,
                "screen",
                vec![
                    ("make".into(), MirrorValue::Text("Gnoblin".into())),
                    ("mode".into(), MirrorValue::Text("800x600@60".into())),
                ],
            ),
            node(6, 1, "window", vec![("parent_window".into(), MirrorValue::Node(7))]),
            node(7, 1, "window", Vec::new()),
        ];
        let state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        assert_eq!(
            read(&state, 5, "make", false),
            Some(JsHostValue::Text("Gnoblin".into()))
        );
        assert_eq!(
            read(&state, 5, "mode", false),
            Some(JsHostValue::Text("800x600@60".into()))
        );
        assert_eq!(read(&state, 5, "model", false), Some(JsHostValue::Text(String::new())));
        assert_eq!(
            read(&state, 5, "transform", false),
            Some(JsHostValue::Text("normal".into()))
        );
        assert_eq!(read(&state, 5, "output_scale", false), Some(JsHostValue::Number(1.0)));
        assert_eq!(read(&state, 5, "enabled", false), Some(JsHostValue::Flag(true)));
        assert_eq!(read(&state, 5, "primary", false), Some(JsHostValue::Flag(false)));
        assert_eq!(read(&state, 6, "parent_window", false), Some(JsHostValue::Number(7.0)));
        assert_eq!(read(&state, 7, "parent_window", false), Some(JsHostValue::Number(0.0)));
        assert_eq!(read(&state, 7, "from_launcher", false), Some(JsHostValue::Flag(false)));
    }

    #[test]
    fn visible_opacity_and_the_transform_read_as_their_schema_defaults_when_never_written() {
        let state = state();
        assert_eq!(read(&state, 3, "visible", false), Some(JsHostValue::Flag(true)));
        assert_eq!(read(&state, 3, "tiled", false), Some(JsHostValue::Flag(false)));
        assert_eq!(read(&state, 3, "opacity", false), Some(JsHostValue::Number(1.0)));
        assert_eq!(read(&state, 3, "z", false), Some(JsHostValue::Number(0.0)));
        assert_eq!(
            read(&state, 3, "rotation", false),
            Some(JsHostValue::Numbers(vec![0.0, 0.0, 0.0]))
        );
        assert_eq!(
            read(&state, 3, "scale", false),
            Some(JsHostValue::Numbers(vec![1.0, 1.0, 1.0]))
        );
    }

    #[test]
    fn a_list_property_takes_exactly_three_finite_numbers() {
        let mut state = state();
        let list = |numbers: &[f64]| JsScriptValue::Numbers(numbers.to_vec());
        assert_eq!(write(&mut state, 3, "rotation", &list(&[0.0, 1.5, 0.0]), None), None);
        assert_eq!(
            read(&state, 3, "rotation", false),
            Some(JsHostValue::Numbers(vec![0.0, 1.5, 0.0]))
        );
        assert_eq!(
            write(&mut state, 3, "rotation", &list(&[0.0, 1.5]), None),
            Some("invalid-value")
        );
        assert_eq!(
            write(&mut state, 3, "scale", &list(&[1.0, f64::INFINITY, 1.0]), None),
            Some("invalid-value")
        );
        assert_eq!(
            write(&mut state, 3, "scale", &JsScriptValue::Number(2.0), None),
            Some("invalid-value")
        );
        assert_eq!(state.tick.ops().len(), 1);
    }

    #[test]
    fn an_animated_write_queues_its_curve_and_a_bad_curve_queues_nothing() {
        let mut state = state();
        let curve = (0.25, &[0.0, 0.5, 1.0][..]);
        assert_eq!(
            write(&mut state, 3, "x", &JsScriptValue::Number(7.0), Some(curve)),
            None
        );
        assert_eq!(
            state.tick.ops().last(),
            Some(&TxOp::Set {
                node: 3,
                key: "x".to_owned(),
                value: MirrorValue::Double(7.0),
                seconds: Some(0.25),
                delay: None,
                tween: None,
                samples: Some(vec![0.0, 0.5, 1.0]),
            })
        );
        let short = (0.25, &[1.0][..]);
        let outside = (0.25, &[0.0, 2.0][..]);
        let negative = (-1.0, &[0.0, 1.0][..]);
        for bad in [short, outside, negative] {
            assert_eq!(
                write(&mut state, 3, "x", &JsScriptValue::Number(8.0), Some(bad)),
                Some("invalid-value")
            );
        }
        assert_eq!(
            write(&mut state, 3, "visible", &JsScriptValue::Flag(false), Some(curve)),
            Some("not-animated")
        );
        assert_eq!(state.tick.ops().len(), 1);
    }

    #[test]
    fn prepare_answers_what_a_write_would_and_queues_nothing() {
        let state = state();
        let curve = (0.25, &[0.0, 1.0][..]);
        assert_eq!(
            prepare(&state, 3, "x", &JsScriptValue::Number(7.0), None),
            Ok(MirrorValue::Double(7.0))
        );
        assert_eq!(
            prepare(&state, 3, "x", &JsScriptValue::Number(7.0), Some(curve)),
            Ok(MirrorValue::Double(7.0))
        );
        assert_eq!(
            prepare(&state, 3, "visible", &JsScriptValue::Flag(false), Some(curve)),
            Err("not-animated")
        );
        assert_eq!(
            prepare(&state, 3, "x", &JsScriptValue::Text("far".to_owned()), None),
            Err("invalid-value")
        );
        assert_eq!(
            prepare(&state, 3, "nothing", &JsScriptValue::Number(1.0), None),
            Err("unknown-property")
        );
        assert_eq!(
            prepare(&state, 99, "x", &JsScriptValue::Number(1.0), None),
            Err("unknown-node")
        );
        assert!(state.tick.ops().is_empty(), "checking queues no write");
    }

    #[test]
    fn a_focus_call_is_a_write_of_the_seats_focus_and_needs_a_seat() {
        use gnoblin_tree_client::tree_mirror::TxOp;

        let mut state = state();
        assert_eq!(call(&mut state, 3, "focus"), None);
        assert!(
            matches!(
                state.tick.ops(),
                [TxOp::Set { node: 4, key, value: MirrorValue::Node(3), .. }] if key == "focus"
            ),
            "{:?}",
            state.tick.ops()
        );
        assert_eq!(state.tokens, 0, "a write is not a call");
        let nodes = [node(1, 0, "node", Vec::new()), node(3, 1, "window", Vec::new())];
        let mut bare = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        assert_eq!(call(&mut bare, 3, "focus"), Some("no-seat"));
    }

    #[test]
    fn a_raised_window_goes_last_after_the_unparented_windows_are_given_the_root() {
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(3, 0, "window", Vec::new()),
            node(4, 0, "window", Vec::new()),
            node(5, 0, "window", Vec::new()),
        ];
        let mut state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        assert_eq!(
            stacked_windows(&state),
            [3, 4, 5],
            "unparented windows keep their id order"
        );
        assert_eq!(raise(&mut state, 5), None);
        assert!(state.tick.ops().is_empty(), "the top window queues nothing");
        assert_eq!(raise(&mut state, 3), None);
        assert_eq!(stacked_windows(&state), [4, 5, 3]);
        assert_eq!(raise(&mut state, 4), None);
        assert_eq!(
            stacked_windows(&state),
            [5, 3, 4],
            "a second raise in the same frame sees the first"
        );
        assert_eq!(raise(&mut state, 99), Some("unknown-node"));
        assert_eq!(raise(&mut state, 1), Some("unknown-method"));
    }

    #[test]
    fn a_lowered_window_goes_first_after_the_unparented_windows_are_given_the_root() {
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(3, 0, "window", Vec::new()),
            node(4, 0, "window", Vec::new()),
            node(5, 0, "window", Vec::new()),
        ];
        let mut state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        assert_eq!(lower(&mut state, 3), None);
        assert!(state.tick.ops().is_empty(), "the bottom window queues nothing");
        assert_eq!(lower(&mut state, 5), None);
        assert_eq!(stacked_windows(&state), [5, 3, 4]);
        assert_eq!(lower(&mut state, 4), None);
        assert_eq!(
            stacked_windows(&state),
            [4, 5, 3],
            "a second move in the same frame sees the first"
        );
        assert_eq!(lower(&mut state, 99), Some("unknown-node"));
        assert_eq!(lower(&mut state, 1), Some("unknown-method"));
    }

    #[test]
    fn only_close_focus_and_kill_can_be_called_on_a_window() {
        let mut state = state();
        assert_eq!(call(&mut state, 3, "close"), None);
        assert_eq!(call(&mut state, 3, "kill"), None);
        assert_eq!(call(&mut state, 3, "destroy"), Some("unknown-method"));
        assert_eq!(call(&mut state, 99, "close"), Some("unknown-node"));
        assert_eq!(state.tick.ops().len(), 2);
        assert_eq!(state.tokens, 2);
    }
}
