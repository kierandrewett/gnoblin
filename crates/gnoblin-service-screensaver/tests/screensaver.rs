//! The ScreenSaver service over a private bus: a real D-Bus client calls `Inhibit`,
//! `UnInhibit` and subscribes to `ActiveChanged`, while the compositor seam records what
//! the service tells it.
//!
//! Every test starts its own `dbus-daemon` (see `gnoblin-service-testkit`), so nothing here can
//! reach the developer's session bus. The caller is a real client connection, so the
//! method names, the argument signatures and the sender a call is attributed to are the
//! ones a media player or a video player uses. The compositor is the [`Compositor`] seam:
//! a recorder standing in for the compositor's `idle` and `lock` objects.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use futures_lite::StreamExt;
use gnoblin_model::{ObjectId, Observation, Revision, Value};
use gnoblin_service_screensaver::{BUS_NAME, Compositor, LockReading, PATHS, ScreenSaver};
use gnoblin_service_testkit::Bus;
use zbus::Connection;
use zbus::connection::Builder;

/// How long a test waits for the service to act on a bus event.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The lock object the recorder reports, unlocked when the service starts.
const LOCK_OBJECT: u64 = 7;

/// The compositor's hold names, in the order the service set them.
#[derive(Clone, Default)]
struct Recorder {
    holds: Arc<Mutex<Vec<String>>>,
}

impl Recorder {
    fn holds(&self) -> Vec<String> {
        self.holds.lock().unwrap().clone()
    }
}

impl Compositor for Recorder {
    fn idle_ms(&self) -> Result<u64, String> {
        Ok(0)
    }

    fn lock_reading(&self) -> Result<Option<LockReading>, String> {
        Ok(Some(LockReading {
            object: ObjectId::new(LOCK_OBJECT),
            locked: false,
        }))
    }

    fn lock(&self) -> Result<(), String> {
        Err("locking is not part of this test".to_owned())
    }

    fn inhibit(&self, holder: &str) -> Result<(), String> {
        self.holds.lock().unwrap().push(holder.to_owned());
        Ok(())
    }

    fn uninhibit(&self, holder: &str) -> Result<(), String> {
        self.holds.lock().unwrap().retain(|held| held != holder);
        Ok(())
    }
}

async fn connect(address: &str) -> Connection {
    Builder::address(address).build().await.expect("the private bus is up")
}

/// Serve the ScreenSaver interface on `address`, with `recorder` as the compositor. The
/// returned service is the handle a test feeds lock observations through.
fn serve(address: &str, recorder: Recorder) -> (Connection, ScreenSaver) {
    let server = zbus::block_on(connect(address));
    let service = ScreenSaver::new(Box::new(recorder));
    service.register(&server).expect("the service registers on the bus");
    (server, service)
}

/// A client's unique name and its connection, on its own connection to the bus.
fn caller(address: &str) -> (Connection, String) {
    let client = zbus::block_on(connect(address));
    let unique = client
        .unique_name()
        .expect("a connected client has a unique name")
        .to_string();
    (client, unique)
}

/// Poll `condition` until it holds, failing the test at the deadline.
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "the service never reached: {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A `locked` change to the lock object, as the services role delivers it.
fn lock_changed(locked: bool) -> Observation {
    Observation::Changed {
        object: ObjectId::new(LOCK_OBJECT),
        property: "locked".to_owned(),
        value: Value::Bool(locked),
        revision: Revision::new(1),
    }
}

