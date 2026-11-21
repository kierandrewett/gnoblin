//! Touchpad gestures: a claimed swipe or pinch, and what the fingers do until they leave the pad.
//!
//! The compositor sends a claimed gesture as `begin`, `update`, `end` and `cancel`, and this matcher keeps one run for each claim from
//! `begin` to the end of its hooks. As for a pointer press ([`super::pointer`]), what a message decides is decided from a turn queued for it.
//!
//! - **The candidates** of a gesture are the registrations of the claim whose scope accepts the window under the pointer, best first.
//! - **A direction.** A trigger with a direction (`Swipe3Left`, `Pinch2In`) waits until the fingers have gone far enough to have one: a
//!   swipe 12 touchpad units, a pinch 0.1 of scale. It is offered the gesture only if the gesture went its way. While the first such
//!   candidate has no direction to judge by, the ones after it wait.
//! - **A function** bound to a gesture has no state to keep, so it takes the gesture at once and runs once, when the gesture ends having
//!   gone far enough its way. An object (`gesture_binding`) is told every step.
//! - **A cancel** reaches the binding that took the gesture when the compositor ends it, the window under it closes, the binding is
//!   removed, the context ends, or the generation does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{Obj, Val};

use super::context::hit_client;
use super::hooks::{self, Basis, InputHook};
use super::trigger::SeatClaim;
use super::{BindingAction, BindingRegistration, ClaimEntry, Matcher, seat, with};
use crate::events::ClaimNotice;

/// Touchpad units of movement of the centre of the fingers before a swipe has a direction.
const SWIPE_DECIDE: f64 = 12.0;
/// How far a scale must be from 1 before a pinch has a direction.
const PINCH_DECIDE: f64 = 0.1;
/// Touchpad units for a whole swipe (`progress` 1), and the change of scale for a whole pinch.
const SWIPE_TRAVEL: f64 = 400.0;
const PINCH_TRAVEL: f64 = 0.5;
/// The least progress at which a function bound to a swipe runs when the gesture ends.
const SWIPE_COMMIT: f64 = 48.0 / SWIPE_TRAVEL;
/// The least progress at which a function bound to a pinch runs when the gesture ends.
const PINCH_COMMIT: f64 = 0.12 / PINCH_TRAVEL;

/// What the fingers have done since the gesture began.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FingerSample {
    dx: f64,
    dy: f64,
    scale: f64,
    rotation: f64,
}

/// One gesture from its begin to its end or cancel.
pub(super) struct InputGestureRun {
    context: Obj,
    seat: u64,
    spec_key: String,
    device: String,
    fingers: i64,
    /// The window under the pointer when the gesture began.
    client: Option<Obj>,
    /// The candidates, once the turn that follows the begin has asked the scopes.
    candidates: Option<Vec<InputHook>>,
    latest: FingerSample,
    /// When the last hook ran, on the clock of the session.
    last_time: f64,
    started: Option<InputHook>,
    /// No candidate can take this gesture any more.
    dead: bool,
    done: bool,
    /// A turn is queued to run an update: the latest sample wins.
    scheduled: bool,
}

pub(super) type RunRef = Rc<RefCell<InputGestureRun>>;

/// The gestures in flight on the contexts of a generation.
#[derive(Default)]
pub(super) struct GestureMatcher {
    /// The run of each claim that has not ended: the next updates are its.
    runs: HashMap<i64, RunRef>,
    /// Every run whose hooks may still be called, which includes one that ended and waits for the turn that tells its binding.
    all: Vec<RunRef>,
}

impl GestureMatcher {
    /// Takes the runs a binding has taken, for the generation that ends: no turn will run for them.
    pub(super) fn take_inflight(&mut self) -> Vec<RunRef> {
        self.runs.clear();
        std::mem::take(&mut self.all)
            .into_iter()
            .filter(|run| {
                let held = run.borrow();
                !held.done && held.started.is_some()
            })
            .collect()
    }

    fn forget(&mut self, run: &RunRef) {
        self.runs.retain(|_, held| !Rc::ptr_eq(held, run));
    }

    fn matching(&self, keep: impl Fn(&InputGestureRun) -> bool) -> Vec<RunRef> {
        self.all.iter().filter(|run| keep(&run.borrow())).cloned().collect()
    }
}

