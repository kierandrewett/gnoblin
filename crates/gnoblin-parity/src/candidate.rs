//! The Rust candidate target: a real headless session, driven the way a user would.
//!
//! [`CandidateTarget::observe`] starts `gnoblin-compositor` on a thread with both of
//! its sockets in a private temporary directory, waits until the IPC socket accepts,
//! then runs the scenario's steps in order against it:
//!
//! - the IPC peer is the `gnoblinctl` library entry point, called the way its binary
//!   calls it, so the observations are what a script would read;
//! - the Wayland peer is a real `wayland-client` connection that creates and maps an
//!   `xdg_toplevel`, and destroys it when the scenario asks.
//!
//! A step's id selects the probe that produces its observation. The probes are a
//! closed set, so a step with an unknown id fails the run instead of being observed as
//! nothing. The session is stopped when the steps are done; the Wayland peer is dropped
//! first, so the session sees its client leave before it stops.

use std::ffi::OsString;
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_compositor::{Compositor, SessionConfig, StopHandle};
use gnoblinctl::SocketConnection;
use serde_json::{Value, json};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection as WaylandConnection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::target::{Observations, Target, TargetError};

/// The target name the report prints and the errors name.
const TARGET: &str = "candidate";
/// How long a step waits for the session to reach the state it asks for.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How often a waiting step looks again.
const POLL: Duration = Duration::from_millis(5);
/// The app id and title of the toplevel the window steps create.
const APP_ID: &str = "org.gnoblin.parity";
const WINDOW_TITLE: &str = "parity-window";
/// A window id no session hands out, for the refusal probe.
const UNKNOWN_WINDOW: &str = "9999999";

/// Runs each scenario against a fresh headless Rust compositor.
#[derive(Debug)]
pub struct CandidateTarget;

impl CandidateTarget {
    /// The target itself needs no configuration, so it is a unit.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CandidateTarget {
    fn default() -> Self {
        Self::new()
    }
}

impl Target for CandidateTarget {
    fn name(&self) -> &str {
        TARGET
    }

    fn observe(&self, scenario: &crate::scenario::Scenario) -> Result<Observations, TargetError> {
        let mut session = CandidateSession::start()?;
        let mut observations = Observations::new();
        for step in &scenario.steps {
            let value = session.probe(&step.id)?;
            observations.insert(step.id.clone(), value);
        }
        session.finish()?;
        Ok(observations)
    }
}

/// A `TargetError` for this target.
fn live(detail: impl Into<String>) -> TargetError {
    TargetError::Live {
        target: TARGET.to_owned(),
        detail: detail.into(),
    }
}

/// One running session and the Wayland peer the steps keep between them.
struct CandidateSession {
    peers: Option<Peers>,
    thread: Option<JoinHandle<Result<(), gnoblin_compositor::Error>>>,
    stop: StopHandle,
    ipc: PathBuf,
    wayland: PathBuf,
    /// Kept alive for the session's lifetime: dropping it would unlink both sockets.
    _directory: tempfile::TempDir,
}

/// The Wayland client and the toplevel it may have mapped.
struct Peers {
    client: Wayland,
    window: Option<CandidateWindow>,
}

impl CandidateSession {
    /// Starts a session whose sockets are inside a private temporary directory.
    fn start() -> Result<Self, TargetError> {
        let directory = private_directory()?;
        let ipc = directory.path().join("ipc.sock");
        let wayland = directory.path().join("wayland.sock");
        let stop = StopHandle::new();
        let config = SessionConfig {
            ipc_socket: Some(ipc.clone()),
            stop: Some(stop.clone()),
            wayland: gnoblin_wayland::Config {
                socket_path: Some(wayland.clone()),
                ..gnoblin_wayland::Config::default()
            },
            ..SessionConfig::default()
        };
        let thread = std::thread::Builder::new()
            .name("candidate-compositor".to_owned())
            .spawn(move || Compositor::run(config))
            .map_err(|error| live(format!("cannot start the compositor thread: {error}")))?;
        let mut session = Self {
            peers: None,
            thread: Some(thread),
            stop,
            ipc,
            wayland,
            _directory: directory,
        };
        session.wait_until_listening()?;
        Ok(session)
    }

