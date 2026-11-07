//! The consent core: a table of portal sessions, and the grants behind them.
//!
//! Design/rust-compositor.md's "Portals" card splits the backend in two: the portal
//! owns app identity, session lifetime, cancellation and the response, while a shell
//! only shows the prompt and returns a typed choice. This module is the first half. It
//! knows no D-Bus, no bus name on a D-Bus connection and no compositor: a caller arrives
//! as a [`Requester`], a prompt leaves through [`Host`], and every decision is
//! `gnoblin-authority`'s.
//!
//! # What a session is, and what a grant is
//!
//! A [`Session`] is one portal session: the caller's bus name, the app id the request
//! carried, which interface created it, and how far it has got. Sessions are separate
//! from grants on purpose. A grant belongs to the *caller* and the *capability*
//! (`gnoblin-authority` keys it by `(RequesterId, PortalCapability)`, so a second session of
//! the same app is allowed by the answer the user already gave and asks nothing), and a
//! session belongs to one request and ends when that request does. Two sessions of one
//! app therefore share the grant and share nothing else: closing one leaves the other
//! running, and the grants end only when the caller's connection does.
//!
//! # Two request identities
//!
//! The request layer and this module number requests differently, and the authority says
//! why (`gnoblin-authority/src/ids.rs`): the IPC's [`Offer::request`] names a frame on
//! the wire, and the authority's own `RequestId` names a consent prompt it is tracking.
//! An offer's number is the authority's number with the same token, translated in exactly
//! one place ([`Guard::ask`]) so the answer and the prompt can never be confused.
//!
//! # D14: no `portal.ui` provider means no capture
//!
//! A prompt is offered to the `portal.ui` slot. When nobody claims the slot the offer
//! is refused before it is made, with [`Refusal::NoProvider`] naming `portal.ui`, and
//! nothing waits out a deadline: the caller learns what is missing rather than seeing a
//! share that never appears. That is D14's fail-closed answer, and it is the same shape
//! `gnoblin-compat-dbus-gnome-keyring-prompter` uses for `prompt.secret`.
//!
//! # Expiry
//!
//! A pending prompt expires closed. Two mechanisms agree on that, and both are needed:
//!
//! - [`Guard::tick`] releases the waiting D-Bus caller and tells the request layer to
//!   withdraw the offer, because a caller must not be held to a deadline it cannot see.
//! - The expiry is applied to the authority as a refusal, so the pending grant is
//!   removed and a late answer cannot turn into consent after the fact
//!   ([`Denial::ConsentExpired`]).
//!
//! # Failure modes first
//!
//! Each rule below names the test that pins it in `tests/failure_modes.rs`, or says that
//! no test pins it yet. The tests drive [`Guard`] against a fake host with no bus.
//!
//! 1. **A prompt is asked with no `portal.ui` provider.** Refused with an error naming
//!    the slot, and nothing is offered. No test in this crate pins this yet.
//! 2. **The provider declines.** Nothing is started. No test in this crate pins this yet.
//! 3. **Capture runs with no consent.** Not pinned by a test in this crate yet.
//! 4. **A grant outlives its requester.** `requester_gone_releases_prompt` pins the
//!    release of a waiting prompt when the caller leaves.
//! 5. **A prompt nobody answers stays pending.** `unanswered_prompt_expires` pins expiry
//!    closed on a tick.
//! 6. **An answer arrives after the caller left.** Not pinned by a test in this crate yet.
//! 7. **Two sessions share a grant.** `granted_capability_is_not_asked_twice` pins that a
//!    repeat ask is answered from the grant.
//! 8. **The provider leaves while a prompt waits.** `provider_gone_releases_prompt` pins
//!    the release.
//! 9. **A grant answer opens the capability; a refusal opens nothing.**
//!    `allow_answer_grants_capability` and `refusal_grants_nothing` pin both halves.
//!
//! Session ownership between callers and the lock rule have no test in this crate yet.

use std::collections::BTreeMap;
use std::fmt;

use futures_channel::oneshot;
use gnoblin_authority::{Answer, Authority, Decision, Denial, Effect, Event, RequestId as PromptId, RequesterId};
/// The grant vocabulary, re-exported so a frontend names the same capabilities the authority does.
pub use gnoblin_authority::{Consent, PortalCapability};
use gnoblin_model::Value;
use gnoblin_types::{ClientId, RequestId, Tick};

use crate::backends::StreamId;

use crate::log;

