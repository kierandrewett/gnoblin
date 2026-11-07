//! What a `window-rules` list must be accepted for, and what a refusal must name.
//!
//! A rule the compositor cannot honour is a load error, not a silent no-op: a
//! refusal that names the field lets the user fix the document rather than find
//! a window somewhere they did not intend.

mod support;

use gnoblin_config::{Config, Fault, Value, WindowRuleType};

use support::{flag, int, list, table, text};

fn problems(document: Value) -> Vec<(String, Fault)> {
    let report = Config::from_value(document).expect_err("the document must be refused");
    report
        .problems()
        .iter()
        .map(|problem| (problem.path.to_string(), problem.fault.clone()))
        .collect()
}

fn document(rules: Vec<Value>) -> Value {
    table([("window-rules", list(rules))])
}

#[test]
fn a_rule_selects_and_places_a_window() {
    let config = Config::from_value(document(vec![table([
        (
            "match",
            table([
                ("type", text("window")),
                ("app-id", text("^firefox$")),
                ("focused", flag(false)),
                ("client-type", text("wayland")),
            ]),
        ),
        ("workspace", table([("number", int(2))])),
        ("maximise", flag(true)),
    ])]))
    .expect("a rule with a pattern, a client type and a workspace validates");
    let rules = &config.window_rules;
    assert_eq!(rules.len(), 1);
    let matcher = rules[0].r#match.as_ref().expect("the match table");
    assert_eq!(matcher.r#type, Some(WindowRuleType::Window));
    assert_eq!(matcher.app_id.as_deref(), Some("^firefox$"));
    assert_eq!(matcher.focused, Some(false));
    assert!(matcher.client_type.is_some(), "the client type is read");
    assert_eq!(
        rules[0].workspace.as_ref().and_then(|workspace| workspace.number),
        Some(2)
    );
    assert_eq!(rules[0].maximise, Some(true));
}

#[test]
fn a_rule_marks_a_window_secure() {
    let config = Config::from_value(document(vec![table([
        ("match", table([("app-id", text("^keepassxc$"))])),
        ("secure", flag(true)),
    ])]))
    .expect("a rule with a secure flag validates");
    assert_eq!(config.window_rules[0].secure, Some(true));
}

#[test]
fn a_secure_flag_must_be_a_boolean() {
    let problems = problems(document(vec![table([("secure", text("yes"))])]));
    assert!(
        problems.iter().any(|(path, _)| path.contains("secure")),
        "the refusal names the secure field: {problems:?}"
    );
}

#[test]
fn an_empty_rule_selects_every_window() {
    let config = Config::from_value(document(vec![table([])])).expect("an empty rule validates");
    assert_eq!(config.window_rules.len(), 1);
    assert!(
        config.window_rules[0].r#match.is_none(),
        "an omitted match is not invented"
    );
}

#[test]
fn a_field_that_is_not_built_yet_is_refused_by_name() {
    for key in ["opacity", "corners", "shader", "shader-uniforms", "animation", "frame"] {
        let problems = problems(document(vec![table([(key, flag(true))])]));
        assert!(
            problems
                .iter()
                .any(|(path, fault)| path.contains(key) && matches!(fault, Fault::Refused { .. })),
            "{key} must be refused by name: {problems:?}"
        );
    }
}

#[test]
fn a_refused_field_keeps_its_legacy_underscored_spelling_in_the_refusal() {
    let problems = problems(document(vec![table([("shader_uniforms", flag(true))])]));
    assert!(
        problems
            .iter()
            .any(|(path, fault)| path.contains("shader_uniforms") && matches!(fault, Fault::Refused { .. })),
        "the underscored spelling is caught too: {problems:?}"
    );
}

#[test]
fn layer_surfaces_are_refused_until_the_selector_exists() {
    let by_type = problems(document(vec![table([("match", table([("type", text("layer"))]))])]));
    assert!(
        by_type
            .iter()
            .any(|(path, fault)| path.ends_with("type") && matches!(fault, Fault::Refused { .. })),
        "type = layer is refused: {by_type:?}"
    );
    let by_namespace = problems(document(vec![table([("match", table([("layer", text("^bar"))]))])]));
    assert!(
        by_namespace
            .iter()
            .any(|(path, fault)| path.ends_with("layer") && matches!(fault, Fault::Refused { .. })),
        "a layer namespace pattern is refused: {by_namespace:?}"
    );
}

#[test]
fn a_malformed_pattern_is_refused_at_its_key() {
    for key in ["app-id", "title"] {
        let problems = problems(document(vec![table([("match", table([(key, text("[abc"))]))])]));
        assert!(
            problems
                .iter()
                .any(|(path, fault)| path.ends_with(key) && matches!(fault, Fault::UnsupportedValue { .. })),
            "a broken {key} pattern is refused: {problems:?}"
        );
    }
}

#[test]
fn a_rule_places_on_exactly_one_workspace() {
    let neither = table([]);
    let both = table([("id", text("main")), ("number", int(1))]);
    for workspace in [neither, both] {
        let problems = problems(document(vec![table([("workspace", workspace)])]));
        assert!(
            problems
                .iter()
                .any(|(path, fault)| path.ends_with("workspace") && matches!(fault, Fault::UnsupportedValue { .. })),
            "a rule names exactly one of id or number: {problems:?}"
        );
    }
}

#[test]
fn a_workspace_number_is_one_based() {
    let problems = problems(document(vec![table([("workspace", table([("number", int(0))]))])]));
    assert!(!problems.is_empty(), "workspace number 0 is outside 1 to 1024");
}

#[test]
fn the_rule_list_is_capped_at_256() {
    let at_cap = (0..256).map(|_| table([])).collect();
    assert!(
        Config::from_value(document(at_cap)).is_ok(),
        "256 rules is the cap, not over it"
    );
    let over = (0..257).map(|_| table([])).collect();
    let problems = problems(document(over));
    assert!(
        problems
            .iter()
            .any(|(path, fault)| path.contains("window-rules") && matches!(fault, Fault::UnsupportedValue { .. })),
        "257 rules is refused: {problems:?}"
    );
}
