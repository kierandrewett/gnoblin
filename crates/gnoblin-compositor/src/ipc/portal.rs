//! The compositor-side service that connects `portal.ui` requests to the [`PortalUi`]
//! session. It records the owner of each admitted request, settles each outcome to
//! that owner, withdraws a departed connection's requests, and settles open requests on a lock.
//!
//! [`PortalService`] owns the [`PortalUi`] session and the owner of every request
//! it admitted. Each method returns the replies the dispatcher must write, so the
//! service itself never touches a socket. Its rules:
//!
//! - A request is answered by the connection that submitted it, and by no other.
//!   When the owner departs, its requests are withdrawn and no reply is sent for them.
//! - `portal.cancel` withdraws every request the calling connection holds. The
//!   wire carries no target id, so a connection's cancel names its own requests.
//! - `portal.key` routes one key to the open dialog and replies with an empty ok.
//!   Keys the dialog does not use are ignored by the dialog itself.
//! - Interfaces without a built-in dialog, and the file and app choosers (the
//!   wire's `Fields` options are not yet mapped to their typed dialogs), settle
//!   `error(unsupported-ui)`. A session lock settles `error(locked)`.
//! - A `parent_window` is honoured only when the window it names belongs to the
//!   caller's bus pid. The pid is the relay's verified value, never one from the payload,
//!   and an X11 parent is refused until the compositor can map XIDs to windows.
//!
//! # Failure modes
//!
//! - Request id already held by an open or queued request: refused `request-taken`.
//! - `portal.cancel` from a connection that holds nothing: refused `request-unknown`.
//! - `portal.key` naming a key the dialog grammar does not know: refused
//!   `unknown-key`, and nothing reaches the dialog.
//! - Admission refused by the queue or the lock: settles `error(busy|locked)` to the
//!   submitter, not a refusal, so the requester learns the verdict it waits for.

use std::collections::BTreeSet;

use gnoblin_client::wire::{self, PortalRequest, VerifiedCaller};
use gnoblin_model::RequestOutcome;
use gnoblin_portal_core::model::{Consent, Dialog, Grant, Interface, ScreenKey};
use gnoblin_portal_ui::ui::DialogScheme;
use gnoblin_proto::Envelope;
use gnoblin_render_frame::frame::Overlay;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{ClientId, KeyState, OutputId, ParentWindow, Rect, RequestId, Size, Tick};
use gnoblin_wayland::{Modal, ModalEvent, WindowProcess};

use crate::log;
use crate::portal::{CancelReason, PortalError, PortalUi, Verdict};

/// The built-in dialog's frame size, in pixels.
const FRAME_WIDTH: usize = 640;
const FRAME_HEIGHT: usize = 360;

/// The wire code for a key name the dialog grammar does not know.
const UNKNOWN_KEY: &str = "unknown-key";

/// The wire code for a payload that does not decode.
pub const MALFORMED: &str = "malformed";

/// A reply the dispatcher writes: the connection it goes to, and the frame.
pub type Reply = (ClientId, Envelope);

/// A request the service refuses before admitting it: the wire code and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRefusal {
    /// Stable wire code the client matches on, such as `REQUEST_TAKEN`.
    pub code: &'static str,
    /// Human-readable detail for logs and error dialogs; not part of the contract.
    pub message: String,
}

/// How long after a request is offered to a `portal.ui` claimant an `allow` answer is not believed.
///
/// The built-in dialog ignores input for the same 500 ms after it appears, so a click already in
/// progress cannot accept it (D19, "One gesture to allow"). The compositor cannot see the
/// claimant's dialog, so it applies the rule to the one thing it can see: an `allow` that arrives
/// sooner than a person could have read the request settles as a refusal.
const ARMING_MS: u64 = 500;

/// The wire answer a `portal.ui` claimant gives to allow the request. Anything else it sends is
/// not understood and the built-in dialog serves the request instead; `deny` refuses it.
const ANSWER_ALLOW: &[u8] = b"allow";

/// The wire answer a `portal.ui` claimant gives to refuse the request.
const ANSWER_DENY: &[u8] = b"deny";

/// A request offered to the shell that claimed `portal.ui`, kept until the claimant settles it or
/// the compositor falls back to the built-in dialog.
struct ClaimantOffer {
    /// The broker request the claimant was offered.
    offer: RequestId,
    /// The connection that submitted the request, and the only one its outcome goes to.
    client: ClientId,
    /// The envelope id the outcome is sent under.
    envelope_id: u64,
    /// The request as the relay sent it, so the built-in dialog can serve it if the claimant cannot.
    request: PortalRequest,
    /// The interface the request asked for.
    interface: Interface,
    /// When the claimant was offered the request.
    started: Tick,
}

/// The connection that submitted an admitted request, and the envelope id its
/// outcome is sent under.
struct HeldRequest {
    /// The request the client submitted, so the outcome can be routed back to it.
    request: RequestId,
    /// The connection that submitted the request; replies go only to this client.
    client: ClientId,
    /// The envelope id the outcome is sent under, so the client can pair it with its call.
    envelope_id: u64,
    /// The interface the request asked for, so a granted consent is recorded for it.
    interface: Interface,
    /// The verified app id that asked, the key a secret grant is recorded under.
    app_id: String,
}

/// The portal UI session and the routing of its requests to their owners.
pub struct PortalService {
    /// The dialog UI that shows requests to the user, one dialog at a time.
    ui: PortalUi,
    /// Admitted requests still waiting for an outcome, in admission order.
    owners: Vec<HeldRequest>,
    /// Requests offered to a `portal.ui` claimant and not yet settled.
    routed: Vec<ClaimantOffer>,
    /// Offers whose requester is gone, cancelled or locked out, for the dispatcher to end.
    abandoned: Vec<RequestId>,
    /// The output the dialog modal is shown on, while one is shown.
    shown: Option<OutputId>,
    /// Apps whose secret the user allowed this session. Held in memory only, so a new
    /// session asks again, and it is the compositor's own record, never the relay's.
    granted: BTreeSet<String>,
    /// Whether the session is locked, as the last `set_locked` said.
    locked: bool,
}

