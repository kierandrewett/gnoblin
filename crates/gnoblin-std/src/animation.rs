//! Animation: easing curves and transitions.
//!
//! A [`Transition`] draws a change of state as one animation. [`Std::transition`] runs a closure once, now, and stages the property
//! writes it makes through a [`Tx`] (and the layout and style changes the library makes in answer to them); the compositor's tween
//! player then moves what is drawn from where it is to where the closure put it. The state is committed when the transition is
//! requested: reading a property after the call gives the end value, and the compositor draws the way there. The compositor plays the
//! tracks and says when they end, so a transition ends when its tracks do and not by the shell's clock.
//!
//! ```ignore
//! let std = g.std()?;
//! let t = std.transition(TransitionOptions::new().duration_ms(250.0).easing(Easing::EaseOut), |tx| {
//!     tx.set(client.window().id(), "opacity", 0.85)?;
//!     tx.set(client.window().id(), "x", 120.0)?;
//!     workspace.set_layout(MasterStackLayout::new().ratio(0.7)) // computed once, against the staged state
//! })?;
//! t.on_finished(|result| println!("cancelled: {}", result.cancelled))?;
//! ```
//!
//! This is `data/js/gnoblin-std/transition/` for Rust. What differs: a closure stages writes of the shell's own and of the library's
//! passes, and a failure puts nothing in the tree, but it cannot restore what the closure changed of the shell's own state (a layout
//! object it assigned, a collection it edited); the TypeScript std restores the style and layout of the workspaces, the SDK does not.
//! A `Transition` has no `Promise`: [`Transition::on_finished`] is the listener.
#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use gnoblin_core::NodeId;
use gnoblin_core_abi as core_abi;
use gnoblin_std_abi as abi;
use gnoblin_std_abi::{GnoblinStdEvent, GnoblinStdProp, GnoblinStdTransition, GnoblinStdTransitionInfo};

use crate::api::Std;
use crate::collection::drop_slot;
use crate::error::Result;
use crate::subscription::Subscription;

/// One of the CSS timing functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Easing {
    /// Constant speed.
    Linear,
    /// `cubic-bezier(0.25, 0.1, 0.25, 1)`.
    Ease,
    /// `cubic-bezier(0.42, 0, 1, 1)`.
    EaseIn,
    /// `cubic-bezier(0, 0, 0.58, 1)`: the default of a transition.
    #[default]
    EaseOut,
    /// `cubic-bezier(0.42, 0, 0.58, 1)`.
    EaseInOut,
}

impl Easing {
    fn raw(self) -> u32 {
        match self {
            Easing::Linear => abi::STD_EASING_LINEAR,
            Easing::Ease => abi::STD_EASING_EASE,
            Easing::EaseIn => abi::STD_EASING_EASE_IN,
            Easing::EaseOut => abi::STD_EASING_EASE_OUT,
            Easing::EaseInOut => abi::STD_EASING_EASE_IN_OUT,
        }
    }

    /// The eased value at progress `t`, both from 0 to 1.
    ///
    /// # Errors
    ///
    /// The library is not usable (the generation retired).
    pub fn value(self, std: &Std, t: f64) -> Result<f64> {
        let mut out = 0.0;
        // SAFETY: `easing_sample` writes one double through the pointer it is given, a live local.
        std.check(unsafe { (std.api().easing_sample)(std.ctx(), self.raw(), t, &mut out) })?;
        Ok(out)
    }

    /// The curve a timed write carries: progress from 0 to 1 sampled 33 times.
    ///
    /// # Errors
    ///
    /// As for [`Easing::value`].
    pub fn samples(self, std: &Std) -> Result<Vec<f64>> {
        (0..=32).map(|step| self.value(std, f64::from(step) / 32.0)).collect()
    }
}

/// The options a transition takes. The default of each is on its setter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionOptions {
    duration_ms: f64,
    easing: Easing,
    delay_ms: f64,
    autoplay: bool,
}

impl Default for TransitionOptions {
    fn default() -> TransitionOptions {
        TransitionOptions {
            duration_ms: 250.0,
            easing: Easing::EaseOut,
            delay_ms: 0.0,
            autoplay: true,
        }
    }
}

impl TransitionOptions {
    /// A transition of 250 milliseconds, eased out, with no delay, that plays at once.
    pub fn new() -> TransitionOptions {
        TransitionOptions::default()
    }

