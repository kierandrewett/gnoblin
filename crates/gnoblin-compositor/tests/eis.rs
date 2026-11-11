//! The EIS endpoint a session serves: it exists with private permissions, input from an
//! ungranted EI client never reaches a Wayland client, and input does once it is granted.
//!
//! Each test starts a headless session, connects a real `reis` client to the endpoint
//! beside the IPC socket and reads what a real Wayland client received.

mod support;

use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client as IpcClient, ClientError, wire};
use gnoblin_input_eis::Caps;
use reis::{PendingRequestResult, ei};
use support::{Client, Session};

/// How long a test waits for a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `KEY_A` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;

/// `KEY_B`.
const KEY_B: u32 = 48;

/// The Wayland key state for a press, from `wl_keyboard.key_state`.
const PRESSED: u32 = 1;

/// A minimal EI client: enough of the protocol to bind a keyboard and send keys.
struct EiClient {
    context: ei::Context,
    seat: Option<ei::Seat>,
    capabilities: u64,
    devices: Vec<Device>,
    serial: u32,
    /// Whether the server told this client it was disconnected.
    disconnected: bool,
}

/// One device the server created, with its interfaces and resume state.
struct Device {
    wire: ei::Device,
    interfaces: std::collections::HashMap<String, reis::Object>,
    done: bool,
    resumed: Option<u32>,
}

