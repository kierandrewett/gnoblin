//! The `layers` object admits only built-in providers. A socket connection holds no
//! `register-layer` capability, so `register` is refused `power-missing` before the graph sees it.
//! Without this gate any client on the socket could stack a layer over a window.

mod support;

use std::os::unix::net::UnixStream;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError};
use gnoblin_model::{Fields, Filter, Value};
use gnoblin_types::{ObjectId, Version};

use support::Session;

/// The version this build serves.
fn version() -> Version {
    gnoblin_compositor::version()
}

/// A connection the session does not attribute to any role.
fn socket_client(session: &Session) -> Client<UnixTransport> {
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

/// A real client over one role's privileged end, so the call comes from that role's
/// connection rather than an unattributed socket.
fn role_client(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Client<UnixTransport> {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    let (_, stream) = ends.remove(index);
    Client::connect(UnixTransport::from_stream(stream).expect("a transport"), version()).expect("the handshake")
}

/// Asks `client` to register a layer over the window with id 1, and requires the refusal
/// that comes before the graph sees the call.
fn register_is_refused_power_missing(client: &mut Client<UnixTransport>, layers: ObjectId, who: &str) {
    let fields: Fields = [
        ("local".to_owned(), Value::Text("badge".to_owned())),
        ("target".to_owned(), Value::Uint(1)),
    ]
    .into_iter()
    .collect();

    match client.call(layers, "register", &fields) {
        Err(ClientError::Refused { code, .. }) => assert_eq!(code.as_str(), "power-missing"),
        other => panic!("{who} must not register a layer: {other:?}"),
    }
}

#[test]
fn a_socket_client_cannot_register_a_layer() {
    // The failure this prevents: an ordinary client stacks a layer above a window, so privilege
    // was never checked on the call that changes what the user sees.
    let (session, _ends) = Session::start_privileged(&["runtime", "services"]);
    let mut client = socket_client(&session);
    let layers = layers_object(&mut client);
    register_is_refused_power_missing(&mut client, layers, "an ordinary connection");
}

#[test]
fn the_runtime_role_cannot_register_a_layer() {
    // The runtime holds config and input policy, not layers. Only `services` draws the
    // keyring and polkit prompts, so a runtime connection stacking a layer is a privilege it
    // was never given.
    let (_session, mut ends) = Session::start_privileged(&["runtime", "services"]);
    let mut runtime = role_client(&mut ends, "runtime");
    let layers = layers_object(&mut runtime);
    register_is_refused_power_missing(&mut runtime, layers, "the runtime role");
}
