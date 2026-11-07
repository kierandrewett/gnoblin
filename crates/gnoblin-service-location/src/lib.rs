//! The GeoClue agent: `org.freedesktop.GeoClue2.Agent`, the object GeoClue calls when an
//! application asks for its location.
//!
//! This crate is the only place that knows GeoClue's bus names. It registers with the
//! manager as `gnoblin`, answers `AuthorizeApp` by offering the request on the
//! `location.agent` slot, and grants the accuracy the shell answered, capped by policy.
//!
//! # Failure modes first
//!
//! 1. **GeoClue is absent.** Registration fails and the agent logs and skips; nothing
//!    else in the role depends on it.
//! 2. **No accuracy is granted without an answer.** An empty answer, an answer that is
//!    not a decimal level, a cancelled request and a timed-out request all deny. See
//!    [`grant`].
//! 3. **The answer exceeds policy.** The granted level is the largest GeoClue level at
//!    or below the requested level, the policy ceiling and the answer.
//! 4. **Policy is absent.** The ceiling defaults to [`Policy::DENY`], which denies every
//!    request.
//!
//! # Agent states
//!
//! The agent is `Offline` until GeoClue accepts its registration, and `Available` after.
//! `in_use` exists only while `Available`. Each transition is a method on `AgentState`
//! and every refused transition leaves the state unchanged.
//!
//! | from        | event                  | to          | notes                                   |
//! |-------------|------------------------|-------------|-----------------------------------------|
//! | `Offline`   | `start(in_use)`        | `Available` | registration accepted                   |
//! | `Available` | `start(in_use)`        | unchanged   | refused: already registered             |
//! | `Available` | `stop`                 | `Offline`   | GeoClue left the bus; `in_use` cleared  |
//! | `Offline`   | `stop`                 | unchanged   | nothing to withdraw                     |
//! | `Available` | `watch_in_use(in_use)` | `Available` | `InUse` changed                         |
//! | `Offline`   | `watch_in_use(in_use)` | unchanged   | refused: no registration to report on   |
//!
//! Offers are accepted only in `Available`. A `grant` is still [`grant`], a pure function of
//! the answer, the request and the policy ceiling.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use futures_channel::oneshot;
use futures_lite::StreamExt;
use gnoblin_config::{Inherit, LocationAccuracy, SearchPaths, Store};

use gnoblin_model::{RequestId, RequestOutcome, Tick};
use zbus::names::{BusName, WellKnownName};
use zbus::{Connection, fdo, proxy};

use gnoblin_service_core::agent::{Offerer, Proposition};

gnoblin_log::logger!(log = "location");

/// Where GeoClue calls the agent.
pub const AGENT_PATH: &str = "/org/freedesktop/GeoClue2/Agent";

/// The agent's interface, as GeoClue declares it.
pub const AGENT_INTERFACE: &str = "org.freedesktop.GeoClue2.Agent";

/// The manager's well-known name, which is also what the agent watches for.
pub const GEOCLUE_SERVICE: &str = "org.freedesktop.GeoClue2";

/// The manager's object path.
pub const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";

/// The id Gnoblin registers under with the manager.
pub const AGENT_ID: &str = "gnoblin";

/// The accuracy levels GeoClue defines, in ascending order.
pub const LEVELS: [u32; 6] = [0, 1, 4, 5, 6, 8];

/// The GeoClue manager, as the agent uses it.
#[proxy(
    interface = "org.freedesktop.GeoClue2.Manager",
    default_service = "org.freedesktop.GeoClue2",
    default_path = "/org/freedesktop/GeoClue2/Manager"
)]
trait Manager {
    fn add_agent(&self, id: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn in_use(&self) -> zbus::Result<bool>;
}

/// The accuracy ceiling the configuration sets for approved requests.
///
/// The ceiling is a GeoClue level. A ceiling of `0` denies every request, which is the
/// default, so a missing or unreadable policy fails closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// The greatest GeoClue level an approved request may receive.
    pub max_level: u32,
}

impl Policy {
    /// The policy that denies every request.
    pub const DENY: Policy = Policy { max_level: 0 };
}

impl Default for Policy {
    fn default() -> Policy {
        Policy::DENY
    }
}

