//! Pointer and gesture bindings: the press, threshold, start-or-decline and cancel rules, run once for every language.
//!
//! A binding names a trigger (a pointer chord such as `Super+Button1`, or a touchpad gesture such as `Swipe3Left`) and takes the
//! drags or gestures that trigger starts. The library claims each trigger from the core once, however many bindings share it, and
//! routes the claim's steps as `data/js/gnoblin-core/input/{pointer,gesture}.ts` does (the reference):
//!
//! - **Candidates.** The bindings of a trigger are asked in the order of their priority (higher first) and then of their
//!   registration. Two of one trigger and one priority are refused: the matcher could not choose.
//! - **A drag.** A press records where it was and which window it landed on. When the pointer has travelled past a binding's
//!   threshold the binding is asked to start (its start event says where the press was); one that declines leaves the drag to the
//!   next candidate, and a binding whose threshold is not reached yet makes the ones after it wait. When every candidate that was due
//!   has declined, the press stays one and is not a click. Every later motion is an update; the release ends the drag, and the
//!   compositor cancelling it, the window it pressed closing, or the binding going away cancels it.
//! - **A gesture.** A binding with a direction waits until the fingers have gone far enough to have one and is asked only if the
//!   gesture went its way; `progress` counts the trigger's own way.
//!
//! The bindings of a shell (a callback and a `user`) and the bindings the library runs itself (`drags.rs`: move, resize, tile swap,
//! split resize) are the same kind of registration, so they share triggers and priorities. A callback runs from the core's dispatch,
//! never inside a table call, and outside every borrow of the library's state.
#![allow(unsafe_code)]

mod drags;
mod gesture;
mod step_api;
pub(crate) mod steps;

pub(crate) use self::drags::RuleDrag;
pub(crate) use self::gesture::way_past;
pub(crate) use self::step_api::{drag_clamp, drag_resize, drag_step, drag_tile, drag_zone, gesture_way};

use core::ffi::c_void;
use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core::host::{DragPhase, GestureEvent, GestureKind, PointerDrag, Rect, Subscription};
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinDropFn};
use gnoblin_std_abi::layouts::binding_kind;
use gnoblin_std_abi::{
    GnoblinStdBindingSpec, GnoblinStdDragEvent, GnoblinStdDragFn, GnoblinStdGestureEvent, GnoblinStdGestureFn,
    GnoblinStdOption, GnoblinStdRect, GnoblinStdReg, STD_CONSTRAINT_MINIMUM_STRIP, STD_CONSTRAINT_WORK_AREA,
    STD_DRAG_CANCEL, STD_DRAG_EDGE_TILING, STD_DRAG_END, STD_DRAG_HAS_LOCAL, STD_DRAG_HAS_WORK_AREA, STD_DRAG_START,
    STD_DRAG_UPDATE, STD_GESTURE_BEGIN, STD_GESTURE_CANCEL, STD_GESTURE_END, STD_GESTURE_HAS_POSITION,
    STD_GESTURE_PINCH, STD_GESTURE_SWIPE, STD_GESTURE_UPDATE,
};

use crate::builtin::{OptionSet, Pt, Val, read_set, read_text};
use crate::error::{Fail, Res};
use crate::nodes;
use crate::state::{Id, NodeId, State, Std, next_id};
use crate::table::{call_reg, read_ext};
use crate::turn::schedule;
use gnoblin_object::ErrorCode;

use self::gesture::{GestureTrigger, progress_of, way_of};
use self::steps::DragRule;

/// How far a pointer travels before a press becomes a drag, in logical pixels.
const DRAG_THRESHOLD: f64 = 4.0;

/// Runs `f` over the library's state, then posts a turn and tells the listeners what it queued.
pub(crate) fn with_state<R>(std: &Rc<Std>, f: impl FnOnce(&mut State) -> R) -> R {
    let result = f(&mut std.st.borrow_mut());
    schedule(std);
    std.drain();
    result
}

