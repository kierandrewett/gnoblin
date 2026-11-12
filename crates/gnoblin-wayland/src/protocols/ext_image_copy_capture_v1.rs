//! XML protocol `ext_image_copy_capture_v1` (`<protocol name="ext_image_copy_capture_v1">`); interfaces served: `ext_image_copy_capture_v1`.
//!
//! `ext_image_copy_capture_v1`: sessions that copy a source's pixels into client buffers.
//!
//! A session is opened for an [`ImageCaptureSource`](smithay::wayland::image_capture_source)
//! and each frame is answered from the shared pixel path in
//! [`capture_shared`](super::capture_shared). Its refusals map onto the protocol's
//! failure reasons here. The protocol's own objects, and the check that a client's buffer meets the
//! constraints a session announced, are in [`image_copy_session`](super::image_copy_session).
//!
//! # Shared memory and GPU buffers
//!
//! A session always offers `wl_shm` (`argb8888`, `xrgb8888`). It also offers `dma_buf`, with the render node and the
//! format and modifier pairs, when the backend can write the target into a client's GPU buffer
//! ([`Backend::dmabuf_capture_support`](gnoblin_backend::Backend::dmabuf_capture_support), and
//! `dmabuf_picture_support` for a View): a software session, a Vulkan session and a session with no GPU offer
//! shared memory only, and a client that needs a GPU buffer finds none offered. A frame into shared memory is
//! copied when the request arrives. A frame into a GPU buffer is queued, and the loop makes the copy where the
//! renderer is: see [`capture_shared`](super::capture_shared) and `capture_dmabuf`.
//!
//! # Failure modes
//!
//! 1. **A GPU buffer is accepted when none was offered.** The frame is refused `buffer_constraints` before the
//!    compositor sees it: `a_gpu_buffer_is_refused_when_none_was_offered` in `tests/dmabuf_capture.rs`.
//! 2. **The session lock does not stop a GPU capture.** The lock is applied when the copy is made, so a request that
//!    arrived before the lock is refused after it: `a_gpu_capture_requested_before_the_lock_is_refused_after_it`.
//! 3. **A destroyed frame is copied into.** The queued copy is dropped and its claim released:
//!    `a_destroyed_frame_is_not_copied_into`.

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::image_capture_source::ImageCaptureSource;

use gnoblin_capture::{
    BufferId, CaptureOutcome, Format, PixelFormat, Refusal, Session as CaptureSession, SessionId, Stop, Target,
};
use gnoblin_lock::CaptureStream;
use gnoblin_types::{ClientId, OutputId, Point, Tick};

use super::capture_shared::{Pixels, SessionKey, key_of};
use super::image_copy_session::{BufferConstraints, FailureReason, Frame, FrameRef, Session, SessionRef};
use crate::State;

use crate::log;

impl State {
    /// The constraints a new session of `source` must announce, or `None` when it may not capture at all.
    pub(crate) fn image_copy_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        // Refused during the lock before any geometry reaches the client: a session that
        // cannot capture must not even learn how big its target is.
        if !self.capture_allowed() {
            return None;
        }
        let target = self.capture.target_of(source)?;
        let bounds = self.capture_bounds(target)?;
        Some(BufferConstraints {
            size: bounds.size(),
            shm: vec![wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888],
            // Offered only for a target the backend can write into a client's GPU buffer.
            dma: self.dmabuf_constraints_for(target),
        })
    }

    /// Opens the capture session a client made.
    pub(crate) fn image_copy_new_session(&mut self, session: Session) {
        let Some(target) = self.capture.target_of(&session.source()) else {
            log::warn!(area = "capture", "a session names a source this session does not know");
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
        log::debug!(area = "capture", %target, session = %id, "session opened");
        self.publish();
    }

    /// Answers a client's request for one frame: a frame into shared memory now, a frame into a GPU buffer when
    /// the loop next draws.
    pub(crate) fn image_copy_frame(&mut self, session: &SessionRef, frame: Frame) {
        let Some(id) = key_of(session.user_data()) else {
            frame.fail(FailureReason::Stopped);
            return;
        };
        let Some(buffer) = frame.buffer() else {
            frame.fail(FailureReason::BufferConstraints);
            return;
        };
        let tick = self.capture.tick;
        // The answer is the lock's, read at the copy: the embargo is applied at the frame, not at the session.
        let entry = self.capture.table.get(id);
        let stream = entry.map_or(CaptureStream::Requested, CaptureSession::stream);
        let target = entry.map_or(Target::Output(OutputId::default()), CaptureSession::target);
        let buffer_id = BufferId::new(u64::from(buffer.id().protocol_id()));
        let claim = match self.capture.table.request(id, buffer_id, tick) {
            Ok(claim) => claim,
            Err(refusal) => {
                log::debug!(
                    area = "capture",
                    reason = refusal.as_str(),
                    "the frame could not be claimed"
                );
                frame.fail(failure_reason(refusal));
                return;
            }
        };
        if let Ok(dmabuf) = get_dmabuf(&buffer) {
            // A GPU buffer is written where the renderer is, which is not here: the loop makes the copy.
            let dmabuf = dmabuf.clone();
            self.queue_image_copy(id, claim, frame, dmabuf);
            return;
        }
        let capture = self.lock_capture(stream);
        let targets = self.capture_targets();
        let mut source = Pixels {
            targets,
            frames: &self.capture.pixels,
            buffer,
            offset: Point::new(0, 0),
            // This protocol has no cursor option, so its frames never carry the cursor.
            with_cursor: false,
        };
        // The presentation time is the frame the pixels came out of, not the tick the client asked on: those are
        // the same tick only by accident.
        let presented = source.presented(target).unwrap_or(tick);
        let outcome = self.capture.table.capture(claim, tick, capture, &mut source);
        self.complete_image_copy(id, frame, outcome, presented);
    }

    /// Tells a client how its frame ended, and stops a session that can never produce another.
    pub(crate) fn complete_image_copy(
        &mut self,
        id: SessionId,
        frame: Frame,
        outcome: CaptureOutcome,
        presented: Tick,
    ) {
        match outcome {
            CaptureOutcome::Copied(report) => {
                frame.success(&report.damage, std::time::Duration::from_millis(presented.get()));
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

    /// A client destroyed a frame object: a copy still queued for it is dropped.
    pub(crate) fn image_copy_frame_aborted(&mut self, frame: &FrameRef) {
        self.abort_image_copy(frame);
    }

    /// A client destroyed a session object.
    pub(crate) fn image_copy_session_destroyed(&mut self, session: SessionRef) {
        if let Some(id) = key_of(session.user_data()) {
            self.capture.table.close(id);
            log::debug!(area = "capture", session = %id, "session closed");
        }
        self.capture.sessions.retain(|held| *held != session);
        self.publish();
    }
}

/// The protocol's failure reason for a refusal.
fn failure_reason(refusal: Refusal) -> FailureReason {
    match refusal {
        Refusal::Locked | Refusal::Stopped(_) | Refusal::Closed | Refusal::TargetGone => FailureReason::Stopped,
        Refusal::Source | Refusal::Paced | Refusal::Busy | Refusal::Stale => FailureReason::Unknown,
    }
}
