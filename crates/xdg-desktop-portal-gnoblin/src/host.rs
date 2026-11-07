//! The privileged IPC connection: offers out, answers and settings observations in.
//!
//! The plan gives the portal one connection, "and it is the capability to capture and inject
//! input." The services role passes it when it starts the portal; a bus activation has none,
//! and then [`NoShell`] takes its place. This module is that
//! connection's whole life. One thread owns it ([`IpcLoop`]), because a [`Client`] is a
//! single reader: the answers to our prompts and the settings object's property diffs
//! arrive on the same socket, and two readers would split a frame stream.
//!
//! # Two channels, one reader
//!
//! A method that needs consent is a D-Bus handler, and the offer has to leave from the
//! thread that owns the client. So [`IpcHost`] is a handle: it sends the prompt to the loop
//! and waits for the offer's identity. That wait is microseconds (one round trip on a local
//! socket), and it is over before the handler starts waiting for the user's answer, which
//! is the long wait and happens on the handler's own future.
//!
//! The loop polls its socket with a short read timeout, so a pending offer is serviced
//! within one timeout even when nothing else arrives, and drains every request that has
//! already arrived on each pass. It never spins: a read that times out is [`ClientError::Idle`],
//! which is the loop's cue to look at the queue.
//!
//! # Why the provider's identity is a constant
//!
//! `gnoblin-authority` records who may answer a prompt and refuses any other answerer. Over
//! D-Bus that check is what stops one client answering another's dialogue; over the IPC the
//! request broker performs exactly the same check before a byte leaves the compositor
//! (`RequestBroker::answer` refuses a connection that does not own the request), and a
//! client of the IPC is never told which connection holds the slot. So the identity this
//! host reports is [`crate::consent::OFFERER`], documented where it is defined: the check
//! has already happened, and inventing a per-connection number would be a number that means
//! nothing.
//!
//! # The answer's encoding
//!
//! The prompt is offered here, so the question is this backend's and so is the answer's
//! shape: `allow` or `deny`. A provider that answers anything else, or declines the
//! request, refuses — an unreadable answer never consents.

use std::collections::HashMap;
use std::time::Duration;

use futures_channel::{mpsc, oneshot};
use gnoblin_client::wire::{PortalRequest, VerifiedCaller};
use gnoblin_client::{Client, ClientError, Event, PortalOutcome, PortalTicket, Transport};
use gnoblin_model::{Fields, Filter};
use gnoblin_types::{RequestId, Version};

use crate::consent::{Host, OFFERER, Offer, PortalCapability, Prompt, Refusal};

use crate::log;

/// The slot consent prompts are offered to.
const SLOT: &str = crate::consent::PORTAL_UI;

/// How long one socket read waits before the loop looks at its queue.
///
/// It is the loop's whole latency budget for a prompt that arrives while the compositor is
/// quiet: 20 ms is below the threshold a person notices and far above the cost of a wake-up
/// eight times a second.
const READ_TIMEOUT: Duration = Duration::from_millis(20);

/// One thing the connection's thread has to do.
enum Command {
    /// Offer a prompt and answer with its identity.
    Offer {
        prompt: Prompt,
        timeout: Duration,
        reply: oneshot::Sender<Result<Offer, Refusal>>,
    },
    /// Withdraw an offer that can no longer be answered.
    Decline(RequestId),
}

/// The `portal.ui` host, as a handler sees it.
pub struct IpcHost {
    commands: mpsc::UnboundedSender<Command>,
}

impl std::fmt::Debug for IpcHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("IpcHost")
    }
}

impl IpcHost {
    /// The handle half of a connection, and the loop that must be started with the other.
    ///
    /// The two are made together so a handle can never exist without a loop: an offer sent
    /// into a channel nobody reads is a handler that waits for ever, which is the failure
    /// this shape removes.
    #[must_use]
    pub fn pair<T: Transport>(client: Client<T>) -> (IpcHost, IpcLoop<T>) {
        let (commands, queue) = mpsc::unbounded();
        (
            IpcHost { commands },
            IpcLoop {
                client,
                queue,
                pending: HashMap::new(),
                next_request: 0,
            },
        )
    }
}