    /// Waits until the IPC socket accepts, or the compositor stops first.
    fn wait_until_listening(&mut self) -> Result<(), TargetError> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if UnixStream::connect(&self.ipc).is_ok() {
                return Ok(());
            }
            if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
                let result = self.join();
                return Err(live(format!("the compositor stopped before it listened: {result:?}")));
            }
            if Instant::now() >= deadline {
                return Err(live(format!("the compositor never listened on {}", self.ipc.display())));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Stops the session and reports how the compositor ended.
    fn finish(mut self) -> Result<(), TargetError> {
        self.peers = None;
        self.stop.request();
        self.join()
            .map_err(|error| live(format!("the compositor failed: {error}")))
    }

    fn join(&mut self) -> Result<(), gnoblin_compositor::Error> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        match thread.join() {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Produces the observation for one step id.
    fn probe(&mut self, step: &str) -> Result<Value, TargetError> {
        match step {
            "workspace.list" => self.ctl_json(&["--json", "workspace", "list"]),
            "window.open" => self.open_window(),
            "window.close" => self.close_window(),
            "window.kill.unknown" => Ok(self.kill_unknown_window()),
            other => Err(live(format!("no probe for step `{other}`"))),
        }
    }

    /// Runs `gnoblinctl` against the session and returns its exit code and output.
    fn ctl(&self, arguments: &[&str]) -> Ctl {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = gnoblinctl::run(
            arguments.iter().map(|argument| OsString::from(*argument)),
            SocketConnection::at(self.ipc.clone()),
            &mut out,
            &mut err,
        );
        Ctl {
            code,
            out: String::from_utf8_lossy(&out).into_owned(),
            err: String::from_utf8_lossy(&err).into_owned(),
        }
    }

    /// Runs `gnoblinctl` and reads its standard output as JSON. A non-zero exit is a
    /// failure of the step, with the message the command printed.
    fn ctl_json(&self, arguments: &[&str]) -> Result<Value, TargetError> {
        let command = arguments.join(" ");
        let ctl = self.ctl(arguments);
        if ctl.code != 0 {
            return Err(live(format!(
                "`gnoblinctl {command}` exited {}: {}",
                ctl.code,
                ctl.err.trim_end()
            )));
        }
        serde_json::from_str(ctl.out.trim_end())
            .map_err(|error| live(format!("`gnoblinctl {command}` printed no JSON: {error}")))
    }

    /// Whether `window list` names the test window, by the quoted title it prints.
    fn window_listed(&self) -> Result<bool, TargetError> {
        let ctl = self.ctl(&["window", "list"]);
        if ctl.code != 0 {
            return Err(live(format!(
                "`gnoblinctl window list` exited {}: {}",
                ctl.code,
                ctl.err.trim_end()
            )));
        }
        Ok(ctl.out.contains(&format!("\"{WINDOW_TITLE}\"")))
    }

    /// Waits until the test window is listed (`present`) or gone.
    fn wait_for_window(&self, present: bool) -> Result<(), TargetError> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if self.window_listed()? == present {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let state = if present { "listed" } else { "unlisted" };
                return Err(live(format!("window `{WINDOW_TITLE}` never became {state}")));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Maps a toplevel through a real Wayland client, then reports what `window list`
    /// shows for it: its title, app id, and whether it has focus.
    fn open_window(&mut self) -> Result<Value, TargetError> {
        let mut client = Wayland::connect(&self.wayland)?;
        let window = client.map_window(APP_ID, WINDOW_TITLE)?;
        self.peers = Some(Peers {
            client,
            window: Some(window),
        });
        self.wait_for_window(true)?;
        let ctl = self.ctl(&["window", "list"]);
        Ok(json!({
            "listed": true,
            "app_id_listed": ctl.out.contains(&format!("\"{APP_ID}\"")),
            "focused": ctl.out.contains("focused=true"),
        }))
    }

    /// Destroys the toplevel the way a well-behaved client does, then reports whether
    /// the session still lists it.
    fn close_window(&mut self) -> Result<Value, TargetError> {
        let peers = self
            .peers
            .as_mut()
            .ok_or_else(|| live("`window.close` needs `window.open` first"))?;
        if let Some(window) = peers.window.take() {
            peers.client.destroy(window)?;
        }
        self.wait_for_window(false)?;
        let ctl = self.ctl(&["window", "list"]);
        Ok(json!({ "listed": false, "list_empty": ctl.out.is_empty() }))
    }

    /// Asks the session to kill a window that is not there. The refusal is the
    /// observation: the exit code and the message the command printed.
    fn kill_unknown_window(&self) -> Value {
        let ctl = self.ctl(&["window", "call", UNKNOWN_WINDOW, "kill"]);
        json!({ "exit": ctl.code, "stderr": ctl.err.trim_end() })
    }
}

impl Drop for CandidateSession {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.peers = None;
            self.stop.request();
            let _ = self.join();
        }
    }
}

