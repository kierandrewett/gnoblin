//! Seat claims: the chords, gestures, devices, pointer buttons and input grabs a script asks the compositor to hand it, queued as calls
//! on the seat and kept apart while a generation is only being staged.

use gnoblin_secret::SecretText;
use gnoblin_tree_client::tree_mirror::{NodeId, class_name};
use rquickjs::{Ctx, IntoJs, Value};

use crate::bindings::HostState;

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

/// The text of a claim call: the subject, behind the device and a line feed when there is a device.
fn claim_text(subject: &str, device: &str) -> String {
    if device.is_empty() {
        subject.to_owned()
    } else {
        format!("{device}\n{subject}")
    }
}

/// The argument bytes of a claim: two little-endian `i64`s, as the compositor reads them.
pub(super) fn claim_args(id: i64, second: i64) -> Vec<u8> {
    let mut args = Vec::with_capacity(16);
    args.extend_from_slice(&id.to_le_bytes());
    args.extend_from_slice(&second.to_le_bytes());
    args
}

/// A claim made while a generation was being staged, kept until that generation takes over.
pub(crate) struct StagedClaim {
    pub(super) id: i64,
    pub(super) seat: NodeId,
    pub(super) method: &'static str,
    pub(super) second: Option<i64>,
    pub(super) text: String,
}

/// Sends a claim as a call on its seat and records it as held. Fails when the seat is not a node of the tree (a seat
/// that is gone took its claims with it).
fn send_claim(state: &mut HostState, claim: StagedClaim) -> bool {
    let HostState {
        mirror,
        tick,
        tokens,
        input,
        ..
    } = state;
    *tokens += 1;
    let args = match claim.second {
        Some(second) => claim_args(claim.id, second),
        None => claim.id.to_le_bytes().to_vec(),
    };
    if tick
        .call(
            mirror,
            claim.seat,
            claim.method,
            args,
            *tokens,
            SecretText::new(claim.text),
        )
        .is_err()
    {
        return false;
    }
    input.claims.insert(claim.id, claim.seat);
    true
}

/// Makes a claim on `seat` and records it: `second` is the claim's second integer (a chord's grab flag, a gesture's
/// finger count; a device claim has none) and `text` what the compositor reads as its text. The call is queued like any
/// write, or kept until the generation being staged takes over (see the module's "Hand-off"). Refused when the seat is
/// not a live seat node or the device name is not one line.
fn queue_claim(
    state: &mut HostState,
    seat: NodeId,
    method: &'static str,
    second: Option<i64>,
    text: String,
    device: &str,
) -> JsClaimOutcome {
    if device.contains('\n') {
        return JsClaimOutcome::Refused("invalid-value");
    }
    let HostState {
        mirror, tick, input, ..
    } = state;
    let is_seat = tick
        .node(mirror, seat)
        .is_some_and(|record| class_name(&record.class) == "Seat");
    if !is_seat {
        return JsClaimOutcome::Refused("unknown-node");
    }
    let id = input.last_claim + 1;
    let claim = StagedClaim {
        id,
        seat,
        method,
        second,
        text,
    };
    if input.staging {
        input.last_claim = id;
        input.staged.push(claim);
        return JsClaimOutcome::Claimed(id);
    }
    if !send_claim(state, claim) {
        return JsClaimOutcome::Refused("unknown-node");
    }
    state.input.last_claim = id;
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

/// Claims every pointer button of the pointers that report `device` (a name from `Seat.device`, never empty): a press
/// reaches the script as a `button` message instead of a client or a node, until the claim is given up. The call's text
/// is the bare name.
pub(crate) fn claim_pointer(state: &mut HostState, seat: NodeId, device: &str) -> JsClaimOutcome {
    if device.is_empty() {
        return JsClaimOutcome::Refused("invalid-value");
    }
    queue_claim(state, seat, "claim_pointer", None, device.to_owned(), device)
}

/// Takes the seat's `scope` (`pointer`, `keyboard` or `both`) for the node `node`, which then hears the input as it
/// would a node of its own: the keys as `key` events, the buttons, motion and wheel as pointer events. The grab is a
/// claim like the others, so the generation that made it gives it up when it ends and a staged generation's waits for
/// the takeover. The compositor holds one grab at a time and answers a refusal by leaving `Seat.grabbed` as it was.
pub(crate) fn grab(state: &mut HostState, seat: NodeId, node: NodeId, scope: &str) -> JsClaimOutcome {
    if !matches!(scope, "pointer" | "keyboard" | "both") {
        return JsClaimOutcome::Refused("invalid-value");
    }
    queue_claim(state, seat, "grab", Some(node as i64), scope.to_owned(), "")
}

/// Gives a claim up. An id this shell does not hold is ignored, so a double release cannot touch another claim. A claim
/// the staged generation made and has not sent is dropped without a call.
pub(crate) fn unclaim(state: &mut HostState, id: i64) {
    let HostState {
        mirror,
        tick,
        tokens,
        input,
        ..
    } = state;
    if let Some(at) = input.staged.iter().position(|claim| claim.id == id) {
        input.staged.remove(at);
        return;
    }
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

/// A generation is about to be staged: what it claims is kept, not sent, until it takes over.
pub(crate) fn begin_staging(state: &mut HostState) {
    state.input.staging = true;
    state.input.staged.clear();
}

/// The stage failed: the claims it kept are dropped, and the running generation's are as they were.
pub(crate) fn abandon_staging(state: &mut HostState) {
    state.input.staging = false;
    state.input.staged.clear();
}

/// The staged generation takes over: claims are sent as they are made again, and the ones it kept are handed back to
/// be sent by [`send_staged`] once the old generation's have been released.
pub(crate) fn end_staging(state: &mut HostState) -> Vec<StagedClaim> {
    state.input.staging = false;
    std::mem::take(&mut state.input.staged)
}

/// Gives up every claim still held, in the order they were made. The old generation gives its own up when it ends; this
/// is for one whose script threw before it had, which would otherwise keep its chords from the new generation.
pub(crate) fn release_all(state: &mut HostState) {
    let mut ids: Vec<i64> = state.input.claims.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        unclaim(state, id);
    }
}

/// Sends the claims a staged generation kept, in the order it made them. A claim whose seat is gone is dropped: the
/// script that holds its id releases it later, which then finds nothing.
pub(crate) fn send_staged(state: &mut HostState, staged: Vec<StagedClaim>) {
    for claim in staged {
        send_claim(state, claim);
    }
}
