//! The prompter service: `org.gnome.keyring.internal.Prompter`.
//!
//! This is the interface GNOME's `gcr-prompter` serves and every gcr caller expects:
//! `BeginPrompting` registers a callback object, `PerformPrompt` asks for a prompt,
//! and `StopPrompting` closes it. The prompter answers on the caller's callback
//! object with `PromptReady` or `PromptDone` (`callback` is that half), and the
//! passphrase travels inside the encrypted exchange, never as a plain argument.
//!
//! gcr's own implementation is `GcrSystemPrompter`; the interface is what this module
//! serves, and the reasoning behind each signature is in gcr's
//! `gcr-dbus-constants.h` and `gcr-system-prompter.c`, which are the two files that
//! define the wire this adapter must match. Two of their properties are load-bearing
//! here:
//!
//! - **One prompt per callback, one callback per prompt object.** gcr refuses a
//!   second `BeginPrompting` on a path, and a second `PerformPrompt` on a begun one.
//!   Both refusals are here, because a caller that retried into a second prompt would
//!   otherwise answer the first one when the user thinks they answered the second.
//! - **`PerformPrompt` returns before the user answers.** The answer arrives later
//!   as a callback call, so the handler replies as soon as the prompt is offered; it
//!   awaits the decision only to know what to tell the caller.
//!
//! The bus-side work is an [`Effect`] and never a call made from inside a handler,
//! for the ordering reason in `state`: gcr's caller sets the state its callback
//! checks while it handles this interface's reply, so a callback call written before
//! that reply arrives would be dropped.
//!
//! # Failure modes first
//!
//! 1. **A second prompt on one begun callback.** Refused, as gcr's service refuses
//!    it: `a_second_prompt_on_one_callback_is_refused`.
//! 2. **A prompt for a slot with no provider.** Refused with an error that names
//!    `prompt.secret`, before anything is offered:
//!    `no_provider_fails_closed_and_names_the_slot`.
//! 3. **A callback path that was never begun.** Refused instead of a prompt appearing
//!    with nowhere to answer: `a_prompt_needs_a_begun_callback`.
//! 4. **A caller asks for a prompt kind gcr does not have.** Refused with
//!    `InvalidArgs`: `an_unknown_prompt_kind_is_refused`.
//! 5. **The passphrase reaches a log.** It cannot: the only value that carries it is a
//!    `Secret`, whose `Debug` prints its length. `a_passphrase_never_reaches_the_log`
//!    captures the adapter's tracing output for a whole prompt and greps it.
//! 6. **The answer travels unencrypted.** The callback's exchange field is the only
//!    place the passphrase goes, encrypted under the caller's own key, which
//!    `an_answer_arrives_encrypted` checks by decrypting it as the caller would.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use futures_channel::mpsc;
use gnoblin_compat_core::Gate;
use gnoblin_model::{RequestId, RequestOutcome, Tick};
use zbus::message::Header;
use zbus::{Connection, ObjectPath, OwnedVariant, fdo};

use crate::callback::PromptCallbackProxy;
use crate::compat::{Admission, Call};
use crate::state::{Callback, Effect, Offerer, Pending, Prompt, PromptKind, Prompter, Unavailable};

/// The object path gcr's prompter answers on.
pub const PROMPTER_PATH: &str = "/org/gnome/keyring/Prompter";

/// The interface this module serves.
pub const PROMPTER_INTERFACE: &str = "org.gnome.keyring.internal.Prompter";

/// The system-wide name: the one `gnome-keyring` and `pinentry-gnome3` talk to.
pub const SYSTEM_NAME: &str = "org.gnome.keyring.SystemPrompter";

/// The private name, used by callers that want a prompt without other sessions
/// seeing it.
pub const PRIVATE_NAME: &str = "org.gnome.keyring.PrivatePrompter";

