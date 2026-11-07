//! The adapter over a private bus, with the loopback as its fake IPC server.
//!
//! Every test starts its own `dbus-daemon` (see `support/bus.rs`), so nothing here can
//! reach the developer's session bus, and the state the adapter reports comes from a
//! real object graph over the real `gnoblin-client` wire path. The caller is a plain
//! `zbus::Proxy`, which is what GNOME Settings' own proxy resolves to: the interface
//! names, the argument order and the property names are the ones in the vendored XML.
//!
//! The write seam is the one fake: it records the calls the adapter makes instead of
//! sending them, because what a test has to prove about `ApplyMonitorsConfig` is which
//! output objects it addresses and with what, and the client half is proved by the read
//! tests over the same loopback.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_lite::StreamExt as _;
use gnoblin_client::{Loopback, Server};
use gnoblin_compat_core::{Mode, SupportMatrix};
use gnoblin_compat_mutter_displayconfig::{
    BUS_NAME, DisplayConfigService, DisplayError, DisplayLink, INTERFACE, MATRIX, OutputApplier, PATH, export,
};
use gnoblin_model::{Fields, TypeKey, Value};
use support::bus::Bus;
use support::{VERSION, graph, output_properties, unlocked};
use zbus::{Connection, OwnedVariant, Proxy};

/// A property dict, as the interface declares it.
type Props = HashMap<String, OwnedVariant>;

/// One mode: id, width, height, refresh, preferred scale, supported scales, properties.
type ModeEntry = (String, i32, i32, f64, f64, Vec<f64>, Props);

/// One monitor: connector, vendor, product, serial; modes; properties.
type MonitorEntry = ((String, String, String, String), Vec<ModeEntry>, Props);

/// One logical monitor as `GetCurrentState` reports it: x, y, scale, transform,
/// primary, its monitors, properties.
type LogicalEntry = (i32, i32, f64, u32, bool, Vec<(String, String, String, String)>, Props);

/// One logical monitor as `ApplyMonitorsConfig` takes it: the same, but a monitor is a
/// connector, a mode id and a property dict.
type LogicalArg = (i32, i32, f64, u32, bool, Vec<(String, String, Props)>);

/// What `GetCurrentState` answers.
type CurrentState = (u32, Vec<MonitorEntry>, Vec<LogicalEntry>, Props);

/// The write seam, recording what the adapter asked for.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<(String, Fields)>>,
}

/// The seam as the service holds it: one recorder, shared with the test.
struct SharedRecorder(Arc<Recorder>);

impl OutputApplier for SharedRecorder {
    fn apply(&self, output: &str, fields: &Fields) -> Result<(), String> {
        let recorder = &self.0;
        recorder
            .calls
            .lock()
            .expect("the recorder lock is not poisoned")
            .push((output.to_owned(), fields.clone()));
        Ok(())
    }
}

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

/// One server, one link and one published state: the fixture every test starts from.
struct Fixture {
    server: Server,
    link: DisplayLink<Loopback>,
    primary: gnoblin_model::ObjectId,
    second: gnoblin_model::ObjectId,
    lock: gnoblin_model::ObjectId,
}

impl Fixture {
    /// Two outputs and an unlocked session, read through the loopback.
    fn new() -> Fixture {
        let mut server = Server::new(graph(), VERSION);
        let primary = server
            .create(TypeKey::gnoblin("output"), output_properties("eDP-1", 1.5, true))
            .expect("the output type and its properties are in the schema");
        let second = server
            .create(TypeKey::gnoblin("output"), output_properties("DP-1", 1.0, false))
            .expect("a second output is a second object");
        let lock = server
            .create(TypeKey::gnoblin("lock"), unlocked())
            .expect("the lock type and its property are in the schema");
        let client = server.connect(VERSION).expect("the versions match");
        let link = DisplayLink::connect(client).expect("the snapshot is a state this adapter can report");
        Fixture {
            server,
            link,
            primary,
            second,
            lock,
        }
    }

    /// Publish whatever the link has folded.
    async fn publish(&mut self, service: &DisplayConfigService) {
        self.link.drain().expect("the graph's observations are readable");
        service
            .publish(self.link.state().clone(), self.link.lock())
            .await
            .expect("the signal is emitted");
    }
}

