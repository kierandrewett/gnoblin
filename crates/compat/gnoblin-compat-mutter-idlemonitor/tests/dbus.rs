//! The adapter over a private bus, with the loopback as its fake IPC server.
//!
//! Every test starts its own `dbus-daemon` (see `support/bus.rs`), so nothing here can reach
//! the developer's session bus. The caller is a plain `zbus::Proxy`, which is what
//! `gsd-power`'s own proxy resolves to, and the idle time arrives the way it does in a
//! session: as a property diff on the `idle` object that the link folds.

mod support;

use futures_lite::StreamExt as _;
use gnoblin_client::{Loopback, Server};
use gnoblin_compat_core::{Mode, SupportMatrix};
use gnoblin_compat_mutter_idlemonitor::{
    BUS_NAME, CallerWatcher, INTERFACE, IdleError, IdleLink, IdleMonitorService, MATRIX, PATH, export,
};
use gnoblin_model::{ObjectId, TypeKey, Value};
use support::bus::Bus;
use support::{VERSION, graph, idle_properties, unlocked};
use zbus::{Connection, Proxy};

/// The message of a refused call, which is where the adapter's rule id lives.
fn refusal(error: zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(_, Some(detail), _) => detail,
        other => other.to_string(),
    }
}

/// A connection to the private bus at `address`.
async fn connect(address: &str) -> Connection {
    zbus::connection::Builder::address(address)
        .build()
        .await
        .expect("the private bus is up")
}

/// The caller's proxy, which is the interface as a consumer sees it.
async fn caller(connection: &Connection) -> Proxy<'static> {
    Proxy::new(connection, BUS_NAME, PATH, INTERFACE)
        .await
        .expect("the interface is exported")
}

/// Wait for one `WatchFired`, bounded so a missing signal fails the test instead of hanging.
async fn fired(stream: &mut zbus::proxy::SignalStream<'_>) -> Option<u32> {
    futures_lite::future::race(
        async {
            let message = stream.next().await?;
            let (id,): (u32,) = message.body().deserialize().ok()?;
            Some(id)
        },
        async {
            for _ in 0..20_000 {
                futures_lite::future::yield_now().await;
            }
            None
        },
    )
    .await
}

/// Whether a `WatchFired` arrives within a bounded number of yields.
async fn any_fired(stream: &mut zbus::proxy::SignalStream<'_>) -> bool {
    fired(stream).await.is_some()
}

/// One server, one link and one published counter: the fixture every test starts from.
struct Fixture {
    server: Server,
    link: IdleLink<Loopback>,
    idle: ObjectId,
}

impl Fixture {
    /// An idle object at zero and an unlocked session, read through the loopback.
    fn new() -> Fixture {
        let mut server = Server::new(graph(), VERSION);
        let idle = server
            .create(TypeKey::gnoblin("idle"), idle_properties(0))
            .expect("the idle type and its property are in the schema");
        server
            .create(TypeKey::gnoblin("lock"), unlocked())
            .expect("the lock type and its property are in the schema");
        let client = server.connect(VERSION).expect("the versions match");
        let link = IdleLink::connect(client).expect("the snapshot publishes a counter");
        Fixture { server, link, idle }
    }

    /// Publish whatever the link has folded.
    async fn publish(&mut self, service: &IdleMonitorService) {
        self.link.drain().expect("the graph's observations are readable");
        service.publish(self.link.state()).await.expect("the signal is emitted");
    }

    /// Move the counter to `idle_ms` and publish it.
    async fn idle_for(&mut self, service: &IdleMonitorService, idle_ms: u64) {
        self.server
            .set(self.idle, "idle_ms", Value::Uint(idle_ms))
            .expect("idle_ms is a declared uint property");
        self.publish(service).await;
    }
}

#[test]
fn get_idletime_answers_the_published_counter() {
    // The failure mode: the adapter answers with a clock of its own, so a caller's policy
    // fires on a different schedule from the compositor's screensaver.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IdleMonitorService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.idle_for(&service, 4_200).await;

        let proxy = caller(&connection).await;
        let idle: u64 = proxy
            .call_method("GetIdletime", &())
            .await
            .expect("GetIdletime is supported")
            .body()
            .deserialize()
            .expect("the reply is a uint64");
        assert_eq!(idle, 4_200, "the counter is the one the graph published");
    });
}

#[test]
fn a_watch_fires_when_the_counter_crosses() {
    // The failure mode: a watch never fires because the adapter only looks at the value it
    // was given when the watch was added, or it fires on every advance at the same value.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IdleMonitorService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;

        let proxy = caller(&connection).await;
        let mut stream = proxy
            .receive_signal("WatchFired")
            .await
            .expect("the caller subscribes to the signal");
        let id: u32 = proxy
            .call_method("AddIdleWatch", &(2_000u64,))
            .await
            .expect("AddIdleWatch is supported")
            .body()
            .deserialize()
            .expect("the reply is a watch id");

        fixture.idle_for(&service, 1_500).await;
        assert!(
            !any_fired(&mut stream).await,
            "a watch does not fire below its interval"
        );

        fixture.idle_for(&service, 2_500).await;
        assert_eq!(
            fired(&mut stream).await,
            Some(id),
            "the watch fires once the counter crosses its interval"
        );

        fixture.idle_for(&service, 2_500).await;
        assert!(
            !any_fired(&mut stream).await,
            "the same idle time does not fire the watch twice"
        );

        fixture.idle_for(&service, 0).await;
        fixture.idle_for(&service, 2_500).await;
        assert_eq!(
            fired(&mut stream).await,
            Some(id),
            "a second crossing after activity fires it again"
        );
    });
}

