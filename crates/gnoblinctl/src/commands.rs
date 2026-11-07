//! The four things a command does to the object graph, over any transport.
//!
//! Every action is a subscription and, for a call, one method call: the IPC has no
//! "read one object" member, because a snapshot plus diffs is the only way state
//! leaves the graph (design/rust-compositor.md, "One IPC"). `list` and `get`
//! therefore subscribe, print what the snapshot holds and close the connection;
//! `watch` keeps reading; `call` sends the method and waits for the operation's one
//! completion.
//!
//! Nothing here reads a clock or decides policy. The exit code it returns is the
//! contract the tests check.

use std::collections::BTreeMap;
use std::io::{self, Write};

use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_model::{Filter, ObjectId, ObjectView, Observation, TypeKey};

use crate::cli::ObjectAction;
use crate::output;
use crate::tree::CliObject;

/// The error code the wire uses for an identity no live object has.
const OBJECT_UNKNOWN: &str = gnoblin_client::code::OBJECT_UNKNOWN;

/// The error code for a member the compositor does not serve.
const UNKNOWN_MEMBER: &str = gnoblin_client::code::UNKNOWN_MEMBER;

/// Run one object action against a connected client.
pub(crate) fn execute<T: Transport>(
    client: &mut Client<T>,
    object: &'static CliObject,
    action: &ObjectAction,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let outcome = match action {
        ObjectAction::List => list(client, object, json, out),
        ObjectAction::Get { id, property } => get(client, object, *id, property.as_deref(), json, out),
        ObjectAction::Watch { id, count } => watch(client, object, *id, *count, json, out),
        ObjectAction::Call { id, method, arguments } => call(client, *id, method.name, arguments, json, out),
    };
    match outcome {
        Ok(code) => code,
        Err(failure) => failure.report(err, json),
    }
}

/// `list`: every live object of the type, from the snapshot.
fn list<T: Transport>(
    client: &mut Client<T>,
    object: &'static CliObject,
    json: bool,
    out: &mut dyn Write,
) -> Result<u8, Failure> {
    let snapshot = client.subscribe(Filter::All).map_err(Failure::Client)?;
    let key = object.type_key();
    let views: Vec<&ObjectView> = snapshot.objects().iter().filter(|view| view.ty() == &key).collect();
    output::view_list(out, object, &views, json).map_err(Failure::Io)?;
    Ok(0)
}

/// `get`: one object, or one property of it.
fn get<T: Transport>(
    client: &mut Client<T>,
    object: &'static CliObject,
    raw: u64,
    property: Option<&str>,
    json: bool,
    out: &mut dyn Write,
) -> Result<u8, Failure> {
    let id = ObjectId::new(raw);
    let snapshot = client.subscribe(Filter::Object(id)).map_err(Failure::Client)?;
    let Some(view) = snapshot.object(id) else {
        return Err(Failure::Refusal {
            code: OBJECT_UNKNOWN.to_owned(),
            message: format!("no live object has identity {raw}"),
        });
    };
    if view.ty() != &object.type_key() {
        // The identity exists but names another type, so there is no object of the
        // type that was asked for. Saying which type it is beats a bare "unknown".
        return Err(Failure::Refusal {
            code: OBJECT_UNKNOWN.to_owned(),
            message: format!("identity {raw} is a {}, not a {}", view.ty(), object.type_key()),
        });
    }
    let Some(property) = property else {
        output::view(out, object, view, json).map_err(Failure::Io)?;
        return Ok(0);
    };
    if output::secret(object, property) {
        output::redacted(out, json).map_err(Failure::Io)?;
        return Ok(0);
    }
    let Some(value) = view.property(property) else {
        return Err(Failure::Refusal {
            code: UNKNOWN_MEMBER.to_owned(),
            message: format!("the compositor publishes no `{property}` on {}", object.type_key()),
        });
    };
    output::property(out, view, property, value, json).map_err(Failure::Io)?;
    Ok(0)
}

