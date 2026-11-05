//! `gnoblin_session_host_v1`: the host compositor's channel to one hosted session.
//!
//! Design/rust-compositor.md, "Hosting sessions" primitive 2 and D4. A host
//! Gnoblin runs on the real seat; each signed-in user's Gnoblin runs on the
//! nested backend as a client of the host. This protocol is the conversation
//! between the two, and it replaces the `bsm-host` bridge, bsm's own
//! `bsm_session_host_v1` and bsm's `org.gnoblin.Hosted1`.
//!
//! ## Who may speak it
//!
//! Only the connection the compositor brokered for a session, which is a
//! [`ClientRole::SessionHost`](crate::ClientRole) in the access roster and is
//! recorded before its first request. A greeter, an application, or the
//! session's own clients never see the global, so a lock, logout or power
//! request cannot be forged from inside the session it would end.
//!
//! ## The two barriers
//!
//! Protection has two halves and both are required before anything is
//! announced:
//!
//! 1. The host hides the session's surfaces and waits for a frame that was
//!    described afterwards to reach every output
//!    ([`gnoblin_lock::Protection`](../../gnoblin_lock/protection/index.html)).
//! 2. The session covers its own scene and answers `protected` once its own
//!    cover frame was presented, which is the same present barrier the session
//!    lock uses.
//!
//! A session that crashes while hidden never sends the second half, so it is
//! never announced and its cover stays. A session that answers before its frame
//! was presented is not believed: the host's own barrier is independent.
//!
//! ## Nothing here is performed
//!
//! `lock`, `logout` and `power` are published as [`gnoblin_model`] `Request`
//! objects, which is the IPC's one way to ask a provider to decide something.
//! The compositor never locks the seat, ends a session or calls logind because a
//! client asked: bsm's manager claims the request and answers it, and the
//! session keeps running until it does.
//!
//! ## Failure modes first
//!
//! Each entry names a test in `tests/session_host.rs`.
//!
//! 1. **The global is visible to an ordinary client, or to a session on the
//!    privileged socket.** It is offered only to a brokered session connection:
//!    `an_ordinary_client_sees_no_session_host` and
//!    `a_session_sees_only_the_session_host_protocol`.
//! 2. **A cover the host cannot prove is announced as a protection.** It is not:
//!    the announcement waits for the session's acknowledgement *and* the host's
//!    own frame, so a session that is hidden but unproven stays `covering`:
//!    `a_cover_the_host_cannot_prove_never_protects` and
//!    `an_acknowledgement_without_a_cover_frame_announces_nothing`.
//! 3. **A session that acknowledges a protection it was never sent is believed.**
//!    It is a protocol error and its connection ends:
//!    `an_unknown_acknowledgement_is_a_protocol_error`.
//! 4. **A lock, logout or power request is performed by the compositor.** Each
//!    becomes an offered `Request` and nothing else:
//!    `a_lock_request_reaches_the_ipc_as_a_request`,
//!    `a_logout_request_reaches_the_ipc_as_a_request` and
//!    `a_power_request_carries_its_action`.
//! 5. **An output hint with a size nothing can draw is accepted, or a half-sent
//!    set reaches the manager.** Both are refused:
//!    `an_invalid_output_hint_is_a_protocol_error` and
//!    `a_half_sent_hint_set_is_not_published`.
//! 6. **A protected session keeps the keyboard.** Input reaches the windows that
//!    are still on screen and nothing else:
//!    `a_protected_session_receives_no_input`.
//! 7. **Readiness is taken for a request to be shown.** `ready` is published and
//!    ends nothing: `ready_is_recorded_and_ends_nothing`.
//!
//! ## What is not proven yet
//!
//! The host's half of the barrier is translated and armed, and
//! [`gnoblin_lock::Protection`] requires it, but the frame path in this crate does
//! not yet report a presented frame back for it: a protection therefore stays
//! `covering` forever instead of becoming `covered`. That is the fail-closed
//! direction — the session is hidden, receives no input and is never announced —
//! and the tests above assert exactly that, never the opposite. Making the frame
//! path report it is the next change here; nothing else in this module depends on
//! the answer, because the answer is the session manager's to wait for.
//! ## Nothing here is performed
//!
//! `lock`, `logout` and `power` are published as [`gnoblin_model`] `Request`
//! objects, which is the IPC's one way to ask a provider to decide something.
//! The compositor never locks the seat, ends a session or calls logind because a
//! client asked: bsm's manager claims the request and answers it, and the
//! session keeps running until it does.
//!
//! ## Failure modes first
//!
//! Each entry names a test in `tests/session_host.rs`.
//!
//! 1. **The global is visible to an ordinary client, or to a session on the
//!    privileged socket.** It is offered only to a brokered session connection:
//!    `an_ordinary_client_sees_no_session_host` and
//!    `a_session_sees_only_the_session_host_protocol`.
//! 2. **Protection is announced before a rendered cover frame.** The announcement
//!    waits for a frame without the session on every output *and* the session's
//!    acknowledgement: `protection_waits_for_a_presented_cover`.
//! 3. **The session acknowledges before its own cover was presented.** The host
//!    still holds the cover and announces nothing:
//!    `an_acknowledgement_without_a_cover_frame_announces_nothing` (the client
//!    part of the same test).
//! 4. **A session that crashes while protected unlocks something.** The cover
//!    stays and the session is never announced:
//!    `a_session_that_crashes_while_protected_keeps_the_cover`.
//! 5. **A stale acknowledgement is taken for the current protection.** It is
//!    ignored; an acknowledgement of a serial never sent is a protocol error:
//!    `a_stale_acknowledgement_changes_nothing` and
//!    `an_unknown_acknowledgement_is_a_protocol_error`.
//! 6. **A lock, logout or power request is performed by the compositor.** Each
//!    becomes an offered `Request` and nothing else:
//!    `a_logout_request_reaches_the_ipc_as_a_request`,
//!    `a_lock_request_reaches_the_ipc_as_a_request` and
//!    `a_power_request_carries_its_action`.
//! 7. **An output hint with a size nothing can draw is accepted.**
//!    `an_invalid_output_hint_is_a_protocol_error`, and a half-sent set reaches
//!    the manager: `a_half_sent_hint_set_is_not_published`.
//! 8. **A protected session keeps the keyboard.** Input reaches the windows that
//!    are still on screen and nothing else:
//!    `a_protected_session_receives_no_input`.

