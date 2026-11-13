//! The request layer: an offer, the slot it belongs to, and who ends up holding it.
//!
//! Design/rust-compositor.md, "Providers and slots": a request is how the compositor
//! asks a provider to decide something, one connection claims it, and it finishes
//! once. This module is the compositor's half of that: it owns the broker's state
//! machine, the slot registry that decides where an offer goes, and the last
//! configuration document the runtime committed.
//!
//! [`gnoblin_model::RequestBroker`] is reused rather than reimplemented — one owner,
//! one outcome, one tombstone — because a second copy of "who holds this request"
//! would be a second answer to it. What the broker deliberately does not know is *what*
//! a request means, so the slot, the proposition and the connection that offered it are
//! kept here, keyed by the identity the broker assigned.
//!
//! # Where an offer goes
//!
//! The slot decides, and the registry's three providers are three answers:
//!
//! - `shell` — the connection that claimed the slot. A shell draws its own prompt.
//! - `services` — the connection that offered the request, because the services role is
//!   the slot's default provider: it owns the prompt and answers with its own means. The
//!   exception is `secret.derive`: its offers go to the connection that claimed the slot,
//!   and are refused while none does, so the relay that asked is never the one that answers.
//!   For `auth.polkit` that means a refusal, which is the plan's fail-closed answer for
//!   a session with no shell to draw one.
//! - `none` — nobody, and the offer is refused with `no-provider` naming the slot. This
//!   is the difference the plan draws between a prompt that cannot be drawn and a
//!   prompt that hangs until its deadline. The one exception is `prompt.secret`: the
//!   compositor draws that prompt itself (`secret.rs`), so an offer with no claimant is
//!   held by the built-in dialog, which the broker still gives an identity and a deadline.
//!
//! # Failure modes first
//!
//! 1. **A request is offered to a slot nobody provides, and the host waits for its
//!    deadline.** [`Requests::offer`] refuses with `no-provider` and names the slot, so
//!    nothing is offered and the host fails at once (`tests/privilege.rs`:
//!    `an_offer_to_a_slot_with_no_provider_is_refused`). A passphrase prompt is not lost
//!    this way, because the built-in dialog serves it:
//!    `tests::an_unclaimed_prompt_is_opened_in_the_built_in_dialog_and_answered_to_the_host`.
//! 2. **A provider goes away while it holds a request.** Its claimed requests finish as
//!    ended, so the host that needed the decision stops waiting:
//!    `tests::a_departed_provider_still_ends_the_requests_of_slots_with_no_built_in`. A
//!    `prompt.secret` request is handed to the built-in dialog instead, with the same
//!    identity and deadline:
//!    `tests::a_claimant_that_leaves_before_answering_hands_the_prompt_to_the_built_in_dialog`.
//! 3. **The host that offered a request goes away, and its request hangs.**
//!    [`Requests::departed`] abandons every request that connection offered:
//!    `tests::a_departed_offerer_abandons_its_requests`.
//! 4. **An answer arrives from a connection that does not hold the request.** The
//!    broker refuses it with `request-taken`, and the request stays with its owner:
//!    `tests::an_answer_from_a_connection_that_does_not_hold_it_is_refused`.
//! 5. **A request outlives its deadline.** [`Requests::tick`] finishes it as timed out
//!    and both sides are told: `tests::a_request_past_its_deadline_ends_as_timed_out`.
//! 6. **An answer reaches a connection other than the one that offered the request.**
//!    Only that connection is sent the answer; the provider and every subscriber learn
//!    the completion without the value: `tests::only_the_offering_connection_reads_the_answer`.
//! 7. **A relay asks for a secret and answers it itself.** A `secret.derive` retrieve goes
//!    to the services connection that claimed the slot, and is refused with `no-provider`
//!    while none does: `tests::a_secret_retrieve_is_routed_to_the_services_claimant_not_the_relay`.

use std::collections::BTreeMap;
use std::str::FromStr as _;

use gnoblin_proto::Envelope;
use zeroize::Zeroize as _;

use gnoblin_client::wire;
use gnoblin_model::{
    CapabilitySlot, DeclineReason, Fields, Provider, RequestBroker, RequestCompletion, RequestEvent, RequestId,
    RequestOutcome, Slots, Tick,
};
use gnoblin_types::{ClientId, OwnerId};

/// The connection a request the compositor's own dialog serves is held by. It is no real connection, so no
/// client can claim, answer or decline such a request: only [`Requests::answer_builtin`] and
/// [`Requests::decline_builtin`] finish it.
fn builtin_client() -> ClientId {
    ClientId::new(u64::MAX)
}

/// The identity the compositor claims its own dialog's requests under; see [`builtin_client`].
fn builtin_owner() -> OwnerId {
    OwnerId::new(u64::MAX)
}

/// What the compositor's passphrase dialog shows for one request, taken from the offer and never from
/// anything the user or a script wrote.
pub(crate) struct SecretPrompt {
    /// The dialog's title.
    pub title: String,
    /// The question it asks.
    pub message: String,
    /// What the request is about, shown as who asks.
    pub subject: String,
}

/// One request that is live in the broker.
#[derive(Clone, Debug)]
struct Offer {
    /// The slot that owns it.
    slot: CapabilitySlot,
    /// What the provider shows. The schema's `request` object carries only the identity
    /// and the deadline, and a provider that had to ask what a request is about would
    /// need a second round trip before it could draw anything.
    title: String,
    /// The sentence the provider shows.
    message: String,
    /// What the request is about, for a provider that answers by subject. The
    /// compositor carries it without reading it.
    subject: Vec<u8>,
    /// The connection it was sent to.
    provider: ClientId,
    /// The connection that offered it, which is the only one that reads the answer.
    offered_by: ClientId,
    /// Whether the offer is the compositor's own `portal.ui` routing. Its outcome goes to the
    /// portal service, which turns it into the reply the relay is waiting for, never to
    /// `offered_by` as an answer frame: the relay asked a question of the compositor, not of the
    /// provider.
    portal: bool,
    /// Whether the compositor's own dialog serves the request: no shell claimed `prompt.secret`, or the
    /// claimant went away. The request is then held by [`builtin_client`], and the broker still owns
    /// its identity and its deadline.
    builtin: bool,
}

/// One frame the dispatcher has to write, and the connection to write it to.
#[derive(Debug)]
pub(crate) struct FrameDelivery {
    /// The connection to write to.
    pub client: ClientId,
    /// The frame.
    pub envelope: Envelope,
}