/// The GeoClue level a configured `max-accuracy` names.
fn level_of(accuracy: LocationAccuracy) -> u32 {
    match accuracy {
        LocationAccuracy::Country => 1,
        LocationAccuracy::City => 4,
        LocationAccuracy::Neighborhood => 5,
        LocationAccuracy::Street => 6,
        LocationAccuracy::Exact => 8,
    }
}

/// The policy a `location` section asks for.
///
/// `enabled = false` denies every request whatever the cap says. An omitted or inherited
/// `enabled` is `false`, and an omitted or inherited `max-accuracy` is `exact`: those are
/// the schema defaults. Gnoblin does not read the system setting an inherited value would
/// follow, so it fails closed rather than guess at it.
pub fn policy(section: Option<&gnoblin_config::Location>) -> Policy {
    let enabled = section
        .and_then(|location| location.enabled.as_ref())
        .and_then(Inherit::value)
        .copied()
        .unwrap_or(false);
    if !enabled {
        return Policy::DENY;
    }
    let accuracy = section
        .and_then(|location| location.max_accuracy.as_ref())
        .and_then(Inherit::value)
        .copied()
        .unwrap_or(LocationAccuracy::Exact);
    Policy {
        max_level: level_of(accuracy),
    }
}

/// The policy the configuration file in the environment asks for.
///
/// Like the compositor's monitors section, this reads the last accepted document. With no
/// state directory, no file or an unreadable file, the policy is [`Policy::DENY`].
pub fn policy_from_environment() -> Policy {
    let Some(store) = Store::from_env() else {
        log::debug!(area = "services", "no state directory is set; location is denied");
        return Policy::DENY;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => policy_from_store(&store, path.path()),
        Err(error) => {
            log::debug!(area = "services", %error, "no configuration file; location is denied");
            Policy::DENY
        }
    }
}

/// [`policy_from_environment`] for the document at `source`, read through `store`.
pub fn policy_from_store(store: &Store, source: &Path) -> Policy {
    match store.load(source) {
        Ok(stored) => policy(stored.config.location.as_ref()),
        Err(error) => {
            log::warn!(area = "services", %error, "configuration not readable; location is denied");
            Policy::DENY
        }
    }
}

/// The largest GeoClue level at or below `level`, or `0` when there is none.
pub fn snap(level: u32) -> u32 {
    LEVELS.iter().rev().copied().find(|known| *known <= level).unwrap_or(0)
}

/// The accuracy level to grant for `outcome`, given the level the app asked for and the
/// policy ceiling.
///
/// Only an answer that is a decimal level grants anything, and the grant is the answer
/// snapped to a GeoClue level and capped by both the request and the ceiling. A grant of
/// `0` is a denial.
pub fn grant(outcome: &RequestOutcome, requested: u32, ceiling: u32) -> (bool, u32) {
    let RequestOutcome::Answered(bytes) = outcome else {
        return (false, 0);
    };
    let Some(answered) = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse::<u32>().ok())
    else {
        return (false, 0);
    };
    let level = snap(answered.min(requested).min(ceiling));
    if level == 0 { (false, 0) } else { (true, level) }
}

/// Whether GeoClue has accepted this agent, and the `InUse` value GeoClue last reported
/// while it has. See the module's agent-state table for the transitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeoClueAgentState {
    /// GeoClue has not accepted the registration: it is absent, refused the agent or has
    /// left the bus.
    Offline,
    /// GeoClue accepted the registration; `in_use` is its last `InUse` value.
    Available { in_use: bool },
}

impl GeoClueAgentState {
    /// The state after GeoClue accepts a registration. Already registered: unchanged.
    fn start(self, in_use: bool) -> GeoClueAgentState {
        match self {
            GeoClueAgentState::Offline => GeoClueAgentState::Available { in_use },
            available @ GeoClueAgentState::Available { .. } => available,
        }
    }

    /// The state after GeoClue goes away. Clears `in_use` with the registration.
    fn stop(self) -> GeoClueAgentState {
        GeoClueAgentState::Offline
    }