use std::collections::{BTreeMap, HashMap};

use smithay::reexports::wayland_server::backend::ClientId as WaylandClientId;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, WEnum};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use tracing::{debug, info, warn};

use gnoblin_lock::{Protection, ProtectionEffect, ProtectionEvent, ProtectionFailure, ProtectionState};
use gnoblin_model::{RequestBroker, RequestError, RequestState};
use gnoblin_types::{ClientId, OwnerId, RequestId, Tick, WindowId};

use crate::State;
use crate::access::{Access, AccessPolicy, Global};
use crate::protocols::wire::session_host::gnoblin_session_host_v1::{
    self as host, GnoblinSessionHostV1, PowerAction as WirePowerAction,
};

/// How many loop turns a session manager has to claim or answer a request.
///
/// The broker is ticked with the session's own turn counter, the only clock the
/// protocol side reads, so this is the same kind of deadline the activation
/// tokens use. A turn is one loop iteration, so a busier session expires a
/// manager's request sooner in wall-clock terms; the manager is expected to claim
/// what it is offered.
const REQUEST_DEADLINE_TURNS: u64 = 10_000;

/// The longest output name an hint may carry, which bounds a session's memory.
const MAX_HINT_NAME: usize = 128;

/// What a session asked the machine to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum PowerAction {
    /// Restart the machine.
    Reboot,
    /// Power it off.
    PowerOff,
    /// Suspend it.
    Suspend,
    /// Hibernate it.
    Hibernate,
}

impl PowerAction {
    /// The action a wire value names, or [`None`] for one the protocol does not.
    #[must_use]
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Reboot,
            1 => Self::PowerOff,
            2 => Self::Suspend,
            3 => Self::Hibernate,
            _ => return None,
        })
    }

    /// The value the wire uses.
    #[must_use]
    pub const fn to_wire(self) -> u32 {
        match self {
            Self::Reboot => 0,
            Self::PowerOff => 1,
            Self::Suspend => 2,
            Self::Hibernate => 3,
        }
    }

    /// A stable lower-case name for logs, requests and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reboot => "reboot",
            Self::PowerOff => "poweroff",
            Self::Suspend => "suspend",
            Self::Hibernate => "hibernate",
        }
    }
}

