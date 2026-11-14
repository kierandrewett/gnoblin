//! The host side of input: seat claims, the seat's published state, and the chords and gestures the
//! compositor reports.
//!
//! `data/js/gnoblin/input.ts` decides what an input means; this module is what it stands on. It does what the
//! Lua shell's `node.lua` does for `Seat:claim`, `Seat:claim_gesture` and `Seat:unclaim`:
//!
//! - **Claims.** Each claim id is allocated here, per shell, from 1, never reused, one number space for chords
//!   and gestures (the compositor's `unclaim` removes either). The claim is a call on the seat node whose
//!   argument bytes are little-endian `i64`s (`id`, then `grab` or `fingers`) and whose text is the chord or
//!   gesture kind, behind the device name and a line feed when the claim is for one device. A claim is queued
//!   like any write and travels at the end of the frame. The compositor takes a claim it cannot parse
//!   silently (it logs and drops it), so a refusal is not visible here.
//! - **Messages.** A claimed chord or gesture arrives on the tree link as [`TreeIncoming::Chord`] or
//!   [`TreeIncoming::Gesture`]. The shell parks them in [`JsInput::inbox`] and hands the frame's batch to the
//!   module in one call, ahead of the listener queue it then drains.
//! - **Reads.** The seat's pointer, focus and device list come from the mirror: what the compositor last
//!   published, which is also what a drag samples once per frame.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{
    MirrorValue, NIL, NodeId, TreeChord, TreeEvent, TreeGesture, TreeMirror, TreePointerChord, lua_class,
};
use rquickjs::{Array, Ctx, Function, IntoJs, Object, Value};

use crate::bindings::{HostState, NodeHandles, SharedState, parents_first};
use crate::supervisor::JsClock;

/// A claimed chord, gesture or pointer drag the compositor reported, parked until the frame hands it to the module.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsInputMessage {
    Chord(TreeChord),
    Gesture(TreeGesture),
    Pointer(TreePointerChord),
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

/// The event kinds a window's `ClientListener` hears. A scroll is not one: nothing in the stock policy reads it.
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

    /// Keeps a pointer event addressed to a window of the tree; any other event is not input's.
    pub(crate) fn accept_window_event(
        &mut self,
        mirror: &TreeMirror,
        handles: &NodeHandles,
        event: &TreeEvent,
    ) -> bool {
        let on_window = mirror
            .node(event.node)
            .is_some_and(|record| lua_class(&record.class) == "Window");
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

/// What a claim attempt answers: the claim's id, or the refusal code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum JsClaimOutcome {
    Claimed(i64),
    Refused(&'static str),
}

impl<'js> IntoJs<'js> for JsClaimOutcome {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            // Claim ids start at 1 and a script never holds enough of them to lose precision.
            Self::Claimed(id) => Ok(Value::new_number(ctx.clone(), id as f64)),
            Self::Refused(code) => code.into_js(ctx),
        }
    }
}

/// The seat nodes, in tree order.
pub(crate) fn seat_nodes(state: &HostState) -> Vec<NodeId> {
    parents_first(&state.mirror)
        .into_iter()
        .filter(|id| {
            state
                .mirror
                .node(*id)
                .is_some_and(|record| lua_class(&record.class) == "Seat")
        })
        .collect()
}

/// The seat nodes as the numbers a script holds.
fn seats(state: &HostState) -> Vec<f64> {
    seat_nodes(state)
        .into_iter()
        .map(|id| state.handles.to_script(id))
        .collect()
}

fn seat_prop<'a>(state: &'a HostState, seat: NodeId, key: &str) -> Option<&'a MirrorValue> {
    let record = state.mirror.node(seat)?;
    (lua_class(&record.class) == "Seat")
        .then(|| record.props.get(key))
        .flatten()
}

fn seat_name(state: &HostState, seat: NodeId) -> String {
    match seat_prop(state, seat, "name") {
        Some(MirrorValue::Text(name)) => name.clone(),
        _ => String::new(),
    }
}

