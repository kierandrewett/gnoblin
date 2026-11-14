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
use gnoblin_model::{Fields, Filter, ObjectId, OperationOutcome, TypeKey, Value};
use gnoblin_types::ErrorCode;
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
fn idle_id(session: &Session) -> ObjectId {
    let mut client = ipc(session);
    let snapshot = client.subscribe(Filter::All).expect("the snapshot");
    snapshot
        .objects()
        .iter()
        .find(|view| view.ty() == &TypeKey::gnoblin("idle"))
        .map(|view| view.id())
        .expect("the idle object")
}

/// One IPC connection that holds idle under names of its own. The holds belong to the
/// connection, so dropping the value is what a shell exiting looks like to the compositor.
struct Holds {
    client: Client<UnixTransport>,
    idle: ObjectId,
}

impl Holds {
    fn connect(session: &Session) -> Holds {
        Holds {
            client: ipc(session),
            idle: idle_id(session),
        }
    }

    fn call(&mut self, method: &str, holder: &str) -> OperationOutcome {
        let arguments = Fields::from([("holder".to_owned(), Value::Text(holder.to_owned()))]);
        let operation = self
            .client
            .call(self.idle, method, &arguments)
            .expect("the call is served");
        self.client.await_operation(operation).expect("the completion")
    }

    /// Holds under `holder` and asserts the compositor recorded it.
    #[track_caller]
    fn inhibit(&mut self, holder: &str) {
        let outcome = self.call("inhibit", holder);
        assert!(outcome.is_done(), "the hold {holder} was recorded: {outcome:?}");
    }

    #[track_caller]
    fn uninhibit(&mut self, holder: &str) {
        let outcome = self.call("uninhibit", holder);
        assert!(outcome.is_done(), "the release of {holder} completed: {outcome:?}");
    }
}

/// Waits until the compositor publishes `count` inhibitors.
#[track_caller]
fn wait_for_inhibitors(session: &Session, count: u64) {
    wait_for(&format!("the compositor counts {count} inhibitors"), || {
        (inhibitors(session) == count).then_some(())
    });
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
    let mut holds = Holds::connect(&session);
    holds.inhibit("player");
    assert_eq!(inhibitors(&session), 1);

    let mut client = Wayland::connect(session.wayland());
    let _notification = client.notify();
    client.watch_for(HELD_FOR);
    assert_eq!(client.app.idled, 0, "an IPC hold keeps the session awake");

    holds.uninhibit("player");
    wait_for("the idle notification after uninhibit", || {
        client.roundtrip();
        (client.app.idled == 1).then_some(())
    });
    assert_eq!(inhibitors(&session), 0);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_connection_that_closes_gives_back_only_its_own_holds() {
    // The failure mode: a shell is killed without retiring, and its hold keeps the display
    // awake until the session ends; or the hold of a connection that is still there goes
    // with it.
    let session = Session::start();
    let mut first = Holds::connect(&session);
    let mut second = Holds::connect(&session);
    first.inhibit("a");
    second.inhibit("b");
    assert_eq!(inhibitors(&session), 2, "both holds are counted");

    let mut client = Wayland::connect(session.wayland());
    let _notification = client.notify();
    drop(first);
    wait_for_inhibitors(&session, 1);
    client.watch_for(HELD_FOR);
    assert_eq!(
        client.app.idled, 0,
        "the other connection's hold still keeps the session awake"
    );

    drop(second);
    wait_for("the idle notification once the last hold went", || {
        client.roundtrip();
        (client.app.idled == 1).then_some(())
    });
    assert_eq!(inhibitors(&session), 0);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_connection_that_dies_mid_frame_gives_back_its_holds() {
    // An abrupt death, not a close: the peer's last bytes are half a frame, as when a process
    // is killed while it writes.
    let session = Session::start();
    let idle = idle_id(&session);
    let mut survivor = Holds::connect(&session);
    survivor.inhibit("b");

    let mut wire = session.wire();
    let arguments = Fields::from([("holder".to_owned(), Value::Text("a".to_owned()))]);
    wire.send(&gnoblin_client::wire::call(1, idle, "inhibit", &arguments));
    wire.receive().expect("the call is answered");
    wait_for_inhibitors(&session, 2);
    wire.send_bytes(&[0xff, 0xff, 0xff]);
    wire.kill();

    wait_for_inhibitors(&session, 1);
    survivor.uninhibit("b");
    assert_eq!(inhibitors(&session), 0);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_same_holder_name_on_two_connections_is_two_holds() {
    // A name is scoped to the connection that used it: two shells that both call their hold
    // "player" neither merge into one hold nor release each other's.
    let session = Session::start();
    let mut first = Holds::connect(&session);
    let mut second = Holds::connect(&session);
    first.inhibit("player");
    second.inhibit("player");
    assert_eq!(inhibitors(&session), 2, "one name on two connections is two holds");

    first.uninhibit("player");
    assert_eq!(
        inhibitors(&session),
        1,
        "releasing from one connection leaves the other's hold"
    );
    first.uninhibit("player");
    assert_eq!(
        inhibitors(&session),
        1,
        "releasing a name the connection does not hold is a no-op"
    );

    drop(first);
    // Nothing of the first connection is left to release, and the second is untouched.
    let mut third = Holds::connect(&session);
    third.uninhibit("player");
    assert_eq!(
        inhibitors(&session),
        1,
        "a third connection cannot release the second's hold"
    );
    drop(second);
    wait_for_inhibitors(&session, 0);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_holder_cap_is_global_and_a_closed_connection_frees_its_share() {
    // The cap bounds what the session keeps, whichever connection asks, so it is one count
    // across connections. A connection that closes gives its holds back to the cap.
    let session = Session::start();
    let mut first = Holds::connect(&session);
    let mut second = Holds::connect(&session);
    for holder in 0..200 {
        first.inhibit(&format!("first-{holder}"));
    }
    for holder in 0..56 {
        second.inhibit(&format!("second-{holder}"));
    }
    assert_eq!(inhibitors(&session), 256);

    let too_many = OperationOutcome::Failed(ErrorCode::from_static("too-many-inhibitors"));
    assert_eq!(
        second.call("inhibit", "overflow"),
        too_many,
        "the cap is full for this connection"
    );
    assert_eq!(first.call("inhibit", "overflow"), too_many, "and for the other");
    // A name another connection holds is still a new hold for this one.
    assert_eq!(second.call("inhibit", "first-0"), too_many);
    assert_eq!(inhibitors(&session), 256, "a refused hold is not recorded");

    drop(first);
    wait_for_inhibitors(&session, 56);
    second.inhibit("overflow");
    assert_eq!(inhibitors(&session), 57);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_closed_connection_leaves_a_wayland_surface_inhibit_alone() {
    // `zwp_idle_inhibit_v1` surfaces belong to their Wayland client, so the IPC connection's end
    // does not touch them.
    let session = Session::start();
    let mut client = Wayland::connect(session.wayland());
    let surface = client.surface();
    let _inhibitor: zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1 =
        client.inhibit_manager.create_inhibitor(&surface, &client.qh, ());
    client.roundtrip();
    let _notification = client.notify();

    let mut holds = Holds::connect(&session);
    holds.inhibit("a");
    assert_eq!(inhibitors(&session), 2, "the surface and the named hold");
    drop(holds);
    wait_for_inhibitors(&session, 1);
    client.watch_for(HELD_FOR);
    assert_eq!(client.app.idled, 0, "the surface still holds the session awake");
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
