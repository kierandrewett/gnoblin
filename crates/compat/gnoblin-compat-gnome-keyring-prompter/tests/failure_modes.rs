//! One test per documented failure mode of the prompter's state machine.
//!
//! These drive the state machine directly, so a failure mode is a sequence of calls and
//! an assertion. The same transitions over a real bus are `tests/dbus.rs`, and the D-Bus
//! interface's own refusals are there because they belong to the transport rather than to
//! the state.

gnoblin_log::component!("compat-keyring");

mod support;

use std::cell::RefCell;

use futures_channel::mpsc;
use gnoblin_compat_gnome_keyring_prompter::{
    Callback, Decision, Effect, Exchange, Offerer, Prompt, PromptKind, Prompter, Secret, Unavailable,
};
use gnoblin_model::{RequestId, RequestOutcome, Tick};
use support::log::Captured;

const PASSPHRASE: &str = "correct horse battery staple";
const CALLBACK: &str = "/org/example/Keyring1";

/// A host that records the prompts it was asked to offer and can be told to refuse.
#[derive(Debug, Default)]
struct Host {
    prompts: RefCell<Vec<Prompt>>,
    refuse: RefCell<Option<Unavailable>>,
    next: RefCell<u64>,
}

impl Offerer for Host {
    fn offer(&mut self, prompt: &Prompt, _deadline: Tick) -> Result<RequestId, Unavailable> {
        if let Some(refusal) = self.refuse.borrow().clone() {
            return Err(refusal);
        }
        self.prompts.borrow_mut().push(prompt.clone());
        let next = *self.next.borrow() + 1;
        self.next.replace(next);
        Ok(RequestId::new(next))
    }
}

fn callback() -> Callback {
    Callback {
        sender: ":1.7".to_owned(),
        path: CALLBACK.to_owned(),
    }
}

fn password_prompt() -> Prompt {
    Prompt {
        kind: PromptKind::Password,
        callback: CALLBACK.to_owned(),
        title: "Unlock keyring".to_owned(),
        message: "Enter password for the default keyring".to_owned(),
        description: "The login keyring".to_owned(),
        warning: String::new(),
        choice_label: String::new(),
        choice_chosen: false,
        password_new: false,
        caller_window: String::new(),
        continue_label: String::new(),
        cancel_label: String::new(),
    }
}

/// A prompter whose callback has begun prompting, its caller's half of the exchange, and
/// the effects queue.
struct Fixture {
    prompter: Prompter<Host>,
    effects: mpsc::UnboundedReceiver<Effect>,
    caller: Exchange,
    caller_exchange: String,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture::with(Host::default())
    }

    fn with(host: Host) -> Fixture {
        let (mut prompter, mut effects) = Prompter::new(host);
        prompter.begin(&callback()).expect("the callback is new");
        // The begin announces this side's exchange; a test asserting on the answer must
        // not read that announcement as one.
        let announced = effects.try_recv().expect("the began callback is announced");
        assert!(matches!(announced, Effect::Ready { .. }));
        let caller = Exchange::new().expect("the system has randomness");
        let caller_exchange = caller.begin();
        Fixture {
            prompter,
            effects,
            caller,
            caller_exchange,
        }
    }

    fn perform(&mut self) -> Result<Offered, Unavailable> {
        self.prompter
            .perform(&callback(), password_prompt(), &self.caller_exchange, Tick::new(120))
            .map(|request| Offered { request })
    }

    fn effect(&mut self) -> Option<Effect> {
        self.effects.try_recv().ok()
    }
}

/// The request identity, wrapped so a test can read it without keeping the channel.
#[derive(Debug)]
struct Offered {
    request: RequestId,
}

#[test]
fn no_provider_fails_closed_and_names_the_slot() {
    // The failure mode: a prompt is offered to `prompt.secret` with no provider, so it
    // waits out 120 seconds and gnome-keyring reports a cancellation. The refusal must
    // name the slot and nothing may be offered.
    let host = Host::default();
    host.refuse
        .replace(Some(Unavailable::NoProvider { slot: "prompt.secret" }));
    let mut fixture = Fixture::with(host);
    let refusal = fixture.perform().expect_err("no provider");
    assert!(matches!(refusal, Unavailable::NoProvider { .. }));
    assert!(
        refusal.to_string().contains("prompt.secret"),
        "the error names the slot: {refusal}"
    );
    assert!(fixture.prompter.pending().next().is_none(), "nothing waits");
}