/// What a session asked the manager to decide.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostRequestKind {
    /// Lock the seat, because a lock source inside the session fired.
    Lock,
    /// End this session.
    Logout,
    /// Do something to the machine.
    Power(PowerAction),
}

impl HostRequestKind {
    /// A stable lower-case name for logs, requests and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lock => "lock",
            Self::Logout => "logout",
            Self::Power(action) => action.as_str(),
        }
    }
}

/// One request a hosted session made, as the manager sees it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostRequest {
    /// The broker's identity for it, which is what a claim and an answer name.
    pub request: RequestId,
    /// The session that asked.
    pub session: ClientId,
    /// What it asked for.
    pub kind: HostRequestKind,
    /// When an unanswered request fails.
    pub deadline: Tick,
}

/// How a session wants one of its outputs shown.
///
/// Advisory by design: the host places the session's surfaces itself, and this
/// only says what the session would like.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputHint {
    /// The session's own name for the output.
    pub name: String,
    /// Its width in logical pixels.
    pub width: i32,
    /// Its height in logical pixels.
    pub height: i32,
    /// Its scale, as the protocol's 24.8 fixed point.
    pub scale: f64,
}

/// A hosted session, as the manager and a test read it.
#[derive(Clone, Debug, PartialEq)]
pub struct HostedSession {
    /// The identity this session gave the connection, which is also the subject
    /// of a protection cover.
    pub client: ClientId,
    /// The process the connection belongs to.
    pub pid: u32,
    /// Whether the session said it can be shown.
    pub ready: bool,
    /// Where the protection cover is.
    pub protection: ProtectionState,
    /// The last complete set of hints, in name order.
    pub hints: Vec<OutputHint>,
}

/// One hosted session's connection and what it has said.
struct Hosted {
    /// The protocol object, which is how an event reaches the session.
    object: GnoblinSessionHostV1,
    /// The identity the protection cover names.
    client: ClientId,
    /// The process behind the connection.
    pid: u32,
    /// Whether `ready` arrived.
    ready: bool,
    /// The hints of the set being sent.
    staged: BTreeMap<String, OutputHint>,
    /// The last complete set.
    hints: BTreeMap<String, OutputHint>,
    /// The windows that were hidden for this session's cover.
    hidden: Vec<WindowId>,
}

impl Hosted {
    /// The body of what a reader sees.
    fn report(&self, protection: ProtectionState) -> HostedSession {
        HostedSession {
            client: self.client,
            pid: self.pid,
            ready: self.ready,
            protection,
            hints: self.hints.values().cloned().collect(),
        }
    }
}

/// The session-host protocol's objects, its protection covers and its requests.
pub(crate) struct SessionHostTables {
    /// Every hosted connection, by its Wayland identity.
    hosts: HashMap<WaylandClientId, Hosted>,
    /// The protection covers, which is the policy this module performs.
    pub(crate) protection: Protection,
    /// The requests a session asked for, which the manager claims and answers.
    requests: RequestBroker,
    /// What each live request asked for.
    kinds: BTreeMap<RequestId, (ClientId, HostRequestKind, Tick)>,
}

impl Default for SessionHostTables {
    fn default() -> Self {
        Self {
            hosts: HashMap::new(),
            protection: Protection::new(),
            requests: RequestBroker::new(),
            kinds: BTreeMap::new(),
        }
    }
}

/// The global's data: who may bind it.
pub(crate) struct HostGlobal {
    /// The roster that answers which connection is a hosted session.
    access: std::sync::Arc<Access>,
}

impl SessionHostTables {
    /// Registers the global.
    ///
    /// The global exists even with no session connected, because the roster is the
    /// whole answer to who may see it and a session is brokered after the session
    /// is running.
    pub(crate) fn new(display_handle: &DisplayHandle, access: &std::sync::Arc<Access>) -> Self {
        let _ = display_handle.create_global::<State, GnoblinSessionHostV1, HostGlobal>(
            host::GnoblinSessionHostV1::interface().version,
            HostGlobal {
                access: std::sync::Arc::clone(access),
            },
        );
        Self::default()
    }

