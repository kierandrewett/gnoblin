//! The guardian role, as processes.
//!
//! Design/rust-compositor.md, "Process model": one executable, five roles, and a guardian
//! that starts and supervises the others. These tests start the shipped binary and nothing
//! else — no mocks, no fixtures — because the property under test is what happens between
//! processes: a child that dies is restarted, a compositor that dies takes the session with
//! it, a descriptor reaches exactly one role, and a signal stops everything in order.
//!
//! What the assertions read is the guardian's log, which is where it says what it did, and
//! `gnoblinctl`'s own library entry point, which is how a script asks the session what it
//! has. Both are the shipped interfaces: a line this file looks for is a line an operator
//! sees.
//!
//! # Why every test sets `GNOBLIN_TEST_FAST`
//!
//! The guardian's shipped numbers are the C supervisor's: a 250 ms first restart, a ladder
//! up to four seconds and a two-second stop grace. A test that waited those out would take
//! twenty seconds to see the circuit breaker trip, so the hook shrinks the tick, the grace
//! and the ladders. It is documented as test-only where it is read
//! (`gnoblin_session::supervisor`), and the shipped path is what the rest of the numbers
//! are.

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use gnoblinctl::SocketConnection;
use rustix::process::{Pid, Signal};
use tempfile::TempDir;

/// The binary under test, built by cargo for this test.
const BIN: &str = env!("CARGO_BIN_EXE_gnoblin");

/// How long a test waits for a line before failing.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A running session: the guardian process, its log, and the directory it owns.
struct Session {
    /// Kept alive for the session's lifetime: dropping it would remove the sockets' directory
    /// while the children still have it open.
    directory: TempDir,
    guardian: Option<Child>,
    log: PathBuf,
}

impl Session {
    /// Start a session with the fast numbers and a directory of its own.
    fn start() -> Session {
        Session::start_with(&[])
    }

    /// Start a session with extra environment for the guardian and its children.
    fn start_with(extra: &[(&str, &str)]) -> Session {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let log = directory.path().join("session.log");
        let file = std::fs::File::create(&log).expect("the log file");
        let mut command = Command::new(BIN);
        command
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("XDG_STATE_HOME", directory.path().join("state"))
            .env("GNOBLIN_IPC_SOCKET", directory.path().join("ipc.sock"))
            .env("GNOBLIN_TEST_FAST", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().expect("a duplicated log")))
            .stderr(Stdio::from(file));
        for (key, value) in extra {
            command.env(key, value);
        }
        let guardian = command.spawn().expect("the guardian starts");
        Session {
            directory,
            guardian: Some(guardian),
            log,
        }
    }

    /// Everything the guardian and its children have written so far.
    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Wait until the log contains `text`, and return the whole log.
    #[track_caller]
    fn wait_for(&self, text: &str) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let log = self.log();
            if log.contains(text) {
                return log;
            }
            assert!(
                Instant::now() < deadline,
                "the session never logged `{text}`; it said:\n{log}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The pid of `name`'s latest generation, as the guardian logged it.
    #[track_caller]
    fn pid(&self, name: &str) -> i32 {
        let log = self.wait_for(&format!("started {name} ("));
        last_pid(&log, name).unwrap_or_else(|| panic!("no pid for {name} in:\n{log}"))
    }

    /// Every pid the guardian reported starting, in order.
    fn pids(&self) -> Vec<i32> {
        self.log().lines().filter_map(pid_of_line).collect()
    }

    /// The IPC socket a client connects to.
    fn socket(&self) -> PathBuf {
        self.directory.path().join("ipc.sock")
    }

    /// Ask the session which `session` objects it serves, through the real `gnoblinctl` entry
    /// point: a session that answers has a compositor behind it.
    #[track_caller]
    fn sessions(&self) -> String {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = gnoblinctl::run(
            ["session", "list"].map(std::ffi::OsString::from),
            SocketConnection::at(self.socket()),
            &mut out,
            &mut err,
        );
        assert_eq!(code, 0, "gnoblinctl exited {code}: {}", String::from_utf8_lossy(&err));
        let listed = String::from_utf8_lossy(&out).trim().to_owned();
        assert!(!listed.is_empty(), "the session object is served");
        listed
    }

    /// Wait for the guardian to exit and report how it did.
    #[track_caller]
    fn wait(&mut self) -> std::process::ExitStatus {
        let Some(mut guardian) = self.guardian.take() else {
            panic!("the guardian was already waited for");
        };
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = guardian.try_wait().expect("the guardian's status") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the guardian never exited; it said:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut guardian) = self.guardian.take() {
            let _ = guardian.kill();
            let _ = guardian.wait();
        }
    }
}

