//! The slot registry on the wire: the properties it publishes.
//!
//! Everything here goes through the real client and the in-process link, which encodes
//! and decodes the same envelopes a socket will: the slot state is published into a
//! `session` object and read back through a subscription.

use std::path::Path;

use gnoblin_client::{Event, Server};
use gnoblin_model::{Fields, Filter, Graph, Observation, TypeKey, Value};
use gnoblin_proto::Version;
use gnoblin_schema::{Catalog, Type};
use gnoblin_service_core::{CapabilitySlot, Slots};

const VERSION: Version = Version::new(0, 2, 0);

fn catalog() -> Catalog {
    Catalog::load_dir_except(
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../gnoblin-schema/schema")),
        &["tree"],
    )
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
    properties.insert("lock_state".to_owned(), Value::Text("unlocked".to_owned()));
    properties.insert("lock_available".to_owned(), Value::Bool(false));
    properties.insert("lock_revision".to_owned(), Value::Uint(0));
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