    /// The hosted session the process behind a connection runs.
    fn by_pid(&self, pid: u32) -> Option<(ClientId, &Hosted)> {
        self.hosts
            .values()
            .find(|host| host.pid == pid)
            .map(|host| (host.client, host))
    }

    /// The wayland identity of a hosted session's connection.
    fn key_of(&self, client: ClientId) -> Option<&WaylandClientId> {
        self.hosts
            .iter()
            .find(|(_, host)| host.client == client)
            .map(|(key, _)| key)
    }

    /// The hosted session a connection belongs to.
    fn client_of(&self, key: &WaylandClientId) -> Option<ClientId> {
        self.hosts.get(key).map(|host| host.client)
    }

    /// Offers a request to the manager.
    fn offer(&mut self, session: ClientId, kind: HostRequestKind, deadline: Tick) -> RequestId {
        let request = self.requests.offer(deadline);
        self.kinds.insert(request, (session, kind, deadline));
        info!(
            ?session,
            kind = kind.as_str(),
            ?request,
            "[host] a session asked the manager"
        );
        request
    }

    /// Every request still waiting for the manager.
    fn live(&self) -> Vec<HostRequest> {
        self.kinds
            .iter()
            .filter(|(request, _)| matches!(self.requests.state(**request), Some(RequestState::Offered { .. })))
            .map(|(request, (session, kind, deadline))| HostRequest {
                request: *request,
                session: *session,
                kind: *kind,
                deadline: *deadline,
            })
            .collect()
    }
}

impl GlobalDispatch2<GnoblinSessionHostV1, State> for HostGlobal {
    fn bind(
        &self,
        state: &mut State,
        _handle: &DisplayHandle,
        client: &Client,
        resource: New<GnoblinSessionHostV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let object = data_init.init(resource, HostObject);
        let key = client.id();
        // The lock hands out the identity a connection is known by, so the protection
        // cover and a session lock name the same client the same way.
        let Some(identity) = state.client_identity(client) else {
            return;
        };
        // The process is the only thing a manager can name a session by before it
        // has spoken: the compositor brokered the connection, so the pid is the one
        // it started.
        let pid = client
            .get_credentials(&state.display_handle)
            .map_or(0, |credentials| u32::try_from(credentials.pid).unwrap_or(0));
        debug!(?identity, pid, "[host] a session connection bound its channel");
        if state.session_host.hosts.contains_key(&key) {
            // A second object on one connection would give the session two ways to
            // answer one protection, so it is refused and the first stays.
            object.post_error(host::Error::AlreadyBound as u32, "one session host per connection");
            return;
        }
        state.session_host.hosts.insert(
            key,
            Hosted {
                object,
                client: identity,
                pid,
                ready: false,
                staged: BTreeMap::new(),
                hints: BTreeMap::new(),
                hidden: Vec::new(),
            },
        );
    }

    fn can_view(&self, client: &Client) -> bool {
        AccessPolicy::allows(&*self.access, &client.id(), Global::SessionHost)
    }
}

/// The user data of a bound session host object.
struct HostObject;

