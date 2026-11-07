//! The `prompt.secret` state machine: one gcr prompt, one request, one answer.
//!
//! `prompter` is the D-Bus interface and `callback` is the caller's object; this
//! module is what sits between them and knows nothing about either. It is a plain
//! state machine over the requests the adapter has offered, so every failure mode
//! the plan lists for this slot is testable without a bus:
//!
//! - **No provider.** A prompt for a slot with no provider fails at the offer, and
//!   the refusal names `prompt.secret`. Nothing waits for a deadline that would
//!   otherwise end as "the program that asked saw a cancellation".
//! - **Finish once.** A request is pending or finished, never both, and a finished
//!   one keeps a bounded tombstone, so an answer that arrives after a cancel is
//!   refused rather than reviving a prompt the user already dismissed.
//! - **The passphrase is answered, not stored.** [`Decision::Password`] holds a
//!   [`Secret`], which zeroes on drop and prints nothing. The only thing that ever
//!   sees the bytes is the secret exchange that encrypts them for the caller.
//!
//! # Why the effects are values
//!
//! A finished request has to reach the caller as a D-Bus call
//! (`PromptReady`/`PromptDone`) on the same connection the request arrived on.
//! Making that call from inside the handler would write it before the handler's own
//! reply, and gcr's caller is only ready for it after that reply: it sets the state
//! the callback checks while handling the reply. So the state machine never calls
//! D-Bus. It returns [`Effect`] values, and `serve` performs them in order once the
//! handler that produced them has replied.
//!
//! # Failure modes first
//!
//! 1. **A prompt with no provider hangs.** The offer is refused and the D-Bus error
//!    names the slot: `no_provider_fails_closed_and_names_the_slot`.
//! 2. **A prompt is answered after it was cancelled.** Refused with
//!    [`Unavailable::Finished`]: `an_answer_after_a_cancel_is_refused`.
//! 3. **A passphrase reaches a log line.** It cannot: [`Secret`]'s `Debug` and
//!    `Drop` are the only implementations, and the tracing capture in
//!    `tests/dbus.rs` proves a full prompt leaves no passphrase in the log:
//!    `a_passphrase_never_reaches_the_log`.
//! 4. **A caller performs two prompts on one begun callback.** The second is
//!    refused, as gcr's own service refuses it:
//!    `a_second_prompt_on_one_callback_is_refused`.
//! 5. **A caller that is still prompting loses its request to a deadline.**
//!    [`Prompter::expire`] finishes it as cancelled and queues the callback:
//!    `a_request_past_its_deadline_is_cancelled`.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use futures_channel::mpsc;
use gnoblin_model::{RequestId, RequestOutcome, Tick};

use crate::exchange::{Exchange, Secret};

use crate::log;

/// The slot this adapter exists for, as the plan's table names it.
pub const SLOT: &str = "prompt.secret";

/// How many finished requests the state machine remembers.
pub const REMEMBERED_FINISHED: usize = 64;

/// Which of gcr's two prompt kinds the caller asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptKind {
    /// A passphrase. The answer is the passphrase, and an empty passphrase is a
    /// passphrase, not a refusal.
    Password,
    /// A yes/no confirmation. The answer is exactly `yes` or `no`.
    Confirm,
}

impl PromptKind {
    /// The spelling gcr uses in `PerformPrompt`'s type argument.
    pub const fn as_str(self) -> &'static str {
        match self {
            PromptKind::Password => "password",
            PromptKind::Confirm => "confirm",
        }
    }

    /// Parse gcr's type argument.
    pub fn parse(word: &str) -> Option<PromptKind> {
        match word {
            "password" => Some(PromptKind::Password),
            "confirm" => Some(PromptKind::Confirm),
            _ => None,
        }
    }
}

