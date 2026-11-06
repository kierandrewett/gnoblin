//! XML protocol `xdg_activation_v1` (`<protocol name="xdg_activation_v1">`); interfaces served: `xdg_activation_v1`.
//!
//! `xdg_activation_v1`: who may raise a window.
//!
//! The plan's rule is "Activation needs a valid token or a trusted context. A process
//! ID or app ID alone never grants focus" (`design/rust-compositor.md`, "Safety
//! model"). The token machine is `gnoblin-authority`'s: a token is minted once, earned
//! by a trusted context, redeemed at most once, and refused when it was never issued,
//! already used or has expired. What this module adds is the protocol half — what
//! counts as asking for a token, and how a refusal becomes observable.
//!
//! ## What earns a token
//!
//! Two things, which are the two the plan allows:
//!
//! 1. **A trusted input context.** The client names a serial that this session handed
//!    it on an input event, which is the protocol's own way of saying "the user just
//!    did something here" — a click on a link, a key that opened a window. Only
//!    serials this session issued are accepted, so a client cannot invent one.
//! 2. **A privileged or trusted connection.** A shell asks for a token as part of
//!    launching something, and the shell is the session's own process.
//!
//! Anything else is refused, and a refused mint leaves the token nonexistent, so a
//! later request is refused as unknown rather than honoured.
//!
//! ## Known gap: two refusals are silent, not observable
//!
//! Smithay's `xdg_activation_v1` dispatch consults its own `known_tokens` **before** it
//! calls [`smithay::wayland::xdg_activation::XdgActivationHandler::request_activation`],
//! and its `token_created` still answers `done` with a token string when the handler
//! returned `false`. Two consequences, both deliberate to record rather than paper over:
//!
//! - **A fabricated token never reaches this compositor.** The request is dropped inside
//!   Smithay, so it activates nothing and records nothing: there is no
//!   `activation-denied` observation for it, because no decision was taken. The test
//!   `a_fabricated_token_is_refused` asserts exactly that — focus does not move and no
//!   denial appears.
//! - **A token minted without an earned reason still reaches the client** as a string,
//!   but is not kept, so redeeming it also activates nothing and records nothing:
//!   `a_token_needs_a_trusted_context` and `an_app_id_alone_does_not_activate` assert
//!   the absence of focus and of a denial.
//!
//! Only a token this session minted *and kept* can be refused observably, which is the
//! `reused-token` case. Making the other two observable would mean the compositor
//! owning the `xdg_activation_v1` objects instead of delegating them to Smithay; that is
//! a change to file here, not a test to loosen.
//!
//! A token is also bound to the *session*, not to a process. `xdg-activation`'s token is
//! a bearer capability — a launcher hands it to a program it is about to start — so the
//! authority machine's session-leader rule stays at zero until the launch-attribution
//! slice supplies the launched process's leader.
//!
//! ## Failure modes first
//!
//! Each is a test in `tests/activation.rs`.
//!
//! 1. **A token is used twice.** The second request is refused as `reused-token` and
//!    focus does not move: `a_token_cannot_be_redeemed_twice`.
//! 2. **A client invents a token.** A token that was never issued activates nothing,
//!    and — see the known gap above — produces no denial:
//!    `a_fabricated_token_is_refused`.
//! 3. **A client names an app id instead of a token.** An app id is a label a process
//!    chooses, so a request that presents one as its authority buys exactly what an
//!    invented token buys: nothing (`an_app_id_alone_does_not_activate`).
//! 4. **A token without a trusted context is handed out freely.** A client that names a
//!    serial this session never issued, and is not one of the session's own processes,
//!    is not kept as a token holder: `a_token_needs_a_trusted_context`.
//! 5. **A refusal is invisible.** A refusal this module *takes* records
//!    [`crate::observation::ActivationDenial`] with the authority's own reason and asks
//!    the window manager for `ActivationDenied`, so a shell can say *why* a window did
//!    not come up: `a_token_cannot_be_redeemed_twice`.

use std::collections::{BTreeMap, VecDeque};

use gnoblin_log::{debug, trace};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};

use gnoblin_authority::{AppId, Authority, Claimant, Event, LaunchOrigin, SessionLeader, SurfaceId, TokenId};
use gnoblin_types::{ClientId as ConnectionId, Tick, WindowId};
use gnoblin_wm::ActivationDecision;

use crate::State;
use crate::access::ClientRole;
use crate::observation::ActivationDenial;

