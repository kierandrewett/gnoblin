//! The window's actions: close with its fade, minimise, maximise and what the seat says a click does. The stock parts and a shell's own
//! canvases use the same ones.

use std::rc::Rc;

use gnoblin_core::Value;
use gnoblin_object::Val;
use gnoblin_std_abi as abi;

use super::client::{Hook, Win};
use super::{CloseFade, FrameId, en};
use crate::animation::engine::motion;
use crate::animation::staging::Prop;
use crate::ui::widget::Ui;

/// The fade on close: out over 0.18 s, as the window shrinks to 0.92 of its size.
pub(crate) const CLOSE_FADE_MS: f64 = 180.0;
/// A window still there this long after it was asked to close comes back, so a client that asks "save changes?" is not left faded out.
pub(crate) const CLOSE_RESTORE_MS: f64 = 1500.0;
/// The fade back in of a window that stayed open after it was asked to close, in milliseconds.
const RESTORE_DURATION_MS: f64 = 120.0;

/// The seat's property of a click, and what it does when the tree has no seat to ask: its schema default.
fn gesture_key(gesture: u32) -> (&'static str, &'static str) {
    match gesture {
        en::GESTURE_DOUBLE_CLICK => ("double_click", "toggle-maximize"),
        en::GESTURE_MIDDLE_CLICK => ("middle_click", "lower"),
        _ => ("right_click", "menu"),
    }
}

impl Ui {
    /// The clock a frame's actions are timed by, in milliseconds: the frame's own, the decorations', or the core's (see `Ui::core_now`).
    pub(crate) fn frame_now(&self, frame: FrameId) -> f64 {
        let clock = {
            let world = self.frames.borrow();
            world
                .frames
                .get(&frame)
                .and_then(|rec| rec.now.clone())
                .or_else(|| world.deco.now.clone())
        };
        self.clock_ms(clock.as_ref(), "the clock of a window frame")
    }

    /// Whether the window of the frame is fading out to be closed, or has been asked to close and has not gone yet.
    pub(crate) fn frame_closing(&self, frame: FrameId) -> bool {
        self.frames
            .borrow()
            .frames
            .get(&frame)
            .is_some_and(|rec| rec.closing.is_some())
    }

    fn set_closing(&self, frame: FrameId, closing: Option<CloseFade>) {
        if let Some(rec) = self.frames.borrow_mut().frames.get_mut(&frame) {
            rec.closing = closing;
        }
    }

    /// Asks the client of `node` to close the window. A window already gone is not an error.
    fn ask_to_close(&self, node: u64) {
        let _ = self.node(node).call("close", &[]);
    }
}

/// Moves the window's `opacity` and `scale` over `duration` milliseconds, as one transition: the fade of a closing window.
fn fade(ui: &Rc<Ui>, node: u64, opacity: f64, scale: [f64; 3], duration: f64, easing: u32) -> Result<(), String> {
    let std = crate::table::quiet_of(ui.ctx).map_err(|fail| fail.message)?;
    let options = abi::GnoblinStdTransitionOptions {
        size: size_of::<abi::GnoblinStdTransitionOptions>() as u32,
        autoplay: 1,
        easing,
        reserved: 0,
        duration_ms: duration,
        delay_ms: 0.0,
    };
    std.st
        .borrow_mut()
        .transition_begin(&options)
        .map_err(|fail| fail.message)?;
    let staged = {
        let mut st = std.st.borrow_mut();
        st.transition_set(node, "opacity", Prop::Number(opacity))
            .and_then(|()| st.transition_set(node, "scale", Prop::Triple(scale)))
    };
    if let Err(fail) = staged {
        std.st.borrow_mut().transition_abort();
        return Err(fail.message);
    }
    crate::animation::table::commit_staged(&std)
        .map(drop)
        .map_err(|fail| fail.message)
}

/// Starts the fade out of a window that is being closed, and closes it after the fade; a second call while it fades does nothing.
pub(crate) fn close(ui: &Rc<Ui>, frame: FrameId) -> Result<(), String> {
    let Some(node) = ui.frame_node_alive(frame) else {
        return Ok(());
    };
    if ui.frame_closing(frame) {
        return Ok(());
    }
    let length = motion(&ui.g, CLOSE_FADE_MS);
    fade(ui, node, 0.0, [0.92, 0.92, 1.0], length, abi::STD_EASING_EASE_IN)?;
    let now = ui.frame_now(frame);
    if length == 0.0 {
        // Reduced motion: nothing to wait for, so the window is asked to close now. It comes back if it stays.
        ui.set_closing(
            frame,
            Some(CloseFade {
                close_at: None,
                restore_at: Some(now + CLOSE_RESTORE_MS),
            }),
        );
        ui.ask_to_close(node);
    } else {
        ui.set_closing(
            frame,
            Some(CloseFade {
                close_at: Some(now + length),
                restore_at: None,
            }),
        );
    }
    ui.schedule();
    Ok(())
}