/// The pid on a `started <name> (generation N, pid P)` line.
fn pid_of_line(line: &str) -> Option<i32> {
    let rest = line.split("pid ").nth(1)?;
    rest.split(')').next()?.trim().parse().ok()
}

/// The latest pid a log names for `name`.
fn last_pid(log: &str, name: &str) -> Option<i32> {
    log.lines()
        .filter(|line| line.contains(&format!("started {name} (")))
        .filter_map(pid_of_line)
        .next_back()
}

/// Kill one process the way a crash does.
fn kill(pid: i32) {
    let id = Pid::from_raw(pid).unwrap_or_else(|| panic!("{pid} is not a pid"));
    rustix::process::kill_process(id, Signal::KILL).expect("the process is killed");
}

/// Whether a pid is still a live process.
fn alive(pid: i32) -> bool {
    Pid::from_raw(pid).is_some_and(|id| rustix::process::test_kill_process(id).is_ok())
}

/// Whether a process holds a socket with this inode, as `/proc` reports it.
fn socket_inodes(pid: i32) -> Vec<String> {
    let directory = format!("/proc/{pid}/fd");
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .filter(|target| target.starts_with("socket:["))
        .collect()
}

/// The descriptor number a process was given for `variable`, from its environment.
fn inherited(pid: i32, variable: &str) -> Option<i32> {
    let environment = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    let text = String::from_utf8_lossy(&environment);
    text.split('\0')
        .filter_map(|entry| entry.split_once('='))
        .find(|(key, _)| *key == variable)
        .and_then(|(_, value)| value.trim().parse().ok())
}

/// The socket a process holds at `number`.
fn socket_at(pid: i32, number: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/fd/{number}"))
        .ok()
        .map(|target| target.to_string_lossy().into_owned())
}

#[test]
fn the_guardian_starts_a_session_a_client_can_use() {
    let session = Session::start();
    session.wait_for("[guardian] session ready");
    // The session object is what every client subscribes to first, and `gnoblinctl` is the
    // real client: a session that answers it is a session whose three children are up.
    session.sessions();
    let log = session.log();
    for child in ["compositor", "services", "runtime"] {
        assert!(log.contains(&format!("started {child} (")), "no {child} in:\n{log}");
    }
    // The runtime committed the document it validated, and the guardian recorded it.
    assert!(
        log.contains("configuration revision 1 accepted"),
        "the commit is reported:\n{log}"
    );
}

#[test]
fn killing_the_services_child_restarts_it() {
    let session = Session::start();
    session.wait_for("[guardian] session ready");
    let first = session.pid("services");
    kill(first);
    let log = session.wait_for("started services (generation 2");
    // The session is still up: a services crash is not the session's.
    assert!(!log.contains("session stopping"), "the session ended:\n{log}");
    assert!(
        log.contains("restarting services in"),
        "the restart is reported:\n{log}"
    );
    session.sessions();
}

#[test]
fn a_killed_runtime_is_bounded_by_the_breaker_and_the_session_survives() {
    // The compositor keeps the last configuration a runtime committed — that is
    // `gnoblin-compositor`'s `a_committed_configuration_outlives_the_connection`.
    //
    // A restarted runtime cannot join that connection, because a socket pair has two ends and
    // the guardian's copy of the child's end goes with the child that had it. The restarted
    // runtime therefore refuses, names the variable, and the guardian's ladder and circuit
    // breaker bound the attempts instead of burning a core. Reconnecting a privileged child is
    // the one piece this slice does not carry: it needs a handshake a used pair can repeat,
    // and until then a restarted privileged role degrades the session rather than serving it.
    let session = Session::start();
    session.wait_for("configuration revision 1 accepted");
    let first = session.pid("runtime");
    kill(first);
    let log = session.wait_for("the session is degraded");
    assert!(
        log.contains("no inherited IPC connection: GNOBLIN_IPC_FD is not set"),
        "the restarted runtime says why it cannot serve:\n{log}"
    );
    assert!(log.contains("failed after 3 restarts"), "the breaker bound it:\n{log}");
    // The compositor and the services role were left alone, and the session is still usable.
    session.sessions();
}

