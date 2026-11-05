//! The services role: the D-Bus agents, in their own process.
//!
//! Design/rust-compositor.md, "Process model": the services role owns "polkit agent,
//! GeoClue agent, passphrase prompts (`prompt.secret`), PipeWire privacy monitor,
//! `org.freedesktop.ScreenSaver`, idle, inhibit", it is restartable, and "requests fail
//! closed while it is down". This module is the role `gnoblin --role services` plays:
//!
//! 1. Adopt the privileged connection the guardian passed. Its grant is `offer-request`,
//!    which is what lets this process put a request in front of a provider.
//! 2. Claim the slots the plan gives the services role a default for, so the `session`
//!    object publishes who provides them.
//! 3. Serve the agents whose bus and bus name are available — the polkit agent on the
//!    system bus, and each agent the caller links in — reporting a missing or refused
//!    one and carrying on. A session with no polkitd still has a compositor.
//! 4. Pump the privileged connection: offers that arrive for a slot this role provides,
//!    the answers to its own offers, and the ends of both.
//!
//! # Nothing here is fatal except the connection
//!
//! A missing bus, a name another process owns and a polkitd that refuses the registration
//! are all logged and skipped. The connection is the exception: without it this process
//! cannot offer anything, so there is nothing left for it to do and the guardian's restart
//! ladder decides what happens next.
//!
//! # A slot this role provides but cannot draw
//!
//! The plan gives the services role the default provider of `auth.polkit`,
//! `location.agent` and `idle.policy`, and this process draws no UI. An offer that reaches
//! it therefore ends as declined at once, which its host reports as "the provider would not
//! answer": that is the fail-closed answer for a session with no shell, and it is
//! deliberately not a wait until the deadline.
//!
//! # Failure modes first
//!
//! 1. **The guardian did not pass a connection.** The role refuses at once, naming the
//!    variable, so the guardian's log says what is wrong:
//!    `tests::the_role_refuses_without_a_privileged_connection`.
//! 2. **An offer arrives for a slot this role provides and cannot draw.** It is claimed and
//!    declined immediately, so the host is not left waiting:
//!    `tests::an_offer_the_role_cannot_draw_is_declined_at_once`.
//! 3. **A linked agent's name or the polkit bus is unavailable.** The role logs it and serves
//!    what is left, because an agent a session can live without must not end it:
//!    `tests::a_missing_bus_does_not_end_the_role`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_lite::future::block_on;
use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, Environment, Inherited, Reporter, inherited_stream, privilege};
use gnoblin_model::{CapabilitySlot, RequestCompletion, RequestId, RequestOutcome, Tick};
use gnoblin_types::Version;
use tracing::{info, warn};
use zbus::Connection;

use crate::agent::{Proposition, Refusal};
use crate::polkit::{AGENT_PATH, Polkit};
use crate::slot::Provider;

/// How long one read waits before the role looks at its own signals.
pub const PUMP_TIMEOUT: Duration = Duration::from_millis(100);

/// How long a prompt or an authentication stands before it fails as timed out.
///
/// Two minutes is `gcr-prompter`'s own deadline and polkit's default; the compositor owns
/// the deadline, so this is the number the offer asks for.
pub const PROMPT_DEADLINE: Duration = Duration::from_secs(120);

/// Why the services role could not run.
#[derive(Debug)]
pub enum RoleError {
    /// The privileged connection the guardian passes was missing or unusable.
    Connection(Inherited),
    /// The handshake with the compositor failed.
    Connect(String),
    /// Claiming a slot the plan gives this role failed.
    Claim(String),
}

impl std::fmt::Display for RoleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoleError::Connection(error) => {
                write!(formatter, "the services role needs its privileged connection: {error}")
            }
            RoleError::Connect(reason) | RoleError::Claim(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for RoleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RoleError::Connection(error) => Some(error),
            _ => None,
        }
    }
}

