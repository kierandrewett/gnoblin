//! Verbs: the calls and signals of the compositor's `gnoblin` namespace (the session, idle, outputs), as the table's `call` and
//! `subscribe` (`design/modular-framework.md` section 5.9).
//!
//! The tree link carries nodes. The `gnoblin` namespace is the owner socket's object graph and it does not travel on the tree
//! link: a verb such as `session.logout` is one `call` on the compositor's object, answered by an operation that finishes
//! later, and a signal such as `session.bell` is an observation of that graph. So the shell instance holds one more connection
//! for them, a [`VerbLink`], opened the first time a shell calls a verb or subscribes to a signal.
//!
//! The core's side of it is small and is here: a call is checked against the catalog (its class, its method, the arguments'
//! kinds) and handed to the link with a token; whatever the link learns comes back through [`answered`] and [`signalled`], from
//! any thread, as the events the shell's registrations already match (`ShellEvent::Answer`, and the same signal delivery
//! `on_signal` serves, with the payload in the schema's field order). So a call's answer and a signal are delivered on the shell
//! thread, in the mailbox's order, and a generation that retires takes its pending calls and subscriptions with it, as every
//! other registration does.
//!
//! The link itself is a trait because the compositor edge decides what it is: [`OwnerSocketLink`] connects to the compositor's
//! owner socket (`$GNOBLIN_IPC_SOCKET`, as every client does) and runs a thread of its own, and a test supplies a link that
//! answers from a script.

use std::collections::{BTreeMap, HashMap};
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use gnoblin_client::{Client, ClientError, Event, UnixTransport};
use gnoblin_model::{Fields, Filter, ObjectId, ObjectView, Observation, OperationId, OperationOutcome, Value};
use gnoblin_proto::Version;
use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};

use crate::catalog::DefaultDef;
use crate::events::{AnswerNotice, ShellEvent, SignalDelivery};
use crate::values::OwnedValue;
use crate::{catalog, log, registry};

/// The tokens a verb's answer is matched on: a counter with the second-highest bit set, so they cannot meet a tree call's token (a
/// small counter) or a flush's (the top bit).
const VERB_TOKEN_BIT: u64 = 1 << 62;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// A token for a call that is about to go out.
pub(crate) fn next_token() -> u64 {
    VERB_TOKEN_BIT | NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// One call on an object of the compositor's `gnoblin` namespace.
#[derive(Clone, Debug, PartialEq)]
pub struct VerbCall {
    /// What the answer is published under ([`answered`]).
    pub token: u64,
    /// The compositor's identity of the object, or zero for the one object of the class.
    pub object: u64,
    /// The object's type as the compositor names it (`session`, `idle`): the class without its namespace.
    pub class: String,
    /// The method's schema name.
    pub method: String,
    /// The arguments by their schema names.
    pub args: Fields,
}

/// Why a verb did not do what it was asked: the compositor's refusal, or the connection's failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerbFailure {
    /// A stable code in the schema's kebab case (`denied`, `unsupported`, `unavailable`).
    pub code: String,
}

/// The shell instance's connection for verbs and the `gnoblin` namespace's signals.
pub trait VerbLink {
    /// Starts `call`. The answer is published with [`answered`] when the compositor gives it, from any thread; an `Err` here is a
    /// call that never went out (the connection is gone), and nothing is published for it.
    ///
    /// # Errors
    ///
    /// The connection is gone.
    fn call(&mut self, call: VerbCall) -> Result<(), String>;
}

/// What the host gives the core to reach the compositor's verbs: a way to open a link, on the shell thread, the first time a shell
/// calls a verb or subscribes to a signal.
pub trait VerbLinkProvider {
    /// Opens the link. A link that cannot be opened fails the call or the subscription that asked, with this text.
    ///
    /// # Errors
    ///
    /// The compositor cannot be reached.
    fn open(&self) -> Result<Box<dyn VerbLink>, String>;
}

