//! `gnoblin-wayland-ext-image-capture-source-v1`: what each source object names is recorded in the capture tables.

use gnoblin_capture::Target;
use gnoblin_wayland_protocols::ext_image_capture_source_v1::window_of_identifier;
use smithay::output::Output as SmithayOutput;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler, OutputCaptureSourceState,
    ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
};

use crate::State;
use crate::log;

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
        // The foreign-toplevel list publishes each window under an identifier derived from its identity, so a source
        // names a window without capture keeping a second table that could disagree with the shell's.
        let Some(window) = window_of_identifier(&toplevel.identifier()) else {
            log::warn!(area = "capture", identifier = %toplevel.identifier(),
                "a capture source names a toplevel this session does not publish"
            );
            return;
        };
        self.capture.sources.insert(source.id(), Target::Window(window));
    }
}
