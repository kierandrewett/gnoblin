//! State-machine tests for the consent guard, against a fake prompt host.
//!
//! These drive `Guard` directly: no bus, no compositor. Each test names the failure it
//! pins so a regression points at the rule that broke.

use futures_channel::oneshot;
use gnoblin_authority::Consent;
use gnoblin_types::{ClientId, RequestId, Tick};
use xdg_desktop_portal_gnoblin::consent::{
    ConsentOutcome, Guard, Host, Interface, NewSession, Offer, Pending, PortalCapability, Prompt, Refusal,
};

const BUS: &str = ":1.42";
const APP: &str = "org.example.Capture";
const PATH: &str = "/org/freedesktop/portal/desktop/session/1";
const LIFETIME_MS: u64 = 1_000;

/// A host that offers every prompt straight away and never draws anything.
struct FakeHost {
    next: u64,
    withdrawn: Vec<RequestId>,
}

impl FakeHost {
    fn boxed() -> Box<dyn Host> {
        Box::new(FakeHost {
            next: 1,
            withdrawn: Vec::new(),
        })
    }
}

impl Host for FakeHost {
    fn offer(&mut self, _prompt: &Prompt, _timeout: std::time::Duration) -> Result<Offer, Refusal> {
        let request = RequestId::new(self.next);
        self.next += 1;
        Ok(Offer {
            request,
            provider: ClientId::new(1),
        })
    }

    fn offer_secret(
        &mut self,
        _app_id: &str,
        _timeout: std::time::Duration,
    ) -> Result<futures_channel::oneshot::Receiver<Option<zeroize::Zeroizing<Vec<u8>>>>, Refusal> {
        Err(Refusal::NoProvider { slot: "secret.derive" })
    }

    fn withdraw(&mut self, request: RequestId) {
        self.withdrawn.push(request);
    }
}

fn guard_with_session() -> Guard {
    let mut guard = Guard::new(FakeHost::boxed(), LIFETIME_MS);
    guard
        .create(NewSession {
            bus_name: BUS.to_owned(),
            app_id: APP.to_owned(),
            sandboxed: Some(true),
            interface: Interface::ScreenCast,
            path: PATH.to_owned(),
        })
        .expect("a fresh path creates a session");
    guard.start(BUS, PATH).expect("a created session starts");
    guard
}

fn ask(guard: &mut Guard, now: u64) -> (RequestId, oneshot::Receiver<ConsentOutcome>) {
    match guard
        .ask(Tick::new(now), BUS, PATH, PortalCapability::Capture, "")
        .expect("an asked session is not refused")
    {
        Pending::Asked { request, answered } => (request, answered),
        Pending::AlreadyGranted => panic!("nothing is granted yet"),
    }
}

/// A user's allow answer grants the capability and the caller can read it back.
#[test]
fn allow_answer_grants_capability() {
    let mut guard = guard_with_session();
    let (request, mut answered) = ask(&mut guard, 0);

    let outcome = guard
        .answer(Tick::new(10), request, Some(b"allow"))
        .expect("the offered request is answerable");

    assert!(outcome.granted());
    assert_eq!(
        answered.try_recv().expect("the waiter is still live"),
        Some(ConsentOutcome::Granted)
    );
    assert!(guard.consent(BUS, APP, PortalCapability::Capture).is_some());
}

/// A refusal is reported as the user's own cancel, and grants nothing.
#[test]
fn refusal_grants_nothing() {
    let mut guard = guard_with_session();
    let (request, mut answered) = ask(&mut guard, 0);

    guard
        .answer(Tick::new(10), request, None)
        .expect("the offered request is answerable");

    assert_eq!(
        answered.try_recv().expect("the waiter is still live"),
        Some(ConsentOutcome::Refused)
    );
    assert_eq!(
        guard.consent(BUS, APP, PortalCapability::Capture),
        Some(Consent::Refused)
    );
}

/// A prompt the user never answers expires at the guard's lifetime and grants nothing.
#[test]
fn unanswered_prompt_expires() {
    let mut guard = guard_with_session();
    let (request, mut answered) = ask(&mut guard, 0);

    let expired = guard.tick(Tick::new(LIFETIME_MS + 1));

    assert_eq!(expired, vec![request]);
    assert_eq!(
        answered.try_recv().expect("the waiter is still live"),
        Some(ConsentOutcome::Expired)
    );
    assert_eq!(guard.pending(), 0);
    assert!(guard.consent(BUS, APP, PortalCapability::Capture).is_none());
}

/// A requester that vanishes while its prompt is up releases the prompt as `Left`.
#[test]
fn requester_gone_releases_prompt() {
    let mut guard = guard_with_session();
    let (_request, mut answered) = ask(&mut guard, 0);

    guard.requester_gone(Tick::new(5), BUS);

    assert_eq!(
        answered.try_recv().expect("the waiter is still live"),
        Some(ConsentOutcome::Left)
    );
    assert_eq!(guard.pending(), 0);
}

/// A provider that exits mid-prompt releases the prompt as `ProviderGone`, not as a grant.
#[test]
fn provider_gone_releases_prompt() {
    let mut guard = guard_with_session();
    let (_request, mut answered) = ask(&mut guard, 0);

    guard.provider_gone(Tick::new(5), ClientId::new(1));

    assert_eq!(
        answered.try_recv().expect("the waiter is still live"),
        Some(ConsentOutcome::ProviderGone)
    );
    assert!(guard.consent(BUS, APP, PortalCapability::Capture).is_none());
}

