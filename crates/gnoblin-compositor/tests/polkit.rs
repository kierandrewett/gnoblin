//! polkit authentication through a real compositor: the services role offers an authentication, the compositor
//! draws its own dialog, a fake `polkit-agent-helper-1` checks the password, and only the helper's success
//! answers the request (design/js-research/polkit.md).
//!
//! The helper here is a shell script speaking the real helper's protocol; no PAM and no polkitd is involved, so
//! these tests prove the compositor's half: who may offer and observe, what the dialog does with the helper's
//! prompts and verdicts, and that a cancel, a timeout, a missing helper and a departed requester all end the request
//! unauthenticated with the helper stopped. They do not prove a real polkitd accepts the outcome.
//!
//! # Failure modes first
//!
//! 1. **A script answers.** A shell cannot claim `auth.polkit` or offer to it (`tests/privilege.rs`,
//!    `a_shell_cannot_claim_the_polkit_slot_and_only_the_services_role_offers_it`), and an observer's answer is only
//!    a verdict: `an_observer_that_passes_leaves_the_dialog_and_one_that_denies_refuses_it`.
//! 2. **The wrong password authenticates.** Only the helper's `SUCCESS` answers, and a failure asks again:
//!    `a_wrong_password_asks_again_and_the_right_one_authenticates`.
//! 3. **A helper outlives its request.** Cancel, polkitd's cancel and the deadline each stop it:
//!    `escape_declines_and_stops_the_helper`, `polkitd_cancelling_closes_the_dialog_and_stops_the_helper`,
//!    `an_authentication_nobody_finishes_times_out_and_stops_the_helper`.
//! 4. **A missing helper authenticates, or hangs.** The request fails closed:
//!    `a_missing_helper_fails_closed`.

mod support;

use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::wire::portal_key;
use gnoblin_client::{Client, Event};
use gnoblin_log::__tracing::span::{Attributes, Id, Record};
use gnoblin_log::__tracing::{Event as LogEvent, Metadata, Subscriber};
use gnoblin_log::event::Logged;
use gnoblin_model::{Provider, RequestCompletion};
use gnoblin_polkit_auth::{AuthIdentity, AuthRequest, current_uid, name_of};
use gnoblin_types::RequestId;
use support::{Session, TIMEOUT, Wire};

/// The cookie the fake helper accepts.
const COOKIE: &str = "9-cookie";

/// How long a test lets the compositor take a prompt the helper sent into its dialog before typing: the dispatcher
/// looks at the helper every 50 ms.
const SETTLE: Duration = Duration::from_millis(500);

/// The fake helper: asks for a password, accepts `hunter2` from `@USER@` with the cookie, and records what it did in
/// the directory it lives in.
const HELPER: &str = r#"read cookie
runs=$(cat @DIR@/runs 2>/dev/null || echo 0)
runs=$((runs + 1))
echo $runs > @DIR@/runs
echo $$ > @DIR@/pid
printf 'PAM_TEXT_INFO Place your finger on the reader\n'
printf 'PAM_PROMPT_ECHO_OFF Password: \n'
echo asked >> @DIR@/asked
read password
if [ "$1" = @USER@ ] && [ "$cookie" = 9-cookie ] && [ "$password" = hunter2 ]; then
    printf 'SUCCESS\n'
    exit 0
fi
printf 'PAM_ERROR_MSG Authentication failure\n'
printf 'FAILURE\n'
exit 1"#;

/// Every event any thread of this test process logs, so a test can look for a password in the compositor's own logs.
#[derive(Clone, Default)]
struct Recorder(std::sync::Arc<Mutex<Vec<Logged>>>);

impl Subscriber for Recorder {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &LogEvent<'_>) {
        self.0.lock().expect("the log lock").push(Logged::from_event(event));
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// The recorder every compositor thread of this process logs to, installed once.
fn recorder() -> &'static Recorder {
    static RECORDER: OnceLock<Recorder> = OnceLock::new();
    RECORDER.get_or_init(|| {
        let recorder = Recorder::default();
        let _ = gnoblin_log::__tracing::subscriber::set_global_default(recorder.clone());
        recorder
    })
}