impl Default for PortalService {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalService {
    /// Builds an empty portal service with no owners and no dialog shown.
    ///
    /// The dialog frame is sized to the fixed `FRAME_WIDTH` by `FRAME_HEIGHT`, so every
    /// portal dialog has the same geometry regardless of the request that opened it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ui: PortalUi::new(FRAME_WIDTH, FRAME_HEIGHT),
            owners: Vec::new(),
            routed: Vec::new(),
            abandoned: Vec::new(),
            shown: None,
            granted: BTreeSet::new(),
            locked: false,
        }
    }

    /// Takes the dialog scheme from a committed configuration: `prefer-light` draws
    /// light dialogs, and the default and `prefer-dark` draw dark ones. A configuration
    /// that names no scheme draws dark, as the stock desktop does.
    pub fn configure(&mut self, config: &gnoblin_config::Config) {
        let scheme = match config
            .appearance
            .as_ref()
            .and_then(|appearance| appearance.color_scheme)
        {
            Some(gnoblin_config::ColorScheme::PreferLight) => DialogScheme::Light,
            Some(gnoblin_config::ColorScheme::Default | gnoblin_config::ColorScheme::PreferDark) | None => {
                DialogScheme::Dark
            }
        };
        self.ui.set_scheme(scheme);
    }

    /// `portal.ui`: admit a request from `client`, which is answered under
    /// `envelope_id`. A request the UI refuses at once replies its error now.
    pub fn submit(
        &mut self,
        client: ClientId,
        envelope_id: u64,
        request: &PortalRequest,
        now: Tick,
    ) -> Result<Vec<Reply>, PortalRefusal> {
        let id = RequestId::new(envelope_id);
        if self.owners.iter().any(|owner| owner.request == id)
            || self.routed.iter().any(|routed| routed.envelope_id == envelope_id)
        {
            return Err(PortalRefusal {
                code: wire::code::REQUEST_TAKEN,
                message: format!("portal request {envelope_id} is already held"),
            });
        }
        let Ok(interface) = Interface::from_name(&request.interface) else {
            return Ok(vec![outcome_reply(
                client,
                envelope_id,
                Verdict::Error(PortalError::UnsupportedUi),
            )]);
        };
        // A consent dialog is the only kind offered here: the UI refuses any other
        // interface, including the choosers, as `unsupported-ui`.
        let dialog = Dialog::Consent(Consent::open(
            request.caller.app_id().to_owned(),
            request.prompt.clone(),
            now,
        ));
        match self
            .ui
            .submit(id, request.caller.app_id(), interface, dialog, request.timeout_ms, now)
        {
            Ok(outcome) => {
                log::debug!(
                    area = "portal",
                    request = envelope_id,
                    interface = %request.interface,
                    ?outcome,
                    "the built-in dialog admitted a portal request"
                );
                self.owners.push(HeldRequest {
                    request: id,
                    client,
                    envelope_id,
                    interface,
                    app_id: request.caller.app_id().to_owned(),
                });
                Ok(Vec::new())
            }
            Err(error) => Ok(vec![outcome_reply(client, envelope_id, Verdict::Error(error))]),
        }
    }

    /// Whether `request` can be offered to a `portal.ui` claimant, and the part of the requester's
    /// patience the claimant gets: the interface is one the compositor knows, the envelope id is
    /// free, and the request leaves a window for the built-in dialog to serve it afterwards.
    ///
    /// The claimant gets half of the requester's timeout. The built-in dialog gets what is left,
    /// so a claimant that never answers costs the requester no more than its own timeout.
    #[must_use]
    pub fn routable(&self, envelope_id: u64, request: &PortalRequest) -> Option<(Interface, u64)> {
        let id = RequestId::new(envelope_id);
        let window = request.timeout_ms / 2;
        if window == 0
            || self.owners.iter().any(|owner| owner.request == id)
            || self.routed.iter().any(|routed| routed.envelope_id == envelope_id)
        {
            return None;
        }
        Interface::from_name(&request.interface)
            .ok()
            .map(|interface| (interface, window))
    }

    /// Record that `request` was offered to the `portal.ui` claimant as broker request `offer`.
    pub fn offered_to_claimant(
        &mut self,
        client: ClientId,
        envelope_id: u64,
        offer: RequestId,
        request: &PortalRequest,
        interface: Interface,
        now: Tick,
    ) {
        log::debug!(
            area = "portal",
            request = envelope_id,
            offer = offer.get(),
            interface = %request.interface,
            "a portal request was offered to the portal.ui claimant"
        );
        self.routed.push(ClaimantOffer {
            offer,
            client,
            envelope_id,
            request: request.clone(),
            interface,
            started: now,
        });
    }

    /// The claimant's offer `offer` ended as `outcome`: settle the requester, or fall back.
    ///
    /// Only an exact `allow` or `deny` answer settles it, and an `allow` that comes sooner than
    /// [`ARMING_MS`] after the offer settles as a refusal. Everything else, an answer the
    /// compositor does not understand, a decline, a claimant that left or a deadline that passed,
    /// hands the request to the built-in dialog, which asks the user with Deny focused. No path
    /// here turns a failure into consent.
    pub fn provider_settled(&mut self, offer: RequestId, outcome: &RequestOutcome, now: Tick) -> Vec<Reply> {
        let Some(index) = self.routed.iter().position(|routed| routed.offer == offer) else {
            // Its requester left or cancelled first, so nobody waits for it.
            return Vec::new();
        };
        let routed = self.routed.remove(index);
        log::debug!(
            area = "portal",
            request = routed.envelope_id,
            offer = offer.get(),
            waited_ms = now.since(routed.started),
            outcome = claimant_outcome(outcome),
            "the portal.ui claimant settled an offer"
        );
        let verdict = match outcome {
            RequestOutcome::Answered(answer) if answer.as_slice() == ANSWER_ALLOW => {
                if now.since(routed.started) < ARMING_MS {
                    Some(Verdict::Cancelled(CancelReason::User))
                } else {
                    Some(Verdict::Handled(Grant::Allowed))
                }
            }
            RequestOutcome::Answered(answer) if answer.as_slice() == ANSWER_DENY => {
                Some(Verdict::Cancelled(CancelReason::User))
            }
            RequestOutcome::Answered(_)
            | RequestOutcome::Ended
            | RequestOutcome::Refused(_)
            | RequestOutcome::TimedOut => None,
        };
        match verdict {
            Some(verdict) => {
                if routed.interface == Interface::Secret && verdict == Verdict::Handled(Grant::Allowed) {
                    self.granted.insert(routed.request.caller.app_id().to_owned());
                }
                vec![outcome_reply(routed.client, routed.envelope_id, verdict)]
            }
            None => self.fall_back(routed, now),
        }
    }

    /// Serve a routed request with the built-in dialog, for the time the requester has left.
    fn fall_back(&mut self, routed: ClaimantOffer, now: Tick) -> Vec<Reply> {
        let ClaimantOffer {
            client,
            envelope_id,
            mut request,
            started,
            ..
        } = routed;
        request.timeout_ms = request.timeout_ms.saturating_sub(now.since(started));
        if request.timeout_ms == 0 {
            return vec![outcome_reply(
                client,
                envelope_id,
                Verdict::Cancelled(CancelReason::Timeout),
            )];
        }
        log::warn!(
            area = "portal",
            interface = %request.interface,
            "the portal.ui claimant did not settle the request; the built-in dialog serves it"
        );
        match self.submit(client, envelope_id, &request, now) {
            Ok(replies) => replies,
            Err(_) => vec![outcome_reply(
                client,
                envelope_id,
                Verdict::Error(PortalError::ProviderFailed),
            )],
        }
    }

    /// The broker offers whose requester is gone, cancelled or locked out since the last call.
    /// The dispatcher ends them so the claimant stops drawing a dialog nobody waits for.
    pub fn take_abandoned(&mut self) -> Vec<RequestId> {
        std::mem::take(&mut self.abandoned)
    }

    /// `portal.key`: route one key to the open dialog at `now`. The clock is advanced
    /// first, because input arrives between idle ticks and the dialog's arming and
    /// animations judge the key against the time it was read, not the last tick.
    pub fn key(&mut self, name: &str, repeat: bool, now: Tick) -> Result<Vec<Reply>, PortalRefusal> {
        let Some(key) = parse_key(name) else {
            return Err(PortalRefusal {
                code: UNKNOWN_KEY,
                message: format!("`{name}` is not a key the portal dialog takes"),
            });
        };
        if self.ui.active_request().is_none() {
            // The key is dropped and its caller is acknowledged as for any key, so a key that finds no
            // dialog is otherwise silent: its dialog is not admitted yet, or already settled.
            log::debug!(area = "portal", "a portal key reached no open dialog and was dropped");
        }
        self.ui.tick(now);
        self.ui.key(key, repeat);
        Ok(self.settle())
    }

    /// `portal.cancel`: withdraw every request `client` holds. Refused when it holds
    /// none.
    pub fn cancel(&mut self, client: ClientId) -> Result<Vec<Reply>, PortalRefusal> {
        let (held, kept): (Vec<HeldRequest>, Vec<HeldRequest>) = std::mem::take(&mut self.owners)
            .into_iter()
            .partition(|owner| owner.client == client);
        self.owners = kept;
        let (sent, kept): (Vec<ClaimantOffer>, Vec<ClaimantOffer>) = std::mem::take(&mut self.routed)
            .into_iter()
            .partition(|routed| routed.client == client);
        self.routed = kept;
        if held.is_empty() && sent.is_empty() {
            return Err(PortalRefusal {
                code: wire::code::REQUEST_UNKNOWN,
                message: "this connection holds no portal request".to_owned(),
            });
        }
        let mut replies = Vec::new();
        for owner in held {
            self.ui.cancel(owner.request);
            replies.push(outcome_reply(
                owner.client,
                owner.envelope_id,
                Verdict::Cancelled(CancelReason::User),
            ));
        }
        for routed in sent {
            self.abandoned.push(routed.offer);
            replies.push(outcome_reply(
                routed.client,
                routed.envelope_id,
                Verdict::Cancelled(CancelReason::User),
            ));
        }
        replies.extend(self.settle());
        Ok(replies)
    }

    /// The connection `client` left: withdraw whatever it held. Its replies are
    /// dropped because the owner is gone.
    pub fn departed(&mut self, client: ClientId) -> Vec<Reply> {
        self.withdraw(client);
        let (sent, kept): (Vec<ClaimantOffer>, Vec<ClaimantOffer>) = std::mem::take(&mut self.routed)
            .into_iter()
            .partition(|routed| routed.client == client);
        self.routed = kept;
        self.abandoned.extend(sent.into_iter().map(|routed| routed.offer));
        self.settle()
    }

    /// The clock moved: animations advance and deadlines are enforced.
    pub fn tick(&mut self, now: Tick) -> Vec<Reply> {
        self.ui.tick(now);
        self.settle()
    }

    /// The session lock changed. Locking settles every held request `error(locked)`.
    /// `pump` feeds it from the lock state each tick.
    pub fn set_locked(&mut self, locked: bool) -> Vec<Reply> {
        self.locked = locked;
        let mut replies = Vec::new();
        if locked {
            // A lock ends every secret grant: the next request after unlock asks again.
            self.granted.clear();
            // A request a claimant is drawing a dialog for is refused as the built-in's are.
            for routed in std::mem::take(&mut self.routed) {
                self.abandoned.push(routed.offer);
                replies.push(outcome_reply(
                    routed.client,
                    routed.envelope_id,
                    Verdict::Error(PortalError::Locked),
                ));
            }
        }
        self.ui.set_locked(locked);
        replies.extend(self.settle());
        replies
    }

    /// Follow the session's lock as the observation reports it: settle what a new lock refuses, and
    /// let requests in again once it lifts. Nothing changes while the state is the one already known.
    ///
    /// The portal is told on every sweep, not only while a dialog is open: a request that arrives
    /// while the session is locked must be refused, and the first request after an unlock must not
    /// find the portal still locked.
    pub fn follow_lock(&mut self, locked: bool) -> Vec<Reply> {
        if locked == self.locked {
            return Vec::new();
        }
        self.set_locked(locked)
    }

    /// Whether `app_id` may be offered its secret: the user allowed it this session
    /// and the session is not locked. The compositor decides this, never the relay.
    #[must_use]
    pub fn secret_granted(&self, app_id: &str) -> bool {
        !self.locked && self.granted.contains(app_id)
    }

    /// Whether the dialog modal needs routing: a dialog is open, or one is still
    /// shown and must be cleared. Otherwise the compositor need not drain events.
    #[must_use]
    pub fn wants_pump(&self) -> bool {
        self.shown.is_some() || self.ui.active_request().is_some() || !self.routed.is_empty()
    }

    /// Withdraws every request `client` holds from the UI. Returns whether it held any.
    fn withdraw(&mut self, client: ClientId) -> bool {
        let held: Vec<RequestId> = self
            .owners
            .iter()
            .filter(|owner| owner.client == client)
            .map(|owner| owner.request)
            .collect();
        for request in &held {
            self.ui.cancel(*request);
            self.owners.retain(|owner| owner.request != *request);
        }
        !held.is_empty()
    }

    /// The outcomes that settled since the last call, each addressed to its owner.
    /// An outcome with no owner (withdrawn) is dropped.
    fn settle(&mut self) -> Vec<Reply> {
        let mut replies = Vec::new();
        for (request, verdict) in self.ui.take_outcomes() {
            let Some(index) = self.owners.iter().position(|owner| owner.request == request) else {
                continue;
            };
            let owner = self.owners.remove(index);
            if owner.interface == Interface::Secret && verdict == Verdict::Handled(Grant::Allowed) {
                self.granted.insert(owner.app_id.clone());
            }
            log::debug!(
                area = "portal",
                request = owner.envelope_id,
                ?verdict,
                "the built-in dialog settled a portal request"
            );
            replies.push(outcome_reply(owner.client, owner.envelope_id, verdict));
        }
        replies
    }
}