/// Once granted, a second ask is answered from the grant and does not prompt again.
#[test]
fn granted_capability_is_not_asked_twice() {
    let mut guard = guard_with_session();
    let (request, _answered) = ask(&mut guard, 0);
    guard
        .answer(Tick::new(10), request, Some(b"allow"))
        .expect("answerable");

    let again = guard
        .ask(Tick::new(20), BUS, PATH, PortalCapability::Capture, "")
        .expect("not refused");

    assert!(matches!(again, Pending::AlreadyGranted));
    assert_eq!(guard.pending(), 0);
}

/// A session from a caller with no app id. Such a caller has no identity the portal can
/// trust, so it gets no shared grant: each of its sessions is a request of its own.
fn guard_with_anonymous_sessions(paths: &[&str]) -> Guard {
    let mut guard = Guard::new(FakeHost::boxed(), LIFETIME_MS);
    for path in paths {
        guard
            .create(NewSession {
                bus_name: BUS.to_owned(),
                app_id: String::new(),
                sandboxed: Some(false),
                interface: Interface::ScreenCast,
                path: (*path).to_owned(),
            })
            .expect("a fresh path creates a session");
        guard.start(BUS, path).expect("a created session starts");
    }
    guard
}

/// Ask for a session's capture, returning the request when the prompt was offered.
fn ask_path(guard: &mut Guard, now: u64, path: &str) -> Result<Pending, Refusal> {
    guard.ask(Tick::new(now), BUS, path, PortalCapability::Capture, "")
}

/// A refusal from a caller with no app id is not reused: the next request from the same
/// bus prompts the user again rather than being refused from the cache.
#[test]
fn anonymous_refusal_is_not_reused() {
    let paths = [
        "/org/freedesktop/portal/desktop/session/a",
        "/org/freedesktop/portal/desktop/session/b",
    ];
    let mut guard = guard_with_anonymous_sessions(&paths);
    let Ok(Pending::Asked { request, mut answered }) = ask_path(&mut guard, 0, paths[0]) else {
        panic!("the first anonymous request is offered");
    };

    guard
        .answer(Tick::new(10), request, None)
        .expect("the offered request is answerable");
    assert_eq!(
        answered.try_recv().expect("the waiter is still live"),
        Some(ConsentOutcome::Refused)
    );
    assert!(guard.consent(BUS, "", PortalCapability::Capture).is_none());

    assert!(
        matches!(ask_path(&mut guard, 20, paths[1]), Ok(Pending::Asked { .. })),
        "the second anonymous request prompts again instead of being refused"
    );
}

/// Two anonymous requests from one bus queue as two prompts. The second is not refused as
/// already pending behind the first, and the first refusal does not decide the second.
#[test]
fn anonymous_requests_queue_independently() {
    let paths = [
        "/org/freedesktop/portal/desktop/session/a",
        "/org/freedesktop/portal/desktop/session/b",
    ];
    let mut guard = guard_with_anonymous_sessions(&paths);
    let Ok(Pending::Asked {
        request: first,
        answered: mut first_answer,
    }) = ask_path(&mut guard, 0, paths[0])
    else {
        panic!("the first anonymous request is offered");
    };
    let Ok(Pending::Asked {
        request: second,
        answered: mut second_answer,
    }) = ask_path(&mut guard, 0, paths[1])
    else {
        panic!("the second anonymous request queues rather than being refused");
    };
    assert_eq!(guard.pending(), 2);

    guard
        .answer(Tick::new(10), first, None)
        .expect("the first is answerable");
    guard
        .answer(Tick::new(11), second, Some(b"allow"))
        .expect("the second is answerable");

    assert_eq!(first_answer.try_recv().expect("live"), Some(ConsentOutcome::Refused));
    assert_eq!(second_answer.try_recv().expect("live"), Some(ConsentOutcome::Granted));
    assert_eq!(guard.pending(), 0);
}

/// A full queue of anonymous prompts refuses the next one by name, as `Busy`, and never as
/// the user's refusal.
#[test]
fn anonymous_queue_full_is_named_busy() {
    let path = "/org/freedesktop/portal/desktop/session/a";
    let mut guard = guard_with_anonymous_sessions(&[path]);
    for now in 0..64 {
        assert!(
            matches!(ask_path(&mut guard, now, path), Ok(Pending::Asked { .. })),
            "request {now} fits in the queue"
        );
    }

    assert!(matches!(ask_path(&mut guard, 64, path), Err(Refusal::Busy)));
}

/// A refusal from a named app stands for its grant lifetime, so the same app is refused
/// again without a second prompt.
#[test]
fn named_refusal_is_reused() {
    let mut guard = guard_with_session();
    let (request, _answered) = ask(&mut guard, 0);
    guard
        .answer(Tick::new(10), request, None)
        .expect("the offered request is answerable");

    assert!(matches!(
        guard.ask(Tick::new(20), BUS, PATH, PortalCapability::Capture, ""),
        Err(Refusal::Refused)
    ));
}
