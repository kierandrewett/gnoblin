//! What a document must be refused for, and what a refusal must say.
//!
//! Each test names one failure of the document layer: an unknown key, a wrong
//! shape, an out-of-range value, a removed setting, a silent alias, a conflict
//! between two sections. The assertions are on the fault and the path, not on the
//! wording, so a message can be reworded without rewriting the suite.

mod support;

use gnoblin_config::{Blur, Config, Fault, FocusMode, GestureWhen, Inherit, MAX_DEPTH, Path, TapButtonMap, Value};

use support::{flag, int, kitchen_sink, list, number, table, text};

fn problems(document: Value) -> Vec<(String, Fault)> {
    let report = Config::from_value(document).expect_err("the document must be refused");
    report
        .problems()
        .iter()
        .map(|problem| (problem.path.to_string(), problem.fault.clone()))
        .collect()
}

fn accepted_keys(problem: &(String, Fault)) -> Option<&'static [&'static str]> {
    match &problem.1 {
        Fault::UnknownKey { accepted } => Some(accepted),
        _ => None,
    }
}

#[test]
fn a_complete_document_is_accepted() {
    let config = Config::from_value(kitchen_sink()).expect("the reference document validates");
    assert!(config.deprecations().is_empty(), "no deprecated spelling is used");
    assert_eq!(config.document(), &kitchen_sink(), "the accepted document is the input");
}

#[test]
fn the_typed_values_are_available_without_reparsing() {
    let config = Config::from_value(kitchen_sink()).expect("valid");
    let compositor = config.compositor.as_ref().expect("compositor");
    assert_eq!(compositor.blur, Some(Blur::On));
    assert_eq!(compositor.debug, Some(false));
    let cursor = config.cursor.as_ref().expect("cursor");
    assert_eq!(cursor.theme.as_deref(), Some("default"));
    assert_eq!(cursor.size, Some(24));
    let workspaces = &config.workspaces;
    assert_eq!(workspaces.len(), 2);
    assert_eq!(workspaces[0].id.as_deref(), Some("main"));
    assert_eq!(workspaces[1].name.as_deref(), Some("Chat"));
    let protocols = config.protocols.as_ref().expect("protocols");
    assert_eq!(protocols.toggles.len(), 1);
    let gesture = &config.touchpad_gestures[0];
    assert_eq!(gesture.when, Some(GestureWhen::Unlocked), "when defaults to unlocked");
    assert_eq!(gesture.threshold, Some(48.0), "a swipe threshold defaults to 48");
    let mouse = config
        .input
        .as_ref()
        .and_then(|input| input.mouse.as_ref())
        .expect("mouse");
    assert_eq!(mouse.speed, Some(Inherit::Value(0.5)));
}

#[test]
fn an_unknown_key_names_its_section_and_the_accepted_keys() {
    let document = table([("compositor", table([("debug2", flag(true))]))]);
    let problems = problems(document);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, "compositor.debug2");
    let accepted = accepted_keys(&problems[0]).expect("an unknown key reports what is accepted");
    assert!(accepted.contains(&"debug"));
    assert!(accepted.contains(&"blur"));
}

#[test]
fn every_problem_is_reported_at_once() {
    let document = table([
        ("compositor", table([("debug2", flag(true))])),
        ("cursor", table([("size", int(0))])),
        ("window-management", table([("edge-tiling", text("yes"))])),
    ]);
    let problems = problems(document);
    let paths: Vec<&str> = problems.iter().map(|problem| problem.0.as_str()).collect();
    assert_eq!(
        paths,
        vec!["compositor.debug2", "cursor.size", "window-management.edge-tiling"],
        "one reload reports every problem, in document order"
    );
}

#[test]
fn a_wrong_shape_reports_what_was_expected_and_found() {
    let document = table([("cursor", table([("size", text("24"))]))]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "cursor.size");
    assert_eq!(
        problems[0].1,
        Fault::WrongType {
            expected: "an integer",
            found: "a string"
        }
    );
}

#[test]
fn an_out_of_range_value_reports_the_range() {
    let document = table([("cursor", table([("size", int(0))]))]);
    let problems = problems(document);
    assert_eq!(
        problems[0].1,
        Fault::UnsupportedValue {
            expected: "an integer from 1 to 256".to_owned()
        }
    );
}

#[test]
fn an_out_of_range_monitors_global_scale_is_refused_naming_the_leaf() {
    let document = table([("monitors", table([("global-scale", int(5))]))]);
    let problems = problems(document);
    assert_eq!(problems.len(), 1, "one leaf is wrong, got {problems:?}");
    assert!(
        problems[0].0.contains("global-scale"),
        "the refusal names the leaf, got {}",
        problems[0].0
    );
    assert!(matches!(problems[0].1, Fault::UnsupportedValue { .. }));
}

