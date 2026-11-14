//! The shell's connection for the session: `gnoblin.session`, `gnoblin.runtime.reloadConfig` and the schema's
//! signals for `gnoblin.on`.
//!
//! The owner socket's `gnoblin` namespace is where the `session` object lives, and it does not travel on the tree
//! socket, so the shell holds one more owner connection for it, as it does for slots and for the configuration
//! document. [`JsClientSessionLink`] gives that connection a thread of its own:
//!
//! - the thread subscribes to the whole object graph, keeps the `session` object's lock properties in a shared
//!   status (so [`JsSessionLink::status`] answers at once, as of the last thing the compositor said), and forwards
//!   every signal to the shell as an event;
//! - a verb the script calls (`logout`, `poweroff`, `reboot`, `suspend`, `hibernate`) is one `call` on that object.
//!   The compositor answers with an operation that finishes later, so the call returns a ticket and the end comes
//!   back as an event. Nothing in the frame waits for logind;
//! - `lock` and `reload-config` are not methods of the object: the first takes the lock node of the tree, and the
//!   second asks the runtime to reload, which only the runtime's own turn can answer. Each runs on a connection of
//!   its own, in its own short thread, so a reload that waits for this very process cannot hold the status thread.
//!
//! A test supplies its own [`JsSessionLink`] and so runs without a compositor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, Event};
use gnoblin_model::{Fields, Filter, ObjectId, ObjectView, Observation, OperationId, OperationOutcome, Value};
use gnoblin_proto::Version;
use serde_json::{Map, Value as Json, json};

use crate::config::JsLinkError;

gnoblin_log::logger!(log = "js");

/// How long the status thread waits for a frame before it looks at its commands again.
const POLL: Duration = Duration::from_millis(25);

/// How long a reload may take before the connection that asked gives up. The runtime answers when its turn comes
/// and the compositor has accepted the document, so a longer wait is a runtime that is stuck.
const RELOAD_WAIT: Duration = Duration::from_secs(60);

/// What a script can ask of the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsSessionVerb {
    /// End the session.
    Logout,
    /// Power the machine off.
    Poweroff,
    /// Reboot the machine.
    Reboot,
    /// Suspend the machine to RAM.
    Suspend,
    /// Hibernate the machine to disk.
    Hibernate,
    /// Take the session lock.
    Lock,
    /// Reload the configuration as `gnoblinctl config reload` does.
    ReloadConfig,
}

impl JsSessionVerb {
    /// The verbs a script spells as a name, in the order the module lists them.
    pub const ALL: [(&'static str, Self); 7] = [
        ("logout", Self::Logout),
        ("poweroff", Self::Poweroff),
        ("reboot", Self::Reboot),
        ("suspend", Self::Suspend),
        ("hibernate", Self::Hibernate),
        ("lock", Self::Lock),
        ("reload-config", Self::ReloadConfig),
    ];

    /// The verb a script's name stands for.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, verb)| *verb)
    }

    /// The method of the `session` object this verb calls, for the verbs the object serves.
    fn method(self) -> Option<&'static str> {
        match self {
            Self::Logout => Some("logout"),
            Self::Poweroff => Some("poweroff"),
            Self::Reboot => Some("reboot"),
            Self::Suspend => Some("suspend"),
            Self::Hibernate => Some("hibernate"),
            Self::Lock | Self::ReloadConfig => None,
        }
    }
}

/// The session as the compositor last published it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsSessionStatus {
    /// Whether the session has published a lock state yet.
    pub lock_available: bool,
    /// How many lock state changes it has published.
    pub revision: u64,
    /// `unlocked`, `covering`, `locked` or `failsafe`, once `lock_available`.
    pub lock_state: Option<String>,
}

/// A refusal or failure of an operation: a stable code (`docs/shell-api/errors.md`) and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsOperationError {
    /// `denied`, `unsupported`, `busy`, `unavailable`, `timed_out`, `internal`, `config_rejected` and so on.
    pub code: String,
    /// What the compositor or the system said.
    pub message: String,
}

/// What came back on the session connection since the last frame.
#[derive(Clone, Debug, PartialEq)]
pub enum JsSessionEvent {
    /// A verb the script called has finished. `ticket` is what the call returned.
    Finished {
        /// The ticket [`JsSessionLink::begin`] returned.
        ticket: u64,
        /// The operation's value, as JSON, or its refusal.
        outcome: Result<Json, JsOperationError>,
    },
    /// The compositor raised a signal on an object.
    Signal {
        /// `namespace.object.signal`, the schema spelling.
        name: String,
        /// The identity of the object that raised it.
        object: u64,
        /// The signal's fields, by schema name.
        fields: Map<String, Json>,
    },
    /// The connection is gone. Nothing more arrives; every verb is refused `unavailable` from here on.
    Lost(String),
}

