//! Extensions from the accepted configuration, read into a real session at start.
//!
//! The configuration names one good extension and two bad ones. The good one routes a call
//! to its provider and is granted to a granted role, whose client announces the grant. The
//! bad ones are refused with a named reason, and the session keeps serving its core socket
//! and the good route. A store that cannot be read starts the session with no extensions.
//!
//! Nothing here is a mock: the session is the compositor on its own sockets, the store is a
//! real last-good file, and every client is a real `Client` or `Wire` over a socket.

mod support;

use std::fs;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::{configure_from_store, version};
use gnoblin_config::{Config, Store, Value};
use gnoblin_proto::{Envelope, Grant, Kind, Version};
use support::{Session, Wire};

const NAMESPACE: &str = "example.ext";
const SECOND_NAMESPACE: &str = "example.second";
const MISSING_NAMESPACE: &str = "example.missing";
const PROVIDER: &str = "services";
const GRANTEE: &str = "runtime";
/// A role the configuration grants nothing. It must be one the compositor serves — a real
/// role from the capability table — so the test observes its empty grants frame, not a
/// dropped handover. `portal` is the natural choice: it is privileged, yet the document
/// grants it no extension.
const STRANGER: &str = "portal";
const DEADLINE: Duration = Duration::from_secs(5);

fn grant() -> Grant {
    Grant {
        namespace: NAMESPACE.to_owned(),
        version: Version::new(0, 1, 0),
    }
}

/// One `extensions` entry, in the shape the configuration section reads.
fn entry(namespace: &str, schema: &str, provider: &str, grants: &[&str]) -> Value {
    Value::table([
        ("namespace", Value::string(namespace)),
        ("schema", Value::string(schema)),
        ("provider", Value::string(provider)),
        (
            "grants",
            Value::List(grants.iter().map(|role| Value::string(*role)).collect()),
        ),
    ])
}

/// The configuration the test accepts. The good entry names the neutral fixture by its
/// absolute path; `second` names a provider no privileged connection holds, so the session
/// refuses it; `missing` names a schema file that does not exist, so the loader refuses it.
fn document() -> Value {
    let good = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/tests/fixtures/extensions/example-ext.toml"
    );
    let second = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-config/tests/fixtures/extensions/second-probe.toml"
    );
    Value::table([(
        "extensions",
        Value::List(vec![
            entry(NAMESPACE, good, PROVIDER, &[GRANTEE]),
            entry(SECOND_NAMESPACE, second, "ghost", &[GRANTEE]),
            entry(MISSING_NAMESPACE, "missing.toml", PROVIDER, &[GRANTEE]),
        ]),
    )])
}

/// Accepts [`document`] from `source` into the store under `state`, as the runtime does
/// after a configuration it accepted.
fn accept(state: &Path, source: &Path) -> Store {
    fs::create_dir_all(state).expect("a state directory");
    let config = Config::from_value(document()).expect("the configuration is valid");
    let store = Store::new(state);
    store
        .save(source, &config)
        .expect("the accepted configuration is saved");
    store
}

/// Starts a session whose privileged connections are the provider, the grantee and a
/// role the configuration grants nothing, and loads the accepted configuration into it
/// the way the binary does.
fn start(root: &Path) -> (Session, Vec<(String, UnixStream)>) {
    let source = root.join("config.toml");
    let store = accept(&root.join("state"), &source);
    Session::start_privileged_configured(
        &[(PROVIDER, Vec::new()), (GRANTEE, Vec::new()), (STRANGER, Vec::new())],
        |config| configure_from_store(config, &store, &source),
    )
}

