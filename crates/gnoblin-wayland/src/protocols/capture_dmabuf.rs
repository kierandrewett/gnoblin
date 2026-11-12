//! Captures into a client's GPU buffer.
//!
//! A client that captures can ask for the frame in shared memory (a copy through the CPU, answered the moment the
//! request arrives) or in a buffer it allocated on the GPU (`dma_buf`). The second is the one a screen recorder, a
//! cast or an XR compositor wants: the frame never crosses to the CPU. It is also the one a protocol handler cannot
//! answer, because the copy is made where the renderer is, and a handler holds the state and not the backend. So the
//! request is **queued** and the loop answers it, once per turn, with the backend in hand
//! ([`State::apply_dmabuf_captures`]) — the same shape as the import of a client's buffer
//! ([`State::apply_dmabuf_imports`]).
//!
//! # What is offered, and when
//!
//! A session offers `dma_buf` (and `zwlr_screencopy` sends `linux_dmabuf`) only for a target the backend can write
//! into: [`Backend::dmabuf_capture_support`] for an output or a window, which is read from the output's frame, and
//! [`Backend::dmabuf_picture_support`] for a View, which is its picture uploaded. The answers are kept in
//! [`DmabufCapture`] and learnt by [`State::refresh_dmabuf_capture`], so a protocol handler can read them without a
//! backend. A backend that cannot, or a session with no GPU, offers shared memory only, exactly as before.
//!
//! # The rules are the same rules
//!
//! The queue changes *when* the copy is made and nothing else. The claim on the session's next frame is taken when
//! the request arrives and kept; the session lock, the one-frame-per-tick pacing, the target's existence and the
//! damage report are all applied by [`gnoblin_capture::Table::capture`] when the copy is made, as for shared
//! memory, so a capture requested before the lock and answered after it is refused:
//! `a_gpu_capture_requested_before_the_lock_is_refused_after_it` in `tests/dmabuf_capture.rs`.
//!
//! # Failure modes
//!
//! 1. **A queued copy outlives its buffer or its client.** A frame whose object is gone is dropped from the queue and
//!    its claim released ([`State::abort_image_copy`]); a screencopy frame that is no longer alive is skipped.
//! 2. **A copy is made on a backend that cannot.** The queue is only fed for a buffer a session offered, and the
//!    backend refuses anything else, which is the frame's `failed`.
//! 3. **A secure window shows in a GPU capture.** The backend draws every secure layer as the session's treatment
//!    says for a frame copy ([`Backend::copy_to_dmabuf`]), and a View's picture is already redacted:
//!    `a_secure_window_is_masked_in_a_gpu_capture` in `tests/dmabuf_capture.rs`.

use std::collections::{BTreeMap, BTreeSet};

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;
use smithay::reexports::wayland_server::Resource;

use gnoblin_backend::{Backend, DmabufCopy, DmabufSource, DmabufSupport};
use gnoblin_capture::{
    CaptureOutcome, CaptureSource as PixelSource, Copied, Format, FrameClaim, Session, SessionId, Target,
};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_types::{ClientId, OutputId, Point, Rect, Tick};

use super::{Cached, Targets};
use crate::State;
use crate::log;
use crate::protocols::image_copy_session::{DmabufConstraints, Frame, FrameRef};
use crate::protocols::zwlr_screencopy_v1::answer_copy;

/// What the backend can write a capture into, and the copies waiting for the loop.
#[derive(Default)]
pub(crate) struct DmabufCapture {
    /// The outputs the backend was asked about, so each is asked once.
    asked: BTreeSet<OutputId>,
    /// What each output's frame may be written into, for the outputs where the backend can.
    frames: BTreeMap<OutputId, DmabufSupport>,
    /// Whether the backend was asked about a View's picture.
    pictures_asked: bool,
    /// What a View's picture may be written into, when the backend can.
    pictures: Option<DmabufSupport>,
    /// The copies a handler queued, answered by the next turn of the loop.
    pending: Vec<Pending>,
}

/// A copy a protocol handler could not make and left for the loop.
enum Pending {
    /// A frame of an `ext_image_copy_capture` session.
    ImageCopy(ImageCopyPending),
    /// A `zwlr_screencopy` frame.
    Screencopy(ScreencopyPending),
}

/// A frame of an `ext-image-copy-capture` session that waits for its copy.
struct ImageCopyPending {
    /// The session in the table.
    session: SessionId,
    /// The claim on the session's next frame, taken when the request arrived.
    claim: FrameClaim,
    /// The protocol object the client waits on.
    frame: Frame,
    /// The client's buffer.
    dmabuf: Dmabuf,
}

