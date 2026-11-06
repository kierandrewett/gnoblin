//! The shared capture path behind `ext_image_copy_capture_v1`, `ext_image_capture_source_v1`
//! and `zwlr_screencopy_manager_v1`.
//!
//! Three globals serve one path. `ext-image-copy-capture-v1` is the modern protocol,
//! `ext-image-capture-source-v1` names what to capture, and
//! `zwlr_screencopy_manager_v1` is what shells and tools use today. All three hand their
//! frames to the same rules in `gnoblin-capture` and to the same pixel copy; the
//! rest of this module is that shared part. [`Pixels`] implements
//! [`gnoblin_capture::CaptureSource`] over the last frame each output drew, so an
//! output and a window are cropped out of the frame the compositor already presented
//! rather than composed a second time.
//!
//! ## What a capture reads
//!
//! The frame path in [`crate::frame`] reads each output's pixels back while any
//! capture session is open ([`crate::State::capture_frame`]). A capture therefore
//! returns the last frame that reached the output — the same pixels a screenshot of an
//! idle session returns — which is what "capture the next frame" means for a compositor
//! that presents on request. A request that arrives before the first frame has been
//! drawn has nothing to copy and is answered with the protocol's failure.
//!
//! ## The embargo
//!
//! Every capture protocol asks `gnoblin-capture` at every frame, and that crate applies the
//! lock's answer. Three consequences, and each is a test in `tests/capture.rs`:
//!
//! - a capture requested while the session lock covers the desktop is refused, and the
//!   client's frame fails (`a_capture_during_the_lock_is_refused`);
//! - a stream opened before the lock is stopped, so its client hears `stopped` and no
//!   frame of it survives the lock (`a_stream_started_before_the_lock_stops`);
//! - a refusal is never answered with the pixels of the previous frame
//!   (`a_refused_frame_never_repeats_the_previous_pixels`).
//!
//! ## Access
//!
//! The globals are gated by the session's one access roster, exactly like the shell
//! protocols: [`crate::access::Global::Screencopy`] and
//! [`crate::access::Global::ImageCopyCapture`] are privileged, so an application that did
//! not earn the role never sees them, and `Global::is_advertised` also answers the
//! `protocols` configuration, so a distribution or a user can turn a capture protocol
//! off. Capture keeps no policy of its own: a connection's role is recorded when it is
//! accepted (`access::Registration`), which is the only moment the answer can hide a
//! global instead of refusing one.
//!
//! ## Failure modes first
//!
//! 1. **A capture returns stale or foreign pixels.** A screencopy of the headless output
//!    returns a client's red window at the right place, and its second frame differs
//!    only where the client redrew: `a_screencopy_of_the_output_returns_the_window_pixels`
//!    and `a_second_image_copy_frame_reports_only_the_changed_region`.
//! 2. **A capture is produced during the lock, or a stream survives it.**
//!    `a_capture_during_the_lock_is_refused`, `a_stream_started_before_the_lock_stops`
//!    and `a_refused_frame_never_repeats_the_previous_pixels`.
//! 3. **A session outlives the client that opened it.** The session table is empty after
//!    the client disconnects: `a_session_ends_when_its_client_disconnects`.
//! 4. **A closed window keeps being captured.** A window source follows its window and
//!    ends when the window closes: `a_window_source_follows_its_window_and_ends_with_it`.
//! 5. **A sandboxed client binds a capture global.** A connection from a marked process
//!    sees neither protocol: `a_sandboxed_client_cannot_bind_the_capture_globals`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use gnoblin_log::{debug, warn};
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::{ClientId as WaylandClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::{Client, DisplayHandle};
use smithay::utils::user_data::UserDataMap;
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, OutputCaptureSourceState, ToplevelCaptureSourceState,
};
use smithay::wayland::image_copy_capture::{ImageCopyCaptureState, Session as CaptureSession};