/// Why the request layer refused a member.
#[derive(Debug)]
pub(crate) struct RequestRefusal {
    /// The wire code, which is what the client reads.
    pub code: &'static str,
    /// A sentence for a log and a bug report. Never carries an answer.
    pub message: String,
}

/// The last configuration document the runtime committed.
///
/// The compositor holds it for as long as it runs, which is what makes a runtime
/// restart cost a reload and nothing else: the session keeps the settings it accepted
/// while the process that produced them is gone.
#[derive(Clone, Debug, Default)]
struct Configuration {
    /// The revision the last accepted document was given, or zero for none.
    revision: u64,
    /// The document itself, as the runtime encoded it. The compositor never decodes
    /// it: what a setting means is the configuration layer's business, and a doc that
    /// meant something here would be a second interpreter.
    document: Vec<u8>,
}

/// The request layer's state.
pub(crate) struct Requests {
    broker: RequestBroker,
    offers: BTreeMap<RequestId, Offer>,
    slots: Slots,
    /// The connection that claimed a slot as a shell, which is where its offers go.
    claims: BTreeMap<CapabilitySlot, ClientId>,
    published: Fields,
    configuration: Configuration,
    /// How the compositor's own `portal.ui` offers ended, for the portal service to settle.
    portal_outcomes: Vec<(RequestId, RequestOutcome)>,
    /// Requests the built-in passphrase dialog has to open, since the last call.
    secret_opened: Vec<RequestId>,
    /// Built-in requests that ended, whoever ended them, so the dialog closes.
    secret_ended: Vec<RequestId>,
}

impl Requests {
    /// The layer at the plan's defaults: providers from the slot table, no requests,
    /// no configuration.
    pub(crate) fn new() -> Requests {
        let slots = Slots::new();
        Requests {
            broker: RequestBroker::new(),
            offers: BTreeMap::new(),
            claims: BTreeMap::new(),
            // The session object is created with the same defaults (`SessionSync::new`), so a
            // claim that changes nothing publishes nothing.
            published: Slots::new().fields(),
            slots,
            configuration: Configuration::default(),
            portal_outcomes: Vec::new(),
            secret_opened: Vec::new(),
            secret_ended: Vec::new(),
        }
    }

    /// Offer one request to the slot that owns it.
    pub(crate) fn offer(
        &mut self,
        offered_by: ClientId,
        slot: &str,
        title: &str,
        message: &str,
        subject: &[u8],
        deadline: Tick,
    ) -> Result<(RequestId, Vec<FrameDelivery>), RequestRefusal> {
        let slot = CapabilitySlot::from_str(slot).map_err(|error| RequestRefusal {
            code: wire::code::UNKNOWN_SLOT,
            message: error.to_string(),
        })?;
        // A `prompt.secret` nobody draws is served by the compositor's own dialog, so the keyring can always
        // be unlocked. Every other slot with no provider still fails at once.
        let (provider, builtin) = match self.provider_of(slot, offered_by) {
            Ok(provider) => (provider, false),
            Err(refusal) if slot == CapabilitySlot::PromptSecret && refusal.code == wire::code::NO_PROVIDER => {
                (builtin_client(), true)
            }
            Err(refusal) => return Err(refusal),
        };
        let request = self.broker.offer(deadline);
        // The metadata is in place before the broker's events are drained, so the
        // offer that event describes can be routed and described.
        self.offers.insert(
            request,
            Offer {
                slot,
                title: title.to_owned(),
                message: message.to_owned(),
                subject: subject.to_vec(),
                provider,
                offered_by,
                portal: false,
                builtin,
            },
        );
        if builtin {
            self.take_over(request);
        }
        Ok((request, self.drain()))
    }

    /// Offer a `portal.ui` request to the shell that claimed the slot, on behalf of the relay
    /// connection that asked the compositor for it.
    ///
    /// The offer is the compositor's own, so the answer never travels to `requested_by` as an
    /// answer frame: [`Requests::take_portal_outcomes`] returns how it ended, and the portal
    /// service turns that into the one reply the relay waits for.
    pub(crate) fn offer_portal(
        &mut self,
        requested_by: ClientId,
        provider: ClientId,
        title: &str,
        message: &str,
        subject: &[u8],
        deadline: Tick,
    ) -> (RequestId, Vec<FrameDelivery>) {
        let request = self.broker.offer(deadline);
        self.offers.insert(
            request,
            Offer {
                slot: CapabilitySlot::PortalUi,
                title: title.to_owned(),
                message: message.to_owned(),
                subject: subject.to_vec(),
                provider,
                offered_by: requested_by,
                portal: true,
                builtin: false,
            },
        );
        (request, self.drain())
    }

    /// The connection that claimed `portal.ui`, when a shell holds it.
    pub(crate) fn portal_ui_claimant(&self) -> Option<ClientId> {
        if self.slots.provider(CapabilitySlot::PortalUi) != Provider::Shell {
            return None;
        }
        self.claims.get(&CapabilitySlot::PortalUi).copied()
    }

    /// How the compositor's own `portal.ui` offers ended since the last call.
    pub(crate) fn take_portal_outcomes(&mut self) -> Vec<(RequestId, RequestOutcome)> {
        std::mem::take(&mut self.portal_outcomes)
    }

    /// End a `portal.ui` offer nobody will answer: its requester left, cancelled, or the session
    /// locked. The provider is told it ended.
    pub(crate) fn abandon_portal(&mut self, request: RequestId) -> Vec<FrameDelivery> {
        let _ = self.broker.abandon(request);
        self.drain()
    }

    /// The connection an offer for `slot` goes to.
    fn provider_of(&self, slot: CapabilitySlot, offered_by: ClientId) -> Result<ClientId, RequestRefusal> {
        match self.slots.provider(slot) {
            // A secret retrieve is the relay asking for an app's secret, so it goes to the
            // services connection that claimed the slot and never back to the asker.
            Provider::Services if slot == CapabilitySlot::SecretDerive => {
                self.claims.get(&slot).copied().ok_or_else(|| RequestRefusal {
                    code: wire::code::NO_PROVIDER,
                    message: "no services connection claims `secret.derive`, so the request was not offered".to_owned(),
                })
            }
            Provider::Services => Ok(offered_by),
            Provider::Shell => self.claims.get(&slot).copied().ok_or_else(|| RequestRefusal {
                code: wire::code::NO_PROVIDER,
                message: format!("the `{slot}` slot is claimed by a connection that is gone"),
            }),
            Provider::None => Err(RequestRefusal {
                code: wire::code::NO_PROVIDER,
                message: format!("no provider claims the `{slot}` slot, so the request was not offered"),
            }),
        }
    }

