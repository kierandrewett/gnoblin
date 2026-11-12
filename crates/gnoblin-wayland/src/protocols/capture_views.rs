//! The pictures of the Views a renderer draws on the GPU, read back for the consumers that need bytes.
//!
//! When the backend's renderer draws a View's subtree into a texture ([`gnoblin_backend::Backend::gpu_views`]) the
//! composer keeps no picture of the View on the CPU: the display shows the texture, and nothing is copied. A consumer
//! that needs the pixels (an `ext-image-copy-capture` session, a PipeWire stream, the shell's `capture` still) is
//! served from a **read-back**: the loop asks the backend for the View's picture once per revision, only for a View
//! some consumer is reading, and publishes it where the CPU pictures of the other Views are published, so every
//! consumer reads the same map ([`CaptureTables::view_pictures`](super::CaptureTables)).
//!
//! # When the read happens
//!
//! After the frames of the turn are drawn ([`State::read_gpu_views`]), because that is where the backend is in hand
//! and a handler holds the state and not the backend: the same shape as the copy into a client's GPU buffer
//! (`capture_dmabuf`). A request that arrives before the View has a read-back (a frame of a session that was just
//! opened, a still) is **queued** and answered at the end of the same turn, from the read it triggers:
//! `a_view_drawn_on_the_gpu_is_captured_in_the_turn_it_is_asked` in `tests/view_gpu.rs`.
//!
//! # What a read-back is
//!
//! The capture picture: the scene's capture variant, so a secure node's pixels are never in it, however the display
//! shows the View. A read-back is dropped the moment the composer draws the View again
//! ([`State::sync_view_readbacks`]), so a consumer never reads a picture older than the scene it belongs to, and the
//! one that shows a secure node cannot outlive the redraw that masks it.
//!
//! # Failure modes
//!
//! 1. **The driver refuses the read.** The View has no picture this turn: a consumer is refused as for a View that
//!    draws nothing, nothing stale is repeated, and the read is asked for again on the next revision.
//! 2. **A queued frame outlives its client.** A destroyed frame object drops its queued copy and releases its claim
//!    ([`State::abort_view_copy`]).
//! 3. **A View is destroyed with a consumer waiting.** The queued copy is answered by the table, which finds the
//!    target gone and stops the session.

use std::collections::BTreeSet;
use std::sync::Arc;

use gnoblin_backend::Backend;
use gnoblin_capture::{FrameClaim, Session as CaptureSession, SessionId, Target};
use gnoblin_lock::CaptureStream;
use gnoblin_types::{NodeId, OutputId, Point, Rect};

use super::Pixels;
use crate::State;
use crate::log;
use crate::protocols::image_copy_session::{FailureReason, Frame, FrameRef};

/// A frame of an `ext-image-copy-capture` session of a View that waits for the View's picture to be read back.
pub(crate) struct PendingViewCopy {
    /// The session in the table.
    session: SessionId,
    /// The claim on the session's next frame, taken when the request arrived.
    claim: FrameClaim,
    /// The protocol object the client waits on.
    frame: Frame,
}

impl State {
    /// Drops the read-back of every View the composer has drawn again or no longer has, and with it the picture
    /// published for that View, so no consumer reads a picture older than the View's scene.
    ///
    /// Called once per turn, right after the composer's draws.
    pub(crate) fn sync_view_readbacks(&mut self) {
        let scenes: std::collections::BTreeMap<NodeId, u64> = self
            .composer
            .view_scenes()
            .into_iter()
            .map(|(view, scene)| (view, scene.revision()))
            .collect();
        let stale: Vec<NodeId> = self
            .capture
            .view_readbacks
            .iter()
            .filter(|(view, (revision, _))| scenes.get(*view) != Some(revision))
            .map(|(view, _)| *view)
            .collect();
        for view in stale {
            self.capture.view_readbacks.remove(&view);
            self.capture.view_pictures.remove(&view);
        }
    }

    /// Whether `view` is drawn on the GPU and has no picture read back yet, so a frame of it waits for the loop.
    pub(crate) fn view_awaits_readback(&self, view: NodeId) -> bool {
        self.composer.has_view_scene(view) && !self.capture.view_pictures.contains_key(&view)
    }

