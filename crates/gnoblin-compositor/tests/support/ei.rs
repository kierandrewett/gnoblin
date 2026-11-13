//! A minimal EI client and the Wayland helpers the EIS tests share: enough of libei to bind
//! the capabilities a seat advertises and send input, and a focused Wayland client to watch
//! the input arrive at.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use reis::{PendingRequestResult, ei};

use super::{Client, Session};
use gnoblin_input_eis::Caps;

/// How long a test waits for a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The Wayland key state for a press, from `wl_keyboard.key_state`.
const PRESSED: u32 = 1;

/// A minimal EI client: enough of the protocol to bind a keyboard and send keys.
pub struct EiClient {
    pub context: ei::Context,
    pub seat: Option<ei::Seat>,
    pub capabilities: u64,
    pub devices: Vec<Device>,
    pub serial: u32,
    /// Whether the server told this client it was disconnected.
    pub disconnected: bool,
    /// Whether `start_emulating` was sent. The protocol starts a sequence once; a client that keeps typing sends it
    /// once and then only frames.
    emulating: bool,
    /// The same for the pointer device, which carries the motion and the buttons together.
    moving: bool,
}

/// One device the server created, with its interfaces and resume state.
pub struct Device {
    pub wire: ei::Device,
    pub interfaces: std::collections::HashMap<String, reis::Object>,
    pub done: bool,
    pub resumed: Option<u32>,
}

impl EiClient {
    /// Connects to `path` and completes the handshake as a sender.
    pub fn connect(path: &Path) -> Self {
        Self::over(UnixStream::connect(path).expect("the EIS endpoint is listening"))
    }

    /// Completes the handshake as a sender on a stream the test already holds, such as the
    /// descriptor the portal handed out.
    pub fn over(stream: UnixStream) -> Self {
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
            emulating: false,
            moving: false,
        }
    }

    /// Reads and answers everything the server has sent so far.
    pub fn pump(&mut self) {
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
    pub fn settle(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            self.pump();
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Pumps until `ready`, or fails.
    pub fn wait_for(&mut self, what: &str, ready: impl Fn(&Self) -> bool) {
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
    pub fn bind(&mut self, mask: u64) {
        let seat = self.seat.as_ref().expect("the seat arrived");
        seat.bind(mask);
        let _ = self.context.flush();
    }

    /// Binds the keyboard capability the seat advertised.
    pub fn bind_keyboard(&mut self) {
        self.bind(u64::from(Caps::KEYBOARD.bits()));
    }

    /// The device that has `interface`, which must be resumed, and that interface.
    pub fn device_with<T: reis::Interface>(&self) -> (ei::Device, u32, T) {
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
    pub fn key(&mut self, key: u32, pressed: bool) {
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

    /// Presses or releases `key` as a client that keeps typing does: the emulating sequence is started once, and
    /// later keys are only frames. A client that starts it again for every key stops being heard after the second.
    pub fn typing(&mut self, key: u32, pressed: bool) {
        let (wire, serial, keyboard) = self.device_with::<ei::Keyboard>();
        if !self.emulating {
            wire.start_emulating(serial, 1);
            self.emulating = true;
        }
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
    pub fn motion_absolute(&mut self, x: f32, y: f32) {
        let (wire, serial, pointer) = self.device_with::<ei::PointerAbsolute>();
        if !self.moving {
            wire.start_emulating(serial, 1);
            self.moving = true;
        }
        pointer.motion_absolute(x, y);
        wire.frame(serial, 0);
        let _ = self.context.flush();
    }

    /// Presses or releases the pointer button `button` (an evdev code such as `BTN_LEFT`) on the pointer device.
    pub fn button(&mut self, button: u32, pressed: bool) {
        let (wire, serial, device) = self.device_with::<ei::Button>();
        if !self.moving {
            wire.start_emulating(serial, 1);
            self.moving = true;
        }
        device.button(
            button,
            if pressed {
                ei::button::ButtonState::Press
            } else {
                ei::button::ButtonState::Released
            },
        );
        wire.frame(serial, 0);
        let _ = self.context.flush();
    }
}

/// A keyboard the session grants input to, started the way the EIS tests do, so a key is the real input path's.
pub fn keyboard(session: &Session) -> EiClient {
    let mut remote = EiClient::connect(&session.directory().join("eis.sock"));
    remote.wait_for("the seat", |client| client.seat.is_some());
    assert!(session.eis().grant(Caps::KEYBOARD), "the endpoint is running");
    remote.wait_for("the keyboard capability", |client| {
        client.capabilities & u64::from(Caps::KEYBOARD.bits()) != 0
    });
    remote.bind_keyboard();
    remote.wait_for("the keyboard device", |client| {
        client
            .devices
            .first()
            .is_some_and(|device| device.done && device.resumed.is_some())
    });
    remote
}

/// A keyboard, an absolute pointer and its buttons the session grants input to, so a click and a key are the real
/// input path's. `motion_absolute` takes the global logical coordinates the shell's nodes are placed in.
pub fn pointer_and_keyboard(session: &Session) -> EiClient {
    let mut remote = EiClient::connect(&session.directory().join("eis.sock"));
    remote.wait_for("the seat", |client| client.seat.is_some());
    let caps = Caps::KEYBOARD | Caps::POINTER_ABSOLUTE | Caps::BUTTON;
    assert!(session.eis().grant(caps), "the endpoint is running");
    remote.wait_for("the keyboard, pointer and button capabilities", |client| {
        client.capabilities & u64::from(caps.bits()) == u64::from(caps.bits())
    });
    remote.bind(u64::from(caps.bits()));
    remote.wait_for("the devices", |client| {
        let ready = |interface: &str| {
            client
                .devices
                .iter()
                .any(|device| device.done && device.resumed.is_some() && device.interfaces.contains_key(interface))
        };
        ready("ei_keyboard") && ready("ei_pointer_absolute") && ready("ei_button")
    });
    remote
}

/// A Wayland client with a mapped window, which the keyboard enters.
pub fn focused_client(session: &Session) -> Client {
    let mut client = Client::connect(session.wayland());
    client.map_window("eis-test", "eis-test");
    client.roundtrip();
    client
}

/// Waits until `client` has seen a key press, or fails with what it saw.
pub fn wait_for_key(client: &mut Client, code: u32) {
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

/// A window's frame rectangle, as `window get` prints it.
pub fn frame(session: &Session, title: &str) -> (f32, f32, f32, f32) {
    let id = super::wait_for_listed_id(session, title).to_string();
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