/// How a claimant's offer ended, named for the log. The answer's bytes are never logged: only
/// whether it was `allow`, `deny` or something else.
fn claimant_outcome(outcome: &RequestOutcome) -> &'static str {
    match outcome {
        RequestOutcome::Answered(answer) if answer.as_slice() == ANSWER_ALLOW => "allow",
        RequestOutcome::Answered(answer) if answer.as_slice() == ANSWER_DENY => "deny",
        RequestOutcome::Answered(_) => "an answer the compositor does not understand",
        RequestOutcome::Ended => "the claimant left or the offer ended",
        RequestOutcome::Refused(_) => "the claimant declined",
        RequestOutcome::TimedOut => "the offer timed out",
    }
}

/// The `portal.ui` reply for one verdict.
fn outcome_reply(client: ClientId, envelope_id: u64, verdict: Verdict) -> Reply {
    let envelope = match verdict {
        Verdict::Error(error) => Envelope::error(
            wire::NAMESPACE,
            wire::member::PORTAL_UI,
            envelope_id,
            error.as_str(),
            format!("the portal request settled as `{}`", error.as_str()),
        ),
        Verdict::Handled(grant) => wire::portal_outcome(envelope_id, "handled", grant_name(&grant)),
        Verdict::Cancelled(reason) => wire::portal_outcome(envelope_id, "cancelled", reason.as_str()),
        Verdict::Declined => wire::portal_outcome(envelope_id, "declined", ""),
    };
    (client, envelope)
}