    /// Claim a slot, so offers for it come to this connection.
    ///
    /// `secret.derive` is internal: only the session owner's connection, providing it, may
    /// claim it. A shell's claim is refused, so no shell can answer the consent that releases app secrets.
    pub(crate) fn claim_slot(
        &mut self,
        owner: ClientId,
        slot: CapabilitySlot,
        provider: Provider,
        owner_connection: bool,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
        if slot == CapabilitySlot::SecretDerive && (!owner_connection || provider != Provider::Services) {
            return Err(RequestRefusal {
                code: wire::code::DENIED,
                message: "`secret.derive` is internal: only the session owner's connection may claim it".to_owned(),
            });
        }
        // `portal.ui` is a shell's to claim: the services role has no UI to draw, and the built-in
        // dialog is the default, not a claimant.
        if slot == CapabilitySlot::PortalUi && provider != Provider::Shell {
            return Err(RequestRefusal {
                code: wire::code::DENIED,
                message: "`portal.ui` is claimed by a shell, which draws the dialog itself".to_owned(),
            });
        }
        // A claim names who draws or answers. Claiming `none` would clear another holder's claim
        // without being its holder; giving a slot back is `slot.release`, which only the holder may do.
        if provider == Provider::None {
            return Err(RequestRefusal {
                code: wire::code::DENIED,
                message: "a claim names a provider; give a slot back with `slot.release`".to_owned(),
            });
        }
        self.slots
            .claim(slot, provider, Some(OwnerId::new(owner.get())))
            .map_err(|error| RequestRefusal {
                code: wire::code::DENIED,
                message: error.to_string(),
            })?;
        // Both providers that answer on a connection are recorded: the shell slot and the
        // secret slot each route to the connection that claimed them.
        self.claims.insert(slot, owner);
        Ok(Vec::new())
    }

    /// Release a slot this connection claimed.
    pub(crate) fn release_slot(
        &mut self,
        owner: ClientId,
        slot: CapabilitySlot,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
        self.slots
            .release(slot, OwnerId::new(owner.get()))
            .map_err(|error| RequestRefusal {
                code: wire::code::DENIED,
                message: error.to_string(),
            })?;
        self.claims.remove(&slot);
        Ok(Vec::new())
    }

    /// Claim a request this connection was offered.
    ///
    /// Only the provider the offer was sent to may claim it. A connection that was not
    /// offered the request is refused as unknown to it before the broker sees the claim,
    /// so the broker's first-claim-wins rule cannot hand a request to a stranger.
    pub(crate) fn claim_request(
        &mut self,
        owner: ClientId,
        request: RequestId,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
        if self.offers.get(&request).is_some_and(|offer| offer.provider != owner) {
            let error = gnoblin_model::RequestError::Unknown { request };
            return Err(RequestRefusal {
                code: request_code(&error),
                message: error.to_string(),
            });
        }
        self.broker
            .claim(request, OwnerId::new(owner.get()))
            .map_err(|error| RequestRefusal {
                code: request_code(&error),
                message: error.to_string(),
            })?;
        Ok(self.drain())
    }

    /// Answer a request this connection claimed.
    ///
    /// An empty answer is an answer: the provider chose with no content.
    pub(crate) fn answer(
        &mut self,
        owner: ClientId,
        request: RequestId,
        mut answer: Vec<u8>,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
        // The broker drops an answer it refuses without wiping it, so every refusal is decided
        // here first and the value is wiped on the way out. The codes match the broker's.
        let owner = OwnerId::new(owner.get());
        let refusal = match self.broker.state(request) {
            Some(gnoblin_model::RequestState::Claimed { owner: holder, .. }) if holder == owner => None,
            Some(gnoblin_model::RequestState::Finished(_)) => Some(gnoblin_model::RequestError::Finished { request }),
            Some(_) => Some(gnoblin_model::RequestError::NotOwner { request, owner }),
            None => Some(gnoblin_model::RequestError::Unknown { request }),
        };
        if let Some(error) = refusal {
            answer.zeroize();
            return Err(RequestRefusal {
                code: request_code(&error),
                message: error.to_string(),
            });
        }
        self.broker
            .answer(request, owner, answer)
            .map_err(|error| RequestRefusal {
                code: request_code(&error),
                message: error.to_string(),
            })?;
        Ok(self.drain())
    }

    /// Decline a request this connection claimed, with the reason the provider gives when it has one.
    pub(crate) fn decline(
        &mut self,
        owner: ClientId,
        request: RequestId,
        reason: Option<DeclineReason>,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
        // A decline is the claimant's answer, and it stays one even for `prompt.secret`: the stock shell's
        // Escape is a plain decline, so reading it as "cannot draw this" would open a second dialog over a
        // prompt the user just dismissed. Only a claimant that is gone hands the request over.
        self.broker
            .decline_with(request, OwnerId::new(owner.get()), reason)
            .map_err(|error| RequestRefusal {
                code: request_code(&error),
                message: error.to_string(),
            })?;
        Ok(self.drain())
    }

    /// Fail every request at or past its deadline.
    pub(crate) fn tick(&mut self, now: Tick) -> Vec<FrameDelivery> {
        if self.broker.tick(now) == 0 {
            return Vec::new();
        }
        self.drain()
    }

