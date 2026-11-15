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
struct PeerDevice {
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
    /// The interface each advertised capability stands for. The server chooses the bit of each one and says which
    /// interface it is in the same event, so a mask means nothing without this list.
    advertised: Vec<String>,
    seat_done: bool,
    bound: bool,
    devices: Vec<PeerDevice>,
    serial: u32,
    /// The devices a sequence was started on: the protocol starts one once, and later events are only frames.
    emulating: Vec<ei::Device>,
}

/// The capability names the seat advertised, for the report: `ei_pointer_absolute` is `pointer-absolute`.
fn capability_names(advertised: &[String]) -> Vec<String> {
    advertised
        .iter()
        .map(|interface| interface.trim_start_matches("ei_").replace('_', "-"))
        .collect()
}

/// The interfaces the probes send through: a pointer sends motion, a keyboard keys, a button button edges and a scroll
/// axis events. A seat that does not advertise one of them is not waited on for it.
const SENT_THROUGH: [&str; 4] = ["ei_pointer", "ei_keyboard", "ei_scroll", "ei_button"];

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
            advertised: Vec::new(),
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
                self.advertised.clear();
                self.seat_done = false;
                self.bound = false;
                self.devices.clear();
                self.emulating.clear();
            }
            ei::Event::Connection(_, ei::connection::Event::Ping { ping }) => ping.done(u64::from(self.serial)),
            ei::Event::Seat(_, ei::seat::Event::Capability { mask, interface }) => {
                self.capabilities |= mask;
                self.advertised.push(interface);
            }
            ei::Event::Seat(_, ei::seat::Event::Done) => self.seat_done = true,
            ei::Event::Seat(_, ei::seat::Event::Device { device }) => self.devices.push(PeerDevice {
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

    /// Waits for the seat, binds everything it advertises and waits until every interface the probes send through is on
    /// a resumed device. Returns the capability names the seat advertised, for the report.
    ///
    /// A server may advertise a seat again (a Mutter session does once the remote desktop session starts), and a new
    /// seat forgets what was bound and the devices of the old one, so the binding is made again for each seat, and the
    /// wait is over only when the devices of the seat that is current have all resumed. A server may also create its
    /// devices one after the other, so the first resumed device is not the last.
    pub(crate) fn ready(&mut self) -> Result<Vec<String>, TargetError> {
        let deadline = Instant::now() + SETTLE;
        loop {
            self.pump()?;
            if self.seat.is_some() && self.seat_done && self.capabilities != 0 && !self.bound {
                let mask = self.capabilities;
                if let Some(seat) = &self.seat {
                    seat.bind(mask);
                }
                let _ = self.context.flush();
                self.bound = true;
            }
            if self.bound && self.devices_cover_the_seat() {
                return Ok(capability_names(&self.advertised));
            }
            if Instant::now() >= deadline {
                let wait = if self.bound {
                    "a resumed device for every capability the seat advertised"
                } else {
                    "a seat with capabilities"
                };
                return Err(self.live(format!("the EI peer never saw {wait}")));
            }
            std::thread::sleep(POLL);
        }
    }

    /// The state of the peer in words, for an error: the mask the seat advertised, whether it is bound, and the
    /// interfaces on each device with whether it resumed.
    pub(crate) fn describe(&self) -> String {
        let devices: Vec<String> = self
            .devices
            .iter()
            .map(|device| {
                let mut names: Vec<&str> = device.interfaces.keys().map(String::as_str).collect();
                names.sort_unstable();
                format!("{names:?} done={} resumed={}", device.done, device.resumed.is_some())
            })
            .collect();
        format!(
            "seat mask {:#x}, bound {}, devices [{}]",
            self.capabilities,
            self.bound,
            devices.join("; ")
        )
    }

    /// Whether every device has resumed and every interface the probes send through, for each capability the seat
    /// advertised, is on one of them.
    fn devices_cover_the_seat(&self) -> bool {
        !self.devices.is_empty()
            && self
                .devices
                .iter()
                .all(|device| device.done && device.resumed.is_some())
            && SENT_THROUGH
                .iter()
                .filter(|name| self.advertised.iter().any(|advertised| advertised == **name))
                .all(|name| {
                    self.devices
                        .iter()
                        .any(|device| device.resumed.is_some() && device.interfaces.contains_key(*name))
                })
    }

    /// The device that carries interface `T`, the serial it resumed with and the interface.
    fn device_with<T: reis::Interface>(&self) -> Result<(ei::Device, u32, T), TargetError> {
        let device = self
            .devices
            .iter()
            .find(|device| device.interfaces.contains_key(T::NAME))
            .ok_or_else(|| {
                self.live(format!(
                    "the seat has no device with {} (the seat advertised {:?}; devices: {:?})",
                    T::NAME,
                    capability_names(&self.advertised),
                    self.devices
                        .iter()
                        .map(|device| device.interfaces.keys().cloned().collect::<Vec<_>>())
                        .collect::<Vec<_>>(),
                ))
            })?;
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