/// One prompt, as the caller described it with gcr's prompt properties.
///
/// The fields are the ones a `GcrPrompt` has and this adapter forwards verbatim: it
/// draws nothing and decides nothing about the wording, which belongs to the program
/// that asked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Prompt {
    /// A passphrase or a confirmation.
    pub kind: PromptKind,
    /// Where the answer goes, as the caller named it in `BeginPrompting`.
    pub callback: String,
    /// The prompt's title.
    pub title: String,
    /// The sentence above the field.
    pub message: String,
    /// The secondary text, usually what the passphrase is for.
    pub description: String,
    /// A warning to show, such as "the passphrase is wrong".
    pub warning: String,
    /// The label of the caller's optional choice button.
    pub choice_label: String,
    /// Whether that choice was already chosen.
    pub choice_chosen: bool,
    /// Whether the passphrase is being set rather than entered.
    pub password_new: bool,
    /// The window the caller wants the prompt centred on, if it named one.
    pub caller_window: String,
    /// The caller's label for the accept button.
    pub continue_label: String,
    /// The caller's label for the dismiss button.
    pub cancel_label: String,
}

impl Prompt {
    /// Read gcr's prompt properties into a [`Prompt`].
    ///
    /// The names are gcr's `GcrPrompt` property names, which is what a caller sends in
    /// `PerformPrompt`'s `a{sv}`. A property this build does not know is ignored: the
    /// prompt still shows, because a title and a message are all a provider needs,
    /// and refusing the whole prompt over an extra property would break a newer gcr.
    pub fn from_properties(
        kind: PromptKind,
        callback: &str,
        properties: &std::collections::HashMap<String, zbus::OwnedVariant>,
    ) -> Prompt {
        Prompt {
            kind,
            callback: callback.to_owned(),
            title: text(properties, "title"),
            message: text(properties, "message"),
            description: text(properties, "description"),
            warning: text(properties, "warning"),
            choice_label: text(properties, "choice-label"),
            choice_chosen: flag(properties, "choice-chosen"),
            password_new: flag(properties, "password-new"),
            caller_window: text(properties, "caller-window"),
            continue_label: text(properties, "continue-label"),
            cancel_label: text(properties, "cancel-label"),
        }
    }
}

/// One string property, or the empty string when it is absent or not a string.
fn text(properties: &std::collections::HashMap<String, zbus::OwnedVariant>, name: &str) -> String {
    match properties.get(name).map(|value| &**value) {
        Some(zbus::Variant::Str(value)) => value.as_str().to_owned(),
        _ => String::new(),
    }
}

/// One boolean property, or `false` when it is absent or not a boolean.
fn flag(properties: &std::collections::HashMap<String, zbus::OwnedVariant>, name: &str) -> bool {
    matches!(
        properties.get(name).map(|value| &**value),
        Some(zbus::Variant::Bool(true))
    )
}

/// What the provider decided.
#[derive(Debug)]
pub enum Decision {
    /// The user entered this passphrase.
    Password(Secret),
    /// The user answered a confirmation.
    Confirmed(bool),
    /// Nothing was decided: the request was cancelled, timed out or lost its
    /// provider.
    Cancelled,
}

/// One prompt that ended, for a report or a test.
#[derive(Debug)]
pub struct EndedPrompt {
    /// The request that ended.
    pub request: RequestId,
    /// How it ended.
    pub decision: Decision,
}

/// One prompt waiting for an answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pending {
    /// The request's identity on the IPC.
    pub request: RequestId,
    /// Where the answer goes.
    pub callback: String,
    /// A passphrase or a confirmation.
    pub kind: PromptKind,
    /// When the request fails if nobody answers it.
    pub deadline: Tick,
}

/// What the caller is told once a request ends.
///
/// Both variants name the caller's connection and its callback path, which is all
/// `callback` needs to build the proxy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Answer the caller: the response gcr expects (`yes`, `no` or nothing) and the
    /// exchange string that carries the passphrase when there is one.
    Ready {
        /// The caller's unique bus name.
        sender: String,
        /// The caller's callback object path.
        path: String,
        /// `yes` when the user accepted, `no` when they did not.
        response: String,
        /// The secret exchange string, already encrypted.
        exchange: String,
    },
    /// Tell the caller its prompt is over, without an answer.
    Done {
        /// The caller's unique bus name.
        sender: String,
        /// The caller's callback object path.
        path: String,
    },
}

/// The privileged connection through which a `prompt.secret` request reaches the
/// providers that may claim it.
pub trait Offerer {
    /// Offer one prompt, announcing it to the connections allowed to claim it.
    ///
    /// A slot with no provider is refused here with
    /// [`Unavailable::NoProvider`], before anything is announced: the compositor
    /// knows who provides the slot, and this adapter must not turn "nobody will
    /// answer" into a 120-second wait.
    fn offer(&mut self, prompt: &Prompt, deadline: Tick) -> Result<RequestId, Unavailable>;
}

