//! The polkit authentication agent: `org.freedesktop.PolicyKit1.AuthenticationAgent`.
//!
//! polkitd calls the agent that registered for the session. The agent shows the
//! prompt, authenticates the user, and the reply to `BeginAuthentication` is how
//! polkitd learns whether the user consented. This module is that interface, and
//! nothing else in the crate knows a bus name.
//!
//! Three properties of the interface shape the code:
//!
//! - **The reply waits for the user.** `BeginAuthentication` is answered only once
//!   a provider answered the request it became, so the handler is `async` and awaits
//!   one channel per request. A synchronous handler would hold the connection's
//!   dispatcher and `CancelAuthentication` could never arrive, which is the hang
//!   this shape exists to prevent.
//! - **A cancel ends the call.** polkitd names a cookie, not a request, so the
//!   cookie is what [`Agent::cancel_cookie`] matches. The waiting handler then
//!   returns a D-Bus `Cancelled` error rather than leaving polkitd to time out.
//! - **No secret is held here.** polkit's password never reaches this crate:
//!   polkitd runs the authentication through its own helper, and the request that
//!   this agent offers carries the action, message, icon, details and identities.
//!   A log line therefore has nothing to redact for this slot, which is the reason
//!   the crate's `prompt.secret` answers live in `gnoblin-compat-gnome-keyring-prompter`
//!   instead.
//!
//! The object path and the authority's name are polkit's, not Gnoblin's: an agent
//! that used its own would never be called. They are constants here so a test can
//! serve the authority side of the same conversation on a private bus.
//!
//! # Failure modes first
//!
//! 1. **A second agent registers in one session.** [`Polkit::register`] refuses it
//!    before the bus round trip: `an_agent_registers_once`.
//! 2. **A `BeginAuthentication` for a slot with no provider hangs.** The refusal
//!    happens before anything is offered and the D-Bus error names the slot:
//!    `no_provider_fails_closed_and_names_the_slot`.
//! 3. **A cancelled authentication still returns success.** Cancelling ends the
//!    request and the waiting call returns `Cancelled`:
//!    `a_cancel_ends_the_waiting_call`.
//! 4. **An answer arrives that no handler is waiting for.** It is delivered to the
//!    request's channel exactly once, because the channel is inserted under the same
//!    lock that offers the request: `an_answer_arrives_once`.
//! 5. **The identity the provider reported is treated as authority.** It never
//!    leaves this module except as a log line, and the reply to polkitd says only
//!    "allowed":
//!    `the_reply_carries_no_identity`.

#![forbid(unsafe_code)]

gnoblin_log::component!("polkit");

pub mod subject;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use futures_channel::oneshot;
use gnoblin_model::{RequestId, RequestOutcome, Tick, Value};
use gnoblin_types::OwnerId;
use zbus::proxy;
use zbus::{Connection, ObjectPath, OwnedVariant, Variant, fdo};

use gnoblin_service_core::agent::{Agent, Decision, Finished, Identity, Offerer, Pending, PolkitRequest, Refusal};
use gnoblin_service_core::slot::{CapabilitySlot, Provider, Slots};

/// Where polkitd calls the registered agent.
pub const AGENT_PATH: &str = "/org/freedesktop/PolicyKit1/AuthenticationAgent";

/// The authority that dispatches authentications to the agent.
pub const AUTHORITY_SERVICE: &str = "org.freedesktop.PolicyKit1";

/// The authority object's path.
pub const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";

/// The authority's interface, which registration is a method of.
pub const AUTHORITY_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";

/// The authority's registration methods, as polkitd declares them.
///
/// `RegisterAuthenticationAgent` takes the subject as `(sa{sv})`, the locale the
/// caller wants messages in, and the object path the agent will answer on. The
/// unregister method repeats the subject and the path, without the locale.
#[proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    fn register_authentication_agent(
        &self,
        subject: (String, HashMap<String, OwnedVariant>),
        locale: &str,
        object_path: &ObjectPath<'_>,
    ) -> zbus::Result<()>;

    fn unregister_authentication_agent(
        &self,
        subject: (String, HashMap<String, OwnedVariant>),
        object_path: &ObjectPath<'_>,
    ) -> zbus::Result<()>;
}

/// The state one connection's handlers share.
struct AgentState {
    agent: Agent<Box<dyn Offerer + Send>>,
    replies: BTreeMap<RequestId, oneshot::Sender<Decision>>,
}

impl std::fmt::Debug for AgentState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentState")
            .field("pending", &self.agent.pending().count())
            .field("waiting", &self.replies.len())
            .finish()
    }
}

/// The agent object, exportable and clonable so the connection's object server and
/// the composition root's answer path are the same state.
#[derive(Clone, Debug)]
pub struct Polkit {
    shared: Arc<Mutex<AgentState>>,
    deadline: Tick,
}

