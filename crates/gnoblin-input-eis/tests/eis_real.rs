//! The EIS server against a real EI client, a real Wayland client and a real session
//! lock: one input path, proved end to end.
//!
//! Each test starts a headless compositor on a socket of its own, binds an EIS endpoint
//! to a socket in the same temporary directory, connects the `reis` client to it and
//! asserts on what a `wayland-client` connection received. Nothing here has a display, a
//! seat device or a compositor from the environment: the sockets, the buffers and the
//! input are all the test's own.
//!
//! The endpoint's sink is the seat's own input path ([`InputSink`]), which is the point
//! of the exercise: an EI client cannot take a different route into the session than a
//! device, so the lock gate, focus and grabs apply to it without a second code path.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{DEFAULT_MODE, Headless};
use gnoblin_input_eis::{Caps, Endpoint, Grant, Options, Quota, Region};
use gnoblin_types::{OutputId, Point, Rect};
use gnoblin_wayland::{Config, ControlHandle, InputSink, LoopError, Server, SessionSnapshot};
use reis::{PendingRequestResult, ei};
use tempfile::TempDir;
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a test waits for a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `KEY_A` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;

/// A compositor session plus an EIS endpoint on its own socket.
struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink both sockets.
    #[allow(dead_code)]
    directory: TempDir,
    server: Server,
    handle: Handle,
}

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    /// The Wayland socket the clients connect to.
    wayland: PathBuf,
    /// The EIS socket the EI client connects to.
    eis: PathBuf,
    control: ControlHandle,
    sink: InputSink,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Session {
    /// Starts a headless session with an output of the default size.
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let wayland = directory.path().join("wayland-0.sock");
        let eis = directory.path().join("eis.sock");
        let config = Config {
            socket_path: Some(wayland.clone()),
            ..Config::default()
        };
        let server = Server::new(Box::new(Headless::new()), config).expect("the session starts");
        let control = server.control();
        let handle = Handle {
            wayland,
            eis,
            sink: control.input_sink(),
            control,
            observation: server.observation(),
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the session on this thread and `body` on a worker, then joins both.
    fn run(self, body: impl FnOnce(&Handle) + Send + 'static) {
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

/// Stops the session when the worker finishes, including when it panics.
struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

impl Handle {
    /// The window state right now.
    fn snapshot(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

    /// Waits for the observation to satisfy `predicate`, or fails with the last one.
    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.snapshot();
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

    /// The only output, once the session has published one.
    fn output(&self) -> OutputId {
        let observation = self.wait_for(|observation| !observation.outputs().is_empty());
        observation.outputs()[0]
    }

    /// Binds an EIS endpoint that grants `capabilities` and delivers into the seat.
    fn endpoint(&self, capabilities: Caps) -> Endpoint {
        let output = self.output();
        let options = Options {
            path: self.eis.clone(),
            grant: Grant::granted(capabilities),
            regions: vec![Region {
                output,
                rect: Rect::new(0, 0, DEFAULT_MODE.width, DEFAULT_MODE.height),
            }],
            quota: Quota::default(),
        };
        let sink = self.sink.clone();
        Endpoint::bind(options, move |event| sink.inject(event)).expect("the EIS endpoint binds")
    }

    /// The centre of a window, in the layout's coordinates.
    fn centre(&self, title: &str) -> Point {
        let observation = self.wait_for(|observation| observation.titled(title).is_some_and(|window| window.mapped));
        let window = observation.titled(title).expect("the window is managed");
        Point::new(
            window.geometry.x + i32::try_from(window.geometry.width / 2).unwrap_or(0),
            window.geometry.y + i32::try_from(window.geometry.height / 2).unwrap_or(0),
        )
    }
}

/// What a real Wayland client records.
///
/// The keys are the pairs the client received, in order, so a test can assert both the
/// press and the release rather than a count.
#[derive(Default)]
struct App {
    /// How many `xdg_surface.configure` events arrived.
    configures: u32,
    /// How many of them the client acknowledged.
    acks: u32,
    /// How many times the keyboard entered this client's surface.
    keyboard_enters: u32,
    /// Keys as `(code, state)`, where state is the protocol's own number.
    keys: Vec<(u32, u32)>,
    /// How many times the pointer entered this client's surface.
    pointer_enters: u32,
    /// Pointer motions as `(x, y)`.
    motions: Vec<(f64, f64)>,
    /// How many lock objects received `locked`.
    locked: u32,
    /// How many lock-surface configures arrived.
    lock_configures: u32,
    /// The size the last lock configure carried.
    lock_width: i32,
    lock_height: i32,
}

/// A real Wayland client: an application window, or the session's locker.
struct Client {
    /// The client owns its connection so the socket closes when the client is dropped.
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    globals: GlobalList,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    #[allow(dead_code)]
    seat: wl_seat::WlSeat,
    outputs: Vec<wl_output::WlOutput>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    locks: Vec<ext_session_lock_v1::ExtSessionLockV1>,
}

/// A mapped toplevel.
struct Window {
    surface: wl_surface::WlSurface,
    #[allow(dead_code)]
    xdg_surface: xdg_surface::XdgSurface,
    #[allow(dead_code)]
    toplevel: xdg_toplevel::XdgToplevel,
}

impl Client {
    /// An ordinary application: it may map a window.
    fn app(socket: &Path) -> Self {
        let mut client = Self::connect(socket);
        client.wm_base = Some(
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises xdg_wm_base"),
        );
        client
    }

    /// A locker: it may ask for the session lock.
    fn locker(socket: &Path) -> Self {
        let mut client = Self::connect(socket);
        client.lock_manager = Some(
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises ext_session_lock_manager_v1"),
        );
        client
    }

    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_shm");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_seat");
        let outputs = globals
            .contents()
            .clone_list()
            .iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| {
                globals
                    .registry()
                    .bind::<wl_output::WlOutput, _, _>(global.name, global.version.min(4), &qh, ())
            })
            .collect();
        Self {
            connection,
            queue,
            state: App::default(),
            qh,
            globals,
            compositor,
            shm,
            seat,
            outputs,
            wm_base: None,
            lock_manager: None,
            locks: Vec::new(),
        }
    }

    /// Processes every event the server has sent.
    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// Creates a toplevel and commits it once, which asks for a configure.
    fn create_window(&mut self, app_id: &str, title: &str) -> Window {
        let wm_base = self.wm_base.as_ref().expect("an application client");
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &self.qh, ());
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
    fn attach_buffer(&mut self, surface: &wl_surface::WlSurface, width: i32, height: i32) {
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip();
    }

    /// Creates a toplevel and maps it with a buffer.
    fn map_window(&mut self, app_id: &str, title: &str) -> Window {
        let window = self.create_window(app_id, title);
        self.attach_buffer(&window.surface, 200, 150);
        window
    }

    /// Takes the session lock and presents a cover, which is what makes it `locked`.
    fn lock(&mut self) {
        let manager = self.lock_manager.as_ref().expect("a locker client");
        let lock = manager.lock(&self.qh, ());
        self.roundtrip();
        let output = self.outputs.first().expect("an output").clone();
        let surface = self.compositor.create_surface(&self.qh, ());
        let _lock_surface = lock.get_lock_surface(&surface, &output, &self.qh, ());
        self.roundtrip();
        assert!(self.state.lock_configures >= 1, "the lock surface was configured");
        let (width, height) = (self.state.lock_width, self.state.lock_height);
        self.attach_buffer(&surface, width, height);
        self.locks.push(lock);
    }

    /// Waits until the locker received `locked`.
    fn wait_for_locked(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        while self.state.locked == 0 {
            assert!(Instant::now() < deadline, "the locker never received `locked`");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// A real EI client: the sender half of the protocol, driven synchronously.
///
/// `reis` exposes the objects and requests; the pump below is the event loop a real
/// client would write, with a deadline instead of a compositor to serve.
struct EisClient {
    context: ei::Context,
    seat: Option<ei::Seat>,
    /// The capability mask the seat advertised, as the protocol states it.
    capabilities: u64,
    /// How many `ei_seat.done` events arrived.
    seat_done: u32,
    devices: Vec<EisDevice>,
    /// The last serial the server sent, which requests name.
    serial: u32,
    /// What the server said when it disconnected this client.
    disconnected: Option<(ei::connection::DisconnectReason, Option<String>)>,
}

/// One device a client bound, and what the server told it about the device.
struct EisDevice {
    wire: ei::Device,
    interfaces: HashMap<String, reis::Object>,
    done: bool,
    resumed: Option<u32>,
}

impl EisClient {
    /// Connects and completes the handshake as a sender.
    fn connect(path: &Path, name: &str) -> Self {
        let stream = UnixStream::connect(path).expect("the EIS endpoint is listening");
        let context = ei::Context::new(stream).expect("an EI connection");
        let device = reis::handshake::ei_handshake_blocking(&context, name, ei::handshake::ContextType::Sender)
            .expect("the handshake completes");
        Self {
            context,
            seat: None,
            capabilities: 0,
            seat_done: 0,
            devices: Vec::new(),
            serial: device.serial,
            disconnected: None,
        }
    }

    /// Reads and answers everything the server sent.
    fn pump(&mut self) {
        loop {
            match self.context.read() {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                // A closed socket is reported by `disconnected`, which the server sends
                // before it closes; nothing more can arrive either way.
                Err(_) => break,
            }
        }
        while let Some(pending) = self.context.pending_event() {
            match pending {
                PendingRequestResult::Request(event) => self.dispatch(event),
                PendingRequestResult::ParseError(error) => panic!("the server sent an unparseable event: {error}"),
                PendingRequestResult::InvalidObject(object) => {
                    panic!("the server sent an event for an object this client does not have: {object}")
                }
            }
        }
        let _ = self.context.flush();
    }

    /// Pumps until `ready`, or fails with what arrived instead.
    fn wait_for(&mut self, what: &str, ready: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.pump();
            if ready(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the EI client never saw {what}; capabilities {:#x}, {} devices, disconnected {:?}",
                self.capabilities,
                self.devices.len(),
                self.disconnected
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn dispatch(&mut self, event: ei::Event) {
        match event {
            ei::Event::Connection(_, request) => match request {
                ei::connection::Event::Seat { seat } => self.seat = Some(seat),
                ei::connection::Event::Ping { ping } => {
                    ping.done(u64::from(self.ping_cookie()));
                }
                ei::connection::Event::Disconnected {
                    last_serial,
                    reason,
                    explanation,
                } => {
                    self.serial = last_serial;
                    self.disconnected = Some((reason, explanation));
                }
                _ => {}
            },
            ei::Event::Seat(_, request) => match request {
                ei::seat::Event::Capability { mask, interface: _ } => self.capabilities |= mask,
                ei::seat::Event::Done => self.seat_done += 1,
                ei::seat::Event::Device { device } => self.devices.push(EisDevice {
                    wire: device,
                    interfaces: HashMap::new(),
                    done: false,
                    resumed: None,
                }),
                _ => {}
            },
            ei::Event::Device(device, request) => {
                let Some(entry) = self.devices.iter_mut().find(|entry| entry.wire == device) else {
                    return;
                };
                match request {
                    ei::device::Event::Interface { object } => {
                        entry.interfaces.insert(object.interface().to_owned(), object);
                    }
                    ei::device::Event::Done => entry.done = true,
                    ei::device::Event::Resumed { serial } => {
                        entry.resumed = Some(serial);
                        self.serial = serial;
                    }
                    _ => {}
                }
            }
            ei::Event::Callback(_, request) => {
                // The `ei_callback.done` a request may be answered with; nothing waits on
                // one here, so it is drained and dropped.
                let _ = request;
            }
            _ => {}
        }
    }

    /// The cookie this client sends with a ping answer. The protocol leaves its meaning
    /// to the client, and this one does not use it.
    const fn ping_cookie(&self) -> u32 {
        self.serial
    }

    /// Binds the capabilities the seat advertised.
    fn bind(&mut self) {
        let capabilities = self.capabilities;
        self.bind_mask(capabilities);
    }

    /// Binds a mask the caller chose, which is how a test asks for more than the seat
    /// advertised.
    fn bind_mask(&mut self, mask: u64) {
        let seat = self.seat.as_ref().expect("the seat arrived");
        seat.bind(mask);
        let _ = self.context.flush();
    }

    /// Waits until every device the client bound is ready to emulate.
    fn wait_for_devices(&mut self, count: usize) {
        self.wait_for("every device", |client| {
            client.devices.len() == count
                && client
                    .devices
                    .iter()
                    .all(|device| device.done && device.resumed.is_some())
        });
    }

    /// One device's interface, by the protocol's name.
    fn interface<T: reis::Interface>(&self, device: usize) -> T {
        self.devices[device]
            .interfaces
            .get(T::NAME)
            .unwrap_or_else(|| panic!("device {device} has no {}", T::NAME))
            .clone()
            .downcast()
            .expect("the interface type matches its name")
    }

    /// The device's interface, or `None` when the server did not create it.
    fn maybe_interface<T: reis::Interface>(&self, device: usize) -> Option<T> {
        self.devices[device]
            .interfaces
            .get(T::NAME)
            .cloned()
            .and_then(|object| object.downcast())
    }

    /// Starts an emulation and returns its sequence number.
    fn start_emulating(&mut self, device: usize, sequence: u32) {
        let serial = self.devices[device].resumed.expect("the device resumed");
        self.devices[device].wire.start_emulating(serial, sequence);
        let _ = self.context.flush();
    }

    /// Commits the events sent since the last frame.
    fn frame(&mut self, device: usize) {
        let serial = self.devices[device].resumed.expect("the device resumed");
        self.devices[device].wire.frame(serial, 0);
        let _ = self.context.flush();
    }

    /// Moves an absolute pointer, in the device's own coordinate space.
    fn motion_absolute(&mut self, device: usize, x: f32, y: f32) {
        let pointer: ei::PointerAbsolute = self.interface(device);
        pointer.motion_absolute(x, y);
        self.frame(device);
    }

    /// Presses or releases a key.
    fn key(&mut self, device: usize, key: u32, pressed: bool) {
        let keyboard: ei::Keyboard = self.interface(device);
        keyboard.key(
            key,
            if pressed {
                ei::keyboard::KeyState::Press
            } else {
                ei::keyboard::KeyState::Released
            },
        );
        self.frame(device);
    }
}

/// The device index of the pointer, the keyboard and the touch screen the client bound.
fn devices_of(capabilities: Caps) -> (Option<usize>, Option<usize>) {
    let pointer = capabilities.intersects(Caps::POINTER | Caps::POINTER_ABSOLUTE | Caps::BUTTON | Caps::SCROLL);
    let keyboard = capabilities.contains(Caps::KEYBOARD);
    (pointer.then_some(0), keyboard.then_some(usize::from(pointer)))
}

// The protocol objects the tests need: the client's own bookkeeping for each interface.
impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // The pointer and the keyboard are created when the seat says it has them, which
        // is the only way a client hears about either.
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
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
        match event {
            wl_keyboard::Event::Enter { .. } => state.keyboard_enters += 1,
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(value),
                ..
            } => state.keys.push((key, value as u32)),
            _ => {}
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
        match event {
            wl_pointer::Event::Enter {
                surface_x, surface_y, ..
            } => {
                state.pointer_enters += 1;
                state.motions.push((surface_x, surface_y));
            }
            wl_pointer::Event::Motion {
                surface_x, surface_y, ..
            } => state.motions.push((surface_x, surface_y)),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: <wl_output::WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: <wl_surface::WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: <wl_buffer::WlBuffer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: <wl_shm::WlShm as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: <wl_shm_pool::WlShmPool as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: <wl_compositor::WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
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
            xdg_surface.ack_configure(serial);
            state.configures += 1;
            state.acks += 1;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_session_lock_manager_v1::ExtSessionLockManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        _: <ext_session_lock_manager_v1::ExtSessionLockManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: <ext_session_lock_v1::ExtSessionLockV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            state.locked += 1;
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for App {
    fn event(
        state: &mut Self,
        lock_surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: <ext_session_lock_surface_v1::ExtSessionLockSurfaceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, width, height } = event {
            lock_surface.ack_configure(serial);
            state.lock_configures += 1;
            state.lock_width = i32::try_from(width).unwrap_or(0);
            state.lock_height = i32::try_from(height).unwrap_or(0);
        }
    }
}

/// The capabilities the endpoint grants where a test needs a pointer and a keyboard.
fn pointer_and_keyboard() -> Caps {
    Caps::POINTER | Caps::POINTER_ABSOLUTE | Caps::BUTTON | Caps::SCROLL | Caps::KEYBOARD
}

/// Polls a Wayland client until its state satisfies `ready`.
fn wait_for_app(app: &mut Client, what: &str, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        app.roundtrip();
        if ready(&app.state) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the Wayland client never saw {what}; keys {:?}, motions {:?}, enters {}",
            app.state.keys,
            app.state.motions,
            app.state.pointer_enters
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Connects an EI client, binds it and starts emulating every device it bound.
fn emulating(session: &Handle, capabilities: Caps, devices: usize, sequence: u32) -> EisClient {
    let mut eis = EisClient::connect(&session.eis, "gnoblin-test");
    eis.wait_for("the seat", |client| client.seat_done > 0);
    eis.bind();
    eis.wait_for_devices(devices);
    for index in 0..devices {
        eis.start_emulating(index, sequence);
    }
    assert_eq!(
        devices,
        eis.devices.len(),
        "the server created the devices that were bound"
    );
    let _ = capabilities;
    eis
}

#[test]
fn a_real_client_moves_the_pointer_to_absolute_coordinates() {
    Session::start().run(|session| {
        let mut app = Client::app(&session.wayland);
        let _window = app.map_window("org.gnoblin.Eis", "Pointer");
        let centre = session.centre("Pointer");
        let geometry = session
            .wait_for(|observation| observation.titled("Pointer").is_some_and(|window| window.mapped))
            .titled("Pointer")
            .expect("the window is managed")
            .geometry;

        let _endpoint = session.endpoint(pointer_and_keyboard());
        let mut eis = emulating(session, pointer_and_keyboard(), 2, 1);
        let (pointer, _) = devices_of(pointer_and_keyboard());
        eis.motion_absolute(pointer.expect("a pointer device"), centre.x as f32, centre.y as f32);

        wait_for_app(&mut app, "a pointer motion", |app| !app.motions.is_empty());
        let (x, y) = app.state.motions[0];
        // `wl_pointer` states the position in the surface's own coordinates, so the
        // global position the EI client sent arrives as the offset inside the window.
        let expected = (f64::from(centre.x - geometry.x), f64::from(centre.y - geometry.y));
        assert!(
            (x - expected.0).abs() <= 1.0 && (y - expected.1).abs() <= 1.0,
            "the motion landed at ({x}, {y}), expected {expected:?}"
        );
        assert!(
            x >= 0.0 && x < f64::from(geometry.width) && y >= 0.0 && y < f64::from(geometry.height),
            "the position ({x}, {y}) is inside the window ({geometry:?})"
        );
    });
}

#[test]
fn a_real_client_types_a_key_into_the_focused_window() {
    Session::start().run(|session| {
        let mut app = Client::app(&session.wayland);
        let _window = app.map_window("org.gnoblin.Eis", "Keys");
        session.wait_for(|observation| observation.titled("Keys").is_some_and(|window| window.focused));

        let _endpoint = session.endpoint(pointer_and_keyboard());
        let mut eis = emulating(session, pointer_and_keyboard(), 2, 1);
        let (_, keyboard) = devices_of(pointer_and_keyboard());
        let keyboard = keyboard.expect("a keyboard device");
        eis.key(keyboard, KEY_A, true);
        eis.key(keyboard, KEY_A, false);

        wait_for_app(&mut app, "two key edges", |app| app.keys.len() == 2);
        assert_eq!(app.state.keys, vec![(KEY_A, 1), (KEY_A, 0)], "press then release");
    });
}

#[test]
fn a_client_that_disconnects_releases_the_key_it_held() {
    Session::start().run(|session| {
        let mut app = Client::app(&session.wayland);
        let _window = app.map_window("org.gnoblin.Eis", "Held");
        session.wait_for(|observation| observation.titled("Held").is_some_and(|window| window.focused));

        let _endpoint = session.endpoint(pointer_and_keyboard());
        let mut eis = emulating(session, pointer_and_keyboard(), 2, 1);
        let (_, keyboard) = devices_of(pointer_and_keyboard());
        let keyboard = keyboard.expect("a keyboard device");
        eis.key(keyboard, KEY_A, true);
        wait_for_app(&mut app, "the press", |app| app.keys.len() == 1);
        assert_eq!(app.state.keys, vec![(KEY_A, 1)]);

        // The client goes away without releasing: its socket closes, and the release is
        // the server's job.
        drop(eis);
        wait_for_app(&mut app, "the release the server issued", |app| app.keys.len() == 2);
        assert_eq!(app.state.keys, vec![(KEY_A, 1), (KEY_A, 0)]);
    });
}

#[test]
fn injected_input_reaches_only_the_locker_while_locked() {
    Session::start().run(|session| {
        let mut app = Client::app(&session.wayland);
        let _window = app.map_window("org.gnoblin.Eis", "Covered");
        session.wait_for(|observation| observation.titled("Covered").is_some_and(|window| window.focused));

        let mut locker = Client::locker(&session.wayland);
        locker.lock();
        locker.wait_for_locked();
        session.wait_for(|observation| observation.lock_active());

        let _endpoint = session.endpoint(pointer_and_keyboard());
        let mut eis = emulating(session, pointer_and_keyboard(), 2, 1);
        let (pointer, keyboard) = devices_of(pointer_and_keyboard());
        eis.key(keyboard.expect("a keyboard device"), KEY_A, true);
        eis.key(keyboard.expect("a keyboard device"), KEY_A, false);
        eis.motion_absolute(pointer.expect("a pointer device"), 40.0, 40.0);

        // Give the session every chance to deliver what it is going to deliver.
        for _ in 0..40 {
            app.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.state.keys.is_empty(),
            "a covered session delivered injected keys to a normal client: {:?}",
            app.state.keys
        );
        assert!(
            app.state.motions.is_empty(),
            "a covered session delivered injected motion to a normal client: {:?}",
            app.state.motions
        );
    });
}

#[test]
fn a_capability_the_grant_excludes_gets_no_device() {
    Session::start().run(|session| {
        let _endpoint = session.endpoint(Caps::KEYBOARD);

        let mut eis = EisClient::connect(&session.eis, "keyboard-only");
        eis.wait_for("the seat", |client| client.seat_done > 0);
        assert_eq!(
            eis.capabilities,
            u64::from(Caps::KEYBOARD.bits()),
            "the seat advertises only the granted capability"
        );
        eis.bind();
        eis.wait_for_devices(1);
        assert!(
            eis.maybe_interface::<ei::Keyboard>(0).is_some(),
            "the granted capability is created"
        );
        assert!(
            eis.devices
                .iter()
                .all(|device| !device.interfaces.contains_key("ei_pointer")),
            "a capability the grant excludes gets no device"
        );

        // And asking for it anyway is refused on the wire, with nothing left behind.
        let mut escalating = EisClient::connect(&session.eis, "escalating");
        escalating.wait_for("the seat", |client| client.seat_done > 0);
        escalating.bind_mask(u64::from(Caps::KEYBOARD.bits() | Caps::POINTER.bits()));
        escalating.wait_for("a refusal", |client| client.disconnected.is_some());
        assert!(
            escalating.devices.is_empty(),
            "no device came from an unadvertised capability"
        );
    });
}

#[test]
fn malformed_events_are_refused_and_the_server_keeps_serving() {
    Session::start().run(|session| {
        let mut app = Client::app(&session.wayland);
        let _window = app.map_window("org.gnoblin.Eis", "Malformed");
        let centre = session.centre("Malformed");

        let _endpoint = session.endpoint(pointer_and_keyboard());
        let mut bad = emulating(session, pointer_and_keyboard(), 2, 1);
        let (_, keyboard) = devices_of(pointer_and_keyboard());
        // A key code Linux does not define: the session refuses it and says why.
        bad.key(keyboard.expect("a keyboard device"), 0x400, true);
        bad.wait_for("the refusal", |client| client.disconnected.is_some());
        let (reason, explanation) = bad.disconnected.clone().expect("a disconnect was sent");
        assert_eq!(reason, ei::connection::DisconnectReason::Protocol);
        assert!(
            explanation.as_deref().is_some_and(|text| text.contains("key code")),
            "the explanation names the rule: {explanation:?}"
        );
        drop(bad);

        // The endpoint is still serving: a fresh client's input reaches the window.
        let mut good = emulating(session, pointer_and_keyboard(), 2, 1);
        let (pointer, _) = devices_of(pointer_and_keyboard());
        good.motion_absolute(pointer.expect("a pointer device"), centre.x as f32, centre.y as f32);
        wait_for_app(&mut app, "the recovered client's motion", |app| !app.motions.is_empty());
    });
}