/// The name a grant settles under. A consent grant is `allowed`.
const fn grant_name(grant: &Grant) -> &'static str {
    match grant {
        Grant::Allowed => "allowed",
        Grant::Files { .. } => "files",
        Grant::App { .. } => "app",
        Grant::Print { .. } => "print",
        Grant::Capture { .. } => "capture",
        Grant::Devices { .. } => "devices",
    }
}

/// Whether the window a request names as its `parent_window` belongs to the caller.
///
/// The caller is the relay's verified value: its pid came from the bus, never from the
/// request payload, so a payload that claims another pid is still checked against the bus
/// pid. `owner` is the process the compositor found for the named window. A pid of `0`
/// means the bus could not name the caller, and no process has that id, so nothing
/// matches it. A window with no process the compositor can name, or whose client has gone,
/// is never proof of ownership, so it is refused.
pub(super) fn parent_owner_check(caller: &VerifiedCaller, owner: &WindowProcess) -> Result<(), &'static str> {
    match owner {
        WindowProcess::Connected(identity) if caller.pid() != 0 && identity.pid == caller.pid() => Ok(()),
        WindowProcess::Connected(_) => Err("the parent window belongs to another process"),
        WindowProcess::Unknown => Err("the parent window's process is unknown"),
        WindowProcess::Disconnected => Err("the parent window's client has disconnected"),
    }
}

/// Where a portal dialog opens. Every request opens on the output: no path places a dialog
/// on its parent window, because no trusted app-to-process binding exists yet (design
/// `layers.md`, Needs Kieran 16, decided as output scope).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PortalScope {
    /// The dialog opens on the output. `reason` says what was found about the parent, or
    /// why it was not used; it is empty when the request named no parent at all.
    Output { reason: String },
}

/// Resolves a request's `parent_window` to the scope its dialog opens on.
///
/// The result is always an output scope. The parent is resolved only so the log line can
/// say what was found: a parent whose process matches the caller is reported as resolved
/// but not established, because a pid match is not a trusted binding (design `layers.md`,
/// Needs Kieran 16). It is a pure function over two lookups: `exported` maps a Wayland
/// handle to its window (`None` while the compositor has not answered, `Some(None)` for an
/// unknown or revoked handle), and `process` maps a window to its client process. Nothing
/// here refuses a request, so the app still gets its consent answer.
///
/// An X11 parent is never proven, because the compositor cannot yet map an XID to a
/// process. A malformed string is an output scope with the parse error as the reason.
pub(super) fn resolve_portal_scope<W>(
    caller: &VerifiedCaller,
    parent: &str,
    exported: impl FnOnce(&str) -> Option<Option<W>>,
    process: impl FnOnce(W) -> Option<Option<WindowProcess>>,
) -> PortalScope {
    let output = |reason: &str| PortalScope::Output {
        reason: reason.to_owned(),
    };
    let handle = match ParentWindow::parse(parent) {
        Ok(ParentWindow::None) => return output(""),
        Ok(ParentWindow::X11(_)) => return output("an X11 parent window cannot be checked yet"),
        Ok(ParentWindow::Wayland(handle)) => handle,
        Err(error) => return output(&error.to_string()),
    };
    let window = match exported(&handle) {
        None => return output("the compositor did not resolve the parent window in time"),
        Some(None) => return output("the parent window handle is unknown or revoked"),
        Some(Some(window)) => window,
    };
    let owner = match process(window) {
        None => return output("the compositor did not report the parent window's process in time"),
        Some(None) => return output("the parent window no longer exists"),
        Some(Some(owner)) => owner,
    };
    match parent_owner_check(caller, &owner) {
        Ok(()) => output("the parent window resolved to the caller's process, but ownership is not established"),
        Err(reason) => output(reason),
    }
}

/// The key a `portal.key` name stands for: `escape`, `enter`, `tab`, `up`, `down`,
/// `space`, `backspace`, or `text:` and one character.
fn parse_key(name: &str) -> Option<ScreenKey> {
    Some(match name {
        "escape" => ScreenKey::Escape,
        "enter" => ScreenKey::Enter,
        "tab" => ScreenKey::Tab,
        "up" => ScreenKey::Up,
        "down" => ScreenKey::Down,
        "space" => ScreenKey::Space,
        "backspace" => ScreenKey::Backspace,
        _ => {
            let mut chars = name.strip_prefix("text:")?.chars();
            let character = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            ScreenKey::Text(character)
        }
    })
}

/// The dialog key an evdev keycode stands for, or `None` for a key the dialog does
/// not use. Shift is not a dialog key, and Shift+Tab reaches the dialog as Tab: the
/// consent grammar has no reverse focus step.
#[must_use]
pub fn screen_key(keycode: u32) -> Option<ScreenKey> {
    Some(match keycode {
        1 => ScreenKey::Escape,
        14 => ScreenKey::Backspace,
        15 => ScreenKey::Tab,
        28 | 96 => ScreenKey::Enter,
        57 => ScreenKey::Space,
        103 => ScreenKey::Up,
        108 => ScreenKey::Down,
        _ => return None,
    })
}

/// The evdev code of the left mouse button. Only it answers the dialog.
const BTN_LEFT: u32 = 0x110;

/// One axis of a centred placement: a frame of `frame` pixels at the middle of
/// `extent`, starting at `origin`. A frame larger than the extent sits at `origin`.
fn centred(origin: i32, extent: u32, frame: u32) -> i32 {
    let offset = extent.saturating_sub(frame) / 2;
    origin.saturating_add(i32::try_from(offset).unwrap_or(i32::MAX))
}

/// Where the dialog frame sits on an output: centred, and never past the output's
/// top left corner.
#[must_use]
pub fn dialog_placement(output: Rect, frame_width: u32, frame_height: u32) -> Rect {
    Rect::new(
        centred(output.x, output.width, frame_width),
        centred(output.y, output.height, frame_height),
        frame_width,
        frame_height,
    )
}