#[test]
fn a_removed_setting_names_what_replaced_it() {
    let document = table([("window-management", table([("center-new-windows", flag(true))]))]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "window-management.center-new-windows");
    match &problems[0].1 {
        Fault::Removed { detail } => assert!(detail.contains("new-window-placement")),
        other => panic!("expected a removed setting, got {other:?}"),
    }
}

#[test]
fn the_shell_section_is_refused_with_a_sentence_rather_than_a_typo_hint() {
    let document = table([("shell", table([("animations", flag(true))]))]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "shell");
    match &problems[0].1 {
        Fault::Removed { detail } => assert!(detail.contains("shell client")),
        other => panic!("expected an explanation, got {other:?}"),
    }
}

#[test]
fn a_deprecated_spelling_is_accepted_canonicalised_and_reported() {
    let document = table([("window-management", table([("focus-mode", text("sloppy"))]))]);
    let config = Config::from_value(document).expect("a deprecated spelling is accepted");
    assert_eq!(
        config.window_management.as_ref().expect("window-management").focus_mode,
        Some(FocusMode::Hover)
    );
    let deprecations = config.deprecations();
    assert_eq!(deprecations.len(), 1);
    assert_eq!(deprecations[0].path.to_string(), "window-management.focus-mode");
    assert_eq!(deprecations[0].spelling, "sloppy");
    assert_eq!(deprecations[0].canonical, "hover");
}

#[test]
fn a_deprecated_spelling_never_reaches_the_accepted_document() {
    let document = table([("input", table([("touchpad", table([("tap-button-map", text("lrm"))]))]))]);
    let config = Config::from_value(document).expect("valid");
    assert_eq!(
        config
            .input
            .as_ref()
            .and_then(|input| input.touchpad.as_ref())
            .and_then(|touchpad| touchpad.tap_button_map),
        Some(Inherit::Value(TapButtonMap::LeftRightMiddle))
    );
    let stored = config
        .document()
        .as_table()
        .and_then(|root| root.get("input"))
        .and_then(Value::as_table)
        .and_then(|input| input.get("touchpad"))
        .and_then(Value::as_table)
        .and_then(|touchpad| touchpad.get("tap-button-map"))
        .cloned();
    assert_eq!(
        stored,
        Some(text("left-right-middle")),
        "the document holds the current spelling"
    );
}

#[test]
fn a_deprecated_spelling_belongs_only_to_its_own_setting() {
    // "mouse" is a deprecated focus-mode and left-handed spelling, and it is not
    // a location accuracy.
    let document = table([("location", table([("max-accuracy", text("mouse"))]))]);
    let problems = problems(document);
    assert!(matches!(problems[0].1, Fault::UnsupportedValue { .. }));
}

#[test]
fn inherit_is_kept_apart_from_a_value() {
    let document = table([("input", table([("mouse", table([("speed", text("inherit"))]))]))]);
    let config = Config::from_value(document).expect("valid");
    let mouse = config
        .input
        .as_ref()
        .and_then(|input| input.mouse.as_ref())
        .expect("mouse");
    assert_eq!(mouse.speed, Some(Inherit::Inherit));
}

#[test]
fn a_declared_workspace_list_and_dynamic_workspaces_conflict() {
    let document = table([
        ("window-management", table([("dynamic-workspaces", flag(true))])),
        (
            "workspaces",
            list(vec![table([("id", text("main")), ("name", text("Main"))])]),
        ),
    ]);
    let problems = problems(document);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, "workspaces");
    assert_eq!(
        problems[0].1,
        Fault::Conflict {
            other: Path::root().key("window-management").key("dynamic-workspaces")
        }
    );
}

#[test]
fn a_gesture_needs_exactly_one_of_action_and_command() {
    let gesture = table([
        ("name", text("both")),
        ("gesture", text("swipe")),
        ("fingers", int(3)),
        (
            "path",
            list(vec![
                table([("x", number(0.0)), ("y", number(0.0))]),
                table([("x", number(1.0)), ("y", number(0.0))]),
            ]),
        ),
        ("action", text("workspace.next")),
        ("command", list(vec![text("kgx")])),
    ]);
    let problems = problems(table([("touchpad-gestures", list(vec![gesture]))]));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, "touchpad-gestures[1]");
}

#[test]
fn two_gestures_that_claim_the_same_movement_conflict() {
    let gesture = |name: &str| {
        table([
            ("name", text(name)),
            ("gesture", text("swipe")),
            ("fingers", int(3)),
            (
                "path",
                list(vec![
                    table([("x", number(0.0)), ("y", number(0.0))]),
                    table([("x", number(1.0)), ("y", number(0.0))]),
                ]),
            ),
            ("action", text("workspace.next")),
        ])
    };
    let problems = problems(table([(
        "touchpad-gestures",
        list(vec![gesture("first"), gesture("second")]),
    )]));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, "touchpad-gestures[2]");
    assert!(matches!(problems[0].1, Fault::Conflict { .. }));
}

