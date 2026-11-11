//! The session wlcs runs against, and the devices it injects input through.
//!
//! One [`ConformanceSession`] is one compositor: a headless backend with its single virtual output, the software renderer
//! and a socket in a private directory, running on a thread of its own. wlcs creates one per test and drives it
//! only through the calls here, so nothing below touches a shared socket or the user's session.
//!
//! The harness is safe Rust end to end. The C ABI that hands it to wlcs is in `ffi.rs`.

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gnoblin_backend::headless::Headless;
use gnoblin_backend::{InputEvent, TouchPhase};
use gnoblin_types::{OutputId, Point};
use gnoblin_wayland::{Config, ControlHandle, NewWindowFocus, Server, SessionSnapshot, WmOptions};

/// What the session thread hands back once the compositor is listening.
type Started = Result<(ControlHandle, Arc<Mutex<SessionSnapshot>>), String>;

/// A running compositor and the bookkeeping wlcs needs around it.
pub struct ConformanceSession {
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
    /// The tag the session gave each client, by the descriptor number of the end wlcs holds.
    ///
    /// wlcs names a client by its `wl_display`, and the only thing that identifies the connection from there is the
    /// descriptor `wl_display_get_fd` returns, which is the one [`ConformanceSession::client_socket`] handed out.
    clients: Mutex<HashMap<RawFd, u64>>,
    /// The next touch slot, so two touch devices never share a point.
    next_touch_slot: AtomicU32,
    runner: Mutex<Option<JoinHandle<()>>>,
    /// Kept alive for the session's lifetime: dropping it unlinks the socket's directory.
    _directory: tempfile::TempDir,
}

impl ConformanceSession {
    /// Starts the compositor and returns once it can serve a connection.
    ///
    /// # Errors
    ///
    /// The reason the session did not start: no temporary directory, no thread, or the session's own
    /// [`gnoblin_wayland::StartupError`].
    pub fn start() -> Result<Self, String> {
        let directory = tempfile::tempdir().map_err(|error| format!("no directory for the socket: {error}"))?;
        let socket = directory.path().join("wayland-wlcs.sock");
        let (ready, started) = mpsc::channel();
        let runner = thread::Builder::new()
            .name("gnoblin-wlcs".to_owned())
            .spawn(move || run_session(socket, &ready))
            .map_err(|error| format!("no thread for the session: {error}"))?;
        let (control, observation) = match started.recv() {
            Ok(Ok(parts)) => parts,
            Ok(Err(reason)) => {
                let _ = runner.join();
                return Err(reason);
            }
            Err(_) => {
                let _ = runner.join();
                return Err("the session thread ended before it started".to_owned());
            }
        };
        // A window's pixels and its hit region move together from the first frame: the suite injects input the
        // moment it has placed a window, and a fade or a move still under way would put the window somewhere else.
        control.set_reduced_motion(true);
        Ok(Self {
            control,
            observation,
            clients: Mutex::new(HashMap::new()),
            next_touch_slot: AtomicU32::new(0),
            runner: Mutex::new(Some(runner)),
            _directory: directory,
        })
    }

    /// Stops the compositor and waits for its thread, so nothing of it outlives the test.
    pub fn stop(&self) {
        self.control.stop();
        let runner = self.runner.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(runner) = runner {
            let _ = runner.join();
        }
    }

    /// A new connection to the compositor: the end the client holds.
    ///
    /// The harness remembers the connection by the stream's descriptor number, which is what
    /// [`ConformanceSession::position_window`] is later asked about.
    ///
    /// # Errors
    ///
    /// When the socket pair cannot be made or the session does not adopt the other end.
    pub fn client_stream(&self) -> io::Result<UnixStream> {
        let (server_end, client_end) = UnixStream::pair()?;
        let tag = self
            .control
            .adopt_client(server_end)
            .ok_or_else(|| io::Error::other("the session did not adopt the connection"))?;
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(client_end.as_raw_fd(), tag);
        Ok(client_end)
    }

    /// A new connection to the compositor, as the descriptor of the end the client holds.
    ///
    /// The descriptor belongs to the caller: wlcs hands it to `wl_display_connect_to_fd`, which closes it with the
    /// display.
    ///
    /// # Errors
    ///
    /// As [`ConformanceSession::client_stream`].
    pub fn client_socket(&self) -> io::Result<RawFd> {
        self.client_stream().map(IntoRawFd::into_raw_fd)
    }

