//! The events the host raises from a property change.
//!
//! `derived_events.toml` declares each one. `build.rs` turns that file into the
//! [`DERIVED_EVENTS`] table below, and adds every event to the schema catalog as a
//! signal of its object, so the bindings, the stub and the docs-parity gate see it like
//! any other signal. The host raises an event when the property it names changes to the
//! value it names, and builds the payload from the object as it is after the change.

use gnoblin_model::{Fields, ObjectId, ObjectView, Value as ModelValue};
use gnoblin_schema::derived::DerivedSource;

/// One event the host raises when a boolean property of an object changes to a value.
pub(crate) struct RaisedEvent {
    /// The object type that raises the event.
    pub(crate) object: &'static str,
    /// The signal name, as scripts subscribe to it.
    pub(crate) signal: &'static str,
    /// The boolean property whose change raises the event.
    pub(crate) property: &'static str,
    /// The value the property must change to for the event to fire.
    pub(crate) when: bool,
    /// The payload, in declaration order.
    pub(crate) fields: &'static [RaisedField],
}

/// One field of a derived event's payload and where its value comes from.
pub(crate) struct RaisedField {
    /// The field name.
    pub(crate) name: &'static str,
    /// The value the host copies into the field.
    pub(crate) from: DerivedSource<&'static str>,
}

include!(concat!(env!("OUT_DIR"), "/derived_events.rs"));

/// The payload of `event` for `object`, read from its view after the change.
///
/// A field whose property the view does not hold is left out, which the argument table
/// reports the same way it reports any other missing field.
pub(crate) fn payload(object: ObjectId, event: &RaisedEvent, view: &ObjectView) -> Fields {
    event
        .fields
        .iter()
        .filter_map(|field| {
            let value = match &field.from {
                DerivedSource::Object => ModelValue::Object(object),
                DerivedSource::Property(name) => view.properties().get(*name)?.clone(),
            };
            Some((field.name.to_owned(), value))
        })
        .collect()
}