    /// The state after GeoClue reports `InUse`. Ignored while offline.
    fn watch_in_use(self, in_use: bool) -> GeoClueAgentState {
        match self {
            GeoClueAgentState::Offline => GeoClueAgentState::Offline,
            GeoClueAgentState::Available { .. } => GeoClueAgentState::Available { in_use },
        }
    }

    /// Whether this agent is registered and answers offers.
    fn is_available(self) -> bool {
        matches!(self, GeoClueAgentState::Available { .. })
    }

    /// Whether GeoClue reports a client using location. Always `false` while offline.
    fn in_use(self) -> bool {
        matches!(self, GeoClueAgentState::Available { in_use: true })
    }
}

/// The state the handlers and the watchers share.
struct LocationState {
    offerer: Box<dyn Offerer + Send>,
    waiting: BTreeMap<RequestId, oneshot::Sender<RequestOutcome>>,
    policy: Policy,
    state: GeoClueAgentState,
}

/// The agent object, clonable so the connection's object server and the watchers share
/// one state.
#[derive(Clone)]
pub struct Location {
    shared: Arc<Mutex<LocationState>>,
    deadline: Tick,
}

impl std::fmt::Debug for Location {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        formatter
            .debug_struct("Location")
            .field("state", &state.state)
            .field("waiting", &state.waiting.len())
            .finish()
    }
}