/// Wait for one `ActiveChanged`, bounded so a missing signal fails the test instead of hanging.
async fn active_changed(stream: &mut zbus::proxy::SignalStream<'_>) -> Option<bool> {
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
fn an_inhibit_over_the_bus_holds_the_compositor_until_uninhibit() {
    // The failure mode: a player asks the desktop to stay awake and the compositor never
    // hears about it, so the screen blanks over a film. The hold reaches the compositor
    // under the sender's name, and UnInhibit releases exactly that hold.
    let bus = Bus::start();
    let recorder = Recorder::default();
    let (_server, _service) = serve(bus.address(), recorder.clone());
    let (client, unique) = caller(bus.address());

    let proxy = zbus::block_on(zbus::Proxy::new(&client, BUS_NAME, PATHS[0], BUS_NAME)).expect("the proxy is built");
    let cookie: u32 =
        zbus::block_on(proxy.call("Inhibit", &("player", "playing"))).expect("the service accepts the inhibit");
    assert_eq!(recorder.holds(), vec![format!("{unique}:{cookie}")]);

    zbus::block_on(proxy.call_method("UnInhibit", &(cookie,))).expect("the service accepts the release");
    assert!(recorder.holds().is_empty(), "UnInhibit released the compositor's hold");
}

#[test]
fn an_uninhibit_for_a_foreign_cookie_is_access_denied() {
    // The failure mode: a second client releases a cookie it never got, and the service
    // reports success while the owner's hold is gone, so the owner's playback stops
    // keeping the screen awake without its knowledge. The refusal is an error on the bus
    // and leaves the owner's hold in place. An unheld cookie is refused the same way.
    let bus = Bus::start();
    let recorder = Recorder::default();
    let (_server, _service) = serve(bus.address(), recorder.clone());
    let (owner, owner_unique) = caller(bus.address());
    let (intruder, _) = caller(bus.address());

    let owned = zbus::block_on(zbus::Proxy::new(&owner, BUS_NAME, PATHS[0], BUS_NAME)).expect("the proxy is built");
    let foreign =
        zbus::block_on(zbus::Proxy::new(&intruder, BUS_NAME, PATHS[0], BUS_NAME)).expect("the proxy is built");
    let cookie: u32 = zbus::block_on(owned.call("Inhibit", &("player", "playing"))).expect("inhibit");

    for unheld in [cookie, cookie + 100] {
        let error = zbus::block_on(foreign.call_method("UnInhibit", &(unheld,)))
            .expect_err("a cookie another sender holds, or nobody holds, is refused");
        // zbus turns the standard org.freedesktop.DBus.Error names into its FDO variant, so a
        // MethodError match would never see this reply and the assertion would be vacuous.
        match error {
            zbus::Error::FDO(fdo) => {
                assert!(matches!(*fdo, zbus::fdo::Error::AccessDenied(_)), "got {fdo}");
            }
            other => panic!("expected AccessDenied, got {other}"),
        }
    }
    assert_eq!(recorder.holds(), vec![format!("{owner_unique}:{cookie}")]);

    zbus::block_on(owned.call_method("UnInhibit", &(cookie,))).expect("the owner releases its own hold");
    assert!(recorder.holds().is_empty());
}

#[test]
fn a_sender_leaving_the_bus_releases_its_holds_in_the_compositor() {
    // The failure mode: a player crashes while inhibiting, and the desktop never sleeps
    // again because nobody sent UnInhibit. The service watches the bus for the sender
    // leaving and releases every hold it made for that sender.
    let bus = Bus::start();
    let recorder = Recorder::default();
    let (_server, _service) = serve(bus.address(), recorder.clone());
    let (client, unique) = caller(bus.address());
    let (other, other_unique) = caller(bus.address());

    let proxy = zbus::block_on(zbus::Proxy::new(&client, BUS_NAME, PATHS[1], BUS_NAME)).expect("the proxy is built");
    let kept = zbus::block_on(zbus::Proxy::new(&other, BUS_NAME, PATHS[1], BUS_NAME)).expect("the proxy is built");
    let _: u32 = zbus::block_on(proxy.call("Inhibit", &("player", "playing"))).expect("inhibit");
    let other_cookie: u32 = zbus::block_on(kept.call("Inhibit", &("video", "watching"))).expect("inhibit");
    assert_eq!(recorder.holds().len(), 2);

    drop(proxy);
    drop(client);
    wait_until("the departed sender's hold is released", || {
        recorder.holds() == vec![format!("{other_unique}:{other_cookie}")]
    });
    assert!(!recorder.holds().iter().any(|hold| hold.starts_with(&unique)));
}

#[test]
fn active_changed_reaches_a_subscriber_once_per_change() {
    // The failure mode: `ActiveChanged` was declared and never emitted, so a locker that
    // subscribes never learns the session locked. Each real change reaches the subscriber
    // once and in order, and a repeat of the state already announced emits nothing, so
    // the next signal is the next real change.
    let bus = Bus::start();
    let (server, service) = serve(bus.address(), Recorder::default());
    let destination = server
        .unique_name()
        .expect("a connected server has a unique name")
        .to_string();
    let (client, _) = caller(bus.address());

    let received = zbus::block_on(async {
        let proxy = zbus::Proxy::new(&client, destination.as_str(), PATHS[0], BUS_NAME)
            .await
            .expect("the proxy is built");
        let mut stream = proxy
            .receive_signal("ActiveChanged")
            .await
            .expect("the caller subscribes to the signal");

        // The compositor reports the lock twice with the same value, then unlocks. The
        // service must publish for the first change only, then for the unlock.
        for locked in [true, true, false] {
            if let Some(active) = service.observe(&lock_changed(locked)) {
                ScreenSaver::publish(&server, active);
            }
        }

        // Read exactly the two changes the service should announce. A duplicate `true`
        // would arrive as the second signal and fail the comparison, and a stray signal
        // after the unlock is caught by the final `None`.
        let mut seen = Vec::new();
        for _ in 0..2 {
            seen.push(active_changed(&mut stream).await.expect("a change is announced"));
        }
        let extra = active_changed(&mut stream).await;
        (seen, extra)
    });

    assert_eq!(received, (vec![true, false], None));
}
