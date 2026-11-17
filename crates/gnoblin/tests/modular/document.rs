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

/// `input.keyboard.delay` of the accepted document once the revision in force is `revision`.
fn delay_at(session: &Gnoblin, revision: u64) -> Option<i64> {
    let (current, document) = accepted(session)?;
    if current != revision {
        return None;
    }
    document["input"]["keyboard"]["delay"].as_i64()
}

#[track_caller]
fn wait_delay(session: &mut Gnoblin, revision: u64, delay: i64) {
    session.wait_for(&format!("revision {revision} holds delay {delay}"), |session| {
        delay_at(session, revision) == Some(delay)
    });
}

/// A shell that sets the repeat delay of a keyboard, which the document carries as `input.keyboard.delay`. The session is headless and
/// has no input device, so the script makes the keyboard it writes the setting through: a keyboard setting is the kind's and not the
/// device's.
fn shell_with_delay(delay: i64) -> String {
    format!(
        "import {{ Keyboard, gnoblin }} from \"gnoblin-core\";\n\
         \n\
         const keyboard = new (Keyboard as any)(gnoblin.input.primary, \"test\", \"Test keyboard\", [\"keyboard\"]);\n\
         keyboard.repeatDelay = {delay};\n"
    )
}

/// A shell that asks for the reload itself when a window opens, so the request starts inside the shell and no `gnoblinctl` is involved.
fn shell_that_reloads(delay: i64) -> String {
    format!(
        "{}\ngnoblin.clients.addListener({{\n    onAdded() {{\n        gnoblin.runtime.reloadConfig();\n    }},\n}});\n",
        shell_with_delay(delay)
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
    let (mut session, _dir) = start(&shell_with_delay(32));
    wait_delay(&mut session, 1, 32);
}

#[test]
fn config_reload_commits_the_file_and_answers_with_the_revision() {
    let (mut session, dir) = start(&shell_with_delay(32));
    wait_delay(&mut session, 1, 32);
    save(&dir.path().join("shell.ts"), &shell_with_delay(40));
    let committed = owner(&session)
        .reload_config()
        .expect("the reload is answered with its revision");
    assert_eq!(committed.revision, 2);
    wait_delay(&mut session, 2, 40);
}

#[test]
fn config_reload_of_a_file_that_does_not_load_is_refused_with_why_and_the_accepted_document_stays() {
    let (mut session, dir) = start(&shell_with_delay(32));
    wait_delay(&mut session, 1, 32);
    save(&dir.path().join("shell.ts"), "export const broken: number = ;\n");
    let error = owner(&session)
        .reload_config()
        .expect_err("there is nothing loadable to reload, and the caller is told so");
    let ClientError::Refused { code, message, .. } = error else {
        panic!("expected a refusal, got {error:?}");
    };
    assert_eq!(code, "config-rejected");
    assert!(message.contains("shell.ts"), "the refusal names the file: {message}");
    assert_eq!(delay_at(&session, 1), Some(32), "revision 1 is still in force");
    // The shell that serves stays, and a fixed file commits as the next revision.
    save(&dir.path().join("shell.ts"), &shell_with_delay(48));
    assert_eq!(
        owner(&session).reload_config().expect("a fixed file reloads").revision,
        2
    );
    wait_delay(&mut session, 2, 48);
}

#[test]
fn a_script_that_reloads_the_configuration_commits_what_the_file_says_now() {
    let (mut session, dir) = start(&shell_that_reloads(24));
    wait_delay(&mut session, 1, 24);
    save(&dir.path().join("shell.ts"), &shell_that_reloads(48));
    let mut client = Mapper::connect(&session.wayland());
    client.map_window("reload.test", "reload");
    // The second revision carries the delay the file holds when the window opens, not the one it held at login.
    wait_delay(&mut session, 2, 48);
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
