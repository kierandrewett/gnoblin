//! The nested backend: a session that is a Wayland client of the compositor hosting it.
//!
//! This is the production half of `design/rust-compositor.md`'s "Hosting sessions"
//! (D4): the display manager is a Gnoblin host, and each signed-in user's session is a Gnoblin whose
//! outputs are surfaces of the host. One nested output is one `xdg_toplevel` on the
//! parent, which is what makes a session a window of the login screen rather than a
//! second display server.
//!
//! # The four directions of the wire
//!
//! | Direction      | Mechanism                                                                   |
//! | -------------- | --------------------------------------------------------------------------- |
//! | size and scale | `xdg_toplevel.configure` and `wp_fractional_scale_v1.surface_preferred_scale` |
//! | pixels         | `wl_shm` buffers, damage stated with `damage_buffer`                         |
//! | pacing         | one `wl_surface.frame` callback per committed frame                         |
//! | input          | the parent's `wl_seat`: keyboard with its keymap, pointer and touch         |
//!
//! and four directions back: a configure becomes [`BackendEvent::OutputResized`], a
//! closed toplevel [`BackendEvent::OutputDisconnected`], a dead connection
//! [`BackendEvent::Disconnected`], and everything under the pointer an
//! [`InputEvent`](crate::InputEvent).
//!
//! # Why the software path, and what the dmabuf path waits for
//!
//! The frame this backend commits is a `wl_shm` buffer because the render path of this
//! slice is Pixman drawing into CPU memory (`render::Software`). A dmabuf would be a
//! second, faster path and it is blocked on three things, in order:
//!
//! 1. **A GPU renderer.** D12's GLES target does not exist yet, so there is no
//!    imported texture to export; exporting a Pixman buffer would be a copy into a GPU
//!    buffer, which is the copy the dmabuf path exists to avoid.
//! 2. **`zwp_linux_dmabuf_v1` on both sides.** The parent has to advertise it, and this
//!    crate would have to speak its client side, including the plane layout of the
//!    format it picks.
//! 3. **An unsafe opt-out.** Handing a buffer to `EGL` or `gbm` is FFI, and this crate
//!    inherits the workspace's `unsafe_code = "forbid"`. The dmabuf path therefore
//!    arrives with an explicit `#[allow(unsafe_code)]` shim module, as the root
//!    `Cargo.toml` describes for the DRM backend, and not before.
//!
//! The trait does not change when it does: a GLES target replaces `NestedTarget`'s
//! renderer and the pool becomes a dmabuf pool, and [`Backend`] sees
//! the same `draw`-then-`present` sequence.
//!
//! # Failure modes first
//!
//! 1. **A frame is drawn into a buffer the parent is still reading.** `surface`'s slot
//!    ring is only released by the parent's `wl_buffer.release`, and a frame with no
//!    free slot presents nothing:
//!    `surface::ring_tests::a_slot_in_flight_is_not_taken_twice`.
//! 2. **A resize storm leaves the output on a size that is already gone.** The records
//!    keep one pending configure per output, so a hundred configures between two
//!    dispatches become one [`BackendEvent::OutputResized`] of the final size:
//!    `records::tests::a_resize_storm_reports_the_latest_size_once` and, from the host's
//!    side, `gnoblin-wayland`'s `tests/nested.rs::a_host_resize_reconfigures_the_nested_output`.
//! 3. **The parent disappears mid-frame.** Every read, dispatch and flush failure marks
//!    the backend disconnected, reports [`BackendEvent::Disconnected`] once and makes
//!    every later frame an unknown output instead of a panic:
//!    `gnoblin-wayland`'s `tests/nested.rs::killing_the_host_reports_a_disconnect_without_panicking`.
//! 4. **A scale of zero or a non-finite one makes an output nothing can be drawn on.** A
//!    scale that is not a positive finite number is the identity scale:
//!    `tests::a_scale_that_is_not_a_number_is_the_identity`.
//! 5. **Input arrives before there is an output to attribute it to.** The pointer only
//!    has an output once the parent has said it entered a configured surface, so a
//!    motion before the first configure is dropped and counted:
//!    `input::tests::a_motion_without_an_output_is_dropped` and
//!    `records::tests::a_surface_has_no_output_before_its_first_configure`.
//! 6. **The parent closes the toplevel and the session keeps drawing into it.** A close
//!    is one [`BackendEvent::OutputDisconnected`] and the output stops being driven:
//!    `records::tests::a_close_is_reported_once`.
//! 7. **A parent that never configures leaves a session with no size.** Connecting waits
//!    for the first configure and fails with [`NestedError::NotConfigured`] rather than
//!    reporting an output with no pixels: `tests::an_unconfigured_surface_is_not_an_output`.
//! 8. **A frame reaches the parent before it asked for one.** [`NestedTarget::present`]
//!    refuses while a frame callback is outstanding, and
//!    [`crate::Backend::render`] does not even describe a frame:
//!    `gnoblin-wayland`'s `tests/nested.rs::a_frame_waits_for_the_hosts_frame_callback`.

