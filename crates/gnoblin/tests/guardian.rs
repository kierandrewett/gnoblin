//! The guardian role, as processes.
//!
//! Design/rust-compositor.md, "Process model": one executable, four roles, and a guardian
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
//! # The shell is an isolated child
//!
//! Every session here runs its shell with `--isolate`: the shell is then a child of the guardian (`gnoblin --role shell`) with a
//! privileged connection and a report descriptor of its own, which is the supervision these tests are about. A shell in the
//! compositor's process is `tests/native_shell.rs`'s. The shell is a native example module, so a session needs no runtime
//! library and no configuration directory.
//!
//! # Why every test sets `GNOBLIN_TEST_FAST`
//!
//! The guardian's shipped numbers are the C supervisor's: a 250 ms first restart, a ladder
//! up to four seconds and a two-second stop grace. A test that waited those out would take
//! twenty seconds to see the circuit breaker trip, so the hook shrinks the tick, the grace
//! and the ladders. It is documented as test-only where it is read
//! (`gnoblin_session::supervisor`), and the shipped path is what the rest of the numbers
//! are.

use std::io::{BufRead as _, BufReader};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use gnoblinctl::SocketConnection;
use rustix::process::{Pid, Signal};
use tempfile::TempDir;

mod support;
use support::native_shell::shells;

/// The binary under test, built by cargo for this test.
const BIN: &str = env!("CARGO_BIN_EXE_gnoblin");

/// How long a test waits for a line before failing.
const TIMEOUT: Duration = Duration::from_secs(60);

/// A private message bus for one session, killed on drop.
///
/// `dbus-daemon --session --nofork --print-address` writes the address of a bus with a fresh socket. Nothing
/// reads the ambient `DBUS_SESSION_BUS_ADDRESS`, so a test can never reach the developer's session bus and two
/// tests can never share one.
struct Bus {
    child: Child,
    address: String,
}

impl Bus {
    /// Start a bus and read its address.
    fn start() -> Bus {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is required by the guardian tests (Debian/Ubuntu: apt install dbus-daemon; Fedora: dbus-daemon)");
        let stdout = child.stdout.take().expect("stdout is piped");
        let address = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .find(|line| line.starts_with("unix:"))
            .expect("dbus-daemon prints the address of its bus");
        Bus { child, address }
    }

    fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A running session: the guardian process, its log, and the directory it owns.
struct Session {
    /// Kept alive for the session's lifetime: dropping it would remove the sockets' directory
    /// while the children still have it open.
    directory: TempDir,
    guardian: Option<Child>,
    log: PathBuf,
    /// Set when the test ends, so the watchdog below stops waiting.
    finished: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The session bus the children connect to, killed after the guardian when the session drops.
    _bus: Bus,
}

impl Session {
    /// Start a session with the fast numbers and a directory of its own.
    fn start() -> Session {
        Session::start_with(&[])
    }

    /// Start a session with extra environment for the guardian and its children.
    fn start_with(extra: &[(&str, &str)]) -> Session {
        Session::start_shell(&shells().minimal, extra)
    }

