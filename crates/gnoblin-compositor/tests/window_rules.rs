//! Window rules from the accepted configuration, end to end.
//!
//! A committed document is the only way a rule reaches the window manager: the compositor
//! validates the list at the commit, hands it to the window manager as `RulesChanged`, and
//! applies it to every window taken into management after that. These tests drive the real
//! runtime role over its socket and map real toplevels through the wayland socket, then read
//! the effect back through `gnoblinctl`.
//!
//! # Failure modes first
//!
//! 1. **An accepted rule never reaches the window manager.** The window that matches it is
//!    not maximised: `a_committed_rule_maximises_the_window_it_matches`.
//! 2. **A later commit keeps the rule in force.** A window mapped after the rule was removed
//!    still takes the rule's placement: `a_later_commit_without_the_rule_stops_it_applying`.
//! 3. **A rule the session cannot carry out is accepted and silently ignored.** The commit is
//!    refused, names the field, and records nothing:
//!    `a_rule_the_session_cannot_carry_out_is_refused_at_commit_naming_its_field`.

mod support;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client as RuntimeClient, ClientError, code};
use support::{Client, Session, wait_for, wait_for_listed_id};

/// The headless output the window manager places windows on.
const OUTPUT: (i64, i64) = (1024, 768);

fn version() -> gnoblin_types::Version {
    gnoblin_compositor::version()
}

/// A session with the runtime role granted, and the runtime's connection to it.
fn start() -> (Session, RuntimeClient<UnixTransport>) {
    let (session, mut ends) = Session::start_privileged(&["runtime"]);
    let (_role, stream) = ends.pop().expect("the runtime's end");
    let runtime = RuntimeClient::connect(UnixTransport::from_stream(stream).expect("a transport"), version())
        .expect("the runtime handshakes");
    (session, runtime)
}

/// A configuration whose only rule maximises the windows of `app_id`. The app id is a Lua
/// pattern, so `app_id` must not contain pattern characters such as `-`.
fn maximise_rule(app_id: &str) -> Vec<u8> {
    format!(r#"{{"window-rules":[{{"match":{{"app-id":"^{app_id}$","type":"window"}},"maximise":true}}]}}"#)
        .into_bytes()
}

/// One numeric property of a window, read through the IPC.
fn number(session: &Session, id: u64, property: &str) -> i64 {
    let value = session.ctl(&["window", "get", &id.to_string(), property]).ok();
    value
        .out
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("`{property}` is a number, got {:?}", value.out))
}

/// The size the window manager published for a window.
fn size(session: &Session, id: u64) -> (i64, i64) {
    (number(session, id, "width"), number(session, id, "height"))
}

/// Waits for a window to publish `expected`, and on timeout names the size that was observed.
fn wait_for_size(session: &Session, id: u64, expected: (i64, i64), what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut observed = size(session, id);
    while observed != expected && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        observed = size(session, id);
    }
    assert_eq!(observed, expected, "{what}: the window {id} published this size");
}

#[test]
fn a_committed_rule_maximises_the_window_it_matches() {
    let (session, mut runtime) = start();
    assert_eq!(
        runtime.commit_config(&maximise_rule("rule_app")).expect("the commit"),
        1,
        "the document is the first revision"
    );

    let mut app = Client::connect(session.wayland());
    let _matched = app.map_window("rule_app", "matched");
    let id = wait_for_listed_id(&session, "matched");
    wait_for_size(&session, id, OUTPUT, "the matched window fills the output");
}

#[test]
fn a_later_commit_without_the_rule_stops_it_applying() {
    let (session, mut runtime) = start();
    runtime
        .commit_config(&maximise_rule("rule_app"))
        .expect("the first commit");

    let mut app = Client::connect(session.wayland());
    let _matched = app.map_window("rule_app", "matched");
    let matched = wait_for_listed_id(&session, "matched");
    wait_for("the matched window fills the output", || {
        (size(&session, matched) == OUTPUT).then_some(())
    });

    // The second commit carries no rules, so the list the window manager holds is emptied.
    assert_eq!(
        runtime.commit_config(b"{}").expect("the second commit"),
        2,
        "the empty document is the second revision"
    );
    let _later = app.map_window("rule_app", "later");
    let later = wait_for_listed_id(&session, "later");
    wait_for("the later window has a size", || {
        let (width, height) = size(&session, later);
        (width > 0 && height > 0).then_some(())
    });
    assert_ne!(
        size(&session, later),
        OUTPUT,
        "a window mapped after the rule was removed is not maximised by it"
    );
}

#[test]
fn a_rule_the_session_cannot_carry_out_is_refused_at_commit_naming_its_field() {
    let (_session, mut runtime) = start();
    // A named workspace is not something this session can place a window on yet.
    let document = br#"{"window-rules":[{"match":{"app-id":"^rule_app$","type":"window"},"workspace":{"id":"main"}}]}"#;
    match runtime.commit_config(document) {
        Err(ClientError::Refused {
            code: refused, message, ..
        }) => {
            assert_eq!(
                refused.as_str(),
                code::CONFIG_REJECTED,
                "the commit is refused as rejected"
            );
            assert!(
                message.contains("window-rules[0].workspace.id"),
                "the refusal names the field: {message}"
            );
        }
        other => panic!("the commit should be refused, got {other:?}"),
    }
    // A refused document is never recorded, so the next valid commit is the first revision.
    assert_eq!(
        runtime.commit_config(b"{}").expect("a valid commit"),
        1,
        "the refused document did not take a revision"
    );
}