impl Location {
    /// An agent that offers its requests through `offerer` under `policy`, failing them at
    /// `deadline` if no provider answers.
    pub fn new<O: Offerer + Send + 'static>(offerer: O, policy: Policy, deadline: Tick) -> Location {
        Location {
            shared: Arc::new(Mutex::new(LocationState {
                offerer: Box::new(offerer),
                waiting: BTreeMap::new(),
                policy,
                state: GeoClueAgentState::Offline,
            })),
            deadline,
        }
    }

    /// The shared state, with a poisoned lock recovered: every transition is a map or flag
    /// edit under the lock, so the state stays consistent after a panic.
    fn lock(&self) -> MutexGuard<'_, LocationState> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether GeoClue has accepted the registration.
    pub fn is_available(&self) -> bool {
        self.lock().state.is_available()
    }

    /// Whether GeoClue reports that a client is using location right now.
    pub fn in_use(&self) -> bool {
        self.lock().state.in_use()
    }

    /// The ceiling this agent reports as `MaxAccuracyLevel` and enforces.
    pub fn ceiling(&self) -> u32 {
        self.lock().policy.max_level
    }

    /// Report how a request this agent offered ended. Returns whether this agent owned it.
    pub fn answer(&self, request: RequestId, outcome: &RequestOutcome) -> bool {
        let sender = self.lock().waiting.remove(&request);
        match sender {
            Some(sender) => {
                let _ = sender.send(outcome.clone());
                true
            }
            None => false,
        }
    }

    /// Offer one `AuthorizeApp` and wait for the shell's answer. Denies when the ceiling is
    /// zero, the agent is not registered, or no provider took the offer.
    async fn authorize(&self, desktop_id: &str, requested: u32) -> (bool, u32) {
        let (receiver, ceiling) = {
            let mut state = self.lock();
            let ceiling = state.policy.max_level;
            if ceiling == 0 || !state.state.is_available() {
                return (false, 0);
            }
            let proposition = Proposition::location(desktop_id, &format!("Allow {desktop_id} to use your location?"));
            let request = match state.offerer.offer(&proposition, self.deadline) {
                Ok(request) => request,
                Err(refusal) => {
                    log::warn!(
                        area = "services",
                        ?refusal,
                        app = desktop_id,
                        "location request refused by the offer path"
                    );
                    return (false, 0);
                }
            };
            let (sender, receiver) = oneshot::channel();
            state.waiting.insert(request, sender);
            (receiver, ceiling)
        };
        let outcome = receiver.await.unwrap_or(RequestOutcome::Ended);
        grant(&outcome, requested, ceiling)
    }

    /// Mark GeoClue as gone: deny everything waiting and stop answering until it returns.
    fn withdraw(&self) {
        let waiting = {
            let mut state = self.lock();
            state.state = state.state.stop();
            std::mem::take(&mut state.waiting)
        };
        for (_, sender) in waiting {
            let _ = sender.send(RequestOutcome::Ended);
        }
    }

    /// Register with GeoClue's manager, recording whether it accepted.
    async fn register(&self, connection: &Connection) -> zbus::Result<()> {
        let manager = ManagerProxy::new(connection).await?;
        manager.add_agent(AGENT_ID).await?;
        let in_use = manager.in_use().await.unwrap_or(false);
        let mut state = self.lock();
        state.state = state.state.start(in_use);
        Ok(())
    }

    /// Export the agent at [`AGENT_PATH`] and start watching GeoClue on `connection`.
    ///
    /// Returns once the object is exported; registration and watching run on the
    /// connection's executor. An absent GeoClue is logged and skipped, not an error.
    pub async fn start(&self, connection: &Connection) -> zbus::Result<()> {
        connection.object_server().at(AGENT_PATH, self.clone()).await?;
        let owner = self.clone();
        let connection_owner = connection.clone();
        let watcher = self.clone();
        let connection_watcher = connection.clone();
        // The role has no async runtime, so the supervision loop and the InUse watch share
        // one thread that blocks on both, as the role's polkit agent does.
        std::thread::Builder::new()
            .name("gnoblin-geoclue".into())
            .spawn(move || {
                futures_lite::future::block_on(futures_lite::future::zip(
                    owner.supervise(connection_owner),
                    watcher.watch_in_use(connection_watcher),
                ));
            })
            .map_err(|error| zbus::Error::Failure(error.to_string()))?;
        Ok(())
    }

    /// Register whenever the manager's name gains an owner, and withdraw when it loses one.
    async fn supervise(self, connection: Connection) {
        let dbus = match fdo::DBusProxy::new(&connection).await {
            Ok(dbus) => dbus,
            Err(error) => {
                log::info!(area = "services", %error, "no bus proxy, so no GeoClue agent");
                return;
            }
        };
        let mut owners = match dbus.receive_name_owner_changed_with_args(&[(0, GEOCLUE_SERVICE)]).await {
            Ok(owners) => owners,
            Err(error) => {
                log::info!(area = "services", %error, "GeoClue name watch unavailable, so no GeoClue agent");
                return;
            }
        };
        self.reconcile(&connection, &dbus).await;
        while owners.next().await.is_some() {
            self.reconcile(&connection, &dbus).await;
        }
    }

    /// Bring the registration in line with whether GeoClue owns its name now.
    async fn reconcile(&self, connection: &Connection, dbus: &fdo::DBusProxy<'_>) {
        let name = WellKnownName::from_static_str_unchecked(GEOCLUE_SERVICE);
        let owned = dbus.name_has_owner(BusName::WellKnown(name)).await.unwrap_or(false);
        if !owned {
            if self.is_available() {
                log::info!(area = "services", "GeoClue left the bus; location requests are denied");
            }
            self.withdraw();
            return;
        }
        if self.is_available() {
            return;
        }
        match self.register(connection).await {
            Ok(()) => log::info!(area = "services", "registered the GeoClue agent"),
            Err(error) => log::info!(area = "services", %error, "GeoClue refused the agent, so no location agent"),
        }
    }

    /// Follow GeoClue's `InUse` property for as long as the connection lives.
    async fn watch_in_use(self, connection: Connection) {
        let manager = match ManagerProxy::new(&connection).await {
            Ok(manager) => manager,
            Err(error) => {
                log::debug!(area = "services", %error, "no GeoClue manager proxy, so no InUse watch");
                return;
            }
        };
        let mut changes = manager.receive_in_use_changed().await;
        while let Some(change) = changes.next().await {
            match change.get().await {
                Ok(in_use) => {
                    let mut state = self.lock();
                    state.state = state.state.watch_in_use(in_use);
                }
                Err(error) => log::debug!(area = "services", %error, "unreadable GeoClue InUse change"),
            }
        }
    }
}

/// The agent's D-Bus interface.
#[zbus::interface(name = "org.freedesktop.GeoClue2.Agent")]
impl Location {
    /// Answer whether `desktop_id` may have its location at up to `req_accuracy_level`,
    /// and at what level.
    async fn authorize_app(&self, desktop_id: &str, req_accuracy_level: u32) -> (bool, u32) {
        self.authorize(desktop_id, req_accuracy_level).await
    }

