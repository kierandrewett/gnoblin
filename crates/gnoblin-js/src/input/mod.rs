//! The host side of input: seat claims, the seat's published state, and the chords and gestures the
//! compositor reports.
//!
//! `data/js/gnoblin-core/input/` decides what an input means; this module is what it stands on. It does what the
//! retired shell's `node.retired shell` does for `Seat:claim`, `Seat:claim_gesture` and `Seat:unclaim`:
//!
//! - **Claims.** Each claim id is allocated here, per shell, from 1, never reused, one number space for chords
//!   and gestures (the compositor's `unclaim` removes either). The claim is a call on the seat node whose
//!   argument bytes are little-endian `i64`s (`id`, then `grab` or `fingers`) and whose text is the chord or
//!   gesture kind, behind the device name and a line feed when the claim is for one device. A claim is queued
//!   like any write and travels at the end of the frame. The compositor takes a claim it cannot parse
//!   silently (it logs and drops it), so a refusal is not visible here.
//! - **Hand-off.** The compositor refuses a chord that another claim already holds, and a reload stages the new
//!   generation while the old one is still the one holding its claims. A claim a staged generation makes is
//!   therefore kept here (it has its id at once, and the script goes on) and sent only when the generation takes
//!   over, behind the unclaims of the old one: the compositor applies one transaction's calls in order, so the
//!   chords are free when the new claims arrive and there is no moment in which a chord is held by nobody. A
//!   stage that fails drops what it kept.
//! - **Messages.** A claimed chord or gesture arrives on the tree link as [`TreeIncoming::Chord`] or
//!   [`TreeIncoming::Gesture`]. The shell parks them in [`JsInput::inbox`] and hands the frame's batch to the
//!   module in one call, ahead of the listener queue it then drains.
//! - **Reads.** The seat's pointer, focus and device list come from the mirror: what the compositor last
//!   published, which is also what a drag samples once per frame.
//!
//! The module is split by concern. This file holds the shell's input state and the messages the compositor reports;
//! `claims` makes and hands off seat claims, `seats` reads the seats and `install` puts the functions on the host
//! bridge.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_tree_client::tree_mirror::{
    NodeId, TreeChord, TreeEvent, TreeGesture, TreeMirror, TreePointerButton, TreePointerChord, class_name,
};
use rquickjs::{Array, Ctx, Object};

use self::claims::StagedClaim;
use crate::bindings::NodeHandles;
use crate::supervisor::JsClock;

mod claims;
mod install;
mod seats;

pub(crate) use self::install::install;

pub(crate) use self::seats::focus_window;

pub(crate) use self::seats::seat_nodes;

pub(crate) use self::claims::send_staged;

pub(crate) use self::claims::release_all;

pub(crate) use self::claims::end_staging;

pub(crate) use self::claims::abandon_staging;

pub(crate) use self::claims::begin_staging;

/// A claimed chord, gesture, pointer drag or pointer button the compositor reported, parked until the frame hands it to the module.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsInputMessage {
    Chord(TreeChord),
    Gesture(TreeGesture),
    Pointer(TreePointerChord),
    Button(TreePointerButton),
}

/// A pointer edge on a window, in the window's own coordinates: the compositor tells the shell where the pointer
/// enters, moves over, presses on, releases on and leaves a window, and the focus and raise policy follows them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JsWindowPointer {
    /// The window, as the script's number for the node.
    node: f64,
    kind: String,
    x: f64,
    y: f64,
    button: f64,
}

/// The event kinds a window's `ClientListener` and a scene node's pointer handlers hear. A scroll is not one: nothing
/// in the stock policy reads it.
const WINDOW_POINTER_KINDS: &[&str] = &["press", "release", "motion", "enter", "leave"];

impl JsWindowPointer {
    /// The event as a script object: `{ node, kind, x, y, button }`.
    fn to_js<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let item = Object::new(ctx.clone())?;
        item.set("node", self.node)?;
        item.set("kind", self.kind.as_str())?;
        item.set("x", self.x)?;
        item.set("y", self.y)?;
        item.set("button", self.button)?;
        Ok(item)
    }
}

/// Everything input keeps on the host between frames.
pub(crate) struct JsInput {
    /// The last claim id handed out.
    last_claim: i64,
    /// The seat each live claim was made on, so `unclaim` finds the node and a stale id is a no-op.
    claims: HashMap<i64, NodeId>,
    /// Whether a generation is being staged: its claims wait in `staged` instead of being sent.
    staging: bool,
    /// The claims a staged generation made, in order, which go out when it takes over and never if it does not.
    staged: Vec<StagedClaim>,
    /// Chords and gestures that arrived since the last frame's dispatch, oldest first.
    pub inbox: Vec<JsInputMessage>,
    /// The pointer edges on windows that arrived since the last frame's dispatch, oldest first.
    window_events: Vec<JsWindowPointer>,
    /// The clock `now` reads: the shell's own, or the one a test injected.
    clock: Arc<dyn JsClock>,
    /// What the clock read when this was made, so `now` counts from the shell's start as it always has.
    origin: Duration,
}