/// How long a minted token stays redeemable, in session turns.
///
/// The clock is the session's own turn counter — deterministic, monotonic and never a
/// wall clock — so a token's life is measured in the work the session did rather than
/// in seconds, and a test never depends on timing.
const TOKEN_LIFETIME: u64 = 10_000;

/// How many input serials are remembered for a token request.
///
/// A token may name a serial from a gesture that is a few events old, so the window is
/// wider than one event; it is bounded so a long session cannot grow the record.
const ISSUED_SERIALS: usize = 64;

/// One minted token and how it was earned.
#[derive(Clone, Debug)]
struct XdgActivationV1Minted {
    /// The authority's identity for it.
    token: TokenId,
    /// The session leader it was minted for; zero until launch attribution lands.
    leader: u32,
}

/// The activation tokens and the authority that decides about them.
pub(crate) struct XdgActivationV1 {
    /// The pure token machine.
    authority: Authority,
    /// The protocol's token string to the authority's identity.
    tokens: BTreeMap<String, XdgActivationV1Minted>,
    /// The next authority token to mint.
    next_token: u128,
    /// The serials this session handed to clients on input events.
    issued: VecDeque<u32>,
    /// The last refusal, for the observation.
    denied: Option<ActivationDenial>,
    /// Whether the session is covered, as the authority last heard.
    locked: bool,
}

impl Default for XdgActivationV1 {
    fn default() -> Self {
        Self {
            authority: Authority::new(),
            tokens: BTreeMap::new(),
            // Zero is never minted: an authority token of zero would be
            // indistinguishable from "no token" in a log line.
            next_token: 1,
            issued: VecDeque::new(),
            denied: None,
            locked: false,
        }
    }
}

impl XdgActivationV1 {
    /// The last refusal.
    pub(crate) fn denied(&self) -> Option<&ActivationDenial> {
        self.denied.as_ref()
    }

    /// Whether a serial is one this session issued.
    fn issued(&self, serial: u32) -> bool {
        self.issued.contains(&serial)
    }

    /// Remembers a serial handed to a client.
    fn remember_serial(&mut self, serial: u32) {
        if self.issued.back() == Some(&serial) {
            return;
        }
        self.issued.push_back(serial);
        while self.issued.len() > ISSUED_SERIALS {
            self.issued.pop_front();
        }
    }
}

/// The session leader a token is minted for and redeemed by.
///
/// Zero, because this slice binds a token to the session rather than to a process: the
/// launch-attribution slice is what will put the launched process's leader here, and
/// until it does, the token's single use and its earned-at-mint check are the whole of
/// the authority a client can present. Zero is not a plausible pid, so a log line that
/// shows it says exactly that.
const LEADER: u32 = 0;

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    fn token_created(&mut self, token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        let serial = data.serial.as_ref().map(|(serial, _seat)| u32::from(*serial));
        // A sandboxed client is not privileged, but it is a real application asking on
        // its own behalf: it needs the same trusted context every other application
        // needs, so sandboxing neither helps nor hurts here.
        let trusted_context = serial.is_some_and(|serial| self.activation.issued(serial));
        let privileged = data
            .client_id
            .as_ref()
            .is_some_and(|client| matches!(self.access.role(client), ClientRole::Privileged | ClientRole::Trusted));
        if !trusted_context && !privileged {
            debug!(?serial, "[activation] a token was refused: no trusted context");
            return false;
        }
        // The token is bound to the session rather than to a process: `xdg-activation`'s
        // token is a bearer capability, and this slice has no launch attribution to bind
        // it to a session leader. What the mint records is *how* it was earned, which is
        // what the trusted-context check above decided; redemption is then gated on the
        // token existing and being unspent.
        let leader = LEADER;
        // The spent tokens are dropped here rather than at redemption, so a reuse is
        // still refused as a reuse and a long session cannot grow the map: the authority
        // stops tracking a token the moment it is spent.
        let authority = &self.activation.authority;
        self.activation
            .tokens
            .retain(|_, minted| authority.token_expiry(minted.token).is_some());
        let token_id = TokenId::new(self.activation.next_token);
        self.activation.next_token = self.activation.next_token.saturating_add(1);
        let effects = self.activation.authority.apply(
            self.now(),
            Event::Launch {
                origin: LaunchOrigin {
                    leader: SessionLeader::new(leader),
                    app_id: data.app_id.clone().map(AppId::new),
                },
                token: token_id,
                lifetime: TOKEN_LIFETIME,
            },
        );
        let mut issued = false;
        for effect in effects {
            match effect {
                gnoblin_authority::Effect::IssueToken { .. } => issued = true,
                gnoblin_authority::Effect::RefuseLaunch { reason, .. } => {
                    debug!(
                        reason = reason.as_str(),
                        "[activation] the authority refused to mint a token"
                    );
                }
                _ => {}
            }
        }
        if !issued {
            return false;
        }
        self.activation.tokens.insert(
            token.as_str().to_owned(),
            XdgActivationV1Minted {
                token: token_id,
                leader,
            },
        );
        debug!(leader, ?serial, "[activation] token minted");
        true
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        self.redeem_activation(token.as_str().to_owned(), token_data, &surface);
    }
}

