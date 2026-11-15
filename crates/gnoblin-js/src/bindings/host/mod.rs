//! The functions of `globalThis.__gnoblinHost`.
//!
//! Each is a thin closure over a free function that works on [`HostState`] and plain Rust values, so the
//! logic is testable without an engine. A function that can fail returns `undefined` for success and a
//! short code for a refusal (`unknown-node`, `unknown-property`, `read-only`, `invalid-value`,
//! `unknown-method`), which `data/js/gnoblin/host.ts` turns into an error naming the member.
//!
//! The tests are in `tests.rs`.

use gnoblin_proto::tree::effect;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NIL, NodeId, TreeMirror, TxAnimation, class_name};
use rquickjs::function::Opt;
use rquickjs::{Ctx, Function, Object, Value};

use super::props::{JsPropDef, JsPropKind, default_value, defs_for, may_call};
use super::{HostState, JsHostValue, NodeHandles, SharedState, effect_from_js};

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
                .is_some_and(|record| class_name(&record.class) == class)
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

/// What a millisecond is of the seconds the tree's wire counts in.
const MILLIS_PER_SECOND: f64 = 1000.0;

/// The curve of an animated write as the tree takes it: the seconds it takes, and samples of progress from 0 to 1 it follows.
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
            Some(record) if class_name(&record.class) != "Tween" => return Some("invalid-value"),
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
    let is_window = |class: &str| class_name(class) == "Window";
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
    if class_name(&record.class) != "Window" {
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
    if class_name(&record.class) != "Window" {
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

    // `animate(node, key, value, duration, samples, delay?, tween?)`: the same write, carried out over `duration`
    // milliseconds along `samples`, after `delay` milliseconds, and driven by the `tween` node when one is named (0 is
    // none). The script API counts milliseconds everywhere; the tree's wire counts seconds, so the conversion is here.
    let shared = state.clone();
    host.set(
        "animate",
        Function::new(
            ctx.clone(),
            move |node: f64,
                  key: String,
                  value: Value<'js>,
                  duration: f64,
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
                    Some((duration / MILLIS_PER_SECOND, &samples[..])),
                    delay.0.unwrap_or(0.0) / MILLIS_PER_SECOND,
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

    // The nodes whose writes the shell gave up on since the last call, each once, as script numbers.
    let shared = state.clone();
    host.set(
        "lostWrites",
        Function::new(ctx.clone(), move || -> Vec<f64> {
            let mut guard = shared.borrow_mut();
            let state = &mut *guard;
            let mut lost = std::mem::take(&mut state.lost_writes);
            lost.sort_unstable();
            lost.dedup();
            lost.into_iter().map(|node| state.handles.to_script(node)).collect()
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
    crate::output_apply::install(ctx, &host, state)?;

    ctx.globals().set("__gnoblinHost", host)
}

#[cfg(test)]
mod tests;
