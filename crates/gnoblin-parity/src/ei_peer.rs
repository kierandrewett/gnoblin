//! The EI client the input probes drive: libei's sender side, as an EIS server sees an input emulator.
//!
//! A compositor serves input emulation to a client over EIS, through whatever door it has: stock Mutter hands out a
//! socket from `org.gnome.Mutter.RemoteDesktop.Session.ConnectToEIS` ([`crate::remote_desktop`]), the Rust candidate
//! serves `eis.sock` beside its IPC socket. This module is the client at the other end of either socket, so the same
//! keystrokes and pointer motion reach each compositor through the same protocol and the input probes compare what a
//! Wayland client then receives.
//!
//! The client binds every capability the seat advertises and waits for the devices that carry them to resume, because
//! a compositor decides how to split the capabilities into devices and a client that asks for one shape would only
//! work on the compositor that shares it.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use reis::{PendingRequestResult, ei};
use rustix::time::{ClockId, clock_gettime};

use crate::target::TargetError;

/// How long the peer waits for the seat and its devices.
const SETTLE: Duration = Duration::from_secs(10);
/// How often a waiting peer looks again.
const POLL: Duration = Duration::from_millis(2);

/// How a compositor hands out an EIS socket. A door is opened once per session, by the first step that needs input.
pub(crate) trait EisDoor {
    /// Asks the compositor for a socket to its EIS server and returns it.
    fn open(&mut self) -> Result<UnixStream, TargetError>;
}

/// A door that is a socket in the file system: the candidate's `eis.sock` of a session whose endpoint was granted
/// when it started, as `gnoblin-parity serve-candidate` does.
pub(crate) struct SocketDoor {
    target: &'static str,
    path: std::path::PathBuf,
}

impl SocketDoor {
    /// A door to the socket at `path`.
    pub(crate) fn new(target: &'static str, path: std::path::PathBuf) -> Self {
        Self { target, path }
    }
}

impl EisDoor for SocketDoor {
    fn open(&mut self) -> Result<UnixStream, TargetError> {
        let deadline = Instant::now() + SETTLE;
        loop {
            match UnixStream::connect(&self.path) {
                Ok(stream) => return Ok(stream),
                Err(error) if Instant::now() >= deadline => {
                    return Err(TargetError::Live {
                        target: self.target.to_owned(),
                        detail: format!("cannot connect to {}: {error}", self.path.display()),
                    });
                }
                Err(_) => std::thread::sleep(POLL),
            }
        }
    }
}

/// One device the server created, with the interfaces it carries and its resume state.
struct Device {
    wire: ei::Device,
    interfaces: HashMap<String, reis::Object>,
    done: bool,
    resumed: Option<u32>,
}

/// A client of one EIS endpoint.
pub(crate) struct EiPeer {
    target: &'static str,
    context: ei::Context,
    seat: Option<ei::Seat>,
    /// The union of the capability bits the seat advertised.
    capabilities: u64,
    seat_done: bool,
    bound: bool,
    devices: Vec<Device>,
    serial: u32,
    /// The devices a sequence was started on: the protocol starts one once, and later events are only frames.
    emulating: Vec<ei::Device>,
}

/// The capability names of a seat mask, in the protocol's bit order.
fn capability_names(mask: u64) -> Vec<&'static str> {
    const NAMES: [(u64, &str); 6] = [
        (1 << 0, "pointer"),
        (1 << 1, "pointer-absolute"),
        (1 << 2, "keyboard"),
        (1 << 3, "touch"),
        (1 << 4, "scroll"),
        (1 << 5, "button"),
    ];
    NAMES
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| *name)
        .collect()
}

/// The monotonic clock in microseconds, the time base `ei_device.frame` carries.
fn now_micros() -> u64 {
    let now = clock_gettime(ClockId::Monotonic);
    u64::try_from(now.tv_sec)
        .unwrap_or_default()
        .saturating_mul(1_000_000)
        .saturating_add(u64::try_from(now.tv_nsec).unwrap_or_default() / 1_000)
}

impl EiPeer {
    /// Completes the handshake as a sender on `stream`, which the compositor's EIS door handed out.
    pub(crate) fn connect(target: &'static str, stream: UnixStream) -> Result<Self, TargetError> {
        let live = |detail: String| TargetError::Live {
            target: target.to_owned(),
            detail,
        };
        let context =
            ei::Context::new(stream).map_err(|error| live(format!("cannot start an EI connection: {error}")))?;
        let handshake =
            reis::handshake::ei_handshake_blocking(&context, "gnoblin-parity", ei::handshake::ContextType::Sender)
                .map_err(|error| live(format!("the EI handshake failed: {error}")))?;
        Ok(Self {
            target,
            context,
            seat: None,
            capabilities: 0,
            seat_done: false,
            bound: false,
            devices: Vec::new(),
            serial: handshake.serial,
            emulating: Vec::new(),
        })
    }

    fn live(&self, detail: impl Into<String>) -> TargetError {
        TargetError::Live {
            target: self.target.to_owned(),
            detail: detail.into(),
        }
    }

