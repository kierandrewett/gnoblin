//! The GeoClue agent over a private bus: a fake manager registers it, a client calls
//! `AuthorizeApp`, and the offer is answered from the test.
//!
//! Every test starts its own `dbus-daemon` (see `gnoblin-service-testkit`), so nothing here can
//! reach the developer's system bus or a real GeoClue.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use futures_lite::future;
use gnoblin_model::{RequestId, RequestOutcome, Tick};
use gnoblin_service_core::{CapabilitySlot, Offerer, Proposition, Refusal};
use gnoblin_service_location::{AGENT_INTERFACE, AGENT_PATH, GEOCLUE_SERVICE, Location, MANAGER_PATH, Policy};
use gnoblin_service_testkit::Bus;
use zbus::connection::Builder;
use zbus::{Connection, Proxy, fdo};

/// What the fake manager was told.
#[derive(Debug, Default)]
struct ManagerState {
    agents: Mutex<Vec<String>>,
}

/// A GeoClue manager that only records `AddAgent`.
struct FakeManager {
    state: Arc<ManagerState>,
}

#[zbus::interface(name = "org.freedesktop.GeoClue2.Manager")]
impl FakeManager {
    fn add_agent(&self, id: &str) -> fdo::Result<()> {
        self.state
            .agents
            .lock()
            .expect("the manager lock is not poisoned")
            .push(id.to_owned());
        Ok(())
    }

    #[zbus(property)]
    fn in_use(&self) -> bool {
        false
    }
}

/// Serve a fake manager on `address` and return the connection that owns its name.
async fn fake_manager(address: &str, state: Arc<ManagerState>) -> Connection {
    Builder::address(address)
        .serve_at(MANAGER_PATH, FakeManager { state })
        .name(GEOCLUE_SERVICE)
        .build()
        .await
        .expect("the fake manager connects")
}

/// A provider that records each offer, so the test can answer it.
#[derive(Clone, Default)]
struct Host {
    offers: Arc<Mutex<Vec<(RequestId, Proposition)>>>,
    next: Arc<Mutex<u64>>,
}

impl Offerer for Host {
    fn offer(&mut self, proposition: &Proposition, _deadline: Tick) -> Result<RequestId, Refusal> {
        let mut next = self.next.lock().expect("the counter lock is not poisoned");
        *next += 1;
        let request = RequestId::new(*next);
        self.offers
            .lock()
            .expect("the offer lock is not poisoned")
            .push((request, proposition.clone()));
        Ok(request)
    }
}

/// Wait until `condition` holds, failing the test after a few seconds.
fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Start the agent on its own connection to `address`, with `policy`.
async fn start_agent(address: &str, host: Host, policy: Policy) -> (Location, Connection) {
    let connection = Builder::address(address).build().await.expect("the agent connects");
    let location = Location::new(host, policy, Tick::new(5_000));
    location.start(&connection).await.expect("the agent exports");
    (location, connection)
}

/// Call `AuthorizeApp` on the agent from a separate client, on its own thread, so the test
/// can answer the offer while the call waits.
fn authorize(address: String, agent: String, app: &'static str) -> thread::JoinHandle<(bool, u32)> {
    thread::spawn(move || {
        future::block_on(async move {
            let client = Builder::address(address.as_str())
                .build()
                .await
                .expect("the client connects");
            let proxy = Proxy::new(&client, agent.as_str(), AGENT_PATH, AGENT_INTERFACE)
                .await
                .expect("the agent proxy builds");
            proxy
                .call("AuthorizeApp", &(app, 8u32))
                .await
                .expect("AuthorizeApp answers")
        })
    })
}

/// The offer the agent made for the one pending request.
fn one_offer(host: &Host) -> (RequestId, Proposition) {
    wait_until("an offer", || !host.offers.lock().expect("the offer lock").is_empty());
    host.offers.lock().expect("the offer lock").remove(0)
}

#[test]
fn the_agent_registers_with_a_present_geoclue() {
    let bus = Bus::start();
    let address = bus.address().to_owned();
    let state = Arc::new(ManagerState::default());
    let _manager = future::block_on(fake_manager(&address, state.clone()));
    let (location, _connection) = future::block_on(start_agent(&address, Host::default(), Policy { max_level: 8 }));

    wait_until("the registration", || location.is_available());
    assert_eq!(
        state.agents.lock().expect("the manager lock").as_slice(),
        ["gnoblin".to_owned()]
    );
}

#[test]
fn the_agent_skips_when_geoclue_is_absent() {
    let bus = Bus::start();
    let address = bus.address().to_owned();
    let host = Host::default();
    let (location, connection) = future::block_on(start_agent(&address, host.clone(), Policy { max_level: 8 }));

    thread::sleep(Duration::from_millis(200));
    assert!(!location.is_available());
    let agent = connection.unique_name().expect("unique name").to_string();
    let answer = authorize(address, agent, "org.example.Absent")
        .join()
        .expect("the client thread");
    assert_eq!(answer, (false, 0));
    assert!(host.offers.lock().expect("the offer lock").is_empty());
}

#[test]
fn an_answered_request_grants_the_level_the_shell_names() {
    let bus = Bus::start();
    let address = bus.address().to_owned();
    let _manager = future::block_on(fake_manager(&address, Arc::new(ManagerState::default())));
    let host = Host::default();
    let (location, connection) = future::block_on(start_agent(&address, host.clone(), Policy { max_level: 8 }));
    wait_until("the registration", || location.is_available());

    let agent = connection.unique_name().expect("unique name").to_string();
    let call = authorize(address.clone(), agent, "org.example.Maps");
    let (request, proposition) = one_offer(&host);
    assert_eq!(proposition.slot, CapabilitySlot::LocationAgent);
    assert_eq!(proposition.request.action_id, "org.example.Maps");
    assert!(location.answer(request, &RequestOutcome::Answered(b"5".to_vec())));

    assert_eq!(call.join().expect("the client thread"), (true, 5));
}

#[test]
fn an_empty_or_ended_request_is_denied() {
    let bus = Bus::start();
    let address = bus.address().to_owned();
    let _manager = future::block_on(fake_manager(&address, Arc::new(ManagerState::default())));
    let host = Host::default();
    let (location, connection) = future::block_on(start_agent(&address, host.clone(), Policy { max_level: 8 }));
    wait_until("the registration", || location.is_available());
    let agent = connection.unique_name().expect("unique name").to_string();

    let empty = authorize(address.clone(), agent.clone(), "org.example.Empty");
    let (request, _) = one_offer(&host);
    location.answer(request, &RequestOutcome::Answered(Vec::new()));
    assert_eq!(empty.join().expect("the client thread"), (false, 0));

    let ended = authorize(address.clone(), agent, "org.example.Ended");
    let (request, _) = one_offer(&host);
    location.answer(request, &RequestOutcome::Ended);
    assert_eq!(ended.join().expect("the client thread"), (false, 0));
}

#[test]
fn a_zero_policy_denies_without_offering() {
    let bus = Bus::start();
    let address = bus.address().to_owned();
    let _manager = future::block_on(fake_manager(&address, Arc::new(ManagerState::default())));
    let host = Host::default();
    let (location, connection) = future::block_on(start_agent(&address, host.clone(), Policy::DENY));
    wait_until("the registration", || location.is_available());
    let agent = connection.unique_name().expect("unique name").to_string();

    let answer = authorize(address, agent, "org.example.Denied")
        .join()
        .expect("the client thread");
    assert_eq!(answer, (false, 0));
    assert!(host.offers.lock().expect("the offer lock").is_empty());
}