/// Read the whole state from a proxy.
async fn state(proxy: &Proxy<'_>) -> CurrentState {
    let reply = proxy
        .call_method("GetCurrentState", &())
        .await
        .expect("GetCurrentState is supported");
    reply
        .body()
        .deserialize::<CurrentState>()
        .expect("the reply is the declared signature")
}

#[test]
fn get_current_state_matches_the_output_objects_and_follows_a_change() {
    // The failure mode: the adapter reports a layout of its own instead of the objects
    // the compositor published, or it keeps answering the snapshot it started from. The
    // first half is the snapshot, the second half is one property diff arriving as a
    // `MonitorsChanged` the caller can see.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let recorder = Arc::new(Recorder::default());
        let service =
            DisplayConfigService::new(Box::new(SharedRecorder(Arc::clone(&recorder)))).expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;

        let proxy = caller(&connection).await;
        let mut changed = proxy
            .receive_signal("MonitorsChanged")
            .await
            .expect("the caller subscribes to the signal");
        let (serial, monitors, logical, _properties) = state(&proxy).await;

        assert_eq!(serial, service.serial(), "the serial is the one the adapter publishes");
        assert_eq!(
            monitors
                .iter()
                .map(|(connector, ..)| connector.0.as_str())
                .collect::<Vec<_>>(),
            vec!["DP-1", "eDP-1"],
            "every output object is a monitor, in connector order"
        );
        let (_, modes, _) = &monitors[1];
        assert_eq!(
            modes
                .iter()
                .map(|(id, width, height, ..)| (id.as_str(), *width, *height))
                .collect::<Vec<_>>(),
            vec![("1920x1080@60.000", 1920, 1080), ("1280x720@60.000", 1280, 720)],
            "the modes are the output's own list, parsed from the id the schema documents"
        );
        assert_eq!(
            logical
                .iter()
                .map(|(_, _, scale, _, primary, monitors, _)| (monitors[0].0.as_str(), *scale, *primary))
                .collect::<Vec<_>>(),
            vec![("DP-1", 1.0, false), ("eDP-1", 1.5, true)],
            "one logical monitor per enabled output, with the object's scale and primary flag"
        );

        fixture
            .server
            .set(fixture.primary, "scale", Value::Double(2.0))
            .expect("scale is a declared double property");
        fixture.publish(&service).await;
        let signalled = futures_lite::future::race(async { changed.next().await.map(|_| ()) }, async {
            for _ in 0..10_000 {
                futures_lite::future::yield_now().await;
            }
            None
        })
        .await;
        assert!(
            signalled.is_some(),
            "MonitorsChanged was not emitted for a changed output"
        );

        let (serial, _monitors, logical, _properties) = state(&proxy).await;
        assert_ne!(serial, 1, "the serial advances with the state");
        assert_eq!(
            logical[1].2, 2.0,
            "the second read reports the scale the graph published"
        );
    });
}

#[test]
fn an_apply_is_refused_in_the_default_mode() {
    // The failure mode: a write is allowed because the adapter forgot which mode it
    // serves in. D13's default is `portal`, so the apply is refused by name and the
    // property a caller uses to decide whether to offer a configuration says so.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let recorder = Arc::new(Recorder::default());
        let service =
            DisplayConfigService::new(Box::new(SharedRecorder(Arc::clone(&recorder)))).expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;

        let allowed: bool = proxy
            .get_property("ApplyMonitorsConfigAllowed")
            .await
            .expect("the property is readable");
        assert!(!allowed, "a write is not allowed in the default mode");

        let (serial, ..) = state(&proxy).await;
        let logical: Vec<LogicalArg> = vec![(
            0i32,
            0i32,
            1.5f64,
            0u32,
            true,
            vec![("eDP-1".to_owned(), "1920x1080@60.000".to_owned(), Props::new())],
        )];
        let error = proxy
            .call_method("ApplyMonitorsConfig", &(serial, 1u32, logical, Props::new()))
            .await
            .expect_err("a write is refused in the default mode");
        let message = refusal(error);
        assert!(
            message.contains("write-not-enabled"),
            "the refusal names the rule: {message}"
        );
        assert!(
            recorder
                .calls
                .lock()
                .expect("the recorder lock is not poisoned")
                .is_empty(),
            "a refused apply never reaches the write seam"
        );

        // The gate's precedence: a write is refused by the mode before the matrix gets a
        // say, even where the matrix would also refuse it.
        let error = proxy
            .call_method(
                "ApplyConfiguration",
                &(
                    serial,
                    true,
                    Vec::<(u32, i32, i32, i32, u32, Vec<u32>, Props)>::new(),
                    Vec::<(u32, Props)>::new(),
                ),
            )
            .await
            .expect_err("the legacy write is refused too");
        let message = refusal(error);
        assert!(
            message.contains("write-not-enabled"),
            "the mode refuses a write before the matrix does: {message}"
        );
    });
}

