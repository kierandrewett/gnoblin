//! A loaded extension, used from a real configuration end to end.
//!
//! The session routes `example.ext` to a provider on its own socket. A granted role's
//! configuration is run by a real `Host` over that role's real client. The provider is a
//! thread on its own privileged socket that answers the call and emits the signal. A
//! stranger and an unknown namespace are refused by `require`, and a call after the
//! provider has gone fails with `provider-gone`.

mod support;

use std::fs;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_client::wire::{decode_fields, encode_fields};
use gnoblin_compositor::{ExtensionRoute, version};
use gnoblin_config::SearchPaths;
use gnoblin_lua::{ExtensionRegistry, Host, Limits};
use gnoblin_model::{Fields, Tick, Value};
use gnoblin_proto::{Envelope, Grant, Version};
use support::{Session, Wire};

/// The neutral fixture the extension tests read. The grant, the route and the registry
/// entry all name the namespace it declares.
const FIXTURE: &str = include_str!("../../gnoblin-schema/tests/fixtures/extensions/example-ext.toml");
const NAMESPACE: &str = "example.ext";
const PROVIDER: &str = "services";
const GRANTEE: &str = "viewer";
const STRANGER: &str = "stranger";
const DEADLINE: Duration = Duration::from_secs(5);
/// The read deadline a role's client carries while its host pumps: the one `gnoblin-lua`'s
/// role uses.
const PUMP: Duration = Duration::from_millis(100);

/// A configuration that calls the provider, checks the reply, and registers a handler that
/// reports the signal it hears through the script error channel.
const CALL_AND_LISTEN: &str = r#"
local ex = require("example.ext")
local probe = ex.probe
assert(probe.answer == nil, "the secret method is absent from the Lua object")
local reply = probe.begin{user = "ada"}
assert(reply.greeting == "hello ada", "the reply was " .. tostring(reply.greeting))
probe.on("question", function(event)
  error("heard " .. event.message, 0)
end)
"#;

fn grant() -> Grant {
    Grant {
        namespace: NAMESPACE.to_owned(),
        version: Version::new(0, 1, 0),
    }
}

fn route() -> ExtensionRoute {
    ExtensionRoute {
        namespace: NAMESPACE.to_owned(),
        provider: PROVIDER.to_owned(),
    }
}

/// `services` provides `example.ext`, `viewer` is granted it, and `stranger` holds nothing.
fn session() -> (Session, Vec<(String, UnixStream)>) {
    Session::start_privileged_extensions(
        &[(PROVIDER, Vec::new()), (GRANTEE, vec![grant()]), (STRANGER, Vec::new())],
        &[route()],
    )
}

/// Takes one role's privileged end as a frame wire, which the guardian hands out once.
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

/// A real client over one role's privileged end.
fn role_client(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Client<UnixTransport> {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    let (_, stream) = ends.remove(index);
    Client::connect(UnixTransport::from_stream(stream).expect("a transport"), version()).expect("the handshake")
}

fn loaded() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    let schema = gnoblin_schema::parse_schema("example-ext.toml", FIXTURE).expect("the fixture parses");
    registry.register_extension(schema).expect("the fixture registers");
    registry
}

/// A payload of text fields, as the provider sends.
fn text_fields(pairs: &[(&str, String)]) -> Fields {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Value::Text(value.clone())))
        .collect()
}

fn text(arguments: &Fields, key: &str) -> String {
    match arguments.get(key) {
        Some(Value::Text(text)) => text.clone(),
        other => panic!("the argument {key} is {other:?}"),
    }
}

