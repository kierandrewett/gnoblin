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

use std::time::Duration;

use futures_lite::future::block_on;
use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, Environment, Inherited, Reporter, inherited_stream, privilege};

use gnoblin_model::{CapabilitySlot, RequestCompletion, RequestId, RequestOutcome, Tick};
use gnoblin_service_core::{Host, LinkedAgent, Provider};
use gnoblin_service_polkit::{AGENT_PATH, Polkit};
use gnoblin_types::Version;
use zbus::Connection;

use gnoblin_service_screensaver::ScreenSaver;
use gnoblin_service_sessionidle::SessionIdle;

use crate::log;

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
        log::info!(slots = ?holding.iter().map(|slot| slot.name()).collect::<Vec<_>>(),
            polkit = agents.polkit.describe(),
            screensaver = agents.screensaver.is_some(),
            session_idle = agents.session_idle.is_some(),
            linked = agents.linked.len(),
            "serving"
        );

        while !stop() {
            match host.next_event() {
                Ok(event) => agents.dispatch(event, &host),
                Err(ClientError::Idle) => continue,
                Err(ClientError::Disconnected) => break,
                Err(error) => {
                    log::warn!(%error, "the privileged connection failed");
                    break;
                }
            }
        }
        agents.shutdown();
        Ok(())
    }
}

/// The agents this process is serving, when their buses were available.
#[derive(Default)]
struct Agents {
    polkit: PolkitSlot,
    screensaver: Option<(ScreenSaver, Connection)>,
    /// Served on the ScreenSaver connection, so it exists only when that agent does.
    session_idle: Option<SessionIdle>,
    linked: Vec<Box<dyn LinkedAgent>>,
}

impl Agents {
    /// Start each agent whose bus and name are available, reporting the ones that are not.
    fn start(host: &Host, deadline: Duration, buses: Buses, linked: Vec<Box<dyn LinkedAgent>>) -> Agents {
        if buses == Buses::None {
            return Agents::default();
        }
        let tick = Tick::new(u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX));
        let screensaver = gnoblin_service_screensaver::start(host);
        // The idle object is read once here, which is the subscription that keeps its
        // observations coming on this connection; `dispatch` folds them from then on.
        let session_idle = screensaver
            .as_ref()
            .and_then(|(_, connection)| gnoblin_service_sessionidle::serve(host, connection));
        Agents {
            polkit: start_polkit(host, tick),
            screensaver,
            session_idle,
            linked: start_linked(host, tick, linked),
        }
    }

    /// Deliver one event to whichever agent owns the request it names.
    fn dispatch(&self, event: gnoblin_client::Event, host: &Host) {
        match event {
            gnoblin_client::Event::Model(observation) => {
                // The services role mirrors object state only for the agents that serve
                // it: the idle object feeds SessionIdle, and nothing else is kept here.
                if let Some(session_idle) = &self.session_idle
                    && let Some(activity) = session_idle.observe(&observation)
                {
                    session_idle.publish(activity);
                }
            }
            gnoblin_client::Event::RequestOffered { request, slot, .. } => {
                // This role provides the slot and draws nothing, so the offer ends here
                // rather than waiting for a deadline nobody will meet.
                log::info!(request = request.get(), %slot, "declining an offer this role cannot draw");
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
            // An extension event reaches only the connections a loaded extension grants,
            // and the services role is not one of them.
            gnoblin_client::Event::Extension { .. } => {}
        }
    }

    /// Report how a request this role offered ended.
    ///
    /// Both agents are told and the one that did not offer it refuses with
    /// `unknown-request`, which is the cheapest correct answer: neither registry knows
    /// which slot another agent's request came from.
    fn answer(&self, request: RequestId, outcome: &RequestOutcome) {
        if let PolkitSlot::Running(polkit, _) = &self.polkit
            && let Err(error) = polkit.answer(request, outcome)
        {
            log::debug!(%error, request = request.get(), "polkit did not own the answer");
        }
        for agent in &self.linked {
            agent.answer(request, outcome);
        }
    }

    /// Give the agents back: polkitd stops calling this process only once it is told.
    fn shutdown(&self) {
        if let PolkitSlot::Running(polkit, connection) = &self.polkit {
            block_on(polkit.unregister_registration(connection));
        }
    }
}

/// The polkit agent's lifecycle in this role.
///
/// `Absent` when there is no system bus to serve on. `Failed` when the agent could not be
/// exported or polkitd refused its registration, and keeps the reason for the log. Only
/// `Running` answers requests and unregisters on shutdown.
#[derive(Default)]
enum PolkitSlot {
    /// No system bus, or the role was started without buses: nothing to serve.
    #[default]
    Absent,
    /// The agent could not be exported, or polkitd refused its registration.
    Failed(String),
    /// Exported and registered with polkitd.
    Running(Polkit, Connection),
}

impl PolkitSlot {
    /// What the serving log reports for this slot.
    fn describe(&self) -> String {
        match self {
            PolkitSlot::Absent => "absent".to_owned(),
            PolkitSlot::Failed(reason) => format!("failed: {reason}"),
            PolkitSlot::Running(..) => "running".to_owned(),
        }
    }
}

/// The polkit agent, when the system bus is reachable and accepts the registration.
fn start_polkit(host: &Host, deadline: Tick) -> PolkitSlot {
    let connection = match block_on(Connection::system()) {
        Ok(connection) => connection,
        Err(error) => {
            log::info!(%error, "no system bus, so no polkit agent");
            return PolkitSlot::Absent;
        }
    };
    let polkit = Polkit::new(host.clone(), deadline);
    let exported = block_on(async { connection.object_server().at(AGENT_PATH, polkit.clone()).await });
    if let Err(error) = exported {
        log::warn!(%error, "the polkit agent could not be exported");
        return PolkitSlot::Failed(error.to_string());
    }
    if let Err(error) = block_on(polkit.register(&connection)) {
        log::warn!(%error, "polkitd refused the agent");
        return PolkitSlot::Failed(error.to_string());
    }
    PolkitSlot::Running(polkit, connection)
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
