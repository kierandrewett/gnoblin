//! The runtime role, over a real socket pair.
//!
//! The runtime is the compositor's privileged client, so the peer here is the compositor's
//! side of the pair: it accepts the handshake with the grant a runtime connection receives,
//! answers the subscribe and the configuration commit, and refuses one of them to prove what
//! the role does when it cannot commit. The role itself is the shipped one.
//!
//! # Failure modes first
//!
//! 1. **The guardian did not pass a connection.** The role refuses at once and names the
//!    variable: `the_role_refuses_without_a_privileged_connection`.
//! 2. **The compositor refuses the commit.** The role ends with the refusal and commits
//!    nothing more, and the document the compositor already had stays in force:
//!    `a_refused_commit_ends_the_role`.
//! 3. **The role commits something other than the document it validated.** The frame the peer
//!    receives carries the loaded document, as JSON, which is what the compositor stores:
//!    `the_role_commits_the_document_it_loaded`.

use std::io::{BufReader, Write as _};
use std::os::fd::IntoRawFd as _;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gnoblin_client::wire;
use gnoblin_config::SearchPaths;
use gnoblin_lua::{Limits, Role};
use gnoblin_model::{Revision, Snapshot};
use gnoblin_proto::{Capabilities, Capability, Envelope, Handshake, HandshakeKind, read_frame, write_frame};

/// The compositor's side of one privileged pair.
struct Peer {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Peer {
    /// Answer the client's hello with the grant a runtime connection holds.
    fn accept(stream: UnixStream) -> Peer {
        let mut peer = Peer {
            reader: BufReader::new(stream.try_clone().expect("a duplicated socket")),
            stream,
        };
        let hello = peer.handshake();
        assert_eq!(hello.kind, HandshakeKind::Hello, "the role opens with a hello");
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        let answer = Handshake::grant(
            gnoblin_proto::Version::new(major, minor, patch),
            Capabilities::of(&[Capability::CommitConfig, Capability::InputPolicy]),
        );
        peer.stream.write_all(&answer.encode()).expect("the accept");
        peer.stream.flush().expect("the accept");
        peer
    }

    fn handshake(&mut self) -> Handshake {
        let mut frame = [0u8; gnoblin_proto::HANDSHAKE_LEN];
        std::io::Read::read_exact(&mut self.reader, &mut frame).expect("a handshake");
        Handshake::decode(&frame).expect("a valid handshake")
    }

    fn next(&mut self) -> Option<Envelope> {
        read_frame(&mut self.reader).ok().flatten()
    }

    fn reply(&mut self, envelope: &Envelope, payload: Vec<u8>) {
        let reply = Envelope::reply(
            &envelope.namespace,
            envelope.member.clone(),
            envelope.request_id,
            payload,
        );
        write_frame(&mut self.stream, &reply).expect("the reply");
        self.stream.flush().expect("the reply");
    }

    fn refuse(&mut self, envelope: &Envelope, code: &str) {
        let refusal = Envelope::error(
            &envelope.namespace,
            envelope.member.clone(),
            envelope.request_id,
            code,
            "this connection does not serve that",
        );
        write_frame(&mut self.stream, &refusal).expect("the refusal");
        self.stream.flush().expect("the refusal");
    }

    /// Answer the subscription and return the commit frame, which is what the role does next.
    fn subscribe(&mut self) -> Envelope {
        let subscribe = self.next().expect("the role subscribes first");
        assert_eq!(subscribe.member, wire::member::SUBSCRIBE, "{subscribe:?}");
        // The snapshot a real compositor sends: the session object and its version.
        self.reply(
            &subscribe,
            wire::snapshot_payload(&Snapshot::new(Revision::new(0), Vec::new())),
        );
        let commit = self.next().expect("the role commits the document it loaded");
        assert_eq!(commit.member, wire::member::CONFIG_COMMIT, "{commit:?}");
        commit
    }
}

/// One running role, its stop flag and the thread it owns.
struct Running {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<(), gnoblin_lua::RoleError>>>,
}

impl Running {
    fn start(number: i32, config_home: PathBuf) -> Running {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let role = Role {
            env: Box::new(move |name| (name == "GNOBLIN_IPC_FD").then(|| number.to_string())),
            paths: SearchPaths {
                home: None,
                config_home: Some(config_home),
                override_path: None,
            },
            limits: Limits::default(),
        };
        let thread = std::thread::Builder::new()
            .name("runtime-role".to_owned())
            .spawn(move || role.run(move || flag.load(Ordering::SeqCst)))
            .expect("the role thread");
        Running {
            stop,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> Result<(), gnoblin_lua::RoleError> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread
            .take()
            .expect("the role thread")
            .join()
            .expect("the role did not panic")
    }
}

/// A configuration directory with no `init.lua`, which is a first login.
fn config_home() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

#[test]
fn the_role_refuses_without_a_privileged_connection() {
    let role = Role {
        env: Box::new(|_| None),
        paths: SearchPaths {
            home: None,
            config_home: None,
            override_path: None,
        },
        limits: Limits::default(),
    };
    let message = role.run(|| true).expect_err("no connection").to_string();
    assert!(message.contains("GNOBLIN_IPC_FD"), "{message}");
}

#[test]
fn the_role_commits_the_document_it_loaded() {
    let (role_end, peer_end) = UnixStream::pair().expect("a socket pair");
    let mut peer = Peer::accept(peer_end);
    let config = config_home();
    let running = Running::start(role_end.into_raw_fd(), config.path().to_path_buf());

    let commit = peer.subscribe();
    let document = wire::read_config_commit(commit.payload().unwrap_or(&[])).expect("a document");
    // The compositor never decodes the document; what it stores is what the runtime validated,
    // so the frame must be the JSON tree `gnoblin.configure` produced.
    let parsed: serde_json::Value = serde_json::from_slice(&document).expect("the document is JSON");
    assert!(parsed.is_object(), "a configuration document is a table: {parsed}");
    peer.reply(&commit, 1_u64.to_be_bytes().to_vec());

    running.finish().expect("the role ends cleanly");
}

#[test]
fn a_refused_commit_ends_the_role() {
    // The failure this prevents: the compositor refuses the commit and the runtime carries on
    // as if the session were running the document it just wrote.
    let (role_end, peer_end) = UnixStream::pair().expect("a socket pair");
    let mut peer = Peer::accept(peer_end);
    let config = config_home();
    let running = Running::start(role_end.into_raw_fd(), config.path().to_path_buf());

    let commit = peer.subscribe();
    peer.refuse(&commit, gnoblin_client::code::DENIED);

    let message = running.finish().expect_err("the role ends").to_string();
    assert!(message.contains("denied"), "the refusal is reported: {message}");
}
