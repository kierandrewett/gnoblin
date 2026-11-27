//! `operation`, `output_operation`, `timer` and `timers`: a request the compositor answers later, a call that waits for its time, and the
//! service that sets it.
//!
//! An operation settles once. A shell hears it with `then`, `catch` and the `settled` signal, and follows it with `map` (a new operation
//! that carries what a function makes of the value) and `timeout` (a new one that fails `timed_out` when this one is late). Both make
//! their operation with [`begin`] and chain it to the source ([`Chained`]), which the source runs in the turn after it settled.
//!
//! `cancel` settles a pending operation as `cancelled`. The request behind it is not undone, because most requests cannot be: a result
//! that comes later settles nothing, as the operation settled already. An owner of a request it can drop registers the way with
//! [`on_cancel`], which runs once when the operation is cancelled and never when it settles by itself (`timeout` ends its timer so).
#![allow(unsafe_code)]
use std::cell::Cell;
use std::ffi::c_void;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{Callable, ErrorCode, Fail, Obj, Res, Val};

use crate::{registry, session};

/// Where an operation stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OpStatus {
    Pending,
    Succeeded,
    Failed,
}

/// How a settled operation ended.
#[derive(Clone)]
enum Settlement {
    /// It succeeded with a value (none for an operation that carries none).
    Done(Val),
    /// It failed with the words and the code.
    Failed { error: String, code: String },
}

/// What a source runs in the turn after it settled, for an operation made from it.
enum Chained {
    /// `map`: the value goes through `run` and its answer settles `derived`; a failure passes as it is.
    Map { derived: Obj, run: Rc<dyn Callable> },
    /// `timeout`: `derived` settles as the source did, and the deadline's `timer`, if there is one, ends.
    Limit { derived: Obj, timer: Option<Obj> },
}

/// What a library owner of a pending request does when the operation is cancelled.
type Hook = Box<dyn FnOnce(&Cx<'_>)>;

/// A request the compositor answers later.
pub(super) struct OperationState {
    status: OpStatus,
    value: Val,
    error: String,
    code: String,
    on_done: Vec<Rc<dyn Callable>>,
    on_failed: Vec<Rc<dyn Callable>>,
    followers: Vec<Chained>,
    on_cancel: Option<Hook>,
    /// The library holds the operation until it settles ([`begin`]), whatever the shell does with the value it got.
    held: bool,
    /// The output an `output_operation` is for.
    output: Option<Obj>,
}

impl OperationState {
    fn new(held: bool, output: Option<Obj>) -> OperationState {
        OperationState {
            status: OpStatus::Pending,
            value: Val::None,
            error: String::new(),
            code: String::new(),
            on_done: Vec::new(),
            on_failed: Vec::new(),
            followers: Vec::new(),
            on_cancel: None,
            held,
            output,
        }
    }

    pub(super) fn for_output(output: Obj) -> OperationState {
        OperationState::new(false, Some(output))
    }

    /// How it ended; only for one that settled.
    fn outcome(&self) -> Settlement {
        match self.status {
            OpStatus::Failed => Settlement::Failed {
                error: self.error.clone(),
                code: self.code.clone(),
            },
            _ => Settlement::Done(self.value.clone()),
        }
    }
}

/// What a settled operation tells, taken out of its state so that the turn that tells it holds nothing of the operation.
struct Told {
    run: Vec<Rc<dyn Callable>>,
    followers: Vec<Chained>,
    outcome: Settlement,
    /// The library lets go of the operation once it has told.
    release: bool,
}

/// The code of an `apply` the compositor refused, from the words it gave.
fn failure_code(refusal: &str) -> &'static str {
    if let Some(rest) = refusal.strip_prefix("failed: the transaction was refused ")
        && let Some(word) = rest.split_whitespace().next()
    {
        return match word {
            "unknown-node" | "unknown-property" => "not_found",
            "unknown-method" | "not-animated" => "unsupported",
            "read-only" => "read_only",
            "invalid-value" => "invalid_argument",
            "no-seat" => "unavailable",
            _ => "denied",
        };
    }
    if refusal.starts_with("transform: ") {
        "unsupported"
    } else if refusal == "the output is not connected" {
        "not_found"
    } else {
        "denied"
    }
}

/// Settles an operation once: it succeeded, or failed with the compositor's words. Its listeners are told from the next turn.
pub(super) fn settle(cx: &Cx<'_>, op: Obj, outcome: Result<(), &str>) {
    settle_coded(cx, op, outcome.map_err(|words| (failure_code(words), words)));
}

/// Settles an operation once, as [`settle`], with a failure that names its own code (`denied`, `unavailable`, `config_rejected`, ...).
pub(super) fn settle_coded(cx: &Cx<'_>, op: Obj, outcome: Result<(), (&str, &str)>) {
    settle_with(cx, op, outcome.map(|()| Val::None));
}

