//! Idle is held while anything inhibits it, end to end: a real Wayland client asks for an
//! idle notification, the session's own clock runs, and the holds come from a Wayland
//! `zwp_idle_inhibit_v1` surface and from the `idle` object's `inhibit` method over IPC.
//!
//! The threshold is the notification's own timeout, passed in by the client, so each test
//! uses a few hundred milliseconds and waits on the events rather than on fixed sleeps,
//! except for the one window where the test must show that nothing fired.

mod support;

use std::time::{Duration, Instant};

use gnoblin_client::Client;
use gnoblin_client::unix::UnixTransport;
use gnoblin_compositor::version;
use gnoblin_model::{Filter, TypeKey, Value};
use support::{Session, wait_for};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_compositor, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection as WaylandConnection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{ext_idle_notification_v1, ext_idle_notifier_v1};
use wayland_protocols::wp::idle_inhibit::zv1::client::{zwp_idle_inhibit_manager_v1, zwp_idle_inhibitor_v1};

/// The idle threshold each notification asks for.
const THRESHOLD_MS: u32 = 300;

/// How long a held session is watched to show that no idle notification fires.
const HELD_FOR: Duration = Duration::from_millis(1_000);

/// What the client records from the events the session sends it.
#[derive(Default)]
struct App {
    idled: u32,
    resumed: u32,
}

/// A Wayland client with the globals the idle tests need.
struct Wayland {
    _connection: WaylandConnection,
    queue: EventQueue<App>,
    qh: QueueHandle<App>,
    app: App,
    compositor: wl_compositor::WlCompositor,
    seat: wl_seat::WlSeat,
    notifier: ext_idle_notifier_v1::ExtIdleNotifierV1,
    inhibit_manager: zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1,
}

impl Wayland {
    fn connect(socket: &std::path::Path) -> Wayland {
        let stream = std::os::unix::net::UnixStream::connect(socket).expect("the session is listening");
        let connection = WaylandConnection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises wl_compositor");
        let seat = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_seat");
        let notifier = globals
            .bind(&qh, 1..=2, ())
            .expect("the session advertises ext_idle_notifier_v1");
        let inhibit_manager = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises zwp_idle_inhibit_manager_v1");
        Wayland {
            _connection: connection,
            queue,
            qh,
            app: App::default(),
            compositor,
            seat,
            notifier,
            inhibit_manager,
        }
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.app).expect("a round trip");
    }

    /// Asks for an idle notification after the threshold; its `idled` events are counted.
    fn notify(&mut self) -> ext_idle_notification_v1::ExtIdleNotificationV1 {
        let notification = self
            .notifier
            .get_idle_notification(THRESHOLD_MS, &self.seat, &self.qh, ());
        self.roundtrip();
        notification
    }

    /// Runs the event queue for `span`, so a notification that should not fire has the
    /// whole span to do so.
    fn watch_for(&mut self, span: Duration) {
        let until = Instant::now() + span;
        while Instant::now() < until {
            self.roundtrip();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A surface, which is what a `zwp_idle_inhibit_v1` holds on to.
    fn surface(&self) -> wl_surface::WlSurface {
        self.compositor.create_surface(&self.qh, ())
    }
}

/// An IPC client connection at the served version.
fn ipc(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session is listening"),
        version(),
    )
    .expect("the session accepts this build")
}

/// The `idle` object's identity, read from a fresh snapshot.
fn idle_id(session: &Session) -> String {
    let mut client = ipc(session);
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
        .map(|view| view.id().get().to_string())
        .expect("the idle object")
}

/// The `inhibitors` count the compositor publishes, read from a fresh snapshot.
fn inhibitors(session: &Session) -> u64 {
    let mut client = ipc(session);
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    let idle = snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
        .expect("the idle object");
    match idle.property("inhibitors") {
        Some(Value::Uint(count)) => *count,
        other => panic!("inhibitors is not a uint: {other:?}"),
    }
}

#[test]
fn an_idle_notification_fires_after_its_threshold() {
    let session = Session::start();
    let mut client = Wayland::connect(session.wayland());
    let _notification = client.notify();

    wait_for("the idle notification", || {
        client.roundtrip();
        (client.app.idled == 1).then_some(())
    });
    assert_eq!(client.app.idled, 1, "one `idled` for the threshold");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_wayland_surface_inhibit_holds_idle_until_it_is_released() {
    // The failure mode: a video player inhibits idle through zwp_idle_inhibit_v1 and the
    // screen still goes idle over the film, or the hold outlives the surface and idle
    // never comes back.
    let session = Session::start();
    let mut client = Wayland::connect(session.wayland());
    let surface = client.surface();
    let inhibitor: zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1 =
        client.inhibit_manager.create_inhibitor(&surface, &client.qh, ());
    client.roundtrip();
    assert_eq!(inhibitors(&session), 1, "the surface inhibit is counted");

    let _notification = client.notify();
    client.watch_for(HELD_FOR);
    assert_eq!(client.app.idled, 0, "a held session does not go idle");

    inhibitor.destroy();
    client.roundtrip();
    wait_for("the idle notification after release", || {
        client.roundtrip();
        (client.app.idled == 1).then_some(())
    });
    assert_eq!(inhibitors(&session), 0, "the surface inhibit is released");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_ipc_inhibit_holds_idle_until_uninhibit() {
    // The failure mode: the services role inhibits for a playing player, the compositor
    // records the hold, and idle still fires; or the hold is never released.
    let session = Session::start();
    let idle = idle_id(&session);
    session.ctl(&["idle", "call", &idle, "inhibit", "holder=player"]).ok();
    assert_eq!(inhibitors(&session), 1);

    let mut client = Wayland::connect(session.wayland());
    let _notification = client.notify();
    client.watch_for(HELD_FOR);
    assert_eq!(client.app.idled, 0, "an IPC hold keeps the session awake");

    session.ctl(&["idle", "call", &idle, "uninhibit", "holder=player"]).ok();
    wait_for("the idle notification after uninhibit", || {
        client.roundtrip();
        (client.app.idled == 1).then_some(())
    });
    assert_eq!(inhibitors(&session), 0);
    session.stop().expect("the session stops cleanly");
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for App {
    fn event(
        app: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: <ext_idle_notification_v1::ExtIdleNotificationV1 as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => app.idled += 1,
            ext_idle_notification_v1::Event::Resumed => app.resumed += 1,
            _ => {}
        }
    }
}

/// The objects whose events the idle tests do not need to see.
macro_rules! ignored_events {
    ($($object:ty),* $(,)?) => {
        $(
            impl Dispatch<$object, ()> for App {
                fn event(
                    _: &mut Self,
                    _: &$object,
                    _: <$object as Proxy>::Event,
                    _: &(),
                    _: &WaylandConnection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        )*
    };
}

ignored_events!(
    wl_compositor::WlCompositor,
    wl_seat::WlSeat,
    wl_surface::WlSurface,
    ext_idle_notifier_v1::ExtIdleNotifierV1,
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1,
    zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
);
