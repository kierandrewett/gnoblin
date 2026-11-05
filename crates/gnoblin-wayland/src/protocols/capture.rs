//! `ext_image_copy_capture_v1`, its capture sources and `zwlr_screencopy_manager_v1`.
//!
//! Three globals serve one path. `ext-image-copy-capture-v1` is the modern protocol,
//! `ext-image-capture-source-v1` names what to capture, and
//! `zwlr_screencopy_manager_v1` is what shells and tools use today. Both protocols hand
//! their frames to the same rules in `gnoblin-capture` and to the same pixel copy; the
//! middle of this module is that shared part. [`Pixels`] implements
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
//! Both protocols ask `gnoblin-capture` at every frame, and that crate applies the
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::backend::{ClientId as WaylandClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::utils::{Buffer as BufferSpace, Rectangle, Transform, user_data::UserDataMap};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::image_capture_source::{
    ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler, OutputCaptureSourceState,
    ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
};
use smithay::wayland::image_copy_capture::{
    BufferConstraints, CaptureFailureReason, Frame as CaptureFrame, ImageCopyCaptureHandler, ImageCopyCaptureState,
    Session as CaptureSession, SessionRef,
};
use smithay::wayland::shm;
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use tracing::{debug, warn};

use gnoblin_backend::render::Image;
use gnoblin_backend::render::copy_into_shm;
use gnoblin_capture::{
    BufferId, CaptureSource as PixelSource, Copied, Format, Outcome, PixelFormat, Refusal, Session, SessionId, Stop,
    Table, Target,
};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_types::{ClientId, OutputId, Point, Rect, Size, Tick, WindowId};

use crate::State;
use crate::access::{Access, AccessPolicy, Global};

/// The last frame an output presented, which is what a capture copies out of.
#[derive(Clone, Debug)]
struct Cached {
    /// The pixels, in physical output pixels.
    image: Image,
    /// The tick the frame was presented at.
    tick: Tick,
}

/// Where every capturable target is right now.
///
/// A snapshot rather than a borrow of the compositor: the copy path needs geometry and
/// the cached pixels at the same time as it needs the session table mutably, and a
/// snapshot of a handful of rectangles is cheaper than the alternative borrow puzzle.
#[derive(Clone, Debug, Default)]
struct Targets {
    /// Every output, in physical pixels.
    outputs: BTreeMap<OutputId, Size>,
    /// Every window: the output it is drawn on and its physical origin in that frame.
    windows: BTreeMap<WindowId, WindowPlacement>,
}

/// Where a window is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowPlacement {
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
struct SessionKey(SessionId);

/// The session's capture state.
pub(crate) struct CaptureTables {
    /// The session rules and every open session.
    table: Table,
    /// The last frame each output drew, replaced once per frame while a session is open.
    pixels: BTreeMap<OutputId, Cached>,
    /// The tick the loop is on, so a stream is paced by iterations rather than by a
    /// clock nothing else in this crate reads.
    tick: Tick,
    /// The `ext-image-capture-source-v1` objects, mapped to what they capture.
    sources: BTreeMap<usize, Target>,
    /// The sessions `ext-image-copy-capture` created, kept alive so their clients keep
    /// their constraints and hear `stopped` when they end.
    sessions: Vec<CaptureSession>,
    /// The screencopy frame objects that have not been resolved, so the lock can fail
    /// them.
    screencopy_frames: Vec<ZwlrScreencopyFrameV1>,
    /// The identity this session gave each connection, so a session's owner is one value
    /// per connection rather than a token nothing else agrees on.
    owners: HashMap<WaylandClientId, ClientId>,
    /// The next connection identity.
    next_owner: u64,
    /// The manager global, kept for its identity and for the filter it carries.
    #[allow(dead_code)]
    screencopy_global: GlobalId,
    /// The image-copy-capture globals and their sessions.
    image_copy: ImageCopyCaptureState,
    /// The output capture sources.
    output_source: OutputCaptureSourceState,
    /// The toplevel capture sources.
    toplevel_source: ToplevelCaptureSourceState,
    /// The image-copy-capture manager's identity, so a disabled configuration can remove
    /// the global rather than skip it.
    image_copy_global: GlobalId,
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
            ScreencopyGlobal {
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
    fn owner(&mut self, client: &WaylandClientId) -> ClientId {
        if let Some(token) = self.owners.get(client) {
            return *token;
        }
        self.next_owner = self.next_owner.saturating_add(1);
        let token = ClientId::new(self.next_owner);
        self.owners.insert(client.clone(), token);
        token
    }

    /// What a source object captures.
    fn target_of(&self, source: &ImageCaptureSource) -> Option<Target> {
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
    fn stop(&mut self, session: SessionId, reason: Stop) {
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

/// The window a published toplevel identifier names.
///
/// The list's identifier is `gnoblin-` followed by the window's sixteen hex digits; a
/// handle that does not parse is one this session did not publish, which is refused rather
/// than guessed at.
fn window_of_identifier(identifier: &str) -> Option<WindowId> {
    let digits = identifier.strip_prefix("gnoblin-")?;
    u64::from_str_radix(digits, 16).ok().map(WindowId::new)
}

/// The session number stored in a session object's user data.
fn key_of(data: &UserDataMap) -> Option<SessionId> {
    data.get::<SessionKey>().map(|key| key.0)
}

/// The `zwlr_screencopy_manager_v1` global's data.
struct ScreencopyGlobal {
    /// The session's roster, which answers whether a connection may bind it.
    access: Arc<Access>,
}

impl std::fmt::Debug for ScreencopyGlobal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ScreencopyGlobal").finish_non_exhaustive()
    }
}

impl GlobalDispatch2<ZwlrScreencopyManagerV1, State> for ScreencopyGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ScreencopyManager);
    }

    fn can_view(&self, client: &Client) -> bool {
        AccessPolicy::allows(&*self.access, &client.id(), Global::Screencopy)
    }
}

