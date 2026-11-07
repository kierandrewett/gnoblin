//! One test per failure mode that does not need a bus, driven through the loopback.
//!
//! The state the adapter reports is derived from objects the compositor publishes, so
//! the interesting failures are all about a graph that says something the adapter cannot
//! honestly report. Every test here builds that graph through the schema — a property
//! name this crate gets wrong is a `ModelError`, not a silently missing value — and then
//! asserts the refusal rather than a monitor with a zero in it.

mod support;

use gnoblin_client::Server;
use gnoblin_compat_dbus_mutter_displayconfig::{
    ApplyMethod, ApplyPlan, DisplayError, DisplayLink, DisplayState, LogicalMonitor, MonitorAssignment,
};
use gnoblin_model::{ObjectId, TypeKey, Value};
use support::{VERSION, graph, output_properties, unlocked};

/// A server with the lock object, so a link can connect.
fn server() -> Server {
    let mut server = Server::new(graph(), VERSION);
    server
        .create(TypeKey::gnoblin("lock"), unlocked())
        .expect("the lock type is in the schema");
    server
}

/// Create an output with one property replaced.
fn output_with(server: &mut Server, name: &str, property: &str, value: Value) -> ObjectId {
    let mut properties = output_properties(name, 1.0, false);
    properties.insert(property.to_owned(), value);
    server
        .create(TypeKey::gnoblin("output"), properties)
        .expect("the output type and its properties are in the schema")
}

#[test]
fn two_outputs_cannot_claim_one_connector() {
    // The failure mode the plan names for the compat crates: two views disagree about an
    // output's name. A graph that publishes two objects with one connector is refused,
    // because a caller's apply names a connector and could not say which display it
    // meant.
    let mut server = server();
    output_with(&mut server, "DP-1", "scale", Value::Double(1.0));
    output_with(&mut server, "DP-1", "scale", Value::Double(2.0));
    let client = server.connect(VERSION).expect("the versions match");

    match DisplayLink::connect(client) {
        Err(DisplayError::DuplicateConnector { name }) => assert_eq!(name, "DP-1"),
        other => panic!("a repeated connector is refused, got {other:?}"),
    }
}

#[test]
fn a_connector_is_reported_verbatim() {
    // The other half of the same failure mode: the adapter reports the schema's `name`
    // and does not normalise it. A second adapter reading the same objects therefore
    // cannot see a different spelling for one display.
    let mut server = server();
    output_with(&mut server, "HDMI.A-1", "scale", Value::Double(1.0));
    let client = server.connect(VERSION).expect("the versions match");
    let link = DisplayLink::connect(client).expect("one output is a state");

    assert_eq!(
        link.state()
            .outputs
            .iter()
            .map(|output| output.name.as_str())
            .collect::<Vec<_>>(),
        vec!["HDMI.A-1"],
        "the connector is the schema's name, never a normalised form of it"
    );
}

#[test]
fn a_malformed_mode_is_refused() {
    // A mode id is the whole description, so a graph that publishes one without a size is
    // a graph this adapter cannot report: a mode of 0x0 is one a caller will apply.
    let mut server = server();
    output_with(
        &mut server,
        "eDP-1",
        "modes",
        Value::List(vec![Value::Text("1920x1080".to_owned())]),
    );
    let client = server.connect(VERSION).expect("the versions match");

    match DisplayLink::connect(client) {
        Err(DisplayError::Unreadable { property, .. }) => assert_eq!(property, "modes"),
        other => panic!("a malformed mode id is refused, got {other:?}"),
    }
}

#[test]
fn the_current_mode_must_be_one_of_the_outputs_modes() {
    // The failure mode: `GetCurrentState` reports a mode no caller can select back,
    // because the current mode is not in the list it was published with.
    let mut server = server();
    output_with(&mut server, "eDP-1", "mode", Value::Text("800x600@60.000".to_owned()));
    let client = server.connect(VERSION).expect("the versions match");

    match DisplayLink::connect(client) {
        Err(DisplayError::UnknownMode { connector, mode }) => {
            assert_eq!(connector, "eDP-1");
            assert_eq!(mode, "800x600@60.000");
        }
        other => panic!("a current mode outside the list is refused, got {other:?}"),
    }
}

#[test]
fn a_graph_without_a_lock_object_is_refused() {
    // The failure mode: the gate's embargo is built from a guess because nothing told the
    // adapter whether the session is locked. It fails closed at connect instead.
    let mut server = Server::new(graph(), VERSION);
    output_with(&mut server, "eDP-1", "scale", Value::Double(1.0));
    let client = server.connect(VERSION).expect("the versions match");

    match DisplayLink::connect(client) {
        Err(DisplayError::Unreadable { object, property, .. }) => {
            assert_eq!(object, "lock");
            assert_eq!(property, "locked");
        }
        other => panic!("a missing lock object is refused, got {other:?}"),
    }
}