/// A session with the fake helper installed, and the directory the helper records in.
struct Rig {
    session: Session,
    services: Client<UnixTransport>,
    input: Wire,
    directory: PathBuf,
    user: String,
    _scratch: tempfile::TempDir,
}

fn take(ends: &mut Vec<(String, UnixStream)>, role: &str) -> UnixStream {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    ends.remove(index).1
}

impl Rig {
    /// A session whose polkit helper is `script` (with `@DIR@` and `@USER@` filled in), or the path
    /// `/nonexistent/polkit-agent-helper-1` when it is `None`. The services role has claimed `auth.polkit`.
    fn start(script: Option<&str>) -> Rig {
        recorder();
        let scratch = tempfile::tempdir().expect("a scratch directory");
        let directory = scratch.path().to_path_buf();
        let uid = current_uid().expect("this process has a uid");
        let user = name_of(uid).expect("and an account");
        let helper = match script {
            Some(script) => {
                let path = directory.join("polkit-agent-helper-1");
                let body = script
                    .replace("@DIR@", &directory.display().to_string())
                    .replace("@USER@", &user);
                std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("the script is written");
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("it is executable");
                path
            }
            None => PathBuf::from("/nonexistent/polkit-agent-helper-1"),
        };
        let configured = helper.clone();
        let (session, mut ends) = Session::start_privileged_configured(&["services", "input"], move |config| {
            config.polkit_helper = Some(configured);
        });
        let mut services = Client::connect(
            UnixTransport::from_stream(take(&mut ends, "services")).expect("a transport"),
            gnoblin_compositor::version(),
        )
        .expect("the handshake");
        services.set_read_timeout(Some(TIMEOUT)).expect("a read deadline");
        services
            .claim_slot("auth.polkit", Provider::Services)
            .expect("the services role claims its slot");
        let input = Wire::over(take(&mut ends, "input"));
        input.set_read_timeout(TIMEOUT);
        Rig {
            session,
            services,
            input,
            directory,
            user,
            _scratch: scratch,
        }
    }

    /// The payload the agent offers for this process's own account.
    fn payload(&self) -> Vec<u8> {
        AuthRequest::new(
            COOKIE.to_owned(),
            "org.freedesktop.policykit.exec".to_owned(),
            "Authentication is needed to run `/usr/bin/true' as the super user".to_owned(),
            String::new(),
            std::collections::BTreeMap::new(),
            vec![AuthIdentity::UnixUser {
                uid: current_uid().expect("a uid"),
            }],
        )
        .encode()
    }

    fn offer(&mut self, timeout: Duration) -> RequestId {
        let payload = self.payload();
        self.services
            .offer(
                "auth.polkit",
                "org.freedesktop.policykit.exec",
                "Authentication is needed",
                &payload,
                timeout,
            )
            .expect("the compositor takes the authentication")
    }

    /// Waits until the helper has asked for its `n`th password, and gives the compositor a moment to show it.
    fn wait_asked(&self, n: usize) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let asked = std::fs::read_to_string(self.directory.join("asked")).map_or(0, |text| text.lines().count());
            if asked >= n {
                break;
            }
            assert!(Instant::now() < deadline, "the helper never asked for password {n}");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(SETTLE);
    }

    /// Types `text` and Enter into the compositor's dialog.
    fn type_and_confirm(&mut self, text: &str) {
        for (index, character) in text.chars().enumerate() {
            self.key(3_000 + index as u64, &format!("text:{character}"));
        }
        self.key(4_000, "enter");
    }

    fn key(&mut self, id: u64, key: &str) {
        self.input.send(&portal_key(id, key, false));
        let _ack = self.input.receive();
    }

    fn runs(&self) -> usize {
        std::fs::read_to_string(self.directory.join("runs"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0)
    }

    /// The pid the helper's latest run recorded.
    fn helper_pid(&self) -> u32 {
        std::fs::read_to_string(self.directory.join("pid"))
            .expect("the helper recorded its pid")
            .trim()
            .parse()
            .expect("a pid")
    }

    /// The next event of the services role, skipping nothing: what the host is told is exact.
    fn next(&mut self) -> Event {
        self.services
            .next_event()
            .expect("the host is told how the request ended")
    }

    /// The answer the host reads for `request`.
    fn answer(&mut self, request: RequestId) -> Vec<u8> {
        for _ in 0..4 {
            match self.next() {
                Event::RequestAnswered { request: ended, answer } => {
                    assert_eq!(ended, request);
                    return answer.to_vec();
                }
                Event::RequestFinished { .. } => {}
                other => panic!("unexpected event {other:?}"),
            }
        }
        panic!("the host was never given an answer");
    }

    /// The completion the host is told for `request`, which must come with no answer before it.
    fn completion(&mut self, request: RequestId) -> RequestCompletion {
        match self.next() {
            Event::RequestFinished {
                request: ended,
                completion,
            } => {
                assert_eq!(ended, request);
                completion
            }
            other => panic!("expected the request to end without an answer, got {other:?}"),
        }
    }
}