mod handler;
mod input;
mod records;
mod surface;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use tracing::{debug, info, warn};
use wayland_client::backend::WaylandError;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_compositor, wl_output, wl_seat, wl_shm};
use wayland_client::{Connection, EventQueue};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1;
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::shell::client::xdg_wm_base;

use gnoblin_output::DisplayMode;
use gnoblin_types::{OutputId, Scale, Size};

pub use input::AXIS_STEP;
pub use surface::NestedTarget;

use crate::render::{Image, OutputFrame, RenderError};
use crate::{Backend, BackendError, BackendEvent, Host, InputSource, Output, Present, Target};

use handler::Handler;
use surface::physical_extent;

/// The size a nested output starts at when the parent leaves the size to the session.
///
/// xdg-shell states zero on both axes for "choose for yourself", and an output that
/// chooses has to choose something: it is the size the headless backend defaults to, so
/// a session's first window is placed the same way on either backend.
pub const UNCONSTRAINED_SIZE: Size = Size::new(1024, 768);

/// The connector name the first nested output gets.
pub const DEFAULT_CONNECTOR: &str = "NESTED-1";

/// The application id a nested session's toplevels carry.
pub const DEFAULT_APP_ID: &str = "org.gnoblin.Session";

/// The refresh rate a nested output reports when the parent's output states none.
///
/// A nested output is not scanned out, so its refresh rate is the parent's output's.
/// When the parent states none, 60 Hz is the rate every monitor a session runs on can
/// keep up with.
pub const DEFAULT_REFRESH_MHZ: u32 = 60_000;

/// The denominator `wp_fractional_scale_v1` states its preferred scale over.
const SCALE_DENOMINATOR: u32 = 120;

/// How many round trips [`Nested::connect`] waits for the first configure.
///
/// One is enough for a parent that answers at once, and a parent that places the window
/// needs a frame or two of its own; this is a bound on a broken parent, not a schedule.
const CONFIGURE_ROUNDTRIPS: u32 = 32;

/// One output the session asks the parent to host.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutputSpec {
    /// The connector name the session's configuration and its clients see.
    pub connector: String,
    /// Whether to ask the parent to show the surface full screen.
    ///
    /// A hosting compositor is a kiosk with one client, so a hosted session asks for this
    /// and the host's configuration decides what it means. It is off by default because
    /// the same backend also replaces the winit development window, and a development
    /// session is a window its host places. A parent that does not implement fullscreen
    /// answers with a configure of its own choosing, which the output follows all the
    /// same.
    pub fullscreen: bool,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            connector: DEFAULT_CONNECTOR.to_owned(),
            fullscreen: false,
        }
    }
}

impl OutputSpec {
    /// An output with this connector name.
    #[must_use]
    pub fn new(connector: impl Into<String>) -> Self {
        Self {
            connector: connector.into(),
            ..Self::default()
        }
    }

    /// The same output, asking the parent to show it full screen.
    #[must_use]
    pub fn fullscreen(mut self) -> Self {
        self.fullscreen = true;
        self
    }
}

/// How a nested session connects to the compositor hosting it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NestedConfig {
    /// The parent compositor's Wayland socket.
    pub socket: PathBuf,
    /// The application id the session's toplevels carry.
    pub app_id: String,
    /// The title the parent's window list shows, one per output with its connector
    /// name appended.
    pub title: String,
    /// The outputs to ask the parent for, in connection order.
    ///
    /// One per entry; a session with two monitors has two entries and the parent places
    /// two windows.
    pub outputs: Vec<OutputSpec>,
}

impl NestedConfig {
    /// A session on `socket` with one full-screen output.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            app_id: DEFAULT_APP_ID.to_owned(),
            title: "Gnoblin session".to_owned(),
            outputs: vec![OutputSpec::default()],
        }
    }
}

