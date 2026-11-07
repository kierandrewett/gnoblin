//! The SessionIdle service over a private bus: a real D-Bus client calls `GetActivity` and
//! subscribes to `ActivityChanged`, while the test feeds the service the observations the
//! services role would deliver.
//!
//! Every test starts its own `dbus-daemon` (see `gnoblin-service-testkit`), so nothing here can
//! reach the developer's session bus. The compositor is the [`Compositor`] seam, a fake that
//! answers with whatever idle object the test sets.

use futures_lite::StreamExt;
use gnoblin_model::{Fields, ObjectId, ObjectView, Observation, Revision, TypeKey, Value};
use gnoblin_service_sessionidle::{Activity, Compositor, PATH, Reading, SessionIdle, THRESHOLD_MS, serve};
use gnoblin_service_testkit::Bus;
use zbus::connection::Builder;
use zbus::{Connection, Proxy};

const INTERFACE: &str = "org.gnoblin.SessionIdle";

/// A compositor that answers with a fixed idle object, or with a fixed failure.
struct Fake(Result<Option<Reading>, String>);

impl Compositor for Fake {
    fn idle(&self) -> Result<Option<Reading>, String> {
        self.0.clone()
    }
}

struct Fixture {
    _bus: Bus,
    server: Connection,
    client: Connection,
    service: SessionIdle,
}

async fn connect(address: &str) -> Connection {
    Builder::address(address).build().await.expect("the private bus is up")
}

/// Serve the interface on a server connection, seeded from `reading`, and connect a client.
fn served(reading: Option<Reading>) -> Fixture {
    let bus = Bus::start();
    let server = zbus::block_on(connect(bus.address()));
    let client = zbus::block_on(connect(bus.address()));
    let service = serve(&Fake(Ok(reading)), &server).expect("the service is served");
    Fixture {
        _bus: bus,
        server,
        client,
        service,
    }
}

fn reading(object: u64, idle_ms: u64) -> Reading {
    Reading {
        object: ObjectId::new(object),
        idle_ms,
    }
}

fn opened(object: u64, idle_ms: u64) -> Observation {
    let mut properties = Fields::new();
    properties.insert("idle_ms".to_owned(), Value::Uint(idle_ms));
    Observation::Opened {
        view: ObjectView::new(ObjectId::new(object), TypeKey::gnoblin("idle"), properties),
        revision: Revision::new(1),
    }
}

fn changed(object: u64, idle_ms: u64) -> Observation {
    Observation::Changed {
        object: ObjectId::new(object),
        property: "idle_ms".to_owned(),
        value: Value::Uint(idle_ms),
        revision: Revision::new(2),
    }
}

fn closed(object: u64) -> Observation {
    Observation::Closed {
        object: ObjectId::new(object),
        revision: Revision::new(3),
    }
}

/// The `(available, idle, threshold_ms, idle_for_ms)` a client reads from `GetActivity`.
fn get_activity(fixture: &Fixture) -> (bool, bool, u64, u64) {
    let destination = fixture
        .server
        .unique_name()
        .expect("a connected server has a unique name")
        .to_string();
    zbus::block_on(async {
        let proxy = Proxy::new(&fixture.client, destination.as_str(), PATH, INTERFACE)
            .await
            .expect("the proxy is built");
        proxy
            .call_method("GetActivity", &())
            .await
            .expect("GetActivity answers")
            .body()
            .deserialize()
            .expect("GetActivity returns four values")
    })
}

#[test]
fn an_absent_idle_object_is_unavailable() {
    let fixture = served(None);

    assert_eq!(get_activity(&fixture), (false, false, THRESHOLD_MS, 0));
}

#[test]
fn losing_the_idle_object_reports_unavailable_once() {
    let fixture = served(Some(reading(1, 5_000)));

    let emitted = fixture.service.observe(&closed(1));
    assert_eq!(
        emitted,
        Some(Activity {
            available: false,
            idle: false,
            threshold_ms: THRESHOLD_MS,
            idle_for_ms: 0,
        })
    );
    assert_eq!(get_activity(&fixture), (false, false, THRESHOLD_MS, 0));
    assert_eq!(fixture.service.observe(&closed(1)), None);
}

#[test]
fn the_threshold_is_inclusive() {
    let fixture = served(Some(reading(1, THRESHOLD_MS - 1)));

    let idle = fixture
        .service
        .observe(&changed(1, THRESHOLD_MS))
        .expect("crossing the threshold emits");
    assert!(idle.available && idle.idle);
    assert_eq!(idle.idle_for_ms, THRESHOLD_MS);

    let active = fixture
        .service
        .observe(&changed(1, THRESHOLD_MS - 1))
        .expect("falling below the threshold emits");
    assert!(active.available && !active.idle);
    assert_eq!(active.idle_for_ms, 0);
    assert_eq!(get_activity(&fixture), (true, false, THRESHOLD_MS, 0));
}

#[test]
fn a_change_to_the_duration_alone_emits_nothing() {
    let fixture = served(Some(reading(1, THRESHOLD_MS + 10_000)));

    assert_eq!(fixture.service.observe(&changed(1, THRESHOLD_MS + 20_000)), None);
    assert_eq!(
        get_activity(&fixture),
        (true, true, THRESHOLD_MS, THRESHOLD_MS + 20_000)
    );
}

#[test]
fn a_duplicate_observation_emits_nothing() {
    let fixture = served(Some(reading(1, 5_000)));

    assert_eq!(fixture.service.observe(&changed(1, 5_000)), None);
    assert_eq!(fixture.service.observe(&opened(1, 5_000)), None);
}

#[test]
fn changes_to_other_objects_are_ignored() {
    let fixture = served(Some(reading(1, 5_000)));

    assert_eq!(fixture.service.observe(&changed(2, THRESHOLD_MS + 1)), None);
    assert_eq!(fixture.service.observe(&closed(2)), None);
    assert_eq!(get_activity(&fixture), (true, false, THRESHOLD_MS, 0));
}

#[test]
fn an_unreadable_compositor_is_not_served() {
    let bus = Bus::start();
    let server = zbus::block_on(connect(bus.address()));

    assert!(serve(&Fake(Err("the compositor is down".to_owned())), &server).is_none());
}

/// Wait for one `ActivityChanged`, bounded so a missing signal fails the test instead of hanging.
async fn activity_changed(stream: &mut zbus::proxy::SignalStream<'_>) -> Option<(bool, bool, u64, u64)> {
    futures_lite::future::race(
        async {
            let message = stream.next().await?;
            message.body().deserialize().ok()
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

#[test]
fn a_published_change_reaches_a_subscriber() {
    let fixture = served(Some(reading(1, 1_000)));
    let destination = fixture
        .server
        .unique_name()
        .expect("a connected server has a unique name")
        .to_string();
    let body = zbus::block_on(async {
        let proxy = Proxy::new(&fixture.client, destination.as_str(), PATH, INTERFACE)
            .await
            .expect("the proxy is built");
        let mut stream = proxy
            .receive_signal("ActivityChanged")
            .await
            .expect("the caller subscribes to the signal");

        let activity = fixture
            .service
            .observe(&changed(1, THRESHOLD_MS + 1))
            .expect("crossing the threshold emits");
        fixture.service.publish(activity);

        activity_changed(&mut stream).await
    });

    assert_eq!(body, Some((true, true, THRESHOLD_MS, THRESHOLD_MS + 1)));
}