use gnoblin_backend::render::Image;
use gnoblin_backend::render::copy_into_shm;
use gnoblin_capture::{
    BufferId, CaptureOutcome, CaptureSource as PixelSource, Copied, Format, SessionId, Stop, Table, Target,
};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_types::{ClientId, OutputId, Point, Rect, Size, Tick, WindowId};

use super::zwlr_screencopy_v1::ZwlrScreencopyV1Global;
use crate::State;
use crate::access::{Access, AccessPolicy, Global};
use crate::pipewire_export::{ExportError, PipewireExport, PlaneSource, write_region};

/// The last frame an output presented, which is what a capture copies out of.
#[derive(Clone, Debug)]
pub(super) struct Cached {
    /// The pixels, in physical output pixels.
    image: Image,
    /// The same frame with the cursor composed in, when the pointer is over the output.
    cursored: Option<Image>,
    /// The tick the frame was presented at.
    tick: Tick,
}

/// Where every capturable target is right now.
///
/// A snapshot rather than a borrow of the compositor: the copy path needs geometry and
/// the cached pixels at the same time as it needs the session table mutably, and a
/// snapshot of a handful of rectangles is cheaper than the alternative borrow puzzle.
#[derive(Clone, Debug, Default)]
pub(super) struct Targets {
    /// Every output, in physical pixels.
    outputs: BTreeMap<OutputId, Size>,
    /// Every window: the output it is drawn on and its physical origin in that frame.
    windows: BTreeMap<WindowId, CapturedPlacement>,
}

/// Where a window is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CapturedPlacement {
    /// The output whose frame holds it.
    output: OutputId,
    /// Its top-left corner inside that output's frame, in physical pixels.
    origin: Point,
    /// Its size, in physical pixels.
    size: Size,
}

impl Targets {
    /// What `target` covers, in its own pixels.
    fn bounds(&self, target: Target) -> Option<Rect> {
        match target {
            Target::Output(output) => self.outputs.get(&output).map(|size| Rect::from_size(*size)),
            Target::Window(window) => self
                .windows
                .get(&window)
                .map(|placement| Rect::from_size(placement.size)),
        }
    }

    /// The output whose frame a target is copied out of.
    fn output(&self, target: Target) -> Option<OutputId> {
        match target {
            Target::Output(output) => Some(output),
            Target::Window(window) => self.windows.get(&window).map(|placement| placement.output),
        }
    }

    /// Where a target's own origin sits in its output's frame.
    fn origin(&self, target: Target) -> Point {
        match target {
            Target::Output(_) => Point::new(0, 0),
            Target::Window(window) => self
                .windows
                .get(&window)
                .map_or_else(|| Point::new(0, 0), |placement| placement.origin),
        }
    }
}

/// The session number an `ext-image-copy-capture` session object was given.
///
/// It is stored in the object's own user data, which is the only place Smithay hands
/// back on every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SessionKey(pub(super) SessionId);

/// The session's capture state.
pub(crate) struct CaptureTables {
    /// The session rules and every open session.
    pub(super) table: Table,
    /// The last frame each output drew, replaced once per frame while a session is open.
    pub(super) pixels: BTreeMap<OutputId, Cached>,
    /// The tick the loop is on, so a stream is paced by iterations rather than by a
    /// clock nothing else in this crate reads.
    pub(super) tick: Tick,
    /// The `ext-image-capture-source-v1` objects, mapped to what they capture.
    pub(super) sources: BTreeMap<usize, Target>,
    /// The sessions `ext-image-copy-capture` created, kept alive so their clients keep
    /// their constraints and hear `stopped` when they end.
    pub(super) sessions: Vec<CaptureSession>,
    /// The screencopy frame objects that have not been resolved, so the lock can fail
    /// them.
    pub(super) screencopy_frames: Vec<ZwlrScreencopyFrameV1>,
    /// The identity this session gave each connection, so a session's owner is one value
    /// per connection rather than a token nothing else agrees on.
    owners: HashMap<WaylandClientId, ClientId>,
    /// The next connection identity.
    next_owner: u64,
    /// The manager global, kept for its identity and for the filter it carries.
    #[allow(dead_code)]
    screencopy_global: GlobalId,
    /// The image-copy-capture globals and their sessions.
    pub(super) image_copy: ImageCopyCaptureState,
    /// The output capture sources.
    pub(super) output_source: OutputCaptureSourceState,
    /// The toplevel capture sources.
    pub(super) toplevel_source: ToplevelCaptureSourceState,
    /// The image-copy-capture manager's identity, so a disabled configuration can remove
    /// the global rather than skip it.
    image_copy_global: GlobalId,
    /// The PipeWire streams of the granted capture sessions.
    pipewire: PipewireExport,
}