impl EiClient {
    /// Connects to `path` and completes the handshake as a sender.
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).expect("the EIS endpoint is listening");
        let context = ei::Context::new(stream).expect("an EI connection");
        let handshake =
            reis::handshake::ei_handshake_blocking(&context, "eis-test", ei::handshake::ContextType::Sender)
                .expect("the handshake completes");
        Self {
            context,
            seat: None,
            capabilities: 0,
            devices: Vec::new(),
            serial: handshake.serial,
            disconnected: false,
        }
    }

    /// Reads and answers everything the server has sent so far.
    fn pump(&mut self) {
        while let Ok(read) = self.context.read() {
            if read == 0 {
                break;
            }
        }
        while let Some(pending) = self.context.pending_event() {
            match pending {
                PendingRequestResult::Request(event) => self.dispatch(event),
                PendingRequestResult::ParseError(error) => panic!("the server sent an unparseable event: {error}"),
                PendingRequestResult::InvalidObject(object) => {
                    panic!("the server sent an event for an unknown object: {object}")
                }
            }
        }
        let _ = self.context.flush();
    }

    /// Pumps for `duration`, so anything the server will send has arrived.
    fn settle(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            self.pump();
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Pumps until `ready`, or fails.
    fn wait_for(&mut self, what: &str, ready: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.pump();
            if ready(self) {
                return;
            }
            assert!(Instant::now() < deadline, "the EI client never saw {what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn dispatch(&mut self, event: ei::Event) {
        match event {
            ei::Event::Connection(_, ei::connection::Event::Seat { seat }) => self.seat = Some(seat),
            ei::Event::Connection(_, ei::connection::Event::Disconnected { .. }) => self.disconnected = true,
            ei::Event::Connection(_, ei::connection::Event::Ping { ping }) => ping.done(u64::from(self.serial)),
            ei::Event::Seat(_, ei::seat::Event::Capability { mask, .. }) => self.capabilities |= mask,
            ei::Event::Seat(_, ei::seat::Event::Device { device }) => self.devices.push(Device {
                wire: device,
                interfaces: std::collections::HashMap::new(),
                done: false,
                resumed: None,
            }),
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
            _ => {}
        }
    }

    /// Binds the capabilities in `mask`, which the seat advertised.
    fn bind(&mut self, mask: u64) {
        let seat = self.seat.as_ref().expect("the seat arrived");
        seat.bind(mask);
        let _ = self.context.flush();
    }

    /// Binds the keyboard capability the seat advertised.
    fn bind_keyboard(&mut self) {
        self.bind(u64::from(Caps::KEYBOARD.bits()));
    }

    /// The device that has `interface`, which must be resumed, and that interface.
    fn device_with<T: reis::Interface>(&self) -> (ei::Device, u32, T) {
        let device = self
            .devices
            .iter()
            .find(|device| device.interfaces.contains_key(T::NAME))
            .unwrap_or_else(|| panic!("a device with {}", T::NAME));
        let serial = device.resumed.expect("the device resumed");
        let interface: T = device
            .interfaces
            .get(T::NAME)
            .expect("the device has the interface")
            .clone()
            .downcast()
            .expect("the interface type matches its name");
        (device.wire.clone(), serial, interface)
    }

    /// Presses or releases `key` on the keyboard device, which must be resumed.
    fn key(&mut self, key: u32, pressed: bool) {
        let (wire, serial, keyboard) = self.device_with::<ei::Keyboard>();
        wire.start_emulating(serial, 1);
        keyboard.key(
            key,
            if pressed {
                ei::keyboard::KeyState::Press
            } else {
                ei::keyboard::KeyState::Released
            },
        );
        wire.frame(serial, 0);
        let _ = self.context.flush();
    }

    /// Moves the absolute pointer to `(x, y)` in the layout's own coordinates.
    fn motion_absolute(&mut self, x: f32, y: f32) {
        let (wire, serial, pointer) = self.device_with::<ei::PointerAbsolute>();
        wire.start_emulating(serial, 1);
        pointer.motion_absolute(x, y);
        wire.frame(serial, 0);
        let _ = self.context.flush();
    }
}

/// A Wayland client with a mapped window, which the keyboard enters.
fn focused_client(session: &Session) -> Client {
    let mut client = Client::connect(session.wayland());
    client.map_window("eis-test", "eis-test");
    client.roundtrip();
    client
}

/// Waits until `client` has seen a key press, or fails with what it saw.
fn wait_for_key(client: &mut Client, code: u32) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        client.roundtrip();
        if client
            .keys()
            .iter()
            .any(|&(key, state)| key == code && state == PRESSED)
        {
            return;
        }
        assert!(Instant::now() < deadline, "the Wayland client never saw key {code}");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_socket_exists_with_mode_0600() {
    let session = Session::start();
    let socket = session.directory().join("eis.sock");
    assert!(socket.exists(), "the EIS socket is bound beside the IPC socket");
    let mode = std::fs::metadata(&socket)
        .expect("the socket's metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the EIS socket is private to its owner");
    drop(session);
}

#[test]
fn an_ungranted_key_does_not_reach_a_wayland_client_and_a_granted_one_does() {
    let session = Session::start();
    let mut app = focused_client(&session);
    let socket = session.directory().join("eis.sock");

    let mut ungranted = EiClient::connect(&socket);
    ungranted.wait_for("the seat", |client| client.seat.is_some());
    ungranted.settle(Duration::from_millis(200));
    // Binding a capability the seat never advertised is a protocol violation, so the
    // ungranted client only waits: it must see no capability and no device.
    assert_eq!(ungranted.capabilities, 0, "an ungranted seat advertises nothing");
    assert!(ungranted.devices.is_empty(), "an ungranted seat creates no device");
    app.roundtrip();
    assert!(
        app.keys().iter().all(|&(key, _)| key != KEY_A),
        "an ungranted EI client's key reached the seat: {:?}",
        app.keys()
    );

    assert!(session.eis().grant(Caps::KEYBOARD), "the endpoint is running");
    // The grant re-advertises the seat with the granted capabilities; the client binds again.
    ungranted.wait_for("the keyboard capability after the grant", |client| {
        client.capabilities & u64::from(Caps::KEYBOARD.bits()) != 0
    });
    ungranted.bind_keyboard();
    ungranted.wait_for("the keyboard device after the grant", |client| {
        client
            .devices
            .first()
            .is_some_and(|device| device.done && device.resumed.is_some())
    });
    ungranted.key(KEY_A, true);
    wait_for_key(&mut app, KEY_A);
    drop(session);
}

/// A crashed session leaves its `eis.sock` behind, with nothing listening on it. The next
/// session replaces that file rather than refusing to start, as the IPC socket does.
#[test]
fn a_stale_socket_left_by_a_crashed_session_is_replaced() {
    let directory = support::private_directory();
    let stale = directory.path().join("eis.sock");
    // Binding and dropping leaves the socket file on disk with no listener behind it.
    drop(UnixListener::bind(&stale).expect("the stale socket"));
    assert!(stale.exists(), "the stale socket file is on disk");

    let session = Session::start_with(Some(directory.path().join("ipc.sock")));
    assert!(stale.exists(), "the EIS socket is bound over the stale file");
    drop(session);
}

/// The RemoteDesktop portal's `types` bits, which `remote.grant` carries.
const DEVICE_KEYBOARD: u32 = 1;
const DEVICE_POINTER: u32 = 2;

/// The EI capability mask the portal's keyboard and pointer stand for.
fn keyboard_and_pointer_mask() -> u64 {
    u64::from((Caps::KEYBOARD | Caps::POINTER | Caps::POINTER_ABSOLUTE | Caps::BUTTON | Caps::SCROLL).bits())
}

/// An IPC connection at the served version: the portal's, in these tests.
#[track_caller]
fn portal(session: &Session) -> IpcClient<UnixTransport> {
    IpcClient::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        gnoblin_compositor::version(),
    )
    .expect("the session accepts this build")
}