/// The shell's connection for the session.
pub trait JsSessionLink {
    /// The session's state as the link last learned it.
    fn status(&self) -> JsSessionStatus;
    /// Starts `verb` and returns the ticket its end will carry.
    ///
    /// # Errors
    ///
    /// The connection is gone, or the session object is not there to call.
    fn begin(&mut self, verb: JsSessionVerb) -> Result<u64, JsLinkError>;
    /// What arrived since the last call, oldest first.
    fn poll(&mut self) -> Vec<JsSessionEvent>;
}

/// The code a compositor refusal is reported under: the wire's kebab-case, spelled as the docs spell it.
fn script_code(wire: &str) -> String {
    wire.replace('-', "_")
}

/// A value of the model as JSON, for a signal's field or an operation's result.
pub(crate) fn value_to_json(value: &Value) -> Json {
    match value {
        Value::Bool(flag) => json!(flag),
        Value::Int(number) => json!(number),
        Value::Uint(number) => json!(number),
        Value::Double(number) => json!(number),
        Value::Text(text) => json!(text),
        Value::Bytes(bytes) => json!(bytes),
        Value::Object(id) => json!(id.get()),
        Value::List(items) => Json::Array(items.iter().map(value_to_json).collect()),
    }
}

fn fields_to_json(fields: &Fields) -> Map<String, Json> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), value_to_json(value)))
        .collect()
}

/// How an operation's outcome reaches the script.
fn outcome_to_json(outcome: &OperationOutcome) -> Result<Json, JsOperationError> {
    match outcome {
        OperationOutcome::Done => Ok(json!({ "accepted": true })),
        OperationOutcome::Value(value) => Ok(value_to_json(value)),
        OperationOutcome::Failed(code) => Err(JsOperationError {
            code: script_code(code.as_str()),
            message: format!("the compositor refused the request: {}", code.as_str()),
        }),
    }
}

/// A client's failure as the refusal a script sees.
fn client_error(error: &ClientError) -> JsOperationError {
    match error {
        ClientError::Refused { code, message, .. } => JsOperationError {
            code: script_code(code),
            message: message.clone(),
        },
        ClientError::Disconnected | ClientError::Codec(_) => JsOperationError {
            code: "unavailable".to_owned(),
            message: error.to_string(),
        },
        other => JsOperationError {
            code: "internal".to_owned(),
            message: other.to_string(),
        },
    }
}

/// What the status thread knows of the graph: each object's type, and the session's properties.
#[derive(Default)]
struct JsSessionGraph {
    types: HashMap<ObjectId, (String, String)>,
    session: Option<ObjectId>,
}

impl JsSessionGraph {
    fn learn(&mut self, view: &ObjectView, status: &Mutex<JsSessionStatus>) {
        let (namespace, name) = (view.ty().namespace().to_owned(), view.ty().name().to_owned());
        if namespace == "gnoblin" && name == "session" {
            self.session = Some(view.id());
            for (property, value) in view.properties() {
                apply_property(status, property, value);
            }
        }
        self.types.insert(view.id(), (namespace, name));
    }
}

/// Folds one published property of the `session` object into the shared status.
fn apply_property(status: &Mutex<JsSessionStatus>, property: &str, value: &Value) {
    let Ok(mut status) = status.lock() else {
        return;
    };
    match (property, value) {
        ("lock_available", Value::Bool(available)) => status.lock_available = *available,
        ("lock_revision", Value::Uint(revision)) => status.revision = *revision,
        ("lock_state", Value::Text(state)) => status.lock_state = Some(state.clone()),
        _ => {}
    }
}

