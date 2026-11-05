//! XML protocol `ext_image_copy_capture_v1` (`<protocol name="ext_image_copy_capture_v1">`); interfaces served: `ext_image_copy_capture_v1`.
//!
//! `ext_image_copy_capture_v1`: sessions that copy a source's pixels into client buffers.
//!
//! A session is opened for an [`ImageCaptureSource`](smithay::wayland::image_capture_source)
//! and each frame is answered from the shared pixel path in
//! [`capture_shared`](super::capture_shared). Its refusals map onto the protocol's
//! failure reasons here.

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::utils::{Buffer as BufferSpace, Rectangle, Transform};
use smithay::wayland::image_capture_source::ImageCaptureSource;
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CaptureFailureReason, Frame as CaptureFrame, ImageCopyCaptureHandler, ImageCopyCaptureState,
    Session as CaptureSession, SessionRef,
};
use tracing::{debug, warn};

use gnoblin_capture::{BufferId, CaptureOutcome, Format, PixelFormat, Refusal, Session, Stop, Target};
use gnoblin_lock::CaptureStream;
use gnoblin_types::{ClientId, OutputId, Point};

use super::capture_shared::{Pixels, SessionKey, key_of};
use crate::State;

impl ImageCopyCaptureHandler for State {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.capture.image_copy
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        // Refused during the lock before any geometry reaches the client: a session that
        // cannot capture must not even learn how big its target is.
        if !self.capture_allowed() {
            return None;
        }
        let target = self.capture.target_of(source)?;
        let bounds = self.capture_bounds(target)?;
        Some(BufferConstraints {
            size: (bounds.width as i32, bounds.height as i32).into(),
            shm: vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888],
            // Capture advertises SHM only; DMA-BUF capture is not implemented yet.
            dma: None,
        })
    }

    fn new_session(&mut self, session: CaptureSession) {
        let Some(target) = self.capture.target_of(&session.source()) else {
            warn!("[capture] a session names a source this session does not know");
            return;
        };
        let Some(bounds) = self.capture_bounds(target) else {
            return;
        };
        let Some(format) = Format::new(bounds.size(), bounds.width.saturating_mul(4), PixelFormat::Argb8888) else {
            return;
        };
        let owner = session
            .client()
            .map_or_else(ClientId::default, |client| self.capture.owner(&client.id()));
        let id = self.capture.table.open(owner, target, CaptureStream::Requested, format);
        session.user_data().insert_if_missing(|| SessionKey(id));
        self.capture.sessions.push(session);
        debug!(%target, session = %id, "[capture] session opened");
        self.publish();
    }

    fn frame(&mut self, session: &SessionRef, frame: CaptureFrame) {
        let Some(id) = key_of(session.user_data()) else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        let buffer = frame.buffer();
        let tick = self.capture.tick;
        // The answer is the lock's, read now: the embargo is applied at the frame, not at
        // the session.
        let stream = self
            .capture
            .table
            .get(id)
            .map_or(CaptureStream::Requested, Session::stream);
        let capture = self.lock_capture(stream);
        let buffer_id = BufferId::new(u64::from(buffer.id().protocol_id()));
        let mut claim = match self.capture.table.request(id, buffer_id, tick) {
            Ok(claim) => claim,
            Err(refusal) => {
                debug!(reason = refusal.as_str(), "[capture] the frame could not be claimed");
                frame.fail(failure_reason(refusal));
                return;
            }
        };
        let targets = self.capture_targets();
        let mut source = Pixels {
            targets,
            frames: &self.capture.pixels,
            buffer,
            offset: Point::new(0, 0),
        };
        let outcome = self.capture.table.capture(&mut claim, tick, capture, &mut source);
        match outcome {
            CaptureOutcome::Copied(report) => {
                let damage: Vec<Rectangle<i32, BufferSpace>> = report
                    .damage
                    .iter()
                    .map(|rect| Rectangle::new((rect.x, rect.y).into(), (rect.width as i32, rect.height as i32).into()))
                    .collect();
                // The presentation time is the frame the pixels came out of, not the
                // tick the client asked on: those are the same tick only by accident.
                let presented = source
                    .presented(
                        self.capture
                            .table
                            .get(id)
                            .map_or(Target::Output(OutputId::default()), Session::target),
                    )
                    .unwrap_or(tick);
                frame.success(
                    Transform::Normal,
                    damage,
                    std::time::Duration::from_millis(presented.get()),
                );
            }
            CaptureOutcome::Refused(refusal) => {
                frame.fail(failure_reason(refusal));
                if matches!(refusal, Refusal::Locked | Refusal::Stopped(_) | Refusal::TargetGone) {
                    // The session can never produce another frame, so the client is told
                    // `stopped` in the same turn rather than waiting for the next one.
                    let reason = match refusal {
                        Refusal::TargetGone => Stop::TargetGone,
                        _ => Stop::Embargoed,
                    };
                    self.capture.stop(id, reason);
                }
            }
        }
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        if let Some(id) = key_of(session.user_data()) {
            self.capture.table.close(id);
            debug!(session = %id, "[capture] session closed");
        }
        self.capture.sessions.retain(|held| held.as_ref() != session);
        self.publish();
    }
}

/// The protocol's failure reason for a refusal.
fn failure_reason(refusal: Refusal) -> CaptureFailureReason {
    match refusal {
        Refusal::Locked | Refusal::Stopped(_) | Refusal::Closed | Refusal::TargetGone => CaptureFailureReason::Stopped,
        Refusal::Source | Refusal::Paced | Refusal::Busy | Refusal::Stale | Refusal::Used => {
            CaptureFailureReason::Unknown
        }
    }
}