/// The frame pixel under a layout point, or `None` when the point lies outside the
/// placed dialog. The result is in `0..frame` on each axis.
#[must_use]
pub fn frame_point(placement: Rect, x: f64, y: f64) -> Option<(f32, f32)> {
    let left = f64::from(placement.x);
    let top = f64::from(placement.y);
    let right = left + f64::from(placement.width);
    let bottom = top + f64::from(placement.height);
    if x < left || y < top || x >= right || y >= bottom {
        return None;
    }
    Some(((x - left) as f32, (y - top) as f32))
}

/// What one pump changes on the compositor.
#[derive(Default)]
pub struct PortalScene {
    /// `Some(Some(modal))` shows the dialog, `Some(None)` clears it, and `None`
    /// leaves the modal as it is.
    pub modal: Option<Option<Modal>>,
    /// Replies the dispatcher writes.
    pub replies: Vec<Reply>,
}

impl PortalService {
    /// Routes the modal events the compositor queued and says what the modal must
    /// show now. The dialog is drawn centred on `output` while it is open. Keys and
    /// left clicks go to the dialog; a click outside it is swallowed. The caller ticks
    /// the clock first, so a deadline that passed has already settled.
    pub fn pump(&mut self, output: Option<(OutputId, Rect)>, locked: bool, events: &[ModalEvent]) -> PortalScene {
        if locked {
            let replies = self.set_locked(true);
            return PortalScene {
                modal: self.clear_modal(),
                replies,
            };
        }
        // A dialog never outgrows the output it sits on, so the output's size is the
        // bound the dialog fits within. It is set before placement so this frame's
        // pointer routing uses the same bounds the dialog was last drawn within.
        if let Some((_, rect)) = output {
            let width = usize::try_from(rect.width).unwrap_or(usize::MAX);
            let height = usize::try_from(rect.height).unwrap_or(usize::MAX);
            self.ui.set_bounds(width, height);
        }
        let placement = self.placement(output);
        for event in events {
            self.route(event, placement);
        }
        let replies = self.settle();
        let modal = match output {
            Some((output_id, rect)) if placement.is_some() => self.frame_modal(output_id, rect),
            _ => self.clear_modal(),
        };
        PortalScene { modal, replies }
    }

    /// The rectangle the open dialog's frame sits at on `output`, or `None` when no
    /// dialog is open.
    fn placement(&self, output: Option<(OutputId, Rect)>) -> Option<Rect> {
        let (_, rect) = output?;
        self.ui.active_request()?;
        let (width, height) = self.ui.size();
        let width = u32::try_from(width).ok()?;
        let height = u32::try_from(height).ok()?;
        Some(dialog_placement(rect, width, height))
    }

    fn route(&mut self, event: &ModalEvent, placement: Option<Rect>) {
        match *event {
            ModalEvent::Key { keycode, state } => {
                if state == KeyState::Pressed
                    && let Some(key) = screen_key(keycode)
                {
                    self.ui.key(key, false);
                }
            }
            ModalEvent::PointerButton { x, y, button, state } => {
                if button != BTN_LEFT {
                    return;
                }
                let Some(placement) = placement else {
                    return;
                };
                let Some((fx, fy)) = frame_point(placement, x, y) else {
                    return;
                };
                match state {
                    KeyState::Pressed => self.ui.pointer_press(fx, fy),
                    KeyState::Released => self.ui.pointer_release(fx, fy),
                }
            }
        }
    }

    /// Takes the frame the dialog just drew, if its pixels changed, and wraps it as the
    /// modal. `None` when nothing changed, so the modal stays as it is.
    fn frame_modal(&mut self, output: OutputId, rect: Rect) -> Option<Option<Modal>> {
        let frame = self.ui.take_frame()?;
        let width = u32::try_from(frame.width()).ok()?;
        let height = u32::try_from(frame.height()).ok()?;
        // The frame is placed from its own size, not the size the pointer was routed
        // with: a settled or resized dialog may have drawn at a new size in this take.
        let placement = dialog_placement(rect, width, height);
        let image = Image::new(Size::new(width, height), frame.width() * 4, frame.bytes().to_vec())?;
        let overlay = Overlay::new(placement, &image)?.with_secure(true);
        self.shown = Some(output);
        Some(Some(Modal::new(output, overlay)))
    }

