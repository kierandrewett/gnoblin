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
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, CreateGCAux, CreateWindowAux, EventMask, PropMode, Rectangle, WindowClass,
};
use x11rb::rust_connection::{DefaultStream, RustConnection};
use x11rb::wrapper::ConnectionExt as _;

use gnoblin_backend::headless::Headless;
use gnoblin_render::transaction::{TreeChange, TreeValue};
use gnoblin_wayland::{
    Config, ControlHandle, FrameTreeEvent, LoopError, ObservedWindow, Server, SessionSnapshot, TreeBatch,
    TreeFrameInput, TreeHooks,
};

use gnoblin_xwayland::{Capabilities, Handle, Settings, Status, Xwayland};

/// The stand-in shell the window-request tests of `gnoblin-wayland` use: it holds the tree, reads what the compositor
/// publishes and writes the answers a real shell writes. It is the same file, so the two suites cannot drift apart.
#[path = "../../gnoblin-wayland/tests/session/shell.rs"]
#[allow(dead_code)]
mod shell;
use shell::Shell;

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
        Self::start_with_tree(settings, path, TreeHooks::default())
    }

    /// A session whose tree publishes through `tree`, as a shell's connection would hear it.
    fn start_with_tree(mut settings: Settings, path: Option<std::ffi::OsString>, tree: TreeHooks) -> Self {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("a temporary directory");
        // Xwayland inherits only PATH and XDG_RUNTIME_DIR from the compositor, so the
        // test must not depend on the ambient runtime dir: the child gets its own,
        // created mode 0700 as XDG requires.
        let runtime = directory.path().join("xdg-runtime");
        std::fs::create_dir(&runtime).expect("a runtime directory");
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).expect("runtime directory mode");
        settings.runtime_dir = Some(runtime);
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket),
            tree,
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