#[test]
fn an_active_watch_fires_once() {
    // The failure mode: a user-active watch fires on every activity, so a caller that
    // expects one resume event handles a burst of them.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IdleMonitorService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;

        let proxy = caller(&connection).await;
        let mut stream = proxy
            .receive_signal("WatchFired")
            .await
            .expect("the caller subscribes to the signal");
        let id: u32 = proxy
            .call_method("AddUserActiveWatch", &())
            .await
            .expect("AddUserActiveWatch is supported")
            .body()
            .deserialize()
            .expect("the reply is a watch id");

        fixture.idle_for(&service, 3_000).await;
        assert!(!any_fired(&mut stream).await, "the user is still away");
        fixture.idle_for(&service, 0).await;
        assert_eq!(fired(&mut stream).await, Some(id), "the user came back");
        assert_eq!(service.watch_count(), 0, "a fired active watch is gone");

        fixture.idle_for(&service, 3_000).await;
        fixture.idle_for(&service, 0).await;
        assert!(!any_fired(&mut stream).await, "a spent watch never fires again");
    });
}

#[test]
fn a_watch_belongs_to_its_caller() {
    // The failure mode: one process cancels another's idle policy, so a screensaver never
    // learns the session was idle. The id is the caller's, and so is the right to remove it.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let service_connection = connect(&address).await;
        let service = IdleMonitorService::new().expect("the matrix loads");
        export(&service_connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;

        let first = connect(&address).await;
        let second = connect(&address).await;
        let id: u32 = caller(&first)
            .await
            .call_method("AddIdleWatch", &(60_000u64,))
            .await
            .expect("AddIdleWatch is supported")
            .body()
            .deserialize()
            .expect("the reply is a watch id");

        let error = caller(&second)
            .await
            .call_method("RemoveWatch", &(id,))
            .await
            .expect_err("another caller cannot remove the watch");
        let message = refusal(error);
        assert!(
            message.contains("belongs to another caller"),
            "the refusal names the reason: {message}"
        );
        assert_eq!(service.watch_count(), 1, "the watch is still live");

        caller(&first)
            .await
            .call_method("RemoveWatch", &(id,))
            .await
            .expect("the owner removes it");
        assert_eq!(service.watch_count(), 0);
    });
}

#[test]
fn a_watch_ends_with_its_callers_connection() {
    // The failure mode the plan names for this adapter: a watch outlives its caller and
    // fires at a bus name nobody holds, so the next process given that identity inherits a
    // signal it never asked for. The observer is a second caller, because `WatchFired` is
    // broadcast and only a live listener can prove that nothing was emitted.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let service_connection = connect(&address).await;
        let service = IdleMonitorService::new().expect("the matrix loads");
        export(&service_connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let mut watcher = CallerWatcher::new(&service_connection)
            .await
            .expect("the bus reports ownership changes");

        let observer = connect(&address).await;
        let mut observed = caller(&observer)
            .await
            .receive_signal("WatchFired")
            .await
            .expect("the observer subscribes to the signal");

        let departing = connect(&address).await;
        let unique = departing
            .unique_name()
            .expect("a bus connection has a unique name")
            .to_string();
        let proxy = caller(&departing).await;
        proxy
            .call_method("AddIdleWatch", &(1_000u64,))
            .await
            .expect("AddIdleWatch is supported");
        assert_eq!(service.watch_count(), 1);

        // The proxy holds a clone of the connection, so both have to go before the bus sees
        // the unique name released.
        drop(proxy);
        drop(departing);
        let departed = futures_lite::future::race(async { watcher.next().await }, async {
            for _ in 0..20_000 {
                futures_lite::future::yield_now().await;
            }
            None
        })
        .await;
        assert_eq!(
            departed.as_deref(),
            Some(unique.as_str()),
            "the bus says which caller left"
        );
        assert_eq!(service.forget_owner(&unique), 1, "the watch went with the connection");
        assert_eq!(service.watch_count(), 0, "the registry is empty again");

        fixture.idle_for(&service, 5_000).await;
        assert!(
            !any_fired(&mut observed).await,
            "nothing fires for a caller that is gone"
        );
    });
}

#[test]
fn an_unsupported_member_is_refused_by_name() {
    // The failure mode: `ResetIdletime` answers as though it worked, and a caller believes
    // it reset a counter it cannot reach. Every `unsupported` member in the matrix answers
    // with the matrix's rule.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IdleMonitorService::new().expect("the matrix loads");
        service.set_mode(Mode::Unmediated);
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;

        let matrix = SupportMatrix::from_json(MATRIX).expect("the checked-in manifest is readable");
        let interface = matrix
            .interface(INTERFACE)
            .expect("the manifest declares the interface this adapter serves");
        let mut refused = 0;
        for member in interface.members() {
            if !matches!(member.support().word(), "unsupported") {
                continue;
            }
            let error = match member.name() {
                "ResetIdletime" => proxy.call_method("ResetIdletime", &()).await.unwrap_err(),
                other => panic!("the matrix declares `{other}` unsupported and this test does not call it"),
            };
            let message = refusal(error);
            assert!(
                message.contains("unsupported"),
                "`{}` is declared unsupported and must refuse with that rule, not `{message}`",
                member.name()
            );
            refused += 1;
        }
        assert!(refused > 0, "the matrix declares at least one unsupported member");
    });
}

#[test]
fn an_adapter_registers_once() {
    // The failure mode: a second export replaces the object behind a live name while the
    // first one's watches are still firing.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let connection = connect(&address).await;
        let service = IdleMonitorService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        let again = export(&connection, &service).await;
        assert!(
            matches!(again, Err(IdleError::Exported)),
            "the second export is refused, not a second object server entry"
        );
    });
}
