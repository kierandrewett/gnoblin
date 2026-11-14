//! The host function behind `Output.apply`: `__gnoblinHost.outputs`.
//!
//! The compositor's `screen` node has one call, `apply`, whose text is the request (`key=value` lines). The compositor
//! runs it through its transactional output engine and answers later, under the call's token: an empty `error` is
//! success, anything else is why the arrangement was left as it was. Nothing here decides; it sends the text and keeps
//! the answers for the script's next frame, like the lock screen's password check.

use std::collections::HashSet;

use gnoblin_tree_client::tree_mirror::lua_class;
use rquickjs::{Ctx, Function, IntoJs, Object, Value};

use crate::bindings::{HostState, SharedState};

/// The compositor's answer to an `apply` call, as the script reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JsApplyAnswer {
    token: f64,
    error: String,
}

impl<'js> IntoJs<'js> for JsApplyAnswer {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("token", self.token)?;
        object.set("error", self.error)?;
        Ok(object.into_value())
    }
}

/// What the outputs module keeps on the host between frames.
#[derive(Default)]
pub(crate) struct JsOutputApplies {
    /// The tokens of the `apply` calls sent and not yet answered.
    asked: HashSet<u64>,
    answers: Vec<JsApplyAnswer>,
}

impl JsOutputApplies {
    /// Keeps the answer to an `apply` this module sent. Returns false for any other call's answer.
    pub(crate) fn accept_answer(&mut self, token: u64, error: &str) -> bool {
        if !self.asked.remove(&token) {
            return false;
        }
        self.answers.push(JsApplyAnswer {
            token: token as f64,
            error: error.to_owned(),
        });
        true
    }
}

/// Sends `text` as `apply` on the screen node a script holds. Returns the token the answer comes back under, or
/// `None` when the node is not a screen.
pub(crate) fn apply(state: &mut HostState, node: f64, text: String) -> Option<u64> {
    let node = state.handles.resolve(node)?;
    let HostState {
        mirror,
        tick,
        tokens,
        outputs,
        ..
    } = state;
    if tick
        .node(mirror, node)
        .is_none_or(|record| lua_class(&record.class) != "Screen")
    {
        return None;
    }
    *tokens += 1;
    let token = *tokens;
    tick.call(
        mirror,
        node,
        "apply",
        Vec::new(),
        token,
        gnoblin_secret::SecretText::new(text),
    )
    .ok()?;
    outputs.asked.insert(token);
    Some(token)
}

/// Installs `__gnoblinHost.outputs`.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let outputs = Object::new(ctx.clone())?;

    let shared = state.clone();
    outputs.set(
        "apply",
        Function::new(ctx.clone(), move |node: f64, text: String| -> Option<f64> {
            apply(&mut shared.borrow_mut(), node, text).map(|token| token as f64)
        })?,
    )?;

    let shared = state.clone();
    outputs.set(
        "takeAnswers",
        Function::new(ctx.clone(), move || -> Vec<JsApplyAnswer> {
            std::mem::take(&mut shared.borrow_mut().outputs.answers)
        })?,
    )?;

    host.set("outputs", outputs)
}
