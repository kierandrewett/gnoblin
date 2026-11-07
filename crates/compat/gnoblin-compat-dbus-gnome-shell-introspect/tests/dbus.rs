//! The adapter over a private bus, with the loopback as its fake IPC server.
//!
//! Every test starts its own `dbus-daemon` (see `support/bus.rs`), so nothing here can reach
//! the developer's session bus. The caller is a plain `zbus::Proxy`, which is what the
//! portal's window picker resolves to, and the list arrives the way it does in a session: as
//! object diffs the link folds.

mod support;

use std::collections::HashMap;

use futures_lite::StreamExt as _;
use gnoblin_client::{Loopback, Server};
use gnoblin_compat_core::{Mode, SupportMatrix};
use gnoblin_compat_dbus_gnome_shell_introspect::{
    BUS_NAME, INTERFACE, IntrospectError, IntrospectLink, IntrospectService, MATRIX, PATH, export,
};
use gnoblin_model::{ObjectId, TypeKey, Value};
use support::bus::Bus;
use support::{VERSION, graph, output_properties, unlocked, window_properties};
use zbus::{Connection, OwnedVariant, Proxy};

/// One window's property dict.
type Properties = HashMap<String, OwnedVariant>;

/// What `GetWindows` answers.
type Windows = HashMap<u64, Properties>;

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

/// Wait for one signal, bounded so a missing one fails the test instead of hanging.
async fn signalled(stream: &mut zbus::proxy::SignalStream<'_>) -> bool {
    futures_lite::future::race(async { stream.next().await.map(|_| true) }, async {
        for _ in 0..20_000 {
            futures_lite::future::yield_now().await;
        }
        None
    })
    .await
    .unwrap_or(false)
}

/// Read the window list from a proxy.
async fn windows(proxy: &Proxy<'_>) -> Windows {
    proxy
        .call_method("GetWindows", &())
        .await
        .expect("GetWindows is supported")
        .body()
        .deserialize()
        .expect("the reply is the declared signature")
}

/// One server, one link and one published state: the fixture every test starts from.
struct Fixture {
    server: Server,
    link: IntrospectLink<Loopback>,
}

impl Fixture {
    /// Two windows, one enabled output and an unlocked session.
    fn new() -> Fixture {
        let mut server = Server::new(graph(), VERSION);
        for (id, title, app) in [
            (1u64, "Editor", "org.example.Editor"),
            (2, "Terminal", "org.example.Terminal"),
        ] {
            server
                .create(TypeKey::gnoblin("window"), window_properties(id, title, app))
                .expect("the window type and its properties are in the schema");
        }
        server
            .create(TypeKey::gnoblin("output"), output_properties("eDP-1", 1920, 1080, true))
            .expect("the output type and its properties are in the schema");
        server
            .create(TypeKey::gnoblin("lock"), unlocked())
            .expect("the lock type and its property are in the schema");
        let client = server.connect(VERSION).expect("the versions match");
        let link = IntrospectLink::connect(client).expect("the snapshot is a state this adapter can report");
        Fixture { server, link }
    }

    /// Publish whatever the link has folded.
    async fn publish(&mut self, service: &IntrospectService) {
        self.link.drain().expect("the graph's observations are readable");
        service
            .publish(self.link.state().clone())
            .await
            .expect("the signal is emitted");
    }

    /// Open one more window.
    fn open(&mut self, id: u64, title: &str, app: &str) -> ObjectId {
        self.server
            .create(TypeKey::gnoblin("window"), window_properties(id, title, app))
            .expect("a window is an object")
    }
}

