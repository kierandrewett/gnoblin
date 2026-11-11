//! Interactive move and resize of a toplevel, run by the compositor when no shell holds the tree.
//!
//! `xdg_toplevel.move` and `xdg_toplevel.resize` ask the compositor to start a user gesture on the window. The
//! serial is the one of the button or touch down that began it. While a shell holds the tree the request is a `move` or
//! `resize` event on the window's node and the shell runs the drag (`State::request_to_shell`); this module is the
//! other path: the compositor's own record moves the window.
//!
//! A gesture is a Smithay grab on the device whose event the serial names. The grab takes the pointer or the touch point
//! away from the client for as long as it lasts, so the client sees `wl_pointer.leave` when the gesture starts and
//! an `enter` where the pointer ends up; every motion becomes a new rectangle for the window from where it started and how
//! far the device has moved, and the gesture ends when the last button or the touch point is released.
//!
//! A request is ignored, without a protocol error, when:
//!
//! - the serial names no grab on the main seat, or the grab started on another client's surface;
//! - the window is maximised, fullscreen or not managed;
//! - the session is locked.
//!
//! A move changes where the window is and sends the client nothing, because the protocol does not tell a client its
//! position. A resize sends a configure with the new size and the `resizing` state for as long as the gesture lasts; the
//! client draws the size it likes, and the window is positioned by the rectangle the gesture decided.

use smithay::input::Seat;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, GrabStartData as PointerStart, MotionEvent, PointerGrab, PointerInnerHandle,
    RelativeMotionEvent,
};
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchStart, MotionEvent as TouchMotionEvent, OrientationEvent, ShapeEvent, TouchGrab,
    TouchInnerHandle, UpEvent,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Serial};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::{SurfaceCachedState, ToplevelSurface};

use gnoblin_types::{Rect, WindowId};

use crate::State;
use crate::log;

/// The edges of a window a resize gesture drags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    /// The edges a protocol resize edge names; `None` for `none`, which drags nothing.
    pub(crate) const fn of(edge: xdg_toplevel::ResizeEdge) -> Option<Self> {
        let (left, right, top, bottom) = match edge {
            xdg_toplevel::ResizeEdge::Top => (false, false, true, false),
            xdg_toplevel::ResizeEdge::Bottom => (false, false, false, true),
            xdg_toplevel::ResizeEdge::Left => (true, false, false, false),
            xdg_toplevel::ResizeEdge::TopLeft => (true, false, true, false),
            xdg_toplevel::ResizeEdge::BottomLeft => (true, false, false, true),
            xdg_toplevel::ResizeEdge::Right => (false, true, false, false),
            xdg_toplevel::ResizeEdge::TopRight => (false, true, true, false),
            xdg_toplevel::ResizeEdge::BottomRight => (false, true, false, true),
            _ => return None,
        };
        Some(Self {
            left,
            right,
            top,
            bottom,
        })
    }
}

/// What a gesture does to the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DragKind {
    /// The window follows the device.
    Move,
    /// The window's edges follow the device.
    Resize(Edges),
}

/// The smallest and largest size a client allows its window, from `xdg_toplevel.set_min_size` and
/// `set_max_size`. A zero on an axis leaves it unconstrained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SizeLimits {
    min: (u32, u32),
    max: (u32, u32),
}

impl SizeLimits {
    /// The limits a surface's committed state holds.
    fn of(surface: &WlSurface) -> Self {
        with_states(surface, |states| {
            let mut cached = states.cached_state.get::<SurfaceCachedState>();
            let current = cached.current();
            let axis = |value: i32| u32::try_from(value).unwrap_or(0);
            Self {
                min: (axis(current.min_size.w), axis(current.min_size.h)),
                max: (axis(current.max_size.w), axis(current.max_size.h)),
            }
        })
    }

    /// `size` held between the limits, and at least one pixel.
    fn fit(self, width: i64, height: i64) -> (i64, i64) {
        let fit = |value: i64, min: u32, max: u32| {
            let upper = if max == 0 { i64::from(u32::MAX) } else { i64::from(max) };
            value.max(i64::from(min)).min(upper).max(1)
        };
        (fit(width, self.min.0, self.max.0), fit(height, self.min.1, self.max.1))
    }
}

/// The rectangle a gesture gives a window that started at `start`, once the device has moved by `delta`.
///
/// A move keeps the size. A resize moves the dragged edges by the delta; a size the limits refuse stops the dragged
/// edge where the limit is, and the edge opposite it stays where it was.
pub(crate) fn dragged(start: Rect, gesture: DragKind, delta: (i32, i32), limits: SizeLimits) -> Rect {
    let (dx, dy) = (i64::from(delta.0), i64::from(delta.1));
    let clamp = |value: i64| i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX });
    let (x, y) = (i64::from(start.x), i64::from(start.y));
    let (width, height) = (i64::from(start.width), i64::from(start.height));
    let DragKind::Resize(edges) = gesture else {
        return Rect::new(clamp(x + dx), clamp(y + dy), start.width, start.height);
    };
    let (mut left, mut right) = (x, x + width);
    let (mut top, mut bottom) = (y, y + height);
    if edges.left {
        left += dx;
    }
    if edges.right {
        right += dx;
    }
    if edges.top {
        top += dy;
    }
    if edges.bottom {
        bottom += dy;
    }
    let (width, height) = limits.fit(right - left, bottom - top);
    let left = if edges.left { right - width } else { left };
    let top = if edges.top { bottom - height } else { top };
    Rect::new(
        clamp(left),
        clamp(top),
        u32::try_from(width).unwrap_or(u32::MAX),
        u32::try_from(height).unwrap_or(u32::MAX),
    )
}

