//! The JavaScript role's duties through the in-process path: the configuration document it commits, the revision it answers
//! `gnoblinctl config reload` with, the refusal of a reload that cannot load, and the reload a script asks for itself.
//!
//! The session is `gnoblin ./shell.ts` with the file watcher off (`GNOBLIN_JS_WATCH=0`), so a file change takes effect only when a
//! reload is asked for, as the owner does with `gnoblinctl config reload` (the same `config.reload` verb, sent here on an owner
//! connection) or as a script does with `gnoblin.runtime.reloadConfig()`. What is asserted is what the compositor holds: its
//! revision and document, read back on a second connection.

use std::fs;
use std::time::{Duration, Instant};

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError};
use gnoblin_proto::Version;

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Start, start_script};
use crate::{save, serving, support};

/// The shell is started with the watcher off so that only a request reloads.
const NO_WATCH: [(&str, &str); 1] = [("GNOBLIN_JS_WATCH", "0")];

fn owner(session: &Gnoblin) -> Client<UnixTransport> {
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Client::connect(
        UnixTransport::connect(&session.ipc()).expect("the session listens"),
        Version::new(major, minor, patch),
    )
    .expect("the handshake")
}

/// The revision the compositor holds and its document, or `None` while it holds none.
fn accepted(session: &Gnoblin) -> Option<(u64, serde_json::Value)> {
    let (revision, bytes) = owner(session).configuration().expect("the read")?;
    Some((revision, serde_json::from_slice(&bytes).expect("the document is JSON")))
}

/// `compositor.user-shaders` of the accepted document once the revision in force is `revision`.
fn shaders_at(session: &Gnoblin, revision: u64) -> Option<bool> {
    let (current, document) = accepted(session)?;
    if current != revision {
        return None;
    }
    document["compositor"]["user-shaders"].as_bool()
}

#[track_caller]
fn wait_shaders(session: &mut Gnoblin, revision: u64, enabled: bool) {
    session.wait_for(
        &format!("revision {revision} holds user-shaders {enabled}"),
        |session| shaders_at(session, revision) == Some(enabled),
    );
}

/// A shell that writes the compositor's `user_shaders` setting, which the document carries as `compositor.user-shaders`. The session is
/// headless and has no input device, and a script cannot make one (a device is reported by the seat), so the setting is one that belongs
/// to no device.
fn shell_with_shaders(enabled: bool) -> String {
    format!(
        "import {{ gnoblin }} from \"gnoblin-core\";\n\
         \n\
         gnoblin.compositor.userShaders = {enabled};\n"
    )
}

/// A shell that asks for the reload itself when a window opens, so the request starts inside the shell and no `gnoblinctl` is involved.
fn shell_that_reloads(enabled: bool) -> String {
    format!(
        "{}\ngnoblin.clients.addListener({{\n    onAdded() {{\n        gnoblin.runtime.reloadConfig();\n    }},\n}});\n",
        shell_with_shaders(enabled)
    )
}

fn start(shell_text: &str) -> (Gnoblin, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a directory for the shell");
    let shell = dir.path().join("shell.ts");
    fs::write(&shell, shell_text).expect("the shell");
    let mut session = start_script(Start::Compositor, &shell, &NO_WATCH);
    serving(&mut session);
    (session, dir)
}

#[test]
fn the_shell_commits_its_document_as_revision_one() {
    let (mut session, _dir) = start(&shell_with_shaders(false));
    wait_shaders(&mut session, 1, false);
}

#[test]
#[ignore = "vrnw: needs config.reload answered by the native runtime: the owner's reload_config() is refused with reload-timeout (the runtime did not answer the reload before its deadline)"]
fn config_reload_commits_the_file_and_answers_with_the_revision() {
    let (mut session, dir) = start(&shell_with_shaders(false));
    wait_shaders(&mut session, 1, false);
    save(&dir.path().join("shell.ts"), &shell_with_shaders(true));
    let committed = owner(&session)
        .reload_config()
        .expect("the reload is answered with its revision");
    assert_eq!(committed.revision, 2);
    wait_shaders(&mut session, 2, true);
}

#[test]
#[ignore = "vrnw: needs config.reload answered by the native runtime: the owner's reload_config() is refused with code reload-timeout, not config-rejected"]
fn config_reload_of_a_file_that_does_not_load_is_refused_with_why_and_the_accepted_document_stays() {
    let (mut session, dir) = start(&shell_with_shaders(false));
    wait_shaders(&mut session, 1, false);
    save(&dir.path().join("shell.ts"), "export const broken: number = ;\n");
    let error = owner(&session)
        .reload_config()
        .expect_err("there is nothing loadable to reload, and the caller is told so");
    let ClientError::Refused { code, message, .. } = error else {
        panic!("expected a refusal, got {error:?}");
    };
    assert_eq!(code, "config-rejected");
    assert!(message.contains("shell.ts"), "the refusal names the file: {message}");
    assert_eq!(shaders_at(&session, 1), Some(false), "revision 1 is still in force");
    // The shell that serves stays, and a fixed file commits as the next revision.
    save(&dir.path().join("shell.ts"), &shell_with_shaders(true));
    assert_eq!(
        owner(&session).reload_config().expect("a fixed file reloads").revision,
        2
    );
    wait_shaders(&mut session, 2, true);
}

#[test]
#[ignore = "vrnw: needs config.reload answered by the native runtime: gnoblin.runtime.reloadConfig() raises the request and nothing commits revision 2 (revision 2 never holds user-shaders true)"]
fn a_script_that_reloads_the_configuration_commits_what_the_file_says_now() {
    let (mut session, dir) = start(&shell_that_reloads(false));
    wait_shaders(&mut session, 1, false);
    save(&dir.path().join("shell.ts"), &shell_that_reloads(true));
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("reload.test", "reload");
    // The second revision carries the setting the file holds when the window opens, not the one it held at login.
    wait_shaders(&mut session, 2, true);
}

#[test]
fn the_starter_shell_loads_as_a_user_gets_it_and_commits_a_document() {
    // `gnoblinctl config init` writes this file, and the packaged default is the same text: a starter that fails tells every new user the
    // session is broken before they have changed anything.
    let (session, _dir) = start(support::STARTER_CONFIG);
    let deadline = Instant::now() + Duration::from_secs(60);
    while accepted(&session).is_none() {
        assert!(Instant::now() < deadline, "the starter's document was never committed");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        session.log_text().lines().all(|line| !line.contains("ERROR")),
        "{}",
        session.log_text()
    );
}