/// Takes the close of one frame on: closes the window after the fade, or brings it back if the client stayed open.
fn advance(ui: &Rc<Ui>, frame: FrameId) {
    let (node, closing) = {
        let world = ui.frames.borrow();
        let Some(rec) = world.frames.get(&frame) else {
            return;
        };
        (rec.node, rec.closing)
    };
    let Some(closing) = closing else {
        return;
    };
    if !Win::of(ui, node).alive() {
        ui.set_closing(frame, None);
        return;
    }
    let now = ui.frame_now(frame);
    if closing.close_at.is_some_and(|at| now >= at) {
        ui.set_closing(
            frame,
            Some(CloseFade {
                close_at: None,
                restore_at: Some(now + CLOSE_RESTORE_MS),
            }),
        );
        ui.ask_to_close(node);
    } else if closing.restore_at.is_some_and(|at| now >= at) {
        ui.set_closing(frame, None);
        if let Err(message) = fade(
            ui,
            node,
            1.0,
            [1.0, 1.0, 1.0],
            RESTORE_DURATION_MS,
            abi::STD_EASING_EASE_OUT,
        ) {
            ui.warn(&format!("a window that stayed open could not fade back in: {message}"));
        }
    }
}

/// Takes the close of every frame that is closing on.
pub(crate) fn advance_all(ui: &Rc<Ui>) {
    let closing: Vec<FrameId> = ui
        .frames
        .borrow()
        .frames
        .iter()
        .filter(|(_, rec)| rec.closing.is_some())
        .map(|(id, _)| *id)
        .collect();
    for frame in closing {
        advance(ui, frame);
    }
}

/// How long until a close is next due, in milliseconds, while one is going.
pub(crate) fn wake_in(ui: &Rc<Ui>) -> Option<u64> {
    let due: Vec<(FrameId, f64)> = ui
        .frames
        .borrow()
        .frames
        .iter()
        .filter_map(|(id, rec)| {
            rec.closing
                .and_then(|closing| closing.close_at.or(closing.restore_at))
                .map(|at| (*id, at))
        })
        .collect();
    due.into_iter()
        .map(|(frame, at)| (at - ui.frame_now(frame)).max(1.0).ceil() as u64)
        .min()
}

/// Maximises the window, or restores it, as the decorations' `maximise` hook does it, else by writing the flag.
pub(crate) fn toggle_maximise(ui: &Rc<Ui>, frame: FrameId) -> Result<(), String> {
    let Some(node) = ui.frame_node_alive(frame) else {
        return Ok(());
    };
    let on = !Win::of(ui, node).maximised();
    match ui.hook(Hook::Maximise) {
        Some(hook) => hook
            .call(0, &[Win::of(ui, node).record(), Val::Bool(on)])
            .map(drop)
            .map_err(|fail| fail.message),
        None => write_flag(ui, node, "maximised", on),
    }
}

/// Minimises the window as the decorations' `minimise` hook does it, else by writing the flag.
pub(crate) fn minimise(ui: &Rc<Ui>, frame: FrameId) -> Result<(), String> {
    let Some(node) = ui.frame_node_alive(frame) else {
        return Ok(());
    };
    match ui.hook(Hook::Minimise) {
        Some(hook) => hook
            .call(0, &[Win::of(ui, node).record()])
            .map(drop)
            .map_err(|fail| fail.message),
        None => write_flag(ui, node, "minimised", true),
    }
}

/// Opens the window menu where the pointer is, through the decorations' `menu` hook, else the library's own menu when its service has
/// started.
pub(crate) fn menu(ui: &Rc<Ui>, frame: FrameId) -> Result<(), String> {
    let Some(node) = ui.frame_node_alive(frame) else {
        return Ok(());
    };
    match ui.hook(Hook::Menu) {
        Some(hook) => hook
            .call(0, &[Win::of(ui, node).record()])
            .map(drop)
            .map_err(|fail| fail.message),
        None => {
            crate::menus::show_at_pointer(ui, node);
            Ok(())
        }
    }
}

/// Puts the window under the others, through the decorations' `lower` hook, else natively.
pub(crate) fn lower(ui: &Rc<Ui>, frame: FrameId) -> Result<(), String> {
    let Some(node) = ui.frame_node_alive(frame) else {
        return Ok(());
    };
    match ui.hook(Hook::Lower) {
        Some(hook) => hook
            .call(0, &[Win::of(ui, node).record()])
            .map(drop)
            .map_err(|fail| fail.message),
        None => {
            let root = ui.g.root().map_err(|error| error.to_string())?;
            ui.node(node)
                .insert_into(&root, Some(0))
                .map_err(|error| error.to_string())
        }
    }
}

fn write_flag(ui: &Ui, node: u64, key: &str, on: bool) -> Result<(), String> {
    ui.node(node)
        .set(key, Value::Bool(on))
        .map_err(|error| format!("the host refused {key} ({error})"))
}

/// Runs what the seat says the click `gesture` does. An action the schema does not name is an error.
pub(crate) fn gesture(ui: &Rc<Ui>, frame: FrameId, gesture: u32) -> Result<(), String> {
    let (property, default) = gesture_key(gesture);
    let action = ui.seat_action(property).unwrap_or_else(|| default.to_owned());
    match action.as_str() {
        "toggle-maximize" => toggle_maximise(ui, frame),
        "lower" => lower(ui, frame),
        "menu" => menu(ui, frame),
        "minimize" => minimise(ui, frame),
        // Maximising along one axis has no stock behaviour yet: it does nothing.
        "toggle-maximize-horizontally" | "toggle-maximize-vertically" | "none" => Ok(()),
        _ => Err(format!("{property} must name a titlebar action, got {action}")),
    }
}
