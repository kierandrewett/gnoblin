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
//! # One renderer path with the other backends, presented through shared memory
//!
//! Each output is drawn offscreen by the renderer the session names ([`NestedConfig::renderer`]: software, GLES or
//! Vulkan, the same [`AnyRenderer`](crate::render::renderer::AnyRenderer) the headless backend draws with). The
//! frame is read back with the cursor composed in and written into a `wl_shm` slot, so the host receives the same
//! pixels whichever renderer drew them, and every GPU-facing question of the contract (dmabuf import, Views on the
//! GPU, effects, the memory footprint, the texture limit) is answered by the renderer through the code the headless
//! backend shares ([`crate::gpu`]), not by a nested variant of it.
//!
//! What is not built is presenting through the host's `zwp_linux_dmabuf_v1`, which would skip the readback (one
//! `glReadPixels` or its Vulkan equivalent per presented frame). It needs the host's linux-dmabuf client side, a
//! format and modifier the host and the session's device agree on, and a render-into-host-buffer path in each
//! renderer. Until then a GPU renderer costs a readback per frame that a DRM session does not pay; the contract does
//! not change when the path arrives, because [`Backend`] only sees `draw` then `present`.
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
//! 9. **A supersample factor the parent's window cannot honour.** The parent decides the mode and the
//!    scale, so a nested output is configured with a factor and nothing else; [`Backend::configure_output`]
//!    refuses another mode or scale. The renderer composes at the factor and the frame is averaged down
//!    into the `wl_shm` buffer, which stays the size of the parent's window; a parent that rounded a
//!    fractional scale (no viewport) has a buffer that is not the mode, and the factor is refused for it
//!    by name (`surface::NestedTarget::check_supersample`). A configure from the parent keeps the factor.
//!    From the host's side: `gnoblin-wayland`'s `tests/nested.rs::a_nested_output_can_be_supersampled`.

/// The parent-facing protocol handler: state and event dispatch.
mod handler;
/// Translation of the parent's pointer, keyboard and touch events.
mod input;
/// Per-output records and the table the parent's events update.
mod records;
/// Each nested output's surface, pool and frame presentation.
mod surface;

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use wayland_client::backend::WaylandError;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_compositor, wl_output, wl_seat, wl_shm};
use wayland_client::{Connection, EventQueue, Proxy};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1;
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1;
use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_pointer_constraints_v1;
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gestures_v1;
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_manager_v1;
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::shell::client::xdg_wm_base;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1;

use gnoblin_output::DisplayMode;
use gnoblin_types::{OutputId, Scale, Size};

pub use input::AXIS_STEP;
pub use surface::NestedTarget;

use crate::gpu::DmabufHub;
use crate::render::renderer::RendererKind;
use crate::{Backend, BackendError, BackendEvent, Host, InputSource, Output, Present, Target};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{OutputFrame, SecureTreatment};
use gnoblin_render_frame::image::Image;

use handler::{Bound, Handler, HostOutput};
use surface::physical_extent;

use crate::log;

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
    /// The icon name the session's toplevels carry (`xdg_toplevel_icon_v1`), looked up in the host's icon theme.
    /// A host that does not offer the protocol shows its own default icon.
    pub icon: Option<String>,
    /// The title the parent's window list shows, one per output with its connector
    /// name appended.
    pub title: String,
    /// The outputs to ask the parent for, in connection order.
    ///
    /// One per entry; a session with two monitors has two entries and the parent places
    /// two windows.
    pub outputs: Vec<OutputSpec>,
    /// What a capture draws in place of a secure layer.
    pub secure_treatment: SecureTreatment,
    /// The renderer every output draws with. `auto` is software, as on the headless backend.
    pub renderer: RendererKind,
    /// The size an output starts at when the parent leaves the size to the session (xdg-shell's zero by zero
    /// configure). A parent that states a size always wins.
    pub initial_size: Size,
}

