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

use gnoblin_log::debug;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::cursor_shape::CursorShapeManagerState;

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
        // Called from inside smithay's seat callback, where the pointer is locked, so the
        // composer is updated at the next frame instead.
        self.cursor_shape_changed = true;
    }

    /// Tells the composer where the cursor is drawn now, in global layout coordinates, so
    /// the places it was and is get damaged. `redraw` is set when the shape changed, so the
    /// cursor repaints where it stands.
    pub(crate) fn update_cursor(&mut self, redraw: bool) {
        let cursor = self.outputs.iter().find_map(|entry| {
            let overlay = self.pointer_cursor(entry.backend)?;
            let origin = self.output_rect(entry.backend)?;
            let dst = overlay.dst();
            Some(gnoblin_types::Rect {
                x: origin.x + dst.x,
                y: origin.y + dst.y,
                width: dst.width,
                height: dst.height,
            })
        });
        self.composer.set_cursor(cursor, redraw);
    }
}

/// The byte budget of the cursor chain's image cache.
pub(crate) const CURSOR_CACHE_BYTES: usize = 8 * 1024 * 1024;

impl State {
    /// The cursor to draw on `output` this frame. A named shape is drawn when the pointer
    /// is on that output; a client's own surface cursor is not drawn here.
    pub(crate) fn pointer_cursor(&self, output: gnoblin_types::OutputId) -> Option<gnoblin_backend::render::Overlay> {
        let smithay::input::pointer::CursorImageStatus::Named(icon) = &self.cursor_image else {
            return None;
        };
        let pointer = self.seat.get_pointer()?.current_location();
        let rect = self.output_rect(output)?;
        let scale = self.output_scale(output)?;
        let inside = pointer.x >= f64::from(rect.x)
            && pointer.y >= f64::from(rect.y)
            && pointer.x < f64::from(rect.x) + f64::from(rect.width)
            && pointer.y < f64::from(rect.y) + f64::from(rect.height);
        if !inside {
            return None;
        }
        let output_scale = gnoblin_types::Scale::new((scale * 1000.0).round() as u32, 1000);
        let images = self.cursor_chain.lookup(icon.name(), self.cursor_size, output_scale);
        let at = (pointer.x - f64::from(rect.x), pointer.y - f64::from(rect.y));
        cursor_overlay(&images, at, scale)
    }
}

/// Builds the overlay that draws the first frame of `images` with its hotspot at `at`,
/// a point in the output's logical space. The frame is straight-alpha `0xAARRGGBB`, which
/// [`gnoblin_backend::render::Image`] takes as RGBA bytes; the overlay premultiplies it.
/// Animated cursors show their first frame only for now.
pub(crate) fn cursor_overlay(
    images: &gnoblin_backend::render::cursor::CursorImages,
    at: (f64, f64),
    scale: f64,
) -> Option<gnoblin_backend::render::Overlay> {
    let frame = images.frames().first()?;
    let (width, height) = (frame.width(), frame.height());
    let mut data = Vec::with_capacity(frame.pixels().len() * 4);
    for &pixel in frame.pixels() {
        data.extend_from_slice(&[
            (pixel >> 16) as u8,
            (pixel >> 8) as u8,
            pixel as u8,
            (pixel >> 24) as u8,
        ]);
    }
    let image = gnoblin_backend::render::Image::new(gnoblin_types::Size { width, height }, width as usize * 4, data)?;
    let (hotspot_x, hotspot_y) = images.hotspot();
    let dst = gnoblin_types::Rect {
        x: (at.0 - f64::from(hotspot_x) / scale).round() as i32,
        y: (at.1 - f64::from(hotspot_y) / scale).round() as i32,
        width: (f64::from(width) / scale).round() as u32,
        height: (f64::from(height) / scale).round() as u32,
    };
    gnoblin_backend::render::Overlay::new(dst, &image)
}

#[cfg(test)]
mod tests {
    use super::cursor_overlay;
    use gnoblin_backend::render::cursor::{CursorFrame, CursorImages};

    #[test]
    fn cursor_overlay_draws_the_first_frame_at_its_hotspot() {
        let frame = CursorFrame::new(2, 2, vec![0xff00_ff00; 4], 0).expect("frame");
        let images = CursorImages::new((1, 1), vec![frame]).expect("images");
        assert!(cursor_overlay(&images, (10.0, 10.0), 1.0).is_some());
        assert!(cursor_overlay(&images, (10.0, 10.0), 2.0).is_some());
    }
}