/// Settles an operation once with a value or a failure.
fn settle_with(cx: &Cx<'_>, op: Obj, outcome: Result<Val, (&str, &str)>) {
    if let Ok(Some(told)) = cx.with_state::<OperationState, _>(op, |state| settle_in(state, outcome)) {
        announce(cx, op, told);
    }
}

/// Settles the state of an operation, once: nothing for one that settled already.
fn settle_in(state: &mut OperationState, outcome: Result<Val, (&str, &str)>) -> Option<Told> {
    if state.status != OpStatus::Pending {
        return None;
    }
    // A hook that was not asked for goes with the operation: the owner's request ended by itself.
    state.on_cancel = None;
    let (run, drop, outcome) = match outcome {
        Ok(value) => {
            state.status = OpStatus::Succeeded;
            state.value = value.clone();
            (
                std::mem::take(&mut state.on_done),
                std::mem::take(&mut state.on_failed),
                Settlement::Done(value),
            )
        }
        Err((code, error)) => {
            state.status = OpStatus::Failed;
            state.error = error.to_owned();
            state.code = code.to_owned();
            (
                std::mem::take(&mut state.on_failed),
                std::mem::take(&mut state.on_done),
                Settlement::Failed {
                    error: error.to_owned(),
                    code: code.to_owned(),
                },
            )
        }
    };
    drop_all(drop);
    Some(Told {
        run,
        followers: std::mem::take(&mut state.followers),
        outcome,
        release: state.held,
    })
}

/// Tells of a settled operation: the `settled` signal now, and from the next turn its listeners and the operations that follow it.
fn announce(cx: &Cx<'_>, op: Obj, told: Told) {
    cx.emit_on(op, "settled", vec![cx.object_value(op)]);
    let Told {
        run,
        followers,
        outcome,
        release,
    } = told;
    if run.is_empty() && followers.is_empty() && !release {
        return;
    }
    cx.later_on(op, move |cx| {
        call_all(cx, run, &outcome);
        for follower in followers {
            follow(cx, follower, &outcome);
        }
        if release {
            cx.unhold(op);
        }
    });
}

/// A new pending operation the library holds until it settles, whatever the shell does with the value it got: the request the library
/// itself makes of the compositor or of the system (a session verb, a process's end), and the operation `map` and `timeout` make.
pub(super) fn begin(cx: &Cx<'_>) -> Res<Obj> {
    let op = cx.spawn("operation", OperationState::new(true, None))?;
    cx.hold(op)?;
    Ok(op)
}

/// Settles an operation [`begin`] made; the library lets go of it once the shell has been told. A result that comes after a `cancel`
/// settles nothing.
pub(super) fn finish(cx: &Cx<'_>, op: Obj, outcome: Result<(), (&str, &str)>) {
    settle_coded(cx, op, outcome);
}

/// Registers what to do when the operation is cancelled, for an owner of a pending request it can drop. It runs once, in the turn of the
/// `cancel`, after the operation settled as `cancelled`; an operation that settles any other way forgets it.
pub(super) fn on_cancel(cx: &Cx<'_>, op: Obj, hook: Hook) {
    let _ = cx.with_state::<OperationState, _>(op, |state| {
        if state.status == OpStatus::Pending {
            state.on_cancel = Some(hook);
        }
    });
}

/// An operation that has failed already, as the value a call returns: `unavailable` for a request that could not go out.
pub(super) fn failed(cx: &Cx<'_>, code: &str, message: &str) -> Res<Val> {
    let op = begin(cx)?;
    finish(cx, op, Err((code, message)));
    cx.handout(op)
}

fn drop_all(closures: Vec<Rc<dyn Callable>>) {
    drop(closures);
}

/// Runs the closures of a settled operation: `then` and `catch` functions take what the outcome says.
fn call_all(cx: &Cx<'_>, run: Vec<Rc<dyn Callable>>, outcome: &Settlement) {
    let args: Vec<Val> = match outcome {
        Settlement::Done(_) => Vec::new(),
        Settlement::Failed { error, code } => vec![Val::Text(error.clone()), Val::Text(code.clone())],
    };
    for closure in run {
        if let Err(fail) = closure.call(0, &args) {
            cx.log(abi::LOG_WARN, &format!("an operation's listener failed: {fail}"));
        }
    }
}

/// Runs the closures of a settled operation from the next turn.
fn queue_calls(cx: &Cx<'_>, op: Obj, run: Vec<Rc<dyn Callable>>, outcome: Settlement) {
    if run.is_empty() {
        return;
    }
    cx.later_on(op, move |cx| call_all(cx, run, &outcome));
}

