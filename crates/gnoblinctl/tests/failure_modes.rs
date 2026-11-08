//! One test per documented failure mode, driven through the real parse, the real
//! command logic and the real exit codes.
//!
//! Two servers are used, and neither is a compositor:
//!
//! - the in-process loopback ([`gnoblin_client::Server`]) for everything that needs
//!   a live [`gnoblin_model::Graph`], behind a transport that steps the model
//!   whenever the client has nothing to read. Without that step a single-threaded
//!   test would deadlock the moment `watch` or `call` waited for a change it had not
//!   produced yet;
//! - a real Unix socket, for the paths that are about the socket rather than the
//!   graph: a missing socket, a refused version, and an error frame.
//!
//! The failure modes are listed in `src/lib.rs`.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::Write as _;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use gnoblin_client::Transport as ClientTransport;
use gnoblin_client::{Client, HandshakeError, Loopback, Server};
use gnoblin_model::{ErrorCode, Fields, Graph, ObjectId, OperationOutcome, TypeKey, Value};
use gnoblin_proto::Transport as WireTransport;
use gnoblin_proto::{CodecError, Envelope, Grant, Handshake, Version, write_frame};
use gnoblin_schema::Catalog;

use gnoblinctl::{ConnectFailure, Connection, SocketConnection, UnixTransport};

/// The version this build speaks, which the fixture server serves.
const VERSION: Version = Version::new(0, 2, 0);

/// The verbs written by hand rather than generated from the schema. They are not
/// object commands, so the drift check must not expect a schema object behind them.
const HAND_WRITTEN: &[&str] = &["ping", "status", "version", "capabilities"];

/// A unique path under the temporary directory.
fn scratch(suffix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "gnoblinctl-{}-{}-{suffix}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn catalog() -> Catalog {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gnoblin-schema/schema");
    Catalog::load_dir(&dir).expect("the checked-in schema loads")
}

fn window_properties(title: &str) -> Fields {
    gnoblin_testkit::window_fields(&[
        ("id", Value::Uint(1)),
        ("title", Value::Text(title.to_owned())),
        ("app_id", Value::Text("org.example.App".to_owned())),
        ("width", Value::Uint(800)),
        ("height", Value::Uint(600)),
    ])
}

/// One step the fixture server takes when the client has nothing to read.
enum Step {
    /// Set one property, which publishes a diff.
    Set(ObjectId, &'static str, Value),
    /// Complete the oldest pending operation.
    Complete(OperationOutcome),
}

/// A transport over the in-process link that steps the model on an empty read.
///
/// `Loopback::receive` answers `Ok(None)` when the outbox is empty, which the client
/// reads as a closed connection. Here that moment is where the test's next mutation
/// is applied, so a single-threaded test can drive `watch` and `call` exactly as a
/// compositor's own thread would.
struct Stepping {
    link: Loopback,
    server: Server,
    steps: VecDeque<Step>,
}

impl Stepping {
    /// Apply the next step, if any is left.
    fn step(&mut self) -> bool {
        let Some(step) = self.steps.pop_front() else {
            return false;
        };
        match step {
            Step::Set(object, property, value) => {
                self.server.set(object, property, value).expect("a declared property");
            }
            Step::Complete(outcome) => {
                let pending: Vec<_> = self.server.graph().pending().collect();
                if let Some(operation) = pending.first() {
                    self.server
                        .complete(operation.id(), outcome)
                        .expect("a pending operation");
                }
            }
        }
        true
    }
}

impl WireTransport for Stepping {
    fn send(&mut self, envelope: &Envelope, fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
        self.link.send(envelope, fds)
    }

    fn receive(&mut self, fds: &mut Vec<OwnedFd>) -> Result<Option<Envelope>, CodecError> {
        loop {
            if let Some(envelope) = self.link.receive(fds)? {
                return Ok(Some(envelope));
            }
            if !self.step() {
                return Ok(None);
            }
        }
    }
}

impl ClientTransport for Stepping {
    fn send_handshake(&mut self, frame: &Handshake) -> Result<(), HandshakeError> {
        self.link.send_handshake(frame)
    }