#[test]
fn get_windows_lists_the_window_objects() {
    // The failure mode: the picker shows a list of its own, or keeps the snapshot it started
    // from. The first half is the snapshot, the second is a window opening and being announced.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IntrospectService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;

        let proxy = caller(&connection).await;
        let mut changed = proxy
            .receive_signal("WindowsChanged")
            .await
            .expect("the caller subscribes to the signal");
        let list = windows(&proxy).await;
        let mut identities: Vec<u64> = list.keys().copied().collect();
        identities.sort_unstable();
        assert_eq!(
            identities,
            vec![1, 2],
            "every window object is an entry, keyed by its identity"
        );
        let editor = &list[&1];
        assert_eq!(
            editor.get("title").map(|value| value.to_string()),
            Some("\"Editor\"".to_owned()),
            "the title is the window's own"
        );
        assert_eq!(
            editor.get("app-id").map(|value| value.to_string()),
            Some("\"org.example.Editor\"".to_owned())
        );
        assert_eq!(
            editor.get("wm-class").map(|value| value.to_string()),
            Some("\"org.example.Editor\"".to_owned()),
            "a Wayland window's wm-class is its app id, as Mutter reports it"
        );
        assert_eq!(
            editor.get("width").and_then(|value| value.downcast_ref::<u32>().ok()),
            Some(800u32),
            "the width is the window's own"
        );
        assert_eq!(
            editor
                .get("has-focus")
                .and_then(|value| value.downcast_ref::<bool>().ok()),
            Some(false)
        );

        fixture.open(3, "Files", "org.example.Files");
        fixture.publish(&service).await;
        assert!(signalled(&mut changed).await, "an opened window is announced");
        assert_eq!(windows(&proxy).await.len(), 3, "and it is in the list");
    });
}

#[test]
fn a_closed_window_leaves_the_list() {
    // The failure mode: the picker offers a window that is gone, and the user's click lands on
    // nothing.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IntrospectService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;
        let mut changed = proxy
            .receive_signal("WindowsChanged")
            .await
            .expect("the caller subscribes to the signal");

        let window = fixture
            .link
            .state()
            .windows
            .iter()
            .find(|window| window.id == 1)
            .map(|window| window.id)
            .expect("the window is in the state");
        let object = fixture
            .server
            .graph()
            .objects()
            .find(|view| matches!(view.property("id"), Some(Value::Uint(id)) if *id == window))
            .map(|view| view.id())
            .expect("the graph holds the window");
        fixture.server.close(object).expect("the object ends once");
        fixture.publish(&service).await;
        assert!(signalled(&mut changed).await, "a closed window is announced");
        assert!(!windows(&proxy).await.contains_key(&1), "and it left the dictionary");
    });
}

#[test]
fn running_applications_changed_fires_when_an_app_id_appears() {
    // The failure mode: a caller that tracks applications never learns a new one started, so
    // its application list goes stale while its window list stays current.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IntrospectService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;
        let mut applications = proxy
            .receive_signal("RunningApplicationsChanged")
            .await
            .expect("the caller subscribes to the signal");

        // A second window of an application that is already running is not a new application.
        fixture.open(3, "Editor (2)", "org.example.Editor");
        fixture.publish(&service).await;
        assert!(
            !signalled(&mut applications).await,
            "a second window of a running application changes no application"
        );

        fixture.open(4, "Files", "org.example.Files");
        fixture.publish(&service).await;
        assert!(signalled(&mut applications).await, "a new application is announced");
    });
}

#[test]
fn screen_size_is_the_union_of_the_enabled_outputs() {
    // The failure mode: the screen size is the bounding box of the windows, or of the disabled
    // outputs, so a picker lays itself out for a desktop that is not there.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        fixture
            .server
            .create(
                TypeKey::gnoblin("output"),
                output_properties("HDMI-1", 2560, 1440, false),
            )
            .expect("a disabled output is still an object");
        let connection = connect(&address).await;
        let service = IntrospectService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;

        let size: (i32, i32) = caller(&connection)
            .await
            .get_property("ScreenSize")
            .await
            .expect("ScreenSize is supported");
        assert_eq!(size, (1920, 1080), "only the enabled output counts");
        let version: u32 = caller(&connection)
            .await
            .get_property("version")
            .await
            .expect("the version property is supported");
        assert_eq!(version, 3, "the version is the one the declaration was written against");
    });
}

#[test]
fn an_unsupported_member_is_refused_by_name() {
    // The failure mode the plan names: a caller depends on a method that is not served and
    // learns nothing. Every `unsupported` member in the matrix answers with the matrix's rule.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = IntrospectService::new().expect("the matrix loads");
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
                "GetRunningApplications" => proxy.call_method("GetRunningApplications", &()).await.unwrap_err(),
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
    // The failure mode: a second export replaces the object behind a live name while the first
    // one's signal emitter still points at the path.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let connection = connect(&address).await;
        let service = IntrospectService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        let again = export(&connection, &service).await;
        assert!(
            matches!(again, Err(IntrospectError::Exported)),
            "the second export is refused, not a second object server entry"
        );
    });
}
