//! The `store` object serves a privileged role's own key-value namespace, so the verbs reach
//! exactly what the owner wrote and nothing else. The tests pin four behaviours:
//!
//! 1. **A privileged role's writes persist in its own namespace.** `put`, `get`, `list` and
//!    `delete` round-trip over IPC, and the value is on disk under the role's name:
//!    `a_privileged_role_writes_its_own_namespace`.
//! 2. **A connection with no owner gets no store.** A socket connection is not attributed to a
//!    role, so the verb is refused before the graph sees it: `a_connection_with_no_owner_is_refused`.
//! 3. **A store that did not open says so.** The verbs complete `store-unavailable` rather than
//!    pretending the write happened: `a_store_that_did_not_open_refuses_verbs`.
//! 4. **A typed store error reaches the caller.** An oversize value completes with the store's
//!    own code, not a generic failure: `an_oversize_value_completes_with_its_typed_code`.

mod support;

use std::os::unix::net::UnixStream;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError};
use gnoblin_model::{Fields, Filter, Operation, OperationOutcome, Value};
use gnoblin_store::Store;
use gnoblin_types::{ErrorCode, ObjectId, Version};

use support::{Session, private_directory};

/// The version this build serves.
fn version() -> Version {
    gnoblin_compositor::version()
}

/// The fields a `store` verb takes, keyed by parameter name.
fn fields(pairs: &[(&str, Value)]) -> Fields {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// The `services` role's client, taken from the streams the session handed out.
fn services_client(ends: &mut Vec<(String, UnixStream)>) -> Client<UnixTransport> {
    let (role, stream) = ends.remove(0);
    assert_eq!(role, "services", "the test grants the services role");
    Client::connect(UnixTransport::from_stream(stream).expect("a transport"), version()).expect("the handshake")
}

/// The store object's id, found in the subscription snapshot the client reads first.
fn store_object(client: &mut Client<UnixTransport>) -> ObjectId {
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    snapshot
        .objects()
        .iter()
        .find(|view| view.ty().to_string() == "gnoblin.store")
        .map(|view| view.id())
        .expect("the store object is published")
}

/// Calls `method` on the store and waits for the outcome the compositor completes it with.
fn verb(client: &mut Client<UnixTransport>, store: ObjectId, method: &str, arguments: &Fields) -> OperationOutcome {
    let operation: Operation = client.call(store, method, arguments).expect("the verb is served");
    client.await_operation(operation).expect("the completion")
}

#[test]
fn a_privileged_role_writes_its_own_namespace() {
    let data = private_directory();
    let (session, mut ends) = Session::start_privileged_configured(&[("services", Vec::new())], |config| {
        config.kv_store = Some(Store::open(data.path()).expect("the store opens"));
    });
    let mut client = services_client(&mut ends);
    let store = store_object(&mut client);

    assert_eq!(
        verb(
            &mut client,
            store,
            "put",
            &fields(&[("key", text("greeting")), ("value", text("hello"))])
        ),
        OperationOutcome::Done
    );
    assert_eq!(
        verb(&mut client, store, "get", &fields(&[("key", text("greeting"))])),
        OperationOutcome::Value(text("hello"))
    );
    assert_eq!(
        verb(
            &mut client,
            store,
            "keys",
            &fields(&[("prefix", text("")), ("after", text("")), ("limit", Value::Uint(10))])
        ),
        OperationOutcome::Value(Value::List(vec![text("greeting")]))
    );
    assert_eq!(
        verb(&mut client, store, "delete", &fields(&[("key", text("greeting"))])),
        OperationOutcome::Done
    );
    assert_eq!(
        verb(&mut client, store, "get", &fields(&[("key", text("greeting"))])),
        OperationOutcome::Done,
        "a deleted key reads as absent"
    );

    // Write one more value, then stop the session so the file is closed before it is read.
    assert_eq!(
        verb(
            &mut client,
            store,
            "put",
            &fields(&[("key", text("kept")), ("value", text("on disk"))])
        ),
        OperationOutcome::Done
    );
    drop(client);
    drop(session);

    let reopened = Store::open(data.path()).expect("the store reopens");
    let owned = reopened.namespace("services").expect("the services namespace");
    let entry = owned.get("kept").expect("the read").expect("the value is on disk");
    assert_eq!(entry.value, text("on disk"));
}

#[test]
fn a_connection_with_no_owner_is_refused() {
    let data = private_directory();
    let (session, _ends) = Session::start_privileged_configured(&[("services", Vec::new())], |config| {
        config.kv_store = Some(Store::open(data.path()).expect("the store opens"));
    });
    let mut client = Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        version(),
    )
    .expect("the handshake");
    let store = store_object(&mut client);

    match client.call(store, "put", &fields(&[("key", text("k")), ("value", text("v"))])) {
        Err(ClientError::Refused { code, .. }) => assert_eq!(code.as_str(), "no-store-owner"),
        other => panic!("an unowned connection must not reach a store: {other:?}"),
    }
}

#[test]
fn a_store_that_did_not_open_refuses_verbs() {
    let (session, mut ends) = Session::start_privileged_configured(&[("services", Vec::new())], |_| {});
    let mut client = services_client(&mut ends);
    let store = store_object(&mut client);

    assert_eq!(
        verb(
            &mut client,
            store,
            "put",
            &fields(&[("key", text("k")), ("value", text("v"))])
        ),
        OperationOutcome::Failed(ErrorCode::from_static("store-unavailable"))
    );
    drop(client);
    drop(session);
}

#[test]
fn an_oversize_value_completes_with_its_typed_code() {
    let data = private_directory();
    let (session, mut ends) = Session::start_privileged_configured(&[("services", Vec::new())], |config| {
        config.kv_store = Some(Store::open(data.path()).expect("the store opens"));
    });
    let mut client = services_client(&mut ends);
    let store = store_object(&mut client);
    let oversize = "x".repeat(70 * 1024);

    assert_eq!(
        verb(
            &mut client,
            store,
            "put",
            &fields(&[("key", text("big")), ("value", text(&oversize))])
        ),
        OperationOutcome::Failed(ErrorCode::from_static("value-too-large"))
    );
    drop(client);
    drop(session);
}