    fn receive_handshake(&mut self) -> Result<Handshake, HandshakeError> {
        self.link.receive_handshake()
    }

    fn receive_grants(&mut self) -> Result<Vec<Grant>, HandshakeError> {
        self.link.receive_grants()
    }
}

/// A connection to an in-process graph, with the steps the test wants applied.
struct ModelConnection {
    server: Server,
    steps: VecDeque<Step>,
}

impl ModelConnection {
    fn new(graph: Graph, steps: Vec<Step>) -> ModelConnection {
        ModelConnection {
            server: Server::new(graph, VERSION),
            steps: steps.into(),
        }
    }
}

impl Connection for ModelConnection {
    type Transport = Stepping;

    fn connect(mut self) -> Result<Client<Stepping>, ConnectFailure> {
        // The link is created by `Server::connect`, which performs one handshake.
        // The transport is taken back out of that client and handed a second time to
        // `Client::connect`, so the command under test sees an ordinary connection
        // while the test keeps the model beside it.
        let probe = self.server.connect(VERSION).map_err(ConnectFailure::from_connect)?;
        let stepping = Stepping {
            link: probe.into_inner(),
            server: self.server,
            steps: self.steps,
        };
        Client::connect(stepping, VERSION).map_err(ConnectFailure::from_connect)
    }
}

/// Run one command line against a fixture, returning its exit code and streams.
fn run(args: &[&str], connection: impl Connection) -> (u8, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = gnoblinctl::run(
        args.iter().map(|arg| OsString::from(*arg)),
        connection,
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8(out).expect("the command writes UTF-8"),
        String::from_utf8(err).expect("the command writes UTF-8"),
    )
}

/// A connection nothing is listening at, for the commands that never connect.
fn nowhere() -> SocketConnection {
    SocketConnection::at(scratch("absent.sock"))
}

/// Two windows and one object of another type, for the tests that only read.
fn two_windows() -> (Graph, ObjectId, ObjectId) {
    let mut graph = Graph::new(catalog());
    let first = graph
        .create(TypeKey::gnoblin("window"), window_properties("First"))
        .expect("window is in the schema");
    let second = graph
        .create(TypeKey::gnoblin("window"), window_properties("Second"))
        .expect("window is in the schema");
    let mut session = Fields::new();
    session.insert("version".to_owned(), Value::Text("0.2.0".to_owned()));
    // Every slot starts with no provider, which is the schema's fail-closed default.
    for slot in [
        "slot_auth_polkit",
        "slot_prompt_secret",
        "slot_location_agent",
        "slot_idle_policy",
    ] {
        session.insert(slot.to_owned(), Value::Text("none".to_owned()));
    }
    graph
        .create(TypeKey::gnoblin("session"), session)
        .expect("session is in the schema");
    (graph, first, second)
}

#[test]
fn a_successful_list_exits_0() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["window", "list"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.lines().count(), 2, "only the two windows are listed:\n{out}");
    assert!(out.contains("title=\"First\""), "{out}");
    assert!(out.contains("title=\"Second\""), "{out}");
    assert!(!out.contains("gnoblin.session"), "the session is another type:\n{out}");
}

#[test]
fn list_prints_json_for_a_script() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["--json", "window", "list"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    let document: serde_json::Value = serde_json::from_str(&out).expect("one JSON document");
    let objects = document["objects"].as_array().expect("an array of objects");
    assert_eq!(objects.len(), 2);
    assert_eq!(objects[0]["type"], "gnoblin.window");
    assert_eq!(objects[0]["properties"]["title"], "First");
    assert_eq!(objects[0]["properties"]["focused"], false);
}

#[test]
fn get_prints_one_property_as_its_bare_value() {
    let (graph, first, _) = two_windows();
    let id = first.get().to_string();
    let (code, out, err) = run(
        &["window", "get", &id, "title"],
        ModelConnection::new(graph, Vec::new()),
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(
        out, "\"First\"\n",
        "a text value keeps its quotes so an empty one is visible"
    );
}

#[test]
fn get_of_an_unknown_identity_is_an_error_response() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["window", "get", "99"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 1, "an identity no object has is an error, not a usage mistake");
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("object-unknown"), "{err}");
}