/// The next offer the observer is sent, whatever else the connection hears first.
fn next_offer(shell: &mut Client<UnixTransport>) -> Event {
    for _ in 0..50 {
        let event = shell.next_event().expect("the observer is shown the request");
        if matches!(event, Event::RequestOffered { .. }) {
            return event;
        }
    }
    panic!("the observer was never shown the request");
}

/// Waits until process `pid` is gone, which a killed and reaped helper is.
fn wait_gone(pid: u32) {
    let deadline = Instant::now() + TIMEOUT;
    while Path::new(&format!("/proc/{pid}")).exists() {
        assert!(Instant::now() < deadline, "the helper {pid} is still running");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_right_password_is_the_helpers_success_and_the_agent_reads_who_authenticated() {
    let mut rig = Rig::start(Some(HELPER));
    let request = rig.offer(Duration::from_secs(60));
    rig.wait_asked(1);
    rig.type_and_confirm("hunter2");

    assert_eq!(
        rig.answer(request),
        format!("unix-user:{}", rig.user).into_bytes(),
        "the agent is told who authenticated, by the compositor, after the helper said SUCCESS"
    );
    assert_eq!(rig.runs(), 1);
}

#[test]
fn a_wrong_password_asks_again_and_the_right_one_authenticates() {
    let mut rig = Rig::start(Some(HELPER));
    let request = rig.offer(Duration::from_secs(60));
    rig.wait_asked(1);
    rig.type_and_confirm("nope");
    // The helper reported FAILURE, so a new helper asks again; nothing was answered.
    rig.wait_asked(2);
    rig.type_and_confirm("hunter2");

    assert_eq!(rig.answer(request), format!("unix-user:{}", rig.user).into_bytes());
    assert_eq!(rig.runs(), 2, "one helper for the wrong password and one for the right");
}

#[test]
fn escape_declines_and_stops_the_helper() {
    let mut rig = Rig::start(Some(HELPER));
    let request = rig.offer(Duration::from_secs(60));
    rig.wait_asked(1);
    let pid = rig.helper_pid();
    rig.type_and_confirm("half");
    rig.key(5_000, "escape");

    assert_eq!(rig.completion(request), RequestCompletion::Ended);
    wait_gone(pid);
}

#[test]
fn polkitd_cancelling_closes_the_dialog_and_stops_the_helper() {
    // The services role withdraws the request it offered when polkitd calls CancelAuthentication.
    let mut rig = Rig::start(Some(HELPER));
    let request = rig.offer(Duration::from_secs(60));
    rig.wait_asked(1);
    let pid = rig.helper_pid();
    rig.services
        .decline(request)
        .expect("the offerer withdraws its request");

    assert_eq!(rig.completion(request), RequestCompletion::Ended);
    wait_gone(pid);
}

#[test]
fn an_authentication_nobody_finishes_times_out_and_stops_the_helper() {
    let mut rig = Rig::start(Some(HELPER));
    let request = rig.offer(Duration::from_secs(2));
    rig.wait_asked(1);
    let pid = rig.helper_pid();

    assert_eq!(rig.completion(request), RequestCompletion::TimedOut);
    wait_gone(pid);
}

#[test]
fn a_missing_helper_fails_closed() {
    let mut rig = Rig::start(None);
    let request = rig.offer(Duration::from_secs(60));
    // The dialog is up and says the helper is missing; typing authenticates nothing, and closing it ends the request.
    rig.type_and_confirm("hunter2");
    rig.key(5_000, "escape");

    assert_eq!(rig.completion(request), RequestCompletion::Ended);
    assert_eq!(rig.runs(), 0);
}

#[test]
fn a_helper_that_never_succeeds_does_not_authenticate() {
    let mut rig = Rig::start(Some("read cookie\nprintf 'FAILURE\\n'\nexit 1"));
    let request = rig.offer(Duration::from_secs(60));
    // Five failures end the attempts; the dialog then offers only Close.
    std::thread::sleep(SETTLE * 2);
    rig.key(5_000, "escape");

    assert_eq!(rig.completion(request), RequestCompletion::Ended);
}

#[test]
fn an_observer_that_passes_leaves_the_dialog_and_one_that_denies_refuses_it() {
    // A shell that claims `dialogs.observe` sees the authentication, without its cookie, and can only refuse it.
    let mut rig = Rig::start(Some(HELPER));
    let mut shell = Client::connect(
        UnixTransport::connect(rig.session.socket()).expect("the session listens"),
        gnoblin_compositor::version(),
    )
    .expect("the handshake");
    shell.set_read_timeout(Some(TIMEOUT)).expect("a read deadline");
    shell
        .claim_slot("dialogs.observe", Provider::Shell)
        .expect("a shell observes the dialogs");

    let denied = rig.offer(Duration::from_secs(60));
    let Event::RequestOffered {
        request,
        slot,
        title,
        subject,
        ..
    } = next_offer(&mut shell)
    else {
        unreachable!("next_offer returns offers");
    };
    assert_eq!(
        (request, slot.as_str(), title.as_str()),
        (denied, "dialogs.observe", "auth")
    );
    let shown = String::from_utf8_lossy(&subject).into_owned();
    assert!(shown.contains("org.freedesktop.policykit.exec"), "{shown}");
    assert!(!shown.contains(COOKIE), "the cookie never reaches a script: {shown}");
    shell.claim(request).expect("the observer claims it");
    shell.answer(request, b"deny").expect("and refuses it");
    assert!(matches!(rig.completion(denied), RequestCompletion::Refused(_)));
    assert_eq!(rig.runs(), 0, "a refused authentication never runs the helper");

    // The next one is passed, and is then the dialog's: only the helper's success answers it.
    let passed = rig.offer(Duration::from_secs(60));
    let Event::RequestOffered { request, .. } = next_offer(&mut shell) else {
        unreachable!("next_offer returns offers");
    };
    assert_eq!(request, passed);
    shell.claim(request).expect("the observer claims it");
    shell
        .answer(request, b"unix-user:root")
        .expect("and says anything but `deny`");
    rig.wait_asked(1);
    assert!(
        shell.answer(request, b"unix-user:root").is_err(),
        "the observer cannot answer it again"
    );
    rig.type_and_confirm("hunter2");
    assert_eq!(rig.answer(passed), format!("unix-user:{}", rig.user).into_bytes());
}

#[test]
fn the_password_is_in_no_log_line_of_the_compositor() {
    let mut rig = Rig::start(Some(HELPER));
    let request = rig.offer(Duration::from_secs(60));
    rig.wait_asked(1);
    rig.type_and_confirm("hunter2");
    assert_eq!(rig.answer(request), format!("unix-user:{}", rig.user).into_bytes());

    let logged = recorder().0.lock().expect("the log lock").clone();
    assert!(!logged.is_empty(), "the compositor logged while it served the request");
    for line in &logged {
        let shown = format!("{} {} {}", line.component, line.message, line.fields_line());
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