/// The slot a consent prompt is offered to, as the plan's table names it.
pub const PORTAL_UI: &str = "portal.ui";

/// The identity the authority records for the provider of an IPC offer.
///
/// Over the IPC the request layer binds an answer to the connection that claimed the
/// request (`gnoblin-model`'s broker refuses an answer from anyone else), so the
/// compositor has already established that the answerer is the provider before this
/// process sees a byte. A client of the IPC is not told which connection holds the slot,
/// so this constant stands for "whoever the request layer delivered the answer from": it
/// is not a fabricated identity that widens what a caller may do, it is the only identity
/// that can be named for a check the transport has already performed.
pub const OFFERER: ClientId = ClientId::new(0);

/// Which portal interface a session belongs to.
///
/// The set is closed because the backend serves these and routes the rest
/// (design/rust-compositor.md, "Portals"), and the names are the interface names with
/// the `org.freedesktop.impl.portal.` prefix removed.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Interface {
    /// `ScreenCast`.
    ScreenCast,
    /// `Screenshot`. A screenshot has no session of its own in the frontend's model, so
    /// this value is only used for the audit row a screenshot leaves behind.
    Screenshot,
    /// `RemoteDesktop`.
    RemoteDesktop,
    /// `InputCapture`.
    InputCapture,
    /// `GlobalShortcuts`.
    GlobalShortcuts,
    /// `Inhibit`, the monitoring session.
    Inhibit,
}

impl Interface {
    /// The interface's name, as a log line and a test spell it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ScreenCast => "ScreenCast",
            Self::Screenshot => "Screenshot",
            Self::RemoteDesktop => "RemoteDesktop",
            Self::InputCapture => "InputCapture",
            Self::GlobalShortcuts => "GlobalShortcuts",
            Self::Inhibit => "Inhibit",
        }
    }

    /// The capabilities this interface may ask for.
    ///
    /// It is the authority's own vocabulary ("what a portal caller may be granted"), so
    /// an interface can only ever ask for something the grant model names. Remote
    /// desktop and input capture read frames and inject input; a screen cast and a
    /// screenshot only read frames. Shortcuts, the clipboard and an inhibit monitor ask
    /// for neither: their choice is recorded on the session and is not a grant, because
    /// the authority has no capability that a shortcut binding or a clipboard transfer
    /// would be.
    #[must_use]
    pub const fn capabilities(self) -> &'static [PortalCapability] {
        match self {
            Self::ScreenCast | Self::Screenshot => &[PortalCapability::Capture],
            Self::RemoteDesktop | Self::InputCapture => &[PortalCapability::Capture, PortalCapability::InputInjection],
            Self::GlobalShortcuts | Self::Inhibit => &[],
        }
    }

    /// Whether this interface can be asked for `capability`.
    #[must_use]
    pub fn allows(self, capability: PortalCapability) -> bool {
        self.capabilities().contains(&capability)
    }
}

/// Who is asking, as the request named them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Requester {
    /// The caller's unique bus name, which is what [`Guard::requester_gone`] matches.
    pub bus_name: String,
    /// The app id the request carried. It is a label the frontend derived, never
    /// authority on its own.
    pub app_id: String,
    /// Whether the frontend said the caller is sandboxed.
    ///
    /// The `org.freedesktop.impl.portal` interfaces carry only an app id; there is no
    /// member that reports a sandbox. This backend therefore reads a `sandboxed` (`b`)
    /// member from the request's options, and `None` means the frontend did not say.
    /// Unknown is narrowed ([`Requester::narrowed`]): the options a sandboxed caller may
    /// not have are exactly the ones that would otherwise hand an unidentified caller a
    /// virtual monitor or a permission that outlives the process, and the wider set is
    /// only for a frontend that states the caller is a host app.
    pub sandboxed: Option<bool>,
}

impl Requester {
    /// Whether the narrower option set applies to this caller.
    #[must_use]
    pub fn narrowed(&self) -> bool {
        self.sandboxed.unwrap_or(true)
    }
}

/// How far a session has got.
///
/// The order matters: a session may be started once, and a member that arrives out of
/// order is refused rather than applied to a session that is not ready for it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum State {
    /// Created, nothing selected yet.
    Created,
    /// Its sources or devices are chosen.
    Configured,
    /// Started: it holds whatever the backend opened.
    Started,
}