/// Publishes the answer to the call that went out under `token`: `Ok` with the operation's value (`None` for a bare `done`), or the
/// failure. Any thread.
pub fn answered(token: u64, outcome: Result<Option<Value>, VerbFailure>) {
    let notice = match outcome {
        Ok(value) => AnswerNotice {
            token,
            result: value.map(|value| json(&value).into_bytes()).unwrap_or_default(),
            error: String::new(),
        },
        Err(failure) => AnswerNotice {
            token,
            result: Vec::new(),
            error: failure.code,
        },
    };
    registry::publish_event(ShellEvent::Answer(notice));
}

/// Publishes a signal `object` of type `class` raised: `signal`, with its `fields` by their schema names. The payload is built in
/// the schema's field order, so a registration made with `subscribe` (or `on_signal`) sees what a tree signal's does. A signal the
/// catalog does not know is dropped: a compositor newer than this core may raise one. Any thread.
pub fn signalled(class: &str, signal: &str, object: u64, fields: &Fields) {
    let Some(class_id) = catalog::class_id(&format!("gnoblin.{class}")) else {
        return;
    };
    let Some(signal_id) = catalog::signal_id(class_id, signal) else {
        return;
    };
    let Some(def) = catalog::class(class_id).and_then(|class| class.signals.get(signal_id as usize - 1)) else {
        return;
    };
    let values = def
        .arg_names
        .split(' ')
        .filter(|name| !name.is_empty())
        .zip(def.args)
        .map(|(name, kind)| {
            fields.get(name).and_then(owned).unwrap_or_else(|| {
                OwnedValue::default_of(*kind, DefaultDef::None).unwrap_or(OwnedValue::Text(String::new()))
            })
        })
        .collect();
    registry::publish(SignalDelivery {
        class: class_id,
        signal: signal_id,
        node: object,
        values,
    });
}

/// A value of the compositor's model as the ABI carries it, `None` for a list the ABI has no kind for.
fn owned(value: &Value) -> Option<OwnedValue> {
    Some(match value {
        Value::Bool(value) => OwnedValue::Bool(*value),
        Value::Int(value) => OwnedValue::Int(*value),
        Value::Uint(value) => OwnedValue::Uint(*value),
        Value::Double(value) => OwnedValue::Double(*value),
        Value::Text(value) => OwnedValue::Text(value.clone()),
        Value::Bytes(value) => OwnedValue::Bytes(value.clone()),
        Value::Object(id) => OwnedValue::Node(id.get()),
        Value::List(items) => {
            let numbers: Option<Vec<f64>> = items
                .iter()
                .map(|item| match item {
                    Value::Int(value) => Some(*value as f64),
                    Value::Uint(value) => Some(*value as f64),
                    Value::Double(value) => Some(*value),
                    _ => None,
                })
                .collect();
            if let Some(numbers) = numbers {
                return Some(OwnedValue::List(numbers));
            }
            let texts: Option<Vec<String>> = items
                .iter()
                .map(|item| match item {
                    Value::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect();
            return texts.map(OwnedValue::Texts);
        }
    })
}

/// The JSON text of a value: what a call's answer carries.
fn json(value: &Value) -> String {
    match value {
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Uint(value) => value.to_string(),
        Value::Double(value) if value.is_finite() => value.to_string(),
        Value::Double(_) => "null".to_owned(),
        Value::Text(text) => json_text(text),
        Value::Bytes(bytes) => format!("[{}]", bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(",")),
        Value::Object(id) => id.get().to_string(),
        Value::List(items) => format!("[{}]", items.iter().map(json).collect::<Vec<_>>().join(",")),
    }
}

fn json_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---- the owner socket ---------------------------------------------------------------------------------------------------

/// How long one read of the socket may wait for the rest of a frame before the thread looks at its commands again.
const FRAME_WAIT: Duration = Duration::from_millis(100);

/// What the compositor's socket gives a [`VerbLinkProvider`] to connect to, found as every client finds it.
pub struct OwnerSocket {
    path: Result<PathBuf, String>,
}

impl OwnerSocket {
    /// The socket named by `$GNOBLIN_IPC_SOCKET`, or by `$XDG_RUNTIME_DIR`, as the environment is when this is called.
    #[must_use]
    pub fn from_environment() -> Self {
        Self {
            path: gnoblin_client::unix::socket_path(|name| std::env::var(name).ok()),
        }
    }

    /// The socket at `path`.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path: Ok(path) }
    }
}

impl VerbLinkProvider for OwnerSocket {
    fn open(&self) -> Result<Box<dyn VerbLink>, String> {
        let path = self.path.as_ref().map_err(Clone::clone)?;
        let transport =
            UnixTransport::connect(path).map_err(|error| format!("cannot connect to {}: {error}", path.display()))?;
        let version = {
            let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
            Version::new(major, minor, patch)
        };
        let mut client = Client::connect(transport, version).map_err(|error| error.to_string())?;
        client
            .set_read_timeout(Some(FRAME_WAIT))
            .map_err(|error| error.to_string())?;
        Ok(Box::new(OwnerSocketLink::start(client)?))
    }
}

/// [`VerbLink`] over a connection to the owner socket, served by a thread of its own: it subscribes to the whole object graph (so
/// it knows each object's type and sees every signal), makes the calls it is given and publishes what comes back.
pub struct OwnerSocketLink {
    commands: Sender<VerbCall>,
    /// Written to wake the thread when a command is waiting.
    wake: OwnedFd,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Set by the thread when the connection ends, so a call after that fails at once.
    lost: Arc<Mutex<Option<String>>>,
}

impl OwnerSocketLink {
    fn start(mut client: Client<UnixTransport>) -> Result<Self, String> {
        let snapshot = client.subscribe(Filter::All).map_err(|error| error.to_string())?;
        let mut graph = ObjectGraph::default();
        for view in snapshot.objects() {
            graph.learn(view);
        }
        let (commands, command_rx) = mpsc::channel();
        let wake = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).map_err(|error| error.to_string())?;
        let thread_wake = wake.try_clone().map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(Mutex::new(None));
        let thread = {
            let (stop, lost) = (Arc::clone(&stop), Arc::clone(&lost));
            std::thread::Builder::new()
                .name("gnoblin-verbs".to_owned())
                .spawn(move || serve(client, graph, &command_rx, &thread_wake, &stop, &lost))
                .map_err(|error| format!("cannot start the verbs thread: {error}"))?
        };
        Ok(Self {
            commands,
            wake,
            stop,
            thread: Some(thread),
            lost,
        })
    }
}

