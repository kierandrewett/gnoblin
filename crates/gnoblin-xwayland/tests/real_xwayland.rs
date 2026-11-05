//! The real Xwayland, driven by a real X11 client against a real session.
//!
//! Nothing here is a mock: a headless `gnoblin-wayland` session starts
//! `/usr/bin/Xwayland` through Smithay's spawn, this test connects an `x11rb` client to the
//! display it took, creates an X11 window and reads both what the window manager did to it
//! (the session's observation) and what the client was told (its own event stream).
//!
//! The tests are skipped **with a printed reason** only when the machine has no `Xwayland`
//! binary: a test suite that silently passes because the feature it checks is absent is
//! worse than one that fails. Everywhere else they run, because a compositor that cannot
//! start Xwayland is the failure they exist for.
//!
//! ## Why the session runs on the test's thread
//!
//! A calloop loop is not `Send`, and neither is the backend it drives. The session runs on
//! the test thread and the X11 client choreography runs on a worker, which is also how the
//! real arrangement looks: one process composes, the other is an X11 application.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, CreateWindowAux, PropMode, WindowClass};
use x11rb::rust_connection::{DefaultStream, RustConnection};
use x11rb::wrapper::ConnectionExt as _;

use gnoblin_backend::headless::Headless;
use gnoblin_wayland::{Config, ControlHandle, LoopError, ObservedWindow, Server, SessionSnapshot};

use gnoblin_xwayland::{Handle, Settings, Status, Xwayland};

