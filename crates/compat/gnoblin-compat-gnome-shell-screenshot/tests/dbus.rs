//! The adapter over a private bus, with the loopback as its fake IPC server.
//!
//! Every test starts its own `dbus-daemon` (see `support/bus.rs`), so nothing here can reach the
//! developer's session bus. The caller is a plain `zbus::Proxy`, and the three refusals are
//! distinguished by the rule each one quotes, which is the whole of what this adapter promises.

mod support;

use gnoblin_client::{Loopback, Server};
use gnoblin_compat_core::{LockState, Mode, SupportMatrix};
use gnoblin_compat_gnome_shell_screenshot::{
    BUS_NAME, INTERFACE, MATRIX, PATH, ScreenshotError, ScreenshotLink, ScreenshotService, export,
};
use gnoblin_model::{ObjectId, TypeKey, Value};
use support::bus::Bus;
use support::{VERSION, graph, lock_properties};
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

/// One server, one link and one published embargo.
struct Fixture {
    server: Server,
    link: ScreenshotLink<Loopback>,
    lock: ObjectId,
}

impl Fixture {
    /// An unlocked session, read through the loopback.
    fn new() -> Fixture {
        let mut server = Server::new(graph(), VERSION);
        let lock = server
            .create(TypeKey::gnoblin("lock"), lock_properties(false))
            .expect("the lock type and its property are in the schema");
        let client = server.connect(VERSION).expect("the versions match");
        let link = ScreenshotLink::connect(client).expect("the snapshot publishes the embargo");
        Fixture { server, link, lock }
    }

    /// Publish whatever the link has folded.
    fn publish(&mut self, service: &ScreenshotService) {
        self.link.drain().expect("the graph's observations are readable");
        service.publish(self.link.lock());
    }
}

#[test]
fn a_screenshot_is_refused_in_portal_mode() {
    // The failure mode D13 exists for: a same-user caller screenshots the desktop with no
    // consent because the adapter forgot which mode it serves in.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = ScreenshotService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service);

        let error = caller(&connection)
            .await
            .call_method("Screenshot", &(false, false, "/tmp/shot.png".to_owned()))
            .await
            .expect_err("capture is refused in the default mode");
        let message = refusal(error);
        assert!(
            message.contains("capture-portal-only"),
            "the refusal names the mode's rule: {message}"
        );
        assert!(
            message.contains("portal"),
            "and it says where capture does happen: {message}"
        );
    });
}

#[test]
fn a_screenshot_is_refused_while_the_session_is_locked() {
    // The failure mode in the other direction: `unmediated` mode is treated as a licence to
    // capture the lock screen. The embargo is the compositor's, read per call.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = ScreenshotService::new().expect("the matrix loads");
        service.set_mode(Mode::Unmediated);
        export(&connection, &service).await.expect("the name is free");

        fixture
            .server
            .set(fixture.lock, "locked", Value::Bool(true))
            .expect("locked is a declared bool property");
        fixture.publish(&service);
        assert_eq!(fixture.link.lock(), LockState::Locked, "the link read the lock object");

        let error = caller(&connection)
            .await
            .call_method("Screenshot", &(false, false, "/tmp/shot.png".to_owned()))
            .await
            .expect_err("a locked session refuses capture in every mode");
        let message = refusal(error);
        assert!(
            message.contains("locked-capture"),
            "the refusal names the embargo: {message}"
        );
    });
}

#[test]
fn a_screenshot_is_unsupported_when_capture_is_unmediated() {
    // The failure mode the plan names: a caller depends on a method that is not served and
    // learns nothing. With the mode and the lock out of the way, the matrix is what refuses.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = ScreenshotService::new().expect("the matrix loads");
        service.set_mode(Mode::Unmediated);
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service);

        let error = caller(&connection)
            .await
            .call_method("Screenshot", &(false, false, "/tmp/shot.png".to_owned()))
            .await
            .expect_err("there is no image-capture member to serve");
        let message = refusal(error);
        assert!(
            message.contains("unsupported"),
            "the refusal quotes the matrix: {message}"
        );
        assert!(
            message.contains("image-capture"),
            "and it says what is missing: {message}"
        );
    });
}

#[test]
fn every_method_is_refused_by_name() {
    // The failure mode: one member answers as though it worked, so a caller believes it took a
    // screenshot. Every member the matrix declares is called, and each one must refuse.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = ScreenshotService::new().expect("the matrix loads");
        service.set_mode(Mode::Unmediated);
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service);
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
                "Screenshot" => proxy
                    .call_method("Screenshot", &(false, false, "/tmp/shot.png".to_owned()))
                    .await
                    .unwrap_err(),
                "ScreenshotArea" => proxy
                    .call_method(
                        "ScreenshotArea",
                        &(0i32, 0i32, 10i32, 10i32, false, "/tmp/shot.png".to_owned()),
                    )
                    .await
                    .unwrap_err(),
                "ScreenshotWindow" => proxy
                    .call_method("ScreenshotWindow", &(true, false, false, "/tmp/shot.png".to_owned()))
                    .await
                    .unwrap_err(),
                "SelectArea" => proxy.call_method("SelectArea", &()).await.unwrap_err(),
                "FlashArea" => proxy
                    .call_method("FlashArea", &(0i32, 0i32, 10i32, 10i32))
                    .await
                    .unwrap_err(),
                "InteractiveScreenshot" => proxy.call_method("InteractiveScreenshot", &()).await.unwrap_err(),
                "PickColor" => proxy.call_method("PickColor", &()).await.unwrap_err(),
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
        assert_eq!(refused, 7, "every member of the interface is declared and refused");
    });
}

#[test]
fn an_adapter_registers_once() {
    // The failure mode: a second export replaces the object behind a live name.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let connection = connect(&address).await;
        let service = ScreenshotService::new().expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        let again = export(&connection, &service).await;
        assert!(
            matches!(again, Err(ScreenshotError::Exported)),
            "the second export is refused, not a second object server entry"
        );
    });
}