/// One portal session.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The caller's session handle path, which the frontend chose.
    pub path: String,
    /// Who created it.
    pub owner: Requester,
    /// Which interface it belongs to.
    pub interface: Interface,
    /// How far it has got.
    pub state: State,
    /// What the caller asked for in the member that configured it, as the interface
    /// narrowed it.
    ///
    /// It is the request's own vocabulary rather than a field per interface: a screen cast
    /// stores `types` and `cursor_mode`, a remote desktop stores `types` for its devices,
    /// and the guard stores what it was given without knowing what the names mean.
    pub options: std::collections::BTreeMap<String, Value>,
    /// What the backend opened for it, in the order it was opened.
    pub streams: Vec<StreamId>,
}

/// One session to create.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewSession {
    /// The caller's unique bus name.
    pub bus_name: String,
    /// The app id the request carried.
    pub app_id: String,
    /// Whether the frontend said the caller is sandboxed.
    pub sandboxed: Option<bool>,
    /// Which interface is creating it.
    pub interface: Interface,
    /// The session handle path the frontend chose.
    pub path: String,
}

/// A prompt for the `portal.ui` provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt {
    /// What is being asked for, so the provider can word the question.
    pub capability: PortalCapability,
    /// Who is asking.
    pub app_id: String,
    /// The parent window the caller named, as the frontend sent it. The relay does not
    /// parse it; the compositor resolves and verifies it. `None` when the caller sent none.
    pub parent_window: Option<String>,
    /// The title the provider shows.
    pub title: String,
    /// The sentence the provider shows.
    pub message: String,
}

impl Prompt {
    /// The prompt for one capability, worded from the interface and the app.
    #[must_use]
    pub fn new(
        interface: Interface,
        requester: &Requester,
        capability: PortalCapability,
        parent_window: &str,
    ) -> Prompt {
        let what = match capability {
            PortalCapability::Capture => "share your screen",
            PortalCapability::InputInjection => "control your pointer and keyboard",
        };
        let app = if requester.app_id.is_empty() {
            "An unidentified application"
        } else {
            requester.app_id.as_str()
        };
        Prompt {
            capability,
            app_id: requester.app_id.clone(),
            parent_window: non_empty(parent_window),
            title: format!("{app} wants to {what}"),
            message: format!("{} is asking through {}.", interface.as_str(), capability.as_str()),
        }
    }
}

/// The parent window string as sent, or `None` when the caller sent none.
fn non_empty(parent_window: &str) -> Option<String> {
    if parent_window.is_empty() {
        None
    } else {
        Some(parent_window.to_owned())
    }
}

/// What a request layer offered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Offer {
    /// The request the provider will answer, as the wire names it.
    pub request: RequestId,
    /// The connection that will answer it.
    pub provider: ClientId,
}

/// The privileged connection a `portal.ui` prompt is offered through.
///
/// The portal holds whichever host the composition root gave it: the IPC connection in a
/// session, or a fake in a test. `offer` refusing with [`Refusal::NoProvider`] is how the
/// host says nobody claims the slot, which is the one case that must never be turned into
/// a wait.
pub trait Host: Send {
    /// Offer one prompt and learn its identity.
    ///
    /// `timeout` is how long the offer stands at the request layer: the compositor fails
    /// the request at its own tick when it elapses, and the guard's own deadline is what
    /// releases the waiting caller, so the two agree on the same length of time.
    fn offer(&mut self, prompt: &Prompt, timeout: std::time::Duration) -> Result<Offer, Refusal>;

    /// Withdraw an offer that can no longer be answered.
    fn withdraw(&mut self, request: RequestId);
}

