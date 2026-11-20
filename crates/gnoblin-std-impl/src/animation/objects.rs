//! The class of `transition.toml`: staging the writes of a transition, committing them as timed writes, and controlling and
//! reporting the transition that results.
//!
//! `data/js/gnoblin-std/transition/` is the reference, and the engine behind it (`engine.rs`, `staging.rs`) is the same one the
//! table's `transition_*` functions drive for the TypeScript library. What the object protocol adds is the `rollback`: the shell's own
//! callback, held for as long as the transition is being staged, and called when the transition is aborted or its commit is refused,
//! after the library has put back what it holds that the callback may have changed (the styles).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::LogLevel;
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Callable, Fail, Obj, Res, Val};
use gnoblin_std_abi as sabi;

use super::easing::Easing;
use super::engine::Options;
use super::staging::{self, Prop};
use super::table::commit_staged;
use crate::object::record::{edit, enumeration, field, flag_field, look, number_field, record};
use crate::object::with_lib;
use crate::state::{Id, State};
use crate::table::quiet_of;
use crate::turn::schedule;

thread_local! {
    /// The object that stands for each transition a shell was given, to tell when it ends.
    static HANDLES: RefCell<HashMap<(GnoblinCtx, Id), Obj>> = RefCell::new(HashMap::new());
}

/// What a transition was when the library last said.
#[derive(Clone, Copy)]
struct Info {
    state: u32,
    duration_ms: f64,
    progress: f64,
    paused: bool,
    reversed: bool,
}

/// What a `transition` object holds.
struct TransitionRef {
    ctx: GnoblinCtx,
    id: Id,
    /// What the library last said it was: the library forgets a transition a while after it ends.
    last: Cell<Option<Info>>,
    /// How it ended, when the turn has told this object.
    ended: Cell<Option<bool>>,
}

impl TransitionRef {
    /// What the transition is now, or what it was when it ended and the library forgot it.
    fn info(&self) -> Res<Info> {
        match look(self.ctx, |st| st.transition_info(self.id)) {
            Ok(raw) => {
                let info = Info {
                    state: raw.state,
                    duration_ms: raw.duration_ms,
                    progress: raw.progress,
                    paused: raw.paused != 0,
                    reversed: raw.reversed != 0,
                };
                self.last.set(Some(info));
                Ok(info)
            }
            Err(fail) if fail.code == abi::E_STALE => {
                let mut info = self.last.get().ok_or(fail)?;
                if let Some(cancelled) = self.ended.get() {
                    info.state = if cancelled {
                        sabi::STD_TRANSITION_CANCELLED
                    } else {
                        sabi::STD_TRANSITION_FINISHED
                    };
                }
                Ok(info)
            }
            Err(fail) => Err(fail),
        }
    }

    /// How the transition ended, when it has.
    fn outcome(&self) -> Option<bool> {
        let info = self.info().ok()?;
        match info.state {
            sabi::STD_TRANSITION_CANCELLED => Some(true),
            sabi::STD_TRANSITION_FINISHED => Some(false),
            _ => None,
        }
    }
}

/// The object that stands for the transition `id`, made for the shell.
fn transition_val(cx: &mut Cx<'_>, id: Id) -> Res<Val> {
    let ctx = cx.ctx();
    let obj = cx.spawn(
        "transition",
        TransitionRef {
            ctx,
            id,
            last: Cell::new(None),
            ended: Cell::new(None),
        },
    )?;
    HANDLES.with(|handles| handles.borrow_mut().insert((ctx, id), obj));
    cx.handout(obj)
}

/// Tells the objects of the transitions that settled that they did, from the next turn.
pub(crate) fn announce(ctx: GnoblinCtx, settled: Vec<(Id, bool)>) {
    for (id, cancelled) in settled {
        let Some(obj) = HANDLES.with(|handles| handles.borrow_mut().remove(&(ctx, id))) else {
            continue;
        };
        let _ = with_lib(|lib| {
            lib.with_cx(ctx, obj, |cx| {
                let _ = cx.with_state(obj, |held: &mut TransitionRef| held.ended.set(Some(cancelled)));
                cx.emit("finished", vec![Val::Bool(cancelled)]);
            })
        });
    }
}

// ---- values ---------------------------------------------------------------------------------------------------------------------

/// The places of `easing`, counted from one, are the ABI's numbers from zero.
fn easing_of(value: &Val) -> Res<(u32, Easing)> {
    let wire = value.enum_value()?.wrapping_sub(1);
    let easing = Easing::from_abi(wire).ok_or_else(|| Fail::type_error("not a value of `easing`"))?;
    Ok((wire, easing))
}

