//! The host functions of the stock lock screen (`data/js/gnoblin/lock.ts`): `__gnoblinHost.lock`.
//!
//! The compositor's tree has one `lock` node. Its `state` is `not-locked`, `pending` or `locked`; the shell records
//! that its lock screen is up in `engaged`, and asks the compositor to check a password with the node's
//! `authenticate` call. The compositor answers later, under the call's token, and unlocks the session itself on an
//! accepted check: nothing here can unlock.
//!
//! The password is the call's secret text. It crosses from the script as one string argument, goes straight into a
//! [`SecretText`] that the transaction wipes once it is sent, and is never kept, logged or put in a retry (the
//! shell's sent log keeps only the token of a call the schema marks secret, and answers it `failed` when its
//! transaction is refused, so the screen takes another attempt). The copy the script held is the script's to clear,
//! and a JavaScript string cannot be wiped: `docs/session-lock.md` says what that leaves.
//!
//! The screen is alive only while the compositor says so: it renews a lease with the node's `heartbeat` call
//! ([`heartbeat`]), and a check the compositor does not answer within [`CHECK_DEADLINE`] is answered `failed` here.
//! Without either, a lock screen that hung or lost a check would hold the lock UI with nothing to type into.
//!
//! Like the other host functions, each is a thin closure over a free function on [`HostState`], so the logic is
//! testable without an engine.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NodeId, class_name};
use rquickjs::{Ctx, Function, IntoJs, Object, Value};

use crate::bindings::{HostState, SharedState, parents_first};

/// What `state` reads while the tree has no lock node (a tree that is not the compositor's).
const NOT_LOCKED: &str = "not-locked";

/// The least time between two `heartbeat` calls. The compositor's lease on the lock screen lasts two seconds
/// (`LOCK_UI_LEASE_MS` in `gnoblin-wayland`), so twice a second is a margin of three lost beats and costs one
/// transaction every half second while the session is locked.
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);

/// How long a password check may go unanswered before the screen is told it failed. The compositor answers every
/// check it receives, so an answer that has not come by now never will (the check was lost on the way); the screen
/// then takes another attempt instead of waiting for ever. A PAM check that takes longer than this still unlocks the
/// session when it ends: only the screen's wait is over.
const CHECK_DEADLINE: Duration = Duration::from_secs(30);

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
pub(crate) struct JsLockState {
    /// The password checks asked and not yet answered, by token, with when each was asked.
    checks: HashMap<u64, Duration>,
    answers: Vec<JsLockAnswer>,
    /// The clock the deadlines and the heartbeat count on: the shell's, or the one a test injected.
    clock: Arc<dyn crate::supervisor::JsClock>,
    /// When the last `heartbeat` call was queued.
    last_beat: Option<Duration>,
}

impl JsLockState {
    pub(crate) fn new(clock: Arc<dyn crate::supervisor::JsClock>) -> Self {
        Self {
            checks: HashMap::new(),
            answers: Vec::new(),
            clock,
            last_beat: None,
        }
    }

    /// Keeps the answer to a password check this module asked. Returns false for any other call's answer.
    pub(crate) fn accept_answer(&mut self, token: u64, result: &[u8], error: &str) -> bool {
        if self.checks.remove(&token).is_none() {
            return false;
        }
        self.answers.push(JsLockAnswer {
            token: token as f64,
            result: String::from_utf8_lossy(result).into_owned(),
            error: error.to_owned(),
        });
        true
    }

    /// The password checks waited on now, kept by a stage to give back.
    pub(crate) fn mark(&self) -> HashMap<u64, Duration> {
        self.checks.clone()
    }

    /// Gives back what a failed stage asked: its checks were rolled back with the pending writes, so nothing will
    /// answer them.
    pub(crate) fn rollback(&mut self, checks: HashMap<u64, Duration>) {
        self.checks = checks;
    }