impl Polkit {
    /// An agent that offers its requests through `offerer`, failing them at
    /// `deadline` if no provider answers.
    pub fn new<O: Offerer + Send + 'static>(offerer: O, deadline: Tick) -> Polkit {
        Polkit {
            shared: Arc::new(Mutex::new(AgentState {
                agent: Agent::new(Box::new(offerer) as Box<dyn Offerer + Send>),
                replies: BTreeMap::new(),
            })),
            deadline,
        }
    }

    /// The shared state, with a poisoned lock recovered rather than propagated.
    ///
    /// A panic inside one transition leaves the state machine consistent — every
    /// transition is a `BTreeMap` edit under the lock — so refusing every later call
    /// because a thread panicked once would be worse than continuing.
    fn lock(&self) -> MutexGuard<'_, AgentState> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The slots and their providers.
    pub fn slots(&self) -> Slots {
        self.lock().agent.slots().clone()
    }

    /// Whether an authentication agent is registered.
    pub fn is_registered(&self) -> bool {
        self.lock().agent.is_registered()
    }

    /// Every request waiting for an answer.
    pub fn pending(&self) -> Vec<Pending> {
        self.lock().agent.pending().cloned().collect()
    }

    /// Claim a slot, cancelling what the previous provider was holding.
    pub fn claim(
        &self,
        slot: CapabilitySlot,
        provider: Provider,
        owner: Option<OwnerId>,
    ) -> Result<Vec<Finished>, Refusal> {
        let finished = {
            let mut shared = self.lock();
            shared.agent.claim(slot, provider, owner)?
        };
        Ok(self.settle(finished))
    }

    /// Release a shell's claim on a slot, cancelling what it was holding.
    pub fn release(&self, slot: CapabilitySlot, owner: OwnerId) -> Result<Vec<Finished>, Refusal> {
        let finished = {
            let mut shared = self.lock();
            shared.agent.release(slot, owner)?
        };
        Ok(self.settle(finished))
    }

    /// Report how a request the provider held ended.
    ///
    /// The reply to the waiting `BeginAuthentication` carries the decision and never
    /// the identity: an identity is what a provider says it authenticated, which is
    /// not authority (design/rust-compositor.md, "Providers and slots").
    pub fn answer(&self, request: RequestId, outcome: &RequestOutcome) -> Result<Decision, Refusal> {
        let (decision, sender) = {
            let mut shared = self.lock();
            let decision = shared.agent.answer(request, outcome)?;
            (decision, shared.replies.remove(&request))
        };
        if let Some(sender) = sender {
            let _ = sender.send(decision.clone());
        }
        Ok(decision)
    }

    /// Cancel a request, as polkitd does when the caller exits.
    pub fn cancel(&self, request: RequestId) -> Result<Decision, Refusal> {
        let (decision, sender) = {
            let mut shared = self.lock();
            let decision = shared.agent.cancel(request)?;
            (decision, shared.replies.remove(&request))
        };
        if let Some(sender) = sender {
            let _ = sender.send(decision.clone());
        }
        Ok(decision)
    }

    /// Cancel every request of a slot whose provider just changed.
    pub fn slot_changed(&self, slot: CapabilitySlot) -> Vec<Finished> {
        let finished = self.lock().agent.cancel_slot(slot);
        self.settle(finished)
    }

    /// Fail every request past the supplied tick.
    pub fn expire(&self, now: Tick) -> Vec<Finished> {
        let finished = self.lock().agent.expire(now);
        self.settle(finished)
    }

    /// Unregister, cancelling everything that was waiting.
    pub fn unregister(&self) -> Vec<Finished> {
        let finished = self.lock().agent.unregister();
        self.settle(finished)
    }

    /// Deliver each finished request to the handler waiting for it.
    fn settle(&self, finished: Vec<Finished>) -> Vec<Finished> {
        let waiting: Vec<(oneshot::Sender<Decision>, Decision)> = {
            let mut shared = self.lock();
            finished
                .iter()
                .filter_map(|ended| {
                    shared
                        .replies
                        .remove(&ended.request)
                        .map(|sender| (sender, ended.decision.clone()))
                })
                .collect()
        };
        for (sender, decision) in waiting {
            let _ = sender.send(decision);
        }
        finished
    }

    /// Register this agent with polkitd, refusing a second registration.
    ///
    /// The agent must already be exported at [`AGENT_PATH`] on `connection`, or
    /// polkitd will call a path nobody serves. Registration is refused by polkitd
    /// when another agent holds the session, and that refusal is what the caller
    /// reports as `polkit_unavailable`.
    pub async fn register(&self, connection: &Connection) -> Result<(), Refusal> {
        self.lock().agent.register()?;
        let authority = match AuthorityProxy::new(connection).await {
            Ok(authority) => authority,
            Err(error) => {
                self.lock().agent.unregister();
                return Err(Refusal::Registration {
                    reason: error.to_string(),
                });
            }
        };
        let path = ObjectPath::try_from(AGENT_PATH).expect("the agent path is a constant");
        let locale = locale();
        if let Err(error) = authority
            .register_authentication_agent(subject::unix_process(), &locale, &path)
            .await
        {
            self.lock().agent.unregister();
            return Err(Refusal::Registration {
                reason: error.to_string(),
            });
        }
        gnoblin_log::info_in!(
            "polkit",
            path = AGENT_PATH,
            "registered as the polkit authentication agent"
        );
        Ok(())
    }

    /// Tell polkitd this process is no longer the session's agent.
    ///
    /// A failure is logged rather than returned: the caller is unloading the agent,
    /// and a bus that cannot be reached has nothing left to unregister from.
    pub async fn unregister_registration(&self, connection: &Connection) {
        let finished = self.unregister();
        if !finished.is_empty() {
            gnoblin_log::info_in!(
                "polkit",
                cancelled = finished.len(),
                "unregistering ended pending authentications"
            );
        }
        let Ok(authority) = AuthorityProxy::new(connection).await else {
            return;
        };
        let Ok(path) = ObjectPath::try_from(AGENT_PATH) else {
            return;
        };
        if let Err(error) = authority
            .unregister_authentication_agent(subject::unix_process(), &path)
            .await
        {
            gnoblin_log::warn_in!("polkit", %error, "polkitd refused the unregistration");
        }
    }
}