/// What a running gesture needs besides the device grab.
#[derive(Clone, Copy, Debug)]
struct Drag {
    window: WindowId,
    gesture: DragKind,
    /// The window's rectangle when the gesture began.
    start: Rect,
    limits: SizeLimits,
}

impl Drag {
    /// Applies a device that has moved by `delta` from where the gesture began. `false` when the window can no
    /// longer be dragged (it closed, or was maximised meanwhile), which ends the gesture.
    fn step(self, state: &mut State, delta: Point<f64, Logical>) -> bool {
        let delta = (delta.x.round() as i32, delta.y.round() as i32);
        state.drag_to(self.window, dragged(self.start, self.gesture, delta, self.limits))
    }
}

/// The pointer's grab for a gesture.
struct PointerDrag {
    start_data: PointerStart<State>,
    drag: Drag,
}

impl PointerGrab<State> for PointerDrag {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // No client has the pointer while its window is dragged.
        handle.motion(data, None, event);
        if !self.drag.step(data, event.location - self.start_data.location) {
            handle.unset_grab(self, data, event.serial, event.time, false);
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, event: &ButtonEvent) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, false);
        }
    }

    fn axis(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>, details: AxisFrame) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &PointerStart<State> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut State) {
        data.drag_ended(self.drag.window);
    }
}

/// The grab for a gesture a touch point began: the point's own events move the window, and no other point does.
struct TouchDrag {
    start_data: TouchStart<State>,
    drag: Drag,
}

impl TouchGrab<State> for TouchDrag {
    fn down(
        &mut self,
        _data: &mut State,
        _handle: &mut TouchInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        _event: &DownEvent,
    ) {
    }

    fn up(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &UpEvent) {
        if event.slot != self.start_data.slot {
            return;
        }
        handle.up(data, event);
        handle.unset_grab(self, data);
    }

    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &TouchMotionEvent,
    ) {
        if event.slot != self.start_data.slot {
            return;
        }
        if !self.drag.step(data, event.location - self.start_data.location) {
            handle.unset_grab(self, data);
        }
    }

    fn frame(&mut self, _data: &mut State, _handle: &mut TouchInnerHandle<'_, State>) {}

    fn cancel(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.cancel(data);
        handle.unset_grab(self, data);
    }

    fn shape(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &ShapeEvent) {
        handle.shape(data, event);
    }

    fn orientation(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &OrientationEvent) {
        handle.orientation(data, event);
    }

    fn start_data(&self) -> &TouchStart<State> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut State) {
        data.drag_ended(self.drag.window);
    }
}

impl State {
    /// Starts a move or resize gesture a client asked for, with no shell to run it.
    ///
    /// The request is honoured only with the serial of a grab on the main seat's pointer or touch that began on a
    /// surface of the same client; any other is ignored, as the protocol allows.
    pub(crate) fn begin_interactive(
        &mut self,
        surface: &ToplevelSurface,
        seat: &WlSeat,
        serial: Serial,
        gesture: DragKind,
    ) {
        if self.lock.state() != gnoblin_lock::State::Unlocked || self.composer.shell_places() {
            return;
        }
        if !Seat::<Self>::from_resource(seat).is_some_and(|seat| seat == self.seat) {
            log::debug!(area = "xdg", "an interactive request for another seat was ignored");
            return;
        }
        let Some(window) = self.window_of(surface.wl_surface()) else {
            return;
        };
        let Some(start) = self
            .fallback
            .window(window)
            .filter(|managed| !managed.state.maximised && !managed.state.fullscreen)
            .map(|managed| managed.geometry)
        else {
            log::debug!(area = "xdg", %window, "an interactive request for a window that is not floating was ignored");
            return;
        };
        let drag = Drag {
            window,
            gesture,
            start,
            limits: SizeLimits::of(surface.wl_surface()),
        };
        let pointer = self.pointer.clone();
        let touch = self.touch.clone();
        let same_client = |focus: Option<&WlSurface>| {
            focus.is_some_and(|focus| focus.id().same_client_as(&surface.wl_surface().id()))
        };
        if pointer.has_grab(serial) {
            let Some(start_data) = pointer.grab_start_data() else {
                return;
            };
            if !same_client(start_data.focus.as_ref().map(|(focus, _)| focus)) {
                log::debug!(area = "xdg", %window, "an interactive request from another client's click was ignored");
                return;
            }
            pointer.set_grab(self, PointerDrag { start_data, drag }, serial, Focus::Clear);
            pointer.frame(self);
        } else if touch.has_grab(serial) {
            let Some(start_data) = touch.grab_start_data() else {
                return;
            };
            if !same_client(start_data.focus.as_ref().map(|(focus, _)| focus)) {
                log::debug!(area = "xdg", %window, "an interactive request from another client's touch was ignored");
                return;
            }
            touch.set_grab(self, TouchDrag { start_data, drag }, serial);
        } else {
            log::debug!(area = "xdg", %window, "an interactive request with a serial that names no grab was ignored");
            return;
        }
        self.dragged = Some(window);
        if matches!(gesture, DragKind::Resize(_)) {
            surface.with_pending_state(|pending| {
                pending.states.set(xdg_toplevel::State::Resizing);
            });
            if surface.is_initial_configure_sent() {
                surface.send_configure();
            }
        }
        log::debug!(area = "xdg", %window, ?gesture, "an interactive gesture began");
    }

