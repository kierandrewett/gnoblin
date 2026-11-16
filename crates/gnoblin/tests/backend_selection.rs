//! The shipped binary picks its backend: `compositor.backend`, `GNOBLIN_BACKEND` and the host decide, the choice is
//! published on the `session` object and a backend that cannot start is refused with the reason.
//!
//! Every test runs `gnoblin --role compositor` itself and asks it through the real `gnoblinctl` entry point, so
//! what is proven is the process a login session starts and not a function that happens to share its names. The
//! nested cases are hosted by another run of the same binary on the headless backend, which is a real Wayland
//! compositor for the purpose.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use gnoblin_config::{Config, Store, Value};
use gnoblinctl::SocketConnection;
use tempfile::TempDir;

/// The binary under test, built by cargo for this test.
const BIN: &str = env!("CARGO_BIN_EXE_gnoblin");

/// How long a test waits for a session to answer before failing.
const TIMEOUT: Duration = Duration::from_secs(60);

/// One compositor process of the shipped binary, with a private runtime directory, killed on drop.
struct Compositor {
    directory: TempDir,
    child: Option<Child>,
    log: PathBuf,
}

impl Compositor {
    /// Starts the compositor with `environment` added to a clean one: nothing of the machine's own display, seat or
    /// session leaks in, so the result is the same on a desktop and in a container.
    fn start(environment: &[(&str, &OsString)]) -> Compositor {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let log = directory.path().join("compositor.log");
        let file = std::fs::File::create(&log).expect("the log file");
        let mut command = Command::new(BIN);
        command
            .args(["--role", "compositor"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("XDG_STATE_HOME", directory.path().join("state"))
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("HOME", directory.path())
            .env("GNOBLIN_IPC_SOCKET", directory.path().join("ipc.sock"))
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().expect("a duplicated log")))
            .stderr(Stdio::from(file));
        for (name, value) in environment {
            command.env(name, value);
        }
        let child = command.spawn().expect("the compositor starts");
        Compositor {
            directory,
            child: Some(child),
            log,
        }
    }

