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
//! 6. **A View stream shows a secure node, or outlives its lock, View or consent.** A PipeWire stream of a View
//!    reads the picture a View capture reads (`view_pictures`, secure nodes already masked), is cut by the lock
//!    whether or not anything reads it, ends when the View is destroyed, and is only started for a session the user
//!    approved: `a_cast_of_a_view_masks_a_secure_window`, `the_session_lock_ends_a_cast_of_a_view_and_refuses_a_new_one`,
//!    `a_cast_of_a_view_ends_cleanly_when_the_view_is_destroyed` (`tests/view_capture_e2e.rs`) and
//!    `a_view_start_with_no_recorded_consent_is_refused_and_makes_no_node` (`gnoblin-compositor/tests/cast.rs`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::image_copy_session::{Session as ImageCopySession, create_image_copy_global};
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

use gnoblin_capture::{
    BufferId, CaptureOutcome, CaptureSource as PixelSource, Copied, Format, FrameClaim, PixelFormat, SessionId, Stop,
    Table, Target,
};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shm::copy_into;
use gnoblin_types::{ClientId, NodeId, OutputId, Point, Rect, Size, Tick, WindowId};

use super::zwlr_screencopy_v1::ZwlrScreencopyV1Global;
use crate::State;
use crate::access::{Access, AccessPolicy, Global};
use gnoblin_pipewire_export::{PipewireExport, PlaneSource, write_region};

use crate::log;

/// Captures into a client's GPU buffer: what is offered, and the copies the loop answers.
#[path = "capture_dmabuf.rs"]
mod dmabuf;
pub(super) use dmabuf::ScreencopyPending;

