//! XML protocol `ext_image_capture_source_v1` (`<protocol name="ext_image_capture_source_v1">`); interfaces served: `ext_image_capture_source_v1`.
//!
//! `ext_image_capture_source_v1`: the source objects a client names a capture by.
//!
//! An output source names an output and a toplevel source names a window; the source is
//! remembered under its object id so a session can resolve it later.

use smithay::output::Output as SmithayOutput;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler, OutputCaptureSourceState,
    ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
};

use gnoblin_capture::Target;
use gnoblin_types::WindowId;

use crate::State;

use crate::log;

/// The window a published toplevel identifier names.
///
/// The list's identifier is `gnoblin-` followed by the window's sixteen hex digits; a
/// handle that does not parse is one this session did not publish, which is refused rather
/// than guessed at.
fn window_of_identifier(identifier: &str) -> Option<WindowId> {
    let digits = identifier.strip_prefix("gnoblin-")?;
    u64::from_str_radix(digits, 16).ok().map(WindowId::new)
}

impl ImageCaptureSourceHandler for State {
    fn source_destroyed(&mut self, source: ImageCaptureSource) {
        self.capture.sources.remove(&source.id());
    }
}

impl OutputCaptureSourceHandler for State {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.capture.output_source
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &SmithayOutput) {
        let Some(id) = self.output_id_of(output) else {
            log::warn!(
                area = "capture",
                "a capture source names an output this session does not drive"
            );
            return;
        };
        self.capture.sources.insert(source.id(), Target::Output(id));
    }
}

impl ToplevelCaptureSourceHandler for State {
    fn toplevel_capture_source_state(&mut self) -> &mut ToplevelCaptureSourceState {
        &mut self.capture.toplevel_source
    }

    fn toplevel_source_created(&mut self, source: ImageCaptureSource, toplevel: ForeignToplevelHandle) {
        // The foreign-toplevel list publishes each window under an identifier derived from
        // its identity (`protocols::ext_foreign_toplevel_list_v1`), so a source names a window
        // without capture keeping a second table that could disagree with the shell's.
        let Some(window) = window_of_identifier(&toplevel.identifier()) else {
            log::warn!(area = "capture", identifier = %toplevel.identifier(),
                "a capture source names a toplevel this session does not publish"
            );
            return;
        };
        self.capture.sources.insert(source.id(), Target::Window(window));
    }
}