impl CaptureTables {
    /// Creates every capture global for a session.
    ///
    /// Every global takes its per-client answer from the session's one roster, which is
    /// the same answer the shell protocols get: capture is one more privileged
    /// capability, not a policy of its own.
    pub(crate) fn new(display: &DisplayHandle, access: &Arc<Access>) -> Self {
        let filter = |global: Global| {
            let access = Arc::clone(access);
            move |client: &Client| AccessPolicy::allows(&*access, &client.id(), global)
        };
        let image_copy = ImageCopyCaptureState::new_with_filter::<State, _>(display, filter(Global::ImageCopyCapture));
        let output_source =
            OutputCaptureSourceState::new_with_filter::<State, _>(display, filter(Global::ImageCopyCapture));
        let toplevel_source =
            ToplevelCaptureSourceState::new_with_filter::<State, _>(display, filter(Global::ImageCopyCapture));
        let image_copy_global = image_copy.global();
        let screencopy_global = display.create_global::<State, ZwlrScreencopyManagerV1, _>(
            3,
            ZwlrScreencopyV1Global {
                access: Arc::clone(access),
            },
        );
        Self {
            table: Table::new(),
            pixels: BTreeMap::new(),
            tick: Tick::ZERO,
            sources: BTreeMap::new(),
            sessions: Vec::new(),
            screencopy_frames: Vec::new(),
            owners: HashMap::new(),
            next_owner: 0,
            screencopy_global,
            image_copy_global,
            image_copy,
            output_source,
            toplevel_source,
            pipewire: PipewireExport::new(),
        }
    }

    /// The screencopy manager's global, so a disabled configuration removes it.
    pub(crate) fn screencopy_global(&self) -> GlobalId {
        self.screencopy_global.clone()
    }

    /// The image-copy-capture manager's global, for the same reason.
    pub(crate) fn image_copy_global(&self) -> GlobalId {
        self.image_copy_global.clone()
    }

    /// The identity this session uses for a connection.
    pub(super) fn owner(&mut self, client: &WaylandClientId) -> ClientId {
        if let Some(token) = self.owners.get(client) {
            return *token;
        }
        self.next_owner = self.next_owner.saturating_add(1);
        let token = ClientId::new(self.next_owner);
        self.owners.insert(client.clone(), token);
        token
    }

    /// What a source object captures.
    pub(super) fn target_of(&self, source: &ImageCaptureSource) -> Option<Target> {
        self.sources.get(&source.id()).copied()
    }

    /// How many sessions are open, for a test and for a log line.
    pub(crate) fn sessions(&self) -> usize {
        self.table.len()
    }

    /// Forgets a window's toplevel handle and ends its capture sessions.
    pub(crate) fn window_closed(&mut self, window: WindowId) {
        let target = Target::Window(window);
        let ended = self.table.close_target(target);
        self.sources.retain(|_, source| *source != target);
        // The sessions that captured it are dropped too, which is what sends their
        // clients `stopped`: a session whose target is gone can never produce a frame.
        let orphaned: Vec<SessionId> = self
            .sessions
            .iter()
            .filter_map(|held| key_of(held.user_data()))
            .filter(|id| self.table.get(*id).is_none())
            .collect();
        for session in orphaned {
            self.stop(session, Stop::TargetGone);
        }
        if ended > 0 {
            debug!(%window, ended, "[capture] the window's sessions ended");
        }
    }

