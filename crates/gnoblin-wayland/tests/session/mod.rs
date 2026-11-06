//! A session on a socket of its own, and the real client the protocol tests drive.
//!
//! Every test in this directory starts a session the same way — a headless backend, a
//! socket in a temporary directory, a worker thread for the client choreography — so
//! the session and the client live here once instead of once per test. The client binds
//! nothing by default: a test binds the globals it exercises, which is also how a test
//! discovers that a global is *not* advertised.
//!
//! The two threads are deliberate: a calloop loop is not `Send` and neither is the
//! backend it drives, so the session runs on the test's own thread and the client
//! choreography runs on a worker. That is also closer to the real arrangement — the
//! compositor is one process and the client is another.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_backend::headless::Headless;
use gnoblin_config::Protocols;
use gnoblin_types::Point;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use gnoblin_wm::{NewWindowFocus, WmOptions};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_keyboard,
    wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1, ext_data_control_manager_v1, ext_data_control_offer_v1, ext_data_control_source_v1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};
use wayland_protocols::wp::security_context::v1::client::{wp_security_context_manager_v1, wp_security_context_v1};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1, zwlr_data_control_source_v1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// How long a test waits for a state before failing.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// `BTN_LEFT` from `linux/input-event-codes.h`.
pub const BTN_LEFT: u32 = 0x110;

/// The bytes a test's clipboard source answers a paste with.
pub const CLIPBOARD_BYTES: &[u8] = b"gnoblin clipboard";

/// What a worker thread can reach of a running session.
#[derive(Clone)]
pub struct Handle {
    socket: PathBuf,
    privileged_socket: PathBuf,
    pub control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Handle {
    /// The socket an ordinary application connects to.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The socket the session's own processes connect to.
    pub fn privileged_socket(&self) -> &Path {
        &self.privileged_socket
    }

    /// The window state right now.
    pub fn windows(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

    /// Waits for the window state to satisfy `predicate`, or fails with the last one.
    pub fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.windows();
            if predicate(&observation) {
                return observation;
            }
            assert!(
                Instant::now() < deadline,
                "the session never reached the expected state; last state: {observation:#?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The logical size of the session's only output.
    pub fn screen(&self) -> (i32, i32) {
        (
            gnoblin_backend::headless::DEFAULT_MODE.width as i32,
            gnoblin_backend::headless::DEFAULT_MODE.height as i32,
        )
    }

    /// Injects a pointer press and release at a global position, as a device would.
    pub fn click(&self, at: Point) {
        let output = self.windows().outputs().first().copied().expect("an output");
        self.control
            .inject_input(InputEvent::PointerMotion { output, position: at });
        self.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed: true,
        });
        self.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed: false,
        });
    }
}

/// Stops the session when the worker finishes, including when it panics: without this a
/// failing assertion would leave the loop running and hang the test.
pub struct StopOnDrop(pub ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A session that has been created but is not running yet.
pub struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink the sockets.
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    server: Server,
    handle: Handle,
}

impl Session {
    /// Creates a session with every global the configuration defaults to.
    pub fn start() -> Self {
        Self::start_with(Protocols::default())
    }

    /// Creates a session where a new window takes focus, as `focus_new_windows = allow`
    /// configures it. The default session refuses that, which is the documented default.
    pub fn start_allowing_focus() -> Self {
        let wm = WmOptions {
            new_window_focus: NewWindowFocus::Allow,
            ..WmOptions::default()
        };
        Self::create(Box::new(Headless::new()), Protocols::default(), wm)
    }

    /// Creates a session whose outputs a test can hotplug.
    pub fn start_detached() -> (Self, gnoblin_backend::headless::HeadlessHandle) {
        let backend = Headless::new();
        let outputs = backend.handle();
        (
            Self::create(Box::new(backend), Protocols::default(), WmOptions::default()),
            outputs,
        )
    }

    /// Creates a session with a specific protocol configuration.
    pub fn start_with(protocols: Protocols) -> Self {
        Self::create(Box::new(Headless::new()), protocols, WmOptions::default())
    }