/// A `zwlr_screencopy` frame that waits for its copy.
pub(crate) struct ScreencopyPending {
    /// The protocol object the client waits on.
    pub(crate) resource: ZwlrScreencopyFrameV1,
    /// The connection that made it.
    pub(crate) owner: ClientId,
    /// The output it captures.
    pub(crate) output: OutputId,
    /// The region, in the output's physical pixels.
    pub(crate) region: Rect,
    /// The geometry the client was told to allocate.
    pub(crate) format: Format,
    /// Whether the cursor is in the copy.
    pub(crate) with_cursor: bool,
    /// Whether the client asked for the damage.
    pub(crate) with_damage: bool,
    /// The client's buffer.
    pub(crate) dmabuf: Dmabuf,
}

/// The pixels a copy into a GPU buffer reads: the same targets as shared memory, but the backend does the copy.
struct DmabufPixels<'a> {
    /// The backend that owns the renderer.
    backend: &'a mut dyn Backend<State>,
    /// Where every target is.
    targets: Targets,
    /// The last frame of every output, which is also how a capture knows an output has drawn at all.
    frames: &'a BTreeMap<OutputId, Cached>,
    /// The client's buffer.
    dmabuf: &'a Dmabuf,
    /// Where the copied region starts inside the target, in physical pixels.
    offset: Point,
    /// Whether the cursor is in the copy.
    with_cursor: bool,
}

impl DmabufPixels<'_> {
    /// The tick the pixels of `target` came from, as [`super::Pixels::presented`] answers it.
    fn presented(&self, target: Target) -> Option<Tick> {
        self.frames.get(&self.targets.output(target)?).map(|cached| cached.tick)
    }
}

impl PixelSource for DmabufPixels<'_> {
    fn bounds(&self, target: Target) -> Option<Rect> {
        self.targets.bounds(target)
    }

    fn copy(&mut self, target: Target, region: Rect, _format: Format) -> Copied {
        let (source, from): (DmabufSource<'_>, Point) = if let Target::View(view) = target {
            // A View is its own picture: no output frame, no origin inside one, no cursor.
            let Some(picture) = self.targets.view_picture(view) else {
                return Copied::Nothing;
            };
            (DmabufSource::Picture(picture), Point::new(0, 0))
        } else {
            let Some(output) = self.targets.output(target) else {
                return Copied::Nothing;
            };
            // An output that has drawn no frame has nothing to copy, as for shared memory.
            if !self.frames.contains_key(&output) {
                return Copied::Nothing;
            }
            let origin = self.targets.origin(target);
            (
                DmabufSource::Frame {
                    output,
                    with_cursor: self.with_cursor,
                },
                Point::new(
                    origin.x.saturating_add(self.offset.x),
                    origin.y.saturating_add(self.offset.y),
                ),
            )
        };
        match self
            .backend
            .copy_to_dmabuf(source, self.dmabuf, DmabufCopy { region, from })
        {
            Ok(()) => Copied::Copied,
            Err(error) => {
                log::warn!(area = "capture", %error, %target, "the frame could not be copied into the client's GPU buffer");
                Copied::Nothing
            }
        }
    }
}

impl State {
    /// Learns what the backend can write a capture into: each output's frame, and a View's picture.
    ///
    /// Asked once per output and once for the pictures, then kept, so a protocol handler that has no backend can
    /// say what a session offers. It runs before the loop dispatches, so a session made in a turn finds the answer,
    /// and again when an output appears.
    pub(crate) fn refresh_dmabuf_capture(&mut self, backend: &mut dyn Backend<State>) {
        let outputs: Vec<OutputId> = backend.outputs().iter().map(|output| output.id).collect();
        let known = &mut self.capture.dmabuf;
        known.frames.retain(|output, _| outputs.contains(output));
        known.asked.retain(|output| outputs.contains(output));
        for output in outputs {
            if !known.asked.insert(output) {
                continue;
            }
            if let Some(support) = backend.dmabuf_capture_support(output) {
                log::info!(
                    area = "capture",
                    %output,
                    formats = support.formats.iter().count(),
                    device = ?support.main_device,
                    "captures of this output can be written into a client's GPU buffer"
                );
                known.frames.insert(output, support);
            }
        }
        if !known.pictures_asked {
            known.pictures_asked = true;
            known.pictures = backend.dmabuf_picture_support();
            if let Some(support) = &known.pictures {
                log::info!(
                    area = "capture",
                    formats = support.formats.iter().count(),
                    "a View's capture can be written into a client's GPU buffer"
                );
            }
        }
    }

    /// The `dma_buf` constraints a session of `target` offers now, or `None` when it offers shared memory only.
    pub(crate) fn dmabuf_constraints_for(&self, target: Target) -> Option<DmabufConstraints> {
        let targets = self.capture_targets();
        let known = &self.capture.dmabuf;
        let support = match target {
            Target::View(_) => known.pictures.as_ref(),
            Target::Output(_) | Target::Window(_) => {
                targets.output(target).and_then(|output| known.frames.get(&output))
            }
        };
        DmabufConstraints::from_support(support?)
    }

    /// What a capture of `output` may be written into, which is what `zwlr_screencopy` announces as `linux_dmabuf`.
    pub(crate) fn screencopy_dmabuf_support(&self, output: OutputId) -> Option<&DmabufSupport> {
        self.capture.dmabuf.frames.get(&output)
    }