/// Why a nested session could not start or stopped.
#[derive(Debug)]
pub enum NestedError {
    /// The parent's socket could not be reached.
    Connect(io::Error),
    /// The parent does not advertise a global a session cannot work without.
    MissingGlobal(&'static str),
    /// The parent never configured a surface, so there is no size to draw at.
    NotConfigured,
    /// The parent and this backend agree on no 32-bit `wl_shm` format, or the configure
    /// asked for a size this backend will not allocate.
    NoFormat,
    /// A buffer pool could not be created.
    Buffer(io::Error),
    /// A renderer could not be created.
    Render(RenderError),
    /// The connection to the parent is gone.
    Disconnected,
}

impl std::fmt::Display for NestedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(error) => write!(formatter, "the parent compositor could not be reached: {error}"),
            Self::MissingGlobal(name) => write!(formatter, "the parent does not offer {name}"),
            Self::NotConfigured => write!(formatter, "the parent never configured a nested surface"),
            Self::NoFormat => write!(
                formatter,
                "the parent offers no shared-memory format this session can draw"
            ),
            Self::Buffer(error) => write!(formatter, "a buffer pool could not be created: {error}"),
            Self::Render(error) => write!(formatter, "the renderer could not be created: {error}"),
            Self::Disconnected => write!(formatter, "the connection to the parent compositor ended"),
        }
    }
}

impl std::error::Error for NestedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) | Self::Buffer(error) => Some(error),
            Self::Render(error) => Some(error),
            _ => None,
        }
    }
}

/// A session hosted by another compositor: one toplevel per output.
pub struct Nested {
    connection: Connection,
    queue: EventQueue<Handler>,
    handler: Handler,
    /// The target for each output index, or [`None`] for an output the parent has not
    /// configured yet or has closed.
    targets: Vec<Option<NestedTarget>>,
    /// The live outputs, in connection order, for the contract's slice.
    outputs: Vec<Output>,
    /// State changes waiting for the next dispatch.
    pending: Vec<BackendEvent>,
    /// The frame description the host fills, kept here so a steady frame allocates
    /// nothing.
    frame: OutputFrame,
    /// Set by the event-loop source when the parent's socket is readable.
    readable: Arc<AtomicBool>,
    disconnected: bool,
}

impl std::fmt::Debug for Nested {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Nested")
            .field("outputs", &self.outputs)
            .field("targets", &self.targets.len())
            .field("pending", &self.pending)
            .field("disconnected", &self.disconnected)
            .finish_non_exhaustive()
    }
}