#[test]
fn an_allowed_apply_addresses_every_output_object() {
    // The other half of the write path: when the mode allows writes, the plan becomes one
    // call per output, including a disable for the output the caller left out, so a
    // removed monitor really goes away.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let recorder = Arc::new(Recorder::default());
        let service =
            DisplayConfigService::new(Box::new(SharedRecorder(Arc::clone(&recorder)))).expect("the matrix loads");
        service.set_mode(Mode::Unmediated);
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;

        let allowed: bool = proxy
            .get_property("ApplyMonitorsConfigAllowed")
            .await
            .expect("the property is readable");
        assert!(allowed, "the property answers the gate's own question");

        let (serial, ..) = state(&proxy).await;
        let logical: Vec<LogicalArg> = vec![(
            0i32,
            0i32,
            1.0f64,
            0u32,
            true,
            vec![("eDP-1".to_owned(), "1280x720@60.000".to_owned(), Props::new())],
        )];
        proxy
            .call_method("ApplyMonitorsConfig", &(serial, 1u32, logical, Props::new()))
            .await
            .expect("the write is allowed in `unmediated`");

        let calls = recorder
            .calls
            .lock()
            .expect("the recorder lock is not poisoned")
            .clone();
        assert_eq!(
            calls.iter().map(|(output, _)| output.as_str()).collect::<Vec<_>>(),
            vec!["DP-1", "eDP-1"],
            "one call per output, so an output the caller left out is switched off"
        );
        let applied = &calls[1].1;
        assert_eq!(applied.get("mode"), Some(&Value::Text("1280x720@60.000".to_owned())));
        assert_eq!(applied.get("enabled"), Some(&Value::Bool(true)));
        assert_eq!(applied.get("primary"), Some(&Value::Bool(true)));
        assert_eq!(
            calls[0].1.get("enabled"),
            Some(&Value::Bool(false)),
            "the output the configuration does not mention is disabled"
        );
    });
}

#[test]
fn an_apply_with_a_stale_serial_is_refused() {
    // The failure mode: a caller applies a configuration it read before a hotplug, and
    // the adapter overwrites the newer layout. The serial is what catches it.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let recorder = Arc::new(Recorder::default());
        let service =
            DisplayConfigService::new(Box::new(SharedRecorder(Arc::clone(&recorder)))).expect("the matrix loads");
        service.set_mode(Mode::Unmediated);
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;
        let (serial, ..) = state(&proxy).await;

        fixture
            .server
            .set(fixture.second, "enabled", Value::Bool(false))
            .expect("enabled is a declared bool property");
        fixture.publish(&service).await;

        let error = proxy
            .call_method(
                "ApplyMonitorsConfig",
                &(serial, 1u32, Vec::<LogicalArg>::new(), Props::new()),
            )
            .await
            .expect_err("a configuration read before the change is refused");
        let message = refusal(error);
        assert!(
            message.contains("is not the current serial"),
            "the refusal names both serials: {message}"
        );
        assert!(
            recorder
                .calls
                .lock()
                .expect("the recorder lock is not poisoned")
                .is_empty(),
            "nothing was written"
        );
    });
}

#[test]
fn a_read_is_allowed_while_the_session_is_locked() {
    // The failure mode in the other direction: the lock embargo refuses everything. It
    // is a capture embargo, so a caller asking what the monitors are still gets an
    // answer, which is what a lock screen's own display settings need.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = DisplayConfigService::new(Box::new(SharedRecorder(Arc::new(Recorder::default()))))
            .expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        fixture
            .server
            .set(fixture.lock, "locked", Value::Bool(true))
            .expect("locked is a declared bool property");
        fixture.publish(&service).await;
        assert_eq!(
            fixture.link.lock(),
            gnoblin_compat_core::LockState::Locked,
            "the link read the lock object"
        );
        let proxy = caller(&connection).await;
        let (_, monitors, ..) = state(&proxy).await;
        assert_eq!(monitors.len(), 2, "reads answer while the session is locked");
    });
}