    /// Reads and answers everything the server has sent so far.
    fn pump(&mut self) -> Result<(), TargetError> {
        while let Ok(read) = self.context.read() {
            if read == 0 {
                break;
            }
        }
        while let Some(pending) = self.context.pending_event() {
            match pending {
                PendingRequestResult::Request(event) => self.dispatch(event),
                PendingRequestResult::ParseError(error) => {
                    return Err(self.live(format!("the EIS server sent an unparseable event: {error}")));
                }
                PendingRequestResult::InvalidObject(object) => {
                    return Err(self.live(format!("the EIS server sent an event for an unknown object: {object}")));
                }
            }
        }
        let _ = self.context.flush();
        Ok(())
    }

    fn dispatch(&mut self, event: ei::Event) {
        match event {
            ei::Event::Connection(_, ei::connection::Event::Seat { seat }) => {
                // A server that advertises again replaces the seat: what the old one carried no longer counts.
                self.seat = Some(seat);
                self.capabilities = 0;
                self.seat_done = false;
                self.bound = false;
                self.devices.clear();
                self.emulating.clear();
            }
            ei::Event::Connection(_, ei::connection::Event::Ping { ping }) => ping.done(u64::from(self.serial)),
            ei::Event::Seat(_, ei::seat::Event::Capability { mask, .. }) => self.capabilities |= mask,
            ei::Event::Seat(_, ei::seat::Event::Done) => self.seat_done = true,
            ei::Event::Seat(_, ei::seat::Event::Device { device }) => self.devices.push(Device {
                wire: device,
                interfaces: HashMap::new(),
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

    /// Pumps until `ready` holds or the wait runs out, naming `what` the peer was waiting for.
    fn wait_for(&mut self, what: &str, ready: impl Fn(&Self) -> bool) -> Result<(), TargetError> {
        let deadline = Instant::now() + SETTLE;
        loop {
            self.pump()?;
            if ready(self) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(self.live(format!("the EI peer never saw {what}")));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Waits for the seat, binds everything it advertises and waits until every device that carries a bound capability
    /// has resumed. Returns the capability names the seat advertised, for the report.
    pub(crate) fn ready(&mut self) -> Result<Vec<&'static str>, TargetError> {
        self.wait_for("a seat with capabilities", |peer| {
            peer.seat.is_some() && peer.seat_done && peer.capabilities != 0
        })?;
        if !self.bound {
            let mask = self.capabilities;
            if let Some(seat) = &self.seat {
                seat.bind(mask);
            }
            let _ = self.context.flush();
            self.bound = true;
        }
        self.wait_for("a resumed device", |peer| {
            !peer.devices.is_empty()
                && peer
                    .devices
                    .iter()
                    .all(|device| device.done && device.resumed.is_some())
        })?;
        Ok(capability_names(self.capabilities))
    }

    /// The device that carries interface `T`, the serial it resumed with and the interface.
    fn device_with<T: reis::Interface>(&self) -> Result<(ei::Device, u32, T), TargetError> {
        let device = self
            .devices
            .iter()
            .find(|device| device.interfaces.contains_key(T::NAME))
            .ok_or_else(|| self.live(format!("the seat has no device with {}", T::NAME)))?;
        let serial = device
            .resumed
            .ok_or_else(|| self.live(format!("the {} device never resumed", T::NAME)))?;
        let interface = device
            .interfaces
            .get(T::NAME)
            .and_then(|object| object.clone().downcast::<T>())
            .ok_or_else(|| self.live(format!("the {} interface is not of its own type", T::NAME)))?;
        Ok((device.wire.clone(), serial, interface))
    }

    /// Starts emulating on `device` the first time it is used, and ends the group of events with a frame.
    fn emit(&mut self, wire: &ei::Device, serial: u32, send: impl FnOnce()) -> Result<(), TargetError> {
        if !self.emulating.contains(wire) {
            wire.start_emulating(serial, 1);
            self.emulating.push(wire.clone());
        }
        send();
        wire.frame(serial, now_micros());
        self.context
            .flush()
            .map_err(|error| self.live(format!("cannot send to the EIS server: {error}")))
    }

    /// Presses or releases the evdev key `code`.
    pub(crate) fn key(&mut self, code: u32, pressed: bool) -> Result<(), TargetError> {
        let (wire, serial, keyboard) = self.device_with::<ei::Keyboard>()?;
        let state = if pressed {
            ei::keyboard::KeyState::Press
        } else {
            ei::keyboard::KeyState::Released
        };
        self.emit(&wire, serial, || keyboard.key(code, state))
    }

    /// Moves the pointer by `(dx, dy)` logical pixels.
    pub(crate) fn motion(&mut self, dx: f32, dy: f32) -> Result<(), TargetError> {
        let (wire, serial, pointer) = self.device_with::<ei::Pointer>()?;
        self.emit(&wire, serial, || pointer.motion_relative(dx, dy))
    }

    /// Presses or releases the evdev button `code`.
    pub(crate) fn button(&mut self, code: u32, pressed: bool) -> Result<(), TargetError> {
        let (wire, serial, button) = self.device_with::<ei::Button>()?;
        let state = if pressed {
            ei::button::ButtonState::Press
        } else {
            ei::button::ButtonState::Released
        };
        self.emit(&wire, serial, || button.button(code, state))
    }

    /// Scrolls by `(dx, dy)` logical pixels.
    pub(crate) fn scroll(&mut self, dx: f32, dy: f32) -> Result<(), TargetError> {
        let (wire, serial, scroll) = self.device_with::<ei::Scroll>()?;
        self.emit(&wire, serial, || scroll.scroll(dx, dy))
    }
}