/// The pointer as the compositor last published it; the origin before it has published one.
fn pointer(state: &HostState, seat: NodeId) -> Vec<f64> {
    let axis = |key: &str| match seat_prop(state, seat, key) {
        Some(MirrorValue::Double(value)) => *value,
        _ => 0.0,
    };
    vec![axis("pointer_x"), axis("pointer_y")]
}

/// The node with keyboard focus, or 0.
fn focus(state: &HostState, seat: NodeId) -> f64 {
    match seat_prop(state, seat, "focus") {
        Some(MirrorValue::Node(node)) => state.handles.to_script(*node),
        _ => 0.0,
    }
}

/// The seat's drag constraint, `work_area` or `minimum_strip`, as the compositor configured it. Any other text
/// is passed on as it is, so the script that reads it can refuse it by name.
fn drag_constraint(state: &HostState, seat: NodeId) -> String {
    match seat_prop(state, seat, "drag_constraint") {
        Some(MirrorValue::Text(text)) => text.clone(),
        _ => "work_area".to_owned(),
    }
}

/// The window management settings the compositor publishes on a seat, which the stock focus policy and edge tiling
/// read. Each is the schema's default (`crates/gnoblin-schema/schema/tree/seat.toml`) until the compositor says
/// otherwise, and the values are passed on as they are: the script's checks refuse a word it does not know, by name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct JsSeatSettings {
    edge_tiling: bool,
    focus_mode: String,
    focus_new_windows: String,
    raise_on_click: bool,
    auto_raise: bool,
    auto_raise_delay: f64,
    focus_change_on_pointer_rest: bool,
    /// `appearance.reduced-motion`: the shell shortens its animations to zero while it is true.
    reduced_motion: bool,
}

impl JsSeatSettings {
    fn of(state: &HostState, seat: NodeId) -> Self {
        let flag = |key: &str, default: bool| match seat_prop(state, seat, key) {
            Some(MirrorValue::Bool(value)) => *value,
            _ => default,
        };
        let word = |key: &str, default: &str| match seat_prop(state, seat, key) {
            Some(MirrorValue::Text(text)) => text.clone(),
            _ => default.to_owned(),
        };
        let delay = match seat_prop(state, seat, "auto_raise_delay") {
            Some(MirrorValue::Int(ms)) => *ms as f64,
            Some(MirrorValue::Uint(ms)) => *ms as f64,
            Some(MirrorValue::Double(ms)) => *ms,
            _ => 500.0,
        };
        Self {
            edge_tiling: flag("edge_tiling", false),
            focus_mode: word("focus_mode", "click"),
            focus_new_windows: word("focus_new_windows", "prevent"),
            raise_on_click: flag("raise_on_click", true),
            auto_raise: flag("auto_raise", false),
            auto_raise_delay: delay,
            focus_change_on_pointer_rest: flag("focus_change_on_pointer_rest", false),
            reduced_motion: flag("reduced_motion", false),
        }
    }
}

impl<'js> IntoJs<'js> for JsSeatSettings {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("edgeTiling", self.edge_tiling)?;
        object.set("focusMode", self.focus_mode)?;
        object.set("focusNewWindows", self.focus_new_windows)?;
        object.set("raiseOnClick", self.raise_on_click)?;
        object.set("autoRaise", self.auto_raise)?;
        object.set("autoRaiseDelay", self.auto_raise_delay)?;
        object.set("focusChangeOnPointerRest", self.focus_change_on_pointer_rest)?;
        object.set("reducedMotion", self.reduced_motion)?;
        Ok(object.into_value())
    }
}

/// Queues a write of the seat's keyboard focus: `target` is a node, or `NIL` for none. Refused when `seat` is not a
/// live seat node.
fn write_focus(state: &mut HostState, seat: NodeId, target: NodeId) -> Option<&'static str> {
    let HostState { mirror, tick, .. } = state;
    let is_seat = tick
        .node(mirror, seat)
        .is_some_and(|record| lua_class(&record.class) == "Seat");
    if !is_seat {
        return Some("unknown-node");
    }
    tick.set(mirror, seat, "focus", MirrorValue::Node(target), None, None)
        .err()
        .map(|_| "unknown-node")
}