#[test]
fn a_crash_looping_runtime_is_cut_off_and_reported() {
    // The runtime exits at once on every start, which is the crash loop the circuit breaker
    // exists for. With the fast ladder it is three restarts and then a stop.
    let session = Session::start_with(&[("GNOBLIN_TEST_RUNTIME_CRASH", "1")]);
    let log = session.wait_for("the session is degraded");
    assert!(
        log.contains("failed after 3 restarts"),
        "the breaker tripped after its budget:\n{log}"
    );
    // It stopped rather than burning a core: no start after the failure.
    let starts = log.matches("started runtime (").count();
    assert!(starts <= 4, "the runtime was started {starts} times:\n{log}");
    // And the session is still usable: the compositor and the services role were left alone.
    session.sessions();
}

#[test]
fn a_compositor_crash_ends_the_guardian_and_stops_every_child() {
    let mut session = Session::start();
    session.wait_for("[guardian] session ready");
    let pids = session.pids();
    let compositor = session.pid("compositor");
    kill(compositor);
    let status = session.wait();
    assert_ne!(status.code(), Some(0), "a lost compositor is not success: {status:?}");
    let log = session.log();
    assert!(log.contains("the session cannot continue"), "{log}");
    // Nothing outlives the session, and the guardian waited for every child it started.
    for pid in pids {
        assert!(!alive(pid), "{pid} outlived the session:\n{log}");
    }
}

#[test]
fn a_termination_signal_stops_the_session_in_reverse_order() {
    let mut session = Session::start();
    session.wait_for("[guardian] session ready");
    let pids = session.pids();
    let guardian = session.guardian.as_ref().expect("the guardian runs").id();
    rustix::process::kill_process(
        Pid::from_raw(guardian as i32).expect("the guardian's pid"),
        Signal::TERM,
    )
    .expect("the signal is delivered");
    let status = session.wait();
    assert_eq!(status.code(), Some(0), "an orderly stop is success: {status:?}");
    let log = session.log();
    assert!(log.contains("session stopping: signal"), "{log}");
    for pid in pids {
        assert!(!alive(pid), "{pid} survived the stop:\n{log}");
    }
}

#[test]
fn a_privileged_descriptor_reaches_only_its_own_role() {
    let session = Session::start();
    session.wait_for("configuration revision 1 accepted");
    let runtime = session.pid("runtime");
    let services = session.pid("services");

    // Each privileged child was given a descriptor, and the two are different connections.
    let runtime_fd = inherited(runtime, "GNOBLIN_IPC_FD").expect("the runtime was given a connection");
    let services_fd = inherited(services, "GNOBLIN_IPC_FD").expect("the services role was given a connection");
    let runtime_socket = socket_at(runtime, runtime_fd).expect("the runtime's connection is a descriptor");
    let services_socket = socket_at(services, services_fd).expect("the services connection is a descriptor");
    assert!(runtime_socket.starts_with("socket:["), "{runtime_socket}");
    assert_ne!(
        runtime_socket, services_socket,
        "the two roles were given the same connection"
    );

    // The failure this prevents: one role inherits the other's connection, so two processes
    // speak one IPC stream and both see the other's frames.
    assert!(
        !socket_inodes(services).contains(&runtime_socket),
        "the services role holds the runtime's connection: {:?}",
        socket_inodes(services)
    );
    assert!(
        !socket_inodes(runtime).contains(&services_socket),
        "the runtime holds the services connection: {:?}",
        socket_inodes(runtime)
    );
}

#[test]
fn the_guardian_refuses_to_start_without_a_runtime_directory() {
    // A guardian with nowhere to put a socket is a session manager's configuration error, and
    // it says so instead of starting children that cannot connect.
    let output = Command::new(BIN)
        .env("XDG_RUNTIME_DIR", "")
        .env("GNOBLIN_IPC_SOCKET", "")
        .env("XDG_STATE_HOME", std::env::temp_dir())
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let message = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(message.contains("GNOBLIN_IPC_SOCKET"), "{message}");
}
