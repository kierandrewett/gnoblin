//! The host functions of `gnoblin.requestTransition` (`data/js/gnoblin/transition.ts`): `__gnoblinHost.check` and
//! `__gnoblinHost.tween`.
//!
//! A transition stages the writes of its callback and sends them all once the callback and the layout after it have
//! run without an error. So that a write that the tree would refuse is refused where the script made it, which is
//! where its error names the property, the staging asks the host to check each write first. `check` is the check a
//! queued write goes through, without the queuing, so staging and queuing can never disagree about what is valid.
//!
//! The tracks of a transition join a `tween` node, the compositor's play state for them (`progress`, `paused`,
//! `reverse`; `crates/gnoblin-schema/schema/tree/tween.toml`). `tween.create` makes one and `tween.destroy` removes it
//! when the transition is over.

use gnoblin_tree_client::tree_mirror::{NodeId, TxError, class_name};
use rquickjs::{Ctx, Function, Object, Value};

use crate::bindings::{HostState, JsHostValue, SharedState, prepare, script_value};

/// The curve a timed write is checked with when only its being timed matters: the shortest the tree accepts.
const PROBE: [f64; 2] = [0.0, 1.0];

/// The tree's class of the node that holds a transition's play state.
const TWEEN: &str = "tween";

/// Makes a `tween` node in this tick and returns its id. `Err` is a refusal code: `refused` when the shell has no id
/// left.
pub(crate) fn create(state: &mut HostState) -> Result<NodeId, &'static str> {
    let HostState { mirror, tick, ui, .. } = state;
    let id = ui.mint(mirror.epoch()).ok_or("refused")?;
    tick.new_node(mirror, id, TWEEN).map_err(|_| "refused")?;
    Ok(id)
}

/// Destroys a `tween` node. A node that is already gone is not an error: the compositor drops a shell's nodes when it
/// detaches. A node that is not a `tween` is refused `not-owned`.
pub(crate) fn destroy(state: &mut HostState, node: NodeId) -> Option<&'static str> {
    let HostState { mirror, tick, .. } = state;
    match tick.node(mirror, node) {
        None => return None,
        Some(record) if class_name(&record.class) != "Tween" => return Some("not-owned"),
        Some(_) => {}
    }
    match tick.destroy(mirror, node) {
        Ok(_) | Err(TxError::UnknownNode(_)) => None,
        Err(_) => Some("refused"),
    }
}

/// Installs `check(node, key, value, timed)`: `undefined` when a write of `value` to `node.key` is valid, otherwise
/// the code the write would be refused with. `timed` also refuses a property the tree cannot animate. And
/// `tween.create()` (the node's number, or a refusal code as text) and `tween.destroy(node)`.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let shared = state.clone();
    host.set(
        "check",
        Function::new(
            ctx.clone(),
            move |node: f64, key: String, value: Value<'js>, timed: bool| -> Option<&'static str> {
                let state = shared.borrow();
                let Some(node) = state.handles.resolve(node) else {
                    return Some("unknown-node");
                };
                let curve = timed.then_some((1.0, &PROBE[..]));
                prepare(&state, node, &key, &script_value(&value), curve).err()
            },
        )?,
    )?;

    let tween = Object::new(ctx.clone())?;
    let shared = state.clone();
    tween.set(
        "create",
        Function::new(ctx.clone(), move || -> JsHostValue {
            let mut state = shared.borrow_mut();
            match create(&mut state) {
                Ok(id) => JsHostValue::Number(state.handles.to_script(id)),
                Err(code) => JsHostValue::Text(code.to_owned()),
            }
        })?,
    )?;
    let shared = state.clone();
    tween.set(
        "destroy",
        Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(node) {
                Some(node) => destroy(&mut state, node),
                None => None,
            }
        })?,
    )?;
    host.set("tween", tween)
}
