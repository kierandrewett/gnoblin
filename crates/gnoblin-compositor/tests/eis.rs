//! The EIS endpoint a session serves: it exists with private permissions, input from an
//! ungranted EI client never reaches a Wayland client, and input does once it is granted.
//!
//! Each test starts a headless session, connects a real `reis` client to the endpoint
//! beside the IPC socket and reads what a real Wayland client received.

mod support;

use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_input_eis::Caps;
use reis::{Interface as _, PendingRequestResult, ei};
use support::{Client, Session};

/// How long a test waits for a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `KEY_A` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;

/// The Wayland key state for a press, from `wl_keyboard.key_state`.
const PRESSED: u32 = 1;

/// A minimal EI client: enough of the protocol to bind a keyboard and send keys.
struct EiClient {
    context: ei::Context,
    seat: Option<ei::Seat>,
    capabilities: u64,
    devices: Vec<Device>,
    serial: u32,
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

    /// Binds the keyboard capability the seat advertised.
    fn bind_keyboard(&mut self) {
        let seat = self.seat.as_ref().expect("the seat arrived");
        seat.bind(u64::from(Caps::KEYBOARD.bits()));
        let _ = self.context.flush();
    }

    /// Presses or releases `key` on the first keyboard device, which must be resumed.
    fn key(&mut self, key: u32, pressed: bool) {
        let device = self.devices.first().expect("a keyboard device");
        let serial = device.resumed.expect("the device resumed");
        let keyboard: ei::Keyboard = device
            .interfaces
            .get(ei::Keyboard::NAME)
            .expect("the device has a keyboard")
            .clone()
            .downcast()
            .expect("the keyboard interface");
        let wire = device.wire.clone();
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