    fn socket(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    /// The Wayland socket this compositor serves, which a nested session names as its host.
    fn display(&self) -> OsString {
        self.directory.path().join("gnoblin-wayland.sock").into_os_string()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Runs one `gnoblinctl` command; `None` while the session does not answer.
    fn ctl(&self, arguments: &[&str]) -> Option<String> {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = gnoblinctl::run(
            arguments.iter().map(OsString::from),
            SocketConnection::at(self.socket()),
            &mut out,
            &mut err,
        );
        (code == 0).then(|| String::from_utf8_lossy(&out).trim().to_owned())
    }

    /// Waits until the session answers, and fails with the log if it exits first.
    #[track_caller]
    fn ready(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if self.ctl(&["ping"]).is_some() {
                return;
            }
            if let Some(status) = self.exited() {
                panic!(
                    "the compositor exited with {status} before it answered:\n{}",
                    self.log()
                );
            }
            assert!(
                Instant::now() < deadline,
                "the compositor never answered:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn exited(&mut self) -> Option<ExitStatus> {
        self.child.as_mut()?.try_wait().expect("the child can be polled")
    }

    /// Waits for the process to end on its own and returns how.
    #[track_caller]
    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.exited() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the compositor kept running:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The `backend` property of the session object, read through `gnoblinctl`.
    #[track_caller]
    fn backend(&self) -> String {
        let listed = self.ctl(&["session", "list"]).expect("the session object is served");
        let id = listed
            .split_whitespace()
            .next()
            .unwrap_or_else(|| panic!("no session object in {listed:?}"))
            .to_owned();
        // `get` prints a string property as JSON text.
        self.ctl(&["session", "get", &id, "backend"])
            .unwrap_or_else(|| panic!("the backend is readable:\n{}", self.log()))
            .trim_matches('"')
            .to_owned()
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn os(text: &str) -> OsString {
    OsString::from(text)
}

#[test]
fn headless_is_what_a_machine_with_nothing_to_draw_on_gets() {
    // No override, no host display, no seat: `auto` falls to headless, and says so.
    let mut session = Compositor::start(&[]);
    session.ready();
    assert_eq!(session.backend(), "headless");
    assert!(
        session.log().contains("headless"),
        "the startup log names the backend:\n{}",
        session.log()
    );
}

#[test]
fn the_environment_names_the_backend() {
    let backend = os("headless");
    let mut session = Compositor::start(&[("GNOBLIN_BACKEND", &backend)]);
    session.ready();
    assert_eq!(session.backend(), "headless");
    assert!(
        session.log().contains("asked for by GNOBLIN_BACKEND"),
        "the log says why:\n{}",
        session.log()
    );
}

#[test]
fn a_value_no_backend_has_ends_the_start_naming_the_accepted_values() {
    let backend = os("x11");
    let mut session = Compositor::start(&[("GNOBLIN_BACKEND", &backend)]);
    let status = session.wait();
    assert_eq!(status.code(), Some(1), "{}", session.log());
    let log = session.log();
    assert!(log.contains("GNOBLIN_BACKEND") && log.contains("x11"), "{log}");
    for accepted in ["auto", "headless", "nested", "drm"] {
        assert!(log.contains(accepted), "the refusal lists {accepted}:\n{log}");
    }
}

#[test]
fn a_nested_session_is_a_window_of_its_host() {
    // The host is the same binary on the headless backend: a real compositor that maps the toplevel.
    let mut host = Compositor::start(&[]);
    host.ready();
    let (backend, display) = (os("nested"), host.display());
    let mut nested = Compositor::start(&[("GNOBLIN_BACKEND", &backend), ("WAYLAND_DISPLAY", &display)]);
    nested.ready();
    assert_eq!(nested.backend(), "nested");

    let deadline = Instant::now() + TIMEOUT;
    let window = loop {
        if let Some(listed) = host.ctl(&["window", "list"])
            && let Some(id) = listed.split_whitespace().next()
        {
            break id.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "the host never mapped the nested window:\n{}",
            host.log()
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        host.ctl(&["window", "get", &window, "app_id"])
            .as_deref()
            .map(|text| text.trim_matches('"')),
        Some("org.gnoblin.Session"),
        "the host holds the nested session as a window"
    );
}

#[test]
fn auto_nests_where_a_host_compositor_is_serving() {
    let mut host = Compositor::start(&[]);
    host.ready();
    let display = host.display();
    let mut nested = Compositor::start(&[("WAYLAND_DISPLAY", &display)]);
    nested.ready();
    assert_eq!(nested.backend(), "nested", "{}", nested.log());
    assert!(
        nested
            .log()
            .contains("a host Wayland compositor is serving the display"),
        "the log says why:\n{}",
        nested.log()
    );
}

#[test]
fn a_nested_session_with_no_host_is_refused_naming_both_places_a_host_comes_from() {
    let backend = os("nested");
    let mut session = Compositor::start(&[("GNOBLIN_BACKEND", &backend)]);
    assert_eq!(session.wait().code(), Some(1));
    let log = session.log();
    assert!(
        log.contains("compositor.nested-display") && log.contains("WAYLAND_DISPLAY"),
        "{log}"
    );
}

#[test]
fn the_document_names_the_backend_and_the_environment_beats_it() {
    // A last accepted document that says `nested` against a host that does not exist is refused naming the socket,
    // and the same start with the environment saying `headless` runs: the document is the user's, the variable the
    // wrapper's.
    let directory = tempfile::tempdir().expect("a temporary directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
    let source = directory.path().join("shell.ts");
    // `GNOBLIN_SHELL` names a shell that has to exist: a path that is not there is an error, not a first login.
    std::fs::write(&source, "// the user's shell\n").expect("the shell file");
    let missing = directory.path().join("no-such-host.sock");
    let document = Value::from_json(&serde_json::json!({
        "compositor": {"backend": "nested", "nested-display": missing.to_str().expect("utf-8 path")}
    }))
    .expect("a document");
    let config = Config::from_value(document).expect("a valid document");
    let state = directory.path().join("state");
    std::fs::create_dir(&state).expect("the state directory");
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).expect("the state mode");
    Store::new(&state)
        .save(&source, &config)
        .expect("the last accepted document is stored");

    let (config_path, state_home) = (source.clone().into_os_string(), state.clone().into_os_string());
    let mut refused = Compositor::start(&[("GNOBLIN_SHELL", &config_path), ("XDG_STATE_HOME", &state_home)]);
    assert_eq!(refused.wait().code(), Some(1), "{}", refused.log());
    assert!(
        refused.log().contains("no-such-host.sock"),
        "the refusal names the socket the document asked for:\n{}",
        refused.log()
    );

    let backend = os("headless");
    let mut session = Compositor::start(&[
        ("GNOBLIN_SHELL", &config_path),
        ("XDG_STATE_HOME", &state_home),
        ("GNOBLIN_BACKEND", &backend),
    ]);
    session.ready();
    assert_eq!(session.backend(), "headless");
}

#[test]
fn the_hardware_backend_with_no_seat_is_refused_with_the_step_and_the_way_out() {
    // seatd is asked for explicitly and pointed at a socket nobody serves, so the answer is the same on a machine
    // with a real seat as on one with none, and the test never takes a display.
    let (backend, libseat, socket) = (os("drm"), os("seatd"), os("/nonexistent/seatd.sock"));
    let mut session = Compositor::start(&[
        ("GNOBLIN_BACKEND", &backend),
        ("LIBSEAT_BACKEND", &libseat),
        ("SEATD_SOCK", &socket),
    ]);
    assert_eq!(session.wait().code(), Some(1), "{}", session.log());
    let log = session.log();
    assert!(log.contains("the drm backend cannot start"), "{log}");
    assert!(log.contains("no seat session"), "the step that failed is named:\n{log}");
    assert!(
        log.contains("display manager") && log.contains("compositor.backend"),
        "the way out is named:\n{log}"
    );
    assert!(!session.socket().exists(), "nothing is served by a refused start");
}

#[test]
fn the_hardware_backend_refuses_a_renderer_it_cannot_scan_out_with() {
    // The renderer is a document setting, so the document carries it; the refusal comes before the seat is touched.
    let directory = tempfile::tempdir().expect("a temporary directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
    let source = directory.path().join("shell.ts");
    // `GNOBLIN_SHELL` names a shell that has to exist: a path that is not there is an error, not a first login.
    std::fs::write(&source, "// the user's shell\n").expect("the shell file");
    let document = Value::from_json(&serde_json::json!({
        "compositor": {"backend": "drm", "renderer": "software"}
    }))
    .expect("a document");
    let config = Config::from_value(document).expect("a valid document");
    let state = directory.path().join("state");
    std::fs::create_dir(&state).expect("the state directory");
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).expect("the state mode");
    Store::new(&state)
        .save(&source, &config)
        .expect("the last accepted document is stored");

    let (config_path, state_home) = (source.into_os_string(), state.into_os_string());
    let mut session = Compositor::start(&[("GNOBLIN_SHELL", &config_path), ("XDG_STATE_HOME", &state_home)]);
    assert_eq!(session.wait().code(), Some(1), "{}", session.log());
    let log = session.log();
    assert!(
        log.contains("compositor.renderer = software") && log.contains("drm"),
        "the setting is named:\n{log}"
    );
}