/// Gives a source an operation that follows it: from the next turn when the source settled already, else when it settles.
fn attach(state: &mut OperationState, cx: &Cx<'_>, follower: Chained) {
    if state.status == OpStatus::Pending {
        state.followers.push(follower);
    } else {
        let outcome = state.outcome();
        cx.later_on(cx.me(), move |cx| follow(cx, follower, &outcome));
    }
}

/// What an operation that follows another does with the way the source ended.
fn follow(cx: &Cx<'_>, follower: Chained, outcome: &Settlement) {
    match follower {
        Chained::Map { derived, run } => match outcome {
            Settlement::Done(value) => match run.call(0, std::slice::from_ref(value)) {
                Ok(mapped) if mapped.is_plain() => settle_with(cx, derived, Ok(mapped)),
                Ok(other) => settle_with(
                    cx,
                    derived,
                    Err((
                        "invalid_argument",
                        &format!(
                            "the function of `map` answered {}, which an operation cannot carry",
                            other.kind_name()
                        ),
                    )),
                ),
                // A script's throw comes with its stack; the operation says what went wrong.
                Err(fail) => settle_with(
                    cx,
                    derived,
                    Err(("internal", fail.message.lines().next().unwrap_or_default())),
                ),
            },
            Settlement::Failed { error, code } => settle_with(cx, derived, Err((code.as_str(), error.as_str()))),
        },
        Chained::Limit { derived, timer } => {
            if let Some(timer) = timer {
                stop_timer(cx, timer);
            }
            match outcome {
                Settlement::Done(value) => settle_with(cx, derived, Ok(value.clone())),
                Settlement::Failed { error, code } => settle_with(cx, derived, Err((code.as_str(), error.as_str()))),
            }
        }
    }
}

/// The members of the base class, for each class that has them.
fn base<'a>(class: ClassBuilder<'a, OperationState>) -> ClassBuilder<'a, OperationState> {
    class
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "an operation is made by the library, from a request",
            ))
        })
        .get("status", |state, cx| {
            let name = match state.status {
                OpStatus::Pending => "pending",
                OpStatus::Succeeded => "succeeded",
                OpStatus::Failed => "failed",
            };
            cx.enum_val("operation_status", name)
        })
        .get("value", |state, _| Ok(state.value.clone()))
        .get("error", |state, _| Ok(Val::Text(state.error.clone())))
        .get("error_code", |state, _| Ok(Val::Text(state.code.clone())))
        .method("then", |state, cx, args| {
            let run = Rc::clone(args[0].closure()?);
            match state.status {
                OpStatus::Pending => state.on_done.push(run),
                OpStatus::Succeeded => queue_calls(cx, cx.me(), vec![run], state.outcome()),
                OpStatus::Failed => {}
            }
            Ok(Val::None)
        })
        .method("catch", |state, cx, args| {
            let run = Rc::clone(args[0].closure()?);
            match state.status {
                OpStatus::Pending => state.on_failed.push(run),
                OpStatus::Failed => queue_calls(cx, cx.me(), vec![run], state.outcome()),
                OpStatus::Succeeded => {}
            }
            Ok(Val::None)
        })
        .method("map", |state, cx, args| {
            let run = Rc::clone(args[0].closure()?);
            let derived = begin(cx)?;
            attach(state, cx, Chained::Map { derived, run });
            cx.handout(derived)
        })
        .method("timeout", |state, cx, args| {
            let ms = args[0].double()?;
            check_ms("operation.timeout", ms)?;
            let derived = begin(cx)?;
            let mut timer = None;
            if state.status == OpStatus::Pending {
                let late = move |cx: &Cx<'_>| {
                    let words = format!("the operation took over {ms} ms");
                    settle_with(cx, derived, Err(("timed_out", &words)));
                };
                match start_with(cx, ms, Box::new(late)) {
                    Ok(started) => {
                        on_cancel(cx, derived, Box::new(move |cx| stop_timer(cx, started)));
                        timer = Some(started);
                    }
                    Err(fail) => {
                        finish(cx, derived, Err(("unavailable", &fail.message)));
                        return cx.handout(derived);
                    }
                }
            }
            attach(state, cx, Chained::Limit { derived, timer });
            cx.handout(derived)
        })
        .method("cancel", |state, cx, _| {
            let hook = state.on_cancel.take();
            let Some(told) = settle_in(state, Err(("cancelled", "the operation was cancelled"))) else {
                return Ok(Val::Bool(false));
            };
            announce(cx, cx.me(), told);
            if let Some(hook) = hook {
                hook(cx);
            }
            Ok(Val::Bool(true))
        })
}

/// A call that runs later.
pub(super) struct TimerState {
    pending: Rc<Cell<bool>>,
    /// The registration that fires it.
    reg: u64,
}

