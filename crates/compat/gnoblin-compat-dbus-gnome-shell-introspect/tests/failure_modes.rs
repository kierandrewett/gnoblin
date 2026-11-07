//! One test per failure mode that needs no bus: the state the adapter derives from the graph.
//!
//! The interesting failures are all about a graph that says something the adapter cannot
//! honestly report, so each test builds that graph through the schema and then asserts the
//! refusal rather than a window with a blank title or a desktop of size zero.

mod support;

use gnoblin_client::Server;
use gnoblin_compat_dbus_gnome_shell_introspect::{IntrospectError, IntrospectLink, IntrospectState};
use gnoblin_model::{Fields, ObjectId, ObjectView, TypeKey, Value};
use support::{VERSION, graph, output_properties, unlocked, window_properties};

/// A server with the lock object, so a link can connect.
fn server() -> Server {
    let mut server = Server::new(graph(), VERSION);
    server
        .create(TypeKey::gnoblin("lock"), unlocked())
        .expect("the lock type is in the schema");
    server
}

#[test]
fn two_windows_cannot_share_an_identity() {
    // The failure mode: the dictionary is keyed by identity, so two windows with one identity
    // would collapse into one entry and a caller's click would reach the wrong window.
    let mut server = server();
    server
        .create(TypeKey::gnoblin("window"), window_properties(7, "One", "org.example.A"))
        .expect("a window is an object");
    server
        .create(TypeKey::gnoblin("window"), window_properties(7, "Two", "org.example.B"))
        .expect("a second window with the same identity is still an object");
    let client = server.connect(VERSION).expect("the versions match");

    match IntrospectLink::connect(client) {
        Err(IntrospectError::DuplicateWindow { id }) => assert_eq!(id, 7),
        other => panic!("a repeated identity is refused, got {other:?}"),
    }
}

#[test]
fn a_window_property_the_schema_does_not_promise_is_refused() {
    // The failure mode: a decoded frame carries a title that is not a string and the picker
    // shows the debug form of a value. The graph validates what it creates, so the only
    // producer of a mistyped value is a peer's frame — which is what an `ObjectView` is on the
    // client side.
    let mut properties = Fields::new();
    properties.insert("id".to_owned(), Value::Uint(1));
    properties.insert("title".to_owned(), Value::Uint(12));
    properties.insert("app_id".to_owned(), Value::Text("org.example.A".to_owned()));
    properties.insert("focused".to_owned(), Value::Bool(false));
    properties.insert("width".to_owned(), Value::Uint(800));
    properties.insert("height".to_owned(), Value::Uint(600));
    let view = ObjectView::new(ObjectId::new(1), TypeKey::gnoblin("window"), properties);

    match IntrospectState::read([&view]) {
        Err(IntrospectError::Unreadable { property, .. }) => assert_eq!(property, "title"),
        other => panic!("a mistyped title is refused, got {other:?}"),
    }
}

#[test]
fn a_graph_without_an_enabled_output_cannot_answer_the_screen_size() {
    // The failure mode: the adapter reports a zero-sized desktop, and a picker lays itself out
    // for a screen that does not exist.
    let mut server = server();
    server
        .create(
            TypeKey::gnoblin("output"),
            output_properties("HDMI-1", 2560, 1440, false),
        )
        .expect("a disabled output is still an object");
    let client = server.connect(VERSION).expect("the versions match");
    let link = IntrospectLink::connect(client).expect("a window list with no screen is readable");

    match link.state().screen_size() {
        Err(IntrospectError::NoScreen) => {}
        other => panic!("no enabled output is refused, got {other:?}"),
    }
}

#[test]
fn a_graph_without_a_lock_object_is_refused() {
    // The failure mode: the gate's embargo is built from a guess because nothing said whether
    // the session is locked.
    let mut server = Server::new(graph(), VERSION);
    server
        .create(
            TypeKey::gnoblin("window"),
            window_properties(1, "Editor", "org.example.A"),
        )
        .expect("a window is an object");
    let client = server.connect(VERSION).expect("the versions match");

    match IntrospectLink::connect(client) {
        Err(IntrospectError::Unreadable { object, property, .. }) => {
            assert_eq!(object, "lock");
            assert_eq!(property, "locked");
        }
        other => panic!("a missing lock object is refused, got {other:?}"),
    }
}