impl Dispatch2<GnoblinSessionHostV1, State> for HostObject {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        resource: &GnoblinSessionHostV1,
        request: host::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let key = client.id();
        let Some(identity) = state.session_host.client_of(&key) else {
            return;
        };
        match request {
            host::Request::Destroy => {
                state.session_host.hosts.remove(&key);
                // A destroyed object is not a release: the cover stays until the
                // manager says otherwise, exactly as a locker's does.
                state.apply_protection(vec![ProtectionEffect::KeepCovered {
                    client: identity,
                    reason: ProtectionFailure::ProtocolDisabled,
                }]);
            }
            host::Request::Ready => {
                let Some(host) = state.session_host.hosts.get_mut(&key) else {
                    return;
                };
                if host.ready {
                    // Readiness is a level, so saying it twice changes nothing.
                    return;
                }
                host.ready = true;
                info!(?identity, "[host] the session can be shown");
                state.publish();
            }
            host::Request::Protected { serial } => {
                let serial = u64::from(serial);
                let effects = state.session_host.protection.apply(ProtectionEvent::Acknowledged {
                    client: identity,
                    serial,
                });
                state.apply_protection(effects);
            }
            host::Request::Lock => {
                let deadline = state.turn.saturating_add(REQUEST_DEADLINE_TURNS);
                state.session_host.offer(identity, HostRequestKind::Lock, deadline);
            }
            host::Request::Logout => {
                let deadline = state.turn.saturating_add(REQUEST_DEADLINE_TURNS);
                state.session_host.offer(identity, HostRequestKind::Logout, deadline);
            }
            host::Request::Power { action } => {
                let action = match action {
                    WEnum::Value(WirePowerAction::Reboot) => Some(PowerAction::Reboot),
                    WEnum::Value(WirePowerAction::PowerOff) => Some(PowerAction::PowerOff),
                    WEnum::Value(WirePowerAction::Suspend) => Some(PowerAction::Suspend),
                    WEnum::Value(WirePowerAction::Hibernate) => Some(PowerAction::Hibernate),
                    WEnum::Unknown(value) => PowerAction::from_wire(value),
                };
                let Some(action) = action else {
                    resource.post_error(host::Error::InvalidPowerAction as u32, "unknown power action");
                    return;
                };
                let deadline = state.turn.saturating_add(REQUEST_DEADLINE_TURNS);
                state
                    .session_host
                    .offer(identity, HostRequestKind::Power(action), deadline);
            }
            host::Request::OutputHint {
                name,
                width,
                height,
                scale,
            } => {
                let valid = !name.is_empty()
                    && name.len() <= MAX_HINT_NAME
                    && width > 0
                    && height > 0
                    && scale.is_finite()
                    && scale > 0.0;
                if !valid {
                    resource.post_error(host::Error::InvalidHint as u32, "invalid output hint");
                    return;
                }
                if let Some(session) = state.session_host.hosts.get_mut(&key) {
                    session.staged.insert(
                        name.clone(),
                        OutputHint {
                            name,
                            width,
                            height,
                            scale,
                        },
                    );
                }
            }
            host::Request::OutputHintDone => {
                let Some(session) = state.session_host.hosts.get_mut(&key) else {
                    return;
                };
                // A set replaces the previous one as a whole, so a manager never
                // acts on a half-updated layout.
                session.hints = std::mem::take(&mut session.staged);
                let count = session.hints.len();
                debug!(?identity, count, "[host] the session published its outputs");
            }
        }
    }

    fn destroyed(&self, state: &mut State, client: WaylandClientId, _resource: &GnoblinSessionHostV1) {
        let Some(host) = state.session_host.hosts.remove(&client) else {
            return;
        };
        let effects = state
            .session_host
            .protection
            .apply(ProtectionEvent::SessionGone { client: host.client });
        state.apply_protection(effects);
        debug!(client = ?host.client, "[host] the session channel is gone");
    }
}

impl State {
    /// Every session request still waiting for the manager.
    #[must_use]
    pub fn host_requests(&self) -> Vec<HostRequest> {
        self.session_host.live()
    }

    /// Lets one owner claim a session request, as the IPC's request object does.
    ///
    /// # Errors
    ///
    /// [`RequestError`] when the request is unknown, finished, or held by another
    /// owner.
    pub fn claim_host_request(&mut self, request: RequestId, owner: OwnerId) -> Result<(), RequestError> {
        self.session_host.requests.claim(request, owner)
    }

    /// Answers a session request its owner holds.
    ///
    /// # Errors
    ///
    /// [`RequestError`] when the owner does not hold the request or it finished.
    pub fn answer_host_request(
        &mut self,
        request: RequestId,
        owner: OwnerId,
        bytes: Vec<u8>,
    ) -> Result<(), RequestError> {
        self.session_host.requests.answer(request, owner, bytes)
    }

    /// Declines a session request its owner holds.
    ///
    /// # Errors
    ///
    /// [`RequestError`] when the owner does not hold the request or it finished.
    pub fn decline_host_request(&mut self, request: RequestId, owner: OwnerId) -> Result<(), RequestError> {
        self.session_host.requests.decline(request, owner)
    }