#[test]
fn ping_exits_0_once_the_handshake_has_succeeded() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["ping"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "pong\n");
}

#[test]
fn version_reports_the_session_and_the_ipc_version() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["version"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "gnoblin 0.2.0\nipc 0.2.0\n");
}

#[test]
fn version_as_json_names_both_versions() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["--json", "version"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    let document: serde_json::Value = serde_json::from_str(&out).expect("one JSON document");
    assert_eq!(document["gnoblin"], "0.2.0");
    assert_eq!(document["ipc"], "0.2.0");
}

#[test]
fn status_reports_the_revision_and_an_unavailable_lock() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["status"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("state     running\n"), "{out}");
    assert!(out.contains("revision  "), "{out}");
    assert!(
        out.contains("lock      unavailable\n"),
        "a compositor with no lock object is not unlocked:\n{out}"
    );
}

#[test]
fn status_as_json_marks_the_lock_unavailable_not_unlocked() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["--json", "status"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    let document: serde_json::Value = serde_json::from_str(&out).expect("one JSON document");
    assert_eq!(document["state"], "running");
    assert_eq!(document["lock_available"], false);
    assert!(document["lock_state"].is_null(), "{out}");
}

#[test]
fn capabilities_lists_every_capability_and_grants_none_on_the_ordinary_socket() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["capabilities"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.starts_with("capability"), "{out}");
    assert!(out.contains("offer-request    no\n"), "{out}");
    assert!(out.contains("commit-config    no\n"), "{out}");
    assert!(
        out.contains("register-layer   no\n"),
        "the name column is padded to the longest name:\n{out}"
    );
    assert!(!out.contains("yes"), "the loopback grants nothing:\n{out}");
}

#[test]
fn capabilities_as_json_is_one_row_per_capability() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["--json", "capabilities"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(code, 0, "stderr: {err}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&out).expect("one JSON array");
    assert_eq!(rows.len(), 11);
    assert_eq!(rows[0]["name"], "offer-request");
    assert_eq!(rows[0]["granted"], false);
    assert_eq!(rows[4]["name"], "portal-input");
    assert_eq!(rows[5]["name"], "register-layer");
    assert_eq!(rows[6]["name"], "retrieve-secret");
    assert_eq!(rows[7]["name"], "read-config");
    assert_eq!(rows[8]["name"], "reload-config");
    assert_eq!(rows[9]["name"], "window-control");
    assert_eq!(rows[10]["name"], "capture-screen");
}

#[test]
fn watch_prints_diffs_in_order() {
    let (graph, first, _) = two_windows();
    let steps = vec![
        Step::Set(first, "title", Value::Text("Second".to_owned())),
        Step::Set(first, "title", Value::Text("Third".to_owned())),
    ];
    let (code, out, err) = run(&["window", "watch", "--count", "2"], ModelConnection::new(graph, steps));
    assert_eq!(code, 0, "stderr: {err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].ends_with("changed 1 title = \"Second\""), "{out}");
    assert!(lines[1].ends_with("changed 1 title = \"Third\""), "{out}");
}

#[test]
fn watching_an_unknown_identity_is_an_error_response() {
    let (graph, _, _) = two_windows();
    let (code, _, err) = run(&["window", "watch", "99"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(
        code, 1,
        "an identity is never reused, so watching it cannot wait for anything"
    );
    assert!(err.contains("object-unknown"), "{err}");
}

#[test]
fn a_closed_connection_ends_a_watch() {
    let (graph, _, _) = two_windows();
    let (code, out, err) = run(&["window", "watch"], ModelConnection::new(graph, Vec::new()));
    assert_eq!(
        code, 1,
        "a watch that cannot read again fails instead of blocking forever"
    );
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("closed the connection"), "{err}");
}

#[test]
fn a_call_returns_its_operation_result() {
    let (graph, first, _) = two_windows();
    let id = first.get().to_string();
    let steps = vec![Step::Complete(OperationOutcome::Done)];
    let (code, out, err) = run(&["window", "call", &id, "focus"], ModelConnection::new(graph, steps));
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "operation 1: done\n");
}