    /// Milliseconds, 0 or more. Default 250. Zero commits the change at once, still as one transaction.
    #[must_use]
    pub fn duration_ms(mut self, duration_ms: f64) -> TransitionOptions {
        self.duration_ms = duration_ms;
        self
    }

    /// How the animation moves from its start to its end. Default [`Easing::EaseOut`].
    #[must_use]
    pub fn easing(mut self, easing: Easing) -> TransitionOptions {
        self.easing = easing;
        self
    }

    /// Milliseconds, 0 or more, the compositor holds the start state before the animation begins. Default 0; none under reduced
    /// motion.
    #[must_use]
    pub fn delay_ms(mut self, delay_ms: f64) -> TransitionOptions {
        self.delay_ms = delay_ms;
        self
    }

    /// False stages the transition with its tracks held at the start: [`Transition::set_progress`] scrubs them,
    /// [`Transition::play`] and [`Transition::reverse`] run them, [`Transition::pause`] holds them. Default true.
    #[must_use]
    pub fn autoplay(mut self, autoplay: bool) -> TransitionOptions {
        self.autoplay = autoplay;
        self
    }

    fn raw(self) -> abi::GnoblinStdTransitionOptions {
        abi::GnoblinStdTransitionOptions {
            size: size_of::<abi::GnoblinStdTransitionOptions>() as u32,
            autoplay: u32::from(self.autoplay),
            easing: self.easing.raw(),
            reserved: 0,
            duration_ms: self.duration_ms,
            delay_ms: self.delay_ms,
        }
    }
}

/// A value a property takes in a transition or a scene write.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    /// A number (`x`, `opacity`).
    Number(f64),
    /// A boolean (`visible`).
    Bool(bool),
    /// Three numbers (`rotation`, `scale`).
    Triple([f64; 3]),
    /// Text (`name`, `projection`).
    Text(String),
    /// A node (`source`, `camera`).
    Node(NodeId),
}

impl From<f64> for PropValue {
    fn from(value: f64) -> PropValue {
        PropValue::Number(value)
    }
}

impl From<bool> for PropValue {
    fn from(value: bool) -> PropValue {
        PropValue::Bool(value)
    }
}

impl From<[f64; 3]> for PropValue {
    fn from(value: [f64; 3]) -> PropValue {
        PropValue::Triple(value)
    }
}

impl From<&str> for PropValue {
    fn from(value: &str) -> PropValue {
        PropValue::Text(value.to_owned())
    }
}

impl From<String> for PropValue {
    fn from(value: String) -> PropValue {
        PropValue::Text(value)
    }
}

impl From<NodeId> for PropValue {
    fn from(value: NodeId) -> PropValue {
        PropValue::Node(value)
    }
}

impl PropValue {
    /// The ABI struct for the value, borrowing its text for as long as `self` lives.
    pub(crate) fn raw(&self) -> GnoblinStdProp {
        let mut raw = GnoblinStdProp {
            size: size_of::<GnoblinStdProp>() as u32,
            kind: abi::STD_PROP_UNSET,
            a: 0.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            text: core_abi::GnoblinStr::EMPTY,
            node: 0,
        };
        match self {
            PropValue::Number(value) => {
                raw.kind = abi::STD_PROP_NUMBER;
                raw.a = *value;
            }
            PropValue::Bool(value) => {
                raw.kind = abi::STD_PROP_BOOL;
                raw.a = f64::from(u8::from(*value));
            }
            PropValue::Triple([a, b, c]) => {
                raw.kind = abi::STD_PROP_TRIPLE;
                (raw.a, raw.b, raw.c) = (*a, *b, *c);
            }
            PropValue::Text(text) => {
                raw.kind = abi::STD_PROP_TEXT;
                raw.text = core_abi::GnoblinStr::borrowed(text);
            }
            PropValue::Node(node) => {
                raw.kind = abi::STD_PROP_NODE;
                raw.node = node.0;
            }
        }
        raw
    }

    /// The value an ABI struct holds, copied; `None` for kinds a plain property does not have.
    pub(crate) fn from_raw(raw: &GnoblinStdProp) -> Option<PropValue> {
        Some(match raw.kind {
            abi::STD_PROP_NUMBER => PropValue::Number(raw.a),
            abi::STD_PROP_BOOL => PropValue::Bool(raw.a != 0.0),
            abi::STD_PROP_TRIPLE => PropValue::Triple([raw.a, raw.b, raw.c]),
            abi::STD_PROP_NODE => PropValue::Node(NodeId(raw.node)),
            abi::STD_PROP_TEXT => PropValue::Text(lent_text(&raw.text)),
            _ => return None,
        })
    }
}

