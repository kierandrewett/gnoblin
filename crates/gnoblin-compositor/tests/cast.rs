//! The screen cast consent flow against a real PipeWire daemon: the portal reports the user's
//! answer, and the compositor starts a stream only for a session that has one.
//!
//! Each test starts a headless session pointed at a private `pipewire` daemon (the recipe in
//! `design/pipewire-capture-spike.md`), speaks the portal's half of the IPC over a real socket,
//! and asks the daemon with `pw-dump` what it holds. `gnoblin-pipewire-export`'s own tests prove
//! the frames a stream carries; what is proven here is who may have a stream at all.

mod support;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, wire};
use support::Session;

/// How long a test waits for the daemon to answer or a node to appear or go.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The output the portal names when the user chose none: the first one the session has.
const OUTPUT: u64 = wire::OUTPUT_FIRST;

/// A private `pipewire` daemon, killed with its runtime directory when dropped.
struct Daemon {
    child: Child,
    directory: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("a runtime directory for the daemon");
        let binary = std::env::var_os("GNOBLIN_PIPEWIRE_BIN").unwrap_or_else(|| "pipewire".into());
        let child = Command::new(&binary)
            .env("XDG_RUNTIME_DIR", directory.path())
            .env("PIPEWIRE_RUNTIME_DIR", directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| panic!("start {binary:?}: {error}"));
        let daemon = Self { child, directory };
        let deadline = Instant::now() + TIMEOUT;
        while !daemon.socket().exists() {
            assert!(Instant::now() < deadline, "the daemon never made its socket");
            std::thread::sleep(Duration::from_millis(20));
        }
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.directory.path().join("pipewire-0")
    }

    /// What the daemon says about object `id`, as `pw-dump` prints it.
    fn dump(&self, id: u32) -> String {
        let output = Command::new("pw-dump")
            .arg(id.to_string())
            .env("PIPEWIRE_RUNTIME_DIR", self.directory.path())
            .env("XDG_RUNTIME_DIR", self.directory.path())
            .output()
            .expect("pw-dump runs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Every object the daemon holds, as `pw-dump` prints them.
    fn dump_all(&self) -> String {
        let output = Command::new("pw-dump")
            .env("PIPEWIRE_RUNTIME_DIR", self.directory.path())
            .env("XDG_RUNTIME_DIR", self.directory.path())
            .output()
            .expect("pw-dump runs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Whether the daemon holds a stream node of the compositor's.
    fn holds_stream(&self) -> bool {
        self.dump_all().contains("gnoblin-capture-")
    }

    /// Waits until the stream node is gone, or fails.
    #[track_caller]
    fn wait_until_gone(&self, node: u32) {
        let deadline = Instant::now() + TIMEOUT;
        while self.dump(node).contains("gnoblin-capture-") {
            assert!(Instant::now() < deadline, "node {node} is still in the daemon");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A session whose casts go to `daemon`.
fn session_on(daemon: &Daemon) -> Session {
    let socket = daemon.socket();
    let (session, _) = Session::start_privileged_configured(&[], move |config| {
        config.wayland.pipewire_socket = Some(socket);
    });
    session
}

/// The portal's IPC connection.
#[track_caller]
fn portal(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        gnoblin_compositor::version(),
    )
    .expect("the session accepts this build")
}

/// The code of a compositor refusal, which a test matches on.
#[track_caller]
fn refusal_code<T: std::fmt::Debug>(outcome: Result<T, ClientError>) -> String {
    match outcome {
        Err(ClientError::Refused { code, .. }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_start_with_no_recorded_consent_is_refused_and_makes_no_node() {
    let daemon = Daemon::start();
    let session = session_on(&daemon);
    let mut portal = portal(&session);
    assert_eq!(
        refusal_code(portal.cast_start("/session/1")),
        wire::code::NO_CONSENT,
        "a stream is not started for a session the user was never asked about"
    );
    assert!(!daemon.holds_stream(), "no node was made for it");
}

#[test]
fn an_approved_cast_starts_a_stream_the_daemon_holds_and_ends_with_the_session() {
    let daemon = Daemon::start();
    let session = session_on(&daemon);
    let mut portal = portal(&session);
    portal
        .cast_grant("/session/1", OUTPUT)
        .expect("the portal reports the user's yes");
    let stream = portal.cast_start("/session/1").expect("an approved cast starts");
    assert_ne!(stream.node, 0, "the portal has a node to return");
    assert!(
        stream.width > 0 && stream.height > 0,
        "the stream has the output's size: {stream:?}"
    );
    assert!(
        daemon.dump(stream.node).contains("gnoblin-capture-"),
        "the daemon holds the node the compositor named"
    );
    assert_eq!(
        portal.cast_start("/session/1").expect("a retried start").node,
        stream.node,
        "a start the portal repeats does not make a second stream"
    );

    portal.cast_end("/session/1").expect("the portal ends the session");
    daemon.wait_until_gone(stream.node);
    assert_eq!(
        refusal_code(portal.cast_start("/session/1")),
        wire::code::NO_CONSENT,
        "an ended session holds no approval"
    );
}

#[test]
fn a_cast_of_an_output_that_is_not_there_is_refused() {
    let daemon = Daemon::start();
    let session = session_on(&daemon);
    let mut portal = portal(&session);
    portal
        .cast_grant("/session/1", 999)
        .expect("the answer is recorded as given");
    assert_eq!(
        refusal_code(portal.cast_start("/session/1")),
        wire::code::CAPTURE_NO_OUTPUT
    );
    assert!(!daemon.holds_stream());
}

#[test]
fn a_cast_with_no_daemon_is_refused_unavailable() {
    let directory = tempfile::tempdir().expect("a directory with no daemon in it");
    let socket = directory.path().join("pipewire-0");
    let (session, _) = Session::start_privileged_configured(&[], move |config| {
        config.wayland.pipewire_socket = Some(socket);
    });
    let mut portal = portal(&session);
    portal.cast_grant("/session/1", OUTPUT).expect("the answer is recorded");
    assert_eq!(
        refusal_code(portal.cast_start("/session/1")),
        wire::code::CAPTURE_UNAVAILABLE,
        "no PipeWire daemon is a named refusal, not a hang or a fabricated node"
    );
}

#[test]
fn a_cast_ends_when_the_connection_that_held_it_goes_away() {
    let daemon = Daemon::start();
    let session = session_on(&daemon);
    let mut portal = portal(&session);
    portal
        .cast_grant("/session/1", OUTPUT)
        .expect("the portal reports the user's yes");
    let stream = portal.cast_start("/session/1").expect("an approved cast starts");

    // The portal crashes: the user's answer, and the stream it allowed, went with it.
    drop(portal);
    daemon.wait_until_gone(stream.node);
}

#[test]
fn another_connection_can_neither_start_nor_end_a_cast_it_did_not_decide() {
    let daemon = Daemon::start();
    let session = session_on(&daemon);
    let mut first = portal(&session);
    let mut second = portal(&session);
    first
        .cast_grant("/session/1", OUTPUT)
        .expect("the first connection decides");
    assert_eq!(
        refusal_code(second.cast_start("/session/1")),
        wire::code::SESSION_UNKNOWN
    );
    assert_eq!(refusal_code(second.cast_end("/session/1")), wire::code::SESSION_UNKNOWN);
    assert_eq!(
        refusal_code(second.cast_grant("/session/1", OUTPUT)),
        wire::code::SESSION_TAKEN
    );
    assert!(!daemon.holds_stream(), "none of that made a stream");
    first.cast_start("/session/1").expect("the holder still can");
}