/// The options a `transition_options` record says, as the library's checks take them: a record or nothing, left out as the defaults.
fn options_raw(value: &Val) -> Res<sabi::GnoblinStdTransitionOptions> {
    let none: &[Val] = &[];
    let fields = if value.is_none() { none } else { value.record()? };
    let easing = match field(fields, 1) {
        Some(easing) => easing_of(easing)?.0,
        None => sabi::STD_EASING_EASE_OUT,
    };
    Ok(sabi::GnoblinStdTransitionOptions {
        size: size_of::<sabi::GnoblinStdTransitionOptions>() as u32,
        autoplay: u32::from(flag_field(fields, 3, true)?),
        easing,
        reserved: 0,
        duration_ms: number_field(fields, 0, 250.0)?,
        delay_ms: number_field(fields, 2, 0.0)?,
    })
}

/// The value of a property a `transition_value` record gives: exactly one of its fields.
fn prop_of(value: &Val) -> Res<Prop> {
    let fields = value.record()?;
    let given: Vec<usize> = (0..5).filter(|at| field(fields, *at).is_some()).collect();
    let [at] = given[..] else {
        return Err(Fail::type_error(
            "a transition value holds exactly one of `number`, `flag`, `text`, `triple` and `node`",
        ));
    };
    let held = &fields[at];
    Ok(match at {
        0 => Prop::Number(held.double()?),
        1 => Prop::Bool(held.bool()?),
        2 => Prop::Text(held.text()?.to_owned()),
        3 => {
            let parts = held.list()?;
            let [x, y, z] = parts else {
                return Err(Fail::type_error("a `triple` is three numbers"));
            };
            Prop::Triple([x.double()?, y.double()?, z.double()?])
        }
        _ => Prop::Node(held.uint()?),
    })
}

/// A property's value as a `transition_value` record: every field is left out for a node that has no such property.
fn value_of(prop: Option<&Prop>) -> Val {
    let mut fields = vec![Val::None; 5];
    match prop {
        Some(Prop::Number(number)) => fields[0] = Val::Double(*number),
        Some(Prop::Bool(flag)) => fields[1] = Val::Bool(*flag),
        Some(Prop::Text(text)) => fields[2] = Val::Text(text.clone()),
        Some(Prop::Triple(parts)) => fields[3] = Val::List(parts.iter().map(|part| Val::Double(*part)).collect()),
        Some(Prop::Node(node)) => fields[4] = Val::Uint(*node),
        None => {}
    }
    record(fields)
}

// ---- staging --------------------------------------------------------------------------------------------------------------------

/// Starts staging, holding `rollback` for as long as it lasts.
fn begin(ctx: GnoblinCtx, options: &Val, rollback: Option<Rc<dyn Callable>>) -> Res<()> {
    let raw = options_raw(options)?;
    edit(ctx, |st| st.transition_begin_undoable(&raw, rollback))
}

/// Puts back what a transition that does not commit changed: the library's own state first, then the shell's, by its rollback. A
/// rollback that fails is reported and does not stop the failure that called it.
fn roll_back(ctx: GnoblinCtx) {
    let Ok(std) = quiet_of(ctx) else { return };
    let undo = std.st.borrow_mut().transition_take_undo();
    let Some(undo) = undo else { return };
    let staging::Undo { rollback, styles } = undo;
    std.st.borrow_mut().styles.restore(styles);
    schedule(&std);
    if let Some(rollback) = rollback
        && let Err(fail) = rollback.call(0, &[])
    {
        std.g().log_at(
            LogLevel::Warn,
            &format!("gnoblin-std: a transition's rollback failed: {}", fail.message),
        );
    }
}

/// Ends staging and drops what was staged, then puts back what the transition's callback may have changed.
fn abort(ctx: GnoblinCtx) -> Res<()> {
    let std = quiet_of(ctx).map_err(Fail::from)?;
    if staging::active() {
        std.st.borrow_mut().transition_abort();
        roll_back(ctx);
    }
    Ok(())
}

/// Ends staging and starts the transition; when the commit is refused the transition is rolled back.
fn commit(cx: &mut Cx<'_>) -> Res<Val> {
    let ctx = cx.ctx();
    let std = quiet_of(ctx).map_err(Fail::from)?;
    match commit_staged(&std) {
        Ok(id) => {
            // A transition that committed has nothing to put back: the rollback is let go.
            let undo = std.st.borrow_mut().transition_take_undo();
            drop(undo);
            transition_val(cx, id)
        }
        Err(fail) => {
            roll_back(ctx);
            Err(fail.into())
        }
    }
}