/// The locale polkitd's messages should be translated to.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok())
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

/// The `authentication_agent` interface itself.
///
/// The methods are private on purpose: the object server calls them, and every other
/// caller goes through [`Polkit::answer`] and [`Polkit::cancel`], which are the same
/// transitions from the provider's side.
#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Polkit {
    /// Ask the provider to authenticate the user for `action_id`.
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedVariant>)>,
    ) -> fdo::Result<()> {
        let identities = identities
            .into_iter()
            .map(|(kind, details)| Identity {
                kind,
                details: details
                    .iter()
                    .filter_map(|(key, variant)| value_of(variant).map(|value| (key.clone(), value)))
                    .collect(),
            })
            .collect();
        let request = PolkitRequest {
            cookie,
            action_id,
            message,
            icon_name,
            details: details.into_iter().collect(),
            identities,
        };
        let (request, receiver) = {
            let mut shared = self.lock();
            let offered = shared.agent.begin(request, self.deadline).map_err(refused)?;
            // The sender is inserted under the lock that offered the request, so an
            // answer from another thread cannot arrive before something is waiting
            // for it, and cannot arrive twice.
            let (sender, receiver) = oneshot::channel();
            shared.replies.insert(offered.request, sender);
            (offered.request, receiver)
        };
        gnoblin_log::info_in!("polkit", request = request.get(), "offered a polkit authentication");
        match receiver.await {
            Ok(Decision::Allowed { identity }) => {
                gnoblin_log::info_in!("polkit", request = request.get(), %identity, "authentication allowed");
                Ok(())
            }
            Ok(Decision::Denied) | Err(_) => Err(fdo::Error::AuthFailed("Not authorized".to_owned())),
            Ok(Decision::Cancelled) => Err(fdo::Error::Failed(
                "cancelled: an authentication request was cancelled".to_owned(),
            )),
        }
    }

    /// Stop a request polkitd no longer wants an answer to.
    ///
    /// A cookie that names nothing is not an error: polkitd cancels requests that
    /// already finished.
    async fn cancel_authentication(&self, cookie: String) -> fdo::Result<()> {
        let finished = self.lock().agent.cancel_cookie(&cookie);
        if let Some(finished) = finished {
            gnoblin_log::info_in!(
                "polkit",
                request = finished.request.get(),
                "polkitd cancelled an authentication"
            );
            self.settle(vec![finished]);
        }
        Ok(())
    }
}

/// Why a request could not be offered, as a D-Bus error.
fn refused(error: Refusal) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}

/// One variant of polkit's identity details, in the model's vocabulary.
///
/// A variant this mapping does not know is dropped rather than guessed at: the name
/// a provider shows comes from the message, and a detail it cannot read is better
/// omitted than mistyped.
fn value_of(variant: &OwnedVariant) -> Option<Value> {
    Some(match &**variant {
        Variant::Bool(value) => Value::Bool(*value),
        Variant::U32(value) => Value::Uint(u64::from(*value)),
        Variant::U64(value) => Value::Uint(*value),
        Variant::I32(value) => Value::Int(i64::from(*value)),
        Variant::I64(value) => Value::Int(*value),
        Variant::Str(value) => Value::Text(value.as_str().to_owned()),
        _ => return None,
    })
}