    /// Moves the window of a client's surface so its top left is at `(x, y)` in compositor space.
    ///
    /// `client_fd` is `wl_display_get_fd` of the client's display and `surface` is `wl_proxy_get_id` of its
    /// `wl_surface`. The position is exact, with no work-area clamp: the suite places windows where its geometry needs
    /// them. `false` when the client or the surface is unknown or the surface is not a floating managed window.
    #[must_use]
    pub fn position_window(&self, client_fd: RawFd, surface: u32, x: i32, y: i32) -> bool {
        let tag = self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&client_fd)
            .copied();
        let Some(tag) = tag else {
            return false;
        };
        self.control
            .place_surface(tag, surface, Point::new(x, y))
            .unwrap_or(false)
    }

    /// An input source for one fake device.
    #[must_use]
    pub fn injector(&self) -> Injector {
        Injector {
            control: self.control.clone(),
            observation: Arc::clone(&self.observation),
        }
    }

    /// A touch slot no other touch device of this session uses.
    #[must_use]
    pub fn next_touch_slot(&self) -> u32 {
        self.next_touch_slot.fetch_add(1, Ordering::Relaxed)
    }

    /// The window state right now.
    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot {
        self.observation.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Drop for ConformanceSession {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Builds and runs the session on the calling thread, which is the thread the loop must stay on.
fn run_session(socket: PathBuf, ready: &mpsc::Sender<Started>) {
    let config = Config {
        socket_path: Some(socket),
        // The conformance suite expects a new window to take the keyboard, as the compositors it was written for do;
        // the document's default refuses that.
        wm: WmOptions {
            new_window_focus: NewWindowFocus::Allow,
            ..WmOptions::default()
        },
        // A client that does not answer a ping would have its windows marked unresponsive in the middle of a test.
        check_alive_timeout: Duration::ZERO,
        ..Config::default()
    };
    let server = match Server::new(Box::new(Headless::new()), config) {
        Ok(server) => server,
        Err(error) => {
            let _ = ready.send(Err(format!("the session did not start: {error}")));
            return;
        }
    };
    if ready.send(Ok((server.control(), server.observation()))).is_err() {
        return;
    }
    if let Err(error) = server.run() {
        // The thread has no caller to return to: the next call into the harness finds the session gone.
        eprintln!("gnoblin-wlcs: the session's loop ended with an error: {error}");
    }
}

/// Injects the events of one fake device into the session.
///
/// Every call returns after the session has applied the event, so the client that asks for a round trip next is
/// answered after the event reached it. Injection alone only queues the event, and a round trip can overtake it.
#[derive(Clone)]
pub struct Injector {
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Injector {
    /// The output a position is relative to: the session's first, which is the only one.
    fn output(&self) -> Option<OutputId> {
        self.observation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .outputs()
            .first()
            .copied()
    }

    fn deliver(&self, event: InputEvent) {
        self.control.inject_input(event);
        let _ = self.control.sync();
    }

    /// Moves the pointer to a position in compositor space.
    pub fn pointer_move_to(&self, x: i32, y: i32) {
        if let Some(output) = self.output() {
            self.deliver(InputEvent::PointerMotion {
                output,
                position: Point::new(x, y),
            });
        }
    }

    /// Moves the pointer by a distance in compositor space.
    pub fn pointer_move_by(&self, dx: i32, dy: i32) {
        self.deliver(InputEvent::PointerMotionRelative { dx, dy });
    }

    /// Presses or releases a pointer button, by its Linux input event code.
    pub fn pointer_button(&self, button: u32, pressed: bool) {
        self.deliver(InputEvent::PointerButton { button, pressed });
    }

    /// Presses or releases a key, by its Linux input event code.
    pub fn key(&self, keycode: u32, pressed: bool) {
        self.deliver(InputEvent::Key { keycode, pressed });
    }

    /// A touch point changing state at a position in compositor space.
    pub fn touch(&self, slot: u32, x: i32, y: i32, phase: TouchPhase) {
        if let Some(output) = self.output() {
            self.deliver(InputEvent::Touch {
                output,
                slot,
                position: Point::new(x, y),
                phase,
            });
        }
    }
}
