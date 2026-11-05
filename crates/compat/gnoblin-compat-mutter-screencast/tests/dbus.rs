//! The adapter over a private bus, with a fake IPC host as the compositor's half.
//!
//! Every test starts its own `dbus-daemon` (see `support/bus.rs`), so nothing here can
//! reach the developer's session bus. The caller is a plain `zbus::Proxy`, which is
//! what `xdg-desktop-portal-gnome`'s own proxy resolves to: the interface names, the
//! argument order and the property names are the ones in the vendored XML.
//!
//! The one fake is the capture seam: [`CaptureBridge`] is the compositor's half, and
//! the test host records what the adapter asked it to open and close. What the
//! compositor's capture table does with a session is `gnoblin-capture`'s, and the
//! adapter cannot see it either.

mod support;

use std::collections::HashMap;
use std::sync::Arc;

use futures_lite::future;
use gnoblin_compat_core::{CallerWatcher, Mode, Support, SupportMatrix};
use gnoblin_compat_mutter_screencast::{
    BUS_NAME, INTERFACE, MATRIX, SESSION_INTERFACE, STREAM_INTERFACE, ScreenCastService, export, watch_callers,
};
use support::SharedHost;
use support::bus::Bus;
use support::until;
use zbus::{Connection, ObjectPath, OwnedVariant, Proxy};

/// A property dict, as the interface declares it.
type Props = HashMap<String, OwnedVariant>;

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

/// One exported adapter, its fake host and its caller.
struct Fixture {
    connection: Connection,
    service: ScreenCastService,
    host: Arc<support::Host>,
    caller: Connection,
}

impl Fixture {
    async fn new(address: &str, mode: Mode) -> Fixture {
        let connection = connect(address).await;
        let caller = connect(address).await;
        let host = Arc::new(support::Host::default());
        let service = ScreenCastService::new(Box::new(SharedHost(Arc::clone(&host)))).expect("the matrix loads");
        service.set_mode(mode);
        export(&connection, &service).await.expect("the name is free");
        Fixture {
            connection,
            service,
            host,
            caller,
        }
    }

    async fn proxy(&self) -> Proxy<'static> {
        Proxy::new(
            &self.caller,
            BUS_NAME,
            gnoblin_compat_mutter_screencast::PATH,
            INTERFACE,
        )
        .await
        .expect("the interface is exported")
    }

    async fn session(&self, path: &ObjectPath<'_>) -> Proxy<'static> {
        Proxy::new(&self.caller, BUS_NAME, path.to_string(), SESSION_INTERFACE)
            .await
            .expect("the session object is exported")
    }

    async fn stream(&self, path: &ObjectPath<'_>) -> Proxy<'static> {
        Proxy::new(&self.caller, BUS_NAME, path.to_string(), STREAM_INTERFACE)
            .await
            .expect("the stream object is exported")
    }

    async fn create(&self) -> ObjectPath<'static> {
        let reply = self
            .proxy()
            .await
            .call_method("CreateSession", &(Props::new()))
            .await
            .expect("CreateSession is supported in unmediated mode");
        reply
            .body()
            .deserialize::<zbus::OwnedObjectPath>()
            .expect("a session path")
            .into()
    }
}

#[test]
fn the_default_mode_refuses_every_capture_call() {
    // The failure mode D13 names: a caller opens a capture session on the session bus
    // without consent. `portal` refuses every capture member with the portal rule, while
    // the version property still answers.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Portal).await;
        let proxy = fixture.proxy().await;
        let error = proxy
            .call_method("CreateSession", &(Props::new()))
            .await
            .expect_err("portal mode refuses capture");
        let message = refusal(error);
        assert!(
            message.contains("capture-portal-only"),
            "the refusal names the portal rule: {message}"
        );
        assert_eq!(fixture.host.opened(), 0, "a refused call never reaches the compositor");

        let version: i32 = proxy
            .get_property("Version")
            .await
            .expect("a read answers in portal mode");
        assert_eq!(version, 4, "the version is Mutter 51.0's ScreenCast version");
    });
}