    /// Start a session whose isolated shell is `shell`, with extra environment for the guardian and its children.
    fn start_shell(shell: &std::path::Path, extra: &[(&str, &str)]) -> Session {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
        let log = directory.path().join("session.log");
        let file = std::fs::File::create(&log).expect("the log file");
        let bus = Bus::start();
        let mut command = Command::new(BIN);
        command
            .arg("--isolate")
            .arg(shell)
            .env("XDG_RUNTIME_DIR", directory.path())
            // The portal child owns a name on the session bus, so the session gets a bus of its own: the
            // ambient one is another session's (or, on a build machine, there is none at all).
            .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
            .env("XDG_STATE_HOME", directory.path().join("state"))
            // Discovery reads the config and data directories, so the session must not use the machine's own.
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("HOME", directory.path().join("home"))
            .env_remove("GNOBLIN_SHELL")
            // A terminal opened from a Gnoblin session inherits that session's descriptor variable. The guardian
            // under test is no child of it, and `environment` takes this variable to mean "a child's own environment".
            .env_remove("GNOBLIN_READY_FD")
            .env("GNOBLIN_IPC_SOCKET", directory.path().join("ipc.sock"))
            .env("GNOBLIN_TEST_FAST", "1")
            // The session has to run headless however the machine that runs the test is set up: `auto` would nest in a
            // desktop that sets WAYLAND_DISPLAY, or take the seat of a login session.
            .env("GNOBLIN_BACKEND", "headless")
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().expect("a duplicated log")))
            .stderr(Stdio::from(file));
        for (key, value) in extra {
            command.env(key, value);
        }
        let guardian = command.spawn().expect("the guardian starts");
        // A real-process test that overruns must fail, not hang: after the wall-clock limit this
        // kills the guardian and everything it started, so the test's own deadlines are bounded
        // waits rather than the only thing between a regression and a stuck suite.
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let finished = std::sync::Arc::clone(&finished);
            let log = log.clone();
            let guardian = guardian.id();
            std::thread::Builder::new()
                .name("guardian-test-watchdog".to_owned())
                .spawn(move || {
                    let deadline = Instant::now() + TIMEOUT;
                    while Instant::now() < deadline {
                        if finished.load(std::sync::atomic::Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    eprintln!(
                        "the session overran {TIMEOUT:?}; killing it. It said:\n{}",
                        log_text(&log)
                    );
                    for pid in std::fs::read_to_string(&log)
                        .unwrap_or_default()
                        .lines()
                        .filter_map(pid_of_line)
                        .collect::<Vec<i32>>()
                    {
                        if let Some(id) = Pid::from_raw(pid) {
                            let _ = rustix::process::kill_process(id, Signal::KILL);
                        }
                    }
                    if let Some(id) = Pid::from_raw(guardian as i32) {
                        let _ = rustix::process::kill_process(id, Signal::KILL);
                    }
                })
                .expect("the watchdog thread");
        }
        Session {
            directory,
            guardian: Some(guardian),
            log,
            finished,
            _bus: bus,
        }
    }

