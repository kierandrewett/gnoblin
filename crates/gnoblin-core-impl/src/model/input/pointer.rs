//! Pointer chords: a press of a claimed pointer button, and what the pointer does until the button is let go. Past the threshold the press is
//! a drag, which the first candidate that takes it follows to its end; a press that ends before it is a click.
//!
//! The compositor sends a claimed pointer chord as `start`, `update`, `end` and `cancel`, and this matcher keeps one press for each claim
//! from `start` to the end of its hooks. A press asks nothing of the shell while the table or the matchers are borrowed: what it decides at a
//! message it decides from a turn queued for it (`Cx::later`), in the order the messages came.
//!
//! - **A press** records where it was, the window under it and, from the turn that follows, the candidates: the registrations of the claim
//!   that are enabled and whose scope accepts the window, in the order of the tiers, the priorities and the age.
//! - **A drag** starts when the pointer is `threshold` away from the press. The candidates are asked in order; one that is not due yet makes
//!   the ones after it wait, one that declines leaves the drag to the next, and when every candidate that was due has declined the press is
//!   no click either. From then on every sample is an update; the release ends the drag.
//! - **A click** is a press that ended before the pointer went far enough. The best candidate for a click that does not decline has it.
//! - **A cancel** reaches the binding that took the drag when the compositor ends the drag, the window under it closes, the binding is
//!   removed, a device moves to another context, the context ends, or the generation does. A binding that throws in an update has lost the
//!   drag and is told once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{Obj, Val};

use super::binding::DEFAULT_THRESHOLD;
use super::context::hit_client;
use super::hooks::{self, Basis, HookAnswer, InputHook};
use super::{BindingRegistration, ClaimEntry, Matcher, with};
use crate::events::ClaimNotice;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerPressPhase {
    /// The button is down and no drag has started.
    Pressed,
    /// A binding took the drag.
    Dragging,
    /// Its hooks were told the last they will be.
    Done,
}

/// One pointer chord from its start to its end or cancel.
pub(super) struct PointerPress {
    context: Obj,
    seat: u64,
    spec_key: String,
    device: String,
    origin: (f64, f64),
    button: i64,
    /// The window under the press.
    client: Option<Obj>,
    /// The candidates, once the turn that follows the press has asked the scopes.
    candidates: Option<Vec<InputHook>>,
    /// Where the compositor last said the pointer is.
    at: (f64, f64),
    /// Where the last hook that ran saw it.
    last: (f64, f64),
    /// When the last hook ran, on the clock of the session.
    last_time: f64,
    phase: PointerPressPhase,
    active: Option<InputHook>,
    /// A turn is queued to sample the pointer: the latest position wins.
    scheduled: bool,
    /// The pointer went past the drag threshold and no binding took the drag: this press is not a click.
    declined: bool,
}

pub(super) type PressRef = Rc<RefCell<PointerPress>>;

/// The pointer presses in flight on the contexts of a generation.
#[derive(Default)]
pub(super) struct PointerMatcher {
    /// The press of each claim that has not ended: the next samples of the pointer are its.
    presses: HashMap<i64, PressRef>,
    /// Every press whose hooks may still be called, which includes one that ended and waits for the turn that tells its binding.
    all: Vec<PressRef>,
}

impl PointerMatcher {
    /// Takes the presses whose drag is under way, for the generation that ends: no turn will run for them.
    pub(super) fn take_inflight(&mut self) -> Vec<PressRef> {
        self.presses.clear();
        std::mem::take(&mut self.all)
            .into_iter()
            .filter(|press| press.borrow().phase == PointerPressPhase::Dragging)
            .collect()
    }

    fn forget(&mut self, press: &PressRef) {
        self.presses.retain(|_, held| !Rc::ptr_eq(held, press));
    }

    /// The presses `keep` accepts.
    fn matching(&self, keep: impl Fn(&PointerPress) -> bool) -> Vec<PressRef> {
        self.all.iter().filter(|press| keep(&press.borrow())).cloned().collect()
    }
}

