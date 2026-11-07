//! One test per documented failure mode, over real sockets and real peers.
//!
//! The list is in `src/lib.rs`; each entry names its test here. Nothing is mocked:
//! the session is a real headless compositor, the Wayland peer is a real
//! `wayland-client` connection, and the IPC peer is either the real `gnoblinctl`
//! library entry or a socket that speaks the framing by hand to send what a client
//! library would refuse to send.
//!
//! The tests in this file are the ones about the *boundary*: a socket that is stale,
//! taken, or in a directory another account could reach; a peer with the wrong
//! version; a peer that dies; a frame that does not decode. The happy path — a shell
//! listing, watching and calling through `gnoblinctl` — is `end_to_end.rs`.

mod support;

use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ConnectError, Event};
use gnoblin_compositor::{Error, IpcError};
use gnoblin_model::{Fields, Filter, ObjectId, Observation, Value};
use gnoblin_proto::HandshakeKind;
use gnoblin_types::Version;

use support::{Client as WaylandClient, Command, Session, config, run_refused, wait_for, wait_for_listed_id};

/// The version this build serves.
fn version() -> Version {
    gnoblin_compositor::version()
}

/// An IPC client connection at the served version.
#[track_caller]
fn ipc(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        version(),
    )
    .expect("the session accepts this build")
}

#[test]
fn a_stale_socket_is_replaced() {
    let directory = support::private_directory();
    let path = directory.path().join("ipc.sock");
    // Exactly what a session that was killed leaves: a bound socket with nobody
    // listening on it.
    let listener = UnixListener::bind(&path).expect("bind");
    drop(listener);
    assert!(path.exists(), "the stale file is in the way");

    let session = Session::start_with(Some(path.clone()));
    session.ctl(&["window", "list"]).ok();
    session.stop().expect("the session stops cleanly");
    assert!(!path.exists(), "the socket is removed when the session stops");
}

#[test]
fn two_compositors_on_one_socket_are_refused() {
    let session = Session::start();
    let directory = support::private_directory();
    let error = run_refused(config(directory.path(), session.socket().to_path_buf()));
    match error {
        Error::Ipc(IpcError::Running { path }) => assert_eq!(path, session.socket()),
        other => panic!("a live socket must be refused, got {other:?}"),
    }
    // The first session never noticed: it is still the one serving that socket.
    session.ctl(&["session", "list"]).ok();
    session.stop().expect("the first session stops cleanly");
}

#[test]
fn a_reachable_socket_directory_is_refused() {
    let directory = support::private_directory();
    let shared = directory.path().join("shared");
    std::fs::create_dir(&shared).expect("the directory");
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).expect("the mode");
    let error = run_refused(config(&shared, shared.join("ipc.sock")));
    match error {
        Error::Ipc(IpcError::Directory { reason, .. }) => {
            assert!(reason.contains("0700"), "the refusal names the mode: {reason}");
        }
        other => panic!("a directory another account can enter must be refused, got {other:?}"),
    }
}