    /// Builds the session, its sockets and the handle a worker thread uses.
    fn create(
        backend: Box<dyn gnoblin_backend::Backend<gnoblin_wayland::State>>,
        protocols: Protocols,
        wm: WmOptions,
    ) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let privileged_socket = directory.path().join("wayland-0-privileged.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            privileged_socket_path: Some(privileged_socket.clone()),
            protocols,
            wm,
            ..Config::default()
        };
        let server = Server::new(backend, config).expect("the session starts");
        let handle = Handle {
            socket,
            privileged_socket,
            control: server.control(),
            observation: server.observation(),
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the session on this thread and `body` on a worker, then joins both.
    pub fn run(self, body: impl FnOnce(&Handle) + Send + 'static) {
        let handle = self.handle.clone();
        let worker = thread::spawn({
            let handle = handle.clone();
            move || {
                let _stop = StopOnDrop(handle.control.clone());
                body(&handle);
            }
        });
        let outcome: Result<(), LoopError> = self.server.run();
        if let Err(payload) = worker.join() {
            // A failed assertion in the worker is the test's failure.
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// One published foreign toplevel, as a taskbar saw it.
#[derive(Clone, Debug, Default)]
pub struct Toplevel {
    /// The title the compositor last sent.
    pub title: String,
    /// The application id the compositor last sent.
    pub app_id: String,
    /// The state array the compositor last sent.
    pub states: Vec<u8>,
    /// How many `done` events arrived.
    pub dones: u32,
    /// Whether the compositor said it is closed.
    pub closed: bool,
    /// The outputs it entered.
    pub outputs: Vec<u32>,
    /// The identifier, from `ext_foreign_toplevel_list_v1`.
    pub identifier: String,
}

/// What a test's client records from the events it receives.
#[derive(Default)]
pub struct App {
    /// Every global the session advertised to this connection.
    ///
    /// Filled from the live [`GlobalList`] after each round trip rather than from the
    /// `wl_registry` events: `registry_queue_init` consumes the initial burst before
    /// the client state exists, so only the library's own list has it.
    pub globals: BTreeMap<String, u32>,
    /// The keyboard object, created when the seat announced the capability.
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The pointer object, created when the seat announced the capability.
    pub pointer: Option<wl_pointer::WlPointer>,
    /// The last serial a `wl_pointer.button` carried.
    pub button_serial: Option<u32>,
    /// How many times the keyboard entered a surface of this client.
    pub keyboard_enters: u32,
    /// How many `xdg_surface.configure` events arrived.
    pub configures: u32,
    /// The last surface configure serial.
    pub last_serial: u32,
    /// The size the server last suggested for a toplevel; zero when it suggested none.
    pub width: i32,
    pub height: i32,
    /// Whether the server asked a toplevel to close.
    pub closed: bool,
    /// The size a layer surface was last configured with.
    pub layer_size: Option<(u32, u32)>,
    /// The last layer surface configure serial.
    pub layer_serial: Option<u32>,
    /// Whether the server told a layer surface it is closed.
    pub layer_closed: bool,
    /// The minted activation token, once `xdg_activation_token_v1.done` arrived.
    pub token: Option<String>,
    /// The mime types of the current clipboard offer, or `None` for no selection.
    pub selection: Option<Vec<String>>,
    /// The offer object the current selection names, so a test can paste from it.
    pub selection_offer: Option<wl_data_offer::WlDataOffer>,
    /// How many selection events arrived, so a test can wait for a change.
    pub selection_events: u32,
    /// The mime types of every offer this client was given, by object id.
    pub offers: BTreeMap<u32, Vec<String>>,
    /// The mime type a `wl_data_source.send` carried, with the file descriptor.
    pub sends: Vec<String>,
    /// Whether the server cancelled this client's source.
    pub source_cancelled: bool,
    /// The wlr foreign-toplevel handles, so a test can send a taskbar's requests.
    pub handles: BTreeMap<u32, zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1>,
    /// The ext foreign-toplevel handles.
    pub ext_handles: BTreeMap<u32, ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1>,
    /// The wlr foreign-toplevel handles, by object id.
    pub toplevels: BTreeMap<u32, Toplevel>,
    /// The ext foreign-toplevel identifiers, by object id.
    pub ext_toplevels: BTreeMap<u32, Toplevel>,
}

impl App {
    /// The interface names the session advertised, as of the last round trip.
    pub fn interfaces(&self) -> Vec<String> {
        self.globals.keys().cloned().collect()
    }

    /// Whether the session advertised a global.
    pub fn has(&self, interface: &str) -> bool {
        self.globals.contains_key(interface)
    }
}

/// A real Wayland client for one connection.
pub struct Client {
    /// The client owns its connection so the socket closes when the client is dropped,
    /// which is what the "owner went away" tests rely on.
    #[allow(dead_code)]
    pub connection: Connection,
    pub queue: EventQueue<App>,
    pub state: App,
    pub qh: QueueHandle<App>,
    pub globals: GlobalList,
    pub compositor: wl_compositor::WlCompositor,
    pub shm: wl_shm::WlShm,
    pub wm_base: xdg_wm_base::XdgWmBase,
    pub seat: wl_seat::WlSeat,
    /// The output the client bound, which is what a foreign-toplevel handle's
    /// `output_enter` names: without one the compositor has no object to send.
    pub output: wl_output::WlOutput,
}

impl Client {
    /// Connects to a socket and binds the globals every protocol needs.
    pub fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        // A socket the server never accepts would block the round trip for ever; a
        // read timeout turns that into a failed test instead of a hung binary.
        stream
            .set_read_timeout(Some(TIMEOUT))
            .expect("a read timeout on the client socket");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .expect("every session advertises wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("every session advertises wl_shm");
        let wm_base: xdg_wm_base::XdgWmBase = globals
            .bind(&qh, 1..=5, ())
            .expect("every session advertises xdg_wm_base");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("every session advertises wl_seat");
        let output: wl_output::WlOutput = globals
            .bind(&qh, 1..=4, ())
            .expect("every session advertises wl_output");
        let mut client = Self {
            connection,
            queue,
            state: App::default(),
            qh,
            globals,
            compositor,
            shm,
            wm_base,
            seat,
            output,
        };
        client.roundtrip();
        client
    }

    /// Processes every event the server has sent.
    pub fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
        self.refresh_globals();
    }

    /// Copies the live global list into the state a test reads.
    fn refresh_globals(&mut self) {
        let mut globals = BTreeMap::new();
        self.globals.contents().with_list(|list| {
            for global in list {
                globals.insert(global.interface.clone(), global.version);
            }
        });
        self.state.globals = globals;
    }

    /// Processes whatever has arrived, then waits for the predicate to hold.
    pub fn wait_until(&mut self, what: &str, predicate: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.roundtrip();
            if predicate(&self.state) {
                return;
            }
            assert!(Instant::now() < deadline, "the client never saw {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Whether the session advertised a global to this connection.
    pub fn has_global(&self, interface: &str) -> bool {
        self.state.has(interface)
    }

    /// Creates a surface with a shared-memory buffer and returns its size.
    pub fn buffer(&self, width: i32, height: i32) -> wl_buffer::WlBuffer {
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ())
    }

    /// Creates a toplevel and commits it once, which is what asks for a configure.
    pub fn create_window(&mut self, app_id: &str, title: &str) -> Window {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip();
        Window {
            surface,
            xdg_surface,
            toplevel,
        }
    }

    /// Attaches a shared-memory buffer and commits, which maps the surface.
    pub fn attach_buffer(&mut self, surface: &wl_surface::WlSurface, width: i32, height: i32) {
        let buffer = self.buffer(width, height);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip();
    }

    /// Creates a toplevel and maps it with a buffer.
    pub fn map_window(&mut self, app_id: &str, title: &str) -> Window {
        let window = self.create_window(app_id, title);
        self.attach_buffer(&window.surface, 200, 150);
        window
    }

    /// Destroys a toplevel the way a well-behaved client does.
    pub fn destroy(&mut self, window: Window) {
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        self.roundtrip();
    }

    /// The keyboard and pointer objects, created when the seat announces them.
    pub fn seat_devices(&mut self) -> (Option<wl_keyboard::WlKeyboard>, Option<wl_pointer::WlPointer>) {
        self.roundtrip();
        (self.state.keyboard.clone(), self.state.pointer.clone())
    }

    /// Creates a layer surface on an output with no output argument.
    pub fn layer_surface(
        &mut self,
        shell: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        namespace: &str,
        layer: zwlr_layer_shell_v1::Layer,
    ) -> LayerWindow {
        let surface = self.compositor.create_surface(&self.qh, ());
        let layer_surface = shell.get_layer_surface(&surface, None, layer, namespace.to_owned(), &self.qh, ());
        LayerWindow { surface, layer_surface }
    }
}

/// The objects of one toplevel.
pub struct Window {
    pub surface: wl_surface::WlSurface,
    pub xdg_surface: xdg_surface::XdgSurface,
    pub toplevel: xdg_toplevel::XdgToplevel,
}

/// The objects of one layer surface.
pub struct LayerWindow {
    pub surface: wl_surface::WlSurface,
    pub layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
}

impl LayerWindow {
    /// Acknowledges the last configure the server sent for this surface.
    pub fn ack(&mut self, client: &mut Client) {
        let serial = client.state.layer_serial.expect("a layer configure arrived");
        self.layer_surface.ack_configure(serial);
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The live list in `GlobalListContents` is the record; `Client::roundtrip` copies
        // it, so nothing is collected from the events themselves.
        let _ = (state, event);
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Enter { serial, .. } = event {
            state.keyboard_enters += 1;
            state.last_serial = serial;
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: <wl_pointer::WlPointer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button { serial, .. } = event {
            state.button_serial = Some(serial);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            state.configures += 1;
            state.last_serial = serial;
            xdg_surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                state.width = width;
                state.height = height;
            }
            xdg_toplevel::Event::Close => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: <zwlr_layer_surface_v1::ZwlrLayerSurfaceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                state.layer_serial = Some(serial);
                state.layer_size = Some((width, height));
            }
            zwlr_layer_surface_v1::Event::Closed => state.layer_closed = true,
            _ => {}
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: <xdg_activation_token_v1::XdgActivationTokenV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for App {
    wayland_client::event_created_child!(App, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);

    fn event(
        state: &mut Self,
        device: &wl_data_device::WlDataDevice,
        event: <wl_data_device::WlDataDevice as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                // The offer's mime types arrive on the offer object itself, so it is
                // recorded empty here and filled in by its own `offer` events.
                state.offers.insert(id.id().protocol_id(), Vec::new());
            }
            wl_data_device::Event::Selection { id } => {
                state.selection_events += 1;
                state.selection = id
                    .as_ref()
                    .and_then(|offer| state.offers.get(&offer.id().protocol_id()).cloned());
                state.selection_offer = id.clone();
                let _ = (device, qh);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for App {
    fn event(
        state: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        event: <wl_data_offer::WlDataOffer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            state
                .offers
                .entry(offer.id().protocol_id())
                .or_default()
                .push(mime_type);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: <wl_data_source::WlDataSource as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                state.sends.push(mime_type);
                // The source is the client: it answers the request by writing into the
                // pipe the destination was given. Nothing in the compositor copies it.
                let mut file = std::fs::File::from(fd);
                let _ = std::io::Write::write_all(&mut file, CLIPBOARD_BYTES);
            }
            wl_data_source::Event::Cancelled => state.source_cancelled = true,
            _ => {}
        }
    }
}

impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for App {
    wayland_client::event_created_child!(App, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE =>
            (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        event: <zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                state.toplevels.entry(toplevel.id().protocol_id()).or_default();
                state.handles.insert(toplevel.id().protocol_id(), toplevel);
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {}
            _ => {}
        }
    }
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for App {
    fn event(
        state: &mut Self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        event: <zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let entry = state.toplevels.entry(handle.id().protocol_id()).or_default();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => entry.title = title,
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => entry.app_id = app_id,
            zwlr_foreign_toplevel_handle_v1::Event::State { state: states } => entry.states = states,
            zwlr_foreign_toplevel_handle_v1::Event::Done => entry.dones += 1,
            zwlr_foreign_toplevel_handle_v1::Event::Closed => entry.closed = true,
            zwlr_foreign_toplevel_handle_v1::Event::OutputEnter { output } => {
                entry.outputs.push(output.id().protocol_id());
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputLeave { output } => {
                entry.outputs.retain(|known| *known != output.id().protocol_id());
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for App {
    wayland_client::event_created_child!(App, ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE =>
            (ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        event: <ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.ext_toplevels.entry(toplevel.id().protocol_id()).or_default();
            state.ext_handles.insert(toplevel.id().protocol_id(), toplevel);
        }
    }
}

impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for App {
    fn event(
        state: &mut Self,
        handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: <ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let entry = state.ext_toplevels.entry(handle.id().protocol_id()).or_default();
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                entry.identifier.clone_from(&identifier);
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => entry.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => entry.app_id = app_id,
            ext_foreign_toplevel_handle_v1::Event::Done => entry.dones += 1,
            ext_foreign_toplevel_handle_v1::Event::Closed => entry.closed = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(App: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(App: ignore xdg_popup::XdgPopup);
wayland_client::delegate_noop!(App: ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(App: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
wayland_client::delegate_noop!(App: ignore xdg_activation_v1::XdgActivationV1);
wayland_client::delegate_noop!(App: ignore ext_data_control_manager_v1::ExtDataControlManagerV1);
wayland_client::delegate_noop!(App: ignore ext_data_control_device_v1::ExtDataControlDeviceV1);
wayland_client::delegate_noop!(App: ignore ext_data_control_offer_v1::ExtDataControlOfferV1);
wayland_client::delegate_noop!(App: ignore ext_data_control_source_v1::ExtDataControlSourceV1);
wayland_client::delegate_noop!(App: ignore zwlr_data_control_manager_v1::ZwlrDataControlManagerV1);
wayland_client::delegate_noop!(App: ignore zwlr_data_control_device_v1::ZwlrDataControlDeviceV1);
wayland_client::delegate_noop!(App: ignore zwlr_data_control_offer_v1::ZwlrDataControlOfferV1);
wayland_client::delegate_noop!(App: ignore zwlr_data_control_source_v1::ZwlrDataControlSourceV1);
wayland_client::delegate_noop!(App: ignore wp_security_context_manager_v1::WpSecurityContextManagerV1);
wayland_client::delegate_noop!(App: ignore wp_security_context_v1::WpSecurityContextV1);
