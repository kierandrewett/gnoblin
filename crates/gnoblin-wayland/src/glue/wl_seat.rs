//! `gnoblin-wayland-wl-seat`: the seat's handlers are the state's.

use smithay::input::pointer::CursorImageStatus;
use smithay::input::tablet::TabletSeatHandler;
use smithay::input::touch::{GrabStartData as TouchStart, TouchGrab};
use smithay::input::{SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use gnoblin_wayland_protocols::wl_seat::ImplicitTouchGrab;

use crate::State;

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
        ImplicitTouchGrab::new(start_data)
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