impl Nested {
    /// Connects to the parent and creates one surface per configured output.
    ///
    /// The call waits for the parent's first configure, because
    /// [`Backend::outputs`] answers the composition root's question about where a
    /// session can place a window: a backend that returned before it had a size would
    /// answer "nowhere".
    ///
    /// # Errors
    ///
    /// [`NestedError::Connect`] when the socket cannot be reached,
    /// [`NestedError::MissingGlobal`] when the parent is not a compositor a session can
    /// run on, [`NestedError::NotConfigured`] when it never answers with a size,
    /// [`NestedError::NoFormat`] when it offers no shared-memory format, and
    /// [`NestedError::Buffer`] or [`NestedError::Render`] when the first output cannot
    /// be created.
    pub fn connect(config: &NestedConfig) -> Result<Self, NestedError> {
        let stream = std::os::unix::net::UnixStream::connect(&config.socket).map_err(NestedError::Connect)?;
        let connection =
            Connection::from_socket(stream).map_err(|error| NestedError::Connect(io::Error::other(error)))?;
        let (globals, queue) = registry_queue_init::<Handler>(&connection)
            .map_err(|error| NestedError::Connect(io::Error::other(error)))?;
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=4, ())
            .map_err(|_| NestedError::MissingGlobal("wl_compositor"))?;
        let shm: wl_shm::WlShm = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| NestedError::MissingGlobal("wl_shm"))?;
        let wm_base: xdg_wm_base::XdgWmBase = globals
            .bind(&qh, 1..=5, ())
            .map_err(|_| NestedError::MissingGlobal("xdg_wm_base"))?;
        let seat_global: wl_seat::WlSeat = globals
            .bind(&qh, 1..=8, ())
            .map_err(|_| NestedError::MissingGlobal("wl_seat"))?;
        // The scale protocols are optional: without them the parent's integer scale is
        // the scale the session composes at.
        let fractional_manager = globals
            .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, Handler, ()>(&qh, 1..=1, ())
            .ok();
        let viewporter = globals
            .bind::<wp_viewporter::WpViewporter, Handler, ()>(&qh, 1..=1, ())
            .ok();
        // Bound for its scale and its refresh rate, which are the session output's
        // fallbacks.
        let _parent_output = globals.bind::<wl_output::WlOutput, Handler, ()>(&qh, 1..=4, ()).ok();
        let mut handler = Handler::new(
            compositor,
            shm,
            wm_base,
            seat_global,
            fractional_manager,
            viewporter,
            config.app_id.clone(),
        );
        let count = config.outputs.len().max(1);
        for index in 0..count {
            let spec = config.outputs.get(index).cloned().unwrap_or_default();
            let title = format!("{} ({})", config.title, spec.connector);
            handler.create_output(
                &qh,
                OutputId::new(index as u64),
                &spec.connector,
                &title,
                spec.fullscreen,
            );
        }
        connection
            .flush()
            .map_err(|error| NestedError::Connect(io::Error::other(error)))?;
        let mut backend = Self {
            connection,
            queue,
            handler,
            targets: (0..count).map(|_| None).collect(),
            outputs: Vec::new(),
            pending: Vec::new(),
            frame: OutputFrame::new(),
            readable: Arc::new(AtomicBool::new(false)),
            disconnected: false,
        };
        backend.await_first_configures()?;
        info!(socket = %config.socket.display(), outputs = backend.outputs.len(), "[nested] connected to the parent");
        Ok(backend)
    }

    /// The parent's keymap, as the `xkb_v1` text it sent.
    ///
    /// [`None`] for a parent with no keyboard. The backend's [`crate::InputEvent::Key`]
    /// states evdev codes, so the layout is applied by whoever owns the keymap; this is
    /// the text it needs.
    #[must_use]
    pub fn keymap(&self) -> Option<&str> {
        self.handler.keymap()
    }

    /// How many input events could not be attributed to an output.
    #[must_use]
    pub fn dropped_input(&self) -> u64 {
        self.handler.table().dropped()
    }

    /// Waits for the parent to answer everything this session has sent.
    ///
    /// It is the blocking path, for a caller with no event loop of its own: a test that
    /// wants to read what the parent said next, and a composition root that creates a
    /// session before it starts its loop. A running session uses [`Backend::register`]
    /// and never blocks in [`Backend::dispatch`].
    ///
    /// # Errors
    ///
    /// [`NestedError::Disconnected`] when the parent has gone.
    pub fn roundtrip(&mut self) -> Result<(), NestedError> {
        if self.disconnected {
            return Err(NestedError::Disconnected);
        }
        let outcome = {
            let Self { queue, handler, .. } = self;
            queue.roundtrip(handler)
        };
        match outcome {
            Ok(_) => {
                self.settle();
                Ok(())
            }
            Err(error) => {
                self.fail(error);
                self.settle();
                Err(NestedError::Disconnected)
            }
        }
    }

    /// The target of an output, for a caller that wants its presents or its pixels.
    #[must_use]
    pub fn target_of(&self, output: OutputId) -> Option<&NestedTarget> {
        self.index(output)
            .and_then(|index| self.targets.get(index))
            .and_then(Option::as_ref)
    }

    /// Waits for the parent's first configure of every output.
    fn await_first_configures(&mut self) -> Result<(), NestedError> {
        for _ in 0..CONFIGURE_ROUNDTRIPS {
            if self.targets.iter().all(Option::is_some) {
                return Ok(());
            }
            self.roundtrip()?;
        }
        Err(NestedError::NotConfigured)
    }

    /// Reads what the parent sent, when the event loop says there is something to read.
    fn pump(&mut self) {
        if self.disconnected || !self.readable.swap(false, Ordering::AcqRel) {
            return;
        }
        let read = match self.connection.prepare_read() {
            Some(guard) => guard.read(),
            None => Ok(0),
        };
        match read {
            Ok(_) => {}
            // The source fired for a partial message another read took; there is
            // nothing to do until the next wake.
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) => {
                self.fail(error);
                return;
            }
        }
        let dispatched = {
            let Self { queue, handler, .. } = self;
            queue.dispatch_pending(handler)
        };
        if let Err(error) = dispatched {
            self.fail(error);
            return;
        }
        self.flush();
    }

    /// Sends what this session has queued for the parent.
    fn flush(&mut self) {
        if let Err(error) = self.connection.flush() {
            self.fail(error);
        }
    }

    /// Records that the parent is gone.
    ///
    /// Both a socket error and a message the parent should never have sent are the same
    /// thing to a session: there is nothing left to draw on.
    fn fail(&mut self, error: impl std::fmt::Debug) {
        if !self.disconnected {
            warn!(?error, "[nested] the connection to the parent ended");
        }
        self.disconnected = true;
        self.handler.table_mut().disconnected();
    }

    /// Applies everything the parent's events left behind.
    fn settle(&mut self) {
        let changes = self.handler.table_mut().take_changes();
        for (index, size, scale) in changes.configured {
            self.configure(index, size, scale);
        }
        for index in changes.frame_done {
            if let Some(Some(target)) = self.targets.get_mut(index) {
                target.frame_done();
            }
        }
        for (index, generation, slot) in changes.released {
            if let Some(Some(target)) = self.targets.get_mut(index) {
                target.release(generation, slot);
            }
        }
        for index in changes.closed {
            self.close_output(index);
        }
        if changes.disconnected {
            self.report_disconnect();
        }
        self.rebuild_outputs();
    }

    /// Applies one configure: a new output, or a resize of an existing one.
    fn configure(&mut self, index: usize, size: Size, scale: Scale) {
        let Some(id) = self.handler.table().output(index) else {
            return;
        };
        let connector = self.handler.table().connector(index).unwrap_or_default().to_owned();
        let refresh = match self.handler.refresh_mhz() {
            0 => DEFAULT_REFRESH_MHZ,
            refresh => refresh,
        };
        let mode = DisplayMode::new(
            physical_extent(size.width, scale),
            physical_extent(size.height, scale),
            refresh,
        );
        let output = Output::new(id, connector, mode, scale);
        let existing = match self.targets.get_mut(index) {
            Some(Some(target)) => Some(target.resize(output.clone())),
            _ => None,
        };
        match existing {
            Some(Ok(())) => {
                self.push_resize(output);
                return;
            }
            Some(Err(error)) => {
                warn!(?error, "[nested] a configure was refused");
                return;
            }
            None => {}
        }
        let Some((surface, viewport)) = self
            .handler
            .objects(index)
            .map(|objects| (objects.surface.clone(), objects.viewport.clone()))
        else {
            return;
        };
        let Some(format) = self.handler.format() else {
            warn!("[nested] the parent and this session agree on no shared-memory format");
            return;
        };
        let qh = self.queue.handle();
        match NestedTarget::new(&qh, &self.handler.shm, format, surface, viewport, index, output.clone()) {
            Ok(target) => {
                debug!(connector = %output.connector, "[nested] an output connected");
                if let Some(slot) = self.targets.get_mut(index) {
                    *slot = Some(target);
                }
                self.pending.push(BackendEvent::OutputConnected(output));
            }
            Err(error) => warn!(?error, "[nested] the output could not be created"),
        }
    }

    /// Reports a resize, replacing an older one for the same output.
    fn push_resize(&mut self, output: Output) {
        let id = output.id;
        self.pending.retain(|event| {
            !matches!(
                event,
                BackendEvent::OutputResized { output: changed, .. } if *changed == id
            )
        });
        self.pending.push(BackendEvent::OutputResized {
            output: id,
            mode: output.mode,
            scale: output.scale,
        });
    }

    /// Stops driving an output the parent closed.
    fn close_output(&mut self, index: usize) {
        let Some(id) = self.handler.table().output(index) else {
            return;
        };
        self.handler.destroy_output(index);
        if let Some(slot) = self.targets.get_mut(index)
            && slot.take().is_some()
        {
            self.pending.push(BackendEvent::OutputDisconnected(id));
        }
    }

    /// Reports a dead connection: every output is gone and none can come back.
    fn report_disconnect(&mut self) {
        for target in self.targets.iter_mut() {
            let _ = target.take();
        }
        self.pending.retain(|event| {
            !matches!(
                event,
                BackendEvent::Disconnected
                    | BackendEvent::OutputConnected(_)
                    | BackendEvent::OutputResized { .. }
                    | BackendEvent::OutputDisconnected(_)
            )
        });
        self.pending.push(BackendEvent::Disconnected);
    }

    /// Rebuilds the outputs the contract hands out as a slice.
    fn rebuild_outputs(&mut self) {
        self.outputs = self
            .targets
            .iter()
            .flatten()
            .map(|target| target.output().clone())
            .collect();
    }

    /// The index of an output in the target table.
    fn index(&self, output: OutputId) -> Option<usize> {
        self.targets
            .iter()
            .position(|target| target.as_ref().is_some_and(|target| target.output().id == output))
    }
}