impl Host for IpcHost {
    fn offer(&mut self, prompt: &Prompt, timeout: Duration) -> Result<Offer, Refusal> {
        let (reply, offered) = oneshot::channel();
        let command = Command::Offer {
            prompt: prompt.clone(),
            timeout,
            reply,
        };
        if self.commands.unbounded_send(command).is_err() {
            // The loop is gone, which means the connection is: nothing will ever draw a
            // prompt, so this is the same answer as a slot nobody claims.
            return Err(Refusal::NoProvider { slot: SLOT });
        }
        match futures_lite::future::block_on(offered) {
            Ok(offer) => offer,
            Err(_) => Err(Refusal::NoProvider { slot: SLOT }),
        }
    }

    fn withdraw(&mut self, request: RequestId) {
        // The loop owns the connection, so the decline goes through the same queue as an
        // offer. A queued decline for a request the provider already answered is refused
        // by the compositor and logged there; the guard has already released the caller, so
        // nothing waits on it.
        let _ = self.commands.unbounded_send(Command::Decline(request));
    }
}

/// The host of a session with no compositor IPC: no shell can draw a prompt.
///
/// Every offer is refused before it is made, naming `portal.ui` as the missing slot, which
/// is the same answer [`IpcHost`] gives once its connection has gone (D14). The portal
/// still serves what needs no prompt, such as `Settings`, and refuses what does.
#[derive(Debug, Default)]
pub struct NoShell;

impl Host for NoShell {
    fn offer(&mut self, _prompt: &Prompt, _timeout: Duration) -> Result<Offer, Refusal> {
        Err(Refusal::NoProvider { slot: SLOT })
    }

    fn withdraw(&mut self, _request: RequestId) {}
}

/// Build the `portal.ui` request that carries one consent prompt.
///
/// The compositor's consent screen takes the caller as a verified identity, so the app id
/// goes as the caller's label. This process does not know the caller's pid, so it goes as
/// zero; the compositor resolves the caller from the bus itself. The interface names the
/// portal screen the capability came from.
fn portal_request(prompt: &Prompt, timeout: Duration) -> PortalRequest {
    let interface = match prompt.capability {
        PortalCapability::Capture => "org.freedesktop.impl.portal.ScreenCast",
        PortalCapability::InputInjection => "org.freedesktop.impl.portal.RemoteDesktop",
    };
    PortalRequest {
        interface: interface.to_owned(),
        caller: VerifiedCaller::new(0, prompt.app_id.clone()),
        prompt: format!("{}\n\n{}", prompt.title, prompt.message),
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        options: Fields::new(),
        parent_window: prompt.parent_window.clone().unwrap_or_default(),
    }
}

/// The thread that owns the connection.
pub struct IpcLoop<T: Transport> {
    client: Client<T>,
    queue: mpsc::UnboundedReceiver<Command>,
    /// The compositor's ticket for each prompt still awaiting an answer, and the request the guard knows it by.
    pending: HashMap<PortalTicket, RequestId>,
    /// The last request number this loop handed out.
    next_request: u64,
}

impl<T: Transport> std::fmt::Debug for IpcLoop<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IpcLoop")
            .field("version", &self.client.version())
            .finish_non_exhaustive()
    }
}

impl<T: Transport> IpcLoop<T> {
    /// The negotiated IPC version.
    #[must_use]
    pub fn version(&self) -> Version {
        self.client.version()
    }