    /// Everything the guardian and its children have written so far.
    fn log(&self) -> String {
        log_text(&self.log)
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

    /// Wait until the log contains `text` at least `times` times, and return the whole log.
    #[track_caller]
    fn wait_for_times(&self, text: &str, times: usize) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let log = self.log();
            if log.matches(text).count() >= times {
                return log;
            }
            assert!(
                Instant::now() < deadline,
                "the session never logged `{text}` {times} times; it said:\n{log}"
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

/// What a read of a process's `/proc` entries found: an answer, or the state of a process that is in the middle of
/// `execve`.
enum Proc<T> {
    Ready(T),
    /// Not an answer yet; carries what the read saw, for the failure message.
    Busy(String),
}

/// Read `/proc` until it answers, and fail with what the last read saw if it never does.
///
/// While a process is inside `execve`, what a read of it finds says nothing about what the process has or lacks. On a
/// child shaped like the ones the supervisor starts (`posix_spawn`, a cleared environment, its own process group,
/// piped output) each of these was seen:
///
/// - `environ` reads as empty, because the kernel records the new image's environment only after it has let go of
///   the old image;
/// - `environ` reads as the guardian's own, because a `posix_spawn` child shares the guardian's memory until its
///   `execve`;
/// - `environ` reads as only the start of an environment, when the read straddles an `execve`;
/// - a read of `environ` or of an `fd` entry fails with `EACCES`.
///
/// The guardian logs `started <name> (pid P)` when `spawn` has returned, which is when the child has begun its
/// `execve` and not when it has finished it, so a test that reads on seeing that line can land in any of them. Each
/// was taken for "the variable is not there", which is a different statement. The windows are microseconds wide: what
/// a read waits for is the state, not a duration.
#[track_caller]
fn settle<T>(what: &str, mut read: impl FnMut() -> Proc<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let seen = match read() {
            Proc::Ready(value) => return value,
            Proc::Busy(seen) => seen,
        };
        assert!(
            Instant::now() < deadline,
            "{what} was still not readable after {TIMEOUT:?}; the last read saw: {seen}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Whether a failed `/proc` read is the answer of a process that is mid-`execve`, and so worth reading again.
/// Any other failure says nothing a test may treat as "absent", and ends the test with the error.
fn mid_exec(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
}

/// Wait until `pid` is running `command`, by its command line.
///
/// The XR stand-in is a shell script that `exec`s `sleep`: one pid, two programs. As the shell it is still starting
/// (bash rewrites the environment it imports, in place, while it does) and then it replaces itself, so none of its
/// `/proc` entries can be trusted until it has become `sleep`. Once it has, nothing replaces it again. This waits for
/// that, and for nothing else about the process.
#[track_caller]
fn wait_for_command(pid: i32, command: &[&str]) {
    let path = format!("/proc/{pid}/cmdline");
    let wanted: Vec<u8> = command
        .iter()
        .flat_map(|argument| argument.bytes().chain([0]))
        .collect();
    settle(&path, || match std::fs::read(&path) {
        Ok(bytes) if bytes == wanted => Proc::Ready(()),
        Ok(bytes) => Proc::Busy(format!("the command line {:?}", String::from_utf8_lossy(&bytes))),
        Err(error) if mid_exec(&error) => Proc::Busy(format!("{path}: {error}")),
        Err(error) => panic!("{path} cannot be read: {error}"),
    });
}

/// One descriptor of a process, as `/proc/<pid>/fd/<n>` names it; `None` when the process has no such descriptor.
fn fd_target(path: &std::path::Path) -> Proc<Option<String>> {
    match std::fs::read_link(path) {
        Ok(target) => Proc::Ready(Some(target.to_string_lossy().into_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Proc::Ready(None),
        Err(error) if mid_exec(&error) => Proc::Busy(format!("{}: {error}", path.display())),
        Err(error) => panic!("{} cannot be read: {error}", path.display()),
    }
}

/// The sockets a process holds, by the `socket:[inode]` name `/proc` gives each.
///
/// A listing that cannot be read ends the test: "holds nothing" is not what an unreadable `fd` directory means, and
/// the assertions that a process does *not* hold a socket would pass without having looked.
#[track_caller]
fn socket_inodes(pid: i32) -> Vec<String> {
    let directory = format!("/proc/{pid}/fd");
    settle(&directory, || {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if mid_exec(&error) => return Proc::Busy(format!("{directory}: {error}")),
            Err(error) => panic!("{directory} cannot be listed: {error}"),
        };
        let mut sockets = Vec::new();
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) if mid_exec(&error) => return Proc::Busy(format!("{directory}: {error}")),
                Err(error) => panic!("{directory} cannot be listed: {error}"),
            };
            // A descriptor closed since the listing is not held, which is all this is asked.
            match fd_target(&path) {
                Proc::Ready(Some(target)) if target.starts_with("socket:[") => sockets.push(target),
                Proc::Ready(_) => {}
                Proc::Busy(seen) => return Proc::Busy(seen),
            }
        }
        Proc::Ready(sockets)
    })
}

/// The environment of a process, as `KEY=value` entries separated by NULs, once it is the process's own.
///
/// Every child the guardian starts is given `GNOBLIN_READY_FD` (the guardian strips it from the environment it passes
/// on, so it is never inherited from further up), and a read that does not have it is a read of something else: empty,
/// or the guardian's environment from before the `execve`. See [`settle`].
#[track_caller]
fn environment(pid: i32) -> String {
    const OWN: &str = "GNOBLIN_READY_FD";
    let path = format!("/proc/{pid}/environ");
    settle(&path, || match std::fs::read(&path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if text
                .split('\0')
                .any(|entry| entry.split_once('=').is_some_and(|(key, _)| key == OWN))
            {
                Proc::Ready(text)
            } else if bytes.is_empty() {
                Proc::Busy("an empty environment".to_owned())
            } else {
                Proc::Busy(format!("{} bytes of environment without {OWN}", bytes.len()))
            }
        }
        Err(error) if mid_exec(&error) => Proc::Busy(format!("{path}: {error}")),
        Err(error) => panic!("{path} cannot be read: {error}"),
    })
}

/// The descriptor number a process was given for `variable`, from its environment; `None` when it has no such variable.
#[track_caller]
fn inherited(pid: i32, variable: &str) -> Option<i32> {
    let environment = environment(pid);
    let (_, value) = environment
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .find(|(key, _)| *key == variable)?;
    let number = value.trim().parse().unwrap_or_else(|_| {
        panic!("{variable} is `{value}` in pid {pid}'s environment, which is no descriptor number")
    });
    Some(number)
}

/// The socket a process holds at `number`; `None` when it holds no such descriptor.
#[track_caller]
fn socket_at(pid: i32, number: i32) -> Option<String> {
    let path = std::path::PathBuf::from(format!("/proc/{pid}/fd/{number}"));
    settle(&path.display().to_string(), || fd_target(&path))
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

/// A session log, or nothing when it cannot be read.
fn log_text(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
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

impl Drop for Session {
    fn drop(&mut self) {
        self.finished.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(mut guardian) = self.guardian.take() {
            // The guardian's children are in the session's process group only if it made one;
            // killing the guardian and then every pid it reported is what keeps a failing test
            // from leaving a session behind.
            let pids = self.pids();
            let _ = guardian.kill();
            let _ = guardian.wait();
            for pid in pids {
                if let Some(id) = Pid::from_raw(pid) {
                    let _ = rustix::process::kill_process(id, Signal::KILL);
                }
            }
        }
    }
}

#[test]
fn the_guardian_starts_a_session_a_client_can_use() {
    let session = Session::start();
    session.wait_for("[guardian] session ready");
    // The session object is what every client subscribes to first, and `gnoblinctl` is the
    // real client: a session that answers it is a session whose three children are up.
    session.sessions();
    let log = session.log();
    for child in ["compositor", "services", "shell"] {
        assert!(log.contains(&format!("started {child} (")), "no {child} in:\n{log}");
    }
    // The shell reported that it accepted its (empty) configuration, and the guardian recorded it.
    assert!(
        log.contains("configuration revision 0 accepted"),
        "the shell's report is heard:\n{log}"
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
fn a_killed_shell_restarts_and_reports_again() {
    // The property the whole privileged connection exists for: the shell dies, a new one starts, and it rejoins the descriptor
    // its role owns rather than being refused for the rest of the session, and loads the module again.
    let session = Session::start();
    session.wait_for("configuration revision 0 accepted");
    let first = session.pid("shell");
    kill(first);
    let log = session.wait_for_times("configuration revision 0 accepted", 2);
    assert!(
        log.contains("started shell (generation 2"),
        "the shell came back:\n{log}"
    );
    assert!(
        !log.contains("GNOBLIN_IPC_FD is not set"),
        "the restarted shell was given its connection:\n{log}"
    );
    session.sessions();
}

#[test]
fn a_crash_looping_shell_is_cut_off_and_reported() {
    // The shell exits at once on every start (the fault fixture fails its `init` when told to), which is the crash loop the
    // circuit breaker exists for. With the fast ladder it is three restarts and then a stop.
    let fixture = tempfile::tempdir().expect("a fixture directory");
    std::fs::write(fixture.path().join("fail-init"), "").expect("the command file");
    let session = Session::start_shell(
        &shells().fault,
        &[("GNOBLIN_FIXTURE_DIR", fixture.path().to_str().expect("a path"))],
    );
    let log = session.wait_for("the session is degraded");
    assert!(
        log.contains("failed after 3 restarts"),
        "the breaker tripped after its budget:\n{log}"
    );
    // It stopped rather than burning a core: no start after the failure.
    let starts = log.matches("started shell (").count();
    assert!(starts <= 4, "the shell was started {starts} times:\n{log}");
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
    session.wait_for("configuration revision 0 accepted");
    let host = session.pid("shell");
    let services = session.pid("services");

    // Each privileged child was given a descriptor, and the two are different connections.
    let host_fd = inherited(host, "GNOBLIN_IPC_FD").expect("the shell was given a connection");
    let services_fd = inherited(services, "GNOBLIN_IPC_FD").expect("the services role was given a connection");
    let host_socket = socket_at(host, host_fd).expect("the shell's connection is a descriptor");
    let services_socket = socket_at(services, services_fd).expect("the services connection is a descriptor");
    assert!(host_socket.starts_with("socket:["), "{host_socket}");
    assert_ne!(
        host_socket, services_socket,
        "the two roles were given the same connection"
    );

    // The failure this prevents: one role inherits the other's connection, so two processes
    // speak one IPC stream and both see the other's frames.
    assert!(
        !socket_inodes(services).contains(&host_socket),
        "the services role holds the shell's connection: {:?}",
        socket_inodes(services)
    );
    assert!(
        !socket_inodes(host).contains(&services_socket),
        "the shell holds the services connection: {:?}",
        socket_inodes(host)
    );
}

/// An executable stand-in for the XR extension, in a directory the test owns.
fn xr_stand_in(directory: &TempDir, body: &str) -> PathBuf {
    let path = directory.path().join("gnoblin-xr");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("the stand-in program");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("executable");
    path
}

#[test]
fn the_xr_extension_is_not_started_unless_the_user_opts_in() {
    let programs = tempfile::tempdir().expect("a temporary directory");
    let program = xr_stand_in(&programs, "exec sleep 600");
    // The binary is there and named, and the opt-in is not: the session is the one it always was.
    let session = Session::start_with(&[("GNOBLIN_XR_PROGRAM", program.to_str().expect("a path"))]);
    let log = session.wait_for("[guardian] session ready");
    assert!(
        !log.contains("started xr ("),
        "the extension started without an opt-in:\n{log}"
    );
}

#[test]
fn an_opted_in_xr_extension_is_given_a_wayland_socket_no_other_child_has() {
    let programs = tempfile::tempdir().expect("a temporary directory");
    let program = xr_stand_in(&programs, "exec sleep 600");
    let session = Session::start_with(&[
        ("GNOBLIN_XR", "1"),
        ("GNOBLIN_XR_PROGRAM", program.to_str().expect("a path")),
    ]);
    session.wait_for("configuration revision 0 accepted");
    let xr = session.pid("xr");
    let host = session.pid("shell");
    let compositor = session.pid("compositor");
    // `started xr` is logged when the guardian's `spawn` returns, not when the stand-in has finished starting.
    wait_for_command(xr, &["sleep", "600"]);

    let number = inherited(xr, "WAYLAND_SOCKET")
        .unwrap_or_else(|| panic!("the extension was given a Wayland socket:\n{}", session.log()));
    let held = socket_at(xr, number).expect("the socket is a descriptor of the extension");
    assert!(held.starts_with("socket:["), "{held}");
    assert!(
        inherited(host, "WAYLAND_SOCKET").is_none(),
        "only the extension is handed a connection"
    );
    assert!(
        !socket_inodes(host).contains(&held),
        "the shell holds the extension's connection"
    );
    assert!(
        !socket_inodes(compositor).contains(&held),
        "the compositor holds the other end of the pair, which is a different socket"
    );
    assert!(
        inherited(xr, "GNOBLIN_IPC_FD").is_none(),
        "the extension is no privileged role of the guardian's"
    );
    session.sessions();
}

#[test]
fn a_crash_looping_xr_extension_is_dropped_and_the_session_carries_on() {
    let programs = tempfile::tempdir().expect("a temporary directory");
    let program = xr_stand_in(&programs, "exit 1");
    let session = Session::start_with(&[
        ("GNOBLIN_XR", "1"),
        ("GNOBLIN_XR_PROGRAM", program.to_str().expect("a path")),
    ]);
    let log = session.wait_for("xr failed after 3 restarts; it is optional");
    assert!(
        !log.contains("the session is degraded"),
        "an optional extension does not degrade the session:\n{log}"
    );
    // The compositor, shell and services are untouched, and the session still answers.
    session.sessions();
    assert!(!session.log().contains("session stopping"), "{}", session.log());
}

#[test]
fn the_guardian_refuses_to_start_without_a_runtime_directory() {
    let shell = &shells().minimal;
    // A guardian with nowhere to put a socket is a session manager's configuration error, and
    // it says so instead of starting children that cannot connect.
    let output = Command::new(BIN)
        .arg(shell)
        .env("XDG_RUNTIME_DIR", "")
        .env("GNOBLIN_IPC_SOCKET", "")
        .env("XDG_STATE_HOME", std::env::temp_dir())
        .output()
        .expect("the binary runs");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let message = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(message.contains("GNOBLIN_IPC_SOCKET"), "{message}");
}