impl Matcher for GestureMatcher {
    fn claim_event(&mut self, cx: &Cx<'_>, context: Obj, claim: &ClaimEntry, notice: &ClaimNotice) {
        if notice.kind != abi::CLAIM_EVENT_GESTURE || !matches!(claim.spec, SeatClaim::Gesture { .. }) {
            return;
        }
        self.all.retain(|run| !run.borrow().done);
        let sample = FingerSample {
            dx: notice.dx,
            dy: notice.dy,
            scale: notice.scale,
            rotation: notice.rotation,
        };
        match notice.state.as_str() {
            "begin" => {
                if let Some(previous) = self.runs.remove(&claim.id) {
                    cancel(cx, &previous, false);
                }
                let seat = with(cx.ctx(), |table| {
                    table.contexts.get(&context).map_or(0, |held| held.seat)
                });
                let (x, y) = seat::pointer(seat);
                let client = hit_client(cx, x, y).map(|(client, ..)| client);
                let run = Rc::new(RefCell::new(InputGestureRun {
                    context,
                    seat,
                    spec_key: claim.spec_key.clone(),
                    device: claim.device.clone(),
                    fingers: notice.fingers,
                    client,
                    candidates: None,
                    latest: sample,
                    last_time: crate::session::with(|session| session.now_ms()),
                    started: None,
                    dead: false,
                    done: false,
                    scheduled: false,
                }));
                self.runs.insert(claim.id, Rc::clone(&run));
                self.all.push(Rc::clone(&run));
                cx.later(move |cx| {
                    resolve(cx, &run);
                    decide(cx, &run);
                });
            }
            "update" => {
                let Some(run) = self.runs.get(&claim.id) else {
                    return;
                };
                // The latest wins: an update that waits for its turn is read at its turn, so a second one is not queued behind it.
                let queue = {
                    let mut held = run.borrow_mut();
                    held.latest = sample;
                    !std::mem::replace(&mut held.scheduled, true)
                };
                if queue {
                    let run = Rc::clone(run);
                    cx.later(move |cx| {
                        run.borrow_mut().scheduled = false;
                        step(cx, &run);
                    });
                }
            }
            "end" => {
                if let Some(run) = self.runs.remove(&claim.id) {
                    run.borrow_mut().latest = sample;
                    cx.later(move |cx| finish(cx, &run));
                }
            }
            "cancel" => {
                if let Some(run) = self.runs.remove(&claim.id) {
                    cancel(cx, &run, false);
                }
            }
            _ => {}
        }
    }

    fn retire(&mut self, cx: &Cx<'_>, context: Obj, registration: &BindingRegistration) {
        let running = self.matching(|run| {
            run.context == context
                && run
                    .started
                    .as_ref()
                    .is_some_and(|hook| hook.registration == registration.id)
        });
        for run in running {
            self.forget(&run);
            cancel(cx, &run, false);
        }
    }

    fn cancel_on(&mut self, cx: &Cx<'_>, client: Obj) {
        for run in self.matching(|run| run.client == Some(client)) {
            self.forget(&run);
            cancel(cx, &run, false);
        }
    }

    fn cancel_all(&mut self, cx: &Cx<'_>, context: Obj) {
        for run in self.matching(|run| run.context == context) {
            self.forget(&run);
            cancel(cx, &run, false);
        }
    }
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// The way a swipe has gone by its longer axis, or none while it has moved less than [`SWIPE_DECIDE`].
fn swipe_direction(sample: FingerSample) -> Option<&'static str> {
    if sample.dx.abs().max(sample.dy.abs()) < SWIPE_DECIDE {
        return None;
    }
    Some(if sample.dx.abs() >= sample.dy.abs() {
        if sample.dx > 0.0 { "right" } else { "left" }
    } else if sample.dy > 0.0 {
        "down"
    } else {
        "up"
    })
}

/// The way a pinch has gone, or none while its scale is less than [`PINCH_DECIDE`] from 1.
fn pinch_direction(sample: FingerSample) -> Option<&'static str> {
    if (sample.scale - 1.0).abs() < PINCH_DECIDE {
        return None;
    }
    Some(if sample.scale < 1.0 { "in" } else { "out" })
}

/// The way the gesture has gone, judged as the trigger's kind of gesture is.
fn way_of(hook: &InputHook, sample: FingerSample) -> Option<&'static str> {
    if hook.trigger.gesture == Some("pinch") {
        pinch_direction(sample)
    } else {
        swipe_direction(sample)
    }
}

/// How far a gesture has gone for the trigger, from 0 to 1. A trigger with a direction counts only its own way.
fn progress_of(hook: &InputHook, sample: FingerSample) -> f64 {
    if hook.trigger.gesture == Some("pinch") {
        let spread = match hook.trigger.direction {
            Some("in") => 1.0 - sample.scale,
            Some("out") => sample.scale - 1.0,
            _ => (sample.scale - 1.0).abs(),
        };
        return clamp01(spread / PINCH_TRAVEL);
    }
    clamp01(match hook.trigger.direction {
        Some("left") => -sample.dx / SWIPE_TRAVEL,
        Some("right") => sample.dx / SWIPE_TRAVEL,
        Some("up") => -sample.dy / SWIPE_TRAVEL,
        Some("down") => sample.dy / SWIPE_TRAVEL,
        _ => sample.dx.hypot(sample.dy) / SWIPE_TRAVEL,
    })
}