    /// Serve the connection until it ends.
    ///
    /// `answer` is how the loop reaches the guard: the portal's handlers and this loop share
    /// one state, and a delivered answer or a settings diff is applied on the loop's thread
    /// rather than from a handler that is already waiting on something else.
    pub fn run(mut self, portal: crate::iface::Portal) {
        if let Err(error) = self.client.set_read_timeout(Some(READ_TIMEOUT)) {
            log::warn!(%error, "the IPC socket has no read timeout; offers will wait for an event");
        }
        if let Err(error) = self.client.subscribe(Filter::All) {
            // Without a subscription the settings interface publishes nothing, which is
            // visible and correct; the answers still work, so the loop continues.
            log::warn!(%error, "the portal cannot subscribe to the object graph");
        }
        loop {
            while let Ok(command) = self.queue.try_recv() {
                match command {
                    Command::Offer {
                        prompt,
                        timeout,
                        reply: sender,
                    } => {
                        let request = self.mint_request();
                        let answered = match self.client.portal_ui(&portal_request(&prompt, timeout)) {
                            Ok(ticket) => {
                                self.pending.insert(ticket, request);
                                Ok(Offer {
                                    request,
                                    provider: OFFERER,
                                })
                            }
                            Err(ClientError::Refused { code, message, .. })
                                if code == gnoblin_client::code::NO_PROVIDER =>
                            {
                                // The compositor knows who provides the slot; a slot
                                // nobody provides is refused here rather than turned into a
                                // wait for a deadline that ends as "the app was cancelled".
                                log::warn!(%message, slot = SLOT, "nobody claims the consent slot");
                                Err(Refusal::NoProvider { slot: SLOT })
                            }
                            Err(error) => {
                                log::warn!(%error, "a consent prompt could not be offered");
                                Err(Refusal::NoProvider { slot: SLOT })
                            }
                        };
                        let _ = sender.send(answered);
                    }
                    Command::Decline(request) => {
                        let held = self
                            .pending
                            .iter()
                            .find(|(_, held)| **held == request)
                            .map(|(ticket, _)| *ticket);
                        if let Some(ticket) = held {
                            self.pending.remove(&ticket);
                            if let Err(error) = self.client.portal_cancel(ticket) {
                                log::debug!(%error, request = request.get(), "an abandoned prompt was already over");
                            }
                        }
                    }
                }
            }
            match self.client.next_event() {
                Ok(Event::Model(observation)) => {
                    futures_lite::future::block_on(portal.observe(&observation));
                }
                Ok(Event::RequestAnswered { .. } | Event::RequestFinished { .. }) => {
                    // A portal.ui prompt settles through its ticket, read by `settle_outcomes`,
                    // so the request events for it carry nothing this loop needs.
                }
                Ok(Event::RequestOffered { .. } | Event::Extension { .. }) => {
                    // A request this connection could claim is not its business: the portal
                    // offers, it does not draw. An extension event goes only to the
                    // connections granted that extension, and the portal is granted none.
                }
                Err(ClientError::Idle) => {}
                Err(error) => {
                    log::info!(%error, "the IPC connection ended; the portal serves D-Bus only from here");
                    return;
                }
            }
            self.settle_outcomes(&portal);
        }
    }

    /// Apply one delivered answer to the guard.
    ///
    /// The answer's request is one this connection offered, so a failure here is a request
    /// that already expired or one the guard never offered; both are logged and neither
    /// wakes anything.
    fn answer(&mut self, portal: &crate::iface::Portal, request: RequestId, answer: Option<Vec<u8>>) {
        let outcome = {
            let mut shared = portal.lock();
            let now = shared.now();
            shared.guard.answer(now, request, answer.as_deref())
        };
        match outcome {
            Ok(outcome) => {
                log::info!(request = request.get(), ?outcome, "a consent answer arrived")
            }
            Err(error) => {
                log::debug!(%error, request = request.get(), "an answer arrived for no prompt")
            }
        }
    }

    /// Mint the identity the guard keys a prompt by. The compositor's ticket is its own
    /// handle and not a request id, so this loop numbers the prompts it offers.
    fn mint_request(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId::new(self.next_request)
    }

    /// Apply every portal outcome that has settled since the last pass.
    ///
    /// A `Handled` outcome is an allow and a `Declined` one a deny. A cancelled or failed
    /// prompt has no answer, so the guard releases its caller as it does for a request that
    /// finished without one.
    fn settle_outcomes(&mut self, portal: &crate::iface::Portal) {
        let tickets: Vec<PortalTicket> = self.pending.keys().copied().collect();
        for ticket in tickets {
            let Some(outcome) = self.client.take_portal_outcome(ticket) else {
                continue;
            };
            let Some(request) = self.pending.remove(&ticket) else {
                continue;
            };
            let answer = match outcome {
                Ok(PortalOutcome::Handled(_)) => Some(b"allow".to_vec()),
                Ok(PortalOutcome::Declined) => Some(b"deny".to_vec()),
                Ok(PortalOutcome::Cancelled(reason)) => {
                    log::debug!(%reason, request = request.get(), "a consent prompt was cancelled");
                    None
                }
                Ok(PortalOutcome::Error(reason)) => {
                    log::warn!(%reason, request = request.get(), "a consent prompt failed");
                    None
                }
                Err(error) => {
                    log::warn!(%error, request = request.get(), "a consent outcome could not be read");
                    None
                }
            };
            self.answer(portal, request, answer);
        }
    }
}