/// The manager carries no state of its own: every frame it creates does.
#[derive(Debug)]
struct ScreencopyManager;

impl Dispatch2<ZwlrScreencopyManagerV1, State> for ScreencopyManager {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                // No cursor is composed into a capture yet: the seat has no theme
                // loader, so there is nothing to overlay and the flag changes nothing.
                let _ = overlay_cursor;
                create_screencopy_frame(state, client, frame, &output, None, data_init);
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => {
                let _ = overlay_cursor;
                let region = Rect::new(
                    x,
                    y,
                    u32::try_from(width).unwrap_or(0),
                    u32::try_from(height).unwrap_or(0),
                );
                create_screencopy_frame(state, client, frame, &output, Some(region), data_init);
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

/// One `zwlr_screencopy_frame_v1` object.
#[derive(Debug)]
struct ScreencopyFrame {
    /// The connection that created it, so its session has an owner.
    owner: ClientId,
    /// The output it captures.
    output: OutputId,
    /// The region, in the output's physical pixels.
    region: Rect,
    /// The buffer geometry the client was told to allocate.
    format: Format,
    /// Whether the object has already copied, which the protocol forbids repeating.
    used: AtomicBool,
}

impl Dispatch2<ZwlrScreencopyFrameV1, State> for ScreencopyFrame {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => self.copy(state, frame, &buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => self.copy(state, frame, &buffer, true),
            zwlr_screencopy_frame_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: WaylandClientId, resource: &ZwlrScreencopyFrameV1) {
        state.capture.screencopy_frames.retain(|frame| frame != resource);
    }
}

impl ScreencopyFrame {
    /// Answers a copy request.
    ///
    /// One object is one copy: a second request is the protocol's `already_used` error,
    /// so a client cannot make the compositor write into a buffer it has already read.
    fn copy(&self, state: &mut State, resource: &ZwlrScreencopyFrameV1, buffer: &WlBuffer, with_damage: bool) {
        if self.used.swap(true, Ordering::Relaxed) {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "this frame has already been copied",
            );
            return;
        }
        state.capture.screencopy_frames.retain(|frame| frame != resource);
        // Checked again at the copy boundary: a frame object created before the cover
        // went up must never expose the desktop after it did.
        if !state.capture_allowed() {
            debug!(output = %self.output, "[capture] a screencopy frame is refused during the lock");
            resource.failed();
            state.flush();
            return;
        }
        if !self.buffer_matches(buffer) {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "the buffer does not match the advertised geometry",
            );
            return;
        }
        let capture = state.lock_capture(CaptureStream::Requested);
        let tick = state.capture.tick;
        let offset = Point::new(self.region.x, self.region.y);
        match state.copy_into_buffer(self.owner, self.output, buffer, self.format, offset, capture, tick) {
            Some((copied, presented)) => {
                if with_damage {
                    // One rectangle for the whole frame: the whole buffer was written,
                    // so the copy that happened is the damage, not the scene's.
                    resource.damage(0, 0, copied.width, copied.height);
                }
                resource.flags(zwlr_screencopy_frame_v1::Flags::empty());
                let (high, low, nanos) = timestamp(presented);
                resource.ready(high, low, nanos);
            }
            None => resource.failed(),
        }
        state.flush();
    }

