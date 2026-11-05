//! The machine itself.

use std::collections::BTreeMap;

use crate::denial::{Decision, Denial};
use crate::effect::Effect;
use crate::event::{Answer, Capability, Claimant, ContextRequest, Event, Gesture, LaunchOrigin};
use crate::ids::{ClientId, GestureId, RequestId, RequesterId, SessionLeader, SurfaceId, Tick, TokenId};

/// How many of each value the machine keeps before it refuses new ones.
///
/// The defaults are the C core's own limits, so a runtime that behaved under
/// `gnoblin-control-focus.c` behaves the same here: `MAX_FOCUS_CONTEXTS` 128 and
/// `MAX_MENU_CONTEXTS` 256.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Live activation tokens.
    pub tokens: usize,
    /// Live trusted input contexts.
    pub contexts: usize,
    /// Consent prompts waiting for an answer.
    pub requests: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            tokens: 256,
            contexts: 128,
            requests: 64,
        }
    }
}

/// How far a grant has got.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consent {
    /// Asked and not yet answered. Nothing may be used.
    Pending,
    /// Granted until the grant's expiry.
    Granted,
    /// Refused until the grant's expiry, so a client cannot re-prompt in a
    /// loop.
    Refused,
}

/// A launch the session authorised, and the token that belongs to it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Launch {
    token: TokenId,
    leader: SessionLeader,
    expires_at: Tick,
}

/// A gesture's capability.
#[derive(Clone, Debug, Eq, PartialEq)]
struct InputContext {
    client: ClientId,
    surface: SurfaceId,
    expires_at: Tick,
    consumed: bool,
}

/// A caller's permission for one capability.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Grant {
    consent: Consent,
    expires_at: Tick,
}

/// A consent prompt that is waiting for the provider.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Pending {
    requester: RequesterId,
    capability: Capability,
    provider: ClientId,
    expires_at: Tick,
}

/// Launch attribution, activation tokens, trusted input contexts and grants.
///
/// Pure and deterministic: every method takes the current [`Tick`] from the
/// caller and owns no clock, no process table and no socket, so a sequence of
/// events replays to identical decisions.
#[derive(Clone, Debug)]
pub struct Authority {
    limits: Limits,
    launches: Vec<Launch>,
    /// Tokens already redeemed, so a second redemption is `ReusedToken` rather
    /// than `UnknownToken`, which is what a dictionary attack looks like.
    spent: Vec<TokenId>,
    gestures: BTreeMap<GestureId, InputContext>,
    portal_ui: Option<ClientId>,
    grants: BTreeMap<(RequesterId, Capability), Grant>,
    pending: BTreeMap<RequestId, Pending>,
    locked: bool,
}

impl Default for Authority {
    fn default() -> Self {
        Self::new()
    }
}

