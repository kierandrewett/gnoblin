//! XML protocol `cursor_shape_v1` (`<protocol name="cursor_shape_v1">`); interfaces served: `wp_cursor_shape_manager_v1`.
//!
//! `wp_cursor_shape_manager_v1`: a client asks for a named cursor.
//!
//! Two protocols let a client set the cursor: `wl_pointer.set_cursor`, which ships a
//! buffer, and this one, which names a shape from a fixed list and leaves the picture to
//! the compositor. The second is what a toolkit prefers, because it gets the user's
//! cursor theme for free and ships nothing.
//!
//! Smithay validates the serial: a shape is only accepted with the serial of the
//! `wl_pointer.enter` the client last received, so a client cannot set the cursor while
//! the pointer is over someone else's window. What this module adds is the compositor's
//! record of the answer — the last named shape a client asked for — and the fact that
//! both cursor paths land in one place, [`SeatHandler::cursor_image`], so the renderer
//! that draws a cursor has one value to read rather than two sources to reconcile.
//!
//! # Failure modes
//!
//! 1. **A client sets the cursor for a window it does not own.** The serial check is
//!    Smithay's and happens before this module is reached, so a shape request that does
//!    not follow an enter for that surface is dropped and the recorded cursor is
//!    unchanged.
//! 2. **Two cursor sources fight.** Both `wl_pointer.set_cursor` and this protocol write
//!    the same [`smithay::input::pointer::CursorImageStatus`], so the last request wins
//!    by construction rather than by two code paths agreeing.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use tracing::debug;

use crate::State;

/// Creates the `wp_cursor_shape_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> CursorShapeManagerState {
    CursorShapeManagerState::new::<State>(display)
}

impl State {
    /// Records the cursor a client asked for.
    pub(crate) fn set_cursor_image(&mut self, image: smithay::input::pointer::CursorImageStatus) {
        debug!(
            shape = ?match &image {
                smithay::input::pointer::CursorImageStatus::Named(icon) => Some(*icon),
                _ => None,
            },
            "[cursor] a client set the pointer image"
        );
        self.cursor_image = image;
    }
}
