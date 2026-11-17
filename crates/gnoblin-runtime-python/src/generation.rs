//! The shell this thread runs: its generations, and what a script holds.
//!
//! One interpreter serves for the life of the process, and the loader makes a generation (a context) for each evaluation of the
//! script. This module keeps, on the shell thread, the SDK handle of each live generation, which one the module `_gnoblin_core`
//! acts on (the staged one while the script evaluates, the serving one afterwards), and the handles a script holds: the
//! subscriptions it made and the programs it started. A script holds a number, never a pointer, so a handle that outlives its
//! generation is refused and cannot be used to reach another.
//!
//! The thread-local is the rule "everything on the shell thread": a call from another thread finds no state and is refused with a
//! message that says to `post` the work. The calls that are safe anywhere (`log`, `post`) go through the [`Poster`] kept beside it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use gnoblin_core::{Gnoblin, LogLevel, Poster, Process, Subscription};
use gnoblin_core_abi::{GnoblinApi, GnoblinCtx};
use pyo3::prelude::*;

use crate::errors::to_py;
use crate::watch::Watch;

/// Everything the runtime keeps for the shell thread.
pub(crate) struct State {
    pub(crate) api: &'static GnoblinApi,
    /// The handle of each generation that has not ended.
    pub(crate) handles: HashMap<GnoblinCtx, Gnoblin>,
    /// The generation `_gnoblin_core` acts on: the one whose script is evaluating, then the one that serves.
    pub(crate) current: GnoblinCtx,
    /// The generation that serves, once one does.
    pub(crate) serving: Option<GnoblinCtx>,
    /// The generation `start` staged and `activate` has not committed.
    pub(crate) staged: Option<GnoblinCtx>,
    subscriptions: HashMap<u64, (GnoblinCtx, Subscription)>,
    processes: HashMap<u64, (GnoblinCtx, Process)>,
    next_token: u64,
    /// The files of the serving generation, and what to do when one changes.
    pub(crate) watch: Watch,
    /// The files of the staged generation, which become the watch when it is activated.
    pub(crate) pending_watch: Option<Vec<(String, i128, u64, u64)>>,
    /// What builds a `gnoblin.Node` from an id, for a value that is a node.
    pub(crate) node_factory: Option<Py<PyAny>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The poster of the generation that serves (or, before one does, the one that evaluates): callable from any thread.
static POSTER: Mutex<Option<Poster>> = Mutex::new(None);

/// What a call from a thread that is not the shell thread is told.
pub(crate) const WRONG_THREAD: &str =
    "gnoblin calls run on the shell thread; from another thread, hand the work to `gnoblin.post(fn)`";

/// Runs `f` on the state, or returns `None` when this thread has none (not the shell thread, or no shell is running).
pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|slot| slot.borrow_mut().as_mut().map(f))
}

/// Creates the state for this thread, once.
pub(crate) fn init(api: &'static GnoblinApi) {
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(State {
                api,
                handles: HashMap::new(),
                current: 0,
                serving: None,
                staged: None,
                subscriptions: HashMap::new(),
                processes: HashMap::new(),
                next_token: 1,
                watch: Watch::default(),
                pending_watch: None,
                node_factory: None,
            });
        }
    });
}

/// The handle `_gnoblin_core` acts on.
///
/// # Errors
///
/// This is not the shell thread, or no generation is running.
pub(crate) fn handle() -> PyResult<Gnoblin> {
    with_state(|state| state.handles.get(&state.current).cloned())
        .ok_or_else(|| to_py(None, WRONG_THREAD))?
        .ok_or_else(|| to_py(None, "no Gnoblin shell generation is running"))
}

/// The context `_gnoblin_core` acts on, `0` when none.
pub(crate) fn current_ctx() -> GnoblinCtx {
    with_state(|state| state.current).unwrap_or(0)
}

/// Keeps `subscription` for the generation `ctx`, and returns the number the script holds.
pub(crate) fn keep_subscription(ctx: GnoblinCtx, subscription: Subscription) -> u64 {
    with_state(|state| {
        let token = state.next_token;
        state.next_token += 1;
        state.subscriptions.insert(token, (ctx, subscription));
        token
    })
    .unwrap_or(0)
}

/// Removes the registration behind `token`. A number that is not (or no longer) held is an error.
pub(crate) fn take_subscription(token: u64) -> PyResult<Subscription> {
    with_state(|state| state.subscriptions.remove(&token).map(|(_, subscription)| subscription))
        .ok_or_else(|| to_py(None, WRONG_THREAD))?
        .ok_or_else(|| to_py(None, "that subscription was removed already, or its generation ended"))
}

/// Keeps a program the script started, and returns the number the script holds.
pub(crate) fn keep_process(ctx: GnoblinCtx, process: Process) -> u64 {
    with_state(|state| {
        let token = state.next_token;
        state.next_token += 1;
        state.processes.insert(token, (ctx, process));
        token
    })
    .unwrap_or(0)
}

/// The program behind `token`.
pub(crate) fn process(token: u64) -> PyResult<Process> {
    with_state(|state| state.processes.get(&token).map(|(_, process)| process.clone()))
        .ok_or_else(|| to_py(None, WRONG_THREAD))?
        .ok_or_else(|| to_py(None, "that program is not one this generation started"))
}

/// Forgets everything the generation `ctx` held. Its registrations are the core's to remove.
pub(crate) fn release(ctx: GnoblinCtx) {
    with_state(|state| {
        state.subscriptions.retain(|_, (owner, _)| *owner != ctx);
        state.processes.retain(|_, (owner, _)| *owner != ctx);
        state.handles.remove(&ctx);
    });
}

/// Points the poster at `ctx`'s generation.
pub(crate) fn aim_poster(ctx: GnoblinCtx) {
    let poster = with_state(|state| state.handles.get(&ctx).map(Gnoblin::poster)).flatten();
    *POSTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = poster;
}

/// Forgets the poster: no generation serves.
pub(crate) fn clear_poster() {
    *POSTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// The poster, for `log` and `post` from any thread.
pub(crate) fn poster() -> Option<Poster> {
    *POSTER.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Writes a line to the core's log, from any thread.
pub(crate) fn log(level: LogLevel, text: &str) {
    match poster() {
        Some(poster) => poster.log_at(level, text),
        None => eprintln!("gnoblin python: {text}"),
    }
}

/// Writes an error line to the core's log.
pub(crate) fn log_error(text: &str) {
    log(LogLevel::CoreError, text);
}

/// Sets the text the loader reports for a failure of the generation `ctx`.
pub(crate) fn set_last_error(ctx: GnoblinCtx, text: &str) {
    if let Some(api) = with_state(|state| state.api) {
        crate::table::set_last_error(api, ctx, text);
    }
}

/// Runs what the core queued for the generation `ctx`, waiting up to `wait` for the first. The shell thread holds no lock of the
/// interpreter while it waits (the lock is taken by each callback and let go when it returns), so a thread of the script runs meanwhile.
///
/// # Errors
///
/// The text of the failure: the generation is not one this thread runs, or the core's dispatch failed.
pub(crate) fn dispatch(ctx: GnoblinCtx, wait: Duration) -> Result<u32, String> {
    let handle = with_state(|state| state.handles.get(&ctx).cloned())
        .flatten()
        .ok_or_else(|| format!("the Python runtime has no generation {ctx:#x} on this thread"))?;
    let millis = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
    handle
        .dispatch(millis)
        .map_err(|error| format!("the core's dispatch failed: {error}"))
}