/// Runs `text` as the configuration over `client` with `registry` loaded. Returns the host,
/// so the caller can pump it, and the messages of the errors the run reported.
fn configure(
    mut client: Client<UnixTransport>,
    registry: &ExtensionRegistry,
    text: &str,
) -> (Host<UnixTransport>, Vec<String>) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let init = directory.path().join("init.lua");
    fs::write(&init, text).expect("a test can write a file");
    let paths = SearchPaths {
        home: None,
        config_home: None,
        override_path: Some(init),
    };
    // A host's drain returns on a read that finds nothing, which needs the deadline set
    // before it connects; without one the drain waits for a frame that never comes.
    client.set_read_timeout(Some(PUMP)).expect("a read deadline");
    let mut host = Host::with_extensions(client, Limits::default(), registry).expect("the host starts");
    let report = host.run_config(&paths).expect("the run starts");
    let errors = report.errors.iter().map(|error| error.message().to_owned()).collect();
    (host, errors)
}

#[test]
fn a_granted_configuration_calls_the_provider_and_hears_its_signal() {
    let (_session, mut ends) = session();
    let mut provider = take(&mut ends, PROVIDER);
    let viewer = role_client(&mut ends, GRANTEE);

    let serving = thread::spawn(move || {
        let request = provider.receive().expect("the provider is sent the call");
        assert_eq!(request.namespace, NAMESPACE);
        assert_eq!(request.member, "probe.begin");
        let arguments =
            decode_fields(request.payload().expect("the call carries arguments")).expect("the arguments decode");
        let user = text(&arguments, "user");
        let reply = text_fields(&[("greeting", format!("hello {user}"))]);
        provider.send(&Envelope::reply(
            NAMESPACE,
            "probe.begin",
            request.request_id,
            encode_fields(&reply),
        ));
        // The reply is sent first, so the call returns on it and the signal waits for the pump.
        let signal = text_fields(&[("message", "ready?".to_owned())]);
        provider.send(&Envelope::event(NAMESPACE, "probe.question", encode_fields(&signal)));
        provider
    });

    let (mut host, errors) = configure(viewer, &loaded(), CALL_AND_LISTEN);
    assert!(
        errors.is_empty(),
        "the call and the handler registration run cleanly: {errors:#?}"
    );

    // The handler raises the signal's message as its one script error, which proves it ran
    // with the signal's own field.
    let pumped = host.pump(Tick::new(1)).expect("the pump runs");
    let heard: Vec<String> = pumped.errors.iter().map(|error| error.message().to_owned()).collect();
    assert_eq!(heard.len(), 1, "the handler ran once: {heard:#?}");
    assert!(heard[0].contains("heard ready?"), "{heard:#?}");

    let _provider = serving.join().expect("the provider served its call");
}

#[test]
fn a_stranger_is_refused_and_an_unknown_namespace_is_unknown() {
    let (_session, mut ends) = session();
    let _provider = take(&mut ends, PROVIDER);

    let stranger = role_client(&mut ends, STRANGER);
    let (_host, errors) = configure(stranger, &loaded(), r#"require("example.ext")"#);
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(
        errors[0].contains("extension not granted: example.ext"),
        "{}",
        errors[0]
    );

    let viewer = role_client(&mut ends, GRANTEE);
    let (_host, errors) = configure(viewer, &loaded(), r#"require("no.such")"#);
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(errors[0].contains("unknown module `no.such`"), "{}", errors[0]);
}

#[test]
fn a_call_after_the_provider_is_gone_fails_with_provider_gone() {
    let (_session, mut ends) = session();
    let provider = take(&mut ends, PROVIDER);
    let viewer = role_client(&mut ends, GRANTEE);
    provider.kill();

    // The compositor learns of the disconnect asynchronously, and the call waits until it
    // answers. That answer is the `provider-gone` refusal, which the script checks.
    let script = r#"
local probe = require("example.ext").probe
local ok, err = pcall(function() return probe.begin{user = "ada"} end)
assert(not ok, "a call against a gone provider must fail")
assert(string.find(tostring(err), "provider-gone", 1, true), tostring(err))
"#;
    let (_host, errors) = configure(viewer, &loaded(), script);
    assert!(errors.is_empty(), "{errors:#?}");
}