    /// Clears the modal once it was shown, and only then.
    fn clear_modal(&mut self) -> Option<Option<Modal>> {
        self.shown.take().map(|_| None)
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_model::Fields;

    use gnoblin_wayland::ProcessIdentity;

    use super::*;

    fn screenshot() -> PortalRequest {
        PortalRequest {
            interface: "Screenshot".to_owned(),
            caller: VerifiedCaller::new(0, "org.example.Requester"),
            prompt: "Share the screen?".to_owned(),
            timeout_ms: 30_000,
            options: Fields::default(),
            parent_window: String::new(),
        }
    }

    #[test]
    fn locking_settles_the_open_request_as_locked() {
        let mut service = PortalService::new();
        let requester = ClientId::new(1);
        let opened = service.submit(requester, 1, &screenshot(), Tick::ZERO);
        assert!(
            opened.is_ok_and(|replies| replies.is_empty()),
            "the dialog is admitted before the lock"
        );

        let replies = service.set_locked(true);
        assert_eq!(replies.len(), 1, "the held request is settled once");
        assert_eq!(replies[0].0, requester, "the reply goes to the requester");
        assert_eq!(replies[0].1.error_code(), Some("locked"));

        assert!(service.set_locked(false).is_empty(), "unlocking settles nothing");
    }

    /// A secret consent request from `app_id`, as the relay submits it.
    fn secret_from(app_id: &str) -> PortalRequest {
        PortalRequest {
            interface: "Secret".to_owned(),
            caller: VerifiedCaller::new(0, app_id),
            prompt: "Give this app its secret?".to_owned(),
            timeout_ms: 30_000,
            options: Fields::default(),
            parent_window: String::new(),
        }
    }

    /// Submits `request` from client 1 at tick 0, then feeds `keys` once the dialog has armed.
    fn answer(service: &mut PortalService, request: &PortalRequest, keys: &[&str]) -> Vec<Reply> {
        service
            .submit(ClientId::new(1), 7, request, Tick::ZERO)
            .expect("the consent is admitted");
        let mut replies = Vec::new();
        for &key in keys {
            replies.extend(service.key(key, false, Tick::new(600)).expect("a dialog key"));
        }
        replies
    }

    #[test]
    fn an_allowed_secret_consent_grants_that_app() {
        let mut service = PortalService::new();
        let replies = answer(&mut service, &secret_from("org.example.Keys"), &["tab", "enter"]);
        assert_eq!(replies.len(), 1, "the consent is settled once");
        assert!(service.secret_granted("org.example.Keys"));
        assert!(
            !service.secret_granted("org.example.Other"),
            "a grant is for the app asked"
        );
    }

    #[test]
    fn a_denied_secret_consent_grants_nothing() {
        let mut service = PortalService::new();
        answer(&mut service, &secret_from("org.example.Keys"), &["escape"]);
        assert!(!service.secret_granted("org.example.Keys"));
    }

    #[test]
    fn a_screenshot_grant_is_not_a_secret_grant() {
        let mut service = PortalService::new();
        answer(&mut service, &screenshot(), &["tab", "enter"]);
        assert!(!service.secret_granted("org.example.Requester"));
    }

    #[test]
    fn a_lock_ends_every_secret_grant() {
        let mut service = PortalService::new();
        answer(&mut service, &secret_from("org.example.Keys"), &["tab", "enter"]);
        assert!(service.secret_granted("org.example.Keys"));
        service.set_locked(true);
        assert!(!service.secret_granted("org.example.Keys"), "a locked session refuses");
        service.set_locked(false);
        assert!(
            !service.secret_granted("org.example.Keys"),
            "the grant does not survive a lock"
        );
    }

    #[test]
    fn keycodes_map_to_dialog_keys() {
        assert_eq!(screen_key(1), Some(ScreenKey::Escape));
        assert_eq!(screen_key(15), Some(ScreenKey::Tab));
        assert_eq!(screen_key(28), Some(ScreenKey::Enter));
        assert_eq!(screen_key(96), Some(ScreenKey::Enter), "keypad enter answers too");
        assert_eq!(screen_key(57), Some(ScreenKey::Space));
        assert_eq!(screen_key(103), Some(ScreenKey::Up));
        assert_eq!(screen_key(108), Some(ScreenKey::Down));
        assert_eq!(screen_key(14), Some(ScreenKey::Backspace));
        assert_eq!(screen_key(42), None, "shift is not a dialog key");
        assert_eq!(screen_key(30), None, "letters are not dialog keys");
    }

    #[test]
    fn the_frame_is_centred_on_the_output() {
        let output = Rect::new(100, 50, 1920, 1080);
        assert_eq!(dialog_placement(output, 640, 360), Rect::new(740, 410, 640, 360));
    }

    #[test]
    fn a_frame_larger_than_the_output_sits_at_its_origin() {
        let output = Rect::new(0, 0, 320, 200);
        assert_eq!(dialog_placement(output, 640, 360), Rect::new(0, 0, 640, 360));
    }

    #[test]
    fn pointer_maps_into_the_frame_and_misses_outside() {
        let placement = Rect::new(740, 410, 640, 360);
        assert_eq!(frame_point(placement, 740.0, 410.0), Some((0.0, 0.0)));
        assert_eq!(frame_point(placement, 1000.5, 600.0), Some((260.5, 190.0)));
        assert_eq!(frame_point(placement, 1380.0, 600.0), None, "right edge is exclusive");
        assert_eq!(frame_point(placement, 739.9, 500.0), None);
        assert_eq!(frame_point(placement, 900.0, 770.0), None, "bottom edge is exclusive");
    }

    #[test]
    fn no_dialog_leaves_the_modal_alone() {
        let mut service = PortalService::new();
        let scene = service.pump(Some((OutputId::new(1), Rect::new(0, 0, 1920, 1080))), false, &[]);
        assert!(scene.modal.is_none(), "nothing to show and nothing to clear");
        assert!(scene.replies.is_empty());
    }

    #[test]
    fn a_lock_clears_the_shown_modal_and_settles_the_request() {
        let mut service = PortalService::new();
        let requester = ClientId::new(1);
        let output = OutputId::new(1);
        let rect = Rect::new(0, 0, 1920, 1080);
        assert!(service.submit(requester, 1, &screenshot(), Tick::ZERO).is_ok());
        let shown = service.pump(Some((output, rect)), false, &[]);
        assert!(
            matches!(shown.modal, Some(Some(_))),
            "the dialog is drawn once it is open"
        );

        let locked = service.pump(Some((output, rect)), true, &[]);
        assert!(matches!(locked.modal, Some(None)), "the lock clears the modal");
        assert_eq!(locked.replies.len(), 1);
        assert_eq!(locked.replies[0].1.error_code(), Some("locked"));
    }

    /// A process identity for a connected client, with a fixed start time.
    fn process(pid: u32) -> WindowProcess {
        WindowProcess::Connected(ProcessIdentity { pid, start_time: 7 })
    }

    #[test]
    fn the_owner_check_accepts_a_window_owned_by_the_bus_pid() {
        let caller = VerifiedCaller::new(1000, "org.example.Requester");
        assert_eq!(parent_owner_check(&caller, &process(1000)), Ok(()));
    }

    #[test]
    fn a_payload_claiming_another_pid_is_checked_against_the_bus_pid() {
        // The relay took pid 1000 from the bus. A payload that names a window owned by
        // pid 4242 claims that pid as its own, but the check reads only the bus pid, so
        // the window is refused: it belongs to another process than the caller.
        let caller = VerifiedCaller::new(1000, "org.example.Requester");
        assert_eq!(
            parent_owner_check(&caller, &process(4242)),
            Err("the parent window belongs to another process"),
        );
    }

    #[test]
    fn an_unknown_bus_pid_never_matches_a_window() {
        // Pid 0 means the bus could not name the caller, so no window is proven theirs.
        let caller = VerifiedCaller::new(0, "");
        assert!(parent_owner_check(&caller, &process(0)).is_err());
        assert!(parent_owner_check(&caller, &process(1000)).is_err());
    }

    #[test]
    fn a_window_with_no_nameable_or_live_process_is_refused() {
        let caller = VerifiedCaller::new(1000, "org.example.Requester");
        assert!(parent_owner_check(&caller, &WindowProcess::Unknown).is_err());
        assert!(parent_owner_check(&caller, &WindowProcess::Disconnected).is_err());
    }

    #[test]
    fn the_relay_frame_carries_the_bus_pid_the_check_reads() {
        // The relay writes the bus pid into the frame; decoding returns that same value as
        // the caller, so the check and the relay agree on one number.
        let request = PortalRequest {
            interface: "Screenshot".to_owned(),
            caller: VerifiedCaller::new(1000, "org.example.Requester"),
            prompt: "Share the screen?".to_owned(),
            timeout_ms: 30_000,
            options: Fields::default(),
            parent_window: "wayland:abc".to_owned(),
        };
        let frame = gnoblin_client::wire::portal_ui(1, &request);
        let payload = frame.payload().expect("the request carries a payload");
        let decoded = gnoblin_client::wire::read_portal_ui(payload).expect("decodes");
        assert_eq!(decoded.caller, request.caller);
        assert_eq!(decoded.caller.pid(), 1000);
    }

    /// A lookup that resolves the handle `live` (the part after `wayland:`) to window `7`, owned by `owner`.
    fn resolve(parent: &str, owner: Option<WindowProcess>, caller_pid: u32) -> PortalScope {
        resolve_portal_scope(
            &VerifiedCaller::new(caller_pid, "org.example.Requester"),
            parent,
            |handle| (handle == "live").then_some(Some(7u32)),
            |window| (window == 7).then_some(owner),
        )
    }

    #[test]
    fn a_matching_wayland_parent_still_opens_on_the_output() {
        // The pid matches the caller, but a pid match is not a trusted binding, so the
        // dialog stays on the output and the reason says the ownership is not established.
        assert_eq!(
            resolve("wayland:live", Some(process(1000)), 1000),
            PortalScope::Output {
                reason: "the parent window resolved to the caller's process, but ownership is not established"
                    .to_owned(),
            }
        );
    }

    #[test]
    fn no_parent_opens_on_the_output_with_no_reason() {
        assert_eq!(resolve("", None, 1000), PortalScope::Output { reason: String::new() });
    }

    #[test]
    fn a_malformed_parent_opens_on_the_output() {
        let scope = resolve("not a parent", None, 1000);
        assert!(matches!(scope, PortalScope::Output { ref reason } if !reason.is_empty()));
    }

    #[test]
    fn an_unknown_or_revoked_handle_opens_on_the_output() {
        let scope = resolve_portal_scope(
            &VerifiedCaller::new(1000, "org.example.Requester"),
            "wayland:gone",
            |_: &str| Some(None::<u32>),
            |_: u32| Some(Some(process(1000))),
        );
        assert_eq!(
            scope,
            PortalScope::Output {
                reason: "the parent window handle is unknown or revoked".to_owned()
            }
        );
    }

    #[test]
    fn an_unanswered_lookup_opens_on_the_output() {
        let scope = resolve_portal_scope(
            &VerifiedCaller::new(1000, "org.example.Requester"),
            "wayland:live",
            |_: &str| None::<Option<u32>>,
            |_: u32| Some(Some(process(1000))),
        );
        assert!(matches!(scope, PortalScope::Output { .. }));
    }

    #[test]
    fn a_window_owned_by_another_pid_opens_on_the_output() {
        assert!(matches!(
            resolve("wayland:live", Some(process(4242)), 1000),
            PortalScope::Output { .. }
        ));
    }

    #[test]
    fn an_x11_parent_is_never_proven() {
        assert_eq!(
            resolve("x11:42", Some(process(1000)), 1000),
            PortalScope::Output {
                reason: "an X11 parent window cannot be checked yet".to_owned()
            }
        );
    }

    /// The broker request a test offers to the claimant.
    const OFFER: u64 = 40;

    /// A service that has offered `request` (envelope id 7, from client 1) to a claimant at tick 0.
    fn offered(request: &PortalRequest) -> PortalService {
        let mut service = PortalService::new();
        let (interface, _) = service.routable(7, request).expect("the request can be routed");
        service.offered_to_claimant(
            ClientId::new(1),
            7,
            RequestId::new(OFFER),
            request,
            interface,
            Tick::ZERO,
        );
        service
    }

    /// The `(kind, detail)` a reply carries, or `("error", code)` for an error reply.
    fn settled(replies: &[Reply]) -> (String, String) {
        assert_eq!(replies.len(), 1, "exactly one reply settles the request");
        assert_eq!(replies[0].0, ClientId::new(1), "the reply goes to the requester");
        match replies[0].1.error_code() {
            Some(code) => ("error".to_owned(), code.to_owned()),
            None => wire::read_portal_outcome(replies[0].1.payload().unwrap_or(&[])).expect("a portal outcome"),
        }
    }

    #[test]
    fn the_claimant_gets_half_the_timeout_and_the_built_in_the_rest() {
        let service = PortalService::new();
        let (_, window) = service.routable(7, &screenshot()).expect("routable");
        assert_eq!(window, 15_000, "half of 30 s");
        let mut short = screenshot();
        short.timeout_ms = 1;
        assert!(
            service.routable(7, &short).is_none(),
            "a timeout too short to leave the built-in dialog a window is not routed"
        );
        let mut unknown = screenshot();
        unknown.interface = "NotAnInterface".to_owned();
        assert!(
            service.routable(7, &unknown).is_none(),
            "an interface the compositor does not know is the built-in path's refusal"
        );
    }

    #[test]
    fn an_allow_after_the_arming_window_grants() {
        let mut service = offered(&screenshot());
        let replies = service.provider_settled(
            RequestId::new(OFFER),
            &RequestOutcome::answered(b"allow".to_vec()),
            Tick::new(ARMING_MS),
        );
        assert_eq!(settled(&replies), ("handled".to_owned(), "allowed".to_owned()));
    }

    #[test]
    fn an_allow_before_the_arming_window_is_a_refusal() {
        // The failure this prevents: a click already in progress, or a script, accepts consent
        // before a person could have read it. The built-in dialog ignores input for the same time.
        let mut service = offered(&screenshot());
        let replies = service.provider_settled(
            RequestId::new(OFFER),
            &RequestOutcome::answered(b"allow".to_vec()),
            Tick::new(ARMING_MS - 1),
        );
        assert_eq!(settled(&replies), ("cancelled".to_owned(), "user".to_owned()));
    }

    #[test]
    fn a_deny_refuses() {
        let mut service = offered(&screenshot());
        let replies = service.provider_settled(
            RequestId::new(OFFER),
            &RequestOutcome::answered(b"deny".to_vec()),
            Tick::new(5_000),
        );
        assert_eq!(settled(&replies), ("cancelled".to_owned(), "user".to_owned()));
    }

    #[test]
    fn a_deny_is_terminal_and_the_built_in_dialog_never_opens() {
        // The failure this prevents: an explicit refusal from the claimant is treated like a
        // claimant that failed, so the built-in dialog opens and asks again until the user
        // gives in. `deny` is the claimant's decision and the request ends with it. Only a
        // decline, an answer the compositor cannot read, a missed deadline and a disconnect hand
        // the request on (`a_claimant_that_cannot_answer_hands_the_request_to_the_built_in_dialog`).
        let mut service = offered(&screenshot());
        let replies = service.provider_settled(
            RequestId::new(OFFER),
            &RequestOutcome::answered(b"deny".to_vec()),
            Tick::new(5_000),
        );
        assert_eq!(settled(&replies), ("cancelled".to_owned(), "user".to_owned()));
        assert!(!service.wants_pump(), "no built-in dialog opens after a deny");
        let keys = service.key("enter", false, Tick::new(6_000)).expect("a dialog key");
        assert!(keys.is_empty(), "a key finds no dialog to settle, so none is open");
    }

    #[test]
    fn an_early_allow_is_terminal_and_is_not_asked_again() {
        // An `allow` inside the arming window counts as a refusal and ends the request. The
        // compositor does not re-prompt: asking again would turn a click already in progress
        // into a second chance to consent.
        let mut service = offered(&screenshot());
        let replies = service.provider_settled(
            RequestId::new(OFFER),
            &RequestOutcome::answered(b"allow".to_vec()),
            Tick::new(ARMING_MS - 1),
        );
        assert_eq!(settled(&replies), ("cancelled".to_owned(), "user".to_owned()));
        assert!(!service.wants_pump(), "no built-in dialog opens after an early allow");
        let keys = service
            .key("enter", false, Tick::new(ARMING_MS + 1))
            .expect("a dialog key");
        assert!(keys.is_empty(), "and no dialog is left open to settle");
        // The grant is not recorded either: the request was refused.
        assert!(!service.secret_granted("org.example.Requester"));
    }

    #[test]
    fn an_allowed_secret_through_the_claimant_grants_that_app() {
        let mut service = offered(&secret_from("org.example.Requester"));
        assert!(!service.secret_granted("org.example.Requester"));
        let replies = service.provider_settled(
            RequestId::new(OFFER),
            &RequestOutcome::answered(b"allow".to_vec()),
            Tick::new(2_000),
        );
        assert_eq!(settled(&replies), ("handled".to_owned(), "allowed".to_owned()));
        assert!(
            service.secret_granted("org.example.Requester"),
            "the compositor records the grant, whoever drew the dialog"
        );
    }

    #[test]
    fn an_answer_that_is_neither_allow_nor_deny_is_never_consent() {
        // Anything the compositor does not understand, however close to `allow`, hands the request
        // to the built-in dialog, which opens with Deny focused. It settles nothing by itself.
        for answer in [&b"Allow"[..], b"allow\n", b"yes", b"", b"allowed"] {
            let mut service = offered(&screenshot());
            let replies = service.provider_settled(
                RequestId::new(OFFER),
                &RequestOutcome::answered(answer.to_vec()),
                Tick::new(2_000),
            );
            assert!(replies.is_empty(), "{answer:?} settles nothing");
            assert!(
                service.wants_pump(),
                "{answer:?}: the built-in dialog opened for the request"
            );
        }
    }

    #[test]
    fn a_claimant_that_cannot_answer_hands_the_request_to_the_built_in_dialog() {
        let failures = [
            RequestOutcome::Refused(gnoblin_model::DeclineReason::Unavailable),
            RequestOutcome::Ended,
            RequestOutcome::TimedOut,
        ];
        for failure in failures {
            let mut service = offered(&screenshot());
            let replies = service.provider_settled(RequestId::new(OFFER), &failure, Tick::new(15_000));
            assert!(replies.is_empty(), "{failure:?} settles nothing by itself");
            assert!(
                service.wants_pump(),
                "{failure:?}: the built-in dialog serves the request"
            );
            // The built-in dialog never auto-accepts: Enter on the default focus refuses.
            let replies = service.key("enter", false, Tick::new(16_000)).expect("a dialog key");
            assert_eq!(settled(&replies), ("cancelled".to_owned(), "user".to_owned()));
        }
    }

    #[test]
    fn the_built_in_dialog_gets_only_the_time_the_claimant_left() {
        let mut service = offered(&screenshot());
        // The claimant held the request for the whole of its window, so the built-in dialog serves
        // the remaining 15 s and then cancels it as timed out.
        let replies = service.provider_settled(RequestId::new(OFFER), &RequestOutcome::TimedOut, Tick::new(15_000));
        assert!(replies.is_empty());
        let replies = service.tick(Tick::new(15_000 + 15_000 + 1));
        assert_eq!(settled(&replies), ("cancelled".to_owned(), "timeout".to_owned()));
    }

    #[test]
    fn a_claimant_that_used_the_whole_timeout_leaves_nothing_to_serve() {
        let mut service = offered(&screenshot());
        let replies = service.provider_settled(RequestId::new(OFFER), &RequestOutcome::Ended, Tick::new(30_000));
        assert_eq!(settled(&replies), ("cancelled".to_owned(), "timeout".to_owned()));
        assert!(
            !service.wants_pump(),
            "no dialog opens for a request that has no time left"
        );
    }

    #[test]
    fn the_requester_cancelling_ends_the_offer_and_a_late_answer_does_nothing() {
        let mut service = offered(&screenshot());
        let replies = service
            .cancel(ClientId::new(1))
            .expect("the connection holds a request");
        assert_eq!(settled(&replies), ("cancelled".to_owned(), "user".to_owned()));
        assert_eq!(service.take_abandoned(), vec![RequestId::new(OFFER)]);
        let late = service.provider_settled(
            RequestId::new(OFFER),
            &RequestOutcome::answered(b"allow".to_vec()),
            Tick::new(2_000),
        );
        assert!(late.is_empty(), "nobody waits for the answer");
        assert!(!service.wants_pump(), "and no dialog opens for it");
    }

    #[test]
    fn a_requester_that_leaves_ends_the_offer_and_opens_no_dialog() {
        let mut service = offered(&screenshot());
        assert!(
            service.departed(ClientId::new(1)).is_empty(),
            "no reply: its owner is gone"
        );
        assert_eq!(service.take_abandoned(), vec![RequestId::new(OFFER)]);
        // The broker reports the abandoned offer as ended; that must not fall back to the dialog.
        let replies = service.provider_settled(RequestId::new(OFFER), &RequestOutcome::Ended, Tick::new(100));
        assert!(replies.is_empty());
        assert!(
            !service.wants_pump(),
            "a request whose requester left never opens a dialog"
        );
    }

    #[test]
    fn locking_refuses_a_request_the_claimant_is_drawing() {
        let mut service = offered(&screenshot());
        let replies = service.set_locked(true);
        assert_eq!(settled(&replies), ("error".to_owned(), "locked".to_owned()));
        assert_eq!(service.take_abandoned(), vec![RequestId::new(OFFER)]);
    }

    #[test]
    fn a_routed_envelope_id_cannot_be_reused() {
        let mut service = offered(&screenshot());
        assert!(service.routable(7, &screenshot()).is_none(), "the id is held");
        let refused = service
            .submit(ClientId::new(1), 7, &screenshot(), Tick::ZERO)
            .expect_err("the built-in path refuses it too");
        assert_eq!(refused.code, wire::code::REQUEST_TAKEN);
    }

    #[test]
    fn the_portal_follows_the_lock_and_serves_again_after_it_lifts() {
        // The failure this prevents: the portal only learned of a lock while a dialog was open and
        // never learned the unlock, so a request after the first lock was refused for good, and a
        // request that arrived while locked with no dialog open was not refused at all.
        let mut service = PortalService::new();
        assert!(service.follow_lock(true).is_empty(), "nothing is held to refuse");
        let refused = service
            .submit(ClientId::new(1), 7, &screenshot(), Tick::ZERO)
            .expect("the refusal is a reply, not a refusal of the frame");
        assert_eq!(settled(&refused), ("error".to_owned(), "locked".to_owned()));
        assert!(
            service.follow_lock(true).is_empty(),
            "a lock already known changes nothing"
        );
        assert!(service.follow_lock(false).is_empty());
        let admitted = service
            .submit(ClientId::new(1), 8, &screenshot(), Tick::new(10))
            .expect("the request is admitted once the lock lifts");
        assert!(admitted.is_empty(), "the dialog opened instead of refusing");
        assert!(service.wants_pump());
    }
}