/// What a drag binding is told at each step.
#[derive(Clone, Debug)]
pub(crate) struct DragCtx {
    /// The window the press landed on.
    pub(crate) client: Option<NodeId>,
    /// Where the pointer is. On the start, where the press was.
    pub(crate) position: Pt,
    /// Where the press was.
    pub(crate) origin: Pt,
    /// The press in the window's own coordinates.
    pub(crate) local: Option<Pt>,
    pub(crate) button: i64,
    /// The rectangle windows are kept inside: the output the press was on.
    pub(crate) work_area: Option<Rect>,
    /// `STD_CONSTRAINT_*`.
    pub(crate) constraint: u32,
    /// The seat's `edge_tiling`.
    pub(crate) edge_tiling: bool,
}

/// A drag binding. Every function runs outside any borrow of the library's state.
pub(crate) trait DragHandler {
    /// The press went past the threshold: take the drag (`true`) or decline it.
    fn start(&mut self, std: &Rc<Std>, event: &DragCtx) -> bool;
    fn update(&mut self, std: &Rc<Std>, event: &DragCtx);
    fn end(&mut self, _std: &Rc<Std>, _event: &DragCtx) {}
    /// The drag ended some other way: undo what it did.
    fn cancel(&mut self, _std: &Rc<Std>, _event: &DragCtx) {}
}

/// A drag binding of the shell's: its callback and the closure behind `user`, freed exactly once.
struct ForeignDrag {
    cb: GnoblinStdDragFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
}

impl ForeignDrag {
    fn call(&self, phase: u32, event: &DragCtx) -> i32 {
        let mut flags = 0;
        if event.local.is_some() {
            flags |= STD_DRAG_HAS_LOCAL;
        }
        if event.work_area.is_some() {
            flags |= STD_DRAG_HAS_WORK_AREA;
        }
        if event.edge_tiling {
            flags |= STD_DRAG_EDGE_TILING;
        }
        let area = event.work_area.unwrap_or_default();
        let local = event.local.unwrap_or(Pt { x: 0.0, y: 0.0 });
        let raw = GnoblinStdDragEvent {
            size: size_of::<GnoblinStdDragEvent>() as u32,
            phase,
            flags,
            constraint: event.constraint,
            button: event.button,
            client: event.client.unwrap_or(0),
            x: event.position.x,
            y: event.position.y,
            origin_x: event.origin.x,
            origin_y: event.origin.y,
            local_x: local.x,
            local_y: local.y,
            work_area: GnoblinStdRect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: area.height,
            },
        };
        // SAFETY: `cb` and `user` are what the shell passed to `binding_drag`; the library adopted `user` and frees it only in `drop`,
        // so it is live for the call, which runs on the shell thread. `raw` is a live local.
        catch_unwind(AssertUnwindSafe(|| unsafe { (self.cb)(self.user, &raw) })).unwrap_or(abi::E_PANIC)
    }
}

impl DragHandler for ForeignDrag {
    fn start(&mut self, _std: &Rc<Std>, event: &DragCtx) -> bool {
        self.call(STD_DRAG_START, event) == 1
    }

    fn update(&mut self, _std: &Rc<Std>, event: &DragCtx) {
        self.call(STD_DRAG_UPDATE, event);
    }

    fn end(&mut self, _std: &Rc<Std>, event: &DragCtx) {
        self.call(STD_DRAG_END, event);
    }

    fn cancel(&mut self, _std: &Rc<Std>, event: &DragCtx) {
        self.call(STD_DRAG_CANCEL, event);
    }
}

impl Drop for ForeignDrag {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: the shell gave `user` and `drop_user` together; this is the one call, made when the binding is gone.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop_user(self.user) }));
        }
    }
}

/// A gesture binding of the shell's.
struct ForeignGesture {
    cb: GnoblinStdGestureFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
}

impl Drop for ForeignGesture {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: as for `ForeignDrag`.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop_user(self.user) }));
        }
    }
}

/// One registered drag binding.
struct DragReg {
    id: Id,
    /// The pointer chord, as the claim names it.
    key: String,
    priority: f64,
    order: u64,
    threshold: f64,
    handler: RefCell<Box<dyn DragHandler>>,
}