#[test]
fn an_apply_names_only_known_connectors() {
    // The failure mode: an apply half-succeeds because the first connector was written
    // before the second turned out not to exist. Validation happens first, so nothing is
    // written for a plan the graph cannot honour.
    let mut server = server();
    output_with(&mut server, "eDP-1", "scale", Value::Double(1.0));
    let client = server.connect(VERSION).expect("the versions match");
    let link = DisplayLink::connect(client).expect("one output is a state");

    let plan = ApplyPlan {
        serial: 1,
        method: ApplyMethod::Temporary,
        logical: vec![LogicalMonitor {
            x: 0,
            y: 0,
            scale: 1.0,
            transform: 0,
            primary: true,
            monitors: vec![MonitorAssignment {
                connector: "DP-9".to_owned(),
                mode: "1920x1080@60.000".to_owned(),
            }],
        }],
    };
    match plan.validate(link.state(), 1) {
        Err(DisplayError::UnknownConnector { name }) => assert_eq!(name, "DP-9"),
        other => panic!("an unknown connector is refused, got {other:?}"),
    }
}

#[test]
fn an_apply_names_only_supported_modes() {
    // The failure mode: a caller asks for a mode the output does not list, and the
    // compositor is left to refuse it after other outputs were already written.
    let mut server = server();
    output_with(&mut server, "eDP-1", "scale", Value::Double(1.0));
    let client = server.connect(VERSION).expect("the versions match");
    let link = DisplayLink::connect(client).expect("one output is a state");

    let plan = ApplyPlan {
        serial: 1,
        method: ApplyMethod::Persistent,
        logical: vec![LogicalMonitor {
            x: 0,
            y: 0,
            scale: 1.0,
            transform: 0,
            primary: true,
            monitors: vec![MonitorAssignment {
                connector: "eDP-1".to_owned(),
                mode: "3840x2160@30.000".to_owned(),
            }],
        }],
    };
    match plan.validate(link.state(), 1) {
        Err(DisplayError::UnsupportedMode { connector, mode }) => {
            assert_eq!(connector, "eDP-1");
            assert_eq!(mode, "3840x2160@30.000");
        }
        other => panic!("an unsupported mode is refused, got {other:?}"),
    }
}

#[test]
fn a_connector_appears_in_one_logical_monitor() {
    // The failure mode: a caller mirrors an output into two logical monitors, and the
    // compositor is asked to place one display twice.
    let mut server = server();
    output_with(&mut server, "eDP-1", "scale", Value::Double(1.0));
    let client = server.connect(VERSION).expect("the versions match");
    let link = DisplayLink::connect(client).expect("one output is a state");

    let monitor = MonitorAssignment {
        connector: "eDP-1".to_owned(),
        mode: "1920x1080@60.000".to_owned(),
    };
    let plan = ApplyPlan {
        serial: 1,
        method: ApplyMethod::Temporary,
        logical: vec![
            LogicalMonitor {
                x: 0,
                y: 0,
                scale: 1.0,
                transform: 0,
                primary: true,
                monitors: vec![monitor.clone()],
            },
            LogicalMonitor {
                x: 1920,
                y: 0,
                scale: 1.0,
                transform: 0,
                primary: false,
                monitors: vec![monitor],
            },
        ],
    };
    match plan.validate(link.state(), 1) {
        Err(DisplayError::RepeatedConnector { name }) => assert_eq!(name, "eDP-1"),
        other => panic!("a repeated connector is refused, got {other:?}"),
    }
}

#[test]
fn an_output_the_plan_omits_is_disabled() {
    // The failure mode: a caller removes a monitor from its configuration and the display
    // stays lit, because the adapter only wrote the outputs the plan named. An apply is a
    // whole configuration, so every output gets a call.
    let mut server = server();
    output_with(&mut server, "eDP-1", "scale", Value::Double(1.0));
    output_with(&mut server, "DP-1", "scale", Value::Double(1.0));
    let client = server.connect(VERSION).expect("the versions match");
    let link = DisplayLink::connect(client).expect("two outputs are a state");
    let state: DisplayState = link.state().clone();

    let plan = ApplyPlan {
        serial: 1,
        method: ApplyMethod::Temporary,
        logical: vec![LogicalMonitor {
            x: 0,
            y: 0,
            scale: 1.0,
            transform: 0,
            primary: true,
            monitors: vec![MonitorAssignment {
                connector: "eDP-1".to_owned(),
                mode: "1920x1080@60.000".to_owned(),
            }],
        }],
    };
    let calls = plan.calls(&state);
    assert_eq!(
        calls
            .iter()
            .map(|(output, fields)| (output.as_str(), fields.get("enabled").cloned()))
            .collect::<Vec<_>>(),
        vec![("DP-1", Some(Value::Bool(false))), ("eDP-1", Some(Value::Bool(true)))]
    );
}