    /// A connection ended: end what it held, abandon what it offered, release its
    /// slots.
    pub(crate) fn departed(&mut self, client: ClientId) -> Vec<FrameDelivery> {
        // A secret retrieve this connection claimed is declined with a reason, so the relay
        // that asked learns why. One it was offered but had not claimed is abandoned.
        let secrets: Vec<RequestId> = self
            .offers
            .iter()
            .filter(|(_, offer)| offer.slot == CapabilitySlot::SecretDerive && offer.provider == client)
            .map(|(request, _)| *request)
            .collect();
        for request in secrets {
            let owner = OwnerId::new(client.get());
            if self
                .broker
                .decline_with(request, owner, Some(DeclineReason::Unavailable))
                .is_err()
                && matches!(
                    self.broker.state(request),
                    Some(gnoblin_model::RequestState::Offered { .. })
                )
            {
                let _ = self.broker.abandon(request);
            }
        }
        // A passphrase prompt the departed claimant held is not lost: the compositor's own dialog takes it over
        // on the same request, with the deadline it already had, before the claim below would end it.
        let handed: Vec<RequestId> = self
            .offers
            .iter()
            .filter(|(_, offer)| {
                offer.slot == CapabilitySlot::PromptSecret
                    && !offer.builtin
                    && offer.provider == client
                    && offer.offered_by != client
            })
            .map(|(request, _)| *request)
            .collect();
        for request in handed {
            // A prompt the claimant was only offered is already waiting to be claimed.
            let _ = self.broker.reoffer(request, OwnerId::new(client.get()));
            self.take_over(request);
        }
        // A claim it held is released, so its requests end as gone rather than staying
        // with a connection that will never answer them.
        let _ = self.broker.release(OwnerId::new(client.get()));
        // An offer the departed connection was sent but never claimed ends too: nobody will
        // answer it, and the offering host should learn that now rather than at the deadline.
        // An offer the departed connection made is abandoned for the same reason.
        let abandoned: Vec<RequestId> = self
            .offers
            .iter()
            .filter(|(_, offer)| offer.offered_by == client || offer.provider == client)
            .map(|(request, _)| *request)
            .collect();
        for request in abandoned {
            let _ = self.broker.abandon(request);
        }
        // The requests that ended are reported by `drain`, which reads the broker's
        // queue, and the departed connection is not written to: its writer is gone.
        let deliveries: Vec<FrameDelivery> = self
            .drain()
            .into_iter()
            .filter(|delivery| delivery.client != client)
            .collect();
        // A slot a departed connection held falls back to its default provider, so the next
        // offer for it reaches the services role instead of nowhere. The model owns who holds
        // what, services claims included, so the release follows it rather than the shell map.
        let held: Vec<CapabilitySlot> = self.slots.held_by(OwnerId::new(client.get())).collect();
        for slot in held {
            let _ = self.slots.release(slot, OwnerId::new(client.get()));
            self.claims.remove(&slot);
        }
        deliveries
    }

    /// Commit a configuration document, returning the revision it was given.
    pub(crate) fn commit(&mut self, document: Vec<u8>) -> u64 {
        self.configuration.revision += 1;
        self.configuration.document = document;
        self.configuration.revision
    }

    /// The last configuration the compositor accepted.
    pub(crate) fn configuration(&self) -> (u64, &[u8]) {
        (self.configuration.revision, &self.configuration.document)
    }

    /// The session properties to publish, when a claim changed what they say.
    pub(crate) fn publication(&mut self) -> Option<Fields> {
        let fields = self.slots.fields();
        if fields == self.published {
            return None;
        }
        self.published = fields.clone();
        Some(fields)
    }

    /// Turn every event the broker has queued into the frames that report it.
    ///
    /// One place, so a request can never be reported twice: the broker emits exactly
    /// one `Finished` per request, and this drains the queue once per transition.
    fn drain(&mut self) -> Vec<FrameDelivery> {
        let mut deliveries = Vec::new();
        for event in self.broker.events() {
            match event {
                RequestEvent::Offered { request, deadline } => {
                    let Some(offer) = self.offers.get(&request).cloned() else {
                        continue;
                    };
                    // The compositor's own dialog is not a connection to write to.
                    if offer.builtin {
                        continue;
                    }
                    deliveries.push(FrameDelivery {
                        client: offer.provider,
                        envelope: wire::offer(
                            0,
                            request,
                            deadline,
                            offer.slot.name(),
                            &offer.title,
                            &offer.message,
                            &offer.subject,
                        ),
                    });
                }
                RequestEvent::Finished { request, mut outcome } => {
                    let Some(offer) = self.offers.remove(&request) else {
                        // Nobody is left to read the answer, so it is wiped rather than dropped.
                        wipe(&mut outcome);
                        continue;
                    };
                    if offer.builtin {
                        self.secret_ended.push(request);
                    }
                    if offer.portal {
                        // The relay asked the compositor, not the provider: the provider learns how
                        // its request ended, and the portal service reads the outcome and replies.
                        deliveries.push(FrameDelivery {
                            client: offer.provider,
                            envelope: wire::finished(request, RequestCompletion::from(&outcome)),
                        });
                        self.portal_outcomes.push((request, outcome));
                        continue;
                    }
                    // Only the host that offered the request reads the answer, and the
                    // frame that carries it is flagged sensitive by the schema.
                    if let RequestOutcome::Answered(answer) = &outcome {
                        deliveries.push(FrameDelivery {
                            client: offer.offered_by,
                            envelope: wire::answered(request, answer.as_slice()),
                        });
                    }
                    wipe(&mut outcome);
                    let completion = RequestCompletion::from(&outcome);
                    // The provider and the host can be one connection — the services role is
                    // the default provider of the slots it owns — and it is told once.
                    let mut told = Vec::new();
                    for client in [offer.offered_by, offer.provider] {
                        if told.contains(&client) {
                            continue;
                        }
                        told.push(client);
                        deliveries.push(FrameDelivery {
                            client,
                            envelope: wire::finished(request, completion),
                        });
                    }
                }
            }
        }
        deliveries
    }

    /// Hands a passphrase request that is waiting to be claimed to the compositor's own dialog: it is held by
    /// the built-in identity from now on, and the dialog is told to open.
    fn take_over(&mut self, request: RequestId) {
        if let Some(offer) = self.offers.get_mut(&request) {
            offer.provider = builtin_client();
            offer.builtin = true;
        }
        if self.broker.claim(request, builtin_owner()).is_ok() {
            self.secret_opened.push(request);
        }
    }

    /// The passphrase requests the built-in dialog has to open since the last call, oldest first.
    pub(crate) fn take_secret_opened(&mut self) -> Vec<RequestId> {
        std::mem::take(&mut self.secret_opened)
    }

    /// The built-in requests that ended since the last call, however they ended, so the dialog can close.
    pub(crate) fn take_secret_ended(&mut self) -> Vec<RequestId> {
        std::mem::take(&mut self.secret_ended)
    }

    /// What the built-in dialog shows for `request`, or `None` once it is not a live built-in request.
    pub(crate) fn secret_prompt(&self, request: RequestId) -> Option<SecretPrompt> {
        let offer = self.offers.get(&request).filter(|offer| offer.builtin)?;
        Some(SecretPrompt {
            title: offer.title.clone(),
            message: offer.message.clone(),
            subject: String::from_utf8_lossy(&offer.subject).into_owned(),
        })
    }