/// A boxed offerer is an offerer, so the D-Bus layer can hold whichever connection
/// the composition root gave it without naming its type.
///
/// The box is `Send` because a served interface's method future is spawned and may
/// run on another thread of the connection's runtime. A test that drives the state
/// machine directly uses [`Prompter`] with whatever host it has and is not bound by
/// that.
impl Offerer for Box<dyn Offerer + Send> {
    fn offer(&mut self, prompt: &Prompt, deadline: Tick) -> Result<RequestId, Unavailable> {
        (**self).offer(prompt, deadline)
    }
}

/// Why a prompt could not be started, answered or finished.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Unavailable {
    /// No provider holds the slot, so nothing would show the prompt.
    NoProvider {
        /// The slot with no provider.
        slot: &'static str,
    },
    /// The caller has not begun prompting on this callback.
    NotBegun {
        /// The callback the caller named.
        callback: String,
    },
    /// This callback is already performing a prompt.
    Busy {
        /// The callback that is busy.
        callback: String,
    },
    /// The request is not one this adapter offered, and it is not remembered as
    /// finished either.
    UnknownRequest {
        /// The request the caller named.
        request: RequestId,
    },
    /// The request already ended. An answer here never reopens it.
    Finished {
        /// The request that is already finished.
        request: RequestId,
    },
    /// Another process owns one of the gcr names.
    NameUnavailable {
        /// The name that could not be taken.
        name: String,
    },
    /// The bus refused a call, or the connection could not be set up.
    Bus {
        /// What the bus said.
        reason: String,
    },
}

impl From<Unavailable> for zbus::fdo::Error {
    fn from(error: Unavailable) -> zbus::fdo::Error {
        zbus::fdo::Error::Failed(error.to_string())
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::NoProvider { slot } => write!(
                formatter,
                "no provider claims the `{slot}` slot, so the prompt was not offered"
            ),
            Unavailable::NotBegun { callback } => {
                write!(formatter, "prompting has not begun on `{callback}`")
            }
            Unavailable::Busy { callback } => {
                write!(formatter, "`{callback}` is already performing a prompt")
            }
            Unavailable::UnknownRequest { request } => {
                write!(formatter, "request {} was never offered by this adapter", request.get())
            }
            Unavailable::Finished { request } => write!(formatter, "request {} already ended", request.get()),
            Unavailable::NameUnavailable { name } => {
                write!(formatter, "`{name}` is owned by another process")
            }
            Unavailable::Bus { reason } => write!(formatter, "the bus refused the call: {reason}"),
        }
    }
}

impl std::error::Error for Unavailable {}

/// A callback a caller began prompting on.
#[derive(Debug)]
struct Begun {
    /// This side's half of the secret exchange, kept for the whole callback because
    /// gcr reuses one exchange across the prompts of one prompt object.
    exchange: Exchange,
    /// Whether a prompt is in flight, which is what refuses a second one.
    prompted: bool,
}

/// The live half of one waiting request, kept out of [`Pending`] so a transition can
/// read it and then replace it without holding a borrow.
#[derive(Debug)]
struct Waiting {
    callback: Callback,
    kind: PromptKind,
    exchange: Exchange,
    deadline: Tick,
}

/// One caller's prompt object, which is its connection and the path it chose.
///
/// Both halves are the key: gcr's callers pick their own object paths, so two
/// programs can use the same path on different connections, and the prompter keys
/// them apart exactly as `GcrSystemPrompter` keys its callbacks by `(name, path)`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Callback {
    /// The caller's unique bus name.
    pub sender: String,
    /// The object path the caller answers on.
    pub path: String,
}

/// The adapter's state: the begun callbacks, the requests waiting for an answer, the
/// tombstones of the ones that ended, and the effects the bus still has to deliver.
#[derive(Debug)]
pub struct Prompter<O: Offerer = Box<dyn Offerer + Send>> {
    offerer: O,
    begun: BTreeMap<Callback, Begun>,
    waiting: BTreeMap<RequestId, Waiting>,
    finished: BTreeMap<RequestId, Decision>,
    forgotten: VecDeque<RequestId>,
    effects: mpsc::UnboundedSender<Effect>,
}