/// The state the connection's object server and the effect loop share.
#[derive(Debug)]
struct ServiceState {
    admission: Admission,
    state: Mutex<Prompter<Box<dyn Offerer + Send>>>,
    gate: Gate,
    deadline: Tick,
    effects: Mutex<Option<mpsc::UnboundedReceiver<Effect>>>,
    exported: Mutex<bool>,
}

/// The prompter object: exportable, clonable and cheap, so the object server and the
/// effect loop drive the same state.
#[derive(Clone, Debug)]
pub struct PromptService {
    shared: Arc<ServiceState>,
}

impl PromptService {
    /// An adapter that offers its prompts through `offerer` and fails them at
    /// `deadline` if no provider answers.
    ///
    /// The effects receiver is taken once with [`PromptService::take_effects`] and
    /// belongs to the connection's service loop.
    pub fn new<O: Offerer + Send + 'static>(
        offerer: O,
        gate: Gate,
        deadline: Tick,
    ) -> Result<PromptService, gnoblin_compat_core::MatrixError> {
        let admission = Admission::load()?;
        let (state, effects) = Prompter::new(Box::new(offerer) as Box<dyn Offerer + Send>);
        Ok(PromptService {
            shared: Arc::new(ServiceState {
                admission,
                state: Mutex::new(state),
                gate,
                deadline,
                effects: Mutex::new(Some(effects)),
                exported: Mutex::new(false),
            }),
        })
    }

    /// Record that this service was exported, or refuse a second export.
    ///
    /// One process serves one prompter object: a second export would replace the
    /// object server's entry for the path while the first export's names and effects
    /// loop are still live, and two effect loops would answer the same caller twice.
    pub fn take_export(&self) -> Result<(), Unavailable> {
        let mut exported = self
            .shared
            .exported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *exported {
            return Err(Unavailable::Bus {
                reason: "this adapter is already exported".to_owned(),
            });
        }
        *exported = true;
        Ok(())
    }

    /// Take the channel the state machine queues its bus work on.
    ///
    /// Exactly one loop may own it: it is the connection's service loop, and a second
    /// taker would be a second writer of the same conversation.
    pub fn take_effects(&self) -> Option<mpsc::UnboundedReceiver<Effect>> {
        self.shared
            .effects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    /// The shared state, with a poisoned lock recovered rather than propagated.
    ///
    /// Every transition is a map edit under the lock, so a panic in one leaves the
    /// state consistent and refusing every later call would be worse.
    fn lock(&self) -> MutexGuard<'_, Prompter<Box<dyn Offerer + Send>>> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Admit one call through the support matrix and the mode gate.
    fn admit(&self, call: Call) -> Result<(), Unavailable> {
        self.shared
            .admission
            .admit(call, self.shared.gate)
            .map_err(|refusal| Unavailable::Bus {
                reason: refusal.to_string(),
            })
    }

    /// Report how a prompt the provider held ended.
    ///
    /// The decision itself goes to the caller as a callback effect; what this returns is
    /// only whether the request was still waiting, which is what a log line says.
    pub fn answer(&self, request: RequestId, outcome: &RequestOutcome) -> Result<(), Unavailable> {
        self.lock().answer(request, outcome)
    }

    /// Cancel a prompt.
    pub fn cancel(&self, request: RequestId) -> Result<(), Unavailable> {
        self.lock().cancel(request)
    }

    /// Fail every prompt past the supplied tick.
    pub fn expire(&self, now: Tick) -> usize {
        self.lock().expire(now).len()
    }

    /// Every prompt waiting for an answer.
    pub fn pending(&self) -> Vec<Pending> {
        self.lock().pending().collect()
    }

    /// Whether a prompt is being shown, which is the `Prompting` property.
    fn is_prompting(&self) -> bool {
        self.lock().pending().next().is_some()
    }
}

