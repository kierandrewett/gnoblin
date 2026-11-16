//! Installs the `input` functions of the host bridge (`__gnoblinHost.input`).

use rquickjs::{Ctx, Function, Object};

use super::claims::{JsClaimOutcome, claim, claim_device, claim_gesture, unclaim};
use super::seats::{
    JsSeatSettings, clear_focus, declare_decorations, devices, drag_constraint, focus, pointer, seat_name, seats,
    set_builtin_motion,
};
use crate::bindings::SharedState;

/// Installs `__gnoblinHost.input`, the seat functions `gnoblin-core/input` runs on.
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
        "declareDecorations",
        Function::new(ctx.clone(), move |seat: f64, available: bool| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            let Some(seat) = state.handles.resolve(seat) else {
                return Some("unknown-node");
            };
            declare_decorations(&mut state, seat, available)
        })?,
    )?;

    let shared = state.clone();
    input.set(
        "setBuiltinMotion",
        Function::new(ctx.clone(), move |seat: f64, enabled: bool| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            let Some(seat) = state.handles.resolve(seat) else {
                return Some("unknown-node");
            };
            set_builtin_motion(&mut state, seat, enabled)
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
