//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_seat`.
//!
//! `wl_seat`: keyboard and pointer focus over surfaces.
//!
//! The seat owns focus; this module only states what a focus target is. A focus
//! target is a `WlSurface` rather than a window wrapper, because the window manager
//! already answers "which window", and a second identity between the two would have
//! to be kept in step with this one.
//!
//! Focus itself moves in [`crate::State`], driven by the window manager's
//! `Effect::Focus`, so a protocol handler never decides who has focus.

use smithay::input::SeatHandler;
use smithay::input::SeatState;
use smithay::input::pointer::CursorImageStatus;
use smithay::input::tablet::TabletSeatHandler;
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchStart, MotionEvent as TouchMotionEvent, OrientationEvent, ShapeEvent, TouchGrab,
    TouchInnerHandle, UpEvent,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};

use crate::State;

/// The implicit grab a first touch point installs, which keeps each point with the surface it went down on.
///
/// Smithay's own `TouchDownGrab` sends every point that goes down while the first is held to the *first* point's
/// surface, as one gesture on one window. This session's touchscreen is one seat shared by every client: a second
/// finger on another client's window belongs to that window, and the first client must not see it (or learn where
/// on the other window it landed). Each point's focus is its own, decided when it goes down. A point's motion and
/// up follow its slot, which the seat keeps, so only `down` and `motion` differ from Smithay's grab. The grab ends
/// when the last point lifts; while it lasts it is what `xdg_toplevel.move` and `resize` match their serial against.
struct ImplicitTouchGrab {
    start_data: TouchStart<State>,
    points: usize,
}

impl TouchGrab<State> for ImplicitTouchGrab {
    fn down(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &DownEvent,
    ) {
        handle.down(data, focus, event);
        self.points += 1;
    }

    fn up(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &UpEvent) {
        handle.up(data, event);
        self.points = self.points.saturating_sub(1);
        if self.points == 0 {
            handle.unset_grab(self, data);
        }
    }

    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &TouchMotionEvent,
    ) {
        handle.motion(data, focus, event);
    }

    fn frame(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.frame(data);
    }

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

    fn unset(&mut self, _data: &mut State) {}
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &smithay::input::Seat<Self>, image: CursorImageStatus) {
        // Both cursor protocols land here — a shipped buffer from
        // `wl_pointer.set_cursor` and a named shape from
        // `wp_cursor_shape_manager_v1` — so the renderer that draws a cursor has one value
        // to read.
        self.set_cursor_image(image);
    }

    fn touch_down_grab(&mut self, start_data: TouchStart<Self>) -> impl TouchGrab<Self> {
        ImplicitTouchGrab { start_data, points: 1 }
    }
}

impl TabletSeatHandler for State {
    /// A tablet tool's focus target.
    ///
    /// It is a surface for the same reason the pointer's is: nothing between the seat and
    /// the protocol needs a second identity, and the cursor-shape device asks for this
    /// associated type whether or not a tablet is connected.
    type ToolFocus = WlSurface;
}
