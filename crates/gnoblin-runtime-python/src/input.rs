//! Input claims: key chords, pointer chords, gestures, devices and grabs.
//!
//! Each function makes one claim on the seat through the core SDK. The compositor withholds a claimed chord from every client and
//! tells the shell instead; a reload hands a chord from the old generation to the new without the compositor ever seeing it
//! claimed twice, which is the core's and not something to repeat here.

use gnoblin_core::{ChordState, DragPhase, GestureKind, GrabScope, NodeId};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::callback::{callable, fire};
use crate::errors::core_error;
use crate::generation::{current_ctx, handle, keep_subscription};

const fn phase_name(phase: DragPhase) -> &'static str {
    match phase {
        DragPhase::Start => "start",
        DragPhase::Update => "update",
        DragPhase::End => "end",
        DragPhase::Cancel => "cancel",
    }
}

/// Keeps the claim `made` produced under a number.
fn kept(made: gnoblin_core::Result<gnoblin_core::Subscription>) -> PyResult<u64> {
    let subscription = made.map_err(|e| core_error(&e))?;
    Ok(keep_subscription(current_ctx(), subscription))
}

/// Calls `callback()` when the chord is pressed.
#[pyfunction]
pub(crate) fn bind(chord: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    kept(handle()?.bind(chord, move || {
        fire("bind", &callback, |_, f| f.call0());
    }))
}

/// Calls `callback(chord, state)` for a press, a release and each repeat of the chord (`state` is `press`, `release` or `repeat`).
#[pyfunction]
pub(crate) fn bind_events(chord: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    kept(handle()?.bind_events(chord, move |event| {
        let state = match event.state {
            ChordState::Press => "press",
            ChordState::Release => "release",
            ChordState::Repeat => "repeat",
        };
        fire("bind_events", &callback, |_, f| f.call1((event.chord.as_str(), state)));
    }))
}

/// Calls `callback(phase, x, y, dx, dy, button, modifiers, time)` while a pointer chord is held and dragged.
#[pyfunction]
pub(crate) fn bind_pointer(chord: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    kept(handle()?.bind_pointer(chord, move |drag| {
        fire("bind_pointer", &callback, |_, f| {
            f.call1((
                phase_name(drag.phase),
                drag.x,
                drag.y,
                drag.dx,
                drag.dy,
                drag.button,
                drag.modifiers.as_str(),
                drag.time,
            ))
        });
    }))
}

/// Calls `callback(kind, fingers, phase, dx, dy, scale, rotation)` for a touchpad gesture (`swipe` or `pinch`) with `fingers` fingers.
#[pyfunction]
pub(crate) fn bind_gesture(kind: &str, fingers: u8, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    let kind = match kind {
        "swipe" => GestureKind::Swipe,
        "pinch" => GestureKind::Pinch,
        other => {
            return Err(PyValueError::new_err(format!(
                "a gesture is `swipe` or `pinch`, not `{other}`"
            )));
        }
    };
    kept(handle()?.bind_gesture(kind, fingers, move |gesture| {
        fire("bind_gesture", &callback, |_, f| {
            f.call1((
                gesture.kind.as_str(),
                gesture.fingers,
                phase_name(gesture.phase),
                gesture.dx,
                gesture.dy,
                gesture.scale,
                gesture.rotation,
            ))
        });
    }))
}

/// Calls `callback(button, pressed, x, y, time)` for the buttons of the pointing device `device`.
#[pyfunction]
pub(crate) fn claim_pointer_buttons(device: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    kept(handle()?.claim_pointer_buttons(device, move |button| {
        fire("claim_pointer_buttons", &callback, |_, f| {
            f.call1((button.button, button.pressed, button.x, button.y, button.time))
        });
    }))
}

/// Takes the device `device` from the compositor's own input handling.
#[pyfunction]
pub(crate) fn claim_device(device: &str) -> PyResult<u64> {
    kept(handle()?.claim_device(device))
}

/// Sends the pointer, the keyboard or both (`scope`) to the node, until the claim is removed.
#[pyfunction]
pub(crate) fn grab_input(node: u64, scope: &str) -> PyResult<u64> {
    let scope = match scope {
        "pointer" => GrabScope::Pointer,
        "keyboard" => GrabScope::Keyboard,
        "both" => GrabScope::Both,
        other => {
            return Err(PyValueError::new_err(format!(
                "a grab is `pointer`, `keyboard` or `both`, not `{other}`"
            )));
        }
    };
    let g = handle()?;
    kept(g.grab_input(&g.node(NodeId(node)), scope))
}