#[test]
fn a_wrong_version_client_is_refused() {
    let session = Session::start();

    // The fixed-layout handshake answers with the compositor's own version, so the
    // raw refusal is readable before any envelope is decoded.
    let (_wire, answer) = support::Wire::connect_version(session.socket(), Version::new(0, 3, 0));
    assert_eq!(answer.kind, HandshakeKind::Refuse);
    assert_eq!(answer.version, version());

    // A real client turns that into an error that names both versions.
    let error = Client::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        Version::new(0, 3, 0),
    )
    .expect_err("a 0.3 client is refused by a 0.2 compositor");
    let ConnectError::Refused(_) = &error else {
        panic!("expected a version refusal, got {error:?}");
    };
    let message = error.to_string();
    assert!(message.contains(&version().to_string()), "{message}");
    assert!(message.contains("0.3.0"), "{message}");

    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_ipc_peer_that_dies_does_not_stop_the_compositor() {
    let session = Session::start();
    let mut client = ipc(&session);
    client.subscribe(Filter::All).expect("the snapshot");

    // A process that is killed leaves its subscription behind; the connection ends
    // and nothing else may notice.
    drop(client);

    session.ctl(&["session", "list"]).ok();
    let mut wayland = WaylandClient::connect(session.wayland());
    wayland.map_window("org.gnoblin.Alpha", "Alpha");
    wait_for_listed_id(&session, "Alpha");
    assert_eq!(session.ctl(&["window", "list"]).ok().out.lines().count(), 1);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_window_object_outlives_its_surface_only_until_the_surface_ends() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    let window = wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let id = wait_for_listed_id(&session, "Alpha");

    // Subscribe to the window before it closes, so the end arrives as an observation
    // rather than as a missing snapshot entry.
    let mut client = ipc(&session);
    let snapshot = client
        .subscribe(Filter::Object(ObjectId::new(id)))
        .expect("the snapshot");
    assert!(snapshot.object(ObjectId::new(id)).is_some(), "the window is live");

    wayland.destroy(window);
    let closed = wait_for("the window object closes", || {
        loop {
            match client.next_event().expect("an observation") {
                gnoblin_client::Event::Model(Observation::Closed { object, .. }) => return Some(object),
                gnoblin_client::Event::Model(_) => {}
                gnoblin_client::Event::RequestOffered { .. }
                | gnoblin_client::Event::RequestAnswered { .. }
                | gnoblin_client::Event::RequestFinished { .. }
                | gnoblin_client::Event::Extension { .. } => {}
            }
        }
    });
    assert_eq!(closed, ObjectId::new(id));
    assert_eq!(session.ctl(&["window", "list"]).ok().out, "", "no window is left");

    // The identity is never reused: a new window gets a new object, and this one stays
    // unknown.
    let mut wayland = WaylandClient::connect(session.wayland());
    wayland.map_window("org.gnoblin.Beta", "Beta");
    let beta = wait_for_listed_id(&session, "Beta");
    assert_ne!(beta, id, "an identity is never reused");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn focus_on_a_closed_window_is_refused() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    let window = wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let id = wait_for_listed_id(&session, "Alpha");
    wayland.destroy(window);
    wait_for("the window is gone", || {
        let listed = session.ctl(&["window", "list"]).ok();
        listed.out.is_empty().then_some(())
    });

    // The identity is dead, so the call is refused with the schema's code for an
    // object no live object has, and the session keeps running.
    let mut client = ipc(&session);
    assert!(
        client
            .subscribe(Filter::Object(ObjectId::new(id)))
            .expect("the snapshot")
            .object(ObjectId::new(id))
            .is_none(),
        "the object is not in the snapshot"
    );
    let error = client
        .call(ObjectId::new(id), "focus", &Fields::new())
        .expect_err("a closed window cannot be focused");
    match error {
        gnoblin_client::ClientError::Refused { code, .. } => {
            assert_eq!(code, gnoblin_client::code::OBJECT_UNKNOWN);
        }
        other => panic!("expected object-unknown, got {other:?}"),
    }

    // The same refusal through the real client, as a script sees it.
    let refusal = session.ctl(&["window", "call", &id.to_string(), "focus"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    assert!(refusal.err.contains("object-unknown"), "{}", refusal.err);
    session.stop().expect("the session stops cleanly");
}

/// The window-control methods refuse a dead identity the same way focus does: the
/// object is unknown, and the session keeps running.
#[test]
fn window_control_on_a_closed_window_is_refused() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    let window = wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let id = wait_for_listed_id(&session, "Alpha");
    wayland.destroy(window);
    wait_for("the window is gone", || {
        let listed = session.ctl(&["window", "list"]).ok();
        listed.out.is_empty().then_some(())
    });

    let mut client = ipc(&session);
    for method in ["close", "set_minimized", "set_maximized", "set_fullscreen"] {
        let arguments = Fields::new();
        let error = client
            .call(ObjectId::new(id), method, &arguments)
            .expect_err("a closed window takes no window-control call");
        match error {
            gnoblin_client::ClientError::Refused { code, .. } => {
                assert_eq!(code, gnoblin_client::code::OBJECT_UNKNOWN, "{method}");
            }
            other => panic!("{method}: expected object-unknown, got {other:?}"),
        }
    }
    session.stop().expect("the session stops cleanly");
}

/// A state change needs its boolean. A call without one is refused with the schema's
/// code for a bad member, and the window keeps the state it had.
#[test]
fn set_fullscreen_without_its_value_is_refused() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let id = wait_for_listed_id(&session, "Alpha");

    let mut client = ipc(&session);
    let error = client
        .call(ObjectId::new(id), "set_fullscreen", &Fields::new())
        .expect_err("set_fullscreen needs its value");
    assert!(
        matches!(error, gnoblin_client::ClientError::Refused { .. }),
        "a missing value is refused: {error:?}"
    );
    session.stop().expect("the session stops cleanly");
}