/// Takes one role's privileged end, which the guardian hands out once.
fn take(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Wire {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    let (_, stream) = ends.remove(index);
    let wire = Wire::over(stream);
    wire.set_read_timeout(DEADLINE);
    wire
}

/// A real client over one role's privileged end, for the handshake's announced grants.
fn role_client(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Client<UnixTransport> {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    let (_, stream) = ends.remove(index);
    Client::connect(UnixTransport::from_stream(stream).expect("a transport"), version()).expect("the handshake")
}

#[test]
fn a_configured_extension_routes_a_granted_call_to_its_provider() {
    let root = support::private_directory();
    let (_session, mut ends) = start(root.path());
    let mut provider = take(&mut ends, PROVIDER);
    let mut runtime = take(&mut ends, GRANTEE);

    runtime.send(&Envelope::request(NAMESPACE, "Greet", 7, b"hi".to_vec()));
    let forwarded = provider.receive().expect("the configured provider is sent the call");
    assert_eq!(forwarded.namespace, NAMESPACE);
    assert_eq!(forwarded.member, "Greet");
    provider.send(&Envelope::reply(
        NAMESPACE,
        "Greet",
        forwarded.request_id,
        b"hello".to_vec(),
    ));

    let answer = runtime.receive().expect("the caller is answered");
    assert!(matches!(answer.kind, Some(Kind::Reply(_))), "{answer:?}");
    assert_eq!(answer.request_id, 7);
    assert_eq!(answer.payload(), Some(&b"hello"[..]));
}

#[test]
fn the_configured_grant_is_announced_to_its_role_and_to_no_other() {
    let root = support::private_directory();
    let (session, mut ends) = start(root.path());

    let runtime = role_client(&mut ends, GRANTEE);
    assert_eq!(runtime.granted_extensions(), &[grant()]);
    let stranger = role_client(&mut ends, STRANGER);
    assert!(
        stranger.granted_extensions().is_empty(),
        "an ungranted role is granted nothing"
    );
    let core = Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        version(),
    )
    .expect("the handshake");
    assert!(
        core.granted_extensions().is_empty(),
        "the core socket is granted nothing"
    );
}

#[test]
fn a_refused_entry_is_named_absent_and_the_session_keeps_serving() {
    let root = support::private_directory();
    let (session, mut ends) = start(root.path());
    let mut provider = take(&mut ends, PROVIDER);
    let mut runtime = take(&mut ends, GRANTEE);

    // The entry whose provider is not a privileged connection is absent: its call is unknown.
    runtime.send(&Envelope::request(SECOND_NAMESPACE, "Greet", 3, Vec::new()));
    let refusal = runtime.receive().expect("the refused namespace is answered");
    assert_eq!(refusal.request_id, 3);
    assert_eq!(refusal.error_code(), Some("unknown-member"), "{refusal:?}");

    // The entry whose schema is missing is absent from the core socket too.
    let mut core = Wire::connect(session.socket());
    core.set_read_timeout(DEADLINE);
    core.send(&Envelope::request(MISSING_NAMESPACE, "Greet", 4, Vec::new()));
    let answer = core.receive().expect("the core socket is answered");
    assert_eq!(answer.request_id, 4);
    assert_eq!(answer.error_code(), Some("unknown-member"), "{answer:?}");

    // The good entry still routes, so the bad ones did not take the session down. The
    // provider sees the compositor's own request id, so the caller's id is checked on the reply.
    runtime.send(&Envelope::request(NAMESPACE, "Greet", 5, Vec::new()));
    let forwarded = provider.receive().expect("the good route still reaches its provider");
    assert_eq!(forwarded.member, "Greet");
    provider.send(&Envelope::reply(NAMESPACE, "Greet", forwarded.request_id, Vec::new()));
    let answer = runtime.receive().expect("the caller is answered");
    assert_eq!(answer.request_id, 5);
    assert!(matches!(answer.kind, Some(Kind::Reply(_))), "{answer:?}");
}

#[test]
fn a_store_that_cannot_be_read_starts_the_session_without_extensions() {
    let root = support::private_directory();
    let source = root.path().join("config.toml");
    let store = accept(&root.path().join("state"), &source);
    fs::write(store.path(), b"not a last-good document").expect("a test can corrupt its own store");

    let (session, _ends) =
        Session::start_privileged_configured(&[(PROVIDER, Vec::new()), (GRANTEE, Vec::new())], |config| {
            configure_from_store(config, &store, &source)
        });
    let mut core = Wire::connect(session.socket());
    core.set_read_timeout(DEADLINE);
    core.send(&Envelope::request(NAMESPACE, "Greet", 9, Vec::new()));
    let answer = core.receive().expect("the session still answers its core socket");
    assert_eq!(answer.request_id, 9);
    assert_eq!(answer.error_code(), Some("unknown-member"), "{answer:?}");
}