#[test]
fn a_failed_operation_exits_1() {
    let (graph, first, _) = two_windows();
    let id = first.get().to_string();
    let steps = vec![Step::Complete(OperationOutcome::Failed(ErrorCode::from_static(
        "object-gone",
    )))];
    let (code, out, err) = run(
        &["--json", "window", "call", &id, "focus"],
        ModelConnection::new(graph, steps),
    );
    assert_eq!(code, 1);
    assert!(out.contains("\"outcome\":\"failed\""), "{out}");
    assert!(out.contains("\"code\":\"object-gone\""), "{out}");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn an_error_response_exits_1() {
    let path = scratch("error.sock");
    let listener = UnixListener::bind(&path).expect("a listener in the temporary directory");
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one client");
        let mut grants_writer = stream.try_clone().expect("a duplicated socket");
        let mut transport = UnixTransport::from_stream(stream).expect("a transport");
        let hello = transport.receive_handshake().expect("the client says hello");
        transport
            .send_handshake(&Handshake::accept(hello.version))
            .expect("the handshake is answered");
        gnoblin_proto::write_grants(&mut grants_writer, &[]).expect("the grants frame after the accept");
        let request = transport
            .receive(&mut Vec::new())
            .expect("one request")
            .expect("the client sends a request");
        let refusal = Envelope::error(
            &request.namespace,
            &request.member,
            request.request_id,
            "unknown-member",
            "this connection serves nothing",
        );
        transport.send(&refusal, &[]).expect("the refusal is sent");
    });
    let (code, out, err) = run(&["window", "list"], SocketConnection::at(&path));
    server.join().expect("the server thread finished");
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 1, "an error frame is exit 1, not a connection failure");
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("unknown-member"), "{err}");
}

#[test]
fn a_missing_socket_is_a_connection_error() {
    let path = scratch("missing.sock");
    let (code, out, err) = run(&["window", "list"], SocketConnection::at(&path));
    assert_eq!(code, 3, "nothing is listening, so this is not an error response");
    assert!(out.is_empty(), "{out}");
    assert!(err.contains(&path.display().to_string()), "the path is named: {err}");
}

#[test]
fn a_version_mismatch_exits_4() {
    let path = scratch("mismatch.sock");
    let listener = UnixListener::bind(&path).expect("a listener in the temporary directory");
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("one client");
        let mut transport = UnixTransport::from_stream(stream).expect("a transport");
        transport.receive_handshake().expect("the client says hello");
        transport
            .send_handshake(&Handshake::refuse(Version::new(0, 3, 0)))
            .expect("the refusal is sent");
    });
    let (code, out, err) = run(&["window", "list"], SocketConnection::at(&path));
    server.join().expect("the server thread finished");
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 4, "a version disagreement is its own exit code");
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("0.3.0"), "the compositor's version is named: {err}");
    assert!(err.contains("0.2.0"), "this build's version is named: {err}");
}

#[test]
fn bad_arguments_exit_2() {
    for args in [
        vec!["window", "get", "1", "nosuch"],
        vec!["window", "get", "not-a-number"],
        vec!["window", "call", "1", "nosuch"],
        vec!["window", "call", "1", "focus", "title=First"],
        vec!["window", "call", "1", "focus", "no-equals"],
        vec!["window", "list", "extra"],
        vec!["ipc"],
        vec!["nosuch"],
    ] {
        let (code, out, err) = run(&args, nowhere());
        assert_eq!(code, 2, "{args:?} should be a usage error, stderr: {err}");
        assert!(out.is_empty(), "{args:?} wrote to standard output: {out}");
        assert!(!err.is_empty(), "{args:?} said nothing");
    }
}

#[test]
fn a_missing_method_parameter_exits_2() {
    let (code, _, err) = run(&["request", "call", "1", "claim"], nowhere());
    assert_eq!(code, 2);
    assert!(err.contains("needs `request`"), "{err}");
}

