//! The host function of `gnoblin.requestTransition` (`data/js/gnoblin/transition.ts`): `__gnoblinHost.check`.
//!
//! A transition stages the writes of its callback and sends them all once the callback and the layout after it have
//! run without an error. So that a write that the tree would refuse is refused where the script made it, which is
//! where its error names the property, the staging asks the host to check each write first. `check` is the check a
//! queued write goes through, without the queuing, so staging and queuing can never disagree about what is valid.

use rquickjs::{Ctx, Function, Object, Value};

use crate::bindings::{SharedState, prepare, script_value};

/// The curve a timed write is checked with when only its being timed matters: the shortest the tree accepts.
const PROBE: [f64; 2] = [0.0, 1.0];

/// Installs `check(node, key, value, timed)`: `undefined` when a write of `value` to `node.key` is valid, otherwise
/// the code the write would be refused with. `timed` also refuses a property the tree cannot animate.
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
    )
}