impl fmt::Debug for Box<dyn Offerer + Send> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Box<dyn Offerer>")
    }
}

impl<O: Offerer> Prompter<O> {
    /// An adapter that offers its prompts through `offerer`.
    ///
    /// The returned receiver yields one [`Effect`] per thing the bus still has to be
    /// told, in order. Exactly one caller takes it: the connection's service loop.
    pub fn new(offerer: O) -> (Prompter<O>, mpsc::UnboundedReceiver<Effect>) {
        let (effects, receiver) = mpsc::unbounded();
        (
            Prompter {
                offerer,
                begun: BTreeMap::new(),
                waiting: BTreeMap::new(),
                finished: BTreeMap::new(),
                forgotten: VecDeque::new(),
                effects,
            },
            receiver,
        )
    }

    /// Record that a caller began prompting, returning the exchange string that side
    /// sends first.
    pub fn begin(&mut self, callback: &Callback) -> Result<(), Unavailable> {
        if self.begun.contains_key(callback) {
            return Err(Unavailable::Busy {
                callback: callback.path.clone(),
            });
        }
        let exchange = Exchange::new().map_err(|error| Unavailable::Bus {
            reason: error.to_string(),
        })?;
        let effect = Effect::Ready {
            sender: callback.sender.clone(),
            path: callback.path.clone(),
            response: String::new(),
            exchange: exchange.begin(),
        };
        self.begun.insert(
            callback.clone(),
            Begun {
                exchange,
                prompted: false,
            },
        );
        self.notify(effect);
        Ok(())
    }

    /// Record that a caller stopped prompting on a callback.
    ///
    /// A prompt still in flight ends as cancelled and the caller is told once, with
    /// `PromptDone`: gcr's own service sends no `PromptReady` for a prompt it
    /// stopped, because the caller has already closed the object that would read it.
    pub fn stop(&mut self, callback: &Callback) {
        self.begun.remove(callback);
        let request = self
            .waiting
            .iter()
            .find(|(_, waiting)| &waiting.callback == callback)
            .map(|(request, _)| *request);
        let Some(request) = request else {
            return;
        };
        self.end_without_effect(request);
        self.notify(Effect::Done {
            sender: callback.sender.clone(),
            path: callback.path.clone(),
        });
    }

    /// Offer one prompt to the slot's provider.
    ///
    /// The caller's half of the secret exchange has already been received by the
    /// time this is called, because the exchange string arrives with the prompt.
    pub fn perform(
        &mut self,
        callback: &Callback,
        prompt: Prompt,
        caller_exchange: &str,
        deadline: Tick,
    ) -> Result<RequestId, Unavailable> {
        let Some(begun) = self.begun.get_mut(callback) else {
            return Err(Unavailable::NotBegun {
                callback: callback.path.clone(),
            });
        };
        if begun.prompted {
            return Err(Unavailable::Busy {
                callback: callback.path.clone(),
            });
        }
        begun
            .exchange
            .receive(caller_exchange)
            .map_err(|error| Unavailable::Bus {
                reason: error.to_string(),
            })?;
        let exchange = begun.exchange.clone();
        let offered = self.offerer.offer(&prompt, deadline)?;
        begun.prompted = true;
        self.waiting.insert(
            offered,
            Waiting {
                callback: callback.clone(),
                kind: prompt.kind,
                exchange,
                deadline,
            },
        );
        Ok(offered)
    }

    /// Report how a prompt the provider held ended.
    ///
    /// A password prompt's answer is the passphrase, which may be empty; a confirmation's
    /// is exactly `yes`; and a request that ended without an answer, or timed out, is a
    /// dismissal. The kind is the prompt's, so the same bytes mean one thing only.
    pub fn answer(&mut self, request: RequestId, outcome: &RequestOutcome) -> Result<(), Unavailable> {
        let Some(waiting) = self.waiting.get(&request) else {
            return Err(self.absent(request));
        };
        let decision = match outcome {
            RequestOutcome::Ended | RequestOutcome::TimedOut | RequestOutcome::Refused(_) => Decision::Cancelled,
            RequestOutcome::Answered(bytes) => match waiting.kind {
                PromptKind::Password => Decision::Password(Secret::new(bytes.to_vec())),
                PromptKind::Confirm => Decision::Confirmed(bytes.as_slice() == b"yes"),
            },
        };
        self.commit(request, decision);
        Ok(())
    }