    /// Answers `failed` for every check that has waited longer than [`CHECK_DEADLINE`], and hands over the answers.
    fn take_answers(&mut self) -> Vec<JsLockAnswer> {
        let now = self.clock.now();
        let late: Vec<u64> = self
            .checks
            .iter()
            .filter(|(_, asked)| now.saturating_sub(**asked) >= CHECK_DEADLINE)
            .map(|(token, _)| *token)
            .collect();
        for token in late {
            self.accept_answer(token, b"", "failed: the compositor did not answer the password check");
        }
        std::mem::take(&mut self.answers)
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
    let asked = lock.clock.now();
    lock.checks.insert(token, asked);
    Some(token)
}

/// Renews the lease the compositor gives the lock screen, at most once every [`HEARTBEAT_INTERVAL`]: a call on the lock
/// node that has no answer and changes no node. It is not a secret and may go again, so a refusal that sends it again
/// does no harm; a lost one is made up for by the next. The compositor's own lock card takes the password when the
/// beats stop (`docs/session-lock.md`).
pub(crate) fn heartbeat(state: &mut HostState) {
    let Some(node) = lock_node(state) else {
        return;
    };
    let now = state.lock.clock.now();
    if state
        .lock
        .last_beat
        .is_some_and(|at| now.saturating_sub(at) < HEARTBEAT_INTERVAL)
    {
        return;
    }
    let HostState { mirror, tick, lock, .. } = state;
    if tick
        .call(mirror, node, "heartbeat", Vec::new(), 0, SecretText::new(String::new()))
        .is_ok()
    {
        lock.last_beat = Some(now);
    }
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
        "heartbeat",
        Function::new(ctx.clone(), move || heartbeat(&mut shared.borrow_mut()))?,
    )?;

    let shared = state.clone();
    lock.set(
        "takeAnswers",
        Function::new(ctx.clone(), move || -> Vec<JsLockAnswer> {
            shared.borrow_mut().lock.take_answers()
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
        state_on(lock_state, Arc::new(crate::supervisor::JsSystemClock))
    }

    fn state_on(lock_state: &str, clock: Arc<dyn crate::supervisor::JsClock>) -> HostState {
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
        HostState::with_clock(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"), clock)
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

    fn beats(state: &HostState) -> usize {
        state
            .tick
            .ops()
            .iter()
            .filter(|op| matches!(op, TxOp::Call { node: 2, method, .. } if method == "heartbeat"))
            .count()
    }

    #[test]
    fn a_heartbeat_is_a_call_on_the_lock_node_at_most_twice_a_second() {
        let clock = crate::supervisor::JsManualClock::default();
        let mut state = state_on("locked", Arc::new(clock.clone()));
        heartbeat(&mut state);
        heartbeat(&mut state);
        assert_eq!(beats(&state), 1, "two in one instant are one beat");
        clock.advance(Duration::from_millis(499));
        heartbeat(&mut state);
        assert_eq!(beats(&state), 1, "the next waits for the interval");
        clock.advance(Duration::from_millis(1));
        heartbeat(&mut state);
        assert_eq!(beats(&state), 2);
    }

    #[test]
    fn a_tree_with_no_lock_has_nothing_to_renew() {
        let nodes = [node(1, 0, "node", Vec::new())];
        let mut bare = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        heartbeat(&mut bare);
        assert!(bare.tick.ops().is_empty());
    }

    #[test]
    fn a_check_nobody_answers_is_answered_failed_once_its_deadline_has_passed() {
        let clock = crate::supervisor::JsManualClock::default();
        let mut state = state_on("locked", Arc::new(clock.clone()));
        let token = authenticate(&mut state, "hunter2".to_owned()).expect("a token");
        clock.advance(CHECK_DEADLINE - Duration::from_millis(1));
        assert!(
            state.lock.take_answers().is_empty(),
            "the check is still within its time"
        );
        clock.advance(Duration::from_millis(1));
        let answers = state.lock.take_answers();
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].token, token as f64);
        assert!(answers[0].error.starts_with("failed: "), "{:?}", answers[0]);
        assert!(state.lock.take_answers().is_empty(), "the failure is given once");
        assert!(
            !state.lock.accept_answer(token, b"ok", ""),
            "the compositor's late answer finds nothing waiting"
        );
    }
}