/// What the registration of a timer carries: the generation, the timer's object and what runs when it fires.
struct TimerFire {
    ctx: abi::GnoblinCtx,
    timer: Obj,
    work: Cell<Option<Hook>>,
    pending: Rc<Cell<bool>>,
}

unsafe extern "C" fn fire(user: *mut c_void) -> i32 {
    // SAFETY: `user` is the box `start_with` made; the registration calls this once and then drops it.
    let fire = unsafe { &*(user as *const TimerFire) };
    if !fire.pending.replace(false) {
        return abi::E_OK;
    }
    let work = fire.work.take();
    let timer = fire.timer;
    let _ = crate::object::run_in(fire.ctx, timer, |cx| {
        if let Some(work) = work {
            work(cx);
        }
        // The library's own reference ends with the timer: one nobody else holds is freed.
        cx.unhold(timer);
    });
    abi::E_OK
}

unsafe extern "C" fn forget(user: *mut c_void) {
    // SAFETY: `user` is the box `start_with` made, given up by the registration exactly once.
    drop(unsafe { Box::from_raw(user as *mut TimerFire) });
}

/// The most a timer may wait: a day, in milliseconds.
const MAX_TIMER_MS: f64 = 86_400_000.0;

/// Refuses a wait that is not a finite number of milliseconds from 0 to a day.
fn check_ms(call: &str, ms: f64) -> Res<()> {
    if ms.is_finite() && (0.0..=MAX_TIMER_MS).contains(&ms) {
        Ok(())
    } else {
        Err(Fail::type_error(format!(
            "{call}: ms must be a finite number from 0 to {MAX_TIMER_MS}"
        )))
    }
}

/// Starts a timer for `run` in `ms` milliseconds and returns the object that stands for it.
pub(super) fn start(cx: &Cx<'_>, ms: f64, run: Rc<dyn Callable>) -> Res<Val> {
    check_ms("gnoblin.timers.after", ms)?;
    let timer = start_with(
        cx,
        ms,
        Box::new(move |cx| {
            if let Err(fail) = run.call(0, &[]) {
                cx.log(abi::LOG_WARN, &format!("a timer's function failed: {fail}"));
            }
        }),
    )?;
    cx.handout(timer)
}

/// Starts a timer that runs `work` in `ms` milliseconds, in a call about the timer. The library holds the timer until it ran or ended.
fn start_with(cx: &Cx<'_>, ms: f64, work: Hook) -> Res<Obj> {
    let pending = Rc::new(Cell::new(true));
    let timer = cx.spawn(
        "timer",
        TimerState {
            pending: Rc::clone(&pending),
            reg: 0,
        },
    )?;
    cx.hold(timer)?;
    let user = Box::into_raw(Box::new(TimerFire {
        ctx: cx.ctx(),
        timer,
        work: Cell::new(Some(work)),
        pending,
    }));
    let reg = crate::listen::timer_add(cx.ctx(), ms.ceil() as u32, 0, fire, user.cast(), Some(forget));
    if reg == 0 {
        // SAFETY: the registration refused the box, so it is still this call's.
        drop(unsafe { Box::from_raw(user) });
        return Err(Fail::err("the timer could not be started"));
    }
    cx.with_state::<TimerState, _>(timer, |state| state.reg = reg)?;
    Ok(timer)
}

/// Stops a timer: it never runs after this. One that ran or ended already is ignored.
fn halt(cx: &Cx<'_>, state: &mut TimerState, timer: Obj) {
    if state.pending.replace(false) {
        registry::remove(state.reg);
        cx.unhold(timer);
    }
}

/// [`halt`] for a timer the caller holds by its object.
fn stop_timer(cx: &Cx<'_>, timer: Obj) {
    let _ = cx.with_state::<TimerState, _>(timer, |state| halt(cx, state, timer));
}

pub(super) fn register(builder: &mut Builder) {
    let class = builder.class::<OperationState>("operation");
    let _ = base(class);
    let class = builder.class::<OperationState>("output_operation");
    let _ = base(class).get("output", |state, cx| match state.output {
        Some(output) => cx.handout(output),
        None => Ok(Val::None),
    });
    let _ = builder
        .class::<TimerState>("timer")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "a timer is made with `gnoblin.timers.after`",
            ))
        })
        .get("pending", |state, _| Ok(Val::Bool(state.pending.get())))
        .method("cancel", |state, cx, _| {
            halt(cx, state, cx.me());
            Ok(Val::None)
        });
    let _ = builder
        .class::<()>("timers")
        .ctor(|_, _| Ok(()))
        .method("after", |_, cx, args| {
            start(cx, args[0].double()?, Rc::clone(args[1].closure()?))
        })
        .method("now", |_, _, _| {
            Ok(Val::Double(session::with(|session| session.now_ms())))
        });
}