#[test]
fn the_gate_refuses_a_disabled_adapter() {
    // The failure mode: an adapter that is off still answers, because the mode is only
    // consulted on the write path. `off` refuses every member.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = DisplayConfigService::new(Box::new(SharedRecorder(Arc::new(Recorder::default()))))
            .expect("the matrix loads");
        service.set_mode(Mode::Off);
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;

        let error = proxy
            .call_method("GetCurrentState", &())
            .await
            .expect_err("a disabled adapter answers nothing");
        let message = refusal(error);
        assert!(message.contains("mode-off"), "the refusal names the mode: {message}");
    });
}

#[test]
fn an_unsupported_member_is_refused_by_name() {
    // The failure mode the plan names: a caller depends on a method that is not served
    // and learns nothing. Every member the matrix calls `unsupported` answers with a
    // D-Bus error that quotes the matrix's rule, so the caller's error and
    // `cargo xtask compat-report` agree.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let mut fixture = Fixture::new();
        let connection = connect(&address).await;
        let service = DisplayConfigService::new(Box::new(SharedRecorder(Arc::new(Recorder::default()))))
            .expect("the matrix loads");
        // `unmediated` is the mode in which the gate allows every class, so what refuses
        // these members is the matrix and nothing else.
        service.set_mode(Mode::Unmediated);
        export(&connection, &service).await.expect("the name is free");
        fixture.publish(&service).await;
        let proxy = caller(&connection).await;
        let (serial, ..) = state(&proxy).await;

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
                "GetResources" => proxy.call_method("GetResources", &()).await.unwrap_err(),
                "ApplyConfiguration" => proxy
                    .call_method(
                        "ApplyConfiguration",
                        &(
                            serial,
                            true,
                            Vec::<(u32, i32, i32, i32, u32, Vec<u32>, Props)>::new(),
                            Vec::<(u32, Props)>::new(),
                        ),
                    )
                    .await
                    .unwrap_err(),
                "ChangeBacklight" => proxy
                    .call_method("ChangeBacklight", &(serial, 0u32, 10i32))
                    .await
                    .unwrap_err(),
                "SetBacklight" => proxy
                    .call_method("SetBacklight", &(serial, "eDP-1".to_owned(), 10i32))
                    .await
                    .unwrap_err(),
                "GetCrtcGamma" => proxy.call_method("GetCrtcGamma", &(serial, 0u32)).await.unwrap_err(),
                "SetCrtcGamma" => proxy
                    .call_method("SetCrtcGamma", &(serial, 0u32, vec![1u16], vec![2u16], vec![3u16]))
                    .await
                    .unwrap_err(),
                "SetOutputCTM" => proxy
                    .call_method(
                        "SetOutputCTM",
                        &(serial, 0u32, (1u64, 0u64, 0u64, 0u64, 1u64, 0u64, 0u64, 0u64, 1u64)),
                    )
                    .await
                    .unwrap_err(),
                name @ ("Backlight" | "HasExternalMonitor" | "PowerSaveMode") => {
                    proxy.get_property::<OwnedVariant>(name).await.unwrap_err()
                }
                other => panic!("the matrix declares `{other}` unsupported and this test does not call it"),
            };
            let message = match error {
                zbus::Error::MethodError(_, Some(detail), _) => detail,
                other => other.to_string(),
            };
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
    // first one's signal emitter still points at the path, so two halves of one process
    // answer the same caller.
    let bus = Bus::start();
    let address = bus.address().to_owned();
    zbus::block_on(async move {
        let connection = connect(&address).await;
        let service = DisplayConfigService::new(Box::new(SharedRecorder(Arc::new(Recorder::default()))))
            .expect("the matrix loads");
        export(&connection, &service).await.expect("the name is free");
        let again = export(&connection, &service).await;
        assert!(
            matches!(again, Err(DisplayError::Exported)),
            "the second export is refused, not a second object server entry"
        );
    });
}