    /// What the manager can see about the session process `pid` runs.
    #[must_use]
    pub fn hosted_session(&self, pid: u32) -> Option<HostedSession> {
        self.session_host
            .by_pid(pid)
            .map(|(client, host)| host.report(self.session_host.protection.state(client)))
    }

    /// Where a hosted session's protection cover is.
    #[must_use]
    pub fn hosted_protection(&self, pid: u32) -> Option<ProtectionState> {
        self.session_host
            .by_pid(pid)
            .map(|(client, _)| self.session_host.protection.state(client))
    }

    /// Whether a window belongs to a session that is under a cover.
    ///
    /// This is the one question the input and frame paths ask, so a covered
    /// session's windows are never focused, never hit by the pointer and never
    /// composed.
    #[must_use]
    pub(crate) fn window_is_covered(&self, window: WindowId) -> bool {
        let Some(surface) = self.windows.get(&window).and_then(|entry| entry.surface.wl_surface()) else {
            return false;
        };
        let Some(key) = surface.client().map(|client| client.id()) else {
            return false;
        };
        let Some(client) = self.session_host.client_of(&key) else {
            return false;
        };
        self.session_host.protection.is_hidden(client)
    }

    /// Asks for one hosted session to be protected.
    ///
    /// This is what the manager's `Protect` becomes: the session's surfaces leave
    /// the scene at once, the session is asked to cover itself, and nothing is
    /// announced until both halves are proven. Returns whether a session with that
    /// process was found; a session that is already protected is answered with its
    /// state instead of being asked again.
    pub fn protect_hosted_session(&mut self, pid: u32) -> bool {
        let Some((client, _)) = self.session_host.by_pid(pid) else {
            return false;
        };
        let effects = self.session_host.protection.apply(ProtectionEvent::Protect { client });
        self.apply_protection(effects);
        true
    }

    /// Shows a hosted session again.
    ///
    /// Only the manager asks for this: a session cannot release itself, which is
    /// what makes the manager's protection state the only way a session becomes
    /// visible.
    pub fn release_hosted_session(&mut self, pid: u32) -> bool {
        let Some((client, _)) = self.session_host.by_pid(pid) else {
            return false;
        };
        let effects = self.session_host.protection.apply(ProtectionEvent::Release { client });
        let released = !effects.is_empty();
        self.apply_protection(effects);
        if released {
            // The session's own cover is its own: only this event lifts it, so a
            // release always tells the session even when nothing was on screen.
            let object = self
                .session_host
                .key_of(client)
                .and_then(|key| self.session_host.hosts.get(key))
                .map(|host| host.object.clone());
            if let Some(object) = object
                && object.is_alive()
            {
                object.unprotect();
            }
        }
        released
    }

    /// One output presented a frame described after the cover was raised.
    pub(crate) fn protection_presented(&mut self, output: gnoblin_types::OutputId) {
        let Some(generation) = self.protection_generation else {
            // The frame was described before any cover existed, so it proves nothing.
            return;
        };
        let effects = self
            .session_host
            .protection
            .apply(ProtectionEvent::Presented { generation, output });
        self.apply_protection(effects);
    }

    /// Reports the output layout to the covers, so a hotplug restarts a barrier
    /// exactly as it does for the session lock.
    pub(crate) fn report_hosted_outputs(&mut self) {
        let outputs = self.output_ids();
        let effects = self
            .session_host
            .protection
            .apply(ProtectionEvent::OutputsChanged { outputs });
        self.apply_protection(effects);
    }