impl Matcher for PointerMatcher {
    fn claim_event(&mut self, cx: &Cx<'_>, context: Obj, claim: &ClaimEntry, notice: &ClaimNotice) {
        if notice.kind != abi::CLAIM_EVENT_POINTER_CHORD || !claim.pointer {
            return;
        }
        self.all.retain(|press| press.borrow().phase != PointerPressPhase::Done);
        let at = (notice.x, notice.y);
        match notice.state.as_str() {
            "start" => {
                if let Some(previous) = self.presses.remove(&claim.id) {
                    // A second start before the end: the first one's end was lost. It ends as cancelled.
                    cancel(cx, &previous, false);
                }
                let seat = with(cx.ctx(), |table| {
                    table.contexts.get(&context).map_or(0, |held| held.seat)
                });
                let client = hit_client(cx, at.0, at.1).map(|(client, ..)| client);
                let press = Rc::new(RefCell::new(PointerPress {
                    context,
                    seat,
                    spec_key: claim.spec_key.clone(),
                    device: claim.device.clone(),
                    origin: at,
                    button: notice.button,
                    client,
                    candidates: None,
                    at,
                    last: at,
                    last_time: crate::session::with(|session| session.now_ms()),
                    phase: PointerPressPhase::Pressed,
                    active: None,
                    scheduled: false,
                    declined: false,
                }));
                self.presses.insert(claim.id, Rc::clone(&press));
                self.all.push(Rc::clone(&press));
                cx.later(move |cx| resolve(cx, &press));
            }
            "update" => {
                if let Some(press) = self.presses.get(&claim.id) {
                    press.borrow_mut().at = at;
                    sample(cx, press);
                }
            }
            "end" => {
                if let Some(press) = self.presses.remove(&claim.id) {
                    press.borrow_mut().at = at;
                    cx.later(move |cx| release(cx, &press));
                }
            }
            "cancel" => {
                if let Some(press) = self.presses.remove(&claim.id) {
                    press.borrow_mut().at = at;
                    cancel(cx, &press, false);
                }
            }
            _ => {}
        }
    }

    fn retire(&mut self, cx: &Cx<'_>, context: Obj, registration: &BindingRegistration) {
        let running = self.matching(|press| {
            press.context == context
                && press
                    .active
                    .as_ref()
                    .is_some_and(|hook| hook.registration == registration.id)
        });
        for press in running {
            self.forget(&press);
            cancel(cx, &press, false);
        }
    }

    fn cancel_on(&mut self, cx: &Cx<'_>, client: Obj) {
        for press in self.matching(|press| press.client == Some(client)) {
            self.forget(&press);
            cancel(cx, &press, false);
        }
    }

