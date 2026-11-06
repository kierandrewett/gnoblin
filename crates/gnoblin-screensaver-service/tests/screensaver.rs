//! The ScreenSaver service over a private bus: a real D-Bus client calls `Inhibit` and
//! `UnInhibit`, and the compositor seam records what the service tells it.
//!
//! Every test starts its own `dbus-daemon` (see `gnoblin-service-testkit`), so nothing here can
//! reach the developer's session bus. The caller is a real client connection, so the
//! method names, the argument signatures and the sender a call is attributed to are the
//! ones a media player or a video player uses. The compositor is the [`Compositor`] seam:
//! a recorder standing in for the compositor's `idle` object.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_screensaver_service::{BUS_NAME, Compositor, PATHS, ScreenSaver};
use gnoblin_service_testkit::Bus;
use zbus::Connection;
use zbus::connection::Builder;

/// How long a test waits for the service to act on a bus event.
const TIMEOUT: Duration = Duration::from_secs(10);

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

    fn locked(&self) -> Result<bool, String> {
        Ok(false)
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

/// Serve the ScreenSaver interface on `address`, with `recorder` as the compositor.
fn serve(address: &str, recorder: Recorder) -> Connection {
    let server = zbus::block_on(connect(address));
    let service = ScreenSaver::new(Box::new(recorder));
    service.register(&server).expect("the service registers on the bus");
    server
}

/// A client's unique name and a proxy to the ScreenSaver object, on its own connection.
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

#[test]
fn an_inhibit_over_the_bus_holds_the_compositor_until_uninhibit() {
    // The failure mode: a player asks the desktop to stay awake and the compositor never
    // hears about it, so the screen blanks over a film. The hold reaches the compositor
    // under the sender's name, and UnInhibit releases exactly that hold.
    let bus = Bus::start();
    let recorder = Recorder::default();
    let _server = serve(bus.address(), recorder.clone());
    let (client, unique) = caller(bus.address());

    let proxy = zbus::block_on(zbus::Proxy::new(&client, BUS_NAME, PATHS[0], BUS_NAME)).expect("the proxy is built");
    let cookie: u32 =
        zbus::block_on(proxy.call("Inhibit", &("player", "playing"))).expect("the service accepts the inhibit");
    assert_eq!(recorder.holds(), vec![format!("{unique}:{cookie}")]);

    zbus::block_on(proxy.call_method("UnInhibit", &(cookie,))).expect("the service accepts the release");
    assert!(recorder.holds().is_empty(), "UnInhibit released the compositor's hold");
}

#[test]
fn a_sender_leaving_the_bus_releases_its_holds_in_the_compositor() {
    // The failure mode: a player crashes while inhibiting, and the desktop never sleeps
    // again because nobody sent UnInhibit. The service watches the bus for the sender
    // leaving and releases every hold it made for that sender.
    let bus = Bus::start();
    let recorder = Recorder::default();
    let _server = serve(bus.address(), recorder.clone());
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