enum JsSessionCommand {
    Call { ticket: u64, method: &'static str },
}

/// The status thread's loop: subscribe, then serve commands and forward events until the connection ends or the
/// link is dropped.
fn serve(
    mut client: Client<UnixTransport>,
    commands: &Receiver<JsSessionCommand>,
    events: &Sender<JsSessionEvent>,
    status: &Mutex<JsSessionStatus>,
    stop: &AtomicBool,
    graph_ready: &Sender<Result<(), JsLinkError>>,
) {
    let mut graph = JsSessionGraph::default();
    match client.subscribe(Filter::All) {
        Ok(snapshot) => {
            for view in snapshot.objects() {
                graph.learn(view, status);
            }
            let _ = graph_ready.send(Ok(()));
        }
        Err(error) => {
            let _ = graph_ready.send(Err(JsLinkError(error.to_string())));
            return;
        }
    }
    let mut pending: HashMap<OperationId, u64> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        while let Ok(JsSessionCommand::Call { ticket, method }) = commands.try_recv() {
            let Some(session) = graph.session else {
                let _ = events.send(JsSessionEvent::Finished {
                    ticket,
                    outcome: Err(JsOperationError {
                        code: "unavailable".to_owned(),
                        message: "the compositor publishes no session object".to_owned(),
                    }),
                });
                continue;
            };
            match client.call(session, method, &Fields::new()) {
                Ok(operation) => {
                    pending.insert(operation.id(), ticket);
                }
                Err(error) => {
                    let _ = events.send(JsSessionEvent::Finished {
                        ticket,
                        outcome: Err(client_error(&error)),
                    });
                }
            }
        }
        match client.next_event() {
            Ok(Event::Model(observation)) => match observation {
                Observation::Opened { view, .. } => graph.learn(&view, status),
                Observation::Changed {
                    object,
                    property,
                    value,
                    ..
                } if graph.session == Some(object) => apply_property(status, &property, &value),
                Observation::Closed { object, .. } => {
                    graph.types.remove(&object);
                    if graph.session == Some(object) {
                        graph.session = None;
                    }
                }
                Observation::Signalled {
                    object, signal, fields, ..
                } => {
                    if let Some((namespace, name)) = graph.types.get(&object) {
                        let _ = events.send(JsSessionEvent::Signal {
                            name: format!("{namespace}.{name}.{signal}"),
                            object: object.get(),
                            fields: fields_to_json(&fields),
                        });
                    }
                }
                Observation::Finished { operation, outcome, .. } => {
                    if let Some(ticket) = pending.remove(&operation) {
                        let _ = events.send(JsSessionEvent::Finished {
                            ticket,
                            outcome: outcome_to_json(&outcome),
                        });
                    }
                }
                Observation::Changed { .. } => {}
            },
            Ok(_) => {}
            Err(ClientError::Idle) => {}
            Err(error) => {
                let _ = events.send(JsSessionEvent::Lost(error.to_string()));
                return;
            }
        }
    }
}

/// [`JsSessionLink`] over the owner socket.
pub struct JsClientSessionLink {
    path: PathBuf,
    version: Version,
    status: Arc<Mutex<JsSessionStatus>>,
    commands: Sender<JsSessionCommand>,
    events: Receiver<JsSessionEvent>,
    /// Where the short threads of `lock` and `reload-config` report. They own a clone.
    late: Sender<JsSessionEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    next_ticket: u64,
    lost: bool,
}

impl JsClientSessionLink {
    /// Connects to the owner socket at `path`, subscribes and reads the first snapshot.
    ///
    /// # Errors
    ///
    /// The socket cannot be reached, the handshake or the subscription is refused.
    pub fn connect(path: &Path, version: Version) -> Result<Self, JsLinkError> {
        let mut client = connect_client(path, version)?;
        client
            .set_read_timeout(Some(POLL))
            .map_err(|error| JsLinkError(error.to_string()))?;
        let status = Arc::new(Mutex::new(JsSessionStatus::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (commands, command_rx) = mpsc::channel();
        let (late, events) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let (status, stop, late) = (Arc::clone(&status), Arc::clone(&stop), late.clone());
            std::thread::Builder::new()
                .name("js-session".to_owned())
                .spawn(move || serve(client, &command_rx, &late, &status, &stop, &ready_tx))
                .map_err(|error| JsLinkError(format!("cannot start the session thread: {error}")))?
        };
        // The first snapshot is read before `connect` returns, so `status` never answers the empty default of a
        // thread that has not run yet.
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(JsLinkError("the compositor sent no snapshot of its objects".to_owned())),
        }
        Ok(Self {
            path: path.to_path_buf(),
            version,
            status,
            commands,
            events,
            late,
            stop,
            thread: Some(thread),
            next_ticket: 1,
            lost: false,
        })
    }

    /// Runs `job` on a connection of its own and reports its end under `ticket`.
    fn detached(
        &self,
        ticket: u64,
        job: impl FnOnce(&mut Client<UnixTransport>) -> Result<Json, ClientError> + Send + 'static,
    ) -> Result<(), JsLinkError> {
        let (path, version, late) = (self.path.clone(), self.version, self.late.clone());
        std::thread::Builder::new()
            .name("js-session-call".to_owned())
            .spawn(move || {
                let outcome = connect_client(&path, version)
                    .map_err(|error| JsOperationError {
                        code: "unavailable".to_owned(),
                        message: error.0,
                    })
                    .and_then(|mut client| {
                        client
                            .set_read_timeout(Some(RELOAD_WAIT))
                            .map_err(|error| client_error(&error))?;
                        job(&mut client).map_err(|error| client_error(&error))
                    });
                let _ = late.send(JsSessionEvent::Finished { ticket, outcome });
            })
            .map(drop)
            .map_err(|error| JsLinkError(format!("cannot start the call: {error}")))
    }
}

