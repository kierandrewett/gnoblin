//! The shell VM's session socket: `connect_owner_socket` finds the owner socket by the same rule
//! as every other role, handshakes with it, and attaches the tree on the connection it opens.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gnoblin_client::SOCKET_VARIABLE;
use gnoblin_lua::shell_role::connect_owner_socket;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, SnapshotPayload};
use gnoblin_proto::{Envelope, HANDSHAKE_LEN, Handshake, HandshakeKind, IPC_VERSION, Version, read_frame, write_frame};

const WAIT: Duration = Duration::from_secs(10);

fn lua_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

fn temp_socket() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock")
        .as_nanos();
    std::env::temp_dir().join(format!("gnoblin-owner-{}-{nanos}.sock", std::process::id()))
}

/// Answers the role's hello with an accept and the (empty) grants frame, as the compositor does.
fn accept_handshake(stream: &mut UnixStream) {
    stream.set_read_timeout(Some(WAIT)).expect("the deadline");
    let mut hello = [0u8; HANDSHAKE_LEN];
    stream.read_exact(&mut hello).expect("a hello");
    let hello = Handshake::decode(&hello).expect("a valid hello");
    assert_eq!(hello.kind, HandshakeKind::Hello, "the role opens with a hello");
    let (major, minor, patch) = IPC_VERSION;
    stream
        .write_all(&Handshake::accept(Version::new(major, minor, patch)).encode())
        .expect("the accept");
    gnoblin_proto::write_grants(stream, &[]).expect("the grants frame after the accept");
}

/// Reads the attach request and answers with the attached reply and a one-node snapshot.
fn answer_attach(stream: &mut UnixStream) {
    let request = read_frame(stream).expect("a frame").expect("the role sent attach");
    assert_eq!(
        (request.namespace.as_str(), request.member.as_str()),
        ("tree", "attach")
    );
    let args: AttachArgs = request.payload_as().expect("the attach args decode");
    assert_eq!(args.version, 1, "the VM attaches with protocol version 1");
    write_frame(
        stream,
        &Envelope::reply_message(
            "tree",
            "attach",
            request.request_id,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 0,
            },
        ),
    )
    .expect("the attached reply");
    write_frame(
        stream,
        &Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 0,
                chunk: 0,
                last: true,
                nodes: vec![snapshot_node::Properties {
                    id: 1,
                    parent: 0,
                    index: 0,
                    class: "Node".to_owned(),
                }],
            },
        ),
    )
    .expect("the snapshot event");
}

#[test]
fn owner_socket_is_found_by_the_override_then_handshaken_and_attached() {
    let path = temp_socket();
    let listener = UnixListener::bind(&path).expect("bind the owner socket");
    let compositor = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the role connects");
        accept_handshake(&mut stream);
        answer_attach(&mut stream);
        // A compositor keeps the owner connection open; hold it until the role drops its end.
        let mut rest = [0u8; 1];
        let _ = stream.read(&mut rest);
    });

    let socket = path.display().to_string();
    let env = move |name: &str| (name == SOCKET_VARIABLE).then(|| socket.clone());
    let role = connect_owner_socket(&env, &lua_dir());
    let failure = role.as_ref().err().map(|error| format!("{error:?}"));
    drop(role);
    compositor.join().expect("the fake compositor finished");
    let _ = std::fs::remove_file(&path);
    assert!(
        failure.is_none(),
        "the owner socket must give the shell VM a session: {failure:?}"
    );
}

#[test]
fn owner_socket_refuses_when_no_socket_is_named() {
    let env = |_: &str| None;
    let role = connect_owner_socket(&env, &lua_dir());
    assert!(role.is_err(), "with no socket path there is no session to open");
}