/// A move needs both coordinates and a resize both sizes. A call without them is refused
/// with the schema's code for a bad member, and the window keeps the geometry it had.
#[test]
fn move_and_resize_without_their_geometry_are_refused() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let id = wait_for_listed_id(&session, "Alpha");

    let mut client = ipc(&session);
    let only_x = Fields::from([("x".to_owned(), Value::Int(40))]);
    let only_width = Fields::from([("width".to_owned(), Value::Uint(300))]);
    for (method, arguments) in [
        ("move", Fields::new()),
        ("move", only_x),
        ("resize", Fields::new()),
        ("resize", only_width),
    ] {
        let error = client
            .call(ObjectId::new(id), method, &arguments)
            .expect_err("move and resize need their full geometry");
        match error {
            gnoblin_client::ClientError::Refused { code, .. } => {
                assert_eq!(code, gnoblin_client::code::UNKNOWN_MEMBER, "{method}");
            }
            other => panic!("{method}: expected unknown-member, got {other:?}"),
        }
    }
    session.stop().expect("the session stops cleanly");
}

/// `set_workspace` and `set_output` take the object the session declared for the target.
/// A window's own identity is not a workspace or an output, so both are refused as an
/// object the session does not declare, and the window is left where it was.
#[test]
fn set_workspace_and_set_output_refuse_objects_that_are_not_declared_targets() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let id = wait_for_listed_id(&session, "Alpha");

    let mut client = ipc(&session);
    for (method, name) in [("set_workspace", "workspace"), ("set_output", "output")] {
        let arguments = Fields::from([(name.to_owned(), Value::Object(ObjectId::new(id)))]);
        let error = client
            .call(ObjectId::new(id), method, &arguments)
            .expect_err("a window is not a workspace or an output");
        match error {
            gnoblin_client::ClientError::Refused { code, .. } => {
                assert_eq!(code, gnoblin_client::code::OBJECT_UNKNOWN, "{method}");
            }
            other => panic!("{method}: expected object-unknown, got {other:?}"),
        }
    }
    session.stop().expect("the session stops cleanly");
}