/// The interface itself.
///
/// The methods are private on purpose: the object server calls them and every other
/// caller goes through [`PromptService::answer`], which is the same transition from
/// the provider's side.
#[zbus::interface(name = "org.gnome.keyring.internal.Prompter")]
impl PromptService {
    /// Register a callback object and answer with the prompter's exchange string.
    fn begin_prompting(&self, #[zbus(header)] header: Header<'_>, callback: ObjectPath<'_>) -> fdo::Result<()> {
        self.admit(Call::BeginPrompting).map_err(fdo::Error::from)?;
        let Some(sender) = header.sender().map(ToString::to_string) else {
            // The bus always fills the sender in; a message without one is not a
            // caller this adapter can answer.
            return Err(fdo::Error::Failed("the call carried no sender".to_owned()));
        };
        let key = Callback {
            sender,
            path: callback.to_string(),
        };
        let mut state = self.lock();
        state.begin(&key).map_err(fdo::Error::from)
    }

    /// Ask for a prompt.
    ///
    /// The reply is what releases the caller's `GcrSystemPrompt`, and it is sent as soon
    /// as the prompt is offered: the decision arrives afterwards on the caller's callback
    /// object, exactly as `GcrSystemPrompter` answers `PerformPrompt` before the user has
    /// decided. Answering here instead would hold the reply until the user decided, which
    /// is a different interface from gcr's and one a caller that answers the prompt from
    /// inside its own call would deadlock on.
    fn perform_prompt(
        &self,
        #[zbus(header)] header: Header<'_>,
        callback: ObjectPath<'_>,
        kind: String,
        properties: HashMap<String, OwnedVariant>,
        exchange: String,
    ) -> fdo::Result<()> {
        self.admit(Call::PerformPrompt).map_err(fdo::Error::from)?;
        let Some(sender) = header.sender().map(ToString::to_string) else {
            return Err(fdo::Error::Failed("the call carried no sender".to_owned()));
        };
        let kind = PromptKind::parse(&kind)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("`{kind}` is not a prompt kind")))?;
        let key = Callback {
            sender,
            path: callback.to_string(),
        };
        let prompt = Prompt::from_properties(kind, &key.path, &properties);
        gnoblin_log::info!(
            callback = %key.path,
            kind = kind.as_str(),
            title = %prompt.title,
            "offering a secret prompt"
        );
        self.lock()
            .perform(&key, prompt, &exchange, self.shared.deadline)
            .map_err(fdo::Error::from)?;
        Ok(())
    }

    /// Close a callback object.
    fn stop_prompting(&self, #[zbus(header)] header: Header<'_>, callback: ObjectPath<'_>) -> fdo::Result<()> {
        self.admit(Call::StopPrompting).map_err(fdo::Error::from)?;
        let Some(sender) = header.sender().map(ToString::to_string) else {
            return Err(fdo::Error::Failed("the call carried no sender".to_owned()));
        };
        let key = Callback {
            sender,
            path: callback.to_string(),
        };
        self.lock().stop(&key);
        Ok(())
    }

    /// Whether a prompt is being shown.
    ///
    /// gcr's callers read this to decide whether to wait for a prompt rather than
    /// start one, and it is derived from the pending requests rather than tracked
    /// separately, so it cannot disagree with them.
    #[zbus(property)]
    fn prompting(&self) -> bool {
        self.is_prompting()
    }
}

/// Perform one effect on the connection the request came in on.
pub(crate) async fn perform(connection: &Connection, effect: Effect) -> Result<(), zbus::Error> {
    match effect {
        Effect::Ready {
            sender,
            path,
            response,
            exchange,
        } => {
            let callback = PromptCallbackProxy::builder(connection)
                .destination(sender)
                .path(path)
                .build()
                .await?;
            callback.prompt_ready(&response, HashMap::new(), &exchange).await
        }
        Effect::Done { sender, path } => {
            let callback = PromptCallbackProxy::builder(connection)
                .destination(sender)
                .path(path)
                .build()
                .await?;
            callback.prompt_done().await
        }
    }
}