/// Why something was refused before it could wait for an answer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// No provider claims the slot, so nothing would draw the prompt (D14).
    NoProvider {
        /// The slot with no provider.
        slot: &'static str,
    },
    /// The session handle is not one this caller created.
    UnknownSession {
        /// The handle the caller named.
        path: String,
    },
    /// The session belongs to another caller.
    NotOwner {
        /// The handle the caller named.
        path: String,
    },
    /// The session is not in the state the member needs.
    WrongState {
        /// The handle the caller named.
        path: String,
        /// The state it is in.
        state: State,
    },
    /// A prompt for this caller and capability is already waiting.
    Pending {
        /// The handle the caller named.
        path: String,
    },
    /// The user refused and the refusal still stands.
    Refused,
    /// The session is locked, so capture and injection are refused.
    Locked,
    /// The interface cannot ask for this capability.
    NotServed {
        /// The interface that was asked.
        interface: Interface,
        /// What it cannot ask for.
        capability: PortalCapability,
    },
    /// The answer did not arrive for a request this guard offered.
    UnknownRequest {
        /// The request the answer named.
        request: RequestId,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProvider { slot } => write!(
                formatter,
                "no provider claims the `{slot}` slot, so the prompt was not offered"
            ),
            Self::UnknownSession { path } => write!(formatter, "`{path}` is not a session"),
            Self::NotOwner { path } => write!(formatter, "`{path}` belongs to another caller"),
            Self::WrongState { path, state } => {
                write!(formatter, "`{path}` is {state:?}, which this call does not accept")
            }
            Self::Pending { path } => write!(formatter, "`{path}` is already waiting for an answer"),
            Self::Refused => write!(formatter, "the user refused this request"),
            Self::Locked => write!(formatter, "the session is locked"),
            Self::NotServed { interface, capability } => write!(
                formatter,
                "{} does not ask for {}",
                interface.as_str(),
                capability.as_str()
            ),
            Self::UnknownRequest { request } => {
                write!(
                    formatter,
                    "request {} was not offered here, or already ended",
                    request.get()
                )
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// How a consent question ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsentOutcome {
    /// The provider allowed it.
    Granted,
    /// The provider declined it, or declined the request.
    Refused,
    /// Nobody answered before the deadline.
    Expired,
    /// The caller's connection ended.
    Left,
    /// The `portal.ui` provider went away while the prompt was waiting.
    ProviderGone,
    /// The session was locked.
    Locked,
}

impl ConsentOutcome {
    /// The `Response` code `org.freedesktop.impl.portal` uses: 0 success, 1 the user
    /// cancelled, 2 cancelled for any other reason.
    #[must_use]
    pub const fn response(self) -> u32 {
        match self {
            Self::Granted => 0,
            // The user's own answer is the only one the frontend reports as "the user
            // cancelled", because a frontend shows a different sentence for it.
            Self::Refused => 1,
            Self::Expired | Self::Left | Self::ProviderGone | Self::Locked => 2,
        }
    }

    /// Whether the request may proceed.
    #[must_use]
    pub const fn granted(self) -> bool {
        matches!(self, Self::Granted)
    }
}

/// What [`Guard::ask`] could do.
///
/// `AlreadyGranted` is not a prompt: the answer the user gave still stands, so asking
/// again would be a second question with one answer.
pub enum Pending {
    /// The grant is live, so there is nothing to wait for.
    AlreadyGranted,
    /// The prompt is offered; the answer arrives on the receiver.
    Asked {
        /// The request the provider will answer, so a cancellation can name it.
        request: RequestId,
        /// The answer's arrival.
        answered: oneshot::Receiver<ConsentOutcome>,
    },
}

/// One prompt waiting for the provider.
struct Waiting {
    /// The authority's handle for the prompt it is tracking, or nothing for a question
    /// that is not a grant (a shortcut binding, a clipboard transfer).
    prompt: Option<PromptId>,
    bus_name: String,
    session: String,
    capability: Option<PortalCapability>,
    provider: ClientId,
    deadline: Tick,
    wake: oneshot::Sender<ConsentOutcome>,
}

/// The session table and the grants behind it.
pub struct Guard {
    host: Box<dyn Host>,
    authority: Authority,
    sessions: BTreeMap<String, Session>,
    /// Keyed by the calling connection *and* the app id it named: a grant is the app's,
    /// and it ends when the connection that asked for it does.
    requesters: BTreeMap<(String, String), RequesterId>,
    next_requester: u64,
    /// Keyed by the wire's request id, which is what an answer names.
    waiting: BTreeMap<RequestId, Waiting>,
    /// How long a prompt stays open, in the tick the compositor counts.
    lifetime: u64,
    locked: bool,
}

impl fmt::Debug for Guard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Guard")
            .field("sessions", &self.sessions)
            .field("waiting", &self.waiting.len())
            .field("lifetime", &self.lifetime)
            .field("locked", &self.locked)
            .finish_non_exhaustive()
    }
}

impl Guard {
    /// A guard that offers its prompts through `host` and gives each one `lifetime`
    /// ticks to be answered.
    #[must_use]
    pub fn new(host: Box<dyn Host>, lifetime: u64) -> Guard {
        Guard {
            host,
            authority: Authority::new(),
            sessions: BTreeMap::new(),
            requesters: BTreeMap::new(),
            next_requester: 1,
            waiting: BTreeMap::new(),
            lifetime,
            locked: false,
        }
    }