/// The pictures of the Views a renderer draws on the GPU, read back for the consumers that need bytes.
#[path = "capture_views.rs"]
mod views;
pub(crate) use views::PendingViewCopy;

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
    /// Every View, at the size it is drawn, in physical pixels.
    views: BTreeMap<NodeId, Size>,
    /// The picture each View drew last, which is what a View capture copies.
    view_pictures: BTreeMap<NodeId, Arc<Image>>,
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
            Target::View(view) => self.views.get(&view).map(|size| Rect::from_size(*size)),
        }
    }

    /// The output whose frame a target is copied out of.
    fn output(&self, target: Target) -> Option<OutputId> {
        match target {
            Target::Output(output) => Some(output),
            Target::Window(window) => self.windows.get(&window).map(|placement| placement.output),
            Target::View(_) => None,
        }
    }

    /// The picture a View capture copies from, when the View has drawn one.
    fn view_picture(&self, view: NodeId) -> Option<&Arc<Image>> {
        self.view_pictures.get(&view)
    }

    /// Where a target's own origin sits in its output's frame.
    fn origin(&self, target: Target) -> Point {
        match target {
            Target::Output(_) | Target::View(_) => Point::new(0, 0),
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
    pub(crate) table: Table,
    /// The last frame each output drew, replaced once per frame while a session is open.
    pub(super) pixels: BTreeMap<OutputId, Cached>,
    /// The tick the loop is on, so a stream is paced by iterations rather than by a
    /// clock nothing else in this crate reads.
    pub(super) tick: Tick,
    /// The `ext-image-capture-source-v1` objects, mapped to what they capture.
    pub(super) sources: BTreeMap<usize, Target>,
    /// The Views of the scene, at the size each is drawn: refreshed once per loop iteration, and again by each
    /// view-source `create_source`, which is refused for any node not in this set.
    pub(super) views: BTreeMap<NodeId, Size>,
    /// The picture each View drew on the frame the loop last published. A View capture
    /// copies from it, and a picture that is not the one held here is new damage.
    pub(super) view_pictures: BTreeMap<NodeId, Arc<Image>>,
    /// The picture of each View drawn on the GPU that a consumer read back, with the revision of the scene it was
    /// read from. It is dropped when the composer draws the View again.
    pub(crate) view_readbacks: BTreeMap<NodeId, (u64, Arc<Image>)>,
    /// The shell's `capture` calls on Views drawn on the GPU, which wait for the loop to read the picture back.
    pub(crate) view_stills: Vec<gnoblin_render::transaction::TreeCall>,
    /// The frames of sessions of Views drawn on the GPU that were asked for before the View had a picture.
    pub(crate) view_copies: Vec<PendingViewCopy>,
    /// The sessions `ext-image-copy-capture` created, kept alive so their clients keep
    /// their constraints and hear `stopped` when they end.
    pub(super) sessions: Vec<ImageCopySession>,
    /// The screencopy frame objects that have not been resolved, so the lock can fail
    /// them.
    pub(super) screencopy_frames: Vec<ZwlrScreencopyFrameV1>,
    /// The identity this session gave each connection, so a session's owner is one value
    /// per connection rather than a token nothing else agrees on.
    owners: HashMap<WaylandClientId, ClientId>,
    /// The identity this session gave each IPC connection that holds a consent, drawn from the
    /// same counter as [`Self::owners`] so the two never name one owner.
    portal_owners: HashMap<ClientId, ClientId>,
    /// The next connection identity.
    next_owner: u64,
    /// The manager global, kept for its identity and for the filter it carries.
    #[allow(dead_code)]
    screencopy_global: GlobalId,
    /// What the backend can write a capture into, and the copies of GPU buffers waiting for the loop.
    dmabuf: dmabuf::DmabufCapture,
    /// The output capture sources.
    pub(super) output_source: OutputCaptureSourceState,
    /// The toplevel capture sources.
    pub(super) toplevel_source: ToplevelCaptureSourceState,
    /// The image-copy-capture manager's identity, so a disabled configuration can remove
    /// the global rather than skip it.
    image_copy_global: GlobalId,
    /// The PipeWire streams of the granted capture sessions.
    pipewire: PipewireExport,
    /// The directory a View still may be written under: the session's runtime directory, or `None` when the session
    /// has none of its own and stills have nowhere to go.
    still_root: Option<std::path::PathBuf>,
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
        let output_source =
            OutputCaptureSourceState::new_with_filter::<State, _>(display, filter(Global::ImageCopyCapture));
        let toplevel_source =
            ToplevelCaptureSourceState::new_with_filter::<State, _>(display, filter(Global::ImageCopyCapture));
        let image_copy_global = create_image_copy_global(display, filter(Global::ImageCopyCapture));
        let screencopy_global = display.create_global::<State, ZwlrScreencopyManagerV1, _>(
            3,
            ZwlrScreencopyV1Global {
                access: Arc::clone(access),
            },
        );
        super::gnoblin_view_source_v1::create_global(display, access);
        Self {
            table: Table::new(),
            pixels: BTreeMap::new(),
            tick: Tick::ZERO,
            sources: BTreeMap::new(),
            views: BTreeMap::new(),
            view_pictures: BTreeMap::new(),
            view_readbacks: BTreeMap::new(),
            view_stills: Vec::new(),
            view_copies: Vec::new(),
            sessions: Vec::new(),
            screencopy_frames: Vec::new(),
            owners: HashMap::new(),
            portal_owners: HashMap::new(),
            next_owner: 0,
            screencopy_global,
            image_copy_global,
            dmabuf: dmabuf::DmabufCapture::default(),
            output_source,
            toplevel_source,
            pipewire: PipewireExport::new(),
            still_root: None,
        }
    }

    /// Sets the directory a View still may be written under.
    pub(crate) fn set_still_root(&mut self, root: Option<std::path::PathBuf>) {
        self.still_root = root;
    }

    /// The directory a View still may be written under, if the session has one.
    pub(crate) fn still_root(&self) -> Option<&std::path::Path> {
        self.still_root.as_deref()
    }

    /// The tick the loop is on.
    pub(crate) fn tick(&self) -> Tick {
        self.tick
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

    /// The identity this session gives an IPC connection that holds a consent, so a stream it
    /// started is one value per connection and cannot collide with a Wayland client's.
    pub(super) fn portal_owner(&mut self, client: ClientId) -> ClientId {
        if let Some(token) = self.portal_owners.get(&client) {
            return *token;
        }
        self.next_owner = self.next_owner.saturating_add(1);
        let token = ClientId::new(self.next_owner);
        self.portal_owners.insert(client, token);
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
        let ended = self.target_closed(Target::Window(window));
        if ended > 0 {
            log::debug!(area = "capture", %window, ended, "the window's sessions ended");
        }
    }

    /// Ends the capture sessions of every View that is no longer in the scene, and answers whether any ended.
    ///
    /// A session of a View that was removed can never produce a frame, and an app that holds its stream should hear
    /// that the stream is over now rather than when it next asks for a frame. The privacy indicator counts sessions,
    /// so a stream that outlived its View would keep it lit for nothing.
    fn views_closed(&mut self) -> bool {
        let gone: BTreeSet<NodeId> = self
            .table
            .sessions()
            .filter_map(|session| match session.target() {
                Target::View(view) if !self.views.contains_key(&view) => Some(view),
                _ => None,
            })
            .collect();
        let mut ended = 0;
        for view in gone {
            ended += self.target_closed(Target::View(view));
            log::debug!(area = "capture", view = ?view, "the View's sessions ended");
        }
        ended > 0
    }

    /// Removes `target`'s capture sources and ends its sessions, answering how many ended.
    fn target_closed(&mut self, target: Target) -> usize {
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
        ended
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

    /// Ends, as [`Stop::Secure`], every live PipeWire cast whose target `reaches` answers true for: a cast of a node
    /// the frame just marked secure must not run on. Its stream is disconnected and the daemon synced before this
    /// returns, so the caller can acknowledge the frame's replies only after no more buffers can reach a consumer.
    /// A screencopy or image-copy session is not ended: it keeps delivering the treatment the mark selects.
    pub(crate) fn stop_targets_reaching(&mut self, reaches: impl Fn(Target) -> bool) {
        let ended: Vec<SessionId> = self
            .table
            .sessions()
            .filter(|session| {
                session.stopped().is_none() && session.stream() == CaptureStream::Granted && reaches(session.target())
            })
            .map(|session| session.id())
            .collect();
        for session in ended {
            self.stop(session, Stop::Secure);
        }
        self.pipewire.reap(&self.table);
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
        log::debug!(area = "capture", "every session stopped for the lock");
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

    /// Copies `region` of `target` into the client's buffer.
    ///
    /// The region is honoured for every target: only the pixels of `region` are written, at the same place in the
    /// buffer, and the rest of the buffer is left as the client had it. The table never asks for more than the target
    /// has or the session negotiated, and a View that shrank below its session's size is copied as the smaller region.
    ///
    /// `_format` is the session's negotiated geometry and is not what converts the pixels. The conversion is the
    /// buffer's own: [`copy_into`] writes `argb8888` or `xrgb8888` according to the `wl_shm` format the client made
    /// the buffer with (`xrgb8888` forces an opaque alpha), and refuses any other layout. A buffer whose format the
    /// session never offered, or that is smaller than the size it advertised, is refused by the protocol layer before
    /// this runs, with the frame's `failed` event: `a_buffer_smaller_than_the_view_is_refused_and_left_untouched`.
    fn copy(&mut self, target: Target, region: Rect, _format: Format) -> Copied {
        if let Target::View(view) = target {
            // A View is copied from its own picture: it has no output frame, no
            // origin inside one, and no cursor drawn over it.
            let Some(picture) = self.targets.view_picture(view) else {
                return Copied::Nothing;
            };
            return match copy_into(picture, region, Point::new(0, 0), &self.buffer) {
                Ok(_) => Copied::Copied,
                Err(error) => {
                    log::warn!(area = "capture", %error, %target, "the View picture could not be written into the client's buffer");
                    Copied::Nothing
                }
            };
        }
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
        match copy_into(image, region, from, &self.buffer) {
            Ok(_) => Copied::Copied,
            Err(error) => {
                log::warn!(area = "capture", %error, %target, "the frame could not be written into the client's buffer");
                Copied::Nothing
            }
        }
    }
}

/// The pixels a PipeWire stream reads, written into one plane of a stream buffer: the last frame of an output
/// cropped to the target the stream serves, or the picture a View drew last.
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
        if let Target::View(view) = target {
            // The View's capture picture: the one a View capture reads, with every secure node already masked.
            // It has no output frame, no origin inside one and no cursor.
            let Some(picture) = self.targets.view_picture(view) else {
                return Copied::Nothing;
            };
            return write_region(picture, region, format, plane);
        }
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

/// A screen cast stream the compositor started for a session the user approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CastStream {
    /// The capture session behind the stream, which [`ControlHandle::stop_cast`](crate::ControlHandle::stop_cast) ends.
    pub session: SessionId,
    /// The PipeWire node the app reads.
    pub node: u32,
    /// The top-left corner of what the stream shows, in layout pixels: an output's or a window's place. A View has
    /// none, so it is the origin.
    pub origin: Point,
    /// How large what the stream shows is in the layout, in layout pixels. It differs from [`CastStream::size`] on
    /// an output or a window drawn at a scale above one.
    pub extent: Size,
    /// The stream's size, in pixels.
    pub size: Size,
}

/// Why a screen cast stream was not started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CastRefusal {
    /// The session lock covers the desktop, so no new stream may be opened.
    Locked,
    /// The target is not one the session has now: no such output, window or View, or it has no size.
    NoTarget,
    /// The stream could not be made: there is no PipeWire daemon, or it refused the stream.
    Unavailable(String),
}

impl State {
    /// The lock's answer for `stream` right now.
    pub(crate) fn lock_capture(&self, stream: CaptureStream) -> Capture {
        self.lock.capture(stream)
    }

    /// Starts the PipeWire stream of a screen cast the user approved.
    ///
    /// This is the one place a [`CaptureStream::Granted`] session is opened: the caller is the
    /// IPC connection that holds the user's consent (`cast.start`, which refuses a session no
    /// consent was recorded for), `client` is that connection, and the session it opens
    /// belongs to it alone. The session ends with it, with the target, and with the lock.
    /// A session that is already locked is refused, because a user who consented to a cast
    /// did not consent to one of a locked desktop.
    ///
    /// The target is an output, a window or a View. A window's stream carries the pixels of the output frame inside
    /// the window's rectangle, which is the capture a toplevel source gives, so a secure window or an output-wide
    /// secure layer is what an output capture shows of it (the session's treatment, never the window's pixels) and
    /// the stream follows the window when it moves or is resized. A View's stream carries the picture the View's
    /// capture reads, at the size the View is drawn at. Starting a stream is an audited act, and it is a session in
    /// the table like any other, so the privacy indicator is lit from the moment this answers.
    pub(crate) fn start_cast(&mut self, client: ClientId, target: Target) -> Result<CastStream, CastRefusal> {
        if self.lock_capture(CaptureStream::Granted) != Capture::Desktop || !self.capture_allowed() {
            log::info!(area = "capture", %target, "a screen cast was refused: the session is locked");
            return Err(CastRefusal::Locked);
        }
        // A View the shell made since the loop last looked is a target now, as it is for a view-source session.
        self.capture.views = self.composer.views().into_iter().collect();
        let Some(bounds) = self.capture_bounds(target) else {
            log::info!(area = "capture", %target, "a screen cast was refused: no such target");
            return Err(CastRefusal::NoTarget);
        };
        let Some(format) = Format::new(bounds.size(), bounds.width.saturating_mul(4), PixelFormat::Argb8888) else {
            log::info!(area = "capture", %target, "a screen cast was refused: the target has no size");
            return Err(CastRefusal::NoTarget);
        };
        let layout = self.capture_layout(target, bounds);
        let origin = Point::new(layout.x, layout.y);
        let owner = self.capture.portal_owner(client);
        let session = self.capture.table.open(owner, target, CaptureStream::Granted, format);
        match self.capture.pipewire.start(&self.capture.table, session, owner) {
            Ok(node) => {
                log::info!(area = "capture", %target, node, "a screen cast started");
                self.publish();
                Ok(CastStream {
                    session,
                    node,
                    origin,
                    extent: layout.size(),
                    size: bounds.size(),
                })
            }
            Err(error) => {
                self.capture.table.close(session);
                log::warn!(area = "capture", %error, %target, "a screen cast stream could not be started");
                Err(CastRefusal::Unavailable(error.to_string()))
            }
        }
    }

    /// Ends one cast session. Its PipeWire stream is dropped on the loop's next pump.
    pub(crate) fn stop_cast(&mut self, session: SessionId) {
        if self.capture.table.close(session).is_some() {
            log::info!(area = "capture", session = session.get(), "a screen cast ended");
            self.publish();
        }
    }

    /// Where a target sits in the layout, and how large it is there, in logical pixels: an output's rectangle, a
    /// window's frame as it is shown now, or a View at its own size, which has no place in the layout.
    fn capture_layout(&self, target: Target, bounds: Rect) -> Rect {
        match target {
            Target::Output(output) => self.output_rect(output).unwrap_or(Rect::from_size(bounds.size())),
            Target::Window(window) => self
                .fallback
                .windows()
                .find(|managed| managed.id == window)
                .map(|managed| self.geometry_of(window).unwrap_or(managed.geometry))
                .unwrap_or(Rect::from_size(bounds.size())),
            Target::View(_) => Rect::from_size(bounds.size()),
        }
    }

    /// Ends every cast session `client` started.
    pub(crate) fn stop_casts_of(&mut self, client: ClientId) {
        if let Some(owner) = self.capture.portal_owners.get(&client).copied()
            && self.capture.table.close_owner(owner) > 0
        {
            log::info!(
                area = "capture",
                "the screen casts of a connection that went away ended"
            );
            self.publish();
        }
    }

    /// Points screen casts at the PipeWire daemon listening on `socket`, when one is named.
    pub(crate) fn use_pipewire_socket(&mut self, socket: Option<std::path::PathBuf>) {
        if let Some(socket) = socket {
            self.capture.pipewire = PipewireExport::with_socket(socket);
        }
    }

    /// Runs once per loop iteration: advances the capture tick and cuts every session the
    /// lock has covered.
    pub(crate) fn poll_capture(&mut self) {
        self.capture.tick = self.capture.tick.saturating_add(1);
        self.capture.views = self.composer.views().into_iter().collect();
        self.publish_view_pictures();
        let views_gone = self.capture.views_closed();
        if (!self.capture_allowed() && self.capture.stop_all()) || views_gone {
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

    /// Publishes the picture each View drew this frame, and tells capture that a View
    /// whose picture is new has changed everywhere, so its next frame reports the whole
    /// of it. The picture is compared by identity: the composer hands back the same
    /// `Arc` for as long as a View is not redrawn.
    pub(crate) fn publish_view_pictures(&mut self) {
        let mut pictures: BTreeMap<NodeId, Arc<Image>> = self.composer.view_pictures().into_iter().collect();
        // A View drawn on the GPU has no picture on the CPU until a consumer reads it back.
        pictures.extend(
            self.capture
                .view_readbacks
                .iter()
                .map(|(view, (_, picture))| (*view, Arc::clone(picture))),
        );
        if !self.capture.table.is_empty() {
            for (view, picture) in &pictures {
                let unchanged = self
                    .capture
                    .view_pictures
                    .get(view)
                    .is_some_and(|previous| Arc::ptr_eq(previous, picture));
                if !unchanged {
                    self.capture
                        .table
                        .note_damage(Target::View(*view), Rect::from_size(picture.size()));
                }
            }
        }
        self.capture.view_pictures = pictures;
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
        let mut targets = Targets {
            views: self.capture.views.clone(),
            view_pictures: self.capture.view_pictures.clone(),
            ..Targets::default()
        };
        for output in self.output_ids() {
            let (Some(rect), Some(scale)) = (self.output_rect(output), self.output_scale(output)) else {
                continue;
            };
            targets.outputs.insert(output, scale_size(rect.size(), scale));
        }
        for managed in self.fallback.windows() {
            let window = managed.id;
            let shown = self.geometry_of(window).unwrap_or(managed.geometry);
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

    /// Opens the one-frame session a screencopy frame is and takes its claim, or `None` when the session refuses it.
    fn open_screencopy(
        &mut self,
        owner: ClientId,
        output: OutputId,
        format: Format,
        tick: Tick,
    ) -> Option<(SessionId, FrameClaim)> {
        let session = self
            .capture
            .table
            .open(owner, Target::Output(output), CaptureStream::Requested, format);
        match self.capture.table.request(session, BufferId::new(1), tick) {
            Ok(claim) => Some((session, claim)),
            Err(refusal) => {
                self.capture.table.close(session);
                log::debug!(
                    area = "capture",
                    reason = refusal.as_str(),
                    "the frame could not be claimed"
                );
                None
            }
        }
    }

    /// Closes a screencopy frame's session and answers what was copied and when it was presented, or `None` when the
    /// frame was refused.
    fn close_screencopy(
        &mut self,
        session: SessionId,
        outcome: CaptureOutcome,
        presented: Tick,
    ) -> Option<(Rect, Tick)> {
        self.capture.table.close(session);
        match outcome {
            CaptureOutcome::Copied(report) => Some((report.region, presented)),
            CaptureOutcome::Refused(refusal) => {
                log::debug!(area = "capture", reason = refusal.as_str(), "the frame was refused");
                None
            }
        }
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
        let (session, claim) = self.open_screencopy(owner, output, format, tick)?;
        let targets = self.capture_targets();
        let mut source = Pixels {
            targets,
            frames: &self.capture.pixels,
            buffer: buffer.clone(),
            offset,
            with_cursor,
        };
        let presented = source.presented(Target::Output(output)).unwrap_or(tick);
        let outcome = self.capture.table.capture(claim, tick, capture, &mut source);
        self.close_screencopy(session, outcome, presented)
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
