//! The frame's presentation answer: which surfaces a frame drew, and the time it reached the output.
//!
//! `wp_presentation` itself is `gnoblin-wayland-wp-presentation-time`. What belongs here is the part
//! that reads the compositor: the windows a frame covers and the `presented` stamp the shell asks for.

use gnoblin_types::OutputId;
use gnoblin_wayland_protocols::wp_presentation_time::{PresentationExt as _, refresh_of};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl State {
    /// Answers the feedback of every surface this frame put on an output, and stamps `presented` on the nodes of the
    /// windows whose shell asked for it (`report_presented`).
    ///
    /// The surfaces are the mapped windows whose geometry covers part of the output: a
    /// window that is hidden, minimised or on another output was not drawn, so a feedback
    /// it asked for stays pending, which is what the protocol wants.
    pub(crate) fn present_frame(&mut self, output: OutputId) {
        let Some((smithay_output, rect)) = self.frame_target(output) else {
            return;
        };
        let drawn = self.drawn_windows(rect);
        let surfaces: Vec<WlSurface> = drawn.iter().map(|(_, surface)| surface.clone()).collect();
        let refresh = refresh_of(&smithay_output);
        let Some(time) = self.present_feedback(&smithay_output, &surfaces, refresh) else {
            return;
        };
        let millis = u64::try_from(time.as_millis()).unwrap_or(u64::MAX);
        for (window, _) in &drawn {
            self.composer.stamp_presented(*window, millis);
        }
    }
}
