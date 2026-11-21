//! `gnoblin-wayland-zwlr-screencopy-v1`: the frame cache, the session rules and the queue of GPU copies are the
//! capture engine's (`protocols/capture_shared.rs`).

use std::collections::BTreeSet;

use gnoblin_types::{ClientId, OutputId, Rect, Tick};
use gnoblin_wayland_protocols::zwlr_screencopy_v1::{Host, PendingCopy, ShmCopy};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;
use smithay::reexports::wayland_server::Client;

use crate::State;

impl Host for State {
    fn capture_owner(&mut self, client: &Client) -> ClientId {
        self.capture.owner(&client.id())
    }

    fn capture_allowed(&self) -> bool {
        State::capture_allowed(self)
    }

    fn output_capture_bounds(&self, output: OutputId) -> Option<Rect> {
        self.capture_bounds(gnoblin_capture::Target::Output(output))
    }

    fn track_screencopy_frame(&mut self, frame: ZwlrScreencopyFrameV1) {
        self.capture.screencopy_frames.push(frame);
    }

    fn forget_screencopy_frame(&mut self, frame: &ZwlrScreencopyFrameV1) {
        self.capture.screencopy_frames.retain(|held| held != frame);
    }

    fn resume_capture_cache(&mut self) {
        State::resume_capture_cache(self);
    }

    fn flush(&mut self) {
        State::flush(self);
    }

    fn dmabuf_formats(&self, output: OutputId) -> BTreeSet<u32> {
        self.screencopy_dmabuf_support(output)
            .map(|support| support.formats.iter().map(|format| format.code as u32).collect())
            .unwrap_or_default()
    }

    fn dmabuf_accepts(&self, output: OutputId, dmabuf: &Dmabuf) -> bool {
        self.screencopy_dmabuf_support(output)
            .is_some_and(|support| support.check(dmabuf).is_ok())
    }

    fn copy_into_buffer(&mut self, copy: ShmCopy) -> Option<(Rect, Tick)> {
        State::copy_into_buffer(self, copy)
    }

    fn queue_screencopy(&mut self, copy: PendingCopy) {
        State::queue_screencopy(self, copy);
    }
}