impl State {
    /// Redeems a protocol token.
    fn redeem_activation(&mut self, text: String, data: XdgActivationTokenData, surface: &WlSurface) {
        let window = self.window_of(surface);
        let Some(minted) = self.activation.tokens.get(&text).cloned() else {
            debug!("[activation] a token that was never issued");
            self.refuse_activation(window, gnoblin_authority::Denial::UnknownToken.as_str());
            return;
        };
        let _ = minted.leader;
        let claim = Claimant {
            client: ConnectionId::new(u64::from(LEADER)),
            leader: SessionLeader::new(LEADER),
            surface: SurfaceId::new(u64::from(surface.id().protocol_id())),
            app_id: data.app_id.clone().map(AppId::new),
            token: Some(minted.token),
        };
        let effects = self.activation.authority.apply(self.now(), Event::Activate { claim });
        let mut activated = false;
        let mut refusal = None;
        for effect in effects {
            match effect {
                gnoblin_authority::Effect::Activate { .. } => activated = true,
                gnoblin_authority::Effect::RefuseActivation { reason, .. } => refusal = Some(reason),
                _ => {}
            }
        }
        if activated {
            // The token stays in the map so that a second attempt is refused as
            // `reused-token` by the authority rather than looking like a forgery; the
            // next mint prunes the spent ones.
            match window {
                Some(window) => {
                    debug!(%window, "[activation] allowed");
                    let effects = self.wm.apply(gnoblin_wm::Event::ActivationRequested {
                        window,
                        decision: ActivationDecision::Allow,
                    });
                    self.apply(effects);
                }
                None => {
                    // A token is about a surface; a surface that is not a managed
                    // window has nothing to focus, so the claim buys nothing.
                    debug!("[activation] the token named a surface that is not a window");
                    self.activation.denied = Some(ActivationDenial {
                        window: None,
                        reason: "not-a-window".to_owned(),
                    });
                }
            }
            self.publish();
            return;
        }
        let reason = refusal.map_or("no-authority", |reason| reason.as_str());
        debug!(reason, "[activation] refused");
        self.refuse_activation(window, reason);
    }

    /// Records a refusal and asks the window manager to report it.
    ///
    /// Crate-visible because the `gtk_shell1` seam refuses an activation the same way:
    /// one refusal path is what keeps `activation-denied` meaning the same thing
    /// whichever protocol asked.
    pub(crate) fn refuse_activation(&mut self, window: Option<WindowId>, reason: &str) {
        self.activation.denied = Some(ActivationDenial {
            window,
            reason: reason.to_owned(),
        });
        if let Some(window) = window {
            let effects = self.wm.apply(gnoblin_wm::Event::ActivationRequested {
                window,
                decision: ActivationDecision::Deny,
            });
            self.apply(effects);
        }
        self.publish();
    }

    /// Remembers a serial handed to a client on an input event.
    pub(crate) fn remember_input_serial(&mut self, serial: u32) {
        self.activation.remember_serial(serial);
    }

    /// The session's turn, which is the only clock authority reads.
    pub(crate) fn now(&self) -> Tick {
        self.turn
    }

    /// Tells the authority whether the session is covered.
    ///
    /// A covered session may not be raised over, so a token presented while the lock is
    /// up is refused as `session-locked` rather than focusing a window under the cover.
    pub(crate) fn sync_authority_lock(&mut self) {
        let locked = self.lock.state() != gnoblin_lock::State::Unlocked;
        if self.activation.locked == locked {
            return;
        }
        self.activation.locked = locked;
        let now = self.now();
        let effects = self
            .activation
            .authority
            .apply(now, Event::LockChanged { active: locked });
        trace!(
            locked,
            effects = effects.len(),
            "[activation] the authority heard about the lock"
        );
    }
}
