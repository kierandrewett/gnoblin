//! The X11 session a compositor starts from the `xwayland` settings.
//!
//! A real compositor session is started with the settings the `xwayland` table maps to,
//! and a real `x11rb` client connects to the X display the session's `Xwayland` took. The
//! test does not reach into the session: it finds the display by its socket under
//! `/tmp/.X11-unix`, which is where the X server listens whatever number it was given.
//!
//! The test is skipped **with a printed reason** only when the machine has no `Xwayland`
//! binary on `PATH`, because a suite that passes silently on a host without the feature
//! checks nothing.
//!
//! The commit tests look for the `Xwayland` binary in the directory the session is given
//! (`SessionConfig::xwayland_path`), so they never change the process's environment.

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_client::Client as RuntimeClient;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::SessionConfig;
use support::{Session, TIMEOUT, wait_for};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// The sessions these tests start share the process, and their X servers and clients are
/// timing-sensitive, so the tests run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// The X11 socket directory the X server listens in.
const X11_SOCKETS: &str = "/tmp/.X11-unix";

/// The X display numbers with a listening socket in [`X11_SOCKETS`].
fn displays() -> BTreeSet<u32> {
    let Ok(entries) = std::fs::read_dir(Path::new(X11_SOCKETS)) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.strip_prefix('X')?.parse().ok())
        .collect()
}

#[test]
fn a_session_started_with_xwayland_accepts_an_x11_client() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    if gnoblin_xwayland::probe(std::env::var_os("PATH").as_deref()).is_err() {
        eprintln!("skipped: no Xwayland binary on PATH, so no X11 session can start here");
        return;
    }

    let before = displays();
    // The default refuses byte-swapped clients, which an Xwayland without the
    // `-byteswappedclients` switch cannot do, so the session would not start there.
    // The test allows them to exercise the start on any Xwayland.
    let (session, _roles) = Session::start_privileged_configured(&[], |config: &mut SessionConfig| {
        config.xwayland = Some(gnoblin_xwayland::Settings {
            allow_byte_swapped_clients: true,
            ..gnoblin_xwayland::Settings::default()
        });
    });

    let deadline = Instant::now() + TIMEOUT;
    let display = loop {
        if let Some(display) = displays().difference(&before).next().copied() {
            break display;
        }
        assert!(Instant::now() < deadline, "no X11 socket appeared under {X11_SOCKETS}");
        thread::sleep(Duration::from_millis(50));
    };

    let name = format!(":{display}");
    let (connection, screen) = loop {
        match RustConnection::connect(Some(&name)) {
            Ok(pair) => break pair,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "the X11 client could not connect to {name}: {error}"
                );
                thread::sleep(Duration::from_millis(50));
            }
        }
    };
    let root = connection.setup().roots[screen].root;
    let geometry = connection
        .get_geometry(root)
        .expect("a request on the connection")
        .reply()
        .expect("the root window's geometry");
    assert!(geometry.width > 0 && geometry.height > 0, "the root window has no size");

    drop(connection);
    drop(session);
}
/// The absolute path of a binary `name`, found on the test process's `PATH`.
fn binary_on_path(name: &str) -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or_else(|| panic!("{name} is on PATH"))
}

/// A fake `Xwayland` in its own directory. Each start appends the process id to `marker`,
/// then it execs an absolute `cat` on the Wayland socket Smithay passes as `WAYLAND_SOCKET`,
/// with its stdio detached. Like the real server, it ends when that connection closes, which
/// Smithay causes by disconnecting the client when the server is dropped. The process id is
/// the one the supervisor spawned, and no pipe the test runner reads is held open by it. The
/// supervisor runs the child with the probed directory first on its `PATH`, so the script
/// names `cat` by absolute path. The `-help` probe the supervisor runs for the switches answers
/// at once and is not counted. Dropping the fake kills every process it started.
struct FakeXwayland {
    directory: tempfile::TempDir,
    marker: PathBuf,
}

