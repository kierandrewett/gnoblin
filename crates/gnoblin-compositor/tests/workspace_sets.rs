//! The `workspace_set` object through the real session: `activate` completes `done` for a set
//! the session has, and completes with the named code `unknown-set` for one it does not.
//!
//! The sets are the configuration's; the headless output is the only output. Nothing is
//! mocked: the session, the socket and the window manager are real.

mod support;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_model::{Fields, Filter, ObjectId, OperationOutcome};
use gnoblin_types::{ErrorCode, WorkspaceId};
use gnoblin_wm::{WorkspaceSet, WorkspaceSetEntry};

use support::Session;

/// A set naming workspace `raw` on the primary output.
fn set(name: &str, raw: u64) -> WorkspaceSet {
    WorkspaceSet {
        name: name.to_owned(),
        entries: vec![WorkspaceSetEntry {
            workspace: WorkspaceId::new(raw),
            output: None,
        }],
    }
}

/// Starts a session whose configuration defines `sets`.
fn started(sets: Vec<WorkspaceSet>) -> Session {
    let (session, _) = Session::start_privileged_configured(&[], |config| {
        config.wayland.workspace_sets = sets;
    });
    session
}

/// An IPC client at the served version.
fn ipc(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        gnoblin_compositor::version(),
    )
    .expect("the session accepts this build")
}

/// The `workspace_set` object's identity, read from a fresh snapshot.
fn workspace_set_object(client: &mut Client<UnixTransport>) -> ObjectId {
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    snapshot
        .objects()
        .iter()
        .find(|view| view.ty().to_string() == "gnoblin.workspace_set")
        .map(|view| view.id())
        .expect("the workspace_set object")
}

/// Calls `activate` with `name` and returns how the operation finished.
fn activate(client: &mut Client<UnixTransport>, object: ObjectId, name: &str) -> OperationOutcome {
    let mut arguments = Fields::new();
    arguments.insert("name".to_owned(), gnoblin_model::Value::Text(name.to_owned()));
    let operation = client.call(object, "activate", &arguments).expect("the call is served");
    client.await_operation(operation).expect("the completion")
}

#[test]
fn activating_a_set_the_session_does_not_have_fails_with_unknown_set() {
    let session = started(vec![set("work", 2)]);
    let mut client = ipc(&session);
    let object = workspace_set_object(&mut client);
    assert_eq!(
        activate(&mut client, object, "nope"),
        OperationOutcome::Failed(ErrorCode::from_static("unknown-set")),
    );
}

#[test]
fn activating_a_defined_set_completes_done() {
    let session = started(vec![set("work", 2)]);
    let mut client = ipc(&session);
    let object = workspace_set_object(&mut client);
    assert_eq!(activate(&mut client, object, "work"), OperationOutcome::Done);
}

#[test]
fn activating_the_live_set_again_is_still_done() {
    let session = started(vec![set("work", 2)]);
    let mut client = ipc(&session);
    let object = workspace_set_object(&mut client);
    assert_eq!(activate(&mut client, object, "work"), OperationOutcome::Done);
    assert_eq!(activate(&mut client, object, "work"), OperationOutcome::Done);
}