/// `watch`: observations for the type, in the order the compositor sent them.
fn watch<T: Transport>(
    client: &mut Client<T>,
    object: &'static CliObject,
    raw: Option<u64>,
    count: Option<u64>,
    json: bool,
    out: &mut dyn Write,
) -> Result<u8, Failure> {
    let filter = match raw {
        Some(raw) => Filter::Object(ObjectId::new(raw)),
        None => Filter::All,
    };
    let snapshot = client.subscribe(filter).map_err(Failure::Client)?;
    if let Filter::Object(id) = filter
        && snapshot.object(id).is_none()
    {
        // An identity is allocated when an object opens and never reused, so a
        // subscription to one that is not live would wait for a change that cannot
        // come.
        return Err(Failure::Refusal {
            code: OBJECT_UNKNOWN.to_owned(),
            message: format!("no live object has identity {}", id.get()),
        });
    }
    // Only `opened` carries a type; every other observation carries an identity
    // alone. The snapshot's identities are therefore the only way to tell a
    // window's change from a session's, and an object that opens later joins the
    // map as its `opened` arrives.
    let mut types: BTreeMap<ObjectId, TypeKey> = snapshot
        .objects()
        .iter()
        .map(|view| (view.id(), view.ty().clone()))
        .collect();
    let key = object.type_key();
    let mut printed = 0;
    loop {
        if count.is_some_and(|limit| printed >= limit) {
            return Ok(0);
        }
        match client.next_event().map_err(Failure::Client)? {
            Event::Model(observation) => {
                if let Observation::Opened { view, .. } = &observation {
                    types.insert(view.id(), view.ty().clone());
                }
                if types.get(&observation.object()) != Some(&key) {
                    continue;
                }
                if let Observation::Changed { property, .. } = &observation
                    && output::secret(object, property)
                {
                    continue;
                }
                output::observation(out, object, &observation, json).map_err(Failure::Io)?;
                printed += 1;
            }
            // A request is offered to every subscriber and answered by one, which is
            // not a change to the objects this command was asked about. An answer only
            // ever reaches the connection that offered the request, so `gnoblinctl`
            // never sees one. An extension event goes only to the connections granted
            // that extension, and `gnoblinctl` is never granted one.
            Event::RequestOffered { .. }
            | Event::RequestAnswered { .. }
            | Event::RequestFinished { .. }
            | Event::Extension { .. } => {}
        }
    }
}

/// `call`: one method, and the single completion of the operation it returns.
fn call<T: Transport>(
    client: &mut Client<T>,
    raw: u64,
    method: &str,
    arguments: &gnoblin_model::Fields,
    json: bool,
    out: &mut dyn Write,
) -> Result<u8, Failure> {
    let id = ObjectId::new(raw);
    // A completion is an observation, and observations only reach a subscriber, so
    // the call subscribes first: without this the compositor would finish the
    // operation and this command would wait for a frame nobody is sent.
    client.subscribe(Filter::Object(id)).map_err(Failure::Client)?;
    let operation = client.call(id, method, arguments).map_err(Failure::Client)?;
    let outcome = client.await_operation(operation).map_err(Failure::Client)?;
    output::operation(out, id, operation.id(), &outcome, json).map_err(Failure::Io)?;
    Ok(if outcome.is_done() { 0 } else { 1 })
}

/// Something that stopped an action.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The compositor refused a member, closed the connection, or sent a payload
    /// this build cannot read.
    Client(ClientError),
    /// Writing the output failed, most often because the reader went away.
    Io(io::Error),
    /// This build refused what the compositor's answer means.
    Refusal { code: String, message: String },
}

impl Failure {
    /// Print the failure on standard error and return its exit code.
    pub(crate) fn report(self, err: &mut dyn Write, json: bool) -> u8 {
        let (code, message) = match self {
            Failure::Client(error) => match error {
                ClientError::Refused { code, message, .. } => (code, message),
                ClientError::Disconnected => (
                    "disconnected".to_owned(),
                    "the compositor closed the connection".to_owned(),
                ),
                ClientError::Codec(error) => ("io".to_owned(), error.to_string()),
                ClientError::Idle => ("idle".to_owned(), "the compositor had nothing to deliver".to_owned()),
                ClientError::Protocol { member, reason } => ("protocol".to_owned(), format!("`{member}`: {reason}")),
            },
            Failure::Io(error) => ("io".to_owned(), error.to_string()),
            Failure::Refusal { code, message } => (code, message),
        };
        let _ = output::error(err, json, &code, &message);
        1
    }
}