/// One registered gesture binding.
struct GestureReg {
    id: Id,
    trigger: GestureTrigger,
    priority: f64,
    order: u64,
    handler: ForeignGesture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PressPhase {
    /// Down, and not yet moved far enough.
    Pressed,
    /// A binding took it.
    Dragging,
}

/// One press of a claimed chord and what became of it.
struct Press {
    origin: Pt,
    button: i64,
    client: Option<NodeId>,
    local: Option<Pt>,
    work_area: Option<Rect>,
    candidates: Vec<Rc<DragReg>>,
    /// Where the compositor last said the pointer is.
    at: Pt,
    /// Where the last hook that ran saw it.
    last: Pt,
    phase: PressPhase,
    active: Option<Rc<DragReg>>,
    /// The pointer went past the drag threshold and no binding took the drag: this press is not a click.
    declined: bool,
}

/// The newest sample of a gesture.
#[derive(Clone, Copy, Debug)]
struct GestureSample {
    dx: f64,
    dy: f64,
    scale: f64,
    rotation: f64,
}

/// One touchpad gesture from its begin to its end or cancel.
struct GestureRun {
    client: Option<NodeId>,
    candidates: Vec<Rc<GestureReg>>,
    latest: GestureSample,
    started: Option<Rc<GestureReg>>,
    /// No candidate can start for this gesture any more.
    dead: bool,
    done: bool,
}

type GestureKey = (u8, u8);

/// The bindings of one generation, their claims, and the drags and gestures in flight.
#[derive(Default)]
pub(crate) struct BindingRouter {
    order: u64,
    drags: Vec<Rc<DragReg>>,
    gestures: Vec<Rc<GestureReg>>,
    pointer_claims: HashMap<String, Subscription>,
    gesture_claims: HashMap<GestureKey, Subscription>,
    presses: HashMap<String, Press>,
    runs: HashMap<GestureKey, GestureRun>,
}

fn claim_key(kind: GestureKind, fingers: u8) -> GestureKey {
    (u8::from(matches!(kind, GestureKind::Pinch)), fingers)
}

impl BindingRouter {
    /// Whether a binding with this registration id exists.
    pub(crate) fn has(&self, id: Id) -> bool {
        self.drags.iter().any(|reg| reg.id == id) || self.gestures.iter().any(|reg| reg.id == id)
    }
}

// ---- what the seat and the tree say -----------------------------------------------------------------------------------------

/// The topmost window whose rectangle holds `at`, not minimised: what a press lands on.
fn window_at(std: &Std, at: Pt) -> Option<(NodeId, Rect)> {
    let g = std.g();
    let windows = g.windows().ok()?;
    windows.into_iter().rev().find_map(|window| {
        if window.is_minimised().unwrap_or(false) {
            return None;
        }
        let rect = window.geometry().ok()?;
        let inside = at.x >= rect.x && at.x < rect.x + rect.width && at.y >= rect.y && at.y < rect.y + rect.height;
        inside.then(|| (window.id().0, rect))
    })
}

/// The output rectangle that holds `at`, else the first output's.
fn work_area_at(std: &Std, at: Pt) -> Option<Rect> {
    let st = std.st.borrow();
    let rects: Vec<Rect> = st
        .outputs
        .iter()
        .filter_map(|output| crate::desks::output_rect(&st.g, *output))
        .collect();
    rects
        .iter()
        .copied()
        .find(|r| at.x >= r.x && at.x < r.x + r.width && at.y >= r.y && at.y < r.y + r.height)
        .or_else(|| rects.first().copied())
}

/// The seat's `drag_constraint`.
fn drag_constraint(std: &Std) -> u32 {
    let g = std.g();
    match nodes::seat(&g).and_then(|seat| seat.get_text("drag_constraint").ok()) {
        Some(rule) if rule == "minimum_strip" => STD_CONSTRAINT_MINIMUM_STRIP,
        _ => STD_CONSTRAINT_WORK_AREA,
    }
}

/// The seat's `edge_tiling`.
fn edge_tiling(std: &Std) -> bool {
    let g = std.g();
    nodes::seat(&g)
        .and_then(|seat| seat.get_bool("edge_tiling").ok())
        .unwrap_or(false)
}

fn ctx_of(std: &Std, press: &Press, position: Pt) -> DragCtx {
    DragCtx {
        client: press.client,
        position,
        origin: press.origin,
        local: press.local,
        button: press.button,
        work_area: press.work_area,
        constraint: drag_constraint(std),
        edge_tiling: edge_tiling(std),
    }
}

// ---- the drag matcher -------------------------------------------------------------------------------------------------------

fn candidates_of(router: &BindingRouter, key: &str) -> Vec<Rc<DragReg>> {
    let mut candidates: Vec<Rc<DragReg>> = router.drags.iter().filter(|reg| reg.key == key).cloned().collect();
    candidates.sort_by(|a, b| b.priority.total_cmp(&a.priority).then(a.order.cmp(&b.order)));
    candidates
}

/// Calls a binding's function unless the binding is running already (a callback that asked for its own cancel).
fn with_handler(reg: &DragReg, f: impl FnOnce(&mut dyn DragHandler)) {
    if let Ok(mut handler) = reg.handler.try_borrow_mut() {
        f(&mut **handler);
    }
}

/// Takes the drag as far as the pointer's position allows: starts it past the threshold, else updates it.
fn advance(std: &Rc<Std>, press: &mut Press) {
    if press.phase == PressPhase::Pressed && !press.declined {
        try_start(std, press);
    }
    if press.phase == PressPhase::Dragging
        && let Some(reg) = press.active.clone()
        && press.at != press.last
    {
        let event = ctx_of(std, press, press.at);
        press.last = press.at;
        with_handler(&reg, |handler| handler.update(std, &event));
    }
}

fn try_start(std: &Rc<Std>, press: &mut Press) {
    let moved = (press.origin.x - press.at.x).hypot(press.origin.y - press.at.y);
    for reg in press.candidates.clone() {
        if moved < reg.threshold {
            // A better binding is still waiting for the pointer to go far enough: the others wait too.
            return;
        }
        // The start event says where the press was, not where the pointer has got to, so a binding that measures from the position
        // measures from the press. The update that follows has the current one.
        let event = ctx_of(std, press, press.origin);
        let mut took = false;
        with_handler(&reg, |handler| took = handler.start(std, &event));
        if took {
            press.phase = PressPhase::Dragging;
            press.active = Some(reg);
            press.last = press.origin;
            return;
        }
    }
    // Every candidate that was due has declined, so this is not a drag. The press stays one: its end is still swallowed (the
    // compositor withheld the press from the client), but a pointer that has gone this far is not a click either.
    if moved >= DRAG_THRESHOLD {
        press.declined = true;
    }
}

/// Finishes the drag of the press on `key` as cancelled.
fn cancel_press(std: &Rc<Std>, key: &str) {
    let press = std.router.borrow_mut().presses.remove(key);
    if let Some(press) = press {
        finish_cancelled(std, &press);
    }
}

fn finish_cancelled(std: &Rc<Std>, press: &Press) {
    if press.phase == PressPhase::Dragging
        && let Some(reg) = &press.active
    {
        let event = ctx_of(std, press, press.at);
        with_handler(reg, |handler| handler.cancel(std, &event));
    }
}

fn pointer_event(std: &Rc<Std>, key: &str, drag: &PointerDrag) {
    let here = Pt { x: drag.x, y: drag.y };
    match drag.phase {
        DragPhase::Start => {
            // A start before the end of the last press: the first one's end was lost, so it is finished as cancelled.
            cancel_press(std, key);
            let hit = window_at(std, here);
            let candidates = candidates_of(&std.router.borrow(), key);
            let press = Press {
                origin: here,
                button: drag.button,
                local: hit.as_ref().map(|(_, rect)| Pt {
                    x: here.x - rect.x,
                    y: here.y - rect.y,
                }),
                client: hit.map(|(client, _)| client),
                work_area: work_area_at(std, here),
                candidates,
                at: here,
                last: here,
                phase: PressPhase::Pressed,
                active: None,
                declined: false,
            };
            std.router.borrow_mut().presses.insert(key.to_owned(), press);
        }
        DragPhase::Update => {
            let Some(mut press) = std.router.borrow_mut().presses.remove(key) else {
                return;
            };
            press.at = here;
            advance(std, &mut press);
            std.router.borrow_mut().presses.insert(key.to_owned(), press);
        }
        DragPhase::End => {
            let Some(mut press) = std.router.borrow_mut().presses.remove(key) else {
                return;
            };
            press.at = here;
            // The end may arrive in the frame of the update that starts the drag: start it now.
            advance(std, &mut press);
            if press.phase == PressPhase::Dragging
                && let Some(reg) = press.active.clone()
            {
                let event = ctx_of(std, &press, press.at);
                with_handler(&reg, |handler| handler.end(std, &event));
            }
        }
        DragPhase::Cancel => {
            let Some(mut press) = std.router.borrow_mut().presses.remove(key) else {
                return;
            };
            press.at = here;
            finish_cancelled(std, &press);
        }
    }
    std.drain();
}

// ---- the gesture matcher ----------------------------------------------------------------------------------------------------

fn gesture_candidates(router: &BindingRouter, key: GestureKey) -> Vec<Rc<GestureReg>> {
    let mut candidates: Vec<Rc<GestureReg>> = router
        .gestures
        .iter()
        .filter(|reg| claim_key(reg.trigger.kind, reg.trigger.fingers) == key)
        .cloned()
        .collect();
    candidates.sort_by(|a, b| b.priority.total_cmp(&a.priority).then(a.order.cmp(&b.order)));
    candidates
}

fn gesture_raw(std: &Std, run: &GestureRun, reg: &GestureReg, phase: u32) -> GnoblinStdGestureEvent {
    let sample = run.latest;
    let trigger = reg.trigger;
    let position = nodes::pointer(&std.g());
    GnoblinStdGestureEvent {
        size: size_of::<GnoblinStdGestureEvent>() as u32,
        phase,
        flags: if position.is_some() {
            STD_GESTURE_HAS_POSITION
        } else {
            0
        },
        kind: if matches!(trigger.kind, GestureKind::Pinch) {
            STD_GESTURE_PINCH
        } else {
            STD_GESTURE_SWIPE
        },
        fingers: u32::from(trigger.fingers),
        direction: trigger.direction,
        way: way_of(trigger.kind, sample.dx, sample.dy, sample.scale),
        reserved: 0,
        client: run.client.unwrap_or(0),
        dx: sample.dx,
        dy: sample.dy,
        scale: sample.scale,
        rotation: sample.rotation,
        progress: progress_of(&trigger, sample.dx, sample.dy, sample.scale),
        x: position.map_or(0.0, |at| at.0),
        y: position.map_or(0.0, |at| at.1),
    }
}

fn gesture_call(std: &Std, run: &GestureRun, reg: &GestureReg, phase: u32) -> i32 {
    let raw = gesture_raw(std, run, reg, phase);
    let handler = &reg.handler;
    // SAFETY: `cb` and `user` are what the shell passed to `binding_gesture`; the library adopted `user` and frees it only in
    // `drop`, so it is live for the call, which runs on the shell thread. `raw` is a live local.
    catch_unwind(AssertUnwindSafe(|| unsafe { (handler.cb)(handler.user, &raw) })).unwrap_or(abi::E_PANIC)
}

/// Starts the gesture on the best candidate whose direction has come true, once there is one.
fn decide(std: &Std, run: &mut GestureRun) {
    if run.started.is_some() || run.dead || run.done {
        return;
    }
    let sample = run.latest;
    for reg in run.candidates.clone() {
        let trigger = reg.trigger;
        if trigger.direction != gnoblin_std_abi::STD_WAY_NONE {
            let way = way_of(trigger.kind, sample.dx, sample.dy, sample.scale);
            if way == gnoblin_std_abi::STD_WAY_NONE {
                // Not decided yet. A better candidate may still match, so the rest wait.
                return;
            }
            if way != trigger.direction {
                continue;
            }
        }
        if gesture_call(std, run, &reg, STD_GESTURE_BEGIN) == 1 {
            run.started = Some(reg);
            return;
        }
    }
    // Everyone who could take it has declined, or can never match the way it has gone.
    let undecided = run.candidates.iter().any(|reg| {
        reg.trigger.direction != gnoblin_std_abi::STD_WAY_NONE
            && way_of(reg.trigger.kind, sample.dx, sample.dy, sample.scale) == gnoblin_std_abi::STD_WAY_NONE
    });
    run.dead = !undecided;
}

fn finish_gesture_cancelled(std: &Std, mut run: GestureRun) {
    if run.done {
        return;
    }
    run.done = true;
    if let Some(reg) = run.started.clone() {
        gesture_call(std, &run, &reg, STD_GESTURE_CANCEL);
    }
}

fn cancel_gesture(std: &Rc<Std>, key: GestureKey) {
    let run = std.router.borrow_mut().runs.remove(&key);
    if let Some(run) = run {
        finish_gesture_cancelled(std, run);
    }
}

fn gesture_event(std: &Rc<Std>, key: GestureKey, raw: &GestureEvent) {
    let sample = GestureSample {
        dx: raw.dx,
        dy: raw.dy,
        scale: raw.scale,
        rotation: raw.rotation,
    };
    match raw.phase {
        DragPhase::Start => {
            cancel_gesture(std, key);
            let client = nodes::pointer(&std.g())
                .and_then(|(x, y)| window_at(std, Pt { x, y }))
                .map(|(client, _)| client);
            let candidates = gesture_candidates(&std.router.borrow(), key);
            let mut run = GestureRun {
                client,
                candidates,
                latest: sample,
                started: None,
                dead: false,
                done: false,
            };
            decide(std, &mut run);
            std.router.borrow_mut().runs.insert(key, run);
        }
        DragPhase::Update => {
            let Some(mut run) = std.router.borrow_mut().runs.remove(&key) else {
                return;
            };
            run.latest = sample;
            decide(std, &mut run);
            if let Some(reg) = run.started.clone() {
                gesture_call(std, &run, &reg, STD_GESTURE_UPDATE);
            }
            std.router.borrow_mut().runs.insert(key, run);
        }
        DragPhase::End => {
            let Some(mut run) = std.router.borrow_mut().runs.remove(&key) else {
                return;
            };
            run.latest = sample;
            decide(std, &mut run);
            run.done = true;
            if let Some(reg) = run.started.clone() {
                gesture_call(std, &run, &reg, STD_GESTURE_END);
            }
        }
        DragPhase::Cancel => cancel_gesture(std, key),
    }
    std.drain();
}

// ---- windows that close, bindings that go -----------------------------------------------------------------------------------

/// A window closed: the drags and gestures that pressed it are cancelled once.
pub(crate) fn client_closed(std: &Rc<Std>, client: NodeId) {
    let (presses, runs): (Vec<String>, Vec<GestureKey>) = {
        let router = std.router.borrow();
        (
            router
                .presses
                .iter()
                .filter(|(_, press)| press.client == Some(client))
                .map(|(key, _)| key.clone())
                .collect(),
            router
                .runs
                .iter()
                .filter(|(_, run)| run.client == Some(client))
                .map(|(key, _)| *key)
                .collect(),
        )
    };
    for key in presses {
        cancel_press(std, &key);
    }
    for key in runs {
        cancel_gesture(std, key);
    }
    std.drain();
}

/// Ends the binding `id`: the drag or gesture it runs is cancelled, its claim goes when it was the last of its trigger, and its
/// `drop_user` runs. `false` when there is no such binding.
pub(crate) fn remove(std: &Rc<Std>, id: Id) -> bool {
    let drag = {
        let router = std.router.borrow();
        router.drags.iter().find(|reg| reg.id == id).cloned()
    };
    if let Some(reg) = drag {
        let presses: Vec<String> = {
            let router = std.router.borrow();
            router
                .presses
                .iter()
                .filter(|(_, press)| press.active.as_ref().is_some_and(|active| active.id == id))
                .map(|(key, _)| key.clone())
                .collect()
        };
        for key in presses {
            cancel_press(std, &key);
        }
        let claim = {
            let mut router = std.router.borrow_mut();
            router.drags.retain(|one| one.id != id);
            let shared = router.drags.iter().any(|one| one.key == reg.key);
            if !shared {
                router.presses.remove(&reg.key);
                router.pointer_claims.remove(&reg.key)
            } else {
                None
            }
        };
        if let Some(claim) = claim {
            let _ = claim.unsubscribe();
        }
        // The binding's `drop_user` runs when the last reference goes: here, or after the callback that is running returns.
        drop(reg);
        return true;
    }
    let gesture = {
        let router = std.router.borrow();
        router.gestures.iter().find(|reg| reg.id == id).cloned()
    };
    if let Some(reg) = gesture {
        let key = claim_key(reg.trigger.kind, reg.trigger.fingers);
        let running = std
            .router
            .borrow()
            .runs
            .get(&key)
            .is_some_and(|run| run.started.as_ref().is_some_and(|started| started.id == id));
        if running {
            cancel_gesture(std, key);
        }
        let claim = {
            let mut router = std.router.borrow_mut();
            router.gestures.retain(|one| one.id != id);
            let shared = router
                .gestures
                .iter()
                .any(|one| claim_key(one.trigger.kind, one.trigger.fingers) == key);
            if !shared {
                router.runs.remove(&key);
                router.gesture_claims.remove(&key)
            } else {
                None
            }
        };
        if let Some(claim) = claim {
            let _ = claim.unsubscribe();
        }
        drop(reg);
        return true;
    }
    false
}

/// The generation retired: every binding's `drop_user` runs now, while the shell's code is still mapped.
pub(crate) fn retire(std: &Rc<Std>) {
    let router = std::mem::take(&mut *std.router.borrow_mut());
    drop(router);
}

// ---- registering bindings ---------------------------------------------------------------------------------------------------

fn check_number(value: f64, what: &str) -> Res<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Fail::type_error(format!("{what} must be a finite number")))
    }
}

