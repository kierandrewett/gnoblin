//! The slot registry on the wire: the properties it publishes, and a shell answering a
//! request the services role offered.
//!
//! Everything here goes through the real client and the in-process link, which encodes
//! and decodes the same envelopes a socket will: the slot state is published into a
//! `session` object and read back through a subscription, and a polkit request is
//! announced to a shell that claims and answers it.

use std::path::Path;

use gnoblin_client::{Event, Server};
use gnoblin_model::{Fields, Filter, Graph, Observation, RequestOutcome, Tick, TypeKey, Value};
use gnoblin_proto::Version;
use gnoblin_schema::{Catalog, Type};
use gnoblin_service_core::{
    Agent, CapabilitySlot, Decision, Offerer, PolkitRequest, Proposition, Provider, Refusal, Slots,
};
use gnoblin_types::OwnerId;

const VERSION: Version = Version::new(0, 2, 0);

fn catalog() -> Catalog {
    Catalog::load_dir(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gnoblin-schema/schema"
    )))
    .expect("the checked-in schema loads")
}

fn graph() -> Graph {
    Graph::new(catalog())
}

/// The slot properties the checked-in `session` object declares.
fn declared_slot_properties() -> Vec<(String, Type)> {
    let catalog = catalog();
    let namespace = gnoblin_schema::Namespace::parse("gnoblin").expect("namespace");
    let session = catalog
        .objects(&namespace)
        .find(|object| object.name == "session")
        .expect("the session object is in the schema");
    session
        .properties
        .iter()
        .filter(|property| property.name.starts_with("slot_"))
        .map(|property| (property.name.clone(), property.ty.clone()))
        .collect()
}

fn request(cookie: &str) -> PolkitRequest {
    PolkitRequest {
        cookie: cookie.to_owned(),
        action_id: "org.freedesktop.policykit.exec".to_owned(),
        message: "Authentication is required".to_owned(),
        icon_name: "dialog-password".to_owned(),
        details: Default::default(),
        identities: Vec::new(),
    }
}

/// A host that offers through the in-process compositor, as the privileged connection
/// will offer through the real one.
struct LoopbackHost {
    server: Server,
}

impl Offerer for LoopbackHost {
    fn offer(&mut self, proposition: &Proposition, deadline: Tick) -> Result<gnoblin_model::RequestId, Refusal> {
        // The proposition is what the loopback announces with the offer, exactly as the real
        // compositor announces what the privileged connection sent it.
        Ok(self.server.offer(
            proposition.slot.name(),
            &proposition.request.action_id,
            &proposition.request.message,
            &proposition.subject,
            deadline,
        ))
    }
}

#[test]
fn the_published_slots_are_the_session_properties() {
    // The failure mode: the registry publishes a property the schema does not declare, so
    // a subscribing client drops it and a shell sees fewer slots than exist. The whole
    // field set is compared with the schema, both names and types.
    let mut published: Vec<String> = Slots::new().fields().keys().cloned().collect();
    let mut declared: Vec<String> = declared_slot_properties().into_iter().map(|(name, _)| name).collect();
    published.sort();
    declared.sort();
    assert_eq!(published, declared, "the published properties are the declared ones");
    for (name, ty) in declared_slot_properties() {
        assert_eq!(ty, Type::Text, "`{name}` is published as a string");
    }
    for slot in CapabilitySlot::ALL {
        assert!(
            declared.contains(&slot.property().to_owned()),
            "`{}` is published as `{}`",
            slot.name(),
            slot.property()
        );
    }
}

#[test]
fn a_client_learns_who_provides_each_slot() {
    // The failure mode: the session object lists a slot with no provider and a client
    // cannot tell that a prompt will fail closed. The state is published once, then a
    // claim reaches subscribers as the diff for that one property.
    let mut server = Server::new(graph(), VERSION);
    let slots = Slots::new();
    let mut properties = slots.fields();
    properties.insert("version".to_owned(), Value::Text("0.2.0".to_owned()));
    let session = server
        .create(TypeKey::gnoblin("session"), properties)
        .expect("the slot fields are the schema's");

    let mut shell = server.connect(VERSION).expect("versions match");
    let snapshot = shell.subscribe(Filter::All).expect("subscribe");
    let view = &snapshot.objects()[0];
    assert_eq!(view.id(), session);
    assert_eq!(
        view.properties().get("slot_prompt_secret"),
        Some(&Value::Text("none".to_owned())),
        "a prompt has no provider until a shell claims the slot"
    );
    assert_eq!(
        view.properties().get("slot_auth_polkit"),
        Some(&Value::Text("services".to_owned()))
    );

    server
        .set(session, "slot_prompt_secret", Value::Text("shell".to_owned()))
        .expect("the property is declared as a string");
    let observation = shell.next_event().expect("the diff reaches the subscriber");
    match observation {
        Event::Model(Observation::Changed {
            object,
            property,
            value,
            ..
        }) => {
            assert_eq!(object, session);
            assert_eq!(property, "slot_prompt_secret");
            assert_eq!(value, Value::Text("shell".to_owned()));
        }
        other => panic!("expected a property diff, got {other:?}"),
    }
}

