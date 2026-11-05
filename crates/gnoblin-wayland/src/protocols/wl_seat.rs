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
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

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
}

impl TabletSeatHandler for State {
    /// A tablet tool's focus target.
    ///
    /// It is a surface for the same reason the pointer's is: nothing between the seat and
    /// the protocol needs a second identity, and the cursor-shape device asks for this
    /// associated type whether or not a tablet is connected.
    type ToolFocus = WlSurface;
}