    /// Queues a frame of a View session until the loop has read the View's picture back.
    pub(crate) fn queue_view_copy(&mut self, session: SessionId, claim: FrameClaim, frame: Frame) {
        self.capture.view_copies.push(PendingViewCopy { session, claim, frame });
    }

    /// Forgets the queued frame of a View session and releases its claim, because the client destroyed the frame
    /// object. Answers whether there was one.
    pub(crate) fn abort_view_copy(&mut self, frame: &FrameRef) -> bool {
        let Some(index) = self
            .capture
            .view_copies
            .iter()
            .position(|queued| queued.frame == *frame)
        else {
            return false;
        };
        let copy = self.capture.view_copies.remove(index);
        self.capture.table.cancel(copy.claim);
        true
    }

    /// The Views some consumer is reading: the target of an open session, of a frame waiting for a read-back, and of
    /// a still waiting for one.
    fn viewed(&self) -> BTreeSet<NodeId> {
        let sessions = self
            .capture
            .table
            .sessions()
            .filter_map(|session| match session.target() {
                Target::View(view) => Some(view),
                _ => None,
            });
        let stills = self.capture.view_stills.iter().map(|call| call.node);
        sessions.chain(stills).collect()
    }

    /// Reads back the picture of each GPU View a consumer is reading, answers the frames and stills that waited for
    /// one, and so serves every consumer of a View from the same turn the View was drawn in.
    ///
    /// The backend draws the scene when its renderer holds no texture of the current revision, so a View shown on no
    /// output is still read: nothing here depends on where the View is.
    pub(crate) fn read_gpu_views(&mut self, backend: &mut dyn Backend<State>) {
        let scenes = self.composer.view_scenes();
        let live: Vec<u64> = scenes.iter().map(|(view, _)| view.get()).collect();
        let viewed = self.viewed();
        for (view, scene) in &scenes {
            if !viewed.contains(view) {
                continue;
            }
            let current = self
                .capture
                .view_readbacks
                .get(view)
                .is_some_and(|(revision, _)| *revision == scene.revision());
            if current {
                continue;
            }
            match backend.read_view(scene, true, &live) {
                Ok(picture) => {
                    let picture = Arc::new(picture);
                    let size = picture.size();
                    self.capture
                        .view_readbacks
                        .insert(*view, (scene.revision(), Arc::clone(&picture)));
                    self.capture.view_pictures.insert(*view, picture);
                    // The picture is new, so every consumer's next frame reports the whole of it.
                    self.capture
                        .table
                        .note_damage(Target::View(*view), Rect::from_size(size));
                }
                Err(error) => {
                    log::warn!(
                        area = "capture",
                        %view,
                        revision = scene.revision(),
                        ?error,
                        "a View's picture could not be read back; it has no capture picture until its next change"
                    );
                }
            }
        }
        for copy in std::mem::take(&mut self.capture.view_copies) {
            self.finish_view_copy(copy);
        }
        let stills = std::mem::take(&mut self.capture.view_stills);
        self.run_view_captures_now(stills);
    }

    /// Makes one frame of a View session that waited for the read-back and answers it.
    fn finish_view_copy(&mut self, copy: PendingViewCopy) {
        let PendingViewCopy { session, claim, frame } = copy;
        let Some(buffer) = frame.buffer() else {
            self.capture.table.cancel(claim);
            frame.fail(FailureReason::BufferConstraints);
            return;
        };
        let tick = self.capture.tick;
        let entry = self.capture.table.get(session);
        let stream = entry.map_or(CaptureStream::Requested, CaptureSession::stream);
        let target = entry.map_or(Target::Output(OutputId::default()), CaptureSession::target);
        let capture = self.lock_capture(stream);
        let targets = self.capture_targets();
        let mut source = Pixels {
            targets,
            frames: &self.capture.pixels,
            buffer,
            offset: Point::new(0, 0),
            with_cursor: false,
        };
        let presented = source.presented(target).unwrap_or(tick);
        let outcome = self.capture.table.capture(claim, tick, capture, &mut source);
        self.complete_image_copy(session, frame, outcome, presented);
    }
}
