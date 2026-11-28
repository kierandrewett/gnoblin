//! `window_drag`: presses on a frame's parts. A left press takes the window, and a drag moves it; a second press in quick succession, a
//! middle click and a right click run the seat's `double_click`, `middle_click` and `right_click` actions.

use std::rc::Rc;

use gnoblin_core::host::Value;
use gnoblin_object::Val;

use super::client::{Hook, Win};
use super::{DragState, FrameId, actions, en};
use crate::object::record::record;
use crate::ui::widget::Ui;

/// The buttons of the pointer, as the compositor numbers them (Linux `BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`).
pub(crate) const LEFT: u64 = 272;
pub(crate) const RIGHT: u64 = 273;
pub(crate) const MIDDLE: u64 = 274;

/// Two presses within this many milliseconds are a double click.
const DOUBLE_CLICK_MS: f64 = 500.0;

/// A `frame_press_event`.
pub(crate) fn press_event(x: f64, y: f64, button: u64) -> Val {
    record(vec![Val::Double(x), Val::Double(y), Val::Uint(button)])
}

impl Ui {
    fn drag_state(&self, drag: u64) -> Option<DragState> {
        self.frames.borrow().drags.get(&drag).cloned()
    }

    fn drag_edit(&self, drag: u64, change: impl FnOnce(&mut DragState)) {
        if let Some(state) = self.frames.borrow_mut().drags.get_mut(&drag) {
            change(state);
        }
    }

    /// Tells the decorations' `press` or `release` hook that a button went down or came up on the window's frame, outside its buttons.
    pub(crate) fn frame_button(&self, frame: FrameId, hook: Hook, x: f64, y: f64, button: u64) -> Result<(), String> {
        let (Some(node), Some(hook)) = (self.frame_node_alive(frame), self.hook(hook)) else {
            return Ok(());
        };
        hook.call(0, &[Win::of(self, node).record(), press_event(x, y, button)])
            .map(drop)
            .map_err(|fail| fail.message)
    }
}

/// A press: the window takes it. Always stops the event. Answers what a seat action failed with.
pub(crate) fn press(ui: &Rc<Ui>, drag: u64, x: f64, y: f64, button: u64) -> Result<(), String> {
    let Some(state) = ui.drag_state(drag) else {
        return Ok(());
    };
    let frame = state.frame;
    if button == LEFT {
        ui.frame_button(frame, Hook::Press, x, y, button)?;
    }
    if button == MIDDLE {
        return actions::gesture(ui, frame, en::GESTURE_MIDDLE_CLICK);
    }
    if button == RIGHT {
        return actions::gesture(ui, frame, en::GESTURE_RIGHT_CLICK);
    }
    if button != LEFT {
        return Ok(());
    }
    let now = ui.frame_now(frame);
    let previous = state.last_press;
    ui.drag_edit(drag, |held| held.last_press = Some(now));
    if previous.is_some_and(|earlier| now - earlier <= DOUBLE_CLICK_MS) {
        ui.drag_edit(drag, |held| held.last_press = None);
        return actions::gesture(ui, frame, en::GESTURE_DOUBLE_CLICK);
    }
    ui.drag_edit(drag, |held| held.from = Some((x, y)));
    Ok(())
}

/// A motion: moves the window while a left-button drag is going. Answers whether it was one, which stops the event.
pub(crate) fn motion(ui: &Rc<Ui>, drag: u64, x: f64, y: f64) -> bool {
    let Some(state) = ui.drag_state(drag) else {
        return false;
    };
    let Some((from_x, from_y)) = state.from else {
        return false;
    };
    let Some(node) = ui.frame_node_alive(state.frame) else {
        return true;
    };
    let win = Win::of(ui, node);
    for (key, by) in [("x", x - from_x), ("y", y - from_y)] {
        let target = ui.node(node);
        if let Err(error) = target.set(key, Value::Double(win.number(key) + by)) {
            ui.warn(&format!("the host refused {key} ({error})"));
        }
    }
    true
}

/// A release: ends the drag. Answers whether one was going, which stops the event.
pub(crate) fn release(ui: &Rc<Ui>, drag: u64, x: f64, y: f64, button: u64) -> Result<bool, String> {
    let Some(state) = ui.drag_state(drag) else {
        return Ok(false);
    };
    ui.frame_button(state.frame, Hook::Release, x, y, button)?;
    let dragging = state.from.is_some();
    if dragging {
        ui.drag_edit(drag, |held| held.from = None);
    }
    Ok(dragging)
}