impl JsInput {
    pub(crate) fn new(clock: Arc<dyn JsClock>) -> Self {
        Self {
            last_claim: 0,
            claims: HashMap::new(),
            staging: false,
            staged: Vec::new(),
            inbox: Vec::new(),
            window_events: Vec::new(),
            origin: clock.now(),
            clock,
        }
    }

    /// Milliseconds since the shell started, on the shell's clock: the monotonic time every script timer, fade and
    /// arming window reads.
    pub(crate) fn now_ms(&self) -> f64 {
        self.clock.now().saturating_sub(self.origin).as_secs_f64() * 1000.0
    }

    /// Keeps a pointer event addressed to a window of the tree, or to a `View` or `Node` (a script's own scene node,
    /// which hears it through its handlers: a View whose `input` is `capture` is sent its edges); any other event is
    /// not input's.
    pub(crate) fn accept_window_event(
        &mut self,
        mirror: &TreeMirror,
        handles: &NodeHandles,
        event: &TreeEvent,
    ) -> bool {
        let on_window = mirror
            .node(event.node)
            .is_some_and(|record| matches!(class_name(&record.class).as_str(), "Window" | "View" | "Node"));
        if !on_window || !WINDOW_POINTER_KINDS.contains(&event.kind.as_str()) {
            return false;
        }
        self.window_events.push(JsWindowPointer {
            node: handles.to_script(event.node),
            kind: event.kind.clone(),
            x: event.fields.x,
            y: event.fields.y,
            button: event.fields.button as f64,
        });
        true
    }

    /// Takes the frame's window pointer events as a script array, oldest first.
    pub(crate) fn take_window_events<'js>(&mut self, ctx: &Ctx<'js>) -> rquickjs::Result<Array<'js>> {
        let list = Array::new(ctx.clone())?;
        for (index, event) in std::mem::take(&mut self.window_events).iter().enumerate() {
            list.set(index, event.to_js(ctx)?)?;
        }
        Ok(list)
    }

    /// Drops what has not been handed to the script, for a generation that is going away.
    pub(crate) fn forget_window_events(&mut self) {
        self.window_events.clear();
    }

    /// How many claims are held, for a test.
    #[cfg(test)]
    pub(crate) fn claimed(&self) -> usize {
        self.claims.len()
    }
}

/// The frame's chord, gesture and pointer messages as script objects, for the module's `input` entry.
pub(crate) fn messages_to_js<'js>(ctx: &Ctx<'js>, messages: &[JsInputMessage]) -> rquickjs::Result<Array<'js>> {
    let list = Array::new(ctx.clone())?;
    for (index, message) in messages.iter().enumerate() {
        let item = Object::new(ctx.clone())?;
        match message {
            JsInputMessage::Chord(chord) => {
                item.set("kind", "chord")?;
                item.set("claim", chord.claim as f64)?;
                item.set("chord", chord.chord.as_str())?;
                item.set("state", chord.state.as_str())?;
            }
            JsInputMessage::Gesture(gesture) => {
                item.set("kind", "gesture")?;
                item.set("claim", gesture.claim as f64)?;
                item.set("gesture", gesture.kind.as_str())?;
                item.set("fingers", gesture.fingers as f64)?;
                item.set("phase", gesture.phase.as_str())?;
                item.set("dx", gesture.dx)?;
                item.set("dy", gesture.dy)?;
                item.set("scale", gesture.scale)?;
                item.set("rotation", gesture.rotation)?;
            }
            JsInputMessage::Pointer(pointer) => {
                item.set("kind", "pointer")?;
                item.set("claim", pointer.claim as f64)?;
                item.set("phase", pointer.phase.as_str())?;
                item.set("x", pointer.x)?;
                item.set("y", pointer.y)?;
                item.set("dx", pointer.dx)?;
                item.set("dy", pointer.dy)?;
                item.set("button", pointer.button as f64)?;
                item.set("modifiers", pointer.modifiers.as_str())?;
            }
            JsInputMessage::Button(button) => {
                item.set("kind", "button")?;
                item.set("claim", button.claim as f64)?;
                item.set("button", button.button as f64)?;
                item.set("state", button.state.as_str())?;
                item.set("x", button.x)?;
                item.set("y", button.y)?;
                item.set("time", button.time as f64)?;
            }
        }
        list.set(index, item)?;
    }
    Ok(list)
}

#[cfg(test)]
mod tests;