/// Registers a drag binding of `key` and returns its id. `make` builds the handler, which owns what the caller gave: it runs only
/// once nothing can fail any more.
fn add_drag(
    std: &Rc<Std>,
    key: &str,
    priority: f64,
    threshold: f64,
    make: impl FnOnce() -> Box<dyn DragHandler>,
) -> Res<Id> {
    if std
        .router
        .borrow()
        .drags
        .iter()
        .any(|reg| reg.key == key && reg.priority == priority)
    {
        return Err(Fail::busy(format!(
            "a binding of {key} at priority {priority} exists: give one a different priority or trigger"
        ))
        .with_code(ErrorCode::AlreadyExists));
    }
    if !std.router.borrow().pointer_claims.contains_key(key) {
        let weak = Rc::downgrade(std);
        let claimed = key.to_owned();
        let claim = std
            .g()
            .bind_pointer(key, move |drag| {
                if let Some(std) = weak.upgrade() {
                    pointer_event(&std, &claimed, drag);
                }
            })
            .map_err(Fail::from)?;
        std.router.borrow_mut().pointer_claims.insert(key.to_owned(), claim);
    }
    let handler = make();
    let mut router = std.router.borrow_mut();
    router.order += 1;
    let id = next_id();
    let order = router.order;
    router.drags.push(Rc::new(DragReg {
        id,
        key: key.to_owned(),
        priority,
        order,
        threshold,
        handler: RefCell::new(handler),
    }));
    Ok(id)
}