    /// Ends `session` for `reason` and tells its client, by dropping the object that
    /// owns it.
    pub(super) fn stop(&mut self, session: SessionId, reason: Stop) {
        self.table.stop(session, reason);
        if let Some(index) = self
            .sessions
            .iter()
            .position(|held| key_of(held.user_data()) == Some(session))
        {
            let held = self.sessions.remove(index);
            held.stop();
        }
    }

    /// Ends every session and every unresolved screencopy frame.
    ///
    /// Answers whether anything was cut, so the caller republishes the session count only
    /// when it changed.
    fn stop_all(&mut self) -> bool {
        if self.table.is_empty() && self.screencopy_frames.is_empty() {
            return false;
        }
        for session in self.table.sessions().map(|session| session.id()).collect::<Vec<_>>() {
            self.table.stop(session, Stop::Embargoed);
        }
        for session in std::mem::take(&mut self.sessions) {
            session.stop();
        }
        for frame in std::mem::take(&mut self.screencopy_frames) {
            if frame.is_alive() {
                frame.failed();
            }
        }
        debug!("[capture] every session stopped for the lock");
        true
    }
}

/// The session number stored in a session object's user data.
pub(super) fn key_of(data: &UserDataMap) -> Option<SessionId> {
    data.get::<SessionKey>().map(|key| key.0)
}

/// A tick as the `ready` event's `tv_sec_hi`, `tv_sec_lo` and `tv_nsec`.
pub(super) fn timestamp(tick: Tick) -> (u32, u32, u32) {
    let seconds = tick.get() / 1000;
    let nanos = (tick.get() % 1000) * 1_000_000;
    (
        u32::try_from(seconds >> 32).unwrap_or(0),
        u32::try_from(seconds & 0xFFFF_FFFF).unwrap_or(0),
        u32::try_from(nanos).unwrap_or(0),
    )
}

/// The pixels of whatever a session captures.
///
/// It reads the last frame its output presented and crops it: an output is its own
/// frame, a window is the part of its output's frame it occupies right now. That is
/// what makes a window capture follow the window rather than a rectangle remembered
/// when the session opened.
pub(super) struct Pixels<'a> {
    /// Where every target is.
    pub(super) targets: Targets,
    /// The last frame of every output.
    pub(super) frames: &'a BTreeMap<OutputId, Cached>,
    /// The buffer the copy writes into.
    pub(super) buffer: WlBuffer,
    /// Where the copied region starts inside the target, in physical pixels.
    ///
    /// A whole-target capture starts at its origin. A screencopy of a region starts at
    /// the region's corner, which is why the same path can serve both.
    pub(super) offset: Point,
    /// Whether the client asked for the cursor to be in the copy.
    pub(super) with_cursor: bool,
}

impl Pixels<'_> {
    /// The tick the pixels of `target` came from.
    ///
    /// It is the presentation time a protocol reports: a capture hands over the frame
    /// that reached the screen, so its tick is what a client uses to pace itself rather
    /// than the tick of its request.
    pub(super) fn presented(&self, target: Target) -> Option<Tick> {
        self.frames.get(&self.targets.output(target)?).map(|cached| cached.tick)
    }
}

impl PixelSource for Pixels<'_> {
    fn bounds(&self, target: Target) -> Option<Rect> {
        self.targets.bounds(target)
    }

    fn copy(&mut self, target: Target, region: Rect, _format: Format) -> Copied {
        let Some(output) = self.targets.output(target) else {
            return Copied::Nothing;
        };
        let Some(cached) = self.frames.get(&output) else {
            return Copied::Nothing;
        };
        let origin = self.targets.origin(target);
        let from = Point::new(
            origin.x.saturating_add(self.offset.x),
            origin.y.saturating_add(self.offset.y),
        );
        let image = match (self.with_cursor, cached.cursored.as_ref()) {
            (true, Some(cursored)) => cursored,
            _ => &cached.image,
        };
        match copy_into_shm(image, region, from, &self.buffer) {
            Ok(_) => Copied::Copied,
            Err(error) => {
                warn!(%error, %target, "[capture] the frame could not be written into the client's buffer");
                Copied::Nothing
            }
        }
    }
}