/// A string the library lent, copied.
pub(crate) fn lent_text(text: &core_abi::GnoblinStr) -> String {
    if text.ptr.is_null() || text.len == 0 {
        String::new()
    } else {
        // SAFETY: the library lends `len` readable bytes at `ptr` until the next call into it on this thread, and the caller copies
        // them before then.
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text.ptr, text.len) }).into_owned()
    }
}

/// How a transition stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionState {
    /// It has tracks that have not ended.
    Running,
    /// Every track reached its target.
    Finished,
    /// It was cancelled, a newer write took every property it animated, or it ran back to its start.
    Cancelled,
}

/// What a transition is now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionInfo {
    /// Whether it is running, finished or cancelled.
    pub state: TransitionState,
    /// Its number, which grows with every transition of the process.
    pub id: u64,
    /// The milliseconds it takes: the duration asked for, or 0 under reduced motion.
    pub duration_ms: f64,
    /// Where the tracks were last put, 0 (the start) to 1 (the target), before the easing. The compositor does not report the
    /// place of a running transition.
    pub progress: f64,
    /// Whether the tracks are held: `autoplay(false)` until [`Transition::play`] or [`Transition::reverse`], and after
    /// [`Transition::pause`].
    pub paused: bool,
    /// Whether the tracks run back to their start.
    pub reversed: bool,
}

/// What `on_finished` delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionResult {
    /// True when the transition did not reach its target.
    pub cancelled: bool,
}

/// The writes of one transition's callback: held until the closure returns, then sent as timed writes. A read through it sees the
/// staged value first.
pub struct Tx<'a> {
    std: &'a Std,
}

impl Tx<'_> {
    /// Stages a write of `key` of `node`, checked at once.
    ///
    /// # Errors
    ///
    /// The node is gone, has no such property, the property is read-only or the value is not of its kind.
    pub fn set(&self, node: NodeId, key: &str, value: impl Into<PropValue>) -> Result<()> {
        self.std.set_property(node, key, &value.into())
    }

    /// Stages a write that plays over `options.duration_ms` along its own curve, whatever the transition's options are (a workspace
    /// slide that a layout pass starts is one).
    ///
    /// # Errors
    ///
    /// As for [`Tx::set`], and the property is not one the tree animates.
    pub fn animate(
        &self,
        node: NodeId,
        key: &str,
        value: impl Into<PropValue>,
        options: TransitionOptions,
    ) -> Result<()> {
        self.std.animate(node, key, value, options).map(|_| ())
    }

    /// The staged value of `key` of `node`, else the tree's; `None` for a property that has no plain value.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn get(&self, node: NodeId, key: &str) -> Result<Option<PropValue>> {
        self.std.property(node, key)
    }

    /// The number a property holds, staged or not.
    ///
    /// # Errors
    ///
    /// As for [`Tx::get`].
    pub fn number(&self, node: NodeId, key: &str) -> Result<Option<f64>> {
        Ok(match self.get(node, key)? {
            Some(PropValue::Number(value)) => Some(value),
            _ => None,
        })
    }
}

/// A transition: timed writes that play together, joined to one `tween` node of the compositor.
#[derive(Clone)]
pub struct Transition {
    std: Std,
    id: GnoblinStdTransition,
}

impl std::fmt::Debug for Transition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transition").field("id", &self.id).finish()
    }
}

impl PartialEq for Transition {
    fn eq(&self, other: &Transition) -> bool {
        self.id == other.id
    }
}

impl Transition {
    /// What the transition is now.
    ///
    /// # Errors
    ///
    /// The generation retired, or the transition ended so long ago that the library forgot it.
    pub fn info(&self) -> Result<TransitionInfo> {
        let mut raw = GnoblinStdTransitionInfo {
            size: size_of::<GnoblinStdTransitionInfo>() as u32,
            state: 0,
            id: 0,
            duration_ms: 0.0,
            progress: 0.0,
            paused: 0,
            reversed: 0,
        };
        // SAFETY: `transition_info` fills the struct it is given up to the `size` the caller set; `raw` is a live local.
        self.std
            .check(unsafe { (self.std.api().transition_info)(self.std.ctx(), self.id, &mut raw) })?;
        Ok(TransitionInfo {
            state: match raw.state {
                abi::STD_TRANSITION_RUNNING => TransitionState::Running,
                abi::STD_TRANSITION_FINISHED => TransitionState::Finished,
                _ => TransitionState::Cancelled,
            },
            id: raw.id,
            duration_ms: raw.duration_ms,
            progress: raw.progress,
            paused: raw.paused != 0,
            reversed: raw.reversed != 0,
        })
    }