#[test]
fn a_prompt_needs_a_begun_callback() {
    // The failure mode: a caller asks for a prompt before registering its callback, so
    // the answer has nowhere to go.
    let (mut prompter, _effects) = Prompter::new(Host::default());
    let caller = Exchange::new().expect("randomness");
    let refusal = prompter
        .perform(&callback(), password_prompt(), &caller.begin(), Tick::new(120))
        .expect_err("the callback was never begun");
    assert!(matches!(refusal, Unavailable::NotBegun { .. }));
}

#[test]
fn a_second_prompt_on_one_callback_is_refused() {
    // The failure mode: a caller retries a prompt it already started, and the user ends up
    // answering the first one while the caller waits for the second.
    let mut fixture = Fixture::new();
    fixture.perform().expect("the first prompt is offered");
    let refusal = fixture.perform().expect_err("the callback is busy");
    assert!(matches!(refusal, Unavailable::Busy { .. }));
    assert_eq!(fixture.prompter.pending().count(), 1);
}

#[test]
fn an_answer_after_a_cancel_is_refused() {
    // The failure mode: the user dismissed the prompt and the provider answered anyway.
    let mut fixture = Fixture::new();
    let request = fixture.perform().expect("offered").request;
    fixture.prompter.cancel(request).expect("the request is pending");
    let refusal = fixture
        .prompter
        .answer(request, &RequestOutcome::Answered(PASSPHRASE.as_bytes().to_vec()))
        .expect_err("a finished request stays finished");
    assert_eq!(refusal, Unavailable::Finished { request });
}

#[test]
fn a_request_past_its_deadline_is_cancelled() {
    // The failure mode: a prompt nobody answers hangs gnome-keyring's unlock forever. The
    // expiry is a tick the caller supplies, and it ends as a dismissal.
    let mut fixture = Fixture::new();
    let request = fixture.perform().expect("offered").request;
    assert!(fixture.prompter.expire(Tick::new(119)).is_empty(), "not yet due");
    let finished = fixture.prompter.expire(Tick::new(120));
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].request, request);
    match fixture.effect().expect("the caller is told") {
        Effect::Ready { response, .. } => assert_eq!(response, "no"),
        other => panic!("expected a dismissal, got {other:?}"),
    }
}

#[test]
fn a_stop_ends_the_prompt_with_done() {
    // The failure mode: a caller closes its prompt object while a request is pending, and
    // the request waits for a caller that is gone. gcr's own service answers this with
    // `PromptDone` and no `PromptReady`.
    let mut fixture = Fixture::new();
    fixture.perform().expect("offered");
    fixture.prompter.stop(&callback());
    match fixture.effect().expect("the caller is told") {
        Effect::Done { path, .. } => assert_eq!(path, CALLBACK),
        other => panic!("expected a done, got {other:?}"),
    }
    assert!(fixture.prompter.pending().next().is_none());
    let refusal = fixture
        .prompter
        .perform(&callback(), password_prompt(), &fixture.caller_exchange, Tick::new(120))
        .expect_err("the callback is no longer begun");
    assert!(matches!(refusal, Unavailable::NotBegun { .. }));
}

#[test]
fn an_answer_arrives_encrypted() {
    // The failure mode: the passphrase travels in the clear on the callback. The effect's
    // exchange field is the only place it goes, and the caller can read it only with its
    // own half of the key.
    let mut fixture = Fixture::new();
    let request = fixture.perform().expect("offered").request;
    fixture
        .prompter
        .answer(request, &RequestOutcome::Answered(PASSPHRASE.as_bytes().to_vec()))
        .expect("the request is pending");

    let effect = fixture.effect().expect("the caller is told");
    let Effect::Ready { response, exchange, .. } = effect else {
        panic!("expected a ready effect");
    };
    assert_eq!(response, "yes");
    assert!(
        !exchange.contains(PASSPHRASE),
        "the passphrase is not in the exchange string"
    );
    let secret = fixture
        .caller
        .receive(&exchange)
        .expect("the caller reads the answer")
        .expect("the answer carries the passphrase");
    assert_eq!(secret.as_str(), Some(PASSPHRASE));
}