#[test]
fn a_swipe_path_must_start_at_the_origin() {
    let gesture = table([
        ("name", text("away")),
        ("gesture", text("swipe")),
        ("fingers", int(3)),
        (
            "path",
            list(vec![
                table([("x", number(0.5)), ("y", number(0.0))]),
                table([("x", number(1.0)), ("y", number(0.0))]),
            ]),
        ),
        ("action", text("workspace.next")),
    ]);
    let problems = problems(table([("touchpad-gestures", list(vec![gesture]))]));
    assert_eq!(problems[0].0, "touchpad-gestures[1].path");
}

#[test]
fn an_unknown_protocol_is_refused_with_the_protocol_list() {
    let document = table([("protocols", table([("wlr_screencopy", flag(false))]))]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "protocols.wlr_screencopy");
    let accepted = accepted_keys(&problems[0]).expect("the protocol list is the hint");
    assert!(
        accepted.contains(&"wlr-screencopy"),
        "the hint spells the name in kebab-case"
    );
}

#[test]
fn a_workspace_id_must_be_unique() {
    let document = table([(
        "workspaces",
        list(vec![
            table([("id", text("main")), ("name", text("Main"))]),
            table([("id", text("main")), ("name", text("Other"))]),
        ]),
    )]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "workspaces[2].id");
}

#[test]
fn a_workspace_entry_must_have_a_name() {
    let document = table([("workspaces", list(vec![table([("id", text("main"))])]))]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "workspaces[1].name");
    assert_eq!(
        problems[0].1,
        Fault::Missing {
            expected: "a workspace name"
        }
    );
}

#[test]
fn an_invalid_device_id_is_refused_at_its_path() {
    let document = table([(
        "input",
        table([("tablets", table([("123:", table([("mapping", text("absolute"))]))]))]),
    )]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "input.tablets.123:");
    assert!(matches!(problems[0].1, Fault::UnsupportedValue { .. }));
}

#[test]
fn a_non_finite_number_is_reported_at_its_path() {
    let document = table([("input", table([("mouse", table([("speed", number(f64::NAN))]))]))]);
    let problems = problems(document);
    assert_eq!(problems[0].0, "input.mouse.speed");
    assert_eq!(problems[0].1, Fault::NotFinite);
}

#[test]
fn a_tree_nested_past_the_limit_is_refused_before_any_section() {
    let mut nested = Value::Integer(1);
    for _ in 0..=MAX_DEPTH {
        nested = Value::table([("k", nested)]);
    }
    let problems = problems(nested);
    assert_eq!(
        problems.len(),
        1,
        "a tree that cannot be walked is refused before its keys are read"
    );
    assert!(matches!(problems[0].1, Fault::TooDeep { maximum } if maximum == MAX_DEPTH));
}

#[test]
fn a_root_that_is_not_a_table_names_the_document() {
    let problems = problems(list(vec![]));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, "<document>");
}

#[test]
fn a_layout_section_loads_with_defaults_and_overrides() {
    let document = table([(
        "layout",
        table([
            ("default", text("master_stack")),
            (
                "outputs",
                list(vec![table([("output", text("DP-1")), ("layout", text("columns"))])]),
            ),
            (
                "workspaces",
                list(vec![table([("workspace", text("main")), ("layout", text("floating"))])]),
            ),
        ]),
    )]);
    let config = Config::from_value(document).expect("a layout section validates");
    let layout = config.layout.as_ref().expect("layout");
    assert_eq!(layout.default.as_deref(), Some("master_stack"));
    let outputs = layout.outputs.as_ref().expect("outputs");
    assert_eq!(outputs[0].output.as_deref(), Some("DP-1"));
    assert_eq!(outputs[0].layout.as_deref(), Some("columns"));
    let workspaces = layout.workspaces.as_ref().expect("workspaces");
    assert_eq!(workspaces[0].workspace.as_deref(), Some("main"));
}

#[test]
fn an_empty_layout_section_sets_nothing() {
    let config = Config::from_value(table([("layout", table([]))])).expect("valid");
    let layout = config.layout.as_ref().expect("layout");
    assert!(
        layout.default.is_none(),
        "the floating default is applied by the layout host, not here"
    );
    assert!(layout.outputs.is_none() && layout.workspaces.is_none());
}

#[test]
fn an_unknown_layout_key_is_refused() {
    let found = problems(table([("layout", table([("defualt", text("tile"))]))]));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "layout.defualt");
    assert!(matches!(found[0].1, Fault::UnknownKey { .. }));
}

#[test]
fn a_layout_of_the_wrong_type_is_refused() {
    let found = problems(table([("layout", table([("default", int(3))]))]));
    assert_eq!(found[0].0, "layout.default");
}
