//! The host functions of the stock lock screen (`data/js/gnoblin/lock.ts`): `__gnoblinHost.lock`.
//!
//! The compositor's tree has one `lock` node. Its `state` is `not-locked`, `pending` or `locked`; the shell records
//! that its lock screen is up in `engaged`, and asks the compositor to check a password with the node's
//! `authenticate` call. The compositor answers later, under the call's token, and unlocks the session itself on an
//! accepted check: nothing here can unlock.
//!
//! The password is the call's secret text. It crosses from the script as one string argument, goes straight into a
//! [`SecretText`] that the transaction wipes once it is sent, and is never kept, logged or put in a retry (the
//! shell's sent-log drops a call that carries text). The copy the script held is the script's to clear, and a
//! JavaScript string cannot be wiped: `docs/session-lock.md` says what that leaves.
//!
//! Like the other host functions, each is a thin closure over a free function on [`HostState`], so the logic is
//! testable without an engine.

use std::collections::HashSet;

use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NodeId, class_name};
use rquickjs::{Ctx, Function, IntoJs, Object, Value};

use crate::bindings::{HostState, SharedState, parents_first};

/// What `state` reads while the tree has no lock node (a tree that is not the compositor's).
const NOT_LOCKED: &str = "not-locked";

/// The compositor's answer to an `authenticate` call, as the script reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JsLockAnswer {
    token: f64,
    result: String,
    error: String,
}

impl<'js> IntoJs<'js> for JsLockAnswer {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("token", self.token)?;
        object.set("result", self.result)?;
        object.set("error", self.error)?;
        Ok(object.into_value())
    }
}

/// What the lock screen keeps on the host between frames.
#[derive(Default)]
pub(crate) struct JsLockState {
    /// The tokens of the password checks asked and not yet answered.
    checks: HashSet<u64>,
    answers: Vec<JsLockAnswer>,
}

impl JsLockState {
    /// Keeps the answer to a password check this module asked. Returns false for any other call's answer.
    pub(crate) fn accept_answer(&mut self, token: u64, result: &[u8], error: &str) -> bool {
        if !self.checks.remove(&token) {
            return false;
        }
        self.answers.push(JsLockAnswer {
            token: token as f64,
            result: String::from_utf8_lossy(result).into_owned(),
            error: error.to_owned(),
        });
        true
    }
}

/// The tree's lock node, if it has one.
fn lock_node(state: &HostState) -> Option<NodeId> {
    parents_first(&state.mirror).into_iter().find(|id| {
        state
            .mirror
            .node(*id)
            .is_some_and(|record| class_name(&record.class) == "Lock")
    })
}

/// The session lock's state as the compositor last published it.
pub(crate) fn lock_state(state: &HostState) -> String {
    let Some(node) = lock_node(state) else {
        return NOT_LOCKED.to_owned();
    };
    match state.mirror.node(node).and_then(|record| record.props.get("state")) {
        Some(MirrorValue::Text(text)) => text.clone(),
        _ => NOT_LOCKED.to_owned(),
    }
}

/// The size of the screen the lock screen covers: the primary one, else the first.
pub(crate) fn screen_size(state: &HostState) -> Option<(f64, f64)> {
    let screens: Vec<NodeId> = parents_first(&state.mirror)
        .into_iter()
        .filter(|id| {
            state
                .mirror
                .node(*id)
                .is_some_and(|record| class_name(&record.class) == "Screen")
        })
        .collect();
    let primary = |id: &&NodeId| {
        matches!(
            state.mirror.node(**id).and_then(|record| record.props.get("primary")),
            Some(MirrorValue::Bool(true))
        )
    };
    let screen = screens.iter().find(primary).or_else(|| screens.first())?;
    let number = |key: &str| match state.mirror.node(*screen)?.props.get(key) {
        Some(MirrorValue::Double(value)) => Some(*value),
        _ => None,
    };
    Some((number("width")?, number("height")?))
}

/// Records whether the shell's lock screen is up. The compositor reads it to know the lock UI is held, and routes the
/// keys typed under its own lock to the focused node while it is true.
pub(crate) fn set_engaged(state: &mut HostState, engaged: bool) -> Option<&'static str> {
    let Some(node) = lock_node(state) else {
        return Some("no-lock");
    };
    let HostState { mirror, tick, .. } = state;
    tick.set(mirror, node, "engaged", MirrorValue::Bool(engaged), None, None)
        .err()
        .map(|_| "unknown-node")
}

/// Asks the compositor to check `password` against the session user. Returns the token the answer comes back under,
/// or `None` when the tree has no lock node.
pub(crate) fn authenticate(state: &mut HostState, password: String) -> Option<u64> {
    let node = lock_node(state)?;
    let HostState {
        mirror,
        tick,
        tokens,
        lock,
        ..
    } = state;
    *tokens += 1;
    let token = *tokens;
    tick.call(
        mirror,
        node,
        "authenticate",
        Vec::new(),
        token,
        SecretText::new(password),
    )
    .ok()?;
    lock.checks.insert(token);
    Some(token)
}