#[test]
fn a_declined_confirmation_answers_no_without_a_secret() {
    // A confirmation has no passphrase, so its answer must not carry an encrypted secret a
    // caller could read as one.
    let mut fixture = Fixture::new();
    let mut prompt = password_prompt();
    prompt.kind = PromptKind::Confirm;
    prompt.message = "Allow the application to store a password?".to_owned();
    let request = fixture
        .prompter
        .perform(&callback(), prompt, &fixture.caller_exchange, Tick::new(120))
        .expect("offered");
    fixture
        .prompter
        .answer(request, &RequestOutcome::Answered(b"no".to_vec()))
        .expect("the request is pending");

    let Effect::Ready { response, exchange, .. } = fixture.effect().expect("the caller is told") else {
        panic!("expected a ready effect");
    };
    assert_eq!(response, "no");
    let secret = fixture.caller.receive(&exchange).expect("the caller reads the answer");
    assert!(secret.is_none(), "a confirmation carries no passphrase");
}

#[test]
fn an_ended_request_is_a_dismissal() {
    let mut fixture = Fixture::new();
    let request = fixture.perform().expect("offered").request;
    fixture
        .prompter
        .answer(request, &RequestOutcome::Ended)
        .expect("the request is pending");
    let Effect::Ready { response, .. } = fixture.effect().expect("the caller is told") else {
        panic!("expected a ready effect");
    };
    assert_eq!(response, "no");
    assert!(matches!(fixture.prompter.decision(request), Some(Decision::Cancelled)));
}

#[test]
fn a_passphrase_never_reaches_the_log() {
    // The failure mode the whole `Secret` type exists for: a prompt is offered and
    // answered, and a `{:?}` of the answer, of the request or of the effect puts the
    // passphrase in a log line that a bug report then carries.
    let captured = Captured::default();
    let _guard = captured.install();

    let mut fixture = Fixture::new();
    let request = fixture.perform().expect("offered").request;
    gnoblin_log::info_in!("compat-keyring", "answering the prompt");
    fixture
        .prompter
        .answer(request, &RequestOutcome::Answered(PASSPHRASE.as_bytes().to_vec()))
        .expect("the request is pending");
    let effect = fixture.effect().expect("the caller is told");
    // Logged at `info`: the default filter drops `debug`, and this line must reach the capture.
    gnoblin_log::info_in!("compat-keyring", ?effect, "the effect was queued");

    let lines = captured.lines();
    assert_eq!(lines.len(), 2, "the test's own lines were captured: {lines:?}");
    for line in &lines {
        assert!(!line.contains(PASSPHRASE), "a log line carried the passphrase: {line}");
    }
    // The secret's own rendering is the guard: it names its length and nothing else.
    let rendered = format!("{:?}", Secret::new(PASSPHRASE.as_bytes().to_vec()));
    assert!(!rendered.contains(PASSPHRASE), "{rendered}");
    assert!(rendered.contains("bytes redacted"), "{rendered}");
}

#[test]
fn the_begin_effect_carries_the_prompters_exchange() {
    // gcr's caller waits for its first `PromptReady` before it can prompt at all, so the
    // begun callback must have produced one, and it must carry this side's public key.
    let (mut prompter, mut effects) = Prompter::new(Host::default());
    prompter.begin(&callback()).expect("the callback is new");
    let Effect::Ready {
        sender,
        path,
        response,
        exchange,
    } = effects.try_recv().expect("the began callback is announced")
    else {
        panic!("expected a ready effect");
    };
    assert_eq!(sender, ":1.7");
    assert_eq!(path, CALLBACK);
    assert!(response.is_empty(), "nothing has been decided yet");
    assert!(exchange.starts_with("[sx-aes-1]\npublic="), "{exchange}");
    assert!(!exchange.contains("secret="), "the first string has no secret");
}

#[test]
fn a_begun_callback_is_refused_twice() {
    // gcr refuses a second `BeginPrompting` on one path; the second would replace the
    // caller's exchange and answer the first half with the second's key.
    let (mut prompter, _effects) = Prompter::new(Host::default());
    prompter.begin(&callback()).expect("the first begin");
    assert!(matches!(
        prompter.begin(&callback()).expect_err("the second begin"),
        Unavailable::Busy { .. }
    ));
}
