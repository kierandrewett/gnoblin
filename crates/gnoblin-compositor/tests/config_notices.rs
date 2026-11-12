//! A reload that accepts a setting without applying it says so.
//!
//! The compositor applies most settings when a document is committed, and some it reads only
//! when it starts (`design/config-reload.md`). A reload that accepted such a setting and said
//! nothing would pass for a change that did not happen, so the answer to a commit and to
//! `gnoblinctl config reload` lists every one of them. These tests drive a real session with the
//! runtime's privileged connection: the runtime commits, the owner's CLI reloads.

mod support;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::wire::NoticeKind;
use gnoblin_client::{Client, Event};
use gnoblin_model::{Filter, Observation};
use support::{Command, Session, wait_for};

/// The runtime's connection to `ends`.
fn runtime(ends: &mut Vec<(String, std::os::unix::net::UnixStream)>) -> Client<UnixTransport> {
    let (_, stream) = ends.pop().expect("the runtime's end");
    Client::connect(
        UnixTransport::from_stream(stream).expect("a transport"),
        gnoblin_compositor::version(),
    )
    .expect("the handshake")
}

#[test]
fn a_commit_names_what_the_running_compositor_cannot_apply_and_what_nothing_reads() {
    // The failure this prevents: the renderer is chosen when the compositor starts, so a
    // document that changes it is accepted and the old renderer keeps drawing. The commit must
    // say so, and say it again on every later commit until the compositor restarts.
    let (_session, mut ends) = Session::start_privileged(&["runtime"]);
    let mut runtime = runtime(&mut ends);

    let first = runtime
        .commit_config_reporting(br#"{"compositor":{"renderer":"software","blur":"on"},"keyring":{"enabled":false}}"#)
        .expect("the first commit");
    let restart: Vec<&str> = first
        .notices
        .iter()
        .filter(|notice| notice.kind == NoticeKind::RestartRequired)
        .map(|notice| notice.text.as_str())
        .collect();
    assert_eq!(restart.len(), 1, "{:?}", first.notices);
    assert!(
        restart[0].starts_with("compositor.renderer: restart required"),
        "the setting is named: {restart:?}"
    );
    let unused: Vec<&str> = first
        .notices
        .iter()
        .filter(|notice| notice.kind == NoticeKind::NoEffect)
        .map(|notice| notice.text.as_str())
        .collect();
    assert_eq!(unused.len(), 1, "{:?}", first.notices);
    assert!(unused[0].starts_with("compositor.blur: no effect"), "{unused:?}");

    // The services role starts after this commit and reads what it saved, so the keyring
    // setting the first commit carried is not pending; the same document again changes
    // nothing, so the unused setting is not reported twice, but the renderer still waits.
    let again = runtime
        .commit_config_reporting(br#"{"compositor":{"renderer":"software","blur":"on"},"keyring":{"enabled":false}}"#)
        .expect("the same document again");
    assert_eq!(again.revision, first.revision + 1);
    assert_eq!(again.notices.len(), 1, "{:?}", again.notices);
    assert!(
        again.notices[0].text.starts_with("compositor.renderer"),
        "{:?}",
        again.notices
    );

    // A later change to a service's setting is the one that waits for that service to restart.
    let later = runtime
        .commit_config_reporting(br#"{"compositor":{"renderer":"software","blur":"on"},"keyring":{"enabled":true}}"#)
        .expect("a later commit");
    let texts: Vec<&str> = later.notices.iter().map(|notice| notice.text.as_str()).collect();
    assert!(
        texts
            .iter()
            .any(|text| text.starts_with("keyring.enabled: restart required")),
        "{texts:?}"
    );

    // Changing the renderer back to what the compositor started with clears it.
    let back = runtime
        .commit_config_reporting(br#"{"keyring":{"enabled":true}}"#)
        .expect("the renderer is put back");
    assert!(
        !back
            .notices
            .iter()
            .any(|notice| notice.text.starts_with("compositor.renderer")),
        "{:?}",
        back.notices
    );
}

#[test]
fn gnoblinctl_config_reload_lists_the_settings_that_were_not_applied() {
    // The failure this prevents: `gnoblinctl config reload` prints `reloaded` for a document
    // whose renderer change cannot take effect until the compositor restarts.
    let (session, mut ends) = Session::start_privileged(&["runtime"]);
    let mut runtime = runtime(&mut ends);
    runtime.subscribe(Filter::All).expect("the snapshot");

    let socket = session.socket().to_path_buf();
    let reload = std::thread::spawn(move || Command::run(&socket, &["config", "reload"]));
    wait_for("the reload request reaches the runtime", || {
        loop {
            match runtime.next_event() {
                Ok(Event::Model(Observation::Signalled { signal, .. })) if signal == "reload_requested" => {
                    return Some(());
                }
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    });
    runtime
        .commit_config(br#"{"compositor":{"renderer":"software"}}"#)
        .expect("the runtime commits");
    let output = reload.join().expect("the CLI thread");
    assert_eq!(output.code, 0, "{}", output.err);
    assert!(output.out.starts_with("reloaded: revision 1\n"), "{}", output.out);
    assert!(
        output
            .out
            .contains("not applied: compositor.renderer: restart required (the compositor reads it when it starts)"),
        "{}",
        output.out
    );

    let socket = session.socket().to_path_buf();
    let json = std::thread::spawn(move || Command::run(&socket, &["--json", "config", "reload"]));
    wait_for("the second reload request reaches the runtime", || {
        loop {
            match runtime.next_event() {
                Ok(Event::Model(Observation::Signalled { signal, .. })) if signal == "reload_requested" => {
                    return Some(());
                }
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    });
    runtime
        .commit_config(br#"{"compositor":{"renderer":"software"}}"#)
        .expect("the runtime commits");
    let output = json.join().expect("the CLI thread");
    assert_eq!(output.code, 0, "{}", output.err);
    assert!(output.out.contains("\"revision\":2"), "{}", output.out);
    assert!(
        output
            .out
            .contains("\"restart_required\":[\"compositor.renderer: restart required"),
        "{}",
        output.out
    );
}