    /// Queues the copy of an `ext-image-copy-capture` frame into `dmabuf` for the loop.
    pub(crate) fn queue_image_copy(&mut self, session: SessionId, claim: FrameClaim, frame: Frame, dmabuf: Dmabuf) {
        self.capture.dmabuf.pending.push(Pending::ImageCopy(ImageCopyPending {
            session,
            claim,
            frame,
            dmabuf,
        }));
    }

    /// Queues the copy of a `zwlr_screencopy` frame into a GPU buffer for the loop.
    pub(crate) fn queue_screencopy(&mut self, copy: ScreencopyPending) {
        self.capture.dmabuf.pending.push(Pending::Screencopy(copy));
    }

    /// Forgets the queued copy of `frame` and releases its claim, because the client destroyed the frame object.
    pub(crate) fn abort_image_copy(&mut self, frame: &FrameRef) {
        let pending = &mut self.capture.dmabuf.pending;
        let Some(index) = pending
            .iter()
            .position(|queued| matches!(queued, Pending::ImageCopy(copy) if copy.frame == *frame))
        else {
            return;
        };
        if let Pending::ImageCopy(copy) = pending.remove(index) {
            self.capture.table.cancel(copy.claim);
        }
    }

    /// Answers the copies into a client's GPU buffer that handlers queued since the last turn, from the frames the
    /// loop just drew.
    pub(crate) fn apply_dmabuf_captures(&mut self, backend: &mut dyn Backend<State>) {
        for pending in std::mem::take(&mut self.capture.dmabuf.pending) {
            match pending {
                Pending::ImageCopy(copy) => self.finish_image_copy(backend, copy),
                Pending::Screencopy(copy) => self.finish_screencopy(backend, copy),
            }
        }
    }

    /// Makes one `ext-image-copy-capture` copy and answers its frame.
    fn finish_image_copy(&mut self, backend: &mut dyn Backend<State>, copy: ImageCopyPending) {
        let ImageCopyPending {
            session,
            claim,
            frame,
            dmabuf,
        } = copy;
        let tick = self.capture.tick;
        let entry = self.capture.table.get(session);
        let stream = entry.map_or(CaptureStream::Requested, Session::stream);
        let target = entry.map_or(Target::Output(OutputId::default()), Session::target);
        let capture = self.lock_capture(stream);
        let targets = self.capture_targets();
        let mut source = DmabufPixels {
            backend,
            targets,
            frames: &self.capture.pixels,
            dmabuf: &dmabuf,
            offset: Point::new(0, 0),
            // This protocol has no cursor option, so its frames never carry the cursor.
            with_cursor: false,
        };
        let presented = source.presented(target).unwrap_or(tick);
        let outcome = self.capture.table.capture(claim, tick, capture, &mut source);
        self.complete_image_copy(session, frame, outcome, presented);
    }

    /// Makes one `zwlr_screencopy` copy and answers its frame.
    fn finish_screencopy(&mut self, backend: &mut dyn Backend<State>, copy: ScreencopyPending) {
        let ScreencopyPending {
            resource,
            owner,
            output,
            region,
            format,
            with_cursor,
            with_damage,
            dmabuf,
        } = copy;
        if !resource.is_alive() {
            return;
        }
        // Checked again at the copy: a frame requested before the cover went up must never expose the desktop
        // after it did.
        let copied = if self.capture_allowed() {
            let capture = self.lock_capture(CaptureStream::Requested);
            let tick = self.capture.tick;
            self.copy_into_dmabuf(
                backend,
                (owner, output),
                &dmabuf,
                format,
                Point::new(region.x, region.y),
                with_cursor,
                capture,
                tick,
            )
        } else {
            log::debug!(area = "capture", %output, "a screencopy frame is refused during the lock");
            None
        };
        answer_copy(&resource, with_damage, copied);
        self.flush();
    }

    /// Copies `offset` of an output into `dmabuf` through the session rules, as `copy_into_buffer` does for shared
    /// memory: one screencopy frame is one session, and the damage it reports is the whole region.
    #[allow(clippy::too_many_arguments)]
    fn copy_into_dmabuf(
        &mut self,
        backend: &mut dyn Backend<State>,
        (owner, output): (ClientId, OutputId),
        dmabuf: &Dmabuf,
        format: Format,
        offset: Point,
        with_cursor: bool,
        capture: Capture,
        tick: Tick,
    ) -> Option<(Rect, Tick)> {
        let (session, claim) = self.open_screencopy(owner, output, format, tick)?;
        let targets = self.capture_targets();
        let mut source = DmabufPixels {
            backend,
            targets,
            frames: &self.capture.pixels,
            dmabuf,
            offset,
            with_cursor,
        };
        let presented = source.presented(Target::Output(output)).unwrap_or(tick);
        let outcome: CaptureOutcome = self.capture.table.capture(claim, tick, capture, &mut source);
        self.close_screencopy(session, outcome, presented)
    }
}