/// The pixels a PipeWire stream reads: the last frame of each output, cropped to the target
/// the stream serves, written into one plane of a stream buffer.
struct PlanePixels<'a> {
    /// Where every target is.
    targets: Targets,
    /// The last frame of every output.
    frames: &'a BTreeMap<OutputId, Cached>,
}

impl PlaneSource for PlanePixels<'_> {
    fn bounds(&self, target: Target) -> Option<Rect> {
        self.targets.bounds(target)
    }

    fn copy_into(&mut self, target: Target, region: Rect, format: Format, plane: &mut [u8]) -> Copied {
        let Some(output) = self.targets.output(target) else {
            return Copied::Nothing;
        };
        let Some(cached) = self.frames.get(&output) else {
            return Copied::Nothing;
        };
        let origin = self.targets.origin(target);
        write_region(&cached.image, region.translate(origin.x, origin.y), format, plane)
    }
}

impl State {
    /// The lock's answer for `stream` right now.
    pub(crate) fn lock_capture(&self, stream: CaptureStream) -> Capture {
        self.lock.capture(stream)
    }

    /// Starts the PipeWire stream for `session` and answers its node id, for the connection
    /// that owns the session. The table decides whether the session may stream at all.
    #[allow(dead_code)]
    pub(crate) fn pipewire_start(&mut self, client: &WaylandClientId, session: SessionId) -> Result<u32, ExportError> {
        let owner = self.capture.owner(client);
        self.capture.pipewire.start(&self.capture.table, session, owner)
    }

    /// The node id of `session`, answered only to the connection that opened it.
    #[allow(dead_code)]
    pub(crate) fn pipewire_node(&mut self, client: &WaylandClientId, session: SessionId) -> Result<u32, ExportError> {
        let owner = self.capture.owner(client);
        self.capture.pipewire.node_id(&self.capture.table, session, owner)
    }

    /// Runs once per loop iteration: advances the capture tick and cuts every session the
    /// lock has covered.
    pub(crate) fn poll_capture(&mut self) {
        self.capture.tick = self.capture.tick.saturating_add(1);
        if !self.capture_allowed() && self.capture.stop_all() {
            self.publish();
        }
        if self.capture.pipewire.is_live() {
            let capture = self.lock_capture(CaptureStream::Granted);
            let tick = self.capture.tick;
            let mut source = PlanePixels {
                targets: self.capture_targets(),
                frames: &self.capture.pixels,
            };
            self.capture
                .pipewire
                .pump(&mut self.capture.table, tick, capture, &mut source);
        }
    }

    /// Reads `output` back so a capture can copy the frame that reached the screen.
    ///
    /// The frame hook calls this after the output presented. The readback is
    /// unconditional, and that is what makes a capture correct on an output nothing has
    /// drawn since: a session opened between two frames must still return the frame that
    /// is on screen, which only exists here if every presented frame was kept. It is a
    /// whole-buffer copy per drawn frame while the compositor's own target is the
    /// software buffer; the renderer lane can drop it by lending capture the software
    /// buffer instead of a copy of it.
    pub(crate) fn capture_frame(&mut self, backend: &mut dyn gnoblin_backend::Backend<State>, output: OutputId) {
        let Some(image) = backend.capture(output, false) else {
            return;
        };
        // The cursor is only drawn on an output the pointer is over, so the second
        // readback is paid only there.
        let cursored = if self.pointer_cursor(output).is_some() {
            backend.capture(output, true)
        } else {
            None
        };
        let tick = self.capture.tick;
        self.capture.pixels.insert(output, Cached { image, cursored, tick });
    }