pub(crate) unsafe extern "C" fn binding_drag(
    ctx: GnoblinCtx,
    spec: *const GnoblinStdBindingSpec,
    cb: GnoblinStdDragFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinStdReg {
    call_reg("binding_drag", || {
        let std = crate::state::lookup(ctx)?;
        // SAFETY: the caller guarantees `spec` is a `GnoblinStdBindingSpec` whose trigger is readable.
        let spec = unsafe { read_ext(spec, "binding spec") }?;
        // SAFETY: as above.
        let trigger = unsafe { read_text(&spec.trigger) }?.trim().to_owned();
        if trigger.is_empty() {
            return Err(Fail::type_error("a binding needs a trigger"));
        }
        let priority = check_number(spec.priority, "priority")?;
        let threshold = if spec.threshold == 0.0 {
            DRAG_THRESHOLD
        } else {
            check_number(spec.threshold, "threshold")?
        };
        add_drag(&std, &trigger, priority, threshold, || {
            Box::new(ForeignDrag { cb, user, drop_user })
        })
    })
}

pub(crate) unsafe extern "C" fn binding_gesture(
    ctx: GnoblinCtx,
    spec: *const GnoblinStdBindingSpec,
    cb: GnoblinStdGestureFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinStdReg {
    call_reg("binding_gesture", || {
        let std = crate::state::lookup(ctx)?;
        // SAFETY: the caller guarantees `spec` is a `GnoblinStdBindingSpec` whose trigger is readable.
        let spec = unsafe { read_ext(spec, "binding spec") }?;
        // SAFETY: as above.
        let trigger = GestureTrigger::parse(unsafe { read_text(&spec.trigger) }?)?;
        let priority = check_number(spec.priority, "priority")?;
        let key = claim_key(trigger.kind, trigger.fingers);
        if std
            .router
            .borrow()
            .gestures
            .iter()
            .any(|reg| reg.trigger == trigger && reg.priority == priority)
        {
            return Err(Fail::busy(format!(
                "a binding of {} at priority {priority} exists: give one a different priority or trigger",
                trigger.text()
            ))
            .with_code(ErrorCode::AlreadyExists));
        }
        if !std.router.borrow().gesture_claims.contains_key(&key) {
            let weak = Rc::downgrade(&std);
            let claim = std
                .g()
                .bind_gesture(trigger.kind, trigger.fingers, move |raw| {
                    if let Some(std) = weak.upgrade() {
                        gesture_event(&std, key, raw);
                    }
                })
                .map_err(Fail::from)?;
            std.router.borrow_mut().gesture_claims.insert(key, claim);
        }
        let mut router = std.router.borrow_mut();
        router.order += 1;
        let id = next_id();
        let order = router.order;
        router.gestures.push(Rc::new(GestureReg {
            id,
            trigger,
            priority,
            order,
            handler: ForeignGesture { cb, user, drop_user },
        }));
        Ok(id)
    })
}

pub(crate) unsafe extern "C" fn binding_builtin(
    ctx: GnoblinCtx,
    kind: u32,
    options: *const GnoblinStdOption,
    n_options: usize,
) -> GnoblinStdReg {
    call_reg("binding_builtin", || {
        let std = crate::state::lookup(ctx)?;
        let desc =
            binding_kind(kind).ok_or_else(|| Fail::type_error(format!("unknown built-in binding kind {kind}")))?;
        // SAFETY: the caller guarantees `options` points at `n_options` records.
        let options: OptionSet = unsafe { read_set(desc.name, desc.options, options, n_options) }?;
        let trigger = match options.get("trigger") {
            Val::Text(text) => text.trim().to_owned(),
            _ => String::new(),
        };
        if trigger.is_empty() {
            return Err(Fail::type_error(format!("{}.trigger must not be empty", desc.name)));
        }
        let priority = options.num("priority");
        let rule = DragRule::of(kind, &options)
            .ok_or_else(|| Fail::type_error(format!("unknown built-in binding kind {kind}")))?;
        add_drag(&std, &trigger, priority, DRAG_THRESHOLD, || {
            Box::new(RuleDrag::new(rule))
        })
    })
}