    /// Cancel a prompt.
    pub fn cancel(&mut self, request: RequestId) -> Result<(), Unavailable> {
        if !self.waiting.contains_key(&request) {
            return Err(self.absent(request));
        }
        self.commit(request, Decision::Cancelled);
        Ok(())
    }

    /// Cancel every prompt at or past its deadline.
    ///
    /// The caller supplies the current tick, so nothing here reads a clock and a test
    /// replays the same outcome from the same ticks.
    pub fn expire(&mut self, now: Tick) -> Vec<EndedPrompt> {
        let overdue: Vec<RequestId> = self
            .waiting
            .iter()
            .filter(|(_, waiting)| waiting.deadline <= now)
            .map(|(request, _)| *request)
            .collect();
        self.finish_all(overdue)
    }

    /// Every prompt waiting for an answer.
    pub fn pending(&self) -> impl Iterator<Item = Pending> + '_ {
        self.waiting.iter().map(|(request, waiting)| Pending {
            request: *request,
            callback: waiting.callback.path.clone(),
            kind: waiting.kind,
            deadline: waiting.deadline,
        })
    }

    /// The state of one finished request, for a report or a test.
    pub fn decision(&self, request: RequestId) -> Option<&Decision> {
        self.finished.get(&request)
    }

    /// Queue one effect for the connection's service loop.
    fn notify(&mut self, effect: Effect) {
        let _ = self.effects.unbounded_send(effect);
    }

    /// Why a request is not waiting: finished, or never offered.
    fn absent(&self, request: RequestId) -> Unavailable {
        if self.finished.contains_key(&request) {
            Unavailable::Finished { request }
        } else {
            Unavailable::UnknownRequest { request }
        }
    }

    /// Finish one request: queue what the caller must be told, wake the waiting
    /// handler and remember the tombstone.
    fn commit(&mut self, request: RequestId, decision: Decision) {
        let Some(waiting) = self.waiting.remove(&request) else {
            return;
        };
        if let Some(begun) = self.begun.get_mut(&waiting.callback) {
            begun.prompted = false;
        }
        let effect = match &decision {
            Decision::Password(secret) => waiting.exchange.send(Some(secret.expose())),
            Decision::Confirmed(_) | Decision::Cancelled => waiting.exchange.send(None),
        };
        let response = match &decision {
            Decision::Password(_) | Decision::Confirmed(true) => "yes",
            Decision::Confirmed(false) | Decision::Cancelled => "no",
        };
        match effect {
            Ok(text) => {
                self.notify(Effect::Ready {
                    sender: waiting.callback.sender.clone(),
                    path: waiting.callback.path.clone(),
                    response: response.to_owned(),
                    exchange: text,
                });
            }
            Err(error) => {
                log::warn!(%error, request = request.get(), "could not build the secret exchange string");
                self.notify(Effect::Done {
                    sender: waiting.callback.sender.clone(),
                    path: waiting.callback.path.clone(),
                });
            }
        }
        self.finished.insert(request, decision);
        self.forgotten.push_back(request);
        while self.forgotten.len() > REMEMBERED_FINISHED {
            if let Some(oldest) = self.forgotten.pop_front() {
                self.finished.remove(&oldest);
            }
        }
    }

    /// Finish one request without answering the caller, for a callback that is gone.
    fn end_without_effect(&mut self, request: RequestId) {
        if self.waiting.remove(&request).is_none() {
            return;
        }
        self.finished.insert(request, Decision::Cancelled);
        self.forgotten.push_back(request);
        while self.forgotten.len() > REMEMBERED_FINISHED {
            if let Some(oldest) = self.forgotten.pop_front() {
                self.finished.remove(&oldest);
            }
        }
    }

    /// Finish every named request as cancelled.
    fn finish_all(&mut self, requests: Vec<RequestId>) -> Vec<EndedPrompt> {
        let live: Vec<RequestId> = requests
            .into_iter()
            .filter(|request| self.waiting.contains_key(request))
            .collect();
        for request in &live {
            self.commit(*request, Decision::Cancelled);
        }
        live.into_iter()
            .map(|request| EndedPrompt {
                request,
                decision: Decision::Cancelled,
            })
            .collect()
    }
}