    /// Tells capture that part of an output changed, so the next frame reports it.
    pub(crate) fn note_capture_damage(&mut self, output: OutputId, damage: &[Rect]) {
        if self.capture.table.is_empty() {
            return;
        }
        let target = Target::Output(output);
        for rect in damage {
            self.capture.table.note_damage(target, *rect);
        }
    }

    /// Where a target is now, in its own physical pixels.
    pub(super) fn capture_bounds(&self, target: Target) -> Option<Rect> {
        self.capture_targets().bounds(target)
    }

    /// Where every target is now.
    pub(super) fn capture_targets(&self) -> Targets {
        let mut targets = Targets::default();
        for output in self.output_ids() {
            let (Some(rect), Some(scale)) = (self.output_rect(output), self.output_scale(output)) else {
                continue;
            };
            targets.outputs.insert(output, scale_size(rect.size(), scale));
        }
        for managed in self.wm().windows() {
            let shown = self.shown_geometry(managed);
            let window = managed.id;
            let Some(output) = managed.output else {
                continue;
            };
            let (Some(monitor), Some(scale)) = (self.monitor_of(window), self.output_scale(output)) else {
                continue;
            };
            targets.windows.insert(
                window,
                CapturedPlacement {
                    output,
                    origin: Point::new(
                        scale_axis(shown.x.saturating_sub(monitor.x), scale),
                        scale_axis(shown.y.saturating_sub(monitor.y), scale),
                    ),
                    size: scale_size(shown.size(), scale),
                },
            );
        }
        targets
    }

    /// Copies `region` of an output into `buffer` through the session rules.
    ///
    /// One screencopy object is one frame, so the session lasts exactly this copy; the
    /// damage it reports is therefore the whole region, which is what this protocol's
    /// clients expect.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_into_buffer(
        &mut self,
        owner: ClientId,
        output: OutputId,
        buffer: &WlBuffer,
        format: Format,
        offset: Point,
        with_cursor: bool,
        capture: Capture,
        tick: Tick,
    ) -> Option<(Rect, Tick)> {
        let session = self
            .capture
            .table
            .open(owner, Target::Output(output), CaptureStream::Requested, format);
        let frame = match self.capture.table.request(session, BufferId::new(1), tick) {
            Ok(frame) => frame,
            Err(refusal) => {
                self.capture.table.close(session);
                debug!(reason = refusal.as_str(), "[capture] the frame could not be claimed");
                return None;
            }
        };
        let targets = self.capture_targets();
        let mut source = Pixels {
            targets,
            frames: &self.capture.pixels,
            buffer: buffer.clone(),
            offset,
            with_cursor,
        };
        let target = Target::Output(output);
        let presented = source.presented(target).unwrap_or(tick);
        let outcome = self.capture.table.capture(frame, tick, capture, &mut source);
        self.capture.table.close(session);
        match outcome {
            CaptureOutcome::Copied(report) => Some((report.region, presented)),
            CaptureOutcome::Refused(refusal) => {
                debug!(reason = refusal.as_str(), "[capture] the frame was refused");
                None
            }
        }
    }
}

/// A logical extent as physical pixels.
fn scale_size(size: Size, scale: f64) -> Size {
    Size::new(scale_extent(size.width, scale), scale_extent(size.height, scale))
}

/// One logical extent as physical pixels.
fn scale_extent(value: u32, scale: f64) -> u32 {
    let physical = f64::from(value) * scale;
    if physical.is_finite() && physical > 0.0 {
        physical.round() as u32
    } else {
        0
    }
}

/// One logical coordinate as a physical one.
fn scale_axis(value: i32, scale: f64) -> i32 {
    let physical = f64::from(value) * scale;
    if physical.is_finite() {
        physical.round() as i32
    } else {
        0
    }
}
