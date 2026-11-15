//! Installs the `ui` functions of the host bridge (`__gnoblinHost.ui`).

use rquickjs::{Ctx, Function, Object, Value};

use super::{
    JsUiAnswer, JsUiEvent, create, destroy, focus, focused, insert, locked, measure, mount_top, paint, screen,
    seat_number, seat_text, set_flag, set_number, set_text,
};
use crate::bindings::SharedState;

/// Installs `__gnoblinHost.ui`, the functions `gnoblin/ui` runs on.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, host: &Object<'js>, state: &SharedState) -> rquickjs::Result<()> {
    let ui = Object::new(ctx.clone())?;

    let shared = state.clone();
    ui.set(
        "create",
        Function::new(ctx.clone(), move || -> Option<f64> {
            let mut state = shared.borrow_mut();
            let id = create(&mut state)?;
            Some(state.handles.to_script(id))
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "insert",
        Function::new(
            ctx.clone(),
            move |node: f64, parent: f64, index: f64| -> Option<&'static str> {
                let mut state = shared.borrow_mut();
                let (Some(node), Some(parent)) = (state.handles.resolve(node), state.handles.resolve(parent)) else {
                    return Some("unknown-node");
                };
                insert(&mut state, node, parent, index as i64)
            },
        )?,
    )?;

    let shared = state.clone();
    ui.set(
        "destroy",
        Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(node) {
                Some(node) => destroy(&mut state, node),
                None => Some("unknown-node"),
            }
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "set",
        Function::new(
            ctx.clone(),
            move |node: f64, key: String, value: Value<'js>| -> Option<&'static str> {
                let mut state = shared.borrow_mut();
                let Some(node) = state.handles.resolve(node) else {
                    return Some("unknown-node");
                };
                if let Some(flag) = value.as_bool() {
                    set_flag(&mut state, node, &key, flag)
                } else if let Some(number) = value.as_number() {
                    set_number(&mut state, node, &key, number)
                } else if let Some(word) = value.as_string() {
                    match word.to_string() {
                        Ok(word) => set_text(&mut state, node, &key, word),
                        Err(_) => Some("invalid-value"),
                    }
                } else {
                    Some("invalid-value")
                }
            },
        )?,
    )?;

    let shared = state.clone();
    ui.set(
        "paint",
        Function::new(ctx.clone(), move |node: f64, bytes: Vec<u8>| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(node) {
                Some(node) => paint(&mut state, node, bytes),
                None => Some("unknown-node"),
            }
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "measure",
        Function::new(
            ctx.clone(),
            move |node: f64, font: String, size: f64, text: String| -> Option<f64> {
                let mut state = shared.borrow_mut();
                let node = state.handles.resolve(node)?;
                measure(&mut state, node, &font, size, &text).map(|token| token as f64)
            },
        )?,
    )?;

    let shared = state.clone();
    ui.set(
        "takeAnswers",
        Function::new(ctx.clone(), move || -> Vec<JsUiAnswer> {
            std::mem::take(&mut shared.borrow_mut().ui.answers)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "takeEvents",
        Function::new(ctx.clone(), move || -> Vec<JsUiEvent> {
            std::mem::take(&mut shared.borrow_mut().ui.events)
        })?,
    )?;

    // `focus(0)` takes the keyboard focus away.
    let shared = state.clone();
    ui.set(
        "focus",
        Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            let Some(node) = state.handles.resolve(node) else {
                return Some("unknown-node");
            };
            focus(&mut state, node)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "focused",
        Function::new(ctx.clone(), move || -> Option<f64> {
            let state = shared.borrow();
            focused(&state).map(|node| state.handles.to_script(node))
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "top",
        Function::new(ctx.clone(), move |node: f64| -> Option<&'static str> {
            let mut state = shared.borrow_mut();
            match state.handles.resolve(node) {
                Some(node) => mount_top(&mut state, node),
                None => Some("unknown-node"),
            }
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "seatNumber",
        Function::new(ctx.clone(), move |key: String| -> Option<f64> {
            seat_number(&shared.borrow(), &key)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "seatText",
        Function::new(ctx.clone(), move |key: String| -> Option<String> {
            seat_text(&shared.borrow(), &key)
        })?,
    )?;

    let shared = state.clone();
    ui.set(
        "locked",
        Function::new(ctx.clone(), move || -> bool { locked(&shared.borrow()) })?,
    )?;

    let shared = state.clone();
    ui.set(
        "screen",
        Function::new(ctx.clone(), move || -> Option<Vec<f64>> {
            screen(&shared.borrow()).map(|(width, height)| vec![width, height])
        })?,
    )?;

    host.set("ui", ui)
}