#[test]
fn a_started_session_opens_and_closes_the_compositors_capture() {
    // The real path: a session is created, started (one `capture.open`), a monitor stream
    // is recorded, the stream refuses with the missing PipeWire producer, and stopping
    // the session closes the compositor's session exactly once.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Unmediated).await;
        let session_path = fixture.create().await;
        let session = fixture.session(&session_path).await;
        session.call_method("Start", &()).await.expect("the session starts");

        let unique = fixture.caller.unique_name().expect("the caller has a name").to_string();
        let opened = fixture
            .host
            .opened
            .lock()
            .expect("the host lock is not poisoned")
            .clone();
        assert_eq!(
            opened,
            vec![(unique, String::new())],
            "the compositor was told who opened it"
        );

        let reply = session
            .call_method("RecordMonitor", &("eDP-1", Props::new()))
            .await
            .expect("the stream object is created as upstream");
        let stream_path = reply
            .body()
            .deserialize::<zbus::OwnedObjectPath>()
            .expect("a stream path");
        let stream = fixture.stream(&stream_path).await;
        let parameters: Props = stream
            .get_property("Parameters")
            .await
            .expect("Parameters is partial, not absent");
        assert_eq!(
            parameters.get("output-name").map(|value| &**value),
            Some(&zbus::Variant::Str(zbus::Str::from("eDP-1"))),
            "the connector is reported verbatim"
        );

        let error = stream
            .call_method("Start", &())
            .await
            .expect_err("no PipeWire producer exists");
        assert!(
            refusal(error).contains("unsupported-pipewire"),
            "the refusal names the missing producer"
        );

        session.call_method("Stop", &()).await.expect("the session stops");
        assert_eq!(fixture.service.session_count(), 0);
        assert_eq!(
            fixture.service.stream_count(),
            0,
            "stopping a session removes its streams"
        );
        assert_eq!(fixture.host.closed(), 1, "the compositor's session ended exactly once");
    });
}

#[test]
fn a_refused_open_leaves_the_session_unstarted() {
    // The failure mode: the adapter records a started session whose compositor half does
    // not exist, so a later Stop closes a handle that was never handed out.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Unmediated).await;
        fixture.host.refuse.store(true, std::sync::atomic::Ordering::SeqCst);
        let session_path = fixture.create().await;
        let session = fixture.session(&session_path).await;
        let error = session
            .call_method("Start", &())
            .await
            .expect_err("the compositor refused the capture");
        assert!(refusal(error).contains("refused the capture session"));
        assert_eq!(fixture.host.closed(), 0, "nothing was opened, so nothing is closed");
        session
            .call_method("Stop", &())
            .await
            .expect("a session that never started still stops");
        assert_eq!(fixture.host.closed(), 0);
    });
}

#[test]
fn a_session_ends_with_its_caller() {
    // The plan's rule: a session ends when the caller that opened it disconnects. The
    // watcher loop is the only thing that can notice, so it runs beside the scenario.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Unmediated).await;
        let session_path = fixture.create().await;
        let session = fixture.session(&session_path).await;
        session.call_method("Start", &()).await.expect("the session starts");
        assert_eq!(fixture.service.session_count(), 1);
        let caller = fixture.caller;
        let service = fixture.service.clone();
        // Subscribe before the caller leaves: a departure before the match rule exists
        // raises no signal, so the subscription has to precede the scenario.
        let subscription = CallerWatcher::new(&fixture.connection)
            .await
            .expect("the bus accepts the match rule");
        let watcher = watch_callers(subscription, fixture.service.clone());
        let scenario = async move {
            // The proxy holds its own clone of the caller's connection, so both go.
            drop(session);
            drop(caller);
            until(|| service.session_count() == 0).await;
            assert_eq!(
                fixture.host.closed(),
                1,
                "the caller's departure closed the compositor session"
            );
        };
        future::or(watcher, scenario).await;
    });
}