impl NestedConfig {
    /// A session on `socket` with one full-screen output.
    #[must_use]
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            app_id: DEFAULT_APP_ID.to_owned(),
            icon: Some(DEFAULT_APP_ID.to_owned()),
            title: "Gnoblin session".to_owned(),
            outputs: vec![OutputSpec::default()],
            secure_treatment: SecureTreatment::default(),
            renderer: RendererKind::default(),
            initial_size: UNCONSTRAINED_SIZE,
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
    /// The output cannot be composed at that supersample factor, and the message says why.
    Supersample(String),
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
            Self::Supersample(reason) => write!(formatter, "{reason}"),
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
    /// The Wayland connection to the parent compositor.
    connection: Connection,
    /// The event queue the parent's events are read into before dispatch.
    queue: EventQueue<Handler>,
    /// The state that events from the parent are dispatched into.
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
    /// What a capture draws in place of a secure layer, from the session's configuration.
    secure_treatment: SecureTreatment,
    /// The renderer every output of this session draws with.
    renderer: RendererKind,
    /// What clients were offered of a GPU buffer, kept so each import and copy is checked against it.
    dmabuf: DmabufHub,
    /// Why the renderer could not be created for an output, until [`Nested::connect`] reports it: a session whose
    /// renderer is refused waits for no configure that will ever succeed.
    render_refused: Option<RenderError>,
    /// What a [`NestedHandle`] queued, applied by the next dispatch.
    requests: Arc<Mutex<HandleRequests>>,
    /// The identity the next output a handle connects gets. A counter, so an output that was closed is never named again.
    next_output: u64,
    /// The title every toplevel's connector is appended to.
    title: String,
}