#[test]
fn focus_completes_after_the_diff_it_caused() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let alpha = wait_for_listed_id(&session, "Alpha");
    let mut second = WaylandClient::connect(session.wayland());
    second.map_window("org.gnoblin.Beta", "Beta");
    let beta = wait_for_listed_id(&session, "Beta");
    // Under the documented default a window that maps while nothing has focus takes it,
    // and a later window is refused. Alpha holds focus, so focus beta first, so that
    // asking for alpha changes the focus the subscription watches.
    let mut setup = ipc(&session);
    setup.subscribe(Filter::All).expect("the snapshot");
    let beta_focus = setup
        .call(ObjectId::new(beta), "focus", &Fields::new())
        .expect("the call is served");
    assert!(setup.await_operation(beta_focus).expect("the completion").is_done());

    // Ask for alpha and read the observations in the order the compositor sent them.
    let mut client = ipc(&session);
    client.subscribe(Filter::All).expect("the snapshot");
    let operation = client
        .call(ObjectId::new(alpha), "focus", &Fields::new())
        .expect("the call is served");
    let outcome = client.await_operation(operation).expect("the completion");
    assert!(outcome.is_done(), "focus was carried out: {outcome:?}");

    let mut focused_before_completion = None;
    loop {
        match client.next_event().expect("an observation") {
            gnoblin_client::Event::Model(Observation::Changed {
                object,
                property,
                value,
                ..
            }) if object == ObjectId::new(alpha) && property == "focused" => {
                assert_eq!(value, gnoblin_model::Value::Bool(true), "alpha took focus");
                focused_before_completion = Some(());
            }
            gnoblin_client::Event::Model(Observation::Finished { operation: done, .. }) if done == operation.id() => {
                break;
            }
            gnoblin_client::Event::Model(_) => {}
            gnoblin_client::Event::RequestOffered { .. }
            | gnoblin_client::Event::RequestAnswered { .. }
            | gnoblin_client::Event::RequestFinished { .. }
            | gnoblin_client::Event::Extension { .. } => {}
        }
    }
    assert!(
        focused_before_completion.is_some(),
        "the diff reached the client before the completion"
    );
    assert_ne!(alpha, beta);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_malformed_frame_is_refused_and_the_connection_survives() {
    let session = Session::start();
    let mut wire = support::Wire::connect(session.socket());
    // A subscribe whose payload is a filter tag this build does not know.
    wire.send(&gnoblin_proto::Envelope::request(
        gnoblin_client::NAMESPACE,
        gnoblin_client::member::SUBSCRIBE,
        1,
        vec![0xff],
    ));
    let refusal = wire.receive().expect("an error frame");
    assert_eq!(
        refusal.error_code(),
        Some(gnoblin_client::code::UNKNOWN_MEMBER),
        "the payload is refused with the schema's code"
    );

    // The connection is still usable: the next frame is a real subscribe and it is
    // answered with a snapshot.
    wire.send(&gnoblin_client::wire::subscribe(2, Filter::All));
    let reply = wire.receive().expect("a reply");
    let snapshot =
        gnoblin_client::wire::read_snapshot(reply.payload().unwrap_or_default()).expect("the snapshot payload decodes");
    assert!(
        snapshot
            .objects()
            .iter()
            .any(|view| view.ty().to_string() == "gnoblin.session"),
        "the session object is in the snapshot"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_silent_peer_does_not_block_the_session() {
    let session = Session::start();
    // Connected and then silent: its own reader waits on it, and every other client is
    // served in the meantime. A connection that never speaks gives up on the reader's
    // own handshake deadline instead of holding a thread for the session's lifetime.
    let silent = UnixStream::connect(session.socket()).expect("the session is listening");
    session.ctl(&["session", "list"]).ok();
    session.ctl(&["window", "list"]).ok();
    drop(silent);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_session_object_carries_the_ipc_version() {
    let session = Session::start();
    let mut client = ipc(&session);
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    let session_object = snapshot
        .objects()
        .iter()
        .find(|view| view.ty().to_string() == "gnoblin.session")
        .expect("the session object");
    assert_eq!(
        session_object.property("version"),
        Some(&gnoblin_model::Value::Text(version().to_string()))
    );

    // A short-lived client reads the same value without a subscription of its own.
    // The `ready` signal is raised before the server accepts anyone, so it is this
    // property a client arriving later starts from.
    let printed = Command::run(session.socket(), &["session", "get", "1", "version"]).ok();
    assert_eq!(printed.out.trim(), format!("{:?}", version().to_string()));
    session.stop().expect("the session stops cleanly");
}

/// The `idle` object's identity, read from a fresh snapshot.
#[track_caller]
fn idle_id(session: &Session) -> u64 {
    let mut client = ipc(session);
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &gnoblin_model::TypeKey::gnoblin("idle"))
        .map(|view| view.id().get())
        .expect("the idle object")
}

/// The `inhibitors` count the compositor publishes, read from a fresh snapshot.
#[track_caller]
fn inhibitors(session: &Session) -> u64 {
    let mut client = ipc(session);
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    let idle = snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &gnoblin_model::TypeKey::gnoblin("idle"))
        .expect("the idle object");
    match idle.property("inhibitors") {
        Some(gnoblin_model::Value::Uint(count)) => *count,
        other => panic!("inhibitors is not a uint: {other:?}"),
    }
}

#[test]
fn an_inhibit_is_counted_until_it_is_released() {
    let session = Session::start();
    let idle = idle_id(&session).to_string();
    assert_eq!(inhibitors(&session), 0);

    session.ctl(&["idle", "call", &idle, "inhibit", "holder=a"]).ok();
    assert_eq!(inhibitors(&session), 1);

    // Holding again under the same name changes nothing.
    session.ctl(&["idle", "call", &idle, "inhibit", "holder=a"]).ok();
    assert_eq!(inhibitors(&session), 1);

    session.ctl(&["idle", "call", &idle, "uninhibit", "holder=a"]).ok();
    assert_eq!(inhibitors(&session), 0);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_holder_cap_refuses_the_257th_inhibit() {
    let session = Session::start();
    let idle = idle_id(&session).to_string();
    for holder in 0..256 {
        session
            .ctl(&["idle", "call", &idle, "inhibit", &format!("holder={holder}")])
            .ok();
    }
    assert_eq!(inhibitors(&session), 256);

    let refusal = session.ctl(&["idle", "call", &idle, "inhibit", "holder=overflow"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    let said = format!("{}{}", refusal.out, refusal.err);
    assert!(said.contains("too-many-inhibitors"), "{said}");
    assert_eq!(inhibitors(&session), 256, "the refused hold was not recorded");
    session.stop().expect("the session stops cleanly");
}

/// `window.request_menu` emits `menu_requested` on the window with the anchor it was
/// given and `source = "extension"`, and a subscriber to that window receives it.
///
/// The signal is the only way a menu is shown, so this is the end-to-end proof that an
/// extension asking for a menu reaches the subscriber that would draw it.
#[test]
fn a_request_menu_call_reaches_a_subscriber_with_its_anchor() {
    let session = Session::start();
    let mut wayland = WaylandClient::connect(session.wayland());
    let window = wayland.map_window("org.gnoblin.Alpha", "Alpha");
    let id = wait_for_listed_id(&session, "Alpha");

    let mut client = ipc(&session);
    client
        .subscribe(Filter::Object(ObjectId::new(id)))
        .expect("the snapshot");
    let arguments = Fields::from([
        ("x".to_owned(), Value::Int(40)),
        ("y".to_owned(), Value::Int(50)),
        ("width".to_owned(), Value::Uint(0)),
        ("height".to_owned(), Value::Uint(0)),
    ]);
    client
        .call(ObjectId::new(id), "request_menu", &arguments)
        .expect("the call is accepted");

    // The signal may arrive after other model changes, so read a bounded run of events
    // and stop at the first `menu_requested` on this window.
    let mut seen = None;
    let mut trace: Vec<String> = Vec::new();
    for _ in 0..200 {
        let event = client.next_event().expect("the next event");
        trace.push(format!("{event:?}"));
        match event {
            Event::Model(Observation::Signalled {
                object, signal, fields, ..
            }) if object == ObjectId::new(id) && signal == "menu_requested" => {
                seen = Some(fields);
                break;
            }
            _ => {}
        }
    }
    let fields = seen.unwrap_or_else(|| panic!("menu_requested did not reach the subscriber; events read: {trace:#?}"));
    assert_eq!(fields.get("x"), Some(&Value::Int(40)), "{fields:?}");
    assert_eq!(fields.get("y"), Some(&Value::Int(50)), "{fields:?}");
    assert_eq!(
        fields.get("source"),
        Some(&Value::Text("extension".to_owned())),
        "{fields:?}"
    );
    assert!(matches!(fields.get("seat"), Some(Value::Text(_))), "{fields:?}");

    wayland.destroy(window);
    session.stop().expect("the session stops cleanly");
}