    /// The ceiling Gnoblin applies to every request.
    #[zbus(property)]
    async fn max_accuracy_level(&self) -> u32 {
        self.ceiling()
    }
}

/// A GeoClue agent the services role links in, with the policy that caps what it grants.
///
/// It registers on the system bus when the role starts it. The policy is the default,
/// which denies every location request until the caller passes its own in.
pub struct LocationAgent {
    policy: Policy,
    location: Option<(Location, zbus::Connection)>,
}

impl LocationAgent {
    /// An agent that has not started: it serves nothing until the role starts it.
    pub fn new(policy: Policy) -> LocationAgent {
        LocationAgent { policy, location: None }
    }
}

impl gnoblin_service_core::LinkedAgent for LocationAgent {
    fn start(&mut self, host: &gnoblin_service_core::Host, deadline: Tick) -> bool {
        let connection = match futures_lite::future::block_on(zbus::Connection::system()) {
            Ok(connection) => connection,
            Err(error) => {
                log::info!(area = "services", %error, "no system bus, so no GeoClue agent");
                return false;
            }
        };
        let location = Location::new(host.clone(), self.policy, deadline);
        if let Err(error) = futures_lite::future::block_on(location.start(&connection)) {
            log::warn!(area = "services", %error, "the GeoClue agent could not be exported");
            return false;
        }
        self.location = Some((location, connection));
        true
    }

