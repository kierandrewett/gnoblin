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