#[test]
fn help_exits_0() {
    let (code, out, err) = run(&["--help"], nowhere());
    assert_eq!(code, 0);
    assert!(out.contains("Usage: gnoblinctl"), "{out}");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn completions_are_written_for_every_shell() {
    for shell in ["bash", "zsh", "fish"] {
        let (code, out, err) = run(&["completions", shell], nowhere());
        assert_eq!(code, 0, "{shell}: {err}");
        assert!(out.contains("gnoblinctl"), "{shell} script names the command:\n{out}");
        assert!(
            out.contains("window"),
            "{shell} script completes a schema object:\n{out}"
        );
    }
}

#[test]
fn completions_can_be_written_to_a_file() {
    let path = scratch("completions.bash");
    let (code, _, err) = run(
        &["completions", "bash", "--output", &path.display().to_string()],
        nowhere(),
    );
    assert_eq!(code, 0, "{err}");
    let script = std::fs::read_to_string(&path).expect("the script was written");
    assert!(script.contains("_gnoblinctl"), "{script}");
    let _ = std::fs::remove_file(&path);
}

/// The payload written into the sensitive frame, which the dump must never show.
const SECRET: &[u8] = b"hunter2";

/// A recorded stream: a handshake, a plain event, and a frame whose schema member
/// is secret.
///
/// The stock `request.answer` member is the secret one: its `answer` field carries a
/// password, so the schema flags every frame that carries it. The assertion that the
/// frame is flagged is what keeps this test from passing without a secret in the
/// stream.
fn record_frames(path: &Path) {
    let mut file = std::fs::File::create(path).expect("a recording");
    file.write_all(&Handshake::hello(VERSION).encode())
        .expect("the handshake is written");
    let plain = Envelope::event("gnoblin", "object.changed", vec![1, 2, 3]);
    assert!(!plain.is_sensitive(), "a plain member is not flagged");
    write_frame(&mut file, &plain).expect("the plain frame is written");
    let secret = Envelope::request("gnoblin", "request.answer", 7, SECRET.to_vec());
    assert!(secret.is_sensitive(), "the schema flags `request.answer`");
    write_frame(&mut file, &secret).expect("the secret frame is written");
}

#[test]
fn a_sensitive_frame_is_never_printed() {
    let path = scratch("frames.bin");
    record_frames(&path);
    let recorded = std::fs::read(&path).expect("the recording is readable");
    assert!(
        recorded.windows(SECRET.len()).any(|window| window == SECRET),
        "the recording does not carry the secret, so the assertions below would prove nothing"
    );
    let (code, out, err) = run(&["ipc", "dump", &path.display().to_string()], nowhere());
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "{err}");
    assert!(
        !out.contains("hunter2"),
        "the payload of a sensitive frame was printed:\n{out}"
    );
    assert!(
        out.contains("handshake hello 0.2.0"),
        "the handshake is decoded:\n{out}"
    );
    assert!(out.contains("event    gnoblin.object.changed"), "{out}");
    assert!(
        out.contains("request  gnoblin.request.answer #7 [redacted]"),
        "the sensitive frame keeps its routing fields and loses everything else:\n{out}"
    );
}

#[test]
fn a_truncated_recording_is_reported() {
    let path = scratch("truncated.bin");
    std::fs::write(&path, [0u8, 0, 0, 9, 1, 2]).expect("a short recording");
    let (code, out, err) = run(&["ipc", "dump", &path.display().to_string()], nowhere());
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 1);
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("ended inside a frame"), "{err}");
}