/// Which agents the role tries to serve.
///
/// A test that only wants the request path skips both buses, so it never talks to the
/// machine's own session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Buses {
    /// Serve both agents when their buses and names are available.
    #[default]
    Both,
    /// Serve neither, which is what a test about the request path asks for.
    None,
}

/// How the role runs, for a caller that is not the process itself.
pub struct Role {
    /// The environment to read the inherited descriptors from.
    pub env: Environment,
    /// How long an offer this role makes stands.
    pub deadline: Duration,
    /// Which agents to serve.
    pub buses: Buses,
}

impl Role {
    /// The role as the process finds it.
    pub fn from_environment() -> Role {
        Role {
            env: Box::new(|name| std::env::var(name).ok()),
            deadline: PROMPT_DEADLINE,
            buses: Buses::Both,
        }
    }

    /// Run the role until the guardian asks it to stop or the connection ends.
    ///
    /// # Errors
    ///
    /// [`RoleError`] for each documented failure mode.
    pub fn run(self, stop: impl Fn() -> bool, linked: Vec<Box<dyn LinkedAgent>>) -> Result<(), RoleError> {
        let mut reporter = Reporter::new(&self.env);
        let stream = inherited_stream(privilege::FD_VARIABLE, &self.env)
            .map_err(RoleError::Connection)?
            .ok_or(RoleError::Connection(Inherited::Absent))?;
        let transport = UnixTransport::from_stream(stream).map_err(|error| {
            RoleError::Connection(Inherited::NotASocket {
                number: -1,
                reason: error.to_string(),
            })
        })?;
        let mut client =
            Client::connect(transport, client_version()).map_err(|error| RoleError::Connect(error.to_string()))?;

        // The slots this role provides by default, claimed explicitly so the session
        // object publishes the answer instead of leaving it to be inferred.
        let holding: Vec<CapabilitySlot> = CapabilitySlot::ALL
            .into_iter()
            .filter(|slot| slot.default_provider() == Provider::Services)
            .collect();
        for slot in &holding {
            client
                .claim_slot(slot.name(), Provider::Services)
                .map_err(|error| RoleError::Claim(error.to_string()))?;
        }
        let host = Host::new(client, self.deadline);
        // From here on a read that times out is "nothing yet", which is what lets the role
        // check its own signals in the same loop.
        host.set_read_timeout(Some(PUMP_TIMEOUT));

        let agents = Agents::start(&host, self.deadline, self.buses, linked);
        reporter.ready(&[]);
        info!(
            slots = ?holding.iter().map(|slot| slot.name()).collect::<Vec<_>>(),
            polkit = agents.polkit.is_some(),
            linked = agents.linked.len(),
            "[services] serving"
        );

        while !stop() {
            match host.next_event() {
                Ok(event) => agents.dispatch(event, &host),
                Err(ClientError::Idle) => continue,
                Err(ClientError::Disconnected) => break,
                Err(error) => {
                    warn!(%error, "[services] the privileged connection failed");
                    break;
                }
            }
        }
        agents.shutdown();
        Ok(())
    }
}

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
    fn new(client: Client<UnixTransport>, deadline: Duration) -> Host {
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
    fn set_read_timeout(&self, timeout: Option<Duration>) {
        let _ = self.lock().set_read_timeout(timeout);
    }

    /// The next event the connection delivered.
    fn next_event(&self) -> Result<gnoblin_client::Event, ClientError> {
        self.lock().next_event()
    }

    /// Claim a request this role was offered, so it can end it.
    fn claim(&self, request: RequestId) {
        if let Err(error) = self.lock().claim(request) {
            warn!(%error, request = request.get(), "[services] cannot claim an offer");
        }
    }

    /// Decline a request this role claimed: it draws nothing, so it cannot answer.
    fn decline(&self, request: RequestId) {
        if let Err(error) = self.lock().decline(request) {
            warn!(%error, request = request.get(), "[services] cannot decline an offer");
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

impl crate::Offerer for Host {
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

/// The agents this process is serving, when their buses were available.
#[derive(Default)]
struct Agents {
    polkit: Option<(Polkit, Connection)>,
    linked: Vec<Box<dyn LinkedAgent>>,
}

impl Agents {
    /// Start each agent whose bus and name are available, reporting the ones that are not.
    fn start(host: &Host, deadline: Duration, buses: Buses, linked: Vec<Box<dyn LinkedAgent>>) -> Agents {
        if buses == Buses::None {
            return Agents::default();
        }
        let tick = Tick::new(u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX));
        Agents {
            polkit: start_polkit(host, tick),
            linked: start_linked(host, tick, linked),
        }
    }

    /// Deliver one event to whichever agent owns the request it names.
    fn dispatch(&self, event: gnoblin_client::Event, host: &Host) {
        match event {
            gnoblin_client::Event::Model(_) => {
                // The services role mirrors no object state: it acts on requests, and the
                // graph it would subscribe to is the shells' and `gnoblinctl`'s.
            }
            gnoblin_client::Event::RequestOffered { request, slot, .. } => {
                // This role provides the slot and draws nothing, so the offer ends here
                // rather than waiting for a deadline nobody will meet.
                info!(request = request.get(), %slot, "[services] declining an offer this role cannot draw");
                host.claim(request);
                host.decline(request);
            }
            gnoblin_client::Event::RequestAnswered { request, answer } => {
                self.answer(request, &RequestOutcome::Answered(answer));
            }
            gnoblin_client::Event::RequestFinished { request, completion } => {
                // An answered request already delivered its value; only the ends without
                // one are news to an agent.
                match completion {
                    RequestCompletion::Answered => {}
                    RequestCompletion::Ended => self.answer(request, &RequestOutcome::Ended),
                    RequestCompletion::TimedOut => self.answer(request, &RequestOutcome::TimedOut),
                }
            }
        }
    }

    /// Report how a request this role offered ended.
    ///
    /// Both agents are told and the one that did not offer it refuses with
    /// `unknown-request`, which is the cheapest correct answer: neither registry knows
    /// which slot another agent's request came from.
    fn answer(&self, request: RequestId, outcome: &RequestOutcome) {
        if let Some((polkit, _)) = &self.polkit
            && let Err(error) = polkit.answer(request, outcome)
        {
            tracing::debug!(%error, request = request.get(), "[services] polkit did not own the answer");
        }
        for agent in &self.linked {
            agent.answer(request, outcome);
        }
    }

    /// Give the agents back: polkitd stops calling this process only once it is told.
    fn shutdown(&self) {
        if let Some((polkit, connection)) = &self.polkit {
            block_on(polkit.unregister_registration(connection));
        }
    }
}

/// The polkit agent, when the system bus is reachable and accepts the registration.
fn start_polkit(host: &Host, deadline: Tick) -> Option<(Polkit, Connection)> {
    let connection = match block_on(Connection::system()) {
        Ok(connection) => connection,
        Err(error) => {
            info!(%error, "[services] no system bus, so no polkit agent");
            return None;
        }
    };
    let polkit = Polkit::new(host.clone(), deadline);
    let exported = block_on(async { connection.object_server().at(AGENT_PATH, polkit.clone()).await });
    if let Err(error) = exported {
        warn!(%error, "[services] the polkit agent could not be exported");
        return None;
    }
    if let Err(error) = block_on(polkit.register(&connection)) {
        warn!(%error, "[services] polkitd refused the agent");
        return None;
    }
    Some((polkit, connection))
}

/// Start each linked agent, keeping the ones whose bus and name were available.
fn start_linked(host: &Host, deadline: Tick, linked: Vec<Box<dyn LinkedAgent>>) -> Vec<Box<dyn LinkedAgent>> {
    linked
        .into_iter()
        .filter_map(|mut agent| agent.start(host, deadline).then_some(agent))
        .collect()
}

/// The IPC version this build advertises, from the schema both sides are built from.
fn client_version() -> Version {
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Version::new(major, minor, patch)
}