impl VerbLink for OwnerSocketLink {
    fn call(&mut self, call: VerbCall) -> Result<(), String> {
        if let Some(why) = self.lost.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            return Err(format!("the owner connection is gone: {why}"));
        }
        self.commands
            .send(call)
            .map_err(|_| "the owner connection is gone".to_owned())?;
        let _ = rustix::io::write(&self.wake, &1_u64.to_ne_bytes());
        Ok(())
    }
}

impl Drop for OwnerSocketLink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = rustix::io::write(&self.wake, &1_u64.to_ne_bytes());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What the thread knows of the compositor's objects: each one's type.
#[derive(Default)]
struct ObjectGraph {
    types: BTreeMap<ObjectId, (String, String)>,
}

impl ObjectGraph {
    fn learn(&mut self, view: &ObjectView) {
        self.types.insert(
            view.id(),
            (view.ty().namespace().to_owned(), view.ty().name().to_owned()),
        );
    }

    /// The object a call is for: `object` when it is one of that type, or the one object of the type when `object` is zero.
    fn resolve(&self, class: &str, object: u64) -> Result<ObjectId, VerbFailure> {
        let of_type = |(namespace, name): &(String, String)| namespace == "gnoblin" && name == class;
        if object != 0 {
            let id = ObjectId::new(object);
            return match self.types.get(&id) {
                Some(ty) if of_type(ty) => Ok(id),
                Some(_) => Err(VerbFailure {
                    code: "unknown-member".to_owned(),
                }),
                None => Err(VerbFailure {
                    code: "object-unknown".to_owned(),
                }),
            };
        }
        let mut found = self.types.iter().filter(|(_, ty)| of_type(ty)).map(|(id, _)| *id);
        match (found.next(), found.next()) {
            (Some(only), None) => Ok(only),
            (None, _) => Err(VerbFailure {
                code: "object-unknown".to_owned(),
            }),
            (Some(_), Some(_)) => Err(VerbFailure {
                code: "ambiguous".to_owned(),
            }),
        }
    }
}