/// The settings the real-server tests start with.
///
/// Byte-swapped clients are allowed here, not refused: the default refuses them with
/// `-byteswappedclients`, which a server before that switch (Xwayland 22.1, the
/// environment these tests also run in) rejects as an unknown option. Allowing needs no
/// switch, so the tests run on both old and new servers and exercise the same paths.
/// Refusal itself is covered by the unit tests in `settings.rs`.
fn real_settings() -> Settings {
    Settings {
        allow_byte_swapped_clients: true,
        ..Settings::default()
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

    /// Fills a window with its foreground colour, which is what gives the server pixels to
    /// present for it.
    fn paint(&self, window: u32) {
        let screen = &self.connection.setup().roots[self.screen];
        let gc = self.connection.generate_id().expect("a graphics context id");
        self.connection
            .create_gc(gc, window, &CreateGCAux::new().foreground(screen.black_pixel))
            .expect("a graphics context");
        self.connection
            .poly_fill_rectangle(
                window,
                gc,
                &[Rectangle {
                    x: 0,
                    y: 0,
                    width: 300,
                    height: 200,
                }],
            )
            .expect("a fill");
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
    let session = Session::start(real_settings(), Some(path));
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
    let session = Session::start(real_settings(), Some(path));
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
    let session = Session::start(real_settings(), Some(path));
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
    let session = Session::start(real_settings(), Some(path));
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
    let capabilities = Capabilities {
        byte_order_switch: true,
    };
    let automatic = settings
        .launch(None, capabilities)
        .expect("the default settings launch on a server with the switch");
    assert_eq!(automatic.display, None, "the server picks the number");
    assert!(automatic.env.is_empty(), "a launch adds no environment");

    let pinned = settings
        .launch(Some(7), capabilities)
        .expect("the default settings launch on a server with the switch");
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
    let session = Session::start(real_settings(), Some(path));
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

/// What a shell connected to the session has heard so far: every batch the tree published.
type Heard = Arc<Mutex<Vec<TreeBatch>>>;

/// The batches published so far, once `predicate` holds for them.
fn wait_for_batches(heard: &Heard, predicate: impl Fn(&[TreeBatch]) -> bool) -> Vec<TreeBatch> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let batches = heard.lock().expect("the batch lock").clone();
        if predicate(&batches) {
            return batches;
        }
        assert!(
            Instant::now() < deadline,
            "the shell never heard what the test waits for; heard: {batches:#?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// The nodes a shell has been told are ready, in the order it heard them.
fn ready_nodes(batches: &[TreeBatch]) -> Vec<gnoblin_types::NodeId> {
    batches
        .iter()
        .flat_map(|batch| batch.events.iter())
        .filter(|FrameTreeEvent { event, .. }| matches!(event, TreeFrameInput::Ready))
        .map(|event| event.node)
        .collect()
}

/// The changes a shell has been told about, in the order it heard them.
fn changes_heard(batches: &[TreeBatch]) -> Vec<TreeChange> {
    batches.iter().flat_map(|batch| batch.changes.iter().cloned()).collect()
}

/// The version of the `Xwayland` the path finds, as `(major, minor)`.
fn xwayland_version(path: &std::ffi::OsStr) -> Option<(u32, u32)> {
    let output = std::process::Command::new("Xwayland")
        .arg("-version")
        .env("PATH", path)
        .output()
        .ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let version = text.split("Version ").nth(1)?.split_whitespace().next()?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// An X11 window is the same window node to the shell that a Wayland toplevel is: created as
/// a `window` with its title and application id, announced ready once when it has content,
/// given a size, and destroyed when the window goes.
///
/// A server older than 23.1 does not bind `xwayland-shell-v1`: it names a window's surface by
/// a Wayland object id that Smithay does not let a compositor pair with the window, so no
/// content ever reaches a node. The test is skipped, with a printed reason, on such a server.
#[test]
fn an_x11_window_node_reaches_the_shell_like_a_wayland_window() {
    let Some(path) = display_path() else {
        return;
    };
    if xwayland_version(&path).is_none_or(|version| version < (23, 1)) {
        println!(
            "skipping: the Xwayland on PATH is older than 23.1 or unreadable ({:?}), so it does not pair surfaces through xwayland-shell-v1",
            xwayland_version(&path)
        );
        return;
    }
    let heard: Heard = Arc::default();
    let tree = TreeHooks {
        publish: {
            let heard = Arc::clone(&heard);
            Arc::new(move |batch| heard.lock().expect("the batch lock").push(batch))
        },
        ..TreeHooks::default()
    };
    let session = Session::start_with_tree(real_settings(), Some(path), tree);
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        let window = client.create_window("gnoblin node window", "GnoblinNode", false);
        client.map(window);
        // A window the manager has taken in is one the client can draw into.
        handle.wait_for_title("gnoblin node window");
        client.paint(window);

        let batches = wait_for_batches(&heard, |batches| !ready_nodes(batches).is_empty());
        let ready = ready_nodes(&batches);
        assert_eq!(ready.len(), 1, "the node is announced ready once: {batches:#?}");
        let node = ready[0];

        let changes = changes_heard(&batches);
        assert!(
            changes.iter().any(|change| matches!(
                change,
                TreeChange::Created { node: created, class, .. } if *created == node && class == "window"
            )),
            "the node is created as a window: {changes:#?}"
        );
        let text = |key: &str| {
            changes.iter().find_map(|change| match change {
                TreeChange::Set {
                    node: set,
                    key: set_key,
                    value: TreeValue::Text(value),
                } if *set == node && set_key == key => Some(value.clone()),
                _ => None,
            })
        };
        assert_eq!(text("title").as_deref(), Some("gnoblin node window"));
        assert_eq!(
            text("app_id").as_deref(),
            Some("GnoblinNode"),
            "the WM_CLASS class is the application id"
        );
        assert!(
            changes.iter().any(|change| matches!(
                change,
                TreeChange::Set { node: set, key, value: TreeValue::Double(width) }
                    if *set == node && key == "width" && *width > 0.0
            )),
            "the node is given the size of its content: {changes:#?}"
        );

        client.unmap(window);
        let batches = wait_for_batches(&heard, |batches| {
            changes_heard(batches)
                .iter()
                .any(|change| matches!(change, TreeChange::Destroyed { node: gone, .. } if *gone == node))
        });
        assert_eq!(ready_nodes(&batches).len(), 1, "an unmap announces no second window");
    });
}

/// What an X11 window manager's client sends the root window: an EWMH or ICCCM message about `window`.
impl XClient {
    fn atom(&self, name: &str) -> u32 {
        self.connection
            .intern_atom(false, name.as_bytes())
            .expect("an intern request")
            .reply()
            .expect("an atom")
            .atom
    }

    /// Sends the root window a 32-bit client message of type `kind` about `window`, with the mask the EWMH names so
    /// that the window manager receives it.
    fn root_message(&self, window: u32, kind: &str, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, window, self.atom(kind), data);
        self.connection
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .expect("a send_event request");
        self.connection.flush().expect("the message reaches the server");
    }

    /// `_NET_WM_STATE`: `action` is 0 to remove, 1 to add and 2 to toggle `first` and `second`.
    fn net_wm_state(&self, window: u32, action: u32, first: &str, second: Option<&str>) {
        let first = self.atom(first);
        let second = second.map_or(0, |name| self.atom(name));
        self.root_message(window, "_NET_WM_STATE", [action, first, second, 1, 0]);
    }

    /// `WM_CHANGE_STATE`: 3 asks for `IconicState` and 1 for `NormalState`.
    fn wm_change_state(&self, window: u32, state: u32) {
        self.root_message(window, "WM_CHANGE_STATE", [state, 0, 0, 0, 0]);
    }

    /// The names of the atoms in the window's `_NET_WM_STATE`.
    fn state_names(&self, window: u32) -> Vec<String> {
        let property = self
            .connection
            .get_property(false, window, self.atom("_NET_WM_STATE"), AtomEnum::ATOM, 0, 64)
            .expect("a property request")
            .reply()
            .expect("a property");
        property
            .value32()
            .map(|atoms| {
                atoms
                    .map(|atom| {
                        let name = self
                            .connection
                            .get_atom_name(atom)
                            .expect("a name request")
                            .reply()
                            .expect("a name")
                            .name;
                        String::from_utf8_lossy(&name).into_owned()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The size the server gives the window.
    fn size(&self, window: u32) -> (u16, u16) {
        let geometry = self
            .connection
            .get_geometry(window)
            .expect("a geometry request")
            .reply()
            .expect("a geometry");
        (geometry.width, geometry.height)
    }

    /// Waits until `check` holds for what the client can read of its window, or fails with `what`.
    #[track_caller]
    fn wait_until(&self, what: &str, check: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !check(self) {
            assert!(Instant::now() < deadline, "the window never reached {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits until the window's `_NET_WM_STATE` holds (or no longer holds) `name`.
    #[track_caller]
    fn wait_for_state(&self, window: u32, name: &str, present: bool) {
        self.wait_until(
            &format!("`{name}` {}", if present { "set" } else { "cleared" }),
            |client| client.state_names(window).iter().any(|held| held == name) == present,
        );
    }

    /// Waits until the window is this size.
    #[track_caller]
    fn wait_for_size(&self, window: u32, size: (u16, u16)) {
        self.wait_until(&format!("{}x{}", size.0, size.1), |client| client.size(window) == size);
    }

    /// Declares that the window handles `WM_DELETE_WINDOW`, so a close request reaches it as a message rather than a
    /// destroy.
    fn accept_delete(&self, window: u32) {
        let protocols = self.atom("WM_PROTOCOLS");
        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                protocols,
                AtomEnum::ATOM,
                &[self.atom("WM_DELETE_WINDOW")],
            )
            .expect("a protocols property");
        self.connection.flush().expect("the requests reach the server");
    }

    /// Waits for the `WM_DELETE_WINDOW` message a window manager sends to close a window.
    #[track_caller]
    fn wait_for_delete(&self, window: u32) {
        let (protocols, delete) = (self.atom("WM_PROTOCOLS"), self.atom("WM_DELETE_WINDOW"));
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            while let Some(event) = self.connection.poll_for_event().expect("an event") {
                if let Event::ClientMessage(message) = event
                    && message.window == window
                    && message.type_ == protocols
                    && message.data.as_data32()[0] == delete
                {
                    return;
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("the window never received WM_DELETE_WINDOW");
    }

    /// The window's `_NET_FRAME_EXTENTS`, once it has one.
    fn frame_extents(&self, window: u32) -> Option<Vec<u32>> {
        let property = self
            .connection
            .get_property(false, window, self.atom("_NET_FRAME_EXTENTS"), AtomEnum::CARDINAL, 0, 4)
            .expect("a property request")
            .reply()
            .expect("a property");
        property
            .value32()
            .map(Iterator::collect)
            .filter(|values: &Vec<u32>| !values.is_empty())
    }

    /// The atoms the window manager lists in the root window's `_NET_SUPPORTED`.
    fn supported(&self) -> Vec<u32> {
        self.connection
            .get_property(false, self.root, self.atom("_NET_SUPPORTED"), AtomEnum::ATOM, 0, 256)
            .expect("a property request")
            .reply()
            .expect("a property")
            .value32()
            .map(Iterator::collect)
            .unwrap_or_default()
    }

    /// Where the window's top left corner is on the root window.
    fn position(&self, window: u32) -> (i16, i16) {
        let reply = self
            .connection
            .translate_coordinates(window, self.root, 0, 0)
            .expect("a translate request")
            .reply()
            .expect("coordinates");
        (reply.dst_x, reply.dst_y)
    }

    /// Whether the window holds `name` in its `_NET_WM_STATE` right now.
    fn has_state(&self, window: u32, name: &str) -> bool {
        self.state_names(window).iter().any(|held| held == name)
    }
}

const MAXIMISED_VERT: &str = "_NET_WM_STATE_MAXIMIZED_VERT";
const MAXIMISED_HORZ: &str = "_NET_WM_STATE_MAXIMIZED_HORZ";
const FULLSCREEN: &str = "_NET_WM_STATE_FULLSCREEN";
const HIDDEN: &str = "_NET_WM_STATE_HIDDEN";

/// The Xwayland on `PATH`, or a skip: the requests below need a server that pairs surfaces through
/// `xwayland-shell-v1` (23.1 and later), because a window with no content has no node to answer on.
fn xwayland_with_surfaces() -> Option<std::ffi::OsString> {
    let path = display_path()?;
    if xwayland_version(&path).is_none_or(|version| version < (23, 1)) {
        println!(
            "skipping: the Xwayland on PATH is older than 23.1 or unreadable ({:?}), so it does not pair surfaces through xwayland-shell-v1",
            xwayland_version(&path)
        );
        return None;
    }
    Some(path)
}

/// An X11 client's own maximise, fullscreen, iconify and activate messages are requests the shell hears on the
/// window's node, exactly as an `xdg_toplevel`'s are: nothing changes until the shell answers by writing the node, and
/// the answer is the window's `_NET_WM_STATE` and its size.
#[test]
fn an_x11_clients_own_state_requests_are_answered_by_the_shell() {
    let Some(path) = xwayland_with_surfaces() else {
        return;
    };
    let (shell, hooks) = Shell::new();
    let session = Session::start_with_tree(real_settings(), Some(path), hooks);
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        shell.attach();
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        let window = client.create_window("shell asked", "ShellAsked", false);
        client.accept_delete(window);
        client.map(window);
        handle.wait_for_title("shell asked");
        client.paint(window);
        let node = shell.wait_window_node();
        let steady = client.size(window);

        // Maximise: a request, and no change until the shell answers.
        client.net_wm_state(window, 1, MAXIMISED_VERT, Some(MAXIMISED_HORZ));
        shell.wait_request(node, "maximize", 1);
        thread::sleep(Duration::from_millis(200));
        assert!(
            !client.has_state(window, MAXIMISED_VERT) && !client.has_state(window, MAXIMISED_HORZ),
            "nothing maximises the window until the shell answers: {:?}",
            client.state_names(window)
        );
        assert_eq!(client.size(window), steady, "the size is the shell's to change");
        shell.write(
            node,
            &[
                ("maximised", TreeValue::Bool(true)),
                ("width", TreeValue::Double(640.0)),
                ("height", TreeValue::Double(480.0)),
            ],
        );
        client.wait_for_state(window, MAXIMISED_VERT, true);
        client.wait_for_state(window, MAXIMISED_HORZ, true);
        client.wait_for_size(window, (640, 480));

        client.net_wm_state(window, 0, MAXIMISED_VERT, Some(MAXIMISED_HORZ));
        shell.wait_request(node, "unmaximize", 1);
        shell.write(
            node,
            &[
                ("maximised", TreeValue::Bool(false)),
                ("width", TreeValue::Double(300.0)),
                ("height", TreeValue::Double(200.0)),
            ],
        );
        client.wait_for_state(window, MAXIMISED_VERT, false);
        client.wait_for_state(window, MAXIMISED_HORZ, false);
        client.wait_for_size(window, (300, 200));

        // Fullscreen.
        client.net_wm_state(window, 1, FULLSCREEN, None);
        shell.wait_request(node, "fullscreen", 1);
        thread::sleep(Duration::from_millis(200));
        assert!(!client.has_state(window, FULLSCREEN), "the shell has not answered yet");
        shell.write(
            node,
            &[
                ("fullscreen", TreeValue::Bool(true)),
                ("width", TreeValue::Double(1024.0)),
                ("height", TreeValue::Double(768.0)),
            ],
        );
        client.wait_for_state(window, FULLSCREEN, true);
        client.wait_for_size(window, (1024, 768));
        client.net_wm_state(window, 0, FULLSCREEN, None);
        shell.wait_request(node, "unfullscreen", 1);
        shell.write(
            node,
            &[
                ("fullscreen", TreeValue::Bool(false)),
                ("width", TreeValue::Double(300.0)),
                ("height", TreeValue::Double(200.0)),
            ],
        );
        client.wait_for_state(window, FULLSCREEN, false);
        client.wait_for_size(window, (300, 200));

        // Iconify.
        client.wm_change_state(window, 3);
        shell.wait_request(node, "minimize", 1);
        thread::sleep(Duration::from_millis(200));
        assert!(!client.has_state(window, HIDDEN), "the shell has not answered yet");
        shell.write(node, &[("minimised", TreeValue::Bool(true))]);
        client.wait_for_state(window, HIDDEN, true);
        client.wm_change_state(window, 1);
        shell.wait_request(node, "unminimize", 1);
        shell.write(node, &[("minimised", TreeValue::Bool(false))]);
        client.wait_for_state(window, HIDDEN, false);

        // Activate: another request, which the shell hears again on top of the one the map made.
        let activations = shell.requests(node).iter().filter(|name| **name == "activate").count();
        client.root_message(window, "_NET_ACTIVE_WINDOW", [1, 0, 0, 0, 0]);
        shell.wait_request(node, "activate", activations + 1);

        // Close: the shell hears a request, and the client is asked nothing until the shell calls `close`.
        client.root_message(window, "_NET_CLOSE_WINDOW", [1, 0, 0, 0, 0]);
        shell.wait_request(node, "close", 1);
        shell.call(node, "close");
        client.wait_for_delete(window);
    });
}

/// With no shell holding the tree the compositor's record decides, and applies what the client asked for: its
/// `_NET_WM_STATE` and its size follow the request, and the client can leave the state again.
#[test]
fn without_a_shell_an_x11_clients_own_state_requests_are_applied_by_the_fallback() {
    let Some(path) = xwayland_with_surfaces() else {
        return;
    };
    let session = Session::start(real_settings(), Some(path));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        let window = client.create_window("fallback asked", "FallbackAsked", false);
        client.accept_delete(window);
        client.map(window);
        handle.wait_for_title("fallback asked");
        client.paint(window);
        let floating = client.size(window);

        client.net_wm_state(window, 1, MAXIMISED_VERT, Some(MAXIMISED_HORZ));
        client.wait_for_state(window, MAXIMISED_VERT, true);
        client.wait_for_state(window, MAXIMISED_HORZ, true);
        client.wait_until("a size larger than the floating one", |client| {
            let (width, height) = client.size(window);
            width > floating.0 && height > floating.1
        });
        client.net_wm_state(window, 0, MAXIMISED_VERT, Some(MAXIMISED_HORZ));
        client.wait_for_state(window, MAXIMISED_VERT, false);
        client.wait_for_state(window, MAXIMISED_HORZ, false);
        client.wait_for_size(window, floating);

        client.net_wm_state(window, 1, FULLSCREEN, None);
        client.wait_for_state(window, FULLSCREEN, true);
        client.wait_until("a size larger than the floating one", |client| {
            let (width, height) = client.size(window);
            width > floating.0 && height > floating.1
        });
        client.net_wm_state(window, 0, FULLSCREEN, None);
        client.wait_for_state(window, FULLSCREEN, false);
        client.wait_for_size(window, floating);

        client.wm_change_state(window, 3);
        client.wait_for_state(window, HIDDEN, true);
        client.wm_change_state(window, 1);
        client.wait_for_state(window, HIDDEN, false);

        // `_NET_CLOSE_WINDOW` is the window's own `WM_DELETE_WINDOW`.
        client.root_message(window, "_NET_CLOSE_WINDOW", [1, 0, 0, 0, 0]);
        client.wait_for_delete(window);
    });
}

/// The window manager says it understands the root-window messages Smithay does not decode, a window has no frame,
/// and `_NET_MOVERESIZE_WINDOW` places a window as the record places any window.
#[test]
fn an_x11_client_can_move_and_resize_itself_and_ask_for_its_frame() {
    let Some(path) = xwayland_with_surfaces() else {
        return;
    };
    let session = Session::start(real_settings(), Some(path));
    let xwayland = session.handle.xwayland.clone();
    session.run(move |handle| {
        let display = wait_ready(&xwayland);
        let client = XClient::connect(display);
        client.wait_until("the three messages in _NET_SUPPORTED", |client| {
            let supported = client.supported();
            [
                "_NET_CLOSE_WINDOW",
                "_NET_MOVERESIZE_WINDOW",
                "_NET_REQUEST_FRAME_EXTENTS",
            ]
            .iter()
            .all(|name| supported.contains(&client.atom(name)))
        });
        let window = client.create_window("moved", "Moved", false);
        client.root_message(window, "_NET_REQUEST_FRAME_EXTENTS", [0; 5]);
        client.wait_until("the answer to _NET_REQUEST_FRAME_EXTENTS", |client| {
            client.frame_extents(window) == Some(vec![0, 0, 0, 0])
        });
        client.map(window);
        handle.wait_for_title("moved");
        client.paint(window);

        // Flags: x, y, width and height are all valid; the gravity is the default.
        let flags = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11);
        client.root_message(window, "_NET_MOVERESIZE_WINDOW", [flags, 120, 90, 400, 300]);
        client.wait_for_size(window, (400, 300));
        client.wait_until("the position 120,90", |client| client.position(window) == (120, 90));

        // Only the size is flagged, so the position is left where it is.
        client.root_message(window, "_NET_MOVERESIZE_WINDOW", [1 << 10 | 1 << 11, 0, 0, 500, 350]);
        client.wait_for_size(window, (500, 350));
        assert_eq!(client.position(window), (120, 90), "an unflagged field is not asked");
    });
}
