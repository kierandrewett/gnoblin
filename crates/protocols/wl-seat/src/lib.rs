//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_seat`.
//!
//! `wl_seat`: keyboard and pointer focus over surfaces.
//!
//! The seat owns focus; this module only states what a focus target is. A focus
//! target is a `WlSurface` rather than a window wrapper, because the window manager
//! already answers "which window", and a second identity between the two would have
//! to be kept in step with this one.
//!
//! Focus itself moves in the compositor's state, driven by the window manager's
//! `Effect::Focus`, so a protocol handler never decides who has focus.

use smithay::input::SeatHandler;
use smithay::input::touch::{
    DownEvent, GrabStartData as TouchStart, MotionEvent as TouchMotionEvent, OrientationEvent, ShapeEvent, TouchGrab,
    TouchInnerHandle, UpEvent,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, ServedInterface};

/// What this protocol advertises.
///
/// The seat is not made by a `register` call: the compositor makes the main seat with the session and one more for each
/// virtual input device that asks, through Smithay's `SeatState::new_wl_seat`, so the crate holds what is its own.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wl-seat",
    interfaces: &[ServedInterface::new("wl_seat", 9)],
    privilege: Privilege::Core,
};

/// The implicit grab a first touch point installs, which keeps each point with the surface it went down on.
///
/// Smithay's own `TouchDownGrab` sends every point that goes down while the first is held to the *first* point's
/// surface, as one gesture on one window. This session's touchscreen is one seat shared by every client: a second
/// finger on another client's window belongs to that window, and the first client must not see it (or learn where
/// on the other window it landed). Each point's focus is its own, decided when it goes down. A point's motion and
/// up follow its slot, which the seat keeps, so only `down` and `motion` differ from Smithay's grab. The grab ends
/// when the last point lifts; while it lasts it is what `xdg_toplevel.move` and `resize` match their serial against.
pub struct ImplicitTouchGrab<D: SeatHandler<TouchFocus = WlSurface> + 'static> {
    start_data: TouchStart<D>,
    points: usize,
}

impl<D: SeatHandler<TouchFocus = WlSurface> + 'static> ImplicitTouchGrab<D> {
    /// The grab a first touch point installs, with that point already down.
    #[must_use]
    pub const fn new(start_data: TouchStart<D>) -> Self {
        Self { start_data, points: 1 }
    }
}

impl<D: SeatHandler<TouchFocus = WlSurface> + 'static> TouchGrab<D> for ImplicitTouchGrab<D> {
    fn down(
        &mut self,
        data: &mut D,
        handle: &mut TouchInnerHandle<'_, D>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &DownEvent,
    ) {
        handle.down(data, focus, event);
        self.points += 1;
    }

    fn up(&mut self, data: &mut D, handle: &mut TouchInnerHandle<'_, D>, event: &UpEvent) {
        handle.up(data, event);
        self.points = self.points.saturating_sub(1);
        if self.points == 0 {
            handle.unset_grab(self, data);
        }
    }

    fn motion(
        &mut self,
        data: &mut D,
        handle: &mut TouchInnerHandle<'_, D>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &TouchMotionEvent,
    ) {
        handle.motion(data, focus, event);
    }

    fn frame(&mut self, data: &mut D, handle: &mut TouchInnerHandle<'_, D>) {
        handle.frame(data);
    }

    fn cancel(&mut self, data: &mut D, handle: &mut TouchInnerHandle<'_, D>) {
        handle.cancel(data);
        handle.unset_grab(self, data);
    }

    fn shape(&mut self, data: &mut D, handle: &mut TouchInnerHandle<'_, D>, event: &ShapeEvent) {
        handle.shape(data, event);
    }

    fn orientation(&mut self, data: &mut D, handle: &mut TouchInnerHandle<'_, D>, event: &OrientationEvent) {
        handle.orientation(data, event);
    }

    fn start_data(&self) -> &TouchStart<D> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut D) {}
}