#[test]
fn the_command_tree_matches_the_schema() {
    let catalog = catalog();
    let tree = gnoblinctl::command_tree();

    let mut objects: Vec<(String, &gnoblin_schema::ObjectType)> = Vec::new();
    for schema in catalog.schemas() {
        for object in &schema.objects {
            objects.push((schema.namespace.to_string(), object));
        }
    }
    let mut counts = std::collections::BTreeMap::new();
    for (_, object) in &objects {
        *counts.entry(object.name.as_str()).or_insert(0usize) += 1;
    }

    let mut expected: Vec<String> = Vec::new();
    for (namespace, object) in &objects {
        let command = if counts[object.name.as_str()] <= 1 {
            object.name.clone()
        } else {
            let qualifier = namespace.rsplit('.').next().expect("a namespace has a segment");
            format!("{qualifier}-{}", object.name)
        };
        let subcommand = tree.find_subcommand(&command).unwrap_or_else(|| {
            panic!(
                "the schema declares `{}` but there is no `gnoblinctl {command}`",
                object.name
            )
        });

        let mut actions: Vec<String> = subcommand
            .get_subcommands()
            .map(|action| action.get_name().to_owned())
            .filter(|name| name != "help")
            .collect();
        actions.sort();
        // A command that is also a hand-written verb carries those verbs beside the object's.
        let verbs: &[&str] = match command.as_str() {
            "config" => &["get", "reload", "validate"],
            _ => &[],
        };
        let mut want = vec!["list".to_owned(), "watch".to_owned()];
        want.extend(verbs.iter().map(|verb| (*verb).to_owned()));
        if !object.properties.is_empty() {
            want.push("get".to_owned());
        }
        if !object.methods.is_empty() {
            want.push("call".to_owned());
        }
        want.sort();
        assert_eq!(actions, want, "the actions of `gnoblinctl {command}`");

        if let Some(get) = subcommand
            .find_subcommand("get")
            .filter(|_| !object.properties.is_empty())
        {
            let properties = get
                .get_arguments()
                .find(|argument| argument.get_id() == "property")
                .expect("`get` takes a property")
                .get_possible_values()
                .iter()
                .map(|value| value.get_name().to_owned())
                .collect::<Vec<String>>();
            let declared: Vec<String> = object.properties.iter().map(|property| property.name.clone()).collect();
            assert_eq!(properties, declared, "`gnoblinctl {command} get` offers every property");
        }
        if let Some(call) = subcommand
            .find_subcommand("call")
            .filter(|_| !object.methods.is_empty())
        {
            let methods = call
                .get_arguments()
                .find(|argument| argument.get_id() == "method")
                .expect("`call` takes a method")
                .get_possible_values()
                .iter()
                .map(|value| value.get_name().to_owned())
                .collect::<Vec<String>>();
            let declared: Vec<String> = object.methods.iter().map(|method| method.name.clone()).collect();
            assert_eq!(methods, declared, "`gnoblinctl {command} call` offers every method");
        }
        expected.push(command);
    }

    let mut present: Vec<String> = tree
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_owned())
        .filter(|name| {
            !HAND_WRITTEN.contains(&name.as_str())
                && name != "ipc"
                && name != "completions"
                && name != "log"
                && name != "help"
        })
        .collect();
    present.sort();
    expected.sort();
    assert_eq!(
        present, expected,
        "every subcommand comes from the schema and every object has one"
    );

    assert!(
        tree.get_arguments().any(|argument| argument.get_id() == "json"),
        "`--json` is the one flag the tree adds to the schema"
    );
}

/// The schema's `config` object and the hand-written `config` verbs are one command. Clap
/// resolves a repeated subcommand name to its first occurrence, so two `config` commands hide
/// every verb of the second: `config validate` then fails as an unknown subcommand, exit 2.
#[test]
fn no_root_subcommand_name_is_declared_twice() {
    let tree = gnoblinctl::command_tree();
    let mut names: Vec<&str> = tree.get_subcommands().map(clap::Command::get_name).collect();
    let declared = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        declared,
        "a repeated root subcommand shadows its later declarations"
    );
}

#[test]
fn config_carries_the_schema_object_and_every_hand_written_verb() {
    let tree = gnoblinctl::command_tree();
    let config: Vec<&clap::Command> = tree
        .get_subcommands()
        .filter(|command| command.get_name() == "config")
        .collect();
    assert_eq!(config.len(), 1, "`config` must be declared once");
    let verbs: Vec<&str> = config[0].get_subcommands().map(clap::Command::get_name).collect();
    for expected in ["get", "reload", "validate", "list", "watch"] {
        assert!(
            verbs.contains(&expected),
            "`config {expected}` is missing; `config` has {verbs:?}"
        );
    }
}

#[test]
fn the_wire_version_is_the_schema_version() {
    let catalog = catalog();
    let namespace = gnoblin_schema::Namespace::parse("gnoblin").expect("a valid namespace");
    let declared = catalog.version(&namespace).expect("the core namespace is declared");
    assert_eq!(
        declared,
        gnoblin_schema::Version::new(0, 2, 0),
        "the generated tree speaks the schema's version"
    );
}