/// Installs `__gnoblinHost.lock`, the functions `gnoblin/lock` runs on.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let lock = Object::new(ctx.clone())?;

    let shared = state.clone();
    lock.set(
        "state",
        Function::new(ctx.clone(), move || -> String { lock_state(&shared.borrow()) })?,
    )?;

    let shared = state.clone();
    lock.set(
        "node",
        Function::new(ctx.clone(), move || -> Option<f64> {
            let state = shared.borrow();
            lock_node(&state).map(|node| state.handles.to_script(node))
        })?,
    )?;

    let shared = state.clone();
    lock.set(
        "screen",
        Function::new(ctx.clone(), move || -> Option<Vec<f64>> {
            screen_size(&shared.borrow()).map(|(width, height)| vec![width, height])
        })?,
    )?;

    let shared = state.clone();
    lock.set(
        "engage",
        Function::new(ctx.clone(), move |engaged: bool| -> Option<&'static str> {
            set_engaged(&mut shared.borrow_mut(), engaged)
        })?,
    )?;

    let shared = state.clone();
    lock.set(
        "authenticate",
        Function::new(ctx.clone(), move |password: String| -> Option<f64> {
            authenticate(&mut shared.borrow_mut(), password).map(|token| token as f64)
        })?,
    )?;

    let shared = state.clone();
    lock.set(
        "takeAnswers",
        Function::new(ctx.clone(), move || -> Vec<JsLockAnswer> {
            std::mem::take(&mut shared.borrow_mut().lock.answers)
        })?,
    )?;

    host.set("lock", lock)
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{SnapshotNode, TreeMirror, TxOp};

    use super::*;

    fn node(id: NodeId, parent: NodeId, class: &str, props: Vec<(String, MirrorValue)>) -> SnapshotNode {
        SnapshotNode {
            id,
            parent,
            index: 0,
            class: class.to_owned(),
            title: String::new(),
            app_id: String::new(),
            props,
        }
    }

    fn state(lock_state: &str) -> HostState {
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(
                2,
                1,
                "lock",
                vec![("state".to_owned(), MirrorValue::Text(lock_state.to_owned()))],
            ),
            node(
                3,
                1,
                "screen",
                vec![
                    ("width".to_owned(), MirrorValue::Double(1000.0)),
                    ("height".to_owned(), MirrorValue::Double(800.0)),
                ],
            ),
        ];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    #[test]
    fn the_state_is_the_nodes_and_a_tree_with_no_lock_is_not_locked() {
        assert_eq!(lock_state(&state("locked")), "locked");
        assert_eq!(lock_state(&state("pending")), "pending");
        let nodes = [node(1, 0, "node", Vec::new())];
        let bare = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        assert_eq!(lock_state(&bare), "not-locked");
    }

    #[test]
    fn the_screen_is_the_one_the_lock_covers() {
        assert_eq!(screen_size(&state("locked")), Some((1000.0, 800.0)));
    }

    #[test]
    fn a_password_check_is_a_call_with_the_password_as_its_secret_text_and_no_arguments() {
        let mut state = state("locked");
        let token = authenticate(&mut state, "hunter2".to_owned()).expect("a token");
        let calls: Vec<(NodeId, String, Vec<u8>, String)> = state
            .tick
            .ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::Call {
                    node,
                    method,
                    args,
                    text,
                    ..
                } => Some((*node, method.clone(), args.clone(), text.expose().to_owned())),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            [(2, "authenticate".to_owned(), Vec::new(), "hunter2".to_owned())]
        );
        assert!(
            !state.lock.accept_answer(token + 1, b"", "denied"),
            "another call's answer"
        );
        assert!(state.lock.accept_answer(token, b"", "denied"));
        assert!(
            !state.lock.accept_answer(token, b"", "denied"),
            "an answer is kept once"
        );
        assert_eq!(state.lock.answers.len(), 1);
    }

    #[test]
    fn engaged_is_written_on_the_lock_node() {
        let mut state = state("locked");
        assert_eq!(set_engaged(&mut state, true), None);
        assert!(state.tick.ops().iter().any(|op| matches!(
            op,
            TxOp::Set { node: 2, key, value: MirrorValue::Bool(true), .. } if key == "engaged"
        )));
    }

    #[test]
    fn a_tree_with_no_lock_refuses_both() {
        let nodes = [node(1, 0, "node", Vec::new())];
        let mut bare = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        assert_eq!(set_engaged(&mut bare, true), Some("no-lock"));
        assert_eq!(authenticate(&mut bare, "x".to_owned()), None);
    }
}