#[test]
fn a_shell_claims_and_answers_an_offered_request() {
    // The failure mode this covers is the whole point of the seam: a request the services
    // role offers is announced to a shell, the shell's answer is the request's outcome,
    // and the agent reads that outcome as the user's decision.
    let mut host = LoopbackHost {
        server: Server::new(graph(), VERSION),
    };
    let mut shell = host.server.connect(VERSION).expect("versions match");
    let mut agent = Agent::new(host);
    agent
        .claim(CapabilitySlot::AuthPolkit, Provider::Shell, Some(OwnerId::new(1)))
        .expect("the shell claims the slot");

    let offered = agent
        .begin(request("cookie-1"), Tick::new(15))
        .expect("the slot has a provider");

    let event = shell.next_event().expect("the offer reaches the shell");
    let Event::RequestOffered {
        request,
        deadline,
        slot,
        ..
    } = event
    else {
        panic!("expected an offer, got {event:?}");
    };
    assert_eq!(request, offered.request);
    assert_eq!(deadline, Tick::new(15));
    assert_eq!(slot, "auth.polkit");
    shell.claim(request).expect("the first claim wins");
    shell.answer(request, b"unix-user:1000").expect("the holder answers");

    let outcome = agent.offerer().server.finished(request).expect("the request finished");
    assert_eq!(outcome, RequestOutcome::Answered(b"unix-user:1000".to_vec()));
    let decision = agent.answer(request, &outcome).expect("the request is still pending");
    assert_eq!(
        decision,
        Decision::Allowed {
            identity: "unix-user:1000".to_owned()
        }
    );
}

#[test]
fn two_shells_cannot_claim_one_request() {
    // The failure mode: two connections claim one offer, which would let the loser answer
    // on the winner's behalf. The second claim is refused by the broker, and the agent
    // then refuses the loser's answer too because only the holder answers.
    let mut host = LoopbackHost {
        server: Server::new(graph(), VERSION),
    };
    let mut first = host.server.connect(VERSION).expect("versions match");
    let mut second = host.server.connect(VERSION).expect("versions match");
    let mut agent = Agent::new(host);
    let offered = agent
        .begin(request("cookie-1"), Tick::new(15))
        .expect("the slot has its default provider");

    assert_eq!(
        first.next_event().expect("an offer"),
        Event::RequestOffered {
            request: offered.request,
            deadline: Tick::new(15),
            slot: "auth.polkit".to_owned(),
            title: "org.freedesktop.policykit.exec".to_owned(),
            message: "Authentication is required".to_owned(),
            subject: Vec::new(),
        }
    );
    assert_eq!(
        second.next_event().expect("an offer"),
        Event::RequestOffered {
            request: offered.request,
            deadline: Tick::new(15),
            slot: "auth.polkit".to_owned(),
            title: "org.freedesktop.policykit.exec".to_owned(),
            message: "Authentication is required".to_owned(),
            subject: Vec::new(),
        }
    );
    first.claim(offered.request).expect("the first claim wins");
    let refusal = second.claim(offered.request).expect_err("the second claim is refused");
    assert!(
        refusal.to_string().contains("request"),
        "the refusal names the request: {refusal}"
    );
}

#[test]
fn an_empty_registry_publishes_none_for_a_prompt() {
    // A publication is a value, not a side effect: an empty registry still says which
    // slots exist and what a client should expect from each.
    let fields: Fields = Slots::new().fields();
    assert_eq!(fields.len(), CapabilitySlot::ALL.len());
    let published: Vec<&str> = fields
        .values()
        .map(|value| match value {
            Value::Text(text) => text.as_str(),
            other => panic!("a slot is published as a string, got {other:?}"),
        })
        .collect();
    assert!(published.contains(&"none"));
    assert!(published.contains(&"services"));
}