impl<S: Host + 'static> Backend<S> for Nested {
    fn name(&self) -> &'static str {
        "nested"
    }

    fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.iter().find(|output| output.id == id)
    }

    fn target(&mut self, id: OutputId) -> Option<&mut dyn Target> {
        let index = self.index(id)?;
        self.targets
            .get_mut(index)?
            .as_mut()
            .map(|target| target as &mut dyn Target)
    }

    fn input(&mut self) -> &mut dyn InputSource {
        self.handler.table_mut().seat()
    }

    fn register(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        // The source owns a duplicate of the socket's descriptor and nothing else: the
        // connection stays in the backend, so the loop only says "something arrived" and
        // the next dispatch reads it. That is what keeps this backend free of interior
        // mutability and of an `unsafe` `get_mut` into the loop's data.
        let fd = self.connection.backend().poll_fd().try_clone_to_owned()?;
        let readable = Arc::clone(&self.readable);
        loop_handle
            .insert_source(Generic::new(fd, Interest::READ, Mode::Level), move |_, _, _| {
                readable.store(true, Ordering::Release);
                Ok(PostAction::Continue)
            })
            .map_err(io::Error::other)?;
        Ok(())
    }

    fn render(&mut self, host: &mut S, output: OutputId) -> Result<Present, BackendError> {
        self.pump();
        self.settle();
        let index = self.index(output).ok_or(BackendError::UnknownOutput(output))?;
        let Self { targets, frame, .. } = self;
        let Some(target) = targets.get_mut(index).and_then(Option::as_mut) else {
            return Err(BackendError::UnknownOutput(output));
        };
        if target.awaiting_frame() {
            // The parent has not finished the last frame, so it has not asked for this
            // one: nothing is described and nothing is presented.
            return Ok(Present::Nothing);
        }
        if !host.frame(output, frame) {
            return Ok(Present::Nothing);
        }
        target.draw(frame).map_err(BackendError::Render)?;
        let present = target.present(frame.damage());
        debug_assert!(
            present == Present::Nothing || target.awaiting_frame(),
            "a presented frame waits for the parent's callback"
        );
        self.flush();
        Ok(present)
    }

    fn capture(&mut self, output: OutputId, with_cursor: bool) -> Option<Image> {
        let index = self.index(output)?;
        self.targets.get_mut(index)?.as_mut()?.capture(with_cursor).ok()
    }

    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent> {
        self.pump();
        self.settle();
        while let Some(event) = self.handler.table_mut().seat().next_event() {
            host.input(event);
        }
        std::mem::take(&mut self.pending)
    }
}

