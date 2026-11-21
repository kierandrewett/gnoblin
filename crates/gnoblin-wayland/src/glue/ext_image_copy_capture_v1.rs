//! `gnoblin-wayland-ext-image-copy-capture-v1`: the capture rules behind each object are the engine's
//! (`protocols/capture_image_copy.rs`), because they read the capture tables, the pixel path and the lock.

use gnoblin_backend::DmabufSupport;
use gnoblin_wayland_protocols::ext_image_copy_capture_v1::{
    BufferConstraints, Host, ImageCopyFrame, ImageCopyFrameRef, ImageCopySession, ImageCopySessionRef,
};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::wayland::image_capture_source::ImageCaptureSource;

use crate::State;

impl Host for State {
    fn image_copy_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        State::image_copy_constraints(self, source)
    }

    fn image_copy_new_session(&mut self, session: ImageCopySession) {
        State::image_copy_new_session(self, session);
    }

    fn image_copy_frame(&mut self, session: &ImageCopySessionRef, frame: ImageCopyFrame) {
        State::image_copy_frame(self, session, frame);
    }

    fn image_copy_frame_aborted(&mut self, frame: &ImageCopyFrameRef) {
        State::image_copy_frame_aborted(self, frame);
    }

    fn image_copy_session_destroyed(&mut self, session: ImageCopySessionRef) {
        State::image_copy_session_destroyed(self, session);
    }

    fn dmabuf_planes_fit(dmabuf: &Dmabuf) -> bool {
        DmabufSupport::plane_count_fits(dmabuf)
    }
}