    /// The user typed `answer` into the built-in dialog. The host that offered the request reads it, as it
    /// reads a claimant's. The bytes are wiped here whether or not the request was still live.
    pub(crate) fn answer_builtin(&mut self, request: RequestId, mut answer: Vec<u8>) -> Vec<FrameDelivery> {
        let live = self.offers.get(&request).is_some_and(|offer| offer.builtin);
        if !live
            || self
                .broker
                .answer(request, builtin_owner(), std::mem::take(&mut answer))
                .is_err()
        {
            answer.zeroize();
            return Vec::new();
        }
        self.drain()
    }

    /// The user declined the built-in dialog: the request ends with no answer, as a claimant's plain decline does.
    pub(crate) fn decline_builtin(&mut self, request: RequestId) -> Vec<FrameDelivery> {
        let live = self.offers.get(&request).is_some_and(|offer| offer.builtin);
        if !live || self.broker.decline_with(request, builtin_owner(), None).is_err() {
            return Vec::new();
        }
        self.drain()
    }
}

/// Wipes an answer the request layer has finished with, so the value does not outlive its frame.
fn wipe(outcome: &mut RequestOutcome) {
    if let RequestOutcome::Answered(answer) = outcome {
        answer.zeroize();
    }
}

/// The wire code a broker refusal reports.
fn request_code(error: &gnoblin_model::RequestError) -> &'static str {
    match error {
        gnoblin_model::RequestError::Taken { .. } | gnoblin_model::RequestError::NotOwner { .. } => {
            wire::code::REQUEST_TAKEN
        }
        gnoblin_model::RequestError::Finished { .. } => wire::code::REQUEST_FINISHED,
        gnoblin_model::RequestError::Unknown { .. } => wire::code::REQUEST_UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICES: ClientId = ClientId::new(2);
    const SHELL: ClientId = ClientId::new(3);
    const HOST: ClientId = ClientId::new(4);

    #[test]
    fn a_shell_cannot_claim_the_internal_secret_derive_slot() {
        let mut requests = Requests::new();
        let refusal = requests
            .claim_slot(SHELL, CapabilitySlot::SecretDerive, Provider::Shell, false)
            .expect_err("a shell claim is refused");
        assert_eq!(refusal.code, wire::code::DENIED);
        assert!(
            refusal.message.contains("only the session owner's connection"),
            "{}",
            refusal.message
        );
    }

    #[test]
    fn a_services_connection_claims_secret_derive_as_its_provider() {
        let mut requests = Requests::new();
        assert!(
            requests
                .claim_slot(SERVICES, CapabilitySlot::SecretDerive, Provider::Services, true)
                .is_ok()
        );
        assert!(
            requests
                .claim_slot(SERVICES, CapabilitySlot::SecretDerive, Provider::Shell, true)
                .is_err(),
            "the services role cannot hand the internal slot to a shell"
        );
    }

    #[test]
    fn a_services_originated_offer_still_goes_to_the_offering_connection() {
        let mut requests = Requests::new();
        let (request, _) = requests
            .offer(
                SERVICES,
                "auth.polkit",
                "Authenticate",
                "Confirm",
                b"",
                Tick::new(1_000),
            )
            .expect("the services role provides auth.polkit by default");
        let offer = requests.offers.get(&request).expect("the offer is recorded");
        assert_eq!(offer.provider, SERVICES, "the provider is the connection that offered");
        assert_eq!(offer.offered_by, SERVICES);
    }

    #[test]
    fn a_host_offer_to_a_shell_claimed_slot_goes_to_the_shell() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SHELL, CapabilitySlot::PromptSecret, Provider::Shell, false)
            .expect("a shell claims the prompt slot");
        let (request, _) = requests
            .offer(HOST, "prompt.secret", "Unlock", "Passphrase", b"", Tick::new(1_000))
            .expect("the shell provides the prompt slot");
        let offer = requests.offers.get(&request).expect("the offer is recorded");
        assert_eq!(offer.provider, SHELL);
        assert_eq!(offer.offered_by, HOST);
    }

    #[test]
    fn a_secret_retrieve_is_routed_to_the_services_claimant_not_the_relay() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SERVICES, CapabilitySlot::SecretDerive, Provider::Services, true)
            .expect("the services role claims the secret slot");
        let (request, _) = requests
            .offer(
                HOST,
                CapabilitySlot::SecretDerive.name(),
                "Retrieve",
                "Retrieve",
                b"app-a",
                Tick::new(1_000),
            )
            .expect("the slot has a claimant");
        let offer = requests.offers.get(&request).expect("the offer is recorded");
        assert_eq!(
            offer.provider, SERVICES,
            "the claimant answers, not the relay that asked"
        );
        assert_eq!(offer.offered_by, HOST, "the answer goes back to the relay that asked");
    }

    #[test]
    fn a_secret_retrieve_with_no_claimant_is_refused() {
        let mut requests = Requests::new();
        let refusal = requests
            .offer(
                HOST,
                CapabilitySlot::SecretDerive.name(),
                "Retrieve",
                "Retrieve",
                b"app-a",
                Tick::new(1_000),
            )
            .expect_err("nothing claims the secret slot");
        assert_eq!(refusal.code, wire::code::NO_PROVIDER);
    }

    #[test]
    fn two_retrieves_for_different_apps_stay_apart() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SERVICES, CapabilitySlot::SecretDerive, Provider::Services, true)
            .expect("the services role claims the secret slot");
        let name = CapabilitySlot::SecretDerive.name();
        let (first, _) = requests
            .offer(HOST, name, "Retrieve", "Retrieve", b"app-a", Tick::new(1_000))
            .expect("the slot has a claimant");
        let (second, _) = requests
            .offer(HOST, name, "Retrieve", "Retrieve", b"app-b", Tick::new(1_000))
            .expect("the slot has a claimant");
        requests
            .claim_request(SERVICES, first)
            .expect("the claimant takes the first");
        requests
            .claim_request(SERVICES, second)
            .expect("the claimant takes the second");
        requests
            .answer(SERVICES, first, b"secret-a".to_vec())
            .expect("the claimant answers the first");
        assert!(
            matches!(
                requests.broker.state(second),
                Some(gnoblin_model::RequestState::Claimed { .. })
            ),
            "answering one retrieve leaves the other with its own claim"
        );
        let second_offer = requests
            .offers
            .get(&second)
            .expect("the second offer is still recorded");
        assert_eq!(
            second_offer.subject,
            b"app-b".to_vec(),
            "the second retrieve keeps its own app"
        );
    }

    #[test]
    fn a_retrieve_whose_relay_left_is_not_answered() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SERVICES, CapabilitySlot::SecretDerive, Provider::Services, true)
            .expect("the services role claims the secret slot");
        let (request, _) = requests
            .offer(
                HOST,
                CapabilitySlot::SecretDerive.name(),
                "Retrieve",
                "Retrieve",
                b"app-a",
                Tick::new(1_000),
            )
            .expect("the slot has a claimant");
        requests
            .claim_request(SERVICES, request)
            .expect("the claimant takes it");
        let _ = requests.departed(HOST);
        assert!(
            !requests.offers.contains_key(&request),
            "the departed relay's request is no longer offered"
        );
        let refusal = requests
            .answer(SERVICES, request, b"secret".to_vec())
            .expect_err("an answer for a request whose relay left is refused");
        assert_eq!(refusal.code, wire::code::REQUEST_FINISHED);
    }

    #[test]
    fn a_claimant_that_dies_mid_retrieve_refuses_the_relay_with_a_reason() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SERVICES, CapabilitySlot::SecretDerive, Provider::Services, true)
            .expect("the services role claims the secret slot");
        let (request, _) = requests
            .offer(
                HOST,
                CapabilitySlot::SecretDerive.name(),
                "Retrieve",
                "Retrieve",
                b"app-a",
                Tick::new(1_000),
            )
            .expect("the slot has a claimant");
        requests
            .claim_request(SERVICES, request)
            .expect("the claimant takes it");
        let deliveries = requests.departed(SERVICES);
        assert_eq!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Finished(
                gnoblin_model::RequestCompletion::Refused(DeclineReason::Unavailable)
            )),
            "the relay learns the retrieve failed with a reason, not a bare end"
        );
        assert!(
            deliveries.iter().any(|delivery| delivery.client == HOST),
            "the relay is told the request finished"
        );
    }

    const OTHER: ClientId = ClientId::new(5);

    /// A shell claims the prompt slot and a host offers a request to it, so the provider and
    /// the offering host are two different connections.
    fn shell_provided_request(requests: &mut Requests) -> RequestId {
        requests
            .claim_slot(SHELL, CapabilitySlot::PromptSecret, Provider::Shell, false)
            .expect("a shell claims the prompt slot");
        let (request, _) = requests
            .offer(HOST, "prompt.secret", "Unlock", "Passphrase", b"", Tick::new(1_000))
            .expect("the shell provides the prompt slot");
        request
    }

    #[test]
    fn a_connection_that_was_not_offered_a_request_cannot_claim_or_answer_it() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        let refusal = requests
            .claim_request(OTHER, request)
            .expect_err("a connection that was not offered the request cannot claim it");
        assert_eq!(refusal.code, wire::code::REQUEST_UNKNOWN);
        let refusal = requests
            .claim_request(HOST, request)
            .expect_err("the offering host is not the provider, so it cannot claim the request");
        assert_eq!(refusal.code, wire::code::REQUEST_UNKNOWN);
        assert!(
            matches!(
                requests.broker.state(request),
                Some(gnoblin_model::RequestState::Offered { .. })
            ),
            "a refused claim leaves the request offered to its provider"
        );
        requests
            .claim_request(SHELL, request)
            .expect("the provider claims the request");
        let refusal = requests
            .answer(OTHER, request, b"hunter2".to_vec())
            .expect_err("a connection that does not hold the request cannot answer it");
        assert_eq!(refusal.code, wire::code::REQUEST_TAKEN);
        assert!(
            matches!(
                requests.broker.state(request),
                Some(gnoblin_model::RequestState::Claimed { .. })
            ),
            "a refused answer leaves the claim with its provider"
        );
    }

    #[test]
    fn an_answer_from_a_connection_that_does_not_hold_it_is_refused() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        requests
            .claim_request(SHELL, request)
            .expect("the provider claims the request");
        let refusal = requests
            .answer(HOST, request, b"hunter2".to_vec())
            .expect_err("the offering host does not hold the claim, so it cannot answer");
        assert_eq!(refusal.code, wire::code::REQUEST_TAKEN);
    }

    /// A `prompt.secret` offer with no claimant: the compositor's own dialog holds it.
    fn builtin_request(requests: &mut Requests) -> RequestId {
        let (request, deliveries) = requests
            .offer(
                HOST,
                "prompt.secret",
                "Unlock",
                "Passphrase",
                b"keyring",
                Tick::new(1_000),
            )
            .expect("the built-in dialog serves an unclaimed prompt");
        assert!(
            deliveries.is_empty(),
            "the dialog is not a connection, so no offer is written"
        );
        request
    }

    /// The delivery that carries `answer` to `client`, if any.
    fn carries(deliveries: &[FrameDelivery], client: ClientId, answer: &[u8]) -> bool {
        deliveries.iter().any(|delivery| {
            delivery.client == client
                && delivery
                    .envelope
                    .payload()
                    .is_some_and(|payload| payload.windows(answer.len()).any(|window| window == answer))
        })
    }

    #[test]
    fn an_unclaimed_prompt_is_opened_in_the_built_in_dialog_and_answered_to_the_host() {
        let mut requests = Requests::new();
        let request = builtin_request(&mut requests);
        assert_eq!(requests.take_secret_opened(), vec![request]);
        let prompt = requests.secret_prompt(request).expect("the dialog reads the offer");
        assert_eq!(
            (prompt.title.as_str(), prompt.message.as_str(), prompt.subject.as_str()),
            ("Unlock", "Passphrase", "keyring"),
            "the dialog shows the request's own words"
        );
        let deliveries = requests.answer_builtin(request, b"hunter2".to_vec());
        assert!(
            carries(&deliveries, HOST, b"hunter2"),
            "the host that asked reads the answer"
        );
        assert_eq!(
            requests.take_secret_ended(),
            vec![request],
            "the dialog is told it is over"
        );
        assert!(requests.secret_prompt(request).is_none());
    }

    #[test]
    fn a_declined_built_in_prompt_ends_the_request_without_an_answer() {
        let mut requests = Requests::new();
        let request = builtin_request(&mut requests);
        let deliveries = requests.decline_builtin(request);
        assert!(
            deliveries.iter().any(|delivery| delivery.client == HOST),
            "the host learns the request ended"
        );
        assert_eq!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Finished(
                gnoblin_model::RequestCompletion::Ended
            ))
        );
    }

    #[test]
    fn no_connection_can_claim_answer_or_decline_a_built_in_prompt() {
        let mut requests = Requests::new();
        let request = builtin_request(&mut requests);
        // The host is the services role, and a prompt it asked about must not be one it can answer itself.
        for connection in [HOST, SHELL, SERVICES] {
            assert!(requests.claim_request(connection, request).is_err());
            assert!(requests.answer(connection, request, b"mine".to_vec()).is_err());
            assert!(requests.decline(connection, request, None).is_err());
        }
        assert!(matches!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Claimed { .. })
        ));
    }

    #[test]
    fn a_built_in_prompt_that_outlives_its_deadline_closes_the_dialog() {
        let mut requests = Requests::new();
        let request = builtin_request(&mut requests);
        let _ = requests.take_secret_opened();
        let deliveries = requests.tick(Tick::new(1_000));
        assert!(deliveries.iter().any(|delivery| delivery.client == HOST));
        assert_eq!(requests.take_secret_ended(), vec![request]);
    }

    #[test]
    fn a_built_in_prompt_whose_host_left_closes_the_dialog() {
        let mut requests = Requests::new();
        let request = builtin_request(&mut requests);
        let _ = requests.departed(HOST);
        assert_eq!(requests.take_secret_ended(), vec![request]);
    }

    #[test]
    fn a_claimant_that_leaves_before_answering_hands_the_prompt_to_the_built_in_dialog() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        requests
            .claim_request(SHELL, request)
            .expect("the provider claims the request");
        assert!(
            requests.take_secret_opened().is_empty(),
            "a claimant holds it, not the dialog"
        );

        let deliveries = requests.departed(SHELL);
        assert!(
            deliveries.iter().all(|delivery| delivery.client != HOST),
            "the host is not told the request ended: it is still being asked"
        );
        assert_eq!(
            requests.take_secret_opened(),
            vec![request],
            "the dialog opens on the same request"
        );
        assert_eq!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Claimed {
                owner: builtin_owner(),
                deadline: Tick::new(1_000),
            }),
            "with the deadline the request already had"
        );
        let deliveries = requests.answer_builtin(request, b"hunter2".to_vec());
        assert!(carries(&deliveries, HOST, b"hunter2"));
        assert!(
            requests.answer(SHELL, request, b"late".to_vec()).is_err(),
            "a claimant that is gone cannot answer it"
        );
    }

    #[test]
    fn a_claimant_that_leaves_before_claiming_hands_the_prompt_over_too() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        let deliveries = requests.departed(SHELL);
        assert!(deliveries.iter().all(|delivery| delivery.client != HOST));
        assert_eq!(requests.take_secret_opened(), vec![request]);
    }

    #[test]
    fn a_claimant_that_answers_first_wins() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        requests
            .claim_request(SHELL, request)
            .expect("the provider claims the request");
        let deliveries = requests
            .answer(SHELL, request, b"from the shell".to_vec())
            .expect("the provider answers");
        assert!(carries(&deliveries, HOST, b"from the shell"));
        // Its departure after the fact finds nothing to hand over, and the dialog never opens.
        let _ = requests.departed(SHELL);
        assert!(requests.take_secret_opened().is_empty());
        assert!(
            requests.answer_builtin(request, b"too late".to_vec()).is_empty(),
            "the dialog has nothing left to answer"
        );
    }

    #[test]
    fn a_claimants_decline_is_its_answer_and_ends_the_request() {
        for reason in [None, Some(DeclineReason::Denied)] {
            let mut requests = Requests::new();
            let request = shell_provided_request(&mut requests);
            requests
                .claim_request(SHELL, request)
                .expect("the provider claims the request");
            let deliveries = requests.decline(SHELL, request, reason).expect("the provider declines");
            assert!(
                deliveries.iter().any(|delivery| delivery.client == HOST),
                "the host learns the user dismissed the prompt"
            );
            assert!(
                requests.take_secret_opened().is_empty(),
                "a dismissed prompt is not asked again"
            );
        }
    }

    #[test]
    fn a_departed_provider_still_ends_the_requests_of_slots_with_no_built_in() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SERVICES, CapabilitySlot::SecretDerive, Provider::Services, true)
            .expect("services claim secret.derive");
        let (request, _) = requests
            .offer(HOST, "secret.derive", "Secret", "", b"org.example", Tick::new(1_000))
            .expect("offered to the services claimant");
        requests
            .claim_request(SERVICES, request)
            .expect("the provider claims the request");
        let deliveries = requests.departed(SERVICES);
        assert!(
            deliveries.iter().any(|delivery| delivery.client == HOST),
            "the host learns its request ended"
        );
        assert!(requests.take_secret_opened().is_empty());
    }

    #[test]
    fn a_departed_offerer_abandons_its_requests() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        let deliveries = requests.departed(HOST);
        assert_eq!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Finished(
                gnoblin_model::RequestCompletion::Ended
            )),
            "a request whose host left is abandoned"
        );
        assert!(
            deliveries.iter().any(|delivery| delivery.client == SHELL),
            "the provider is told the request it was offered is gone"
        );
        assert!(
            requests.departed(HOST).is_empty(),
            "a second departure reports nothing: the request finished exactly once"
        );
    }

    #[test]
    fn only_the_offering_connection_reads_the_answer() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        requests
            .claim_request(SHELL, request)
            .expect("the provider claims the request");
        let answer = b"hunter2".to_vec();
        let deliveries = requests
            .answer(SHELL, request, answer.clone())
            .expect("the provider answers");
        let carries = |delivery: &FrameDelivery| {
            delivery
                .envelope
                .payload()
                .is_some_and(|payload| payload.windows(answer.len()).any(|window| window == answer.as_slice()))
        };
        assert!(
            deliveries
                .iter()
                .any(|delivery| delivery.client == HOST && carries(delivery)),
            "the offering host reads the answer"
        );
        assert!(
            deliveries
                .iter()
                .all(|delivery| delivery.client != SHELL || !carries(delivery)),
            "the provider never receives the answer back"
        );
        assert!(
            deliveries
                .iter()
                .all(|delivery| !carries(delivery) || delivery.client == HOST),
            "no other connection receives the answer"
        );
    }

    #[test]
    fn a_request_past_its_deadline_ends_as_timed_out() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        assert!(
            requests.tick(Tick::new(999)).is_empty(),
            "a request before its deadline is left alone"
        );
        let deliveries = requests.tick(Tick::new(1_000));
        assert_eq!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Finished(
                gnoblin_model::RequestCompletion::TimedOut
            ))
        );
        assert!(
            deliveries.iter().any(|delivery| delivery.client == HOST),
            "the host learns its request timed out"
        );
    }

    #[test]
    fn portal_ui_has_no_claimant_until_a_shell_claims_it() {
        let mut requests = Requests::new();
        assert_eq!(
            requests.portal_ui_claimant(),
            None,
            "the built-in dialog is the default"
        );
        requests
            .claim_slot(SHELL, CapabilitySlot::PortalUi, Provider::Shell, true)
            .expect("a shell claims portal.ui");
        assert_eq!(requests.portal_ui_claimant(), Some(SHELL));
        requests
            .release_slot(SHELL, CapabilitySlot::PortalUi)
            .expect("the holder releases it");
        assert_eq!(
            requests.portal_ui_claimant(),
            None,
            "releasing restores the built-in dialog"
        );
    }

    #[test]
    fn the_services_role_cannot_claim_portal_ui() {
        let mut requests = Requests::new();
        let refusal = requests
            .claim_slot(SERVICES, CapabilitySlot::PortalUi, Provider::Services, true)
            .expect_err("the services role draws nothing");
        assert_eq!(refusal.code, wire::code::DENIED);
        assert_eq!(requests.portal_ui_claimant(), None);
    }

    #[test]
    fn a_second_shell_cannot_take_portal_ui_from_the_first() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SHELL, CapabilitySlot::PortalUi, Provider::Shell, true)
            .expect("the first claim wins");
        let refusal = requests
            .claim_slot(HOST, CapabilitySlot::PortalUi, Provider::Shell, true)
            .expect_err("a second claim is refused");
        assert_eq!(refusal.code, wire::code::DENIED);
        assert_eq!(requests.portal_ui_claimant(), Some(SHELL));
        assert!(
            requests.release_slot(HOST, CapabilitySlot::PortalUi).is_err(),
            "a connection that does not hold the slot cannot release it"
        );
        assert_eq!(requests.portal_ui_claimant(), Some(SHELL));
    }

    #[test]
    fn a_none_claim_does_not_clear_another_connections_slot() {
        // The failure this prevents: any connection claiming `none` cleared a slot another
        // connection held, bypassing the holder-only release.
        let mut requests = Requests::new();
        requests
            .claim_slot(SHELL, CapabilitySlot::PromptSecret, Provider::Shell, true)
            .expect("the shell claims the prompt slot");
        let refusal = requests
            .claim_slot(HOST, CapabilitySlot::PromptSecret, Provider::None, true)
            .expect_err("a none claim is refused");
        assert_eq!(refusal.code, wire::code::DENIED);
        assert!(
            requests
                .offer(HOST, "prompt.secret", "Unlock", "Passphrase", b"", Tick::new(1_000))
                .is_ok(),
            "the shell still provides the slot"
        );
    }

    #[test]
    fn a_portal_offer_is_answered_to_the_portal_service_and_never_to_the_relay() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SHELL, CapabilitySlot::PortalUi, Provider::Shell, true)
            .expect("a shell claims portal.ui");
        let (request, offered) =
            requests.offer_portal(HOST, SHELL, "Screenshot", "Share?", b"org.example", Tick::new(1_000));
        assert!(
            offered.iter().all(|delivery| delivery.client == SHELL),
            "only the claimant is offered the request"
        );
        requests.claim_request(SHELL, request).expect("the claimant claims it");
        let deliveries = requests
            .answer(SHELL, request, b"allow".to_vec())
            .expect("the claimant answers");
        assert!(
            deliveries.iter().all(|delivery| delivery.client == SHELL),
            "the relay is told nothing by the request layer: the portal service replies to it"
        );
        let outcomes = requests.take_portal_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].0, request);
        assert!(matches!(&outcomes[0].1, RequestOutcome::Answered(answer) if answer.as_slice() == b"allow"));
        assert!(requests.take_portal_outcomes().is_empty(), "an outcome is taken once");
    }

    #[test]
    fn a_departed_portal_claimant_ends_the_offer_for_the_portal_service() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SHELL, CapabilitySlot::PortalUi, Provider::Shell, true)
            .expect("a shell claims portal.ui");
        let (request, _) = requests.offer_portal(HOST, SHELL, "Screenshot", "Share?", b"org.example", Tick::new(1_000));
        requests.claim_request(SHELL, request).expect("the claimant claims it");
        let _ = requests.departed(SHELL);
        let outcomes = requests.take_portal_outcomes();
        assert_eq!(outcomes.len(), 1, "the offer ended once");
        assert!(matches!(outcomes[0].1, RequestOutcome::Ended));
        assert_eq!(requests.portal_ui_claimant(), None, "its claim went with it");
    }

    #[test]
    fn a_portal_offer_past_its_deadline_times_out_for_the_portal_service() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SHELL, CapabilitySlot::PortalUi, Provider::Shell, true)
            .expect("a shell claims portal.ui");
        let (request, _) = requests.offer_portal(HOST, SHELL, "Screenshot", "Share?", b"org.example", Tick::new(1_000));
        let _ = requests.tick(Tick::new(1_000));
        let outcomes = requests.take_portal_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].0, request);
        assert!(matches!(outcomes[0].1, RequestOutcome::TimedOut));
    }

    #[test]
    fn only_the_claimant_may_claim_or_answer_a_portal_offer() {
        let mut requests = Requests::new();
        requests
            .claim_slot(SHELL, CapabilitySlot::PortalUi, Provider::Shell, true)
            .expect("a shell claims portal.ui");
        let (request, _) = requests.offer_portal(HOST, SHELL, "Screenshot", "Share?", b"org.example", Tick::new(1_000));
        // The relay that asked is neither the provider nor allowed to answer its own question.
        assert!(requests.claim_request(HOST, request).is_err());
        assert!(requests.answer(HOST, request, b"allow".to_vec()).is_err());
        assert!(requests.take_portal_outcomes().is_empty(), "nothing settled");
    }
}