    /// True from the call until the transition finishes or is cancelled.
    ///
    /// # Errors
    ///
    /// As for [`Transition::info`].
    pub fn is_running(&self) -> Result<bool> {
        Ok(self.info()?.state == TransitionState::Running)
    }

    fn control(&self, op: u32, value: f64) -> Result<()> {
        // SAFETY: `transition_control` takes the handle, an operation and a number.
        self.std
            .check(unsafe { (self.std.api().transition_control)(self.std.ctx(), self.id, op, value) })
            .map(|_| ())
    }

    /// Moves every track to `progress` (0 to 1) of its duration at once and draws it; it does not play or pause.
    ///
    /// # Errors
    ///
    /// `progress` is outside 0 to 1, or this is called inside a transition's closure.
    pub fn set_progress(&self, progress: f64) -> Result<()> {
        self.control(abi::STD_CONTROL_SEEK, progress)
    }

    /// Lets the tracks run on towards the target, from where they are.
    ///
    /// # Errors
    ///
    /// This is called inside a transition's closure.
    pub fn play(&self) -> Result<()> {
        self.control(abi::STD_CONTROL_PLAY, 0.0)
    }

    /// Holds the tracks where they are.
    ///
    /// # Errors
    ///
    /// As for [`Transition::play`].
    pub fn pause(&self) -> Result<()> {
        self.control(abi::STD_CONTROL_PAUSE, 0.0)
    }

    /// Runs the tracks back towards their start from where they are. When they get there the transition settles cancelled and the
    /// start values are back in the tree.
    ///
    /// # Errors
    ///
    /// As for [`Transition::play`].
    pub fn reverse(&self) -> Result<()> {
        self.control(abi::STD_CONTROL_REVERSE, 0.0)
    }

    /// Stops the transition: every property it still owns is written at its end value at once, which ends the compositor's track, and
    /// it settles cancelled. The target state was committed when it began, so cancelling puts what is drawn there and does not go
    /// back. Nothing happens once it ended.
    ///
    /// # Errors
    ///
    /// As for [`Transition::play`].
    pub fn cancel(&self) -> Result<()> {
        self.control(abi::STD_CONTROL_CANCEL, 0.0)
    }

    /// Calls `f` once when the transition ends, from the next turn of the dispatch. One added after it ended runs from the next turn.
    ///
    /// # Errors
    ///
    /// The generation retired, or the transition ended so long ago that the library forgot it.
    pub fn on_finished<F>(&self, f: F) -> Result<Subscription>
    where
        F: FnOnce(TransitionResult) + 'static,
    {
        let slot = Box::into_raw(Box::new(FinishedSlot { f: Some(Box::new(f)) }));
        // SAFETY: `slot` is a leaked box the library hands back to `finished_tramp` and `drop_slot::<FinishedSlot>`; on failure the
        // library has not adopted it and it is reclaimed below.
        let reg = unsafe {
            (self.std.api().transition_finished)(
                self.std.ctx(),
                self.id,
                finished_tramp,
                slot.cast(),
                Some(drop_slot::<FinishedSlot>),
            )
        };
        if reg == 0 {
            // SAFETY: the library refused the registration, so it has not adopted the slot and nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
            return Err(self.std.failure());
        }
        Ok(Subscription::new(self.std.clone(), reg))
    }
}

/// The closure behind an `on_finished`.
struct FinishedSlot {
    f: Option<Box<dyn FnOnce(TransitionResult)>>,
}

