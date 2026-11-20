//! `operation`, `output_operation` and `timer`: a request the compositor answers later, and a call that waits for its time.
//!
#![allow(unsafe_code)]
use std::cell::Cell;
use std::ffi::c_void;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{Callable, Fail, Obj, Res, Val};

use crate::registry;

/// Where an operation stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OpStatus {
    Pending,
    Succeeded,
    Failed,
}

/// A request the compositor answers later.
pub(super) struct OperationState {
    status: OpStatus,
    error: String,
    code: String,
    on_done: Vec<Rc<dyn Callable>>,
    on_failed: Vec<Rc<dyn Callable>>,
    /// The output an `output_operation` is for.
    output: Option<Obj>,
}

impl OperationState {
    pub(super) fn for_output(output: Obj) -> OperationState {
        OperationState {
            status: OpStatus::Pending,
            error: String::new(),
            code: String::new(),
            on_done: Vec::new(),
            on_failed: Vec::new(),
            output: Some(output),
        }
    }
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
    let told = cx.with_state::<OperationState, _>(op, |state| {
        if state.status != OpStatus::Pending {
            return None;
        }
        let (run, drop) = match outcome {
            Ok(()) => {
                state.status = OpStatus::Succeeded;
                (std::mem::take(&mut state.on_done), std::mem::take(&mut state.on_failed))
            }
            Err(error) => {
                state.status = OpStatus::Failed;
                state.error = error.to_owned();
                state.code = failure_code(error).to_owned();
                (std::mem::take(&mut state.on_failed), std::mem::take(&mut state.on_done))
            }
        };
        drop_all(drop);
        Some((run, state.error.clone(), state.code.clone(), outcome.is_ok()))
    });
    let Ok(Some((run, error, code, ok))) = told else {
        return;
    };
    cx.emit_on(op, "settled", vec![cx.object_value(op)]);
    queue_calls(cx, op, run, ok, error, code);
}

fn drop_all(closures: Vec<Rc<dyn Callable>>) {
    drop(closures);
}

/// Runs the closures of a settled operation from the next turn.
fn queue_calls(cx: &Cx<'_>, op: Obj, run: Vec<Rc<dyn Callable>>, ok: bool, error: String, code: String) {
    if run.is_empty() {
        return;
    }
    cx.later_on(op, move |cx| {
        for closure in run {
            let args: Vec<Val> = if ok {
                Vec::new()
            } else {
                vec![Val::Text(error.clone()), Val::Text(code.clone())]
            };
            if let Err(fail) = closure.call(0, &args) {
                cx.log(abi::LOG_WARN, &format!("an operation's listener failed: {fail}"));
            }
        }
    });
}

/// The members of the base class, for each class that has them.
fn base<'a>(class: ClassBuilder<'a, OperationState>) -> ClassBuilder<'a, OperationState> {
    class
        .ctor(|_, _| Err(Fail::refused("an operation is made by the library, from a request")))
        .get("status", |state, cx| {
            let name = match state.status {
                OpStatus::Pending => "pending",
                OpStatus::Succeeded => "succeeded",
                OpStatus::Failed => "failed",
            };
            cx.enum_val("operation_status", name)
        })
        .get("error", |state, _| Ok(Val::Text(state.error.clone())))
        .get("error_code", |state, _| Ok(Val::Text(state.code.clone())))
        .method("then", |state, cx, args| {
            let run = Rc::clone(args[0].closure()?);
            match state.status {
                OpStatus::Pending => state.on_done.push(run),
                OpStatus::Succeeded => queue_calls(cx, cx.me(), vec![run], true, String::new(), String::new()),
                OpStatus::Failed => {}
            }
            Ok(Val::None)
        })
        .method("catch", |state, cx, args| {
            let run = Rc::clone(args[0].closure()?);
            match state.status {
                OpStatus::Pending => state.on_failed.push(run),
                OpStatus::Failed => queue_calls(cx, cx.me(), vec![run], false, state.error.clone(), state.code.clone()),
                OpStatus::Succeeded => {}
            }
            Ok(Val::None)
        })
}

/// A call that runs later.
pub(super) struct TimerState {
    pending: Rc<Cell<bool>>,
    /// The registration that fires it.
    reg: u64,
}

/// What the registration of a timer carries: the generation, the timer's object and the shell's function.
struct TimerFire {
    ctx: abi::GnoblinCtx,
    timer: Obj,
    run: Rc<dyn Callable>,
    pending: Rc<Cell<bool>>,
}

unsafe extern "C" fn fire(user: *mut c_void) -> i32 {
    // SAFETY: `user` is the box `start` made; the registration calls this once and then drops it.
    let fire = unsafe { &*(user as *const TimerFire) };
    if !fire.pending.replace(false) {
        return abi::E_OK;
    }
    let run = Rc::clone(&fire.run);
    let timer = fire.timer;
    let _ = crate::object::run_in(fire.ctx, timer, |cx| {
        if let Err(fail) = run.call(0, &[]) {
            cx.log(abi::LOG_WARN, &format!("a timer's function failed: {fail}"));
        }
        // The library's own reference ends with the timer: one nobody else holds is freed.
        cx.unhold(timer);
    });
    abi::E_OK
}

unsafe extern "C" fn forget(user: *mut c_void) {
    // SAFETY: `user` is the box `start` made, given up by the registration exactly once.
    drop(unsafe { Box::from_raw(user as *mut TimerFire) });
}

/// The most a timer may wait: a day, in milliseconds.
const MAX_TIMER_MS: f64 = 86_400_000.0;

/// Starts a timer for `run` in `ms` milliseconds and returns the object that stands for it.
pub(super) fn start(cx: &Cx<'_>, ms: f64, run: Rc<dyn Callable>) -> Res<Val> {
    if !ms.is_finite() || !(0.0..=MAX_TIMER_MS).contains(&ms) {
        return Err(Fail::type_error(format!(
            "gnoblin.timer: ms must be a finite number from 0 to {MAX_TIMER_MS}"
        )));
    }
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
        run,
        pending,
    }));
    let reg = crate::listen::timer_add(cx.ctx(), ms.ceil() as u32, 0, fire, user.cast(), Some(forget));
    if reg == 0 {
        // SAFETY: the registration refused the box, so it is still this call's.
        drop(unsafe { Box::from_raw(user) });
        return Err(Fail::err("the timer could not be started"));
    }
    cx.with_state::<TimerState, _>(timer, |state| state.reg = reg)?;
    cx.handout(timer)
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
        .ctor(|_, _| Err(Fail::refused("a timer is made with `gnoblin.timer`")))
        .get("pending", |state, _| Ok(Val::Bool(state.pending.get())))
        .method("cancel", |state, cx, _| {
            if state.pending.replace(false) {
                registry::remove(state.reg);
                cx.unhold(cx.me());
            }
            Ok(Val::None)
        });
}