    fn cancel_all(&mut self, cx: &Cx<'_>, context: Obj) {
        for press in self.matching(|press| press.context == context) {
            self.forget(&press);
            cancel(cx, &press, false);
        }
    }
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The event a hook of `press` is given for the pointer at `at`: `phase` is none for a click. Answers it and the time it was made.
fn event_of(
    cx: &Cx<'_>,
    press: &PressRef,
    hook: &InputHook,
    at: (f64, f64),
    phase: Option<&str>,
) -> Option<(Val, f64)> {
    let held = press.borrow();
    let (constraint, edge_tiling) = hooks::drag_policy(held.seat);
    let mut extra = vec![
        ("button", Val::Uint(u64::try_from(held.button).unwrap_or(0))),
        ("dx", Val::Double(at.0 - held.origin.0)),
        ("dy", Val::Double(at.1 - held.origin.1)),
        ("drag_constraint", Val::Text(constraint)),
        ("edge_tiling", Val::Bool(edge_tiling)),
    ];
    if let Some(phase) = phase {
        extra.push(("phase", cx.enum_val("binding_phase", phase).ok()?));
    }
    let basis = Basis {
        context: held.context,
        hook,
        client: held.client,
        position: at,
        last_time: held.last_time,
    };
    match hooks::event(cx, &basis, extra) {
        Ok(made) => Some(made),
        Err(fail) => {
            cx.log(
                abi::LOG_WARN,
                &format!("binding \"{}\": the event could not be made: {}", hook.id, fail.message),
            );
            None
        }
    }
}

/// The turn after a press: the candidates are the registrations whose scope accepts the window under it.
fn resolve(cx: &Cx<'_>, press: &PressRef) {
    let (context, spec_key, device, client, phase) = {
        let held = press.borrow();
        (
            held.context,
            held.spec_key.clone(),
            held.device.clone(),
            held.client,
            held.phase,
        )
    };
    if phase == PointerPressPhase::Done {
        return;
    }
    let found = hooks::candidates(cx, context, &spec_key, &device, client);
    press.borrow_mut().candidates = Some(found);
}

/// Queues a sample of the drag unless the pointer has not moved or one is queued already. The sample reads the latest position.
fn sample(cx: &Cx<'_>, press: &PressRef) {
    {
        let mut held = press.borrow_mut();
        if held.phase == PointerPressPhase::Done || held.scheduled || held.at == held.last {
            return;
        }
        held.scheduled = true;
    }
    let press = Rc::clone(press);
    cx.later(move |cx| {
        press.borrow_mut().scheduled = false;
        advance(cx, &press);
    });
}

/// Takes the drag as far as the pointer's place allows: starts it past the threshold, else updates it.
fn advance(cx: &Cx<'_>, press: &PressRef) {
    let (phase, declined) = {
        let held = press.borrow();
        (held.phase, held.declined)
    };
    if phase == PointerPressPhase::Done {
        return;
    }
    if phase == PointerPressPhase::Pressed && !declined {
        try_start(cx, press);
    }
    let (phase, active, at, last) = {
        let held = press.borrow();
        (held.phase, held.active.clone(), held.at, held.last)
    };
    let (PointerPressPhase::Dragging, Some(hook)) = (phase, active) else {
        return;
    };
    if at == last {
        return;
    }
    let Some((event, now)) = event_of(cx, press, &hook, at, Some("update")) else {
        return;
    };
    {
        let mut held = press.borrow_mut();
        held.last = at;
        held.last_time = now;
    }
    if matches!(hooks::call(cx, &hook, "on_update", event), HookAnswer::Threw) {
        // A binding that throws has lost the drag: it is told once, and no other drag is touched.
        cancel(cx, press, false);
    }
}

/// Offers the drag to the candidates in order, once the pointer has gone far enough for the first of them.
fn try_start(cx: &Cx<'_>, press: &PressRef) {
    let (context, origin, at, candidates) = {
        let held = press.borrow();
        (
            held.context,
            held.origin,
            held.at,
            held.candidates.clone().unwrap_or_default(),
        )
    };
    let moved = distance(origin, at);
    for hook in candidates {
        if !hook.is_drag() || !hooks::registered(cx, context, &hook) {
            continue;
        }
        if moved < hook.threshold {
            // A better binding is still waiting for the pointer to go far enough: the others wait too.
            return;
        }
        // The start event says where the press was, not where the pointer has got to, so a binding that measures from `position` measures
        // from the press. The update that follows has the pointer.
        let Some((event, _)) = event_of(cx, press, &hook, origin, Some("start")) else {
            continue;
        };
        if hooks::takes(cx, &hook, "on_start", event) {
            let ended = {
                let mut held = press.borrow_mut();
                if held.phase == PointerPressPhase::Pressed {
                    held.phase = PointerPressPhase::Dragging;
                    held.active = Some(hook.clone());
                    held.last = origin;
                    false
                } else {
                    true
                }
            };
            if ended {
                // The drag ended while its binding was being asked: the binding that took it is told.
                fire_cancel(cx, press, &hook);
            }
            return;
        }
        if press.borrow().phase == PointerPressPhase::Done {
            return;
        }
    }
    // Every candidate that was due has declined, so this is not a drag. The press stays one: its end is still swallowed (the compositor
    // held the press back from the client), but a pointer that has gone this far is not a click either.
    if moved >= DEFAULT_THRESHOLD {
        press.borrow_mut().declined = true;
    }
}

/// The turn after the button was let go: a drag ends, a press that never became one is a click.
fn release(cx: &Cx<'_>, press: &PressRef) {
    if press.borrow().phase == PointerPressPhase::Done {
        return;
    }
    // The end may come in the turn of the update that starts the drag: start it now.
    advance(cx, press);
    let (phase, active, at, declined) = {
        let held = press.borrow();
        (held.phase, held.active.clone(), held.at, held.declined)
    };
    if let (PointerPressPhase::Dragging, Some(hook)) = (phase, active.as_ref()) {
        press.borrow_mut().phase = PointerPressPhase::Done;
        if let Some((event, _)) = event_of(cx, press, hook, at, Some("end")) {
            hooks::call(cx, hook, "on_end", event);
        }
        return;
    }
    let click = phase == PointerPressPhase::Pressed && !declined;
    press.borrow_mut().phase = PointerPressPhase::Done;
    if click {
        click_of(cx, press);
    }
}

/// An end with no drag: the best candidate for a click that does not decline has it.
fn click_of(cx: &Cx<'_>, press: &PressRef) {
    let (context, candidates, at) = {
        let held = press.borrow();
        (held.context, held.candidates.clone().unwrap_or_default(), held.at)
    };
    for hook in candidates {
        if !hook.is_click() || !hooks::registered(cx, context, &hook) {
            continue;
        }
        if matches!(hook.action, super::BindingAction::Swallow) {
            return;
        }
        let Some((event, _)) = event_of(cx, press, &hook, at, None) else {
            continue;
        };
        match hooks::call(cx, &hook, "on_press", event) {
            HookAnswer::Said(Val::Bool(false)) | HookAnswer::Absent => {}
            HookAnswer::Said(_) | HookAnswer::Threw => return,
        }
    }
}

/// Ends the press as cancelled. The binding that took its drag is told: from the next turn, or at once (`now`) when no turn would follow.
pub(super) fn cancel(cx: &Cx<'_>, press: &PressRef, now: bool) {
    let (was, active) = {
        let mut held = press.borrow_mut();
        let was = held.phase;
        held.phase = PointerPressPhase::Done;
        (was, held.active.clone())
    };
    let (PointerPressPhase::Dragging, Some(hook)) = (was, active) else {
        return;
    };
    if now {
        fire_cancel(cx, press, &hook);
    } else {
        let press = Rc::clone(press);
        cx.later(move |cx| fire_cancel(cx, &press, &hook));
    }
}

fn fire_cancel(cx: &Cx<'_>, press: &PressRef, hook: &InputHook) {
    let at = press.borrow().at;
    if let Some((event, _)) = event_of(cx, press, hook, at, Some("cancel")) {
        hooks::call(cx, hook, "on_cancel", event);
    }
}
