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
use std::time::Instant;

use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NodeId, TreeChord, TreeGesture, lua_class};
use rquickjs::{Array, Ctx, Function, IntoJs, Object, Value};

use crate::bindings::{HostState, SharedState, parents_first};

/// A claimed chord or gesture the compositor reported, parked until the frame hands it to the module.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsInputMessage {
    Chord(TreeChord),
    Gesture(TreeGesture),
}

/// Everything input keeps on the host between frames.
pub(crate) struct JsInput {
    /// The last claim id handed out.
    last_claim: i64,
    /// The seat each live claim was made on, so `unclaim` finds the node and a stale id is a no-op.
    claims: HashMap<i64, NodeId>,
    /// Chords and gestures that arrived since the last frame's dispatch, oldest first.
    pub inbox: Vec<JsInputMessage>,
    /// The origin of `now`.
    epoch: Instant,
}

impl JsInput {
    pub(crate) fn new() -> Self {
        Self {
            last_claim: 0,
            claims: HashMap::new(),
            inbox: Vec::new(),
            epoch: Instant::now(),
        }
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
fn seats(state: &HostState) -> Vec<f64> {
    parents_first(&state.mirror)
        .into_iter()
        .filter(|id| {
            state
                .mirror
                .node(*id)
                .is_some_and(|record| lua_class(&record.class) == "Seat")
        })
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

/// Queues a call on `seat` and records the claim it makes. Refused when the seat is not a live seat node or
/// the device name is not one line.
fn queue_claim(
    state: &mut HostState,
    seat: NodeId,
    method: &str,
    second: i64,
    subject: &str,
    device: &str,
) -> JsClaimOutcome {
    if device.contains('\n') {
        return JsClaimOutcome::Refused("invalid-value");
    }
    let text = claim_text(subject, device);
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
    if tick
        .call(
            mirror,
            seat,
            method,
            claim_args(id, second),
            *tokens,
            SecretText::new(text),
        )
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
    queue_claim(state, seat, "claim", i64::from(grab), chord, device)
}

/// Claims every gesture of `kind` (`swipe` or `pinch`) with `fingers` contacts.
pub(crate) fn claim_gesture(
    state: &mut HostState,
    seat: NodeId,
    kind: &str,
    fingers: i64,
    device: &str,
) -> JsClaimOutcome {
    queue_claim(state, seat, "claim_gesture", fingers, kind, device)
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

/// The frame's chord and gesture messages as script objects, for the module's `input` entry.
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
        "unclaim",
        Function::new(ctx.clone(), move |_seat: f64, id: f64| {
            unclaim(&mut shared.borrow_mut(), id as i64);
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "now",
        Function::new(ctx.clone(), move || -> f64 {
            shared.borrow().input.epoch.elapsed().as_secs_f64() * 1000.0
        })?,
    )?;

    host.set("input", input)
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{MirrorValue, SnapshotNode, TreeMirror, TxOp};

    use super::{JsClaimOutcome, claim, claim_args, claim_gesture, devices, focus, pointer, seat_name, seats, unclaim};
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