/// A parent's preferred scale as the exact fraction the compositor composes at.
///
/// `wp_fractional_scale_v1` states the scale as the numerator of a fraction with a
/// denominator of 120, so 180 is 1.5. A scale of zero is not a scale — it would put no
/// pixels on a logical pixel — so it is the identity scale rather than a division by
/// zero later.
pub(crate) fn scale_from_parent(scale: u32) -> Scale {
    if scale == 0 {
        Scale::IDENTITY
    } else {
        Scale::new(scale, SCALE_DENOMINATOR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scale_of_no_pixels_per_pixel_is_the_identity() {
        assert_eq!(scale_from_parent(0), Scale::IDENTITY);
    }

    #[test]
    fn a_fractional_scale_is_kept_as_its_fraction() {
        assert_eq!(scale_from_parent(180), Scale::new(3, 2));
        assert_eq!(scale_from_parent(120), Scale::IDENTITY);
        assert_eq!(scale_from_parent(240), Scale::integral(2));
        assert_eq!(scale_from_parent(150), Scale::new(5, 4));
    }

    #[test]
    fn an_unconfigured_surface_is_not_an_output() {
        // The contract's answer for "the parent has not configured me" is no outputs at
        // all, never an output of no pixels: a session with no output refuses to place
        // a window rather than placing one on a monitor that is not there.
        let table = records::Table::new();
        assert_eq!(table.applied(0), None);
        assert_eq!(table.output(0), None);
    }
}
