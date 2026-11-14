//! One test per documented failure mode of the polkit agent's state machine.
//!
//! Nothing here touches a bus: the agent is a state machine over requests it offered,
//! so a failure mode is a sequence of calls and an assertion, and the D-Bus shape of
//! the same transitions is `tests/dbus.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;

use gnoblin_model::{RequestId, RequestOutcome, Tick};
use gnoblin_service_core::{
    Agent, CapabilitySlot, Decision, Identity, Offerer, PolkitRequest, Proposition, Provider, Refusal, Slots,
};

/// A host that records what it was asked to offer and hands back successive requests.
///
/// It stands in for the privileged connection: the point of the seam is that the agent's
/// state machine can be proved without a compositor, and the offer a provider sees is
/// exactly the proposition this records.
#[derive(Debug, Default)]
struct Host {
    offers: RefCell<Vec<(Proposition, Tick)>>,
    next: RefCell<u64>,
}

impl Host {
    fn offers(&self) -> Vec<(Proposition, Tick)> {
        self.offers.borrow().clone()
    }
}

impl Offerer for Host {
    fn offer(&mut self, proposition: &Proposition, deadline: Tick) -> Result<RequestId, Refusal> {
        self.offers.borrow_mut().push((proposition.clone(), deadline));
        let next = *self.next.borrow() + 1;
        self.next.replace(next);
        Ok(RequestId::new(next))
    }
}

fn request(cookie: &str) -> PolkitRequest {
    let mut details = BTreeMap::new();
    details.insert("polkit.message".to_owned(), "Authentication is required".to_owned());
    PolkitRequest {
        cookie: cookie.to_owned(),
        action_id: "org.freedesktop.policykit.exec".to_owned(),
        message: "Authentication is required to run /usr/bin/true".to_owned(),
        icon_name: "dialog-password".to_owned(),
        details,
        identities: vec![Identity {
            kind: "unix-user".to_owned(),
            details: BTreeMap::new(),
        }],
    }
}

fn agent() -> Agent<Host> {
    Agent::new(Host::default())
}

fn offered(agent: &mut Agent<Host>, cookie: &str) -> RequestId {
    agent
        .begin(request(cookie), Tick::new(15))
        .expect("the slot has its default provider")
        .request
}

#[test]
fn an_answer_after_a_cancel_is_refused() {
    // The failure mode: the user dismissed the prompt, then the provider answers. The
    // answer must not reopen the request or report a decision the user never made.
    let mut agent = agent();
    let request = offered(&mut agent, "c1");
    agent.cancel(request).expect("the request is pending");
    let refusal = agent
        .answer(request, &RequestOutcome::answered(b"unix-user:1000".to_vec()))
        .expect_err("a finished request stays finished");
    assert_eq!(refusal, Refusal::Finished { request });
    assert_eq!(agent.decision(request), Some(&Decision::Cancelled));
}

#[test]
fn a_request_past_its_deadline_is_cancelled() {
    // The failure mode: nothing answers and the request never ends. `expire` is the
    // fail-closed half, and it is a function of the tick the caller supplies.
    let mut agent = agent();
    let request = agent
        .begin(request("c1"), Tick::new(15))
        .expect("the slot has its default provider")
        .request;
    assert!(agent.expire(Tick::new(14)).is_empty(), "not yet due");
    let finished = agent.expire(Tick::new(15));
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].request, request);
    assert_eq!(agent.decision(request), Some(&Decision::Cancelled));
    assert!(
        agent.expire(Tick::new(16)).is_empty(),
        "a finished request is not expired twice"
    );
}

#[test]
fn a_cancel_by_cookie_ends_the_request() {
    // polkitd names its cookie, not our request identity, and a cookie that matches
    // nothing is not an error.
    let mut agent = agent();
    let request = offered(&mut agent, "cookie-7");
    let finished = agent.cancel_cookie("cookie-7").expect("the cookie is pending");
    assert_eq!(finished.request, request);
    assert_eq!(finished.decision, Decision::Cancelled);
    assert!(agent.cancel_cookie("cookie-7").is_none(), "the cancel is idempotent");
    assert!(agent.cancel_cookie("cookie-elsewhere").is_none());
}

#[test]
fn an_agent_registers_once() {
    // The failure mode: two registrations from one process, which would leave two agents
    // answering one session.
    let mut agent = agent();
    agent.register().expect("the first registration");
    assert!(agent.is_registered());
    assert_eq!(
        agent.register().expect_err("the second is refused"),
        Refusal::AlreadyRegistered
    );
}

#[test]
fn an_unregister_cancels_what_the_agent_was_waiting_on() {
    let mut agent = agent();
    agent.register().expect("registered");
    let request = offered(&mut agent, "c1");
    let finished = agent.unregister();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].request, request);
    assert!(!agent.is_registered());
}

#[test]
fn an_unreadable_answer_is_a_refusal() {
    // The failure mode: a provider sends something that is not the slot's encoding. It
    // must never read as consent.
    let mut agent = agent();
    let request = offered(&mut agent, "c1");
    assert_eq!(
        agent
            .answer(request, &RequestOutcome::answered(vec![0xff, 0xfe]))
            .expect("the request is pending"),
        Decision::Denied
    );

    let request = offered(&mut agent, "c2");
    assert_eq!(
        agent
            .answer(request, &RequestOutcome::answered(Vec::new()))
            .expect("the request is pending"),
        Decision::Denied,
        "an empty answer is a refusal, not an empty identity"
    );

    let request = offered(&mut agent, "c3");
    assert_eq!(
        agent
            .answer(request, &RequestOutcome::answered(b"unix-user:1000".to_vec()))
            .expect("the request is pending"),
        Decision::Allowed {
            identity: "unix-user:1000".to_owned()
        }
    );
}

#[test]
fn an_offered_request_carries_the_proposition_and_the_deadline() {
    // What a provider is shown is the request, not just an identity: the slot, the
    // action, the message and the identities are in the offer, and the deadline the
    // request fails at travels with it.
    let mut agent = agent();
    offered(&mut agent, "c1");
    let offers = agent.offerer().offers();
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].0.slot, CapabilitySlot::AuthPolkit);
    assert_eq!(offers[0].0.request.cookie, "c1");
    assert_eq!(
        offers[0].0.request.message,
        "Authentication is required to run /usr/bin/true"
    );
    assert_eq!(offers[0].0.request.identities.len(), 1);
    assert_eq!(offers[0].1, Tick::new(15));
}

#[test]
fn a_timed_out_request_is_a_dismissal() {
    // The connection reports the deadline as an outcome of its own; for this slot it is a
    // provider that went away, which is the same refusal as an ended request.
    let mut agent = agent();
    let request = offered(&mut agent, "c1");
    assert_eq!(
        agent
            .answer(request, &RequestOutcome::TimedOut)
            .expect("the request is pending"),
        Decision::Cancelled
    );
}

#[test]
fn the_slot_names_round_trip_through_their_spellings() {
    // The registry is published, logged and configured by name, so a name that does not
    // parse back is a slot no client can name.
    for slot in CapabilitySlot::ALL {
        assert_eq!(slot.name().parse::<CapabilitySlot>(), Ok(slot));
        assert_eq!(
            Provider::parse(slot.default_provider().as_str()),
            Some(slot.default_provider())
        );
    }
    assert!("auth.polkitx".parse::<CapabilitySlot>().is_err());
    assert_eq!(Provider::parse("somebody"), None);
    let defaults = Slots::new();
    assert_eq!(defaults.provider(CapabilitySlot::PromptSecret), Provider::None);
    assert_eq!(defaults.fields().len(), CapabilitySlot::ALL.len());
}