/// A temporary directory with the mode the IPC socket requires.
///
/// `tempfile` creates its directories `0755`, which the compositor refuses on purpose,
/// so the directory is made private the way `$XDG_RUNTIME_DIR` is.
fn private_directory() -> Result<tempfile::TempDir, TargetError> {
    let directory =
        tempfile::tempdir().map_err(|error| live(format!("cannot create a temporary directory: {error}")))?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .map_err(|error| live(format!("cannot make the temporary directory private: {error}")))?;
    Ok(directory)
}

/// The result of one `gnoblinctl` run.
struct Ctl {
    code: u8,
    out: String,
    err: String,
}

/// The Wayland client's dispatch state. Only the events a toplevel needs are answered:
/// pings are ponged and configures acknowledged, so the session never sees a stalled
/// client.
#[derive(Default)]
struct Protocol;

/// A real Wayland client connection, with the globals a toplevel needs.
struct Wayland {
    queue: EventQueue<Protocol>,
    state: Protocol,
    qh: QueueHandle<Protocol>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
}

/// One toplevel's protocol objects.
struct CandidateWindow {
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

impl Wayland {
    /// Connects and binds the globals a toplevel needs.
    fn connect(socket: &Path) -> Result<Self, TargetError> {
        let stream = UnixStream::connect(socket)
            .map_err(|error| live(format!("cannot connect to the Wayland socket: {error}")))?;
        let connection = WaylandConnection::from_socket(stream)
            .map_err(|error| live(format!("cannot start a Wayland connection: {error}")))?;
        let (globals, queue) = registry_queue_init::<Protocol>(&connection)
            .map_err(|error| live(format!("the session's global list: {error}")))?;
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .map_err(|error| live(format!("the session does not advertise wl_compositor: {error}")))?;
        let shm: wl_shm::WlShm = globals
            .bind(&qh, 1..=1, ())
            .map_err(|error| live(format!("the session does not advertise wl_shm: {error}")))?;
        let wm_base: xdg_wm_base::XdgWmBase = globals
            .bind(&qh, 1..=6, ())
            .map_err(|error| live(format!("the session does not advertise xdg_wm_base: {error}")))?;
        Ok(Self {
            queue,
            state: Protocol,
            qh,
            compositor,
            shm,
            wm_base,
        })
    }

    /// Processes every event the session has sent.
    fn roundtrip(&mut self) -> Result<(), TargetError> {
        self.queue
            .roundtrip(&mut self.state)
            .map(|_| ())
            .map_err(|error| live(format!("the Wayland round trip failed: {error}")))
    }

    /// Creates a toplevel, lets the session configure it, and maps it with a buffer.
    fn map_window(&mut self, app_id: &str, title: &str) -> Result<CandidateWindow, TargetError> {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip()?;
        let window = CandidateWindow {
            surface,
            xdg_surface,
            toplevel,
        };
        self.attach_buffer(&window, 200, 150)?;
        Ok(window)
    }

    /// Attaches a shared-memory buffer and commits, which maps the window.
    fn attach_buffer(&mut self, window: &CandidateWindow, width: i32, height: i32) -> Result<(), TargetError> {
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().map_err(|error| live(format!("cannot create a buffer file: {error}")))?;
        file.set_len(u64::try_from(length).unwrap_or_default())
            .map_err(|error| live(format!("cannot size a buffer file: {error}")))?;
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage(0, 0, width, height);
        window.surface.commit();
        self.roundtrip()
    }

    /// Closes a window the way a well-behaved client does.
    fn destroy(&mut self, window: CandidateWindow) -> Result<(), TargetError> {
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        self.roundtrip()
    }
}

/// Objects whose events a toplevel never needs answered.
macro_rules! ignore_events {
    ($($object:ty),* $(,)?) => {
        $(
            impl Dispatch<$object, ()> for Protocol {
                fn event(
                    _: &mut Self,
                    _: &$object,
                    _: <$object as Proxy>::Event,
                    _: &(),
                    _: &WaylandConnection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        )*
    };
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    xdg_toplevel::XdgToplevel,
);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Protocol {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Protocol {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Protocol {
    fn event(
        _: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
        }
    }
}