    /// Performs what the protection cover asked for, in the order it asked.
    pub(crate) fn apply_protection(&mut self, effects: Vec<ProtectionEffect>) {
        for effect in effects {
            match effect {
                ProtectionEffect::Hide { client, generation } => {
                    self.hide_hosted_windows(client);
                    debug!(?client, generation, "[host] the session's surfaces are hidden");
                }
                ProtectionEffect::Ask { client, serial } => {
                    let serial = u32::try_from(serial).unwrap_or(u32::MAX);
                    let object = self
                        .session_host
                        .key_of(client)
                        .and_then(|key| self.session_host.hosts.get(key))
                        .map(|host| host.object.clone());
                    if let Some(object) = object
                        && object.is_alive()
                    {
                        object.protect(serial);
                    }
                }
                ProtectionEffect::AwaitPresentation { generation, outputs } => {
                    debug!(generation, ?outputs, "[host] waiting for a covered frame");
                }
                ProtectionEffect::Protected { client, serial } => {
                    info!(?client, serial, "[host] the session is protected");
                }
                ProtectionEffect::Show { client } => {
                    self.show_hosted_windows(client);
                    debug!(?client, "[host] the session may be shown again");
                }
                ProtectionEffect::KeepCovered { client, reason } => {
                    info!(?client, reason = reason.as_str(), "[host] the cover stays");
                }
                ProtectionEffect::UnknownAck { client, serial } => {
                    warn!(
                        ?client,
                        serial, "[host] the session acknowledged a protection it never got"
                    );
                    let object = self
                        .session_host
                        .key_of(client)
                        .and_then(|key| self.session_host.hosts.get(key))
                        .map(|host| host.object.clone());
                    if let Some(object) = object {
                        object.post_error(
                            host::Error::StaleProtection as u32,
                            "acknowledged a protection that was never requested",
                        );
                    }
                }
            }
        }
        self.publish();
    }

    /// Takes one session's windows out of the scene.
    fn hide_hosted_windows(&mut self, client: ClientId) {
        let Some(key) = self.session_host.key_of(client).cloned() else {
            return;
        };
        let hidden: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, entry)| {
                entry
                    .surface
                    .wl_surface()
                    .and_then(|surface| surface.client())
                    .is_some_and(|owner| owner.id() == key)
            })
            .map(|(window, _)| *window)
            .collect();
        for window in &hidden {
            self.composer.set_window_visible(*window, false);
        }
        if let Some(host) = self.session_host.hosts.get_mut(&key) {
            host.hidden = hidden;
        }
        // Input stops at the same moment the pixels do: a covered session that still
        // held the keyboard would type into a screen nobody can see.
        if self.wm.focused().is_some_and(|window| self.window_is_covered(window)) {
            self.set_keyboard_focus(None);
            self.clear_pointer_focus();
        }
    }

    /// Puts a session's windows back into the scene.
    fn show_hosted_windows(&mut self, client: ClientId) {
        let Some(key) = self.session_host.key_of(client).cloned() else {
            return;
        };
        let hidden = self
            .session_host
            .hosts
            .get_mut(&key)
            .map(|host| std::mem::take(&mut host.hidden))
            .unwrap_or_default();
        for window in hidden {
            if self.windows.contains_key(&window) {
                self.composer.set_window_visible(window, true);
            }
        }
    }

    /// Notices sessions that went away and requests that expired.
    ///
    /// A session that disconnects sends no request, so its cover keeps standing
    /// until the manager releases a session that no longer exists; the request
    /// broker fails closed on its own deadline.
    pub(crate) fn poll_hosted(&mut self) {
        let dead: Vec<WaylandClientId> = self
            .session_host
            .hosts
            .iter()
            .filter(|(_, host)| !host.object.is_alive())
            .map(|(key, _)| key.clone())
            .collect();
        for key in dead {
            if let Some(host) = self.session_host.hosts.remove(&key) {
                let effects = self
                    .session_host
                    .protection
                    .apply(ProtectionEvent::SessionGone { client: host.client });
                self.apply_protection(effects);
            }
        }
        // A frame that could not be produced takes its damage with it, so a cover
        // that is still unproven is described again every turn until one reaches the
        // outputs. Never the other way round: the protection stays owed.
        if self.session_host.protection.is_proving() {
            self.damage_outputs();
        }
        let expired = self.session_host.requests.tick(self.turn);
        if expired > 0 {
            // A request nobody claimed fails closed: the session hears nothing and
            // the manager sees it expire, exactly as a lock request with no
            // subscriber does today.
            debug!(expired, "[host] unanswered session requests expired");
            self.session_host.kinds.retain(|request, _| {
                !matches!(
                    self.session_host.requests.state(*request),
                    Some(RequestState::Finished(_))
                )
            });
        }
    }
}