/// Queues the seat's keyboard focus going to no node: the stock follow-mouse policy does it when the pointer leaves
/// the focused window for the desktop.
fn clear_focus(state: &mut HostState, seat: NodeId) -> Option<&'static str> {
    write_focus(state, seat, NIL)
}

/// Queues the first seat's keyboard focus going to `window`. The compositor answers no `focus` call on a window; the
/// focus is the seat's, and a shell gives it by writing the seat's `focus` (the Lua shell's `seat.focus = window`).
/// Refused with `no-seat` when the tree has no seat.
pub(crate) fn focus_window(state: &mut HostState, window: NodeId) -> Option<&'static str> {
    let Some(seat) = seat_nodes(state).first().copied() else {
        return Some("no-seat");
    };
    write_focus(state, seat, window)
}

fn devices(state: &HostState, seat: NodeId) -> Vec<String> {
    match seat_prop(state, seat, "device") {
        Some(MirrorValue::Texts(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// The text of a claim call: the subject, behind the device and a line feed when there is a device.
fn claim_text(subject: &str, device: &str) -> String {
    if device.is_empty() {
        subject.to_owned()
    } else {
        format!("{device}\n{subject}")
    }
}

/// The argument bytes of a claim: two little-endian `i64`s, as the compositor reads them.
fn claim_args(id: i64, second: i64) -> Vec<u8> {
    let mut args = Vec::with_capacity(16);
    args.extend_from_slice(&id.to_le_bytes());
    args.extend_from_slice(&second.to_le_bytes());
    args
}

/// Queues a call on `seat` and records the claim it makes: `second` is the claim's second integer (a chord's grab flag, a
/// gesture's finger count; a device claim has none) and `text` what the compositor reads as its text. Refused when the
/// seat is not a live seat node or the device name is not one line.
fn queue_claim(
    state: &mut HostState,
    seat: NodeId,
    method: &str,
    second: Option<i64>,
    text: String,
    device: &str,
) -> JsClaimOutcome {
    if device.contains('\n') {
        return JsClaimOutcome::Refused("invalid-value");
    }
    let HostState {
        mirror,
        tick,
        tokens,
        input,
        ..
    } = state;
    let is_seat = tick
        .node(mirror, seat)
        .is_some_and(|record| lua_class(&record.class) == "Seat");
    if !is_seat {
        return JsClaimOutcome::Refused("unknown-node");
    }
    let id = input.last_claim + 1;
    *tokens += 1;
    let args = match second {
        Some(second) => claim_args(id, second),
        None => id.to_le_bytes().to_vec(),
    };
    if tick
        .call(mirror, seat, method, args, *tokens, SecretText::new(text))
        .is_err()
    {
        return JsClaimOutcome::Refused("unknown-node");
    }
    input.last_claim = id;
    input.claims.insert(id, seat);
    JsClaimOutcome::Claimed(id)
}

/// Claims a chord. `device` is a name from `Seat.device`, or empty for every device.
pub(crate) fn claim(state: &mut HostState, seat: NodeId, chord: &str, grab: bool, device: &str) -> JsClaimOutcome {
    queue_claim(
        state,
        seat,
        "claim",
        Some(i64::from(grab)),
        claim_text(chord, device),
        device,
    )
}

/// Claims every gesture of `kind` (`swipe` or `pinch`) with `fingers` contacts.
pub(crate) fn claim_gesture(
    state: &mut HostState,
    seat: NodeId,
    kind: &str,
    fingers: i64,
    device: &str,
) -> JsClaimOutcome {
    queue_claim(
        state,
        seat,
        "claim_gesture",
        Some(fingers),
        claim_text(kind, device),
        device,
    )
}

/// Makes the devices that report `device` (a name from `Seat.device`) binding-only: their keys reach no client and fire
/// only the chords claimed for them or for every device, until the claim is given up. The call's text is the bare name.
pub(crate) fn claim_device(state: &mut HostState, seat: NodeId, device: &str) -> JsClaimOutcome {
    if device.is_empty() {
        return JsClaimOutcome::Refused("invalid-value");
    }
    queue_claim(state, seat, "claim_device", None, device.to_owned(), device)
}

/// Gives a claim up. An id this shell does not hold is ignored, so a double release cannot touch another claim.
pub(crate) fn unclaim(state: &mut HostState, id: i64) {
    let HostState {
        mirror,
        tick,
        tokens,
        input,
        ..
    } = state;
    let Some(seat) = input.claims.remove(&id) else {
        return;
    };
    *tokens += 1;
    // A seat that is gone took its claims with it, so a refused call here needs no report.
    let _gone = tick.call(
        mirror,
        seat,
        "unclaim",
        id.to_le_bytes().to_vec(),
        *tokens,
        SecretText::default(),
    );
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
        }
        list.set(index, item)?;
    }
    Ok(list)
}

/// Installs `__gnoblinHost.input`, the seat functions `gnoblin/input` runs on.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let input = Object::new(ctx.clone())?;

    let shared = state.clone();
    input.set(
        "seats",
        Function::new(ctx.clone(), move || -> Vec<f64> { seats(&shared.borrow()) })?,
    )?;

    let shared = state.clone();
    input.set(
        "seatName",
        Function::new(ctx.clone(), move |seat: f64| -> String {
            let state = shared.borrow();
            state
                .handles
                .resolve(seat)
                .map_or_else(String::new, |seat| seat_name(&state, seat))
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "pointer",
        Function::new(ctx.clone(), move |seat: f64| -> Vec<f64> {
            let state = shared.borrow();
            state
                .handles
                .resolve(seat)
                .map_or_else(|| vec![0.0, 0.0], |seat| pointer(&state, seat))
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "focus",
        Function::new(ctx.clone(), move |seat: f64| -> f64 {
            let state = shared.borrow();
            state.handles.resolve(seat).map_or(0.0, |seat| focus(&state, seat))
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "dragConstraint",
        Function::new(ctx.clone(), move |seat: f64| -> String {
            let state = shared.borrow();
            state
                .handles
                .resolve(seat)
                .map_or_else(|| "work_area".to_owned(), |seat| drag_constraint(&state, seat))
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "settings",
        Function::new(ctx.clone(), move |seat: f64| -> Option<JsSeatSettings> {
            let state = shared.borrow();
            Some(JsSeatSettings::of(&state, state.handles.resolve(seat)?))
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "clearFocus",
        Function::new(ctx.clone(), move |seat: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            let Some(seat) = state.handles.resolve(seat) else {
                return Some("unknown-node");
            };
            clear_focus(&mut state, seat)
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "devices",
        Function::new(ctx.clone(), move |seat: f64| -> Vec<String> {
            let state = shared.borrow();
            state
                .handles
                .resolve(seat)
                .map_or_else(Vec::new, |seat| devices(&state, seat))
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "claim",
        Function::new(
            ctx.clone(),
            move |seat: f64, chord: String, grab: bool, device: String| -> JsClaimOutcome {
                let mut state = shared.borrow_mut();
                match state.handles.resolve(seat) {
                    Some(seat) => claim(&mut state, seat, &chord, grab, &device),
                    None => JsClaimOutcome::Refused("unknown-node"),
                }
            },
        )?,
    )?;

    let shared = state.clone();
    input.set(
        "claimGesture",
        Function::new(
            ctx.clone(),
            move |seat: f64, kind: String, fingers: f64, device: String| -> JsClaimOutcome {
                let mut state = shared.borrow_mut();
                match state.handles.resolve(seat) {
                    Some(seat) => claim_gesture(&mut state, seat, &kind, fingers as i64, &device),
                    None => JsClaimOutcome::Refused("unknown-node"),
                }
            },
        )?,
    )?;

    let shared = state.clone();
    input.set(
        "claimDevice",
        Function::new(ctx.clone(), move |seat: f64, device: String| -> JsClaimOutcome {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(seat) {
                Some(seat) => claim_device(&mut state, seat, &device),
                None => JsClaimOutcome::Refused("unknown-node"),
            }
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "unclaim",
        Function::new(ctx.clone(), move |_seat: f64, id: f64| {
            unclaim(&mut shared.borrow_mut(), id as i64);
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "now",
        Function::new(ctx.clone(), move || -> f64 { shared.borrow().input.now_ms() })?,
    )?;

    host.set("input", input)
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{MirrorValue, SnapshotNode, TreeMirror, TxOp};

    use super::{
        JsClaimOutcome, JsSeatSettings, claim, claim_args, claim_device, claim_gesture, clear_focus, devices, focus,
        pointer, seat_name, seats, unclaim,
    };
    use crate::bindings::HostState;

    fn node(id: u64, parent: u64, class: &str, props: Vec<(String, MirrorValue)>) -> SnapshotNode {
        SnapshotNode {
            id,
            parent,
            index: 0,
            class: class.into(),
            title: String::new(),
            app_id: String::new(),
            props,
        }
    }

    fn state() -> HostState {
        let seat = vec![
            ("name".to_owned(), MirrorValue::Text("seat0".into())),
            ("pointer_x".to_owned(), MirrorValue::Double(12.5)),
            ("pointer_y".to_owned(), MirrorValue::Double(-3.0)),
            ("focus".to_owned(), MirrorValue::Node(3)),
            (
                "device".to_owned(),
                MirrorValue::Texts(vec!["1:Keeb: Pro:keyboard".into(), "2:Mouse:pointer".into()]),
            ),
        ];
        let nodes = [
            node(1, 0, "node", Vec::new()),
            node(2, 1, "workspace", Vec::new()),
            node(3, 2, "window", Vec::new()),
            node(4, 1, "seat", seat),
        ];
        HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"))
    }

    #[test]
    fn a_seat_is_found_and_read_from_the_mirror() {
        let state = state();
        assert_eq!(seats(&state), [4.0]);
        assert_eq!(seat_name(&state, 4), "seat0");
        assert_eq!(pointer(&state, 4), [12.5, -3.0]);
        assert_eq!(focus(&state, 4), 3.0);
        assert_eq!(devices(&state, 4).len(), 2);
        // A node that is not a seat answers as an absent one.
        assert_eq!(pointer(&state, 3), [0.0, 0.0]);
        assert_eq!(seat_name(&state, 99), "");
    }

    #[test]
    fn clearing_the_focus_queues_a_write_of_no_node_and_only_a_seat_takes_it() {
        let mut state = state();
        assert_eq!(
            clear_focus(&mut state, 3),
            Some("unknown-node"),
            "a window is not a seat"
        );
        assert_eq!(clear_focus(&mut state, 99), Some("unknown-node"));
        assert!(state.tick.ops().is_empty());
        assert_eq!(clear_focus(&mut state, 4), None);
        assert!(matches!(
            state.tick.ops(),
            [TxOp::Set { node: 4, key, value: MirrorValue::Node(0), .. }] if key == "focus"
        ));
    }

    #[test]
    fn seat_settings_are_the_schema_defaults_until_the_compositor_publishes_them() {
        let defaults = JsSeatSettings::of(&state(), 4);
        assert_eq!(
            defaults,
            JsSeatSettings {
                edge_tiling: false,
                focus_mode: "click".into(),
                focus_new_windows: "prevent".into(),
                raise_on_click: true,
                auto_raise: false,
                auto_raise_delay: 500.0,
                focus_change_on_pointer_rest: false,
                reduced_motion: false,
            }
        );
        let published = vec![
            ("edge_tiling".to_owned(), MirrorValue::Bool(true)),
            ("focus_mode".to_owned(), MirrorValue::Text("hover".into())),
            ("raise_on_click".to_owned(), MirrorValue::Bool(false)),
            ("auto_raise_delay".to_owned(), MirrorValue::Int(120)),
            ("reduced_motion".to_owned(), MirrorValue::Bool(true)),
        ];
        let nodes = [node(1, 0, "node", Vec::new()), node(4, 1, "seat", published)];
        let state = HostState::new(TreeMirror::from_snapshot(1, 1, 1, &nodes).expect("a mirror"));
        let settings = JsSeatSettings::of(&state, 4);
        assert!(settings.edge_tiling && !settings.raise_on_click);
        assert!(settings.reduced_motion, "the seat's reduced_motion is read");
        assert_eq!(settings.focus_mode, "hover");
        assert_eq!(settings.auto_raise_delay, 120.0);
        assert_eq!(
            settings.focus_new_windows, "prevent",
            "an unpublished key keeps its default"
        );
    }

    #[test]
    fn a_claim_is_a_call_with_little_endian_arguments_and_the_chord_as_its_text() {
        let mut state = state();
        assert_eq!(claim(&mut state, 4, "<Super>1", true, ""), JsClaimOutcome::Claimed(1));
        assert_eq!(
            claim(&mut state, 4, "<Super>Button1", false, "Macro Pad"),
            JsClaimOutcome::Claimed(2)
        );
        assert_eq!(claim_gesture(&mut state, 4, "swipe", 3, ""), JsClaimOutcome::Claimed(3));
        let calls: Vec<(&str, &[u8], String)> = state
            .tick
            .ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::Call { method, args, text, .. } => {
                    Some((method.as_str(), args.as_slice(), text.expose().to_owned()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(calls[0], ("claim", claim_args(1, 1).as_slice(), "<Super>1".to_owned()));
        assert_eq!(
            calls[1],
            (
                "claim",
                claim_args(2, 0).as_slice(),
                "Macro Pad\n<Super>Button1".to_owned()
            )
        );
        assert_eq!(
            calls[2],
            ("claim_gesture", claim_args(3, 3).as_slice(), "swipe".to_owned())
        );
        assert_eq!(state.input.claimed(), 3);
    }

    #[test]
    fn a_device_claim_is_a_call_with_the_id_and_the_bare_device_name_as_its_text() {
        // The Lua test "Seat:claim_device is a one-line device name" and the one of `bind{ only = true }`.
        let mut state = state();
        assert_eq!(claim_device(&mut state, 4, "Macro Pad"), JsClaimOutcome::Claimed(1));
        let calls: Vec<(&str, &[u8], String)> = state
            .tick
            .ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::Call { method, args, text, .. } => {
                    Some((method.as_str(), args.as_slice(), text.expose().to_owned()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            [("claim_device", 1_i64.to_le_bytes().as_slice(), "Macro Pad".to_owned())]
        );
        for bad in ["", "two\nlines"] {
            assert_eq!(
                claim_device(&mut state, 4, bad),
                JsClaimOutcome::Refused("invalid-value"),
                "{bad:?}"
            );
        }
        assert_eq!(
            claim_device(&mut state, 3, "Macro Pad"),
            JsClaimOutcome::Refused("unknown-node")
        );
        assert_eq!(state.input.claimed(), 1, "a refused claim holds nothing");
    }

    #[test]
    fn a_claim_on_anything_but_a_seat_is_refused_and_ids_are_not_reused() {
        let mut state = state();
        assert_eq!(
            claim(&mut state, 3, "<Super>1", false, ""),
            JsClaimOutcome::Refused("unknown-node")
        );
        assert_eq!(
            claim(&mut state, 99, "<Super>1", false, ""),
            JsClaimOutcome::Refused("unknown-node")
        );
        assert_eq!(
            claim(&mut state, 4, "<Super>1", false, "two\nlines"),
            JsClaimOutcome::Refused("invalid-value")
        );
        assert!(state.tick.ops().is_empty());
        assert_eq!(claim(&mut state, 4, "<Super>1", false, ""), JsClaimOutcome::Claimed(1));
        unclaim(&mut state, 1);
        assert_eq!(claim(&mut state, 4, "<Super>1", false, ""), JsClaimOutcome::Claimed(2));
    }

    #[test]
    fn unclaim_calls_once_and_ignores_an_id_it_does_not_hold() {
        let mut state = state();
        claim(&mut state, 4, "<Super>1", false, "");
        unclaim(&mut state, 1);
        unclaim(&mut state, 1);
        unclaim(&mut state, 77);
        let methods: Vec<&str> = state
            .tick
            .ops()
            .iter()
            .filter_map(|op| match op {
                TxOp::Call { method, .. } => Some(method.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(methods, ["claim", "unclaim"]);
        assert_eq!(state.input.claimed(), 0);
    }
}