#[test]
fn locking_ends_every_session_and_refuses_a_new_one() {
    // The lock embargo: a session opened before the lock ends at the lock rather than
    // pausing, and a new capture while locked is refused.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Unmediated).await;
        let session_path = fixture.create().await;
        let session = fixture.session(&session_path).await;
        session.call_method("Start", &()).await.expect("the session starts");
        let ended = fixture.service.set_lock(gnoblin_compat_core::LockState::Locked).await;
        assert_eq!(ended, 1);
        assert_eq!(fixture.service.session_count(), 0);
        assert_eq!(fixture.host.closed(), 1);

        let error = fixture
            .proxy()
            .await
            .call_method("CreateSession", &(Props::new()))
            .await
            .expect_err("capture is refused while locked");
        assert!(
            refusal(error).contains("locked-capture"),
            "the refusal names the lock embargo"
        );
    });
}

#[test]
fn a_foreign_caller_cannot_use_a_session() {
    // The failure mode: any same-user process that learns a session path can stop another
    // caller's capture or read its parameters.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Unmediated).await;
        let session_path = fixture.create().await;
        let intruder = connect(&address).await;
        let proxy = Proxy::new(&intruder, BUS_NAME, session_path.to_string(), SESSION_INTERFACE)
            .await
            .expect("the object exists");
        let error = proxy
            .call_method("Stop", &())
            .await
            .expect_err("another caller cannot stop the session");
        assert!(
            !error.to_string().is_empty() || matches!(error, zbus::Error::MethodError(..)),
            "the refusal is a D-Bus error, not a silent success: {error:?}"
        );
        assert_eq!(fixture.service.session_count(), 1, "the session is still the owner's");
    });
}

#[test]
fn an_adapter_registers_once() {
    // The failure mode: a second export replaces the object behind a live name while the
    // first one's signal emitter still points at the path.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Unmediated).await;
        let again = export(&fixture.connection, &fixture.service).await;
        assert!(
            matches!(again, Err(gnoblin_compat_mutter_screencast::ScreenCastError::Exported)),
            "the second export is refused, not a second object server entry"
        );
    });
}

#[test]
fn the_matrix_matches_what_the_adapter_serves() {
    // Every declared member is exercised in `unmediated` mode, where the gate allows every
    // class, so what answers is the matrix and nothing else. A supported or partial member
    // answers; an unsupported one answers with the matrix's own reason; every signal is in
    // the served interface.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let fixture = Fixture::new(&address, Mode::Unmediated).await;
        let proxy = fixture.proxy().await;
        let matrix = SupportMatrix::from_json(MATRIX).expect("the checked-in manifest is readable");
        let session_path = fixture.create().await;
        let session = fixture.session(&session_path).await;

        assert_members(
            &proxy,
            &matrix,
            INTERFACE,
            gnoblin_compat_mutter_screencast::PATH,
            &fixture.caller,
        )
        .await;

        // Session members, with a fresh session for the calls that consume one. The XML is
        // read before the loop because `Stop` removes the object it describes.
        let session_xml = introspection(&fixture.caller, &session_path.to_string()).await;
        for member in matrix
            .interface(SESSION_INTERFACE)
            .expect("the manifest declares the session interface")
            .members()
        {
            if member.kind() == gnoblin_compat_core::Kind::Signal {
                assert!(
                    session_xml.contains(&format!("name=\"{}\"", member.name())),
                    "`{}` is declared and must be in the served session interface",
                    member.name()
                );
                continue;
            }
            let error = match member.name() {
                "Start" => session.clone().call_method("Start", &()).await.err(),
                "Stop" => session.clone().call_method("Stop", &()).await.err(),
                "RecordMonitor" => session
                    .clone()
                    .call_method("RecordMonitor", &("eDP-1", Props::new()))
                    .await
                    .err(),
                "RecordWindow" => {
                    let properties: Props = HashMap::from([("window-id".to_owned(), OwnedVariant::from(7u64))]);
                    session.clone().call_method("RecordWindow", &(properties,)).await.err()
                }
                "RecordArea" => session
                    .clone()
                    .call_method("RecordArea", &(0i32, 0i32, 640i32, 480i32, Props::new()))
                    .await
                    .err(),
                "RecordVirtual" => session
                    .clone()
                    .call_method("RecordVirtual", &(Props::new()))
                    .await
                    .err(),
                other => panic!("the matrix declares session member `{other}` and this test does not call it"),
            };
            assert_support(member.support(), error);
        }

        // Stream members, each on a fresh stream so `Stop` cannot hide `Parameters`.
        let stream_session_path = fixture.create().await;
        let stream_session = fixture.session(&stream_session_path).await;
        for member in matrix
            .interface(STREAM_INTERFACE)
            .expect("the manifest declares the stream interface")
            .members()
        {
            let reply = stream_session
                .call_method("RecordMonitor", &("DP-1", Props::new()))
                .await
                .expect("a stream to exercise");
            let stream_path = reply
                .body()
                .deserialize::<zbus::OwnedObjectPath>()
                .expect("a stream path");
            if member.kind() == gnoblin_compat_core::Kind::Signal {
                assert!(
                    introspection(&fixture.caller, &stream_path.to_string())
                        .await
                        .contains(&format!("name=\"{}\"", member.name())),
                    "`{}` is declared and must be in the served stream interface",
                    member.name()
                );
                continue;
            }
            let stream = fixture.stream(&stream_path).await;
            let error = match member.name() {
                "Start" => stream.clone().call_method("Start", &()).await.err(),
                "Stop" => stream.clone().call_method("Stop", &()).await.err(),
                "Parameters" => stream.clone().get_property::<Props>("Parameters").await.err(),
                other => panic!("the matrix declares stream member `{other}` and this test does not call it"),
            };
            assert_support(member.support(), error);
        }
    });
}

