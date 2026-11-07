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
//!   the slot's default provider: it owns the prompt and answers with its own means.
//!   For `auth.polkit` that means a refusal, which is the plan's fail-closed answer for
//!   a session with no shell to draw one.
//! - `none` — nobody, and the offer is refused with `no-provider` naming the slot. This
//!   is the difference the plan draws between a prompt that cannot be drawn and a
//!   prompt that hangs until its deadline.
//!
//! # Failure modes first
//!
//! 1. **A request is offered to a slot nobody provides, and the host waits for its
//!    deadline.** [`Requests::offer`] refuses with `no-provider` and names the slot, so
//!    nothing is offered and the host fails at once:
//!    `tests::an_offer_to_a_slot_with_no_provider_is_refused`.
//! 2. **A provider goes away while it holds a request.** Its claimed requests finish as
//!    ended, so the host that needed the decision stops waiting:
//!    `tests::a_departed_provider_ends_its_requests`.
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

use std::collections::BTreeMap;
use std::str::FromStr as _;

use gnoblin_proto::Envelope;

use gnoblin_client::wire;
use gnoblin_model::{
    CapabilitySlot, DeclineReason, Fields, Provider, RequestBroker, RequestCompletion, RequestEvent, RequestId,
    RequestOutcome, Slots, Tick,
};
use gnoblin_types::{ClientId, OwnerId};

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
        let provider = self.provider_of(slot, offered_by)?;
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
            },
        );
        Ok((request, self.drain()))
    }

    /// The connection an offer for `slot` goes to.
    fn provider_of(&self, slot: CapabilitySlot, offered_by: ClientId) -> Result<ClientId, RequestRefusal> {
        match self.slots.provider(slot) {
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
    /// `secret.derive` is internal: only the services role, providing it, may claim it. A
    /// shell's claim is refused, so no shell can answer the consent that releases app secrets.
    pub(crate) fn claim_slot(
        &mut self,
        owner: ClientId,
        slot: CapabilitySlot,
        provider: Provider,
        services_role: bool,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
        if slot == CapabilitySlot::SecretDerive && (!services_role || provider != Provider::Services) {
            return Err(RequestRefusal {
                code: wire::code::DENIED,
                message: "`secret.derive` is internal: only the services role may claim it".to_owned(),
            });
        }
        self.slots
            .claim(slot, provider, Some(OwnerId::new(owner.get())))
            .map_err(|error| RequestRefusal {
                code: wire::code::DENIED,
                message: error.to_string(),
            })?;
        if provider == Provider::Shell {
            self.claims.insert(slot, owner);
        } else {
            self.claims.remove(&slot);
        }
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
    pub(crate) fn claim_request(
        &mut self,
        owner: ClientId,
        request: RequestId,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
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
        answer: Vec<u8>,
    ) -> Result<Vec<FrameDelivery>, RequestRefusal> {
        self.broker
            .answer(request, OwnerId::new(owner.get()), answer)
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
        // A claim it held is released, so its requests end as gone rather than staying
        // with a connection that will never answer them.
        let _ = self.broker.release(OwnerId::new(client.get()));
        let abandoned: Vec<RequestId> = self
            .offers
            .iter()
            .filter(|(_, offer)| offer.offered_by == client)
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
                RequestEvent::Finished { request, outcome } => {
                    let Some(offer) = self.offers.remove(&request) else {
                        continue;
                    };
                    // Only the host that offered the request reads the answer, and the
                    // frame that carries it is flagged sensitive by the schema.
                    if let RequestOutcome::Answered(answer) = &outcome {
                        deliveries.push(FrameDelivery {
                            client: offer.offered_by,
                            envelope: wire::answered(request, answer),
                        });
                    }
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
            refusal.message.contains("only the services role"),
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
}
