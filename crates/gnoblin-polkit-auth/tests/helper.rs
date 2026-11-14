//! `HelperSession` against fake `polkit-agent-helper-1` scripts.
//!
//! The scripts speak the real helper's protocol (design/js-research/polkit.md): the cookie as the first line of
//! standard input, one escaped line per PAM event on standard output, an answer per prompt, a last line of
//! `SUCCESS` or `FAILURE`. They are not the real helper: nothing here runs PAM, and no polkitd is involved.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_polkit_auth::{HelperError, HelperEvent, HelperSession};
use gnoblin_secret::SecretText;

/// A directory that is removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("gnoblin-polkit-auth-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    fn script(&self, body: &str) -> PathBuf {
        let path = self.0.join("polkit-agent-helper-1");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("the script is written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("the script is executable");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A helper that asks for a password and accepts `hunter2` for `alice` with cookie `3-cookie`.
const PASSWORD_HELPER: &str = r#"
read cookie
printf 'PAM_TEXT_INFO Place your finger on the reader\n'
printf 'PAM_PROMPT_ECHO_OFF Password: \n'
read password
if [ "$1" = alice ] && [ "$cookie" = 3-cookie ] && [ "$password" = hunter2 ]; then
    printf 'SUCCESS\n'
    exit 0
fi
printf 'PAM_ERROR_MSG Authentication failure\n'
printf 'FAILURE\n'
exit 1
"#;

/// Starts `helper`, retrying while the kernel still reports the just-written script as busy: a thread that forks
/// in the test process between the write and the close holds a copy of the descriptor for a moment.
fn start(helper: &Path, user: &str, cookie: &str) -> HelperSession {
    for _ in 0..50 {
        match HelperSession::spawn(helper, user, cookie) {
            Ok(session) => return session,
            Err(HelperError::Spawn { source, .. }) if source.raw_os_error() == Some(26) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("the helper starts: {error}"),
        }
    }
    panic!("the script stayed busy");
}

/// Everything the helper says until it reports a result.
fn until_done(session: &mut HelperSession) -> Vec<HelperEvent> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    while !session.is_done() {
        assert!(Instant::now() < deadline, "the helper did not finish: {events:?}");
        events.extend(session.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    events
}

/// Everything the helper says until it asks for an answer.
fn until_prompt(session: &mut HelperSession) -> Vec<HelperEvent> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut events = Vec::new();
    while !events.iter().any(|event| matches!(event, HelperEvent::Prompt { .. })) {
        assert!(Instant::now() < deadline, "no prompt came: {events:?}");
        events.extend(session.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    events
}

#[test]
fn a_right_password_ends_in_the_helpers_success_and_a_prompt_comes_first() {
    let scratch = Scratch::new("success");
    let helper = scratch.script(PASSWORD_HELPER);
    let mut session = start(&helper, "alice", "3-cookie");

    let asked = until_prompt(&mut session);
    assert_eq!(
        asked,
        [
            HelperEvent::Info("Place your finger on the reader".to_owned()),
            HelperEvent::Prompt {
                text: "Password: ".to_owned(),
                echo: false
            }
        ],
        "the info text comes before the prompt, as PAM sent them"
    );
    session
        .respond(&SecretText::new("hunter2"))
        .expect("the helper reads it");
    assert_eq!(until_done(&mut session), [HelperEvent::Success]);
}

#[test]
fn a_wrong_password_ends_in_failure_with_the_helpers_message() {
    let scratch = Scratch::new("wrong");
    let helper = scratch.script(PASSWORD_HELPER);
    let mut session = start(&helper, "alice", "3-cookie");
    until_prompt(&mut session);
    session
        .respond(&SecretText::new("not it"))
        .expect("the helper reads it");
    assert_eq!(
        until_done(&mut session),
        [
            HelperEvent::Error("Authentication failure".to_owned()),
            HelperEvent::Failure
        ]
    );
    assert!(session.poll().is_empty(), "nothing follows a result");
}

#[test]
fn the_cookie_is_on_standard_input_and_never_in_the_arguments() {
    let scratch = Scratch::new("cookie");
    let arguments = scratch.0.join("arguments");
    let helper = scratch.script(&format!(
        "read cookie\nprintf '%s' \"$*\" > {}\nprintf 'FAILURE\\n'",
        arguments.display()
    ));
    let mut session = start(&helper, "alice", "3-secret-cookie");
    until_done(&mut session);
    let seen = fs::read_to_string(&arguments).expect("the helper recorded its arguments");
    assert_eq!(seen, "alice", "the helper gets the user name and nothing else");
}

#[test]
fn a_helper_that_ends_without_a_result_is_a_failure() {
    let scratch = Scratch::new("dies");
    let helper = scratch.script("read cookie\nexit 3");
    let mut session = start(&helper, "alice", "c");
    assert_eq!(until_done(&mut session), [HelperEvent::Failure]);
}

#[test]
fn a_line_outside_the_protocol_is_a_failure_and_never_a_success() {
    let scratch = Scratch::new("odd");
    let helper = scratch.script("read cookie\nprintf 'AUTHENTICATED\\n'\nprintf 'SUCCESS\\n'");
    let mut session = start(&helper, "alice", "c");
    assert_eq!(
        until_done(&mut session),
        [HelperEvent::Failure],
        "the unknown line ends the session; the success after it is not read"
    );
}

#[test]
fn a_missing_helper_is_an_error_that_names_it() {
    let error = HelperSession::spawn(Path::new("/nonexistent/polkit-agent-helper-1"), "alice", "c")
        .expect_err("nothing to run");
    assert!(
        matches!(error, HelperError::Spawn { .. }),
        "the message names the path: {error}"
    );
    assert!(
        error.to_string().contains("/nonexistent/polkit-agent-helper-1"),
        "{error}"
    );
}

#[test]
fn dropping_the_session_kills_the_helper() {
    let scratch = Scratch::new("drop");
    let pid_file = scratch.0.join("pid");
    let helper = scratch.script(&format!(
        "read cookie\necho $$ > {}\nprintf 'PAM_PROMPT_ECHO_OFF Password: \\n'\nexec sleep 60",
        pid_file.display()
    ));
    let mut session = start(&helper, "alice", "c");
    until_prompt(&mut session);
    let pid: u32 = fs::read_to_string(&pid_file)
        .expect("the helper wrote its pid")
        .trim()
        .parse()
        .expect("a pid");
    assert!(Path::new(&format!("/proc/{pid}")).exists());
    drop(session);
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "the helper is killed and reaped when its session ends"
    );
}

#[test]
fn nothing_the_session_formats_holds_the_password() {
    let scratch = Scratch::new("debug");
    let helper = scratch.script(PASSWORD_HELPER);
    let mut session = start(&helper, "alice", "3-cookie");
    until_prompt(&mut session);
    let secret = SecretText::new("hunter2");
    session.respond(&secret).expect("the helper reads it");
    let events = until_done(&mut session);
    let shown = format!("{session:?} {events:?} {secret:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[test]
fn an_answer_to_a_finished_helper_is_an_error() {
    let scratch = Scratch::new("late");
    let helper = scratch.script("read cookie\nprintf 'FAILURE\\n'");
    let mut session = start(&helper, "alice", "c");
    until_done(&mut session);
    assert!(session.respond(&SecretText::new("late")).is_err());
}