impl Authority {
    /// An authority with no launch, no context and no grant.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(Limits::default())
    }

    /// An authority with explicit limits, for tests and for a constrained host.
    #[must_use]
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            limits,
            launches: Vec::new(),
            spent: Vec::new(),
            gestures: BTreeMap::new(),
            portal_ui: None,
            grants: BTreeMap::new(),
            pending: BTreeMap::new(),
            locked: false,
        }
    }

    /// Whether the session is covered according to the last
    /// [`Event::LockChanged`].
    #[must_use]
    pub const fn locked(&self) -> bool {
        self.locked
    }

    /// The client that claims `portal.ui`, if any.
    #[must_use]
    pub const fn provider(&self) -> Option<ClientId> {
        self.portal_ui
    }

    /// What a caller's grant has reached, for `gnoblinctl` and for tests.
    #[must_use]
    pub fn consent(&self, requester: RequesterId, capability: Capability) -> Option<Consent> {
        self.grants.get(&(requester, capability)).map(|grant| grant.consent)
    }

    /// When a token stops being redeemable, while it is still live.
    #[must_use]
    pub fn token_expiry(&self, token: TokenId) -> Option<Tick> {
        self.launches
            .iter()
            .find(|launch| launch.token == token)
            .map(|launch| launch.expires_at)
    }

    /// Whether a caller may use a capability right now.
    ///
    /// This is the gate the portal and the compositor call on every frame and
    /// every injected event, so it is a query, not an event: nothing about
    /// asking changes the answer.
    #[must_use]
    pub fn gate(&self, now: Tick, requester: RequesterId, capability: Capability) -> Decision {
        if self.locked {
            // Consent survives a lock, frames and injected input do not.
            return Decision::Denied(Denial::Locked);
        }
        let Some(grant) = self.grants.get(&(requester, capability)) else {
            return Decision::Denied(Denial::NoGrant);
        };
        if now >= grant.expires_at {
            return Decision::Denied(Denial::NoGrant);
        }
        match grant.consent {
            Consent::Granted => Decision::Allowed,
            Consent::Pending => Decision::Denied(Denial::NoConsent),
            Consent::Refused => Decision::Denied(Denial::Refused),
        }
    }

    /// Apply one event and return the effects in the order they must happen.
    pub fn apply(&mut self, now: Tick, event: Event) -> Vec<Effect> {
        match event {
            Event::Launch {
                origin,
                token,
                lifetime,
            } => self.launch(now, origin, token, lifetime),
            Event::Activate { claim } => self.activate(now, claim),
            Event::GestureBegan { gesture, lifetime } => self.gesture_began(now, gesture, lifetime),
            Event::GestureEnded { gesture } => self.gesture_ended(gesture),
            Event::ContextRequest {
                gesture,
                client,
                surface,
                request,
            } => self.context_request(now, gesture, client, surface, request),
            Event::PortalUiClaimed { client } => self.portal_ui_claimed(client),
            Event::PortalUiReleased { client } => self.portal_ui_released(client),
            Event::ConsentRequested {
                request,
                requester,
                capability,
                lifetime,
            } => self.consent_requested(now, request, requester, capability, lifetime),
            Event::ConsentAnswered { request, from, answer } => self.consent_answered(now, request, from, answer),
            Event::RequesterGone { requester } => self.requester_gone(requester),
            Event::GrantUsed { requester, capability } => self.grant_used(now, requester, capability),
            Event::LockChanged { active } => self.lock_changed(active),
        }
    }

    /// Record a launch and issue its single-use token.
    fn launch(&mut self, now: Tick, origin: LaunchOrigin, token: TokenId, lifetime: u64) -> Vec<Effect> {
        if self.spent.contains(&token) || self.launches.iter().any(|launch| launch.token == token) {
            // A reused value would make two launches share one token, and the
            // second redemption would be indistinguishable from the first.
            return vec![Effect::RefuseLaunch {
                token,
                reason: Denial::DuplicateToken,
            }];
        }
        if self.launches.len() >= self.limits.tokens {
            self.launches.retain(|launch| now < launch.expires_at);
            if self.launches.len() >= self.limits.tokens {
                return vec![Effect::RefuseLaunch {
                    token,
                    reason: Denial::AtCapacity,
                }];
            }
        }
        let expires_at = now.after(lifetime);
        self.launches.push(Launch {
            token,
            leader: origin.leader,
            expires_at,
        });
        vec![Effect::IssueToken {
            token,
            leader: origin.leader,
            app_id: origin.app_id,
            expires_at,
        }]
    }

    /// Resolve a request for focus.
    ///
    /// Only a live token issued to this session leader activates, and it
    /// activates once. The claimed app id and the bare leader are never
    /// consulted: they are labels and process ids, and both are forgeable.
    fn activate(&mut self, now: Tick, claim: Claimant) -> Vec<Effect> {
        let client = claim.client;
        let surface = claim.surface;
        let refuse = |reason: Denial| {
            vec![Effect::RefuseActivation {
                client,
                surface,
                reason,
            }]
        };
        if self.locked {
            // A covered session may not be covered further by a new window.
            return refuse(Denial::SessionLocked);
        }
        let Some(token) = claim.token else {
            return refuse(Denial::NoAuthority);
        };
        if self.spent.contains(&token) {
            return refuse(Denial::ReusedToken);
        }
        let Some(index) = self.launches.iter().position(|launch| launch.token == token) else {
            return refuse(Denial::UnknownToken);
        };
        let (expires_at, leader) = {
            let launch = &self.launches[index];
            (launch.expires_at, launch.leader)
        };
        if now >= expires_at {
            self.launches.remove(index);
            return refuse(Denial::ExpiredToken);
        }
        if leader != claim.leader {
            // A token is bound to the launch that received it, so a process
            // from another session leader cannot present it.
            return refuse(Denial::ForeignLaunch);
        }
        self.launches.remove(index);
        self.remember(token);
        vec![Effect::Activate { client, surface }]
    }

    fn remember(&mut self, token: TokenId) {
        self.spent.push(token);
        if self.spent.len() > self.limits.tokens {
            self.spent.remove(0);
        }
    }

    /// Record a trusted input context for a gesture that is live now.
    fn gesture_began(&mut self, now: Tick, gesture: Gesture, lifetime: u64) -> Vec<Effect> {
        if self.locked || self.gestures.contains_key(&gesture.id) {
            // A covered session begins no gesture, and a serial that is already
            // live cannot be refreshed into a longer life.
            return Vec::new();
        }
        if self.gestures.len() >= self.limits.contexts {
            self.gestures.retain(|_, context| now < context.expires_at);
            if self.gestures.len() >= self.limits.contexts {
                return Vec::new();
            }
        }
        let expires_at = now.after(lifetime);
        self.gestures.insert(
            gesture.id,
            InputContext {
                client: gesture.client,
                surface: gesture.surface,
                expires_at,
                consumed: false,
            },
        );
        vec![Effect::Context {
            gesture: gesture.id,
            client: gesture.client,
            surface: gesture.surface,
            kind: gesture.kind,
            expires_at,
        }]
    }

    /// The gesture is over, so its context is over.
    fn gesture_ended(&mut self, gesture: GestureId) -> Vec<Effect> {
        if self.gestures.remove(&gesture).is_some() {
            vec![Effect::EndContext { gesture }]
        } else {
            Vec::new()
        }
    }

    /// Authorise one privileged request under a live context.
    ///
    /// A context is single use, expires and belongs to the client and surface of
    /// its gesture, so a queued operation, a stale handle or a relayed call all
    /// fail.
    fn context_request(
        &mut self,
        now: Tick,
        gesture: GestureId,
        client: ClientId,
        surface: SurfaceId,
        request: ContextRequest,
    ) -> Vec<Effect> {
        let refuse = |reason: Denial| {
            vec![Effect::RefuseContext {
                gesture,
                client,
                request,
                reason,
            }]
        };
        if self.locked {
            return refuse(Denial::SessionLocked);
        }
        let Some(mut context) = self.gestures.remove(&gesture) else {
            return refuse(Denial::StaleContext);
        };
        if now >= context.expires_at {
            return refuse(Denial::ExpiredContext);
        }
        if context.client != client || context.surface != surface {
            self.gestures.insert(gesture, context);
            return refuse(Denial::WrongTarget);
        }
        if context.consumed {
            self.gestures.insert(gesture, context);
            return refuse(Denial::ConsumedContext);
        }
        context.consumed = true;
        self.gestures.insert(gesture, context);
        vec![Effect::Authorise {
            gesture,
            client,
            surface,
            request,
        }]
    }

    fn portal_ui_claimed(&mut self, client: ClientId) -> Vec<Effect> {
        if self.portal_ui == Some(client) {
            return Vec::new();
        }
        self.portal_ui = Some(client);
        let mut effects = vec![Effect::PortalUi { provider: Some(client) }];
        // Prompts asked of the previous provider can no longer be answered by
        // the client that has to answer them.
        effects.extend(self.dismiss_pending(Denial::NotProvider));
        effects
    }

    fn portal_ui_released(&mut self, client: ClientId) -> Vec<Effect> {
        if self.portal_ui != Some(client) {
            return Vec::new();
        }
        self.portal_ui = None;
        let mut effects = vec![Effect::PortalUi { provider: None }];
        effects.extend(self.dismiss_pending(Denial::NoPortalUi));
        effects
    }

    /// Ask the provider for consent, or answer from a live grant.
    fn consent_requested(
        &mut self,
        now: Tick,
        request: RequestId,
        requester: RequesterId,
        capability: Capability,
        lifetime: u64,
    ) -> Vec<Effect> {
        let answer = |decision: Decision| vec![Effect::Answer { request, decision }];
        if self.locked {
            return answer(Decision::Denied(Denial::Locked));
        }
        if let Some(grant) = self.grants.get(&(requester, capability))
            && now < grant.expires_at
        {
            // Asking again while the answer still stands asks the user nothing.
            return match grant.consent {
                Consent::Granted => answer(Decision::Allowed),
                Consent::Refused => answer(Decision::Denied(Denial::Refused)),
                Consent::Pending => answer(Decision::Denied(Denial::NoConsent)),
            };
        }
        let Some(provider) = self.portal_ui else {
            // D14: without a shell that claims `portal.ui` nothing can be
            // asked, so the request fails closed.
            return answer(Decision::Denied(Denial::NoPortalUi));
        };
        if self.pending.len() >= self.limits.requests {
            return answer(Decision::Denied(Denial::AtCapacity));
        }
        // A previous prompt for the same grant may have been abandoned. It can
        // never be answered into this new grant, so withdraw it first.
        let mut effects = self.withdraw_prompts(requester, capability);
        let expires_at = now.after(lifetime);
        self.grants.insert(
            (requester, capability),
            Grant {
                consent: Consent::Pending,
                expires_at,
            },
        );
        self.pending.insert(
            request,
            Pending {
                requester,
                capability,
                provider,
                expires_at,
            },
        );
        effects.push(Effect::Prompt {
            request,
            capability,
            requester,
            provider,
        });
        effects
    }

    fn consent_answered(&mut self, now: Tick, request: RequestId, from: ClientId, answer: Answer) -> Vec<Effect> {
        let Some(pending) = self.pending.remove(&request) else {
            return vec![Effect::Ignored { request, from }];
        };
        if now >= pending.expires_at {
            // Only the grant this prompt created, never a newer one that a later
            // request replaced it with.
            let key = (pending.requester, pending.capability);
            if self
                .grants
                .get(&key)
                .is_some_and(|grant| grant.expires_at == pending.expires_at)
            {
                self.grants.remove(&key);
            }
            return vec![Effect::Answer {
                request,
                decision: Decision::Denied(Denial::ConsentExpired),
            }];
        }
        if pending.provider != from {
            // Only the provider answers. A client that claims the slot's
            // identity, its app id or its name changes nothing here.
            self.pending.insert(request, pending);
            return vec![Effect::Ignored { request, from }];
        }
        let consent = match answer {
            Answer::Allow => Consent::Granted,
            Answer::Deny => Consent::Refused,
        };
        self.grants.insert(
            (pending.requester, pending.capability),
            Grant {
                consent,
                expires_at: pending.expires_at,
            },
        );
        let decision = match answer {
            Answer::Allow => Decision::Allowed,
            Answer::Deny => Decision::Denied(Denial::Refused),
        };
        vec![Effect::Answer { request, decision }]
    }

    fn requester_gone(&mut self, requester: RequesterId) -> Vec<Effect> {
        let requests: Vec<RequestId> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.requester == requester)
            .map(|(request, _)| *request)
            .collect();
        let had_grant = self.grants.keys().any(|(owner, _)| *owner == requester) || !requests.is_empty();
        self.grants.retain(|(owner, _), _| *owner != requester);
        let mut effects = Vec::new();
        for request in requests {
            self.pending.remove(&request);
            effects.push(Effect::Dismiss { request });
        }
        if had_grant {
            // A stream belongs to the request that created it, so it ends with
            // the caller rather than outliving it.
            effects.push(Effect::EndGrants { requester });
        }
        effects
    }

    fn grant_used(&mut self, now: Tick, requester: RequesterId, capability: Capability) -> Vec<Effect> {
        let decision = self.gate(now, requester, capability);
        vec![Effect::Gate {
            requester,
            capability,
            decision,
        }]
    }

    fn lock_changed(&mut self, active: bool) -> Vec<Effect> {
        if self.locked == active {
            return Vec::new();
        }
        self.locked = active;
        let mut effects = vec![Effect::Capture { allowed: !active }];
        if active {
            // Every context authorises a gesture that began on the desktop, so
            // a lock ends all of them: nothing queued can be redeemed later.
            let gestures: Vec<GestureId> = self.gestures.keys().copied().collect();
            self.gestures.clear();
            for gesture in gestures {
                effects.push(Effect::EndContext { gesture });
            }
        }
        effects
    }

    /// Withdraw every waiting prompt for one grant, which a new request replaces.
    fn withdraw_prompts(&mut self, requester: RequesterId, capability: Capability) -> Vec<Effect> {
        let stale: Vec<RequestId> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.requester == requester && pending.capability == capability)
            .map(|(request, _)| *request)
            .collect();
        let mut effects = Vec::new();
        for request in stale {
            self.pending.remove(&request);
            effects.push(Effect::Dismiss { request });
        }
        effects
    }

    /// Drop every waiting prompt, answering all of them with `reason`.
    fn dismiss_pending(&mut self, reason: Denial) -> Vec<Effect> {
        let pending = std::mem::take(&mut self.pending);
        let mut effects = Vec::new();
        for (request, prompt) in pending {
            self.grants.remove(&(prompt.requester, prompt.capability));
            effects.push(Effect::Answer {
                request,
                decision: Decision::Denied(reason),
            });
        }
        effects
    }
}