impl FakeXwayland {
    fn new() -> FakeXwayland {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let marker = directory.path().join("started");
        let binary = directory.path().join("Xwayland");
        let script = format!(
            "#!/bin/bash\ncase \"$1\" in\n-help) echo '-byteswappedclients'; exit 0;;\nesac\necho $$ >> '{}'\nexec 0<&\"$WAYLAND_SOCKET\"\nexec '{}' >/dev/null 2>&1\n",
            marker.display(),
            binary_on_path("cat").display()
        );
        std::fs::write(&binary, script).expect("the fake Xwayland");
        std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755)).expect("executable");
        FakeXwayland { directory, marker }
    }

    /// The directory holding the fake binary: the session's `xwayland_path`.
    fn directory(&self) -> &Path {
        self.directory.path()
    }

    /// The process ids the fake has been started as, one per start.
    fn started(&self) -> Vec<u32> {
        std::fs::read_to_string(&self.marker)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }
}

impl Drop for FakeXwayland {
    fn drop(&mut self) {
        for pid in self.started() {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// A session with the runtime role granted, whose `Xwayland` is looked for in `path`, and
/// the runtime's connection to it. The session starts with no X11, as a session with no
/// accepted document does.
fn start_runtime(path: &Path) -> (Session, RuntimeClient<UnixTransport>) {
    let (session, mut ends) = Session::start_privileged_configured(&[("runtime", Vec::new())], |config| {
        config.xwayland_path = Some(path.as_os_str().to_owned());
    });
    let (_role, stream) = ends.pop().expect("the runtime's end");
    let runtime = RuntimeClient::connect(
        UnixTransport::from_stream(stream).expect("a transport"),
        gnoblin_compositor::version(),
    )
    .expect("the runtime handshakes");
    (session, runtime)
}

/// Whether process `pid` is alive and not a zombie.
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        // The state is the field after the command name, which is in parentheses.
        stat.rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z" && state != "X")
    })
}

#[test]
fn a_commit_that_enables_xwayland_starts_it_after_the_commit() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let fake = FakeXwayland::new();
    let (_session, mut runtime) = start_runtime(fake.directory());
    assert!(
        fake.started().is_empty(),
        "no accepted document enables X11, so nothing has started"
    );

    assert_eq!(
        runtime.commit_config(br#"{"xwayland":{}}"#).expect("the commit"),
        1,
        "the document is the first revision"
    );
    wait_for("the Xwayland the commit enabled to start", || {
        (!fake.started().is_empty()).then_some(())
    });
}

#[test]
fn a_commit_with_no_xwayland_binary_never_starts_x11() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    // The fake exists, but its directory is not the session's `xwayland_path`: X11 is
    // disabled on this host.
    let fake = FakeXwayland::new();
    let empty = tempfile::tempdir().expect("a temporary directory");
    let (_session, mut runtime) = start_runtime(empty.path());

    assert_eq!(
        runtime
            .commit_config(br#"{"xwayland":{"allow-grabs":true}}"#)
            .expect("the commit"),
        1,
        "the document is the first revision"
    );
    // A later commit must not start it either: the session stays up without X11.
    assert_eq!(runtime.commit_config(b"{}").expect("the second commit"), 2);
    thread::sleep(Duration::from_millis(300));
    assert!(
        fake.started().is_empty(),
        "no binary is on the search path, so nothing starts"
    );
}

#[test]
fn a_commit_that_changes_xwayland_restarts_it_when_no_x11_client_is_connected() {
    let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let fake = FakeXwayland::new();
    let (_session, mut runtime) = start_runtime(fake.directory());
    runtime
        .commit_config(br#"{"xwayland":{}}"#)
        .expect("the enabling commit");
    let first = wait_for("the Xwayland to start", || fake.started().first().copied());
    assert!(alive(first), "the started Xwayland is running");

    // A commit that leaves the X11 settings as they were does not restart the server.
    runtime
        .commit_config(br#"{"xwayland":{}}"#)
        .expect("the unchanged commit");
    thread::sleep(Duration::from_millis(300));
    assert_eq!(fake.started().len(), 1, "unchanged settings keep the running server");

    runtime
        .commit_config(br#"{"xwayland":{"allow-grabs":true}}"#)
        .expect("the changing commit");
    let second = wait_for("the Xwayland to restart with the new settings", || {
        fake.started().get(1).copied()
    });
    assert_ne!(first, second, "the restart is a new process");
    wait_for("the old Xwayland to stop", || (!alive(first)).then_some(()));
    assert!(alive(second), "the restarted Xwayland is running");
}