    /// The grant state for one caller, app and capability, for a log line and a test.
    #[must_use]
    pub fn consent(&self, bus_name: &str, app_id: &str, capability: PortalCapability) -> Option<Consent> {
        let requester = self.requesters.get(&(bus_name.to_owned(), app_id.to_owned()))?;
        self.authority.consent(*requester, capability)
    }

    /// How many prompts are waiting.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.waiting.len()
    }

    /// Whether the session is covered.
    #[must_use]
    pub const fn locked(&self) -> bool {
        self.locked
    }

    /// The session at `path`, if it is this caller's.
    #[must_use]
    pub fn session(&self, bus_name: &str, path: &str) -> Option<&Session> {
        self.sessions
            .get(path)
            .filter(|session| session.owner.bus_name == bus_name)
    }

    /// Every live session.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> + '_ {
        self.sessions.values()
    }

    /// Record one session.
    ///
    /// A handle that already exists is refused rather than replaced: the frontend chose
    /// the path, and two sessions behind one handle would mean closing one ends the
    /// other's stream.
    pub fn create(&mut self, new: NewSession) -> Result<(), Refusal> {
        if self.sessions.contains_key(&new.path) {
            return Err(Refusal::NotOwner { path: new.path });
        }
        let interface = new.interface;
        self.sessions.insert(
            new.path.clone(),
            Session {
                path: new.path,
                owner: Requester {
                    bus_name: new.bus_name,
                    app_id: new.app_id,
                    sandboxed: new.sandboxed,
                },
                interface,
                state: State::Created,
                options: std::collections::BTreeMap::new(),
                streams: Vec::new(),
            },
        );
        Ok(())
    }

    /// Mark a session configured, recording what the caller asked for.
    ///
    /// A session that is already started is refused: its sources were chosen when it
    /// started, and a later selection would describe something the caller is already
    /// receiving.
    pub fn configure(
        &mut self,
        bus_name: &str,
        path: &str,
        options: std::collections::BTreeMap<String, Value>,
    ) -> Result<Interface, Refusal> {
        let session = self.owned(bus_name, path)?;
        if session.state != State::Created {
            return Err(Refusal::WrongState {
                path: path.to_owned(),
                state: session.state,
            });
        }
        let interface = session.interface;
        let session = self.sessions.get_mut(path).expect("the session was just found");
        session.state = State::Configured;
        session.options = options;
        Ok(interface)
    }

    /// Mark a session started.
    pub fn start(&mut self, bus_name: &str, path: &str) -> Result<Interface, Refusal> {
        let session = self.owned(bus_name, path)?;
        if session.state == State::Started {
            return Err(Refusal::WrongState {
                path: path.to_owned(),
                state: session.state,
            });
        }
        let interface = session.interface;
        self.sessions.get_mut(path).expect("the session was just found").state = State::Started;
        Ok(interface)
    }

    /// Record what the backend opened for a session.
    pub fn record_stream(&mut self, path: &str, stream: StreamId) -> Result<(), Refusal> {
        let session = self
            .sessions
            .get_mut(path)
            .ok_or_else(|| Refusal::UnknownSession { path: path.to_owned() })?;
        session.streams.push(stream);
        Ok(())
    }

    /// The streams a session holds, so the caller can close them.
    #[must_use]
    pub fn streams(&self, bus_name: &str, path: &str) -> Option<&[StreamId]> {
        self.session(bus_name, path).map(|session| session.streams.as_slice())
    }

    /// Close one session, returning it so the caller ends what it opened.
    ///
    /// Only the owner closes: the frontend tells the backend with
    /// `org.freedesktop.impl.portal.Session.Close` on the handle, and a handle is a path
    /// on a bus where any client may call any path. The owner is what makes the call
    /// mean "end the session I created".
    pub fn close(&mut self, bus_name: &str, path: &str) -> Result<Session, Refusal> {
        self.owned(bus_name, path)?;
        let session = self.sessions.remove(path).expect("the session was just found");
        let abandoned: Vec<RequestId> = self
            .waiting
            .iter()
            .filter(|(_, waiting)| waiting.session == path)
            .map(|(request, _)| *request)
            .collect();
        for request in abandoned {
            self.finish(request, ConsentOutcome::Refused);
        }
        Ok(session)
    }

    /// Ask for one capability on behalf of one session.
    ///
    /// The order is the one that makes D14 true: the offer is refused before it is made
    /// when no provider claims the slot, and only an offer that exists is recorded in the
    /// authority. A grant that still stands answers without a prompt, because the user
    /// already answered for this caller and this capability.
    pub fn ask(
        &mut self,
        now: Tick,
        bus_name: &str,
        path: &str,
        capability: PortalCapability,
        parent_window: &str,
    ) -> Result<Pending, Refusal> {
        let session = self.owned(bus_name, path)?.clone();
        if !session.interface.allows(capability) {
            return Err(Refusal::NotServed {
                interface: session.interface,
                capability,
            });
        }
        if self.locked {
            return Err(Refusal::Locked);
        }
        if self
            .waiting
            .values()
            .any(|waiting| waiting.bus_name == bus_name && waiting.capability == Some(capability))
        {
            return Err(Refusal::Pending { path: path.to_owned() });
        }
        let requester = self.requester(&session.owner.bus_name, &session.owner.app_id);
        match self.authority.gate(now, requester, capability) {
            Decision::Allowed => return Ok(Pending::AlreadyGranted),
            Decision::Denied(Denial::Refused) => return Err(Refusal::Refused),
            Decision::Denied(Denial::Locked) => return Err(Refusal::Locked),
            // A prompt for this caller and capability is already open in the authority.
            Decision::Denied(Denial::NoConsent) => {
                return Err(Refusal::Pending { path: path.to_owned() });
            }
            Decision::Denied(_) => {}
        }

        let prompt = Prompt::new(session.interface, &session.owner, capability, parent_window);
        let deadline = now.saturating_add(self.lifetime);
        let offer = self.host.offer(&prompt, self.timeout())?;
        let prompt_id = PromptId::new(offer.request.get());
        // The authority records who may answer, so its own check and the request layer's
        // agree on one identity.
        let effects = self
            .authority
            .apply(now, Event::PortalUiClaimed { client: offer.provider });
        let _ = self.resolve(effects);
        let effects = self.authority.apply(
            now,
            Event::ConsentRequested {
                request: prompt_id,
                requester,
                capability,
                lifetime: self.lifetime,
            },
        );
        // A prompt effect means the offer stands; a decision means the authority answered
        // from the state it already had, so nothing waits.
        match self.resolve(effects) {
            None => {}
            Some(Decision::Allowed) => {
                self.host.withdraw(offer.request);
                return Ok(Pending::AlreadyGranted);
            }
            Some(Decision::Denied(denial)) => {
                self.host.withdraw(offer.request);
                return Err(refusal_for(denial));
            }
        }

        let (wake, answered) = oneshot::channel();
        self.waiting.insert(
            offer.request,
            Waiting {
                prompt: Some(prompt_id),
                bus_name: bus_name.to_owned(),
                session: path.to_owned(),
                capability: Some(capability),
                provider: offer.provider,
                deadline,
                wake,
            },
        );
        log::info!(request = offer.request.get(),
        app = %session.owner.app_id,
        capability = capability.as_str(),
        interface = session.interface.as_str(),
        "asked portal.ui for consent");
        Ok(Pending::Asked {
            request: offer.request,
            answered,
        })
    }

    /// Ask a question that is not a grant.
    ///
    /// A shortcut binding and a clipboard transfer are choices a user makes and neither
    /// is a capability in the authority's model, so the answer is the D-Bus response and
    /// nothing outlives the call. Everything else is the same as [`Guard::ask`]: D14's
    /// refusal when nobody claims `portal.ui`, one waiting entry, expiry closed, and the
    /// same `allow`/`deny` encoding.
    pub fn ask_once(
        &mut self,
        now: Tick,
        bus_name: &str,
        path: &str,
        title: String,
        message: String,
        parent_window: &str,
    ) -> Result<Pending, Refusal> {
        let session = self.owned(bus_name, path)?.clone();
        if self.locked {
            return Err(Refusal::Locked);
        }
        if self.waiting.values().any(|waiting| waiting.bus_name == bus_name) {
            return Err(Refusal::Pending { path: path.to_owned() });
        }
        let prompt = Prompt {
            // The question is not about a capability. The field is the provider's
            // wording hint, so it names the interface that asked.
            capability: PortalCapability::Capture,
            app_id: session.owner.app_id.clone(),
            parent_window: non_empty(parent_window),
            title,
            message,
        };
        let deadline = now.saturating_add(self.lifetime);
        let offer = self.host.offer(&prompt, self.timeout())?;
        let (wake, answered) = oneshot::channel();
        self.waiting.insert(
            offer.request,
            Waiting {
                prompt: None,
                bus_name: bus_name.to_owned(),
                session: path.to_owned(),
                capability: None,
                provider: offer.provider,
                deadline,
                wake,
            },
        );
        Ok(Pending::Asked {
            request: offer.request,
            answered,
        })
    }

    /// Report how a prompt the provider held ended.
    ///
    /// The answer's encoding is this backend's: the prompt was offered here, so the
    /// bytes are the question's. `allow` grants, `deny` refuses, and anything else is a
    /// denial rather than a grant, because an unreadable answer must never consent.
    pub fn answer(&mut self, now: Tick, request: RequestId, answer: Option<&[u8]>) -> Result<ConsentOutcome, Refusal> {
        let Some(waiting) = self.waiting.get(&request) else {
            return Err(Refusal::UnknownRequest { request });
        };
        let reply = match answer {
            Some(b"allow") => Answer::Allow,
            Some(other) => {
                log::warn!(
                    request = request.get(),
                    length = other.len(),
                    "a consent answer was not `allow`; treating it as a refusal"
                );
                Answer::Deny
            }
            None => Answer::Deny,
        };
        if let Some(prompt) = waiting.prompt {
            let effects = self.authority.apply(
                now,
                Event::ConsentAnswered {
                    request: prompt,
                    from: waiting.provider,
                    answer: reply,
                },
            );
            let _ = self.resolve(effects);
        }
        let outcome = if reply == Answer::Allow {
            ConsentOutcome::Granted
        } else {
            ConsentOutcome::Refused
        };
        self.finish(request, outcome);
        Ok(outcome)
    }

    /// End every prompt at or past its deadline, closed.
    pub fn tick(&mut self, now: Tick) -> Vec<RequestId> {
        let overdue: Vec<RequestId> = self
            .waiting
            .iter()
            .filter(|(_, waiting)| waiting.deadline <= now)
            .map(|(request, _)| *request)
            .collect();
        for request in &overdue {
            let Some(waiting) = self.waiting.remove(request) else {
                continue;
            };
            // Expiring in the authority is what removes the pending grant, so a late
            // answer cannot become consent.
            if let Some(prompt) = waiting.prompt {
                let effects = self.authority.apply(
                    now,
                    Event::ConsentAnswered {
                        request: prompt,
                        from: waiting.provider,
                        answer: Answer::Deny,
                    },
                );
                let _ = self.resolve(effects);
            }
            self.host.withdraw(*request);
            let _ = waiting.wake.send(ConsentOutcome::Expired);
            log::info!(request = request.get(), "a consent prompt expired closed");
        }
        overdue
    }

    /// The caller's connection ended: its grants and its sessions end with it.
    ///
    /// Every session the caller created is returned, so the caller of this method ends
    /// the streams the backend opened. A session that outlived a crash is exactly the
    /// case this covers: the bus tells the portal the name is gone, and nothing that
    /// name created stays alive behind it.
    pub fn requester_gone(&mut self, now: Tick, bus_name: &str) -> Vec<Session> {
        let gone: Vec<RequesterId> = self
            .requesters
            .iter()
            .filter(|((name, _), _)| name == bus_name)
            .map(|(_, requester)| *requester)
            .collect();
        if gone.is_empty() {
            return Vec::new();
        }
        // Finish the caller's own prompts as `Left` before the authority is told the
        // requester is gone: its `Dismiss` would otherwise end them as `ProviderGone`.
        let abandoned: Vec<RequestId> = self
            .waiting
            .iter()
            .filter(|(_, waiting)| waiting.bus_name == bus_name)
            .map(|(request, _)| *request)
            .collect();
        for request in abandoned {
            self.finish(request, ConsentOutcome::Left);
        }
        self.requesters.retain(|(name, _), _| name != bus_name);
        for requester in gone {
            let effects = self.authority.apply(now, Event::RequesterGone { requester });
            let _ = self.resolve(effects);
        }
        let paths: Vec<String> = self
            .sessions
            .values()
            .filter(|session| session.owner.bus_name == bus_name)
            .map(|session| session.path.clone())
            .collect();
        paths
            .into_iter()
            .filter_map(|path| self.sessions.remove(&path))
            .collect()
    }

    /// The `portal.ui` provider left.
    ///
    /// Its waiting prompts can no longer be answered, so they end here rather than at
    /// their deadline.
    pub fn provider_gone(&mut self, now: Tick, provider: ClientId) {
        let effects = self.authority.apply(now, Event::PortalUiReleased { client: provider });
        // The authority answers each pending prompt with `NoPortalUi` rather than
        // dismissing it, and `resolve` only reports that decision. Wake each waiter here.
        let answered: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Answer { request, .. } => Some(*request),
                _ => None,
            })
            .collect();
        let _ = self.resolve(effects);
        for request in answered {
            if let Some(ipc) = self
                .waiting
                .iter()
                .find(|(_, waiting)| waiting.prompt == Some(request))
                .map(|(ipc, _)| *ipc)
            {
                self.finish(ipc, ConsentOutcome::ProviderGone);
            }
        }
    }

    /// The session lock changed.
    ///
    /// A pending prompt ends closed: capture through a lock is the case the user was not
    /// asked about.
    pub fn lock(&mut self, now: Tick, active: bool) {
        if self.locked == active {
            return;
        }
        self.locked = active;
        let effects = self.authority.apply(now, Event::LockChanged { active });
        let _ = self.resolve(effects);
        if !active {
            return;
        }
        let waiting: Vec<RequestId> = self.waiting.keys().copied().collect();
        for request in waiting {
            self.finish(request, ConsentOutcome::Locked);
        }
    }

    /// How long an offer stands, in the duration the request layer takes.
    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.lifetime)
    }

    /// The session at `path`, if the caller owns it.
    fn owned(&self, bus_name: &str, path: &str) -> Result<&Session, Refusal> {
        let session = self
            .sessions
            .get(path)
            .ok_or_else(|| Refusal::UnknownSession { path: path.to_owned() })?;
        if session.owner.bus_name != bus_name {
            return Err(Refusal::NotOwner { path: path.to_owned() });
        }
        Ok(session)
    }

    /// The authority's identity for one connection and app, allocated once.
    fn requester(&mut self, bus_name: &str, app_id: &str) -> RequesterId {
        let key = (bus_name.to_owned(), app_id.to_owned());
        if let Some(requester) = self.requesters.get(&key) {
            return *requester;
        }
        let requester = RequesterId::new(self.next_requester);
        self.next_requester += 1;
        self.requesters.insert(key, requester);
        requester
    }

    /// Wake one waiting caller and withdraw its offer.
    ///
    /// The waiting entry is removed here, so a second wake for the same request is
    /// impossible and a late answer is [`Refusal::UnknownRequest`].
    fn finish(&mut self, request: RequestId, outcome: ConsentOutcome) {
        let Some(waiting) = self.waiting.remove(&request) else {
            return;
        };
        self.host.withdraw(request);
        let _ = waiting.wake.send(outcome);
    }

    /// Apply the authority's effects, returning the decision a prompt resolved with.
    ///
    /// A `Dismiss` is a prompt the authority can no longer receive an answer for (the
    /// provider changed), so the waiting caller is woken instead of being left until its
    /// deadline. The effects this guard cannot produce (a launch, a gesture, an
    /// activation) are logged: this authority instance is fed consent and lock events
    /// only, so one of those would be a bug in what fed it.
    fn resolve(&mut self, effects: Vec<Effect>) -> Option<Decision> {
        let mut decision = None;
        for effect in effects {
            match effect {
                Effect::Dismiss { request } => {
                    if let Some(ipc) = self
                        .waiting
                        .iter()
                        .find(|(_, waiting)| waiting.prompt == Some(request))
                        .map(|(ipc, _)| *ipc)
                    {
                        self.finish(ipc, ConsentOutcome::ProviderGone);
                    }
                }
                Effect::Answer { decision: answer, .. } => decision = Some(answer),
                Effect::Prompt { .. }
                | Effect::Ignored { .. }
                | Effect::EndGrants { .. }
                | Effect::Capture { .. }
                | Effect::PortalUi { .. }
                | Effect::Gate { .. } => {}
                other => log::debug!(?other, "the authority produced an effect no prompt uses"),
            }
        }
        decision
    }
}

/// The refusal that names a denial the authority answered with immediately.
fn refusal_for(denial: Denial) -> Refusal {
    match denial {
        Denial::Locked => Refusal::Locked,
        Denial::Refused => Refusal::Refused,
        Denial::NoPortalUi => Refusal::NoProvider { slot: PORTAL_UI },
        Denial::ConsentExpired => Refusal::Refused,
        _ => Refusal::NoProvider { slot: PORTAL_UI },
    }
}