    /// Whether the client's buffer is the geometry the frame advertised.
    fn buffer_matches(&self, buffer: &WlBuffer) -> bool {
        let expected = (
            self.format.size().width,
            self.format.size().height,
            self.format.stride(),
        );
        shm::with_buffer_contents(buffer, |_pointer, _length, data| {
            (
                u32::try_from(data.width).unwrap_or(0),
                u32::try_from(data.height).unwrap_or(0),
                u32::try_from(data.stride).unwrap_or(0),
                data.format,
            ) == (expected.0, expected.1, expected.2, wl_shm::Format::Argb8888)
        })
        .unwrap_or(false)
    }
}

/// The format a frame that cannot be captured is created with: one opaque pixel, so the
/// object exists and every copy of it fails.
fn placeholder() -> Format {
    Format::new(Size::new(1, 1), 4, PixelFormat::Argb8888).expect("a one-pixel format is valid")
}

/// Creates a screencopy frame for `output`, or answers it `failed` when it cannot be
/// captured.
fn create_screencopy_frame(
    state: &mut State,
    client: &Client,
    frame: New<ZwlrScreencopyFrameV1>,
    output: &WlOutput,
    region: Option<Rect>,
    data_init: &mut DataInit<'_, State>,
) {
    let owner = state.capture.owner(&client.id());
    // Creating the frame needs the manager's data, which is not the client's; the
    // identity is resolved first so the borrow of the session's owner table ends here.
    let output = SmithayOutput::from_resource(output)
        .and_then(|output| state.output_id_of(&output))
        .filter(|output| state.output_rect(*output).is_some());
    // No usable buffer is advertised while the cover is up, and an output this session
    // does not drive has no pixels to offer: both answer `failed` before the client
    // allocates anything.
    let refused = output.is_none() || !state.capture_allowed();
    let bounds = output.and_then(|output| state.capture_bounds(Target::Output(output)));
    let region = match (bounds, region) {
        (Some(bounds), Some(region)) => region.intersection(bounds),
        (Some(bounds), None) => Some(bounds),
        (None, _) => None,
    };
    if refused || region.is_none() {
        let resource = data_init.init(
            frame,
            ScreencopyFrame {
                owner,
                output: output.unwrap_or_default(),
                region: Rect::EMPTY,
                format: placeholder(),
                used: AtomicBool::new(true),
            },
        );
        resource.failed();
        state.flush();
        return;
    }
    let (Some(output), Some(region)) = (output, region) else {
        return;
    };
    let Some(format) = Format::new(region.size(), region.width.saturating_mul(4), PixelFormat::Argb8888) else {
        let resource = data_init.init(
            frame,
            ScreencopyFrame {
                owner,
                output,
                region,
                format: placeholder(),
                used: AtomicBool::new(true),
            },
        );
        resource.failed();
        state.flush();
        return;
    };
    let resource = data_init.init(
        frame,
        ScreencopyFrame {
            owner,
            output,
            region,
            format,
            used: AtomicBool::new(false),
        },
    );
    resource.buffer(
        wl_shm::Format::Argb8888,
        format.size().width,
        format.size().height,
        format.stride(),
    );
    if resource.version() >= 3 {
        resource.buffer_done();
    }
    state.capture.screencopy_frames.push(resource);
    state.flush();
}

