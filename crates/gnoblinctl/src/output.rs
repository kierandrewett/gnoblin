//! What a command prints: text for a person, JSON for a script.
//!
//! Design/rust-compositor.md, "One IPC": "JSON stays at the edges for people and
//! scripts: `gnoblinctl --json` prints JSON". So the model's own vocabulary is
//! printed as it is, one JSON object per line where a stream is possible (`watch`)
//! and one document where it is not, and nothing here renames a member.
//!
//! A property the schema marks `secret` is never printed, in either form. The
//! compositor does not publish such a value, and a command line that printed one
//! if it arrived would be the second place secrecy has to be right.

use std::io::{self, Write};

use gnoblin_model::{ObjectId, ObjectView, Observation, OperationOutcome};
use serde_json::{Map, Value as Json, json};

use crate::tree::CliObject;
use crate::value;

/// One error line: a code and a sentence, on standard error.
pub(crate) fn error(err: &mut dyn Write, json: bool, code: &str, message: &str) -> io::Result<()> {
    if json {
        writeln!(err, "{}", json!({"error": {"code": code, "message": message}}))
    } else {
        writeln!(err, "gnoblinctl: {code}: {message}")
    }
}

/// Whether the schema marks a property secret, and so unpublishable.
pub(crate) fn secret(object: &CliObject, property: &str) -> bool {
    object.property(property).is_some_and(|declared| declared.secret)
}

/// A value that must not be shown.
pub(crate) fn redacted(out: &mut dyn Write, json: bool) -> io::Result<()> {
    if json {
        writeln!(out, "{}", json!({"redacted": true}))
    } else {
        writeln!(out, "[redacted]")
    }
}

/// Every object of the type, from the subscription snapshot.
pub(crate) fn view_list(out: &mut dyn Write, object: &CliObject, views: &[&ObjectView], json: bool) -> io::Result<()> {
    if json {
        let objects: Vec<Json> = views.iter().map(|view| view_json(object, view)).collect();
        return writeln!(out, "{}", json!({ "objects": objects }));
    }
    for view in views {
        writeln!(
            out,
            "{}\t{}\t{}",
            view.id().get(),
            view.ty(),
            properties_text(object, view)
        )?;
    }
    Ok(())
}

/// One object.
pub(crate) fn view(out: &mut dyn Write, object: &CliObject, view: &ObjectView, json: bool) -> io::Result<()> {
    if json {
        return writeln!(out, "{}", view_json(object, view));
    }
    for (name, value) in view.properties() {
        if secret(object, name) {
            continue;
        }
        writeln!(out, "{name} = {}", value::display(value))?;
    }
    Ok(())
}

/// One property of one object, as its bare value.
pub(crate) fn property(
    out: &mut dyn Write,
    view: &ObjectView,
    name: &str,
    value: &gnoblin_model::Value,
    json: bool,
) -> io::Result<()> {
    if json {
        return writeln!(
            out,
            "{}",
            json!({"id": view.id().get(), "property": name, "value": value::to_json(value)})
        );
    }
    writeln!(out, "{}", value::display(value))
}

/// One observation, as `watch` prints it.
pub(crate) fn observation(
    out: &mut dyn Write,
    object: &CliObject,
    observation: &Observation,
    json: bool,
) -> io::Result<()> {
    if json {
        return writeln!(out, "{}", observation_json(object, observation));
    }
    match observation {
        Observation::Opened { view, revision } => writeln!(
            out,
            "@{} opened {}\t{}\t{}",
            revision.get(),
            view.id().get(),
            view.ty(),
            properties_text(object, view)
        ),
        Observation::Changed {
            object: id,
            property,
            value,
            revision,
        } => writeln!(
            out,
            "@{} changed {} {property} = {}",
            revision.get(),
            id.get(),
            value::display(value)
        ),
        Observation::Signalled {
            object: id,
            signal,
            fields,
            revision,
        } => {
            let fields = fields_text(object, fields.iter().map(|(name, value)| (name.as_str(), value)));
            writeln!(out, "@{} signalled {} {signal} {fields}", revision.get(), id.get())
        }
        Observation::Closed { object: id, revision } => writeln!(out, "@{} closed {}", revision.get(), id.get()),
        Observation::Finished {
            operation,
            object: id,
            outcome,
            revision,
        } => writeln!(
            out,
            "@{} finished {} operation {} {}",
            revision.get(),
            id.get(),
            operation.get(),
            outcome_text(outcome)
        ),
    }
}