/// The thread's loop: serve commands and forward the compositor's observations until the connection ends or the link is dropped.
fn serve(
    mut client: Client<UnixTransport>,
    mut graph: ObjectGraph,
    commands: &Receiver<VerbCall>,
    wake: &OwnedFd,
    stop: &AtomicBool,
    lost: &Mutex<Option<String>>,
) {
    let mut pending: HashMap<OperationId, u64> = HashMap::new();
    let end = |why: String, pending: &mut HashMap<OperationId, u64>| {
        log::warn!(area = "verbs", "the owner connection ended: {why}");
        *lost.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
        for (_, token) in pending.drain() {
            answered(
                token,
                Err(VerbFailure {
                    code: "unavailable".to_owned(),
                }),
            );
        }
    };
    while !stop.load(Ordering::Relaxed) {
        while let Ok(call) = commands.try_recv() {
            let object = match graph.resolve(&call.class, call.object) {
                Ok(object) => object,
                Err(failure) => {
                    answered(call.token, Err(failure));
                    continue;
                }
            };
            match client.call(object, &call.method, &call.args) {
                Ok(operation) => {
                    pending.insert(operation.id(), call.token);
                }
                Err(ClientError::Refused { code, .. }) => answered(call.token, Err(VerbFailure { code })),
                Err(error) => {
                    answered(
                        call.token,
                        Err(VerbFailure {
                            code: "unavailable".to_owned(),
                        }),
                    );
                    end(error.to_string(), &mut pending);
                    return;
                }
            }
        }
        // Frames already read are served before the socket is waited on.
        let waiting = client.has_input() || {
            let socket = client.wait_fd();
            let mut fds = vec![PollFd::new(wake, PollFlags::IN)];
            if let Some(socket) = &socket {
                fds.push(PollFd::new(socket, PollFlags::IN));
            }
            if poll(&mut fds, None).is_err() {
                continue;
            }
            // The wake descriptor is read here so a command is not seen twice.
            if fds[0].revents().contains(PollFlags::IN) {
                let mut counter = [0_u8; 8];
                let _ = rustix::io::read(wake.as_fd(), &mut counter);
            }
            fds.get(1).is_some_and(|socket| !socket.revents().is_empty())
        };
        if !waiting {
            continue;
        }
        match client.next_event() {
            Ok(Event::Model(observation)) => match observation {
                Observation::Opened { view, .. } => graph.learn(&view),
                Observation::Closed { object, .. } => {
                    graph.types.remove(&object);
                }
                Observation::Signalled {
                    object, signal, fields, ..
                } => {
                    if let Some((_, class)) = graph.types.get(&object) {
                        signalled(class, &signal, object.get(), &fields);
                    }
                }
                Observation::Finished { operation, outcome, .. } => {
                    if let Some(token) = pending.remove(&operation) {
                        answered(
                            token,
                            match outcome {
                                OperationOutcome::Done => Ok(None),
                                OperationOutcome::Value(value) => Ok(Some(value)),
                                OperationOutcome::Failed(code) => Err(VerbFailure {
                                    code: code.as_str().to_owned(),
                                }),
                            },
                        );
                    }
                }
                Observation::Changed { .. } => {}
            },
            Ok(_) | Err(ClientError::Idle) => {}
            Err(error) => {
                end(error.to_string(), &mut pending);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_json_the_way_a_script_reads_it() {
        assert_eq!(json(&Value::Bool(true)), "true");
        assert_eq!(json(&Value::Uint(7)), "7");
        assert_eq!(json(&Value::Int(-2)), "-2");
        assert_eq!(json(&Value::Text("a \"b\"\n".to_owned())), "\"a \\\"b\\\"\\n\"");
        assert_eq!(
            json(&Value::List(vec![Value::Uint(1), Value::Text("x".to_owned())])),
            "[1,\"x\"]"
        );
        assert_eq!(json(&Value::Object(ObjectId::new(9))), "9");
        assert_eq!(json(&Value::Double(f64::NAN)), "null");
    }

    #[test]
    fn a_verbs_token_cannot_meet_a_tree_calls_or_a_flushs() {
        let token = next_token();
        assert_ne!(token & VERB_TOKEN_BIT, 0);
        assert_eq!(token & (1 << 63), 0);
        assert_ne!(next_token(), token);
    }
}