fn connect_client(path: &Path, version: Version) -> Result<Client<UnixTransport>, JsLinkError> {
    let transport = UnixTransport::connect(path)
        .map_err(|error| JsLinkError(format!("cannot connect to {}: {error}", path.display())))?;
    Client::connect(transport, version).map_err(|error| JsLinkError(error.to_string()))
}

impl JsSessionLink for JsClientSessionLink {
    fn status(&self) -> JsSessionStatus {
        self.status.lock().map(|status| status.clone()).unwrap_or_default()
    }

    fn begin(&mut self, verb: JsSessionVerb) -> Result<u64, JsLinkError> {
        if self.lost {
            return Err(JsLinkError("the session connection is gone".to_owned()));
        }
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        match verb {
            JsSessionVerb::ReloadConfig => self.detached(ticket, |client| {
                let committed = client.reload_config()?;
                Ok(json!({
                    "ok": true,
                    "action": "config reload",
                    "settingsRevision": committed.revision,
                    "warnings": Vec::<String>::new(),
                    "notApplied": committed.notices.into_iter().map(|notice| notice.text).collect::<Vec<_>>(),
                }))
            })?,
            JsSessionVerb::Lock => self.detached(ticket, |client| {
                client.watch_lock()?;
                client.lock_session()?;
                Ok(json!({ "dispatched": true }))
            })?,
            other => {
                let Some(method) = other.method() else {
                    return Err(JsLinkError(
                        "this verb is not a method of the session object".to_owned(),
                    ));
                };
                self.commands
                    .send(JsSessionCommand::Call { ticket, method })
                    .map_err(|_| JsLinkError("the session connection is gone".to_owned()))?;
            }
        }
        Ok(ticket)
    }

    fn poll(&mut self) -> Vec<JsSessionEvent> {
        let events: Vec<JsSessionEvent> = self.events.try_iter().collect();
        if events.iter().any(|event| matches!(event, JsSessionEvent::Lost(_))) {
            self.lost = true;
            log::warn!(
                area = "session",
                "the session connection is lost; session calls are refused"
            );
        }
        events
    }
}

impl Drop for JsClientSessionLink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_model::{ObjectId, Value};

    use super::{JsSessionVerb, apply_property, script_code, value_to_json};
    use std::sync::Mutex;

    #[test]
    fn a_verbs_name_round_trips_and_only_five_are_methods() {
        for (name, verb) in JsSessionVerb::ALL {
            assert_eq!(JsSessionVerb::named(name), Some(verb));
        }
        assert_eq!(JsSessionVerb::named("explode"), None);
        let methods = JsSessionVerb::ALL
            .iter()
            .filter(|(_, verb)| verb.method().is_some())
            .count();
        assert_eq!(methods, 5);
    }

    #[test]
    fn wire_codes_are_spelled_as_the_docs_spell_them() {
        assert_eq!(script_code("timed-out"), "timed_out");
        assert_eq!(script_code("config-rejected"), "config_rejected");
        assert_eq!(script_code("denied"), "denied");
    }

    #[test]
    fn the_status_folds_the_sessions_lock_properties_and_ignores_the_rest() {
        let status = Mutex::new(super::JsSessionStatus::default());
        apply_property(&status, "lock_available", &Value::Bool(true));
        apply_property(&status, "lock_revision", &Value::Uint(4));
        apply_property(&status, "lock_state", &Value::Text("locked".to_owned()));
        apply_property(&status, "version", &Value::Text("0.2.0".to_owned()));
        apply_property(&status, "lock_revision", &Value::Text("mistyped".to_owned()));
        let status = status.lock().expect("the status lock").clone();
        assert!(status.lock_available);
        assert_eq!(status.revision, 4);
        assert_eq!(status.lock_state.as_deref(), Some("locked"));
    }

    #[test]
    fn model_values_become_json() {
        let list = Value::List(vec![
            Value::Uint(1),
            Value::Object(ObjectId::new(9)),
            Value::Text("x".into()),
        ]);
        assert_eq!(value_to_json(&list).to_string(), r#"[1,9,"x"]"#);
    }
}