/// How long a test waits for a state before failing.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The display the session reached, or the reason it will not.
fn wait_ready(handle: &Handle) -> u32 {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match handle.status() {
            Status::Ready { display, .. } => return display,
            Status::Disabled { reason } | Status::Stopped { reason } => {
                panic!("the Xwayland session is off: {reason}");
            }
            status => {
                assert!(
                    Instant::now() < deadline,
                    "the session never became ready; last status: {status:?}"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// The `Xwayland` process serving a display number, found by its command line.
///
/// The test kills the *server it started*, not every Xwayland on the machine, so the
/// display number is part of the match.
fn xwayland_pid(display: u32) -> Option<u32> {
    let argument = format!(":{display}");
    for entry in std::fs::read_dir("/proc").ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(command) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let text = String::from_utf8_lossy(&command);
        let mut fields = text.split('\0');
        let Some(program) = fields.next() else {
            continue;
        };
        if !program.ends_with("Xwayland") {
            continue;
        }
        if fields.any(|field| field == argument) {
            return Some(pid);
        }
    }
    None
}

/// What a test can reach of a running session.
#[derive(Clone)]
struct SessionHandle {
    control: ControlHandle,
    observation: std::sync::Arc<std::sync::Mutex<SessionSnapshot>>,
    xwayland: Handle,
}

impl SessionHandle {
    fn windows(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let snapshot = self.windows();
            if predicate(&snapshot) {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "the session never reached the expected state; last state: {snapshot:#?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_title(&self, title: &str) -> ObservedWindow {
        let wanted = title.to_owned();
        let snapshot = self.wait_for(move |snapshot| snapshot.windows().any(|window| window.title == wanted));
        snapshot
            .windows()
            .find(|window| window.title == title)
            .cloned()
            .expect("the window the predicate found")
    }

    fn wait_for_no_windows(&self) -> SessionSnapshot {
        self.wait_for(|snapshot| snapshot.windows().count() == 0)
    }
}

/// Stops the session when the worker finishes, including when it panics.
struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A session that is created but not running yet.
struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink the socket.
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    server: Server,
    handle: SessionHandle,
}

impl Session {
    /// A session with an Xwayland session configured from `settings`.
    fn start(settings: Settings, path: Option<std::ffi::OsString>) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket),
            ..Config::default()
        };
        let xwayland = Xwayland::new(settings, path);
        let handle = xwayland.handle();
        let (session, windows) = xwayland.parts();
        let server = Server::with_xwayland(
            Box::new(Headless::new()),
            config,
            gnoblin_wayland::XwaylandSession::new(Box::new(session), Box::new(windows)),
        )
        .expect("the session starts");
        let handle = SessionHandle {
            control: server.control(),
            observation: server.observation(),
            xwayland: handle,
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the session on this thread and `body` on a worker, then joins both.
    fn run(self, body: impl FnOnce(&SessionHandle) + Send + 'static) {
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
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// The display the tests use, with the skip rule the crate documents.
///
/// The `PATH` is read here rather than inside the session so the test exercises the same
/// probe a supervised start uses.
fn display_path() -> Option<std::ffi::OsString> {
    let path = std::env::var_os("PATH");
    match gnoblin_xwayland::probe(path.as_deref()) {
        Ok(_) => path,
        Err(reason) => {
            println!("skipping: {reason}");
            None
        }
    }
}

/// A real X11 client on a real display.
struct XClient {
    connection: RustConnection,
    screen: usize,
    root: u32,
}

impl XClient {
    fn connect(display: u32) -> Self {
        let path = PathBuf::from(format!("/tmp/.X11-unix/X{display}"));
        let stream = UnixStream::connect(&path)
            .unwrap_or_else(|error| panic!("the X display socket {}: {error}", path.display()));
        let (stream, _) = DefaultStream::from_unix_stream(stream).expect("a non-blocking stream");
        let connection = RustConnection::connect_to_stream(stream, 0)
            .unwrap_or_else(|error| panic!("an X11 connection to display :{display}: {error}"));
        let screen = 0;
        let root = connection.setup().roots[screen].root;
        Self {
            connection,
            screen,
            root,
        }
    }

    /// A mapped-able window with a title and a class, not yet mapped.
    fn create_window(&self, title: &str, class: &str, override_redirect: bool) -> u32 {
        let screen = &self.connection.setup().roots[self.screen];
        let window = self.connection.generate_id().expect("a window id");
        let aux = CreateWindowAux::new()
            .background_pixel(screen.white_pixel)
            .event_mask(
                x11rb::protocol::xproto::EventMask::STRUCTURE_NOTIFY
                    | x11rb::protocol::xproto::EventMask::EXPOSURE
                    | x11rb::protocol::xproto::EventMask::PROPERTY_CHANGE,
            )
            .override_redirect(u32::from(override_redirect));
        self.connection
            .create_window(
                screen.root_depth,
                window,
                self.root,
                10,
                10,
                300,
                200,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &aux,
            )
            .expect("a window");
        self.connection
            .change_property8(
                PropMode::REPLACE,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                title.as_bytes(),
            )
            .expect("a title");
        let class = format!("instance\0{class}\0");
        self.connection
            .change_property8(
                PropMode::REPLACE,
                window,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                class.as_bytes(),
            )
            .expect("a class");
        self.connection.flush().expect("the requests reach the server");
        window
    }

    fn map(&self, window: u32) {
        self.connection.map_window(window).expect("a map request");
        self.connection.flush().expect("the requests reach the server");
    }

    fn unmap(&self, window: u32) {
        self.connection.unmap_window(window).expect("an unmap request");
        self.connection.flush().expect("the requests reach the server");
    }

    /// Waits for the first `ConfigureNotify` about a window.
    fn configured(&self, window: u32) -> Option<x11rb::protocol::xproto::ConfigureNotifyEvent> {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            while let Some(event) = self.connection.poll_for_event().expect("an event") {
                if let Event::ConfigureNotify(configure) = event
                    && configure.window == window
                {
                    return Some(configure);
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    }

    /// Whether the connection still answers a round trip, which is what "the display is
    /// usable" means.
    fn round_trip(&self) -> bool {
        self.connection
            .get_input_focus()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some()
    }
}

/// Xwayland starts, takes a display and answers a real client on it.
#[test]
fn xwayland_starts_and_the_display_is_usable() {
    let Some(path) = display_path() else {
        return;
    };
    let session = Session::start(Settings::default(), Some(path));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |_handle| {
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        assert_eq!(
            client.connection.setup().roots.len(),
            1,
            "one screen, as Xwayland serves"
        );
        assert!(client.round_trip(), "the display answers a round trip");
        assert!(
            client.connection.setup().roots[client.screen].width_in_pixels > 0,
            "the root window has an extent"
        );
    });
}

/// An X11 window maps, becomes a managed window with its title and class, is configured
/// through the window manager's geometry, and leaves the manager when it unmaps.
#[test]
fn an_x11_window_is_managed_configured_and_removed() {
    let Some(path) = display_path() else {
        return;
    };
    let session = Session::start(Settings::default(), Some(path));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        let window = client.create_window("gnoblin test window", "GnoblinTest", false);
        client.map(window);

        let managed = handle.wait_for_title("gnoblin test window");
        assert_eq!(
            managed.app_id, "GnoblinTest",
            "the WM_CLASS class is the application id"
        );
        assert!(managed.mapped, "the window is mapped");
        assert!(managed.visible, "the window is on the active workspace");

        // The client hears the geometry the window manager placed it at, which is what
        // "configured through the wm geometry" means for an X11 client.
        let configure = client.configured(window).expect("the client is configured");
        assert_eq!(
            (i32::from(configure.x), i32::from(configure.y)),
            (managed.geometry.x, managed.geometry.y),
            "the client was told the manager's position"
        );
        assert_eq!(
            (i32::from(configure.width), i32::from(configure.height)),
            (
                i32::try_from(managed.geometry.width).expect("a width that fits"),
                i32::try_from(managed.geometry.height).expect("a height that fits"),
            ),
            "the client was told the manager's size"
        );

        client.unmap(window);
        handle.wait_for_no_windows();
    });
}

/// An override-redirect window is never managed and never focused.
#[test]
fn an_override_redirect_popup_is_not_managed() {
    let Some(path) = display_path() else {
        return;
    };
    let session = Session::start(Settings::default(), Some(path));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        let popup = client.create_window("gnoblin test popup", "GnoblinTest", true);
        client.map(popup);

        // Give the session plenty of time to be wrong about it.
        let deadline = Instant::now() + Duration::from_millis(750);
        while Instant::now() < deadline {
            assert!(
                handle.windows().windows().count() == 0,
                "an override-redirect window was managed: {:?}",
                handle.windows()
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(client.round_trip(), "the session keeps serving the client");
    });
}

/// A client that goes away leaves no window behind: the server destroys its windows with
/// the connection, and the manager hears the destroy rather than an unmap that never comes.
#[test]
fn a_client_that_disconnects_leaves_no_window() {
    let Some(path) = display_path() else {
        return;
    };
    let session = Session::start(Settings::default(), Some(path));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        let window = client.create_window("gnoblin doomed window", "GnoblinTest", false);
        client.map(window);
        handle.wait_for_title("gnoblin doomed window");

        // Dropping the connection is what a crashed client looks like to the server.
        drop(client);
        handle.wait_for_no_windows();
    });
}

/// A `DISPLAY` is never put into the process environment: the number is data the launch
/// carries, so an X11-capable session cannot leak it into every child it starts.
#[test]
fn a_launch_carries_the_display_and_no_environment() {
    let settings = Settings::default();
    let automatic = settings.launch(None);
    assert_eq!(automatic.display, None, "the server picks the number");
    assert!(automatic.env.is_empty(), "a launch adds no environment");

    let pinned = settings.launch(Some(7));
    assert_eq!(pinned.display, Some(7));
    assert!(
        pinned.env.is_empty(),
        "not even a fixed display is an environment variable"
    );
}

/// Killing Xwayland restarts it by policy, and a fresh client works on the new display.
#[test]
fn killing_xwayland_restarts_it_and_a_fresh_client_works() {
    let Some(path) = display_path() else {
        return;
    };
    let session = Session::start(Settings::default(), Some(path));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let first = wait_ready(&xwayland);
        let before = XClient::connect(first);
        assert!(before.round_trip(), "the first display answers");

        let pid = xwayland_pid(first).expect("the Xwayland process is in /proc");
        let killed = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status()
            .expect("kill runs");
        assert!(killed.success(), "the server was signalled");

        // The client that was connected to the dead server is disconnected, which is
        // what "the clients of the old display are gone" means; nothing reconnects it.
        let deadline = Instant::now() + TIMEOUT;
        while before.round_trip() {
            assert!(Instant::now() < deadline, "the client of the dead server still answers");
            thread::sleep(Duration::from_millis(20));
        }

        // The policy restarts with a growing delay, so the wait is the test's clock. The
        // number may be the same one: the lock and the socket went with the server.
        let second = loop {
            match xwayland.status() {
                Status::Ready { display, restarts } if restarts >= 1 => break display,
                Status::Ready { .. } | Status::Restarting { .. } | Status::Starting { .. } => {}
                status => panic!("the session did not restart: {status:?}"),
            }
            assert!(
                Instant::now() < deadline,
                "the session never restarted; last status: {:?}",
                xwayland.status()
            );
            thread::sleep(Duration::from_millis(20));
        };
        assert!(xwayland_pid(second).is_some(), "a live Xwayland serves the new display");

        let client = XClient::connect(second);
        assert!(client.round_trip(), "the new display answers");
        let window = client.create_window("gnoblin restarted window", "GnoblinTest", false);
        client.map(window);
        let managed = handle.wait_for_title("gnoblin restarted window");
        assert_eq!(managed.app_id, "GnoblinTest");
        let _ = first;
    });
}

/// No `Xwayland` binary is a disabled session with a reason, not a panic and not a hang.
#[test]
fn a_missing_binary_is_a_disabled_session() {
    let session = Session::start(Settings::default(), Some(std::ffi::OsString::from("/nonexistent")));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match xwayland.status() {
                Status::Disabled { reason } => {
                    assert!(reason.contains("no Xwayland binary"), "{reason}");
                    break;
                }
                status => {
                    assert!(
                        Instant::now() < deadline,
                        "the session never disabled itself; last status: {status:?}"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        // The session keeps serving Wayland clients and stops cleanly, which is what "no
        // panic" means for a compositor with X11 switched off.
        assert_eq!(handle.windows().windows().count(), 0);
    });
}