impl std::fmt::Debug for Nested {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Nested")
            .field("outputs", &self.outputs)
            .field("targets", &self.targets.len())
            .field("pending", &self.pending)
            .field("connected", &self.handler.table().is_connected())
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
        // The scale protocols are optional: without them the integer scale of the parent outputs a surface is on is
        // the scale the session composes at.
        let fractional_manager = globals
            .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, Handler, ()>(&qh, 1..=1, ())
            .ok();
        let viewporter = globals
            .bind::<wp_viewporter::WpViewporter, Handler, ()>(&qh, 1..=1, ())
            .ok();
        // The shortcut inhibitor and the gestures are optional too: a parent without them keeps its shortcuts, and
        // delivers no swipe or pinch.
        let shortcuts_manager = globals
            .bind::<zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1, Handler, ()>(
                &qh,
                1..=1,
                (),
            )
            .ok();
        let gestures_manager = globals
            .bind::<zwp_pointer_gestures_v1::ZwpPointerGesturesV1, Handler, ()>(&qh, 1..=1, ())
            .ok();
        // Relative motion and pointer constraints are optional: without them a lock in the session holds the
        // session's pointer and not the host's.
        let relative_manager = globals
            .bind::<zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1, Handler, ()>(&qh, 1..=1, ())
            .ok();
        let constraints_manager = globals
            .bind::<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1, Handler, ()>(&qh, 1..=1, ())
            .ok();
        let icon_manager = globals
            .bind::<xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1, Handler, ()>(&qh, 1..=1, ())
            .ok();
        // Every output of the parent is bound, for the scale and the refresh rate of the one a surface is on.
        let parent_outputs: Vec<(u32, u32)> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|global| global.interface == wl_output::WlOutput::interface().name)
                .map(|global| (global.name, global.version))
                .collect()
        });
        let outputs = parent_outputs
            .into_iter()
            .map(|(name, version)| {
                let proxy = globals
                    .registry()
                    .bind::<wl_output::WlOutput, _, _>(name, version.min(4), &qh, ());
                HostOutput::new(name, proxy)
            })
            .collect();
        let mut handler = Handler::new(
            Bound {
                compositor,
                shm,
                wm_base,
                seat: seat_global,
                fractional_manager,
                viewporter,
                shortcuts_manager,
                gestures_manager,
                icon_manager,
                relative_manager,
                constraints_manager,
                outputs,
            },
            config.app_id.clone(),
            config.icon.clone(),
            config.initial_size,
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
            secure_treatment: config.secure_treatment,
            renderer: config.renderer,
            dmabuf: DmabufHub::default(),
            render_refused: None,
            requests: Arc::new(Mutex::new(HandleRequests::default())),
            next_output: count as u64,
            title: config.title.clone(),
        };
        backend.await_first_configures()?;
        log::info!(area = "nested", socket = %config.socket.display(), outputs = backend.outputs.len(), "connected to the parent");
        Ok(backend)
    }

    /// A handle that can open and close output windows after a session has taken this backend.
    #[must_use]
    pub fn handle(&self) -> NestedHandle {
        NestedHandle {
            requests: Arc::clone(&self.requests),
        }
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

    /// Whether the parent granted the request to keep its own shortcuts from `output`'s window, which it does while
    /// the window holds the keyboard: the session's `Super` bindings then reach the session. [`None`] when the parent
    /// does not offer the protocol, or does not know the output, and the parent's shortcuts then win whenever it
    /// wants them.
    #[must_use]
    pub fn shortcuts_inhibited(&self, output: OutputId) -> Option<bool> {
        let index = self.index(output)?;
        self.handler.shortcuts_inhibited(index)
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
        if !self.handler.table().is_connected() {
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
            if let Some(error) = self.render_refused.take() {
                return Err(NestedError::Render(error));
            }
            if self.targets.iter().all(Option::is_some) {
                return Ok(());
            }
            self.roundtrip()?;
        }
        if let Some(error) = self.render_refused.take() {
            return Err(NestedError::Render(error));
        }
        Err(NestedError::NotConfigured)
    }

    /// Reads what the parent sent, when the event loop says there is something to read.
    fn pump(&mut self) {
        if !self.handler.table().is_connected() || !self.readable.swap(false, Ordering::AcqRel) {
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
        if self.handler.table().is_connected() {
            log::warn!(area = "nested", ?error, "the connection to the parent ended");
        }
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
        let mut closed = false;
        for index in changes.closed {
            closed |= self.close_output(index);
        }
        // The person closing the last window ends the session; a handle's disconnect and a dead connection do not.
        if closed && !changes.disconnected && !self.has_window() {
            self.pending.push(BackendEvent::SessionEnded);
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
        let refresh = match self.handler.refresh_mhz(index) {
            0 => DEFAULT_REFRESH_MHZ,
            refresh => refresh,
        };
        let mode = DisplayMode::new(
            physical_extent(size.width, scale),
            physical_extent(size.height, scale),
            refresh,
        );
        // The parent decides the mode and the scale and never says anything about supersampling, so a configure keeps
        // the factor the output has.
        let supersample = match self.targets.get(index) {
            Some(Some(target)) => target.output().supersample,
            _ => Scale::IDENTITY,
        };
        let output = Output::new(id, connector, mode, scale).with_supersample(supersample);
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
                log::warn!(area = "nested", ?error, "a configure was refused");
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
            log::warn!(
                area = "nested",
                "the parent and this session agree on no shared-memory format"
            );
            return;
        };
        let qh = self.queue.handle();
        match NestedTarget::new(
            &qh,
            &self.handler.shm,
            format,
            surface,
            viewport,
            index,
            output.clone(),
            self.renderer,
        ) {
            Ok(target) => {
                log::debug!(area = "nested", connector = %output.connector, "an output connected");
                if let Some(slot) = self.targets.get_mut(index) {
                    *slot = Some(target);
                }
                self.pending.push(BackendEvent::OutputConnected(output));
            }
            Err(error) => {
                log::warn!(area = "nested", ?error, "the output could not be created");
                if let NestedError::Render(error) = error {
                    self.render_refused = Some(error);
                }
            }
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
            supersample: output.supersample,
        });
    }

    /// Stops driving an output, whether the parent closed it or a handle disconnected it. Whether a live output was
    /// dropped.
    fn close_output(&mut self, index: usize) -> bool {
        let Some(id) = self.handler.table().output(index) else {
            return false;
        };
        self.handler.destroy_output(index);
        if let Some(slot) = self.targets.get_mut(index)
            && slot.take().is_some()
        {
            self.pending.push(BackendEvent::OutputDisconnected(id));
            return true;
        }
        false
    }

    /// Whether any output still has a window on the parent, configured or waiting for its first configure.
    fn has_window(&self) -> bool {
        (0..self.targets.len()).any(|index| self.handler.objects(index).is_some())
    }

    /// Applies what a [`NestedHandle`] queued: windows to close and windows to open. A window that opens is an output
    /// once the parent has configured it, as the first ones are.
    fn apply_requests(&mut self) {
        let requests = std::mem::take(&mut *self.requests.lock().unwrap_or_else(PoisonError::into_inner));
        if !self.handler.table().is_connected() {
            return;
        }
        for output in requests.disconnect {
            // Closing an output twice, or one that is gone, names nothing and reports nothing.
            if let Some(index) = self.index(output) {
                self.close_output(index);
            }
        }
        let qh = self.queue.handle();
        for spec in requests.connect {
            let id = OutputId::new(self.next_output);
            self.next_output = self.next_output.saturating_add(1);
            let title = format!("{} ({})", self.title, spec.connector);
            self.handler
                .create_output(&qh, id, &spec.connector, &title, spec.fullscreen);
            self.targets.push(None);
        }
        self.flush();
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

    /// Whether `output` can be driven at `mode`, `scale` and `supersample`, changing nothing.
    ///
    /// The parent's window is the mode and the scale, so nothing here can move them: only the factor is a choice.
    fn check_composition(
        &self,
        output: OutputId,
        mode: DisplayMode,
        scale: Scale,
        supersample: Scale,
    ) -> Result<(), BackendError> {
        let index = self.index(output).ok_or(BackendError::UnknownOutput(output))?;
        let target = self
            .targets
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(BackendError::UnknownOutput(output))?;
        let now = target.output();
        if now.mode != mode || now.scale != scale {
            return Err(BackendError::Unsupported(output));
        }
        target.check_supersample(supersample).map_err(|error| {
            log::warn!(area = "nested", %error, "a supersample factor cannot be used");
            BackendError::Supersample(output)
        })
    }

    /// The index of an output in the target table.
    fn index(&self, output: OutputId) -> Option<usize> {
        self.targets
            .iter()
            .position(|target| target.as_ref().is_some_and(|target| target.output().id == output))
    }
}

/// What a [`NestedHandle`] queued.
#[derive(Debug, Default)]
struct HandleRequests {
    /// Windows to open, one output each.
    connect: Vec<OutputSpec>,
    /// Outputs whose windows to close.
    disconnect: Vec<OutputId>,
}

/// A caller's control over a nested backend a session already owns.
///
/// It is the nested twin of [`HeadlessHandle`](crate::headless::HeadlessHandle): hotplugging an output is a request
/// the next dispatch applies, so the compositor learns of it as a [`BackendEvent`] and never as a call into the
/// backend. A nested output is a window of the parent, so connecting one opens a toplevel and disconnecting one
/// closes it; neither changes a mode, because the parent's window is the mode.
#[derive(Clone, Debug)]
pub struct NestedHandle {
    requests: Arc<Mutex<HandleRequests>>,
}

impl NestedHandle {
    /// Opens a window for a new output on the next dispatch. The output appears when the parent configures it.
    pub fn connect_output(&self, spec: OutputSpec) {
        self.requests().connect.push(spec);
    }

    /// Closes the window of `output` on the next dispatch. The session goes on, even when it was the last one: only
    /// the person closing the last window ends it ([`BackendEvent::SessionEnded`]).
    pub fn disconnect_output(&self, output: OutputId) {
        self.requests().disconnect.push(output);
    }

    fn requests(&self) -> std::sync::MutexGuard<'_, HandleRequests> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<S: Host + 'static> Backend<S> for Nested {
    fn name(&self) -> &'static str {
        "nested"
    }

    fn secure_treatment(&self) -> SecureTreatment {
        self.secure_treatment
    }

    fn effect_kinds(&self) -> &'static [&'static str] {
        self.renderer.effect_kinds()
    }

    fn gpu_views(&self) -> bool {
        self.renderer.draws_views()
    }

    fn footprint(&self, output: OutputId) -> gnoblin_render_frame::footprint::Footprint {
        self.target_of(output).map(NestedTarget::footprint).unwrap_or_default()
    }

    fn dmabuf(&mut self) -> Option<crate::DmabufSupport> {
        let Self { dmabuf, targets, .. } = self;
        dmabuf.query(targets.iter_mut().flatten().next())
    }

    fn import_dmabuf(
        &mut self,
        buffer: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Result<(), crate::DmabufImportError> {
        let Self { dmabuf, targets, .. } = self;
        dmabuf.import(targets.iter_mut().flatten().next(), buffer)
    }

    fn dmabuf_capture_support(&mut self, output: OutputId) -> Option<crate::DmabufSupport> {
        // An output this session does not drive has no frame to export.
        self.index(output)?;
        let Self { dmabuf, targets, .. } = self;
        dmabuf.query_capture(targets.iter_mut().flatten().next())
    }

    fn dmabuf_picture_support(&mut self) -> Option<crate::DmabufSupport> {
        let Self { dmabuf, targets, .. } = self;
        dmabuf.query_capture(targets.iter_mut().flatten().next())
    }

    fn copy_to_dmabuf(
        &mut self,
        source: crate::DmabufSource<'_>,
        buffer: &smithay::backend::allocator::dmabuf::Dmabuf,
        copy: crate::DmabufCopy,
    ) -> Result<(), crate::DmabufCopyError> {
        let index = match &source {
            crate::DmabufSource::Frame { output, .. } => self.index(*output),
            crate::DmabufSource::Picture(_) => None,
        };
        let Self {
            dmabuf,
            targets,
            secure_treatment,
            ..
        } = self;
        let target = match index {
            Some(index) => targets.get_mut(index).and_then(Option::as_mut),
            None if matches!(source, crate::DmabufSource::Picture(_)) => targets.iter_mut().flatten().next(),
            None => None,
        };
        dmabuf.copy(*secure_treatment, target, source, buffer, copy)
    }

    fn read_dmabuf(
        &mut self,
        buffer: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Result<Image, crate::DmabufCopyError> {
        let Self { dmabuf, targets, .. } = self;
        dmabuf.read(targets.iter_mut().flatten().next(), buffer)
    }

    fn read_view(
        &mut self,
        scene: &gnoblin_render_frame::view_scene::ViewScene,
        capture: bool,
        live: &[u64],
    ) -> Result<Image, RenderError> {
        crate::gpu::GpuTarget::read_view(
            self.targets
                .iter_mut()
                .flatten()
                .next()
                .ok_or(RenderError::Unsupported)?,
            scene,
            capture,
            live,
        )
    }

    fn release_views(&mut self) {
        for target in self.targets.iter_mut().flatten() {
            crate::gpu::GpuTarget::release_views(target);
        }
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
        self.flush();
        Ok(present)
    }

    fn capture(&mut self, output: OutputId, with_cursor: bool) -> Option<Image> {
        let index = self.index(output)?;
        let treatment = self.secure_treatment;
        self.targets
            .get_mut(index)?
            .as_mut()?
            .capture(with_cursor, treatment)
            .ok()
    }

    fn pointer_constraint(&mut self, constraint: crate::PointerConstraint) {
        let qh = self.queue.handle();
        self.handler.pointer_constraint(&qh, constraint);
        self.flush();
    }

    fn output_controls(&self) -> crate::OutputControls {
        // The parent's window decides the mode and the scale, so the only thing a nested output is configured with
        // is its supersample factor: `configure_output` refuses a different mode or scale.
        crate::OutputControls {
            configure: true,
            ..crate::OutputControls::default()
        }
    }

    fn configure_output(
        &mut self,
        output: OutputId,
        mode: DisplayMode,
        scale: Scale,
        supersample: Scale,
    ) -> Result<(), BackendError> {
        self.check_composition(output, mode, scale, supersample)?;
        let index = self.index(output).ok_or(BackendError::UnknownOutput(output))?;
        let target = self
            .targets
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or(BackendError::UnknownOutput(output))?;
        if let Err(error) = target.set_supersample(supersample) {
            log::warn!(area = "nested", %error, "a supersample factor was refused");
            return Err(match error {
                NestedError::Render(error) => BackendError::Render(error),
                _ => BackendError::Supersample(output),
            });
        }
        let changed = target.output().clone();
        self.push_resize(changed);
        self.rebuild_outputs();
        Ok(())
    }

    fn test_output(
        &mut self,
        output: OutputId,
        mode: DisplayMode,
        scale: Scale,
        supersample: Scale,
    ) -> Result<(), BackendError> {
        self.check_composition(output, mode, scale, supersample)
    }

    fn max_virtual_edge(&mut self, output: OutputId) -> u32 {
        // The texture limit of the renderer that composes the output, within what this backend allocates.
        let limit = self
            .index(output)
            .and_then(|index| self.targets.get_mut(index))
            .and_then(Option::as_mut)
            .map_or(gnoblin_output::supersample::MAX_VIRTUAL_EDGE, |target| {
                crate::gpu::GpuTarget::max_virtual_edge(target)
            });
        surface::MAX_EXTENT.min(limit)
    }

    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent> {
        self.apply_requests();
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