/// A method call's operation and how it ended.
pub(crate) fn operation(
    out: &mut dyn Write,
    object: ObjectId,
    id: gnoblin_model::OperationId,
    outcome: &OperationOutcome,
    json: bool,
) -> io::Result<()> {
    if json {
        let mut document = Map::new();
        document.insert("object".to_owned(), Json::from(object.get()));
        document.insert("operation".to_owned(), Json::from(id.get()));
        document.insert("outcome".to_owned(), Json::from(outcome_kind(outcome)));
        if let OperationOutcome::Failed(code) = outcome {
            document.insert("code".to_owned(), Json::from(code.as_str()));
        }
        return writeln!(out, "{}", Json::Object(document));
    }
    writeln!(out, "operation {}: {}", id.get(), outcome_text(outcome))
}

/// One object as JSON.
fn view_json(object: &CliObject, view: &ObjectView) -> Json {
    json!({
        "id": view.id().get(),
        "type": view.ty().to_string(),
        "properties": properties_json(object, view.properties().iter().map(|(name, value)| (name.as_str(), value))),
    })
}

/// One observation as JSON, with the same members the text form names.
fn observation_json(object: &CliObject, observation: &Observation) -> Json {
    let revision = observation.revision().get();
    match observation {
        Observation::Opened { view, .. } => json!({
            "revision": revision,
            "kind": "opened",
            "object": view.id().get(),
            "type": view.ty().to_string(),
            "properties": properties_json(object, view.properties().iter().map(|(name, value)| (name.as_str(), value))),
        }),
        Observation::Changed {
            object: id,
            property,
            value,
            ..
        } => json!({
            "revision": revision,
            "kind": "changed",
            "object": id.get(),
            "property": property,
            "value": value::to_json(value),
        }),
        Observation::Signalled {
            object: id,
            signal,
            fields,
            ..
        } => json!({
            "revision": revision,
            "kind": "signalled",
            "object": id.get(),
            "signal": signal,
            "fields": properties_json(object, fields.iter().map(|(name, value)| (name.as_str(), value))),
        }),
        Observation::Closed { object: id, .. } => json!({
            "revision": revision,
            "kind": "closed",
            "object": id.get(),
        }),
        Observation::Finished {
            operation,
            object: id,
            outcome,
            ..
        } => {
            let mut document = Map::new();
            document.insert("revision".to_owned(), Json::from(revision));
            document.insert("kind".to_owned(), Json::from("finished"));
            document.insert("object".to_owned(), Json::from(id.get()));
            document.insert("operation".to_owned(), Json::from(operation.get()));
            document.insert("outcome".to_owned(), Json::from(outcome_kind(outcome)));
            if let OperationOutcome::Failed(code) = outcome {
                document.insert("code".to_owned(), Json::from(code.as_str()));
            }
            Json::Object(document)
        }
    }
}

/// Every publishable property of an object, as a JSON map.
fn properties_json<'a>(object: &CliObject, values: impl Iterator<Item = (&'a str, &'a gnoblin_model::Value)>) -> Json {
    let mut properties = Map::new();
    for (name, value) in values {
        if secret(object, name) {
            continue;
        }
        properties.insert(name.to_owned(), value::to_json(value));
    }
    Json::Object(properties)
}

/// Every publishable property of an object, as `name=value` pairs.
fn properties_text(object: &CliObject, view: &ObjectView) -> String {
    fields_text(
        object,
        view.properties().iter().map(|(name, value)| (name.as_str(), value)),
    )
}

/// Named values as `name=value` pairs, with a secret one left out.
fn fields_text<'a>(object: &CliObject, values: impl Iterator<Item = (&'a str, &'a gnoblin_model::Value)>) -> String {
    let pairs: Vec<String> = values
        .filter(|(name, _)| !secret(object, name))
        .map(|(name, value)| format!("{name}={}", value::display(value)))
        .collect();
    pairs.join(" ")
}

/// Whether an operation ended well, as one JSON word.
fn outcome_kind(outcome: &OperationOutcome) -> &'static str {
    if outcome.is_done() { "done" } else { "failed" }
}

/// How an operation ended, in the words the text form uses.
fn outcome_text(outcome: &OperationOutcome) -> String {
    match outcome {
        OperationOutcome::Done => "done".to_owned(),
        OperationOutcome::Value(_) => "value".to_owned(),
        OperationOutcome::Failed(code) => format!("failed ({code})"),
    }
}