    /// Puts a dragged window at the rectangle the gesture decided. `false` when the window is gone or no longer
    /// floating.
    ///
    /// The effects take the path every placement does, so the client is configured when its size changed, and a move
    /// sends it nothing: a configure goes out only when its state differs from the last one.
    fn drag_to(&mut self, window: WindowId, wanted: Rect) -> bool {
        let Ok(effects) = self.fallback.set_rect(window, wanted) else {
            return false;
        };
        self.apply(effects);
        self.publish();
        true
    }

    /// A gesture ended, because its device was released or its window went away: the window stops being resized.
    fn drag_ended(&mut self, window: WindowId) {
        self.dragged = None;
        let Some(toplevel) = self.toplevel_of(window) else {
            return;
        };
        let was_resizing = toplevel.with_pending_state(|pending| pending.states.unset(xdg_toplevel::State::Resizing));
        if was_resizing && toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
        }
        log::debug!(area = "xdg", %window, "an interactive gesture ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Rect = Rect::new(100, 100, 420, 390);

    fn resize(edge: xdg_toplevel::ResizeEdge) -> DragKind {
        DragKind::Resize(Edges::of(edge).expect("an edge"))
    }

    #[test]
    fn a_move_follows_the_device_and_keeps_the_size() {
        assert_eq!(
            dragged(START, DragKind::Move, (60, -40), SizeLimits::default()),
            Rect::new(160, 60, 420, 390)
        );
    }

    #[test]
    fn dragging_the_top_left_corner_moves_the_origin_and_changes_the_size() {
        assert_eq!(
            dragged(
                START,
                resize(xdg_toplevel::ResizeEdge::TopLeft),
                (60, -40),
                SizeLimits::default()
            ),
            Rect::new(160, 60, 360, 430)
        );
    }

    #[test]
    fn dragging_the_bottom_right_corner_keeps_the_origin() {
        assert_eq!(
            dragged(
                START,
                resize(xdg_toplevel::ResizeEdge::BottomRight),
                (-20, 10),
                SizeLimits::default()
            ),
            Rect::new(100, 100, 400, 400)
        );
    }

    #[test]
    fn a_single_edge_leaves_the_other_axis_alone() {
        assert_eq!(
            dragged(
                START,
                resize(xdg_toplevel::ResizeEdge::Right),
                (30, 99),
                SizeLimits::default()
            ),
            Rect::new(100, 100, 450, 390)
        );
        assert_eq!(
            dragged(
                START,
                resize(xdg_toplevel::ResizeEdge::Top),
                (99, -30),
                SizeLimits::default()
            ),
            Rect::new(100, 70, 420, 420)
        );
    }

    #[test]
    fn a_size_below_the_minimum_stops_the_dragged_edge_and_keeps_the_far_one() {
        let limits = SizeLimits {
            min: (200, 100),
            max: (0, 0),
        };
        // Dragging the left edge 400 px right would leave 20 px; the window stops at 200 with its right edge fixed.
        assert_eq!(
            dragged(START, resize(xdg_toplevel::ResizeEdge::Left), (400, 0), limits),
            Rect::new(320, 100, 200, 390)
        );
    }

    #[test]
    fn a_size_above_the_maximum_stops_the_dragged_edge() {
        let limits = SizeLimits {
            min: (0, 0),
            max: (500, 0),
        };
        assert_eq!(
            dragged(START, resize(xdg_toplevel::ResizeEdge::Right), (300, 0), limits),
            Rect::new(100, 100, 500, 390)
        );
    }

    #[test]
    fn a_window_is_never_resized_to_nothing() {
        assert_eq!(
            dragged(
                START,
                resize(xdg_toplevel::ResizeEdge::BottomRight),
                (-1000, -1000),
                SizeLimits::default()
            ),
            Rect::new(100, 100, 1, 1)
        );
    }

    #[test]
    fn the_none_edge_names_no_gesture() {
        assert_eq!(Edges::of(xdg_toplevel::ResizeEdge::None), None);
    }
}