    fn answer(&self, request: RequestId, outcome: &RequestOutcome) {
        if let Some((location, _)) = &self.location {
            location.answer(request, outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::{Config, Value};

    fn location<const N: usize>(entries: [(&str, Value); N]) -> gnoblin_config::Location {
        let document = Value::table([("location", Value::table(entries))]);
        Config::from_value(document)
            .expect("the document is accepted")
            .location
            .expect("the section is present")
    }

    fn text(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    fn answered(text: &str) -> RequestOutcome {
        RequestOutcome::answered(text.as_bytes().to_vec())
    }

    #[test]
    fn an_answer_grants_the_level_it_names() {
        assert_eq!(grant(&answered("5"), 8, 8), (true, 5));
    }

    #[test]
    fn the_request_and_the_policy_both_cap_the_grant() {
        assert_eq!(grant(&answered("8"), 4, 8), (true, 4));
        assert_eq!(grant(&answered("8"), 8, 5), (true, 5));
    }

    #[test]
    fn a_grant_snaps_to_a_geoclue_level() {
        assert_eq!(grant(&answered("3"), 8, 8), (true, 1));
        assert_eq!(grant(&answered("7"), 8, 8), (true, 6));
    }

    #[test]
    fn an_empty_or_unreadable_answer_denies() {
        assert_eq!(grant(&answered(""), 8, 8), (false, 0));
        assert_eq!(grant(&answered("exact"), 8, 8), (false, 0));
        assert_eq!(grant(&RequestOutcome::answered(vec![0xff]), 8, 8), (false, 0));
    }

    #[test]
    fn an_ended_or_timed_out_request_denies() {
        assert_eq!(grant(&RequestOutcome::Ended, 8, 8), (false, 0));
        assert_eq!(grant(&RequestOutcome::TimedOut, 8, 8), (false, 0));
    }

    #[test]
    fn a_zero_ceiling_denies_even_a_full_answer() {
        assert_eq!(grant(&answered("8"), 8, 0), (false, 0));
    }

    #[test]
    fn the_default_policy_denies() {
        assert_eq!(Policy::default(), Policy::DENY);
        assert_eq!(snap(0), 0);
    }

    #[test]
    fn each_accuracy_cap_maps_to_its_geoclue_level() {
        let cases = [
            ("country", 1),
            ("city", 4),
            ("neighborhood", 5),
            ("street", 6),
            ("exact", 8),
        ];
        for (spelling, level) in cases {
            let section = location([("enabled", Value::Bool(true)), ("max-accuracy", text(spelling))]);
            assert_eq!(policy(Some(&section)), Policy { max_level: level }, "{spelling}");
        }
    }

    #[test]
    fn enabled_false_denies_whatever_the_cap_says() {
        let section = location([("enabled", Value::Bool(false)), ("max-accuracy", text("exact"))]);
        assert_eq!(policy(Some(&section)), Policy::DENY);
    }

    #[test]
    fn an_omitted_section_denies() {
        assert_eq!(policy(None), Policy::DENY);
    }

    #[test]
    fn an_omitted_leaf_takes_its_schema_default() {
        // `enabled` defaults to false, so a cap alone grants nothing.
        let capped = location([("max-accuracy", text("city"))]);
        assert_eq!(policy(Some(&capped)), Policy::DENY);
        // `max-accuracy` defaults to exact, so enabling alone grants the full level.
        let enabled = location([("enabled", Value::Bool(true))]);
        assert_eq!(policy(Some(&enabled)), Policy { max_level: 8 });
    }

    #[test]
    fn an_inherited_leaf_takes_its_schema_default() {
        let section = location([("enabled", text("inherit")), ("max-accuracy", text("inherit"))]);
        assert_eq!(policy(Some(&section)), Policy::DENY);
        let enabled = location([("enabled", Value::Bool(true)), ("max-accuracy", text("inherit"))]);
        assert_eq!(policy(Some(&enabled)), Policy { max_level: 8 });
    }

    #[test]
    fn a_bad_accuracy_is_refused_naming_the_leaf() {
        let document = Value::table([("location", Value::table([("max-accuracy", text("mouse"))]))]);
        let Err(report) = Config::from_value(document) else {
            panic!("a bad accuracy must be refused");
        };
        let names: Vec<String> = report
            .problems()
            .iter()
            .map(|problem| problem.path.to_string())
            .collect();
        assert!(names.iter().any(|name| name.contains("max-accuracy")), "{names:?}");
    }

    #[test]
    fn a_missing_configuration_file_denies() {
        let directory = tempfile::tempdir().expect("a directory");
        let store = Store::new(directory.path());
        let source = directory.path().join("config.toml");
        assert_eq!(policy_from_store(&store, &source), Policy::DENY);
    }

    #[test]
    fn starting_an_offline_agent_registers_it_with_the_reported_in_use() {
        assert_eq!(
            GeoClueAgentState::Offline.start(true),
            GeoClueAgentState::Available { in_use: true }
        );
        assert_eq!(
            GeoClueAgentState::Offline.start(false),
            GeoClueAgentState::Available { in_use: false }
        );
    }

    #[test]
    fn starting_a_registered_agent_keeps_its_registration() {
        let registered = GeoClueAgentState::Available { in_use: true };
        assert_eq!(registered.start(false), registered);
    }

    #[test]
    fn stopping_an_available_agent_takes_it_offline() {
        assert_eq!(
            GeoClueAgentState::Available { in_use: false }.stop(),
            GeoClueAgentState::Offline
        );
    }

    #[test]
    fn stopping_an_available_agent_clears_in_use() {
        assert!(!GeoClueAgentState::Available { in_use: true }.stop().in_use());
    }

    #[test]
    fn stopping_an_offline_agent_changes_nothing() {
        assert_eq!(GeoClueAgentState::Offline.stop(), GeoClueAgentState::Offline);
    }

    #[test]
    fn watching_in_use_updates_an_available_agent() {
        assert_eq!(
            GeoClueAgentState::Available { in_use: false }.watch_in_use(true),
            GeoClueAgentState::Available { in_use: true }
        );
        assert_eq!(
            GeoClueAgentState::Available { in_use: true }.watch_in_use(false),
            GeoClueAgentState::Available { in_use: false }
        );
    }

    #[test]
    fn watching_in_use_on_an_offline_agent_records_nothing() {
        assert_eq!(
            GeoClueAgentState::Offline.watch_in_use(true),
            GeoClueAgentState::Offline
        );
        assert!(!GeoClueAgentState::Offline.watch_in_use(true).in_use());
    }

    #[test]
    fn only_a_registered_agent_is_available_for_offers() {
        assert!(!GeoClueAgentState::Offline.is_available());
        assert!(GeoClueAgentState::Available { in_use: false }.is_available());
    }
}