/// The code of a compositor refusal, which a test matches on.
#[track_caller]
fn refusal_code<T: std::fmt::Debug>(outcome: Result<T, ClientError>) -> String {
    match outcome {
        Err(ClientError::Refused { code, .. }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A window's frame rectangle, as `window get` prints it.
fn frame(session: &Session, title: &str) -> (f32, f32, f32, f32) {
    let id = support::wait_for_listed_id(session, title).to_string();
    let number = |property: &str| -> f32 {
        let value = session.ctl(&["window", "get", &id, property]).ok();
        value
            .out
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("`{property}` is a number, got {:?}", value.out))
    };
    (number("x"), number("y"), number("width"), number("height"))
}

/// Connects the EI client of an approved session through the socket the compositor opened.
#[track_caller]
fn connect_session(portal: &mut IpcClient<UnixTransport>, session: &str) -> EiClient {
    let socket = portal
        .remote_connect(session)
        .expect("an approved session gets a socket");
    EiClient::connect(Path::new(&socket))
}

#[test]
fn an_approved_session_gets_its_own_socket_and_its_client_moves_the_pointer_and_types() {
    let session = Session::start();
    let mut app = focused_client(&session);
    app.pointers_on_every_seat();
    let (x, y, width, height) = frame(&session, "eis-test");

    let mut portal = portal(&session);
    portal
        .remote_grant("/session/1", DEVICE_KEYBOARD | DEVICE_POINTER)
        .expect("the portal reports the user's yes");
    let socket = portal
        .remote_connect("/session/1")
        .expect("an approved session gets a socket");
    let mode = std::fs::metadata(&socket)
        .expect("the socket exists")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the session's socket is private to its owner");

    let mut remote = EiClient::connect(Path::new(&socket));
    remote.wait_for("the approved capabilities", |client| {
        client.capabilities == keyboard_and_pointer_mask()
    });
    assert!(
        !Path::new(&socket).exists(),
        "the socket is removed once its one client connected"
    );
    remote.bind(keyboard_and_pointer_mask());
    remote.wait_for("the keyboard and pointer devices", |client| {
        client.devices.len() == 2
            && client
                .devices
                .iter()
                .all(|device| device.done && device.resumed.is_some())
    });

    // The pointer hits what the scene draws, not the frame the window manager configured: the
    // client committed a 200 by 150 buffer, so the target is inside that.
    remote.motion_absolute(x + 50.0, y + 50.0);
    let deadline = Instant::now() + TIMEOUT;
    while !app
        .pointer_events()
        .iter()
        .any(|(_, event)| matches!(event, support::PointerEvent::Enter { .. }))
    {
        app.roundtrip();
        remote.pump();
        assert!(
            Instant::now() < deadline,
            "the Wayland client never saw the pointer arrive: {:?}; frame {:?}; remote disconnected {}",
            app.pointer_events(),
            (x, y, width, height),
            remote.disconnected
        );
        thread::sleep(Duration::from_millis(5));
    }
    remote.key(KEY_A, true);
    wait_for_key(&mut app, KEY_A);
}

#[test]
fn a_session_nobody_approved_gets_no_socket() {
    let session = Session::start();
    let mut portal = portal(&session);
    assert_eq!(
        refusal_code(portal.remote_connect("/session/1")),
        wire::code::NO_CONSENT,
        "a connect with no recorded answer is refused"
    );
    let directory = std::fs::read_dir(session.directory()).expect("the session directory");
    let sockets: Vec<String> = directory
        .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("eis-"))
        .collect();
    assert!(sockets.is_empty(), "no socket was made for it: {sockets:?}");
}

#[test]
fn a_refused_session_stays_refused_and_gets_no_socket() {
    let session = Session::start();
    let mut portal = portal(&session);
    portal
        .remote_refuse("/session/1")
        .expect("the portal reports the user's no");
    assert_eq!(
        refusal_code(portal.remote_connect("/session/1")),
        wire::code::CONSENT_REFUSED
    );
    assert_eq!(
        refusal_code(portal.remote_grant("/session/1", DEVICE_KEYBOARD)),
        wire::code::SESSION_TAKEN,
        "a later yes does not reopen a refusal"
    );
    assert_eq!(
        refusal_code(portal.remote_connect("/session/1")),
        wire::code::CONSENT_REFUSED,
        "and it is still refused afterwards"
    );
}

#[test]
fn a_grant_that_approves_no_device_is_refused() {
    let session = Session::start();
    let mut portal = portal(&session);
    assert_eq!(
        refusal_code(portal.remote_grant("/session/1", 0)),
        wire::code::NO_DEVICES
    );
    assert_eq!(
        refusal_code(portal.remote_connect("/session/1")),
        wire::code::NO_CONSENT,
        "a grant that was refused recorded nothing"
    );
}

#[test]
fn one_sessions_approval_reaches_neither_another_session_nor_the_endpoints_clients() {
    let session = Session::start();
    let mut portal = portal(&session);
    portal
        .remote_grant("/session/1", DEVICE_KEYBOARD)
        .expect("the first session is approved");
    assert_eq!(
        refusal_code(portal.remote_connect("/session/2")),
        wire::code::NO_CONSENT,
        "a second session nobody answered for gets nothing"
    );

    // A client of eis.sock is still waiting on the endpoint's own decision, which nothing made.
    let mut plain = EiClient::connect(&session.directory().join("eis.sock"));
    plain.wait_for("the seat", |client| client.seat.is_some());
    plain.settle(Duration::from_millis(200));
    assert_eq!(plain.capabilities, 0, "a session's approval does not reach eis.sock");

    let mut remote = connect_session(&mut portal, "/session/1");
    remote.wait_for("the approved keyboard", |client| {
        client.capabilities == u64::from(Caps::KEYBOARD.bits())
    });
}

#[test]
fn ending_a_session_releases_what_it_held_and_stops_its_events() {
    let session = Session::start();
    let mut app = focused_client(&session);
    let mut portal = portal(&session);
    portal
        .remote_grant("/session/1", DEVICE_KEYBOARD)
        .expect("the session is approved");
    let mut remote = connect_session(&mut portal, "/session/1");
    remote.wait_for("the approved keyboard", |client| {
        client.capabilities == u64::from(Caps::KEYBOARD.bits())
    });
    remote.bind(u64::from(Caps::KEYBOARD.bits()));
    remote.wait_for("the keyboard device", |client| {
        client
            .devices
            .first()
            .is_some_and(|device| device.done && device.resumed.is_some())
    });
    remote.key(KEY_A, true);
    wait_for_key(&mut app, KEY_A);

    portal.remote_end("/session/1").expect("the portal ends the session");
    let deadline = Instant::now() + TIMEOUT;
    while !app.keys().iter().any(|&(key, state)| key == KEY_A && state == 0) {
        app.roundtrip();
        assert!(
            Instant::now() < deadline,
            "the held key was never released: {:?}",
            app.keys()
        );
        thread::sleep(Duration::from_millis(5));
    }
    remote.wait_for("the disconnect", |client| client.disconnected);

    // What the client still sends reaches nobody, and the session cannot be connected again.
    remote.key(KEY_B, true);
    for _ in 0..30 {
        app.roundtrip();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.keys().iter().all(|&(key, _)| key != KEY_B),
        "an ended session's events reached the seat: {:?}",
        app.keys()
    );
    assert_eq!(
        refusal_code(portal.remote_connect("/session/1")),
        wire::code::NO_CONSENT,
        "an ended session holds no approval"
    );
}

#[test]
fn a_session_ends_when_the_connection_that_held_it_goes_away() {
    let session = Session::start();
    let mut portal = portal(&session);
    portal
        .remote_grant("/session/1", DEVICE_KEYBOARD)
        .expect("the session is approved");
    let mut remote = connect_session(&mut portal, "/session/1");
    remote.wait_for("the approved keyboard", |client| {
        client.capabilities == u64::from(Caps::KEYBOARD.bits())
    });

    // The portal crashes: the user's answer went with it.
    drop(portal);
    remote.wait_for("the disconnect", |client| client.disconnected);
}

#[test]
fn another_connection_can_neither_connect_nor_end_a_session_it_did_not_decide() {
    let session = Session::start();
    let mut first = portal(&session);
    let mut second = portal(&session);
    first
        .remote_grant("/session/1", DEVICE_KEYBOARD)
        .expect("the session is approved");
    assert_eq!(
        refusal_code(second.remote_connect("/session/1")),
        wire::code::SESSION_UNKNOWN
    );
    assert_eq!(
        refusal_code(second.remote_end("/session/1")),
        wire::code::SESSION_UNKNOWN
    );
    assert_eq!(
        refusal_code(second.remote_grant("/session/1", DEVICE_KEYBOARD)),
        wire::code::SESSION_TAKEN,
        "nor can it decide the same name again"
    );
    first
        .remote_connect("/session/1")
        .expect("the holder is untouched by the other connection's attempts");
}
