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
        // Both providers that answer on a connection are recorded: the shell slot and the
        // secret slot each route to the connection that claimed them.
        if matches!(provider, Provider::Shell | Provider::Services) {
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

    #[test]
    fn a_departed_provider_ends_its_requests() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        requests
            .claim_request(SHELL, request)
            .expect("the provider claims the request");
        let deliveries = requests.departed(SHELL);
        assert_eq!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Finished(
                gnoblin_model::RequestCompletion::Ended
            )),
            "a claimed request whose provider left ends as ended"
        );
        assert!(
            deliveries.iter().any(|delivery| delivery.client == HOST),
            "the host learns its request ended"
        );
    }

    #[test]
    fn a_departed_provider_ends_the_requests_it_was_offered_but_did_not_claim() {
        let mut requests = Requests::new();
        let request = shell_provided_request(&mut requests);
        let deliveries = requests.departed(SHELL);
        assert_eq!(
            requests.broker.state(request),
            Some(gnoblin_model::RequestState::Finished(
                gnoblin_model::RequestCompletion::Ended
            )),
            "an offer nobody will now claim ends rather than waiting for its deadline"
        );
        assert!(
            deliveries.iter().any(|delivery| delivery.client == HOST),
            "the host learns its request ended instead of waiting out the deadline"
        );
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
}