/// Runs `work` once, and commits what it staged.
fn request(cx: &mut Cx<'_>, args: &[Val]) -> Res<Val> {
    let work = Rc::clone(args[0].closure()?);
    let rollback = (!args[2].is_none()).then(|| args[2].closure().cloned()).transpose()?;
    begin(cx.ctx(), &args[1], rollback)?;
    match work.call(0, &[]) {
        Ok(_) => commit(cx),
        Err(fail) => {
            abort(cx.ctx())?;
            Err(fail)
        }
    }
}

fn control(h: &TransitionRef, op: u32, value: f64) -> Res<Val> {
    edit(h.ctx, |st: &mut State| st.transition_control(h.id, op, value))?;
    Ok(Val::None)
}

/// A transition as the options and the property of a timed write give it: the one `animate` makes.
fn animate(cx: &mut Cx<'_>, args: &[Val]) -> Res<Val> {
    let node = args[0].uint()?;
    let key = args[1].text()?.to_owned();
    let prop = prop_of(&args[2])?;
    let raw = options_raw(&args[3])?;
    let options = Options::check(&raw, "gnoblin.transition").map_err(Fail::from)?;
    let made = edit(cx.ctx(), |st| st.transition_animate(node, &key, prop, &options))?;
    match made {
        Some(id) => transition_val(cx, id),
        None => Ok(Val::None),
    }
}

/// Implements the class of the transition module.
pub(crate) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<TransitionRef>("transition")
        .ctor(|_, _| {
            Err(Fail::refused(
                "a transition is the one a commit made: stage its writes with `begin` or `request`, and `commit`",
            ))
        })
        .get("id", |h, _| Ok(Val::Uint(h.id)))
        .get("duration", |h, _| Ok(Val::Double(h.info()?.duration_ms)))
        .get("state", |h, _| Ok(enumeration(h.info()?.state + 1)))
        .get("running", |h, _| {
            Ok(Val::Bool(h.info()?.state == sabi::STD_TRANSITION_RUNNING))
        })
        .get("paused", |h, _| Ok(Val::Bool(h.info()?.paused)))
        .get("reversed", |h, _| Ok(Val::Bool(h.info()?.reversed)))
        .get("progress", |h, _| Ok(Val::Double(h.info()?.progress)))
        .set("progress", |h, _, value| {
            control(h, sabi::STD_CONTROL_SEEK, value.double()?).map(drop)
        })
        .method("play", |h, _, _| control(h, sabi::STD_CONTROL_PLAY, 0.0))
        .method("pause", |h, _, _| control(h, sabi::STD_CONTROL_PAUSE, 0.0))
        .method("reverse", |h, _, _| control(h, sabi::STD_CONTROL_REVERSE, 0.0))
        .method("cancel", |h, _, _| control(h, sabi::STD_CONTROL_CANCEL, 0.0))
        .replay("finished", |cx| {
            let me = cx.me();
            cx.with_state(me, |h: &mut TransitionRef| h.outcome())
                .ok()
                .flatten()
                .map_or_else(Vec::new, |cancelled| vec![vec![Val::Bool(cancelled)]])
        })
        .function("begin", |cx, args| {
            let rollback = (!args[1].is_none()).then(|| args[1].closure().cloned()).transpose()?;
            begin(cx.ctx(), &args[0], rollback)?;
            Ok(Val::None)
        })
        .function("set", |cx, args| {
            let node = args[0].uint()?;
            let key = args[1].text()?.to_owned();
            let prop = prop_of(&args[2])?;
            edit(cx.ctx(), |st| st.transition_set(node, &key, prop))?;
            Ok(Val::None)
        })
        .function("peek", |cx, args| {
            let node = args[0].uint()?;
            let key = args[1].text()?.to_owned();
            look(cx.ctx(), |st| Ok(value_of(st.transition_get(node, &key).as_ref())))
        })
        .function("release", |cx, args| {
            let node = args[0].uint()?;
            let key = args[1].text()?.to_owned();
            edit(cx.ctx(), |st| {
                st.transition_release(node, &key);
                Ok(())
            })?;
            Ok(Val::None)
        })
        .function("commit", |cx, _| commit(cx))
        .function("abort", |cx, _| abort(cx.ctx()).map(|()| Val::None))
        .function("request", request)
        .function("animate", animate)
        .function("sample", |_, args| {
            let (_, easing) = easing_of(&args[0])?;
            Ok(Val::Double(easing.sample(args[1].double()?)))
        });
}