/// A tick as the `ready` event's `tv_sec_hi`, `tv_sec_lo` and `tv_nsec`.
fn timestamp(tick: Tick) -> (u32, u32, u32) {
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
struct Pixels<'a> {
    /// Where every target is.
    targets: Targets,
    /// The last frame of every output.
    frames: &'a BTreeMap<OutputId, Cached>,
    /// The buffer the copy writes into.
    buffer: WlBuffer,
    /// Where the copied region starts inside the target, in physical pixels.
    ///
    /// A whole-target capture starts at its origin. A screencopy of a region starts at
    /// the region's corner, which is why the same path can serve both.
    offset: Point,
}

impl Pixels<'_> {
    /// The tick the pixels of `target` came from.
    ///
    /// It is the presentation time a protocol reports: a capture hands over the frame
    /// that reached the screen, so its tick is what a client uses to pace itself rather
    /// than the tick of its request.
    fn presented(&self, target: Target) -> Option<Tick> {
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
        let source = region.translate(
            origin.x.saturating_add(self.offset.x),
            origin.y.saturating_add(self.offset.y),
        );
        match copy_into_shm(&cached.image, source, &self.buffer) {
            Ok(_) => Copied::Copied,
            Err(error) => {
                warn!(%error, %target, "[capture] the frame could not be written into the client's buffer");
                Copied::Nothing
            }
        }
    }
}

impl State {
    /// The lock's answer for `stream` right now.
    pub(crate) fn lock_capture(&self, stream: CaptureStream) -> Capture {
        self.lock.capture(stream)
    }

    /// Runs once per loop iteration: advances the capture tick and cuts every session the
    /// lock has covered.
    pub(crate) fn poll_capture(&mut self) {
        self.capture.tick = self.capture.tick.saturating_add(1);
        if !self.capture_allowed() && self.capture.stop_all() {
            self.publish();
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
        let Some(image) = backend.screenshot(output) else {
            return;
        };
        let tick = self.capture.tick;
        self.capture.pixels.insert(output, Cached { image, tick });
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
    fn capture_bounds(&self, target: Target) -> Option<Rect> {
        self.capture_targets().bounds(target)
    }

    /// Where every target is now.
    fn capture_targets(&self) -> Targets {
        let mut targets = Targets::default();
        for output in self.output_ids() {
            let (Some(rect), Some(scale)) = (self.output_rect(output), self.output_scale(output)) else {
                continue;
            };
            targets.outputs.insert(output, scale_size(rect.size(), scale));
        }
        for managed in self.wm().windows() {
            let window = managed.id;
            let Some(output) = managed.output else {
                continue;
            };
            let (Some(monitor), Some(scale)) = (self.monitor_of(window), self.output_scale(output)) else {
                continue;
            };
            targets.windows.insert(
                window,
                WindowPlacement {
                    output,
                    origin: Point::new(
                        scale_axis(managed.geometry.x.saturating_sub(monitor.x), scale),
                        scale_axis(managed.geometry.y.saturating_sub(monitor.y), scale),
                    ),
                    size: scale_size(managed.geometry.size(), scale),
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
    fn copy_into_buffer(
        &mut self,
        owner: ClientId,
        output: OutputId,
        buffer: &WlBuffer,
        format: Format,
        offset: Point,
        capture: Capture,
        tick: Tick,
    ) -> Option<(Rect, Tick)> {
        let session = self
            .capture
            .table
            .open(owner, Target::Output(output), CaptureStream::Requested, format);
        let mut frame = match self.capture.table.request(session, BufferId::new(1), tick) {
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
        };
        let target = Target::Output(output);
        let presented = source.presented(target).unwrap_or(tick);
        let outcome = self.capture.table.capture(&mut frame, tick, capture, &mut source);
        self.capture.table.close(session);
        match outcome {
            Outcome::Copied(report) => Some((report.region, presented)),
            Outcome::Refused(refusal) => {
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
            Outcome::Copied(report) => {
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
                frame.success(Transform::Normal, damage, Duration::from_millis(presented.get()));
            }
            Outcome::Refused(refusal) => {
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
            warn!("[capture] a capture source names an output this session does not drive");
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
        // its identity (`protocols::foreign_toplevel`), so a source names a window without
        // capture keeping a second table that could disagree with the shell's.
        let Some(window) = window_of_identifier(&toplevel.identifier()) else {
            warn!(
                identifier = %toplevel.identifier(),
                "[capture] a capture source names a toplevel this session does not publish"
            );
            return;
        };
        self.capture.sources.insert(source.id(), Target::Window(window));
    }
}