/// The event a hook of `run` is given at `phase`. It sets the time the run's last hook ran.
fn event_of(cx: &Cx<'_>, run: &RunRef, hook: &InputHook, phase: &str) -> Option<(Val, f64)> {
    let made = {
        let held = run.borrow();
        let sample = held.latest;
        let extra = vec![
            ("phase", cx.enum_val("binding_phase", phase).ok()?),
            ("fingers", Val::Uint(u64::try_from(held.fingers).unwrap_or(0))),
            ("dx", Val::Double(sample.dx)),
            ("dy", Val::Double(sample.dy)),
            ("scale", Val::Double(sample.scale)),
            ("rotation", Val::Double(sample.rotation)),
            ("progress", Val::Double(progress_of(hook, sample))),
        ];
        let basis = Basis {
            context: held.context,
            hook,
            client: held.client,
            position: seat::pointer(held.seat),
            last_time: held.last_time,
        };
        hooks::event(cx, &basis, extra)
    };
    match made {
        Ok((event, now)) => {
            run.borrow_mut().last_time = now;
            Some((event, now))
        }
        Err(fail) => {
            cx.log(
                abi::LOG_WARN,
                &format!("binding \"{}\": the event could not be made: {}", hook.id, fail.message),
            );
            None
        }
    }
}

/// The turn after a begin: the candidates are the registrations whose scope accepts the window under the pointer.
fn resolve(cx: &Cx<'_>, run: &RunRef) {
    let (context, spec_key, device, client, done) = {
        let held = run.borrow();
        (
            held.context,
            held.spec_key.clone(),
            held.device.clone(),
            held.client,
            held.done,
        )
    };
    if done {
        return;
    }
    let found = hooks::candidates(cx, context, &spec_key, &device, client);
    run.borrow_mut().candidates = Some(found);
}

/// Starts the gesture on the best candidate whose direction has come true, once there is one.
fn decide(cx: &Cx<'_>, run: &RunRef) {
    let (context, candidates, latest) = {
        let held = run.borrow();
        if held.started.is_some() || held.dead || held.done {
            return;
        }
        (held.context, held.candidates.clone().unwrap_or_default(), held.latest)
    };
    for hook in &candidates {
        if !hooks::registered(cx, context, hook) {
            continue;
        }
        if let Some(direction) = hook.trigger.direction {
            match way_of(hook, latest) {
                // Not decided yet. A better candidate may still match, so the rest wait.
                None => return,
                Some(way) if way != direction => continue,
                Some(_) => {}
            }
        }
        let taken = match hook.action {
            // A function has no state to keep and no step to answer: it takes the gesture and runs when it ends.
            BindingAction::Closure(_) | BindingAction::Swallow => true,
            _ => match event_of(cx, run, hook, "start") {
                Some((event, _)) => hooks::takes(cx, hook, "on_begin", event),
                None => false,
            },
        };
        if taken {
            let ended = {
                let mut held = run.borrow_mut();
                if held.done {
                    true
                } else {
                    held.started = Some(hook.clone());
                    false
                }
            };
            if ended {
                fire_cancel(cx, run, hook);
            }
            return;
        }
        if run.borrow().done {
            return;
        }
    }
    // Everyone who could take it has declined, or can never match the way it has gone.
    let undecided = candidates
        .iter()
        .any(|hook| hook.trigger.direction.is_some() && way_of(hook, latest).is_none());
    run.borrow_mut().dead = !undecided;
}

/// An update: the gesture is decided if it was not, and the binding that has it is told.
fn step(cx: &Cx<'_>, run: &RunRef) {
    if run.borrow().done {
        return;
    }
    decide(cx, run);
    let started = run.borrow().started.clone();
    if let Some(hook) = started
        && hears_steps(&hook)
        && let Some((event, _)) = event_of(cx, run, &hook, "update")
    {
        hooks::call(cx, &hook, "on_update", event);
    }
}

/// The fingers left the pad.
fn finish(cx: &Cx<'_>, run: &RunRef) {
    if run.borrow().done {
        return;
    }
    decide(cx, run);
    run.borrow_mut().done = true;
    let started = run.borrow().started.clone();
    let Some(hook) = started else {
        return;
    };
    let Some((event, _)) = event_of(cx, run, &hook, "end") else {
        return;
    };
    if matches!(hook.action, BindingAction::Closure(_)) {
        let commit = if hook.trigger.gesture == Some("pinch") {
            PINCH_COMMIT
        } else {
            SWIPE_COMMIT
        };
        if progress_of(&hook, run.borrow().latest) < commit {
            return;
        }
    }
    hooks::call(cx, &hook, "on_end", event);
}

/// Ends the gesture as cancelled. The binding that took it is told: from the next turn, or at once (`now`) when no turn would follow.
pub(super) fn cancel(cx: &Cx<'_>, run: &RunRef, now: bool) {
    let started = {
        let mut held = run.borrow_mut();
        if held.done {
            return;
        }
        held.done = true;
        held.started.clone()
    };
    let Some(hook) = started else {
        return;
    };
    if now {
        fire_cancel(cx, run, &hook);
    } else {
        let run = Rc::clone(run);
        cx.later(move |cx| fire_cancel(cx, &run, &hook));
    }
}

/// Whether the binding is told of each step. A function runs once, at the end; an object hears the begin, the updates and the cancel too.
fn hears_steps(hook: &InputHook) -> bool {
    matches!(hook.action, BindingAction::Gesture(_))
}

fn fire_cancel(cx: &Cx<'_>, run: &RunRef, hook: &InputHook) {
    if hears_steps(hook)
        && let Some((event, _)) = event_of(cx, run, hook, "cancel")
    {
        hooks::call(cx, hook, "on_cancel", event);
    }
}