/// Check the main interface's non-signal members.
async fn assert_members(
    proxy: &Proxy<'_>,
    matrix: &SupportMatrix,
    interface: &str,
    path: &str,
    connection: &Connection,
) {
    for member in matrix
        .interface(interface)
        .expect("the manifest declares the interface this adapter serves")
        .members()
    {
        if member.kind() == gnoblin_compat_core::Kind::Signal {
            assert!(
                introspection(connection, path)
                    .await
                    .contains(&format!("name=\"{}\"", member.name())),
                "`{}` is declared and must be in the served interface",
                member.name()
            );
            continue;
        }
        let error = match member.name() {
            "CreateSession" => proxy.call_method("CreateSession", &(Props::new())).await.err(),
            "Version" => proxy.get_property::<i32>("Version").await.err(),
            other => panic!("the matrix declares `{other}` and this test does not call it"),
        };
        assert_support(member.support(), error);
    }
}

/// Assert that a call's outcome matches its declaration.
fn assert_support(support: &Support, error: Option<zbus::Error>) {
    match (support, error) {
        (Support::Unsupported { reason }, Some(error)) => {
            let message = refusal(error);
            assert!(
                message.contains("unsupported"),
                "an unsupported member must refuse with the rule, not `{message}`"
            );
            assert!(
                message.contains(reason.as_str()),
                "the refusal quotes the matrix: `{message}` does not contain `{reason}`"
            );
        }
        (Support::Unsupported { reason }, None) => {
            panic!("a member declared unsupported ({reason}) answered successfully")
        }
        (Support::Supported | Support::Partial { .. }, Some(error)) => {
            panic!("a declared member refused: {}", refusal(error))
        }
        (Support::Supported | Support::Partial { .. }, None) => {}
    }
}

/// The introspection XML the object server publishes for `path`.
async fn introspection(connection: &Connection, path: &str) -> String {
    let proxy = Proxy::new(
        connection,
        BUS_NAME,
        path.to_owned(),
        "org.freedesktop.DBus.Introspectable",
    )
    .await
    .expect("the introspectable interface is there");
    let reply = proxy
        .call_method("Introspect", &())
        .await
        .expect("the object server introspects itself");
    reply.body().deserialize::<String>().expect("the XML is a string")
}