unsafe extern "C" fn finished_tramp(user: *mut core::ffi::c_void, event: *const GnoblinStdEvent) -> i32 {
    // SAFETY: `user` is the `Box<FinishedSlot>` leaked by `on_finished`; the library passes it back unchanged, on the shell thread,
    // and does not free it before `drop_slot` runs.
    let slot = unsafe { &mut *user.cast::<FinishedSlot>() };
    // SAFETY: the library passes a pointer that is valid until this callback returns, or null.
    let Some(event) = (unsafe { event.as_ref() }) else {
        return core_abi::E_ERR;
    };
    let result = TransitionResult {
        cancelled: event.flag != 0,
    };
    match slot.f.take() {
        Some(f) => match catch_unwind(AssertUnwindSafe(|| f(result))) {
            Ok(()) => core_abi::E_OK,
            Err(_) => core_abi::E_PANIC,
        },
        None => core_abi::E_OK,
    }
}

impl Std {
    /// Draws the changes `f` makes as one animation. `f` runs once, now; the writes it stages through the [`Tx`], and what the library
    /// writes in answer (a layout it changed is computed once, against the staged state, and each window that moves tweens from where
    /// it is drawn), travel in the shell's one transaction as timed writes.
    ///
    /// # Errors
    ///
    /// The options are bad (a negative duration or delay), another transition is being staged, `f` fails (nothing is written), or the
    /// compositor refuses a write.
    pub fn transition<F>(&self, options: TransitionOptions, f: F) -> Result<Transition>
    where
        F: FnOnce(&Tx<'_>) -> Result<()>,
    {
        let raw = options.raw();
        // SAFETY: `transition_begin` reads the options struct for the call.
        self.check(unsafe { (self.api().transition_begin)(self.ctx(), &raw) })?;
        match catch_unwind(AssertUnwindSafe(|| f(&Tx { std: self }))) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.abort_transition();
                return Err(error);
            }
            Err(panic) => {
                self.abort_transition();
                resume_unwind(panic);
            }
        }
        let mut id = 0;
        // SAFETY: `transition_commit` writes one handle through the pointer it is given, a live local.
        let committed = self.check(unsafe { (self.api().transition_commit)(self.ctx(), &mut id) });
        if let Err(error) = committed {
            self.abort_transition();
            return Err(error);
        }
        Ok(Transition { std: self.clone(), id })
    }

    fn abort_transition(&self) {
        // SAFETY: `transition_abort` takes only the context.
        let _ = unsafe { (self.api().transition_abort)(self.ctx()) };
    }

    /// One timed write of `key` of `node`: a transition of that one property. Inside a transition's closure it joins the transition
    /// being staged, with its own duration and curve, and returns `None`.
    ///
    /// # Errors
    ///
    /// As for [`Std::transition`], and the property is not one the tree animates (`x`, `y`, `z`, `width`, `height`, `opacity`,
    /// `rotation`, `scale`).
    pub fn animate(
        &self,
        node: NodeId,
        key: &str,
        value: impl Into<PropValue>,
        options: TransitionOptions,
    ) -> Result<Option<Transition>> {
        let value = value.into();
        let (raw_value, raw_options) = (value.raw(), options.raw());
        let mut id = 0;
        // SAFETY: the key, the value (and its text, borrowed from `value`) and the options are read for the call; `id` is a live local.
        self.check(unsafe {
            (self.api().transition_animate)(
                self.ctx(),
                node.0,
                key.as_ptr(),
                key.len(),
                &raw_value,
                &raw_options,
                &mut id,
            )
        })?;
        Ok((id != 0).then(|| Transition { std: self.clone(), id }))
    }

    /// Writes `key` of `node` outside a transition's closure: at once, as a plain write (it ends the compositor's track on the
    /// property if a transition still plays it). Inside a closure it is staged.
    pub(crate) fn set_property(&self, node: NodeId, key: &str, value: &PropValue) -> Result<()> {
        let raw = value.raw();
        // SAFETY: the key and the value (and its text, borrowed from `value`) are read for the call.
        self.check(unsafe { (self.api().transition_set)(self.ctx(), node.0, key.as_ptr(), key.len(), &raw) })
            .map(|_| ())
    }

    /// Reads `key` of `node`: the staged value while a transition is staging, else the tree's.
    pub(crate) fn property(&self, node: NodeId, key: &str) -> Result<Option<PropValue>> {
        let mut raw = PropValue::Number(0.0).raw();
        // SAFETY: the key is read for the call; `transition_get` fills the struct up to the `size` the caller set, and `raw` is a live
        // local. A text it lends is copied before the next call into the library.
        self.check(unsafe { (self.api().transition_get)(self.ctx(), node.0, key.as_ptr(), key.len(), &mut raw) })?;
        Ok(PropValue::from_raw(&raw))
    }
}
