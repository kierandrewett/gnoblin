//! The connection every service offers through, and the seam a linked agent serves from.
//!
//! The services role adopts one privileged connection, and every agent it serves (polkit,
//! GeoClue, ScreenSaver, and any agent a build links in) offers its requests over that same
//! connection. [`Host`] is that connection behind a lock, and [`LinkedAgent`] is what the
//! role asks of an agent it starts.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError};

use gnoblin_model::{Fields, Filter, OperationOutcome, RequestId, RequestOutcome, Snapshot, Tick};
use gnoblin_types::ObjectId;

use crate::agent::{Offerer, Proposition, Refusal};

use crate::log;

/// The offerer the agents share: one privileged connection, one frame stream.
///
/// The trait the agents use takes `&mut self`, which is why the connection is behind a
/// lock: the traits are the agents' seam and their signatures are not this crate's to
/// change, and one connection is what keeps an offer and its answer on the same socket.
#[derive(Clone, Debug)]
pub struct Host {
    client: Arc<Mutex<Client<UnixTransport>>>,
    deadline: Duration,
}

impl Host {
    /// A host that offers over `client`.
    pub fn new(client: Client<UnixTransport>, deadline: Duration) -> Host {
        Host {
            client: Arc::new(Mutex::new(client)),
            deadline,
        }
    }

    /// The connection, with a poisoned lock recovered rather than propagated.
    fn lock(&self) -> MutexGuard<'_, Client<UnixTransport>> {
        self.client.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Give the connection the pump's read deadline, so an idle read is "nothing yet".
    pub fn set_read_timeout(&self, timeout: Option<Duration>) {
        let _ = self.lock().set_read_timeout(timeout);
    }

    /// The next event the connection delivered.
    pub fn next_event(&self) -> Result<gnoblin_client::Event, ClientError> {
        self.lock().next_event()
    }

    /// A snapshot of the compositor's objects, which the screensaver service reads.
    pub fn snapshot(&self) -> Result<Snapshot, ClientError> {
        self.lock().subscribe(Filter::All)
    }

    /// Calls a declared method on an object and waits for the operation's outcome.
    pub fn call_operation(&self, object: ObjectId, method: &str) -> Result<OperationOutcome, ClientError> {
        self.call_operation_with(object, method, &Fields::new())
    }

    /// Calls a declared method with `arguments` and waits for the operation's outcome.
    pub fn call_operation_with(
        &self,
        object: ObjectId,
        method: &str,
        arguments: &Fields,
    ) -> Result<OperationOutcome, ClientError> {
        let mut client = self.lock();
        let operation = client.call(object, method, arguments)?;
        client.await_operation(operation)
    }

    /// Claim a request this role was offered, so it can end it.
    pub fn claim(&self, request: RequestId) {
        if let Err(error) = self.lock().claim(request) {
            log::warn!(%error, request = request.get(), "[services] cannot claim an offer");
        }
    }

    /// Decline a request this role claimed: it draws nothing, so it cannot answer.
    pub fn decline(&self, request: RequestId) {
        if let Err(error) = self.lock().decline(request) {
            log::warn!(%error, request = request.get(), "[services] cannot decline an offer");
        }
    }

    /// Offer one request on `slot` to whoever provides it.
    pub fn offer(&self, slot: &str, title: &str, message: &str, deadline: Tick) -> Result<RequestId, OfferFailure> {
        // The agent's deadline is what it told the compositor to enforce; the role's own
        // default is the floor, because an agent with no deadline of its own must still
        // get one.
        let timeout = Duration::from_millis(deadline.get()).max(self.deadline);
        match self.lock().offer(slot, title, message, timeout) {
            Ok(request) => Ok(request),
            Err(ClientError::Refused { code, message, .. }) if code == gnoblin_client::code::NO_PROVIDER => {
                Err(OfferFailure::NoProvider { reason: message })
            }
            Err(error) => Err(OfferFailure::Refused {
                reason: error.to_string(),
            }),
        }
    }
}

/// Why an offer could not be made.
#[derive(Debug)]
pub enum OfferFailure {
    /// Nothing provides the slot.
    NoProvider {
        /// What the compositor said, which names the slot.
        reason: String,
    },
    /// The connection refused or failed.
    Refused {
        /// What went wrong.
        reason: String,
    },
}

impl Offerer for Host {
    fn offer(&mut self, proposition: &Proposition, deadline: Tick) -> Result<RequestId, Refusal> {
        // A polkit request has no title of its own: the action id is the one name polkitd
        // gives it, and the message is the sentence it wants shown.
        let title = proposition.request.action_id.clone();
        match Host::offer(
            self,
            proposition.slot.name(),
            &title,
            &proposition.request.message,
            deadline,
        ) {
            Ok(request) => Ok(request),
            Err(OfferFailure::NoProvider { reason }) => Err(Refusal::NoProvider {
                slot: proposition.slot,
                reason,
            }),
            Err(OfferFailure::Refused { reason }) => Err(Refusal::Connection { reason }),
        }
    }
}

/// An agent the caller links into the services role.
///
/// The role owns the privileged connection and the loop that pumps it. An agent owns one
/// bus registration and the work that serves it, and offers through the [`Host`] it is
/// started with. The caller decides which agents a build links, so this crate names none.
pub trait LinkedAgent {
    /// Start serving on `host`. Returns `false` when the agent's bus or name is
    /// unavailable, after logging why: the role serves what is left.
    fn start(&mut self, host: &Host, deadline: Tick) -> bool;

    /// Report how a request this agent offered ended. Other requests are not its own.
    fn answer(&self, request: RequestId, outcome: &RequestOutcome);
}
