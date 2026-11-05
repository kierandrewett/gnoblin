//! The extension grants a real compositor announces after every accept, read by a real
//! client, and the Lua `require` gate exercised against those connections.
//!
//! Nothing here is a mock: the compositor is a running session on its own sockets, each
//! client is a `Client` over a `UnixTransport`, and the Lua host runs a real config file.

mod support;

use std::fs;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::version;
use gnoblin_config::SearchPaths;
use gnoblin_lua::{ExtensionRegistry, Host, Limits};
use gnoblin_proto::{Grant, Version};
use support::Session;

/// The read deadline a role's client carries, the same one `gnoblin-lua`'s role uses.
const HOST_PUMP: Duration = Duration::from_millis(100);

/// The neutral fixture the extension tests read. The grant and the registry entry both
/// name the namespace it declares.
const FIXTURE: &str = include_str!("../../gnoblin-schema/tests/fixtures/extensions/example-ext.toml");
const NAMESPACE: &str = "example.ext";
const CLASSES: &[&str] = &["probe", "counter"];

/// The grant for the fixture's namespace, checked against the fixture so the two cannot
/// drift apart.
fn example_grant() -> Grant {
    assert!(
        FIXTURE.contains(&format!("namespace = \"{NAMESPACE}\"")),
        "the fixture declares the namespace the tests grant"
    );
    assert!(
        FIXTURE.contains("version = \"0.1.0\""),
        "the fixture declares the version the tests grant"
    );
    Grant {
        namespace: NAMESPACE.to_owned(),
        version: Version::new(0, 1, 0),
    }
}

/// A real client over the session's IPC socket: a core connection, which is granted
/// nothing.
fn core_client(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        version(),
    )
    .expect("the handshake")
}

/// A real client over one role's privileged end. The guardian hands each end out once.
fn role_client(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Client<UnixTransport> {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    let (_, stream) = ends.remove(index);
    Client::connect(UnixTransport::from_stream(stream).expect("a transport"), version()).expect("the handshake")
}

/// Runs `text` as the configuration over `client` with `registry` loaded, and returns the
/// one script error it reported.
fn require_error(mut client: Client<UnixTransport>, registry: &ExtensionRegistry, text: &str) -> String {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let init = directory.path().join("init.lua");
    fs::write(&init, text).expect("a test can write a file");
    let paths = SearchPaths {
        home: None,
        config_home: None,
        override_path: Some(init),
    };
    // A host's drain returns on a read that finds nothing, which needs the deadline the
    // role sets before it connects; without one the drain waits for a frame that never comes.
    client.set_read_timeout(Some(HOST_PUMP)).expect("a read deadline");
    let mut host = Host::with_extensions(client, Limits::default(), registry).expect("the host starts");
    let report = host.run_config(&paths).expect("the run starts");
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    report.errors[0].message().to_owned()
}

fn loaded_fixture() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry
        .register_extension(NAMESPACE, CLASSES)
        .expect("the fixture registers");
    registry
}

#[test]
fn the_core_socket_announces_no_grants_and_a_granted_role_announces_its_own() {
    let session = Session::start();
    assert!(
        core_client(&session).granted_extensions().is_empty(),
        "a core connection is granted nothing by default"
    );

    let (session, mut ends) =
        Session::start_privileged_granting(&[("services", vec![example_grant()]), ("runtime", Vec::new())]);
    assert!(
        core_client(&session).granted_extensions().is_empty(),
        "the core socket stays empty even when a role is granted"
    );
    let services = role_client(&mut ends, "services");
    assert_eq!(services.granted_extensions(), &[example_grant()]);
    let runtime = role_client(&mut ends, "runtime");
    assert!(runtime.granted_extensions().is_empty());
}

#[test]
fn require_is_gated_by_the_grant_the_compositor_announced() {
    let loaded = loaded_fixture();

    // Loaded and granted: both gates pass. The stock build generates no bindings for the
    // fixture, so the gate's final answer is that the table cannot be built, not a grant
    // or unknown-module refusal.
    let (_granting, mut ends) = Session::start_privileged_granting(&[("services", vec![example_grant()])]);
    let granted = role_client(&mut ends, "services");
    let message = require_error(granted, &loaded, r#"require("example.ext")"#);
    assert!(
        message.contains("loaded and granted but its object types have no generated bindings"),
        "{message}"
    );

    // Loaded but not granted: the core connection holds no grant.
    let (core, _ends) = Session::start_privileged_granting(&[("services", Vec::new())]);
    let message = require_error(core_client(&core), &loaded, r#"require("example.ext")"#);
    assert!(message.contains("extension not granted: example.ext"), "{message}");

    // Granted but not loaded: the host has no such namespace, so it is unknown.
    let (_granting, mut ends) = Session::start_privileged_granting(&[("services", vec![example_grant()])]);
    let granted = role_client(&mut ends, "services");
    let message = require_error(granted, &ExtensionRegistry::new(), r#"require("example.ext")"#);
    assert!(message.contains("unknown module `example.ext`"), "{message}");

    // Neither loaded nor granted: unknown.
    let (core, _ends) = Session::start_privileged_granting(&[("services", Vec::new())]);
    let message = require_error(core_client(&core), &ExtensionRegistry::new(), r#"require("no.such")"#);
    assert!(message.contains("unknown module `no.such`"), "{message}");
}
