//! The `layers` object is served to the session owner's connection. A connection from another
//! uid is refused `denied` before the graph sees the call; that refusal is the unit test
//! `layers_register_requires_the_owner_connection` in the server, because a test socket is
//! always the owner's (same uid). These tests prove the owner's call reaches the graph with the
//! argument the schema declares.

mod support;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_model::{Fields, Filter, Value};
use gnoblin_types::{ObjectId, Version};

use support::Session;

/// The version this build serves.
fn version() -> Version {
    gnoblin_compositor::version()
}

/// A connection from the test's own uid, which is the session owner's connection.
fn owner_client(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        version(),
    )
    .expect("the handshake")
}

/// The layers object's id, found in the subscription snapshot the client reads first.
fn layers_object(client: &mut Client<UnixTransport>) -> ObjectId {
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    snapshot
        .objects()
        .iter()
        .find(|view| view.ty().to_string() == "gnoblin.layers")
        .map(|view| view.id())
        .expect("the layers object is published")
}

#[test]
fn the_owner_connection_registers_a_layer_through_the_graph() {
    // The failure this prevents: the owner's `register` is refused before the graph sees it,
    // or the graph rejects the argument the schema declares (`name`) and the call is refused
    // `unknown-member` as if the member did not exist.
    let (session, _ends) = Session::start_privileged(&["runtime", "services"]);
    let mut client = owner_client(&session);
    let layers = layers_object(&mut client);
    let fields: Fields = [
        ("name".to_owned(), Value::Text("badge".to_owned())),
        ("target".to_owned(), Value::Uint(1)),
    ]
    .into_iter()
    .collect();

    client
        .call(layers, "register", &fields)
        .expect("the graph accepts the owner's register call");
}
