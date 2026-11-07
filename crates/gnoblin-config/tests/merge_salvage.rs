//! Layering and salvage: what several files produce, and what survives a broken
//! section.
//!
//! The two are tested together because they answer one question from both sides:
//! which values does the session end up with, and what did it have to give up to
//! get them.

mod support;

use gnoblin_config::{Config, Fault, FocusMode, Value, merge_layers, salvage};

use support::{flag, int, list, number, table, text};

#[test]
fn the_last_layer_wins_and_tables_merge_by_key() {
    let defaults = table([
        ("compositor", table([("blur", text("auto")), ("debug", flag(false))])),
        ("cursor", table([("size", int(24))])),
    ]);
    let user = table([("compositor", table([("blur", text("off"))]))]);
    let merged = merge_layers(vec![defaults, user]);
    let config = Config::from_value(merged).expect("valid");
    let compositor = config.compositor.as_ref().expect("compositor");
    assert_eq!(compositor.blur, Some(gnoblin_config::Blur::Off), "the last layer wins");
    assert_eq!(compositor.debug, Some(false), "an earlier key survives the merge");
    assert_eq!(config.cursor.as_ref().expect("cursor").size, Some(24));
}

#[test]
fn reversing_the_layers_changes_which_value_wins() {
    let lower = table([("window-management", table([("focus-mode", text("click"))]))]);
    let higher = table([("window-management", table([("focus-mode", text("hover"))]))]);
    let forwards = Config::from_value(merge_layers(vec![lower.clone(), higher.clone()])).expect("valid");
    let backwards = Config::from_value(merge_layers(vec![higher, lower])).expect("valid");
    assert_eq!(
        forwards.window_management.as_ref().expect("management").focus_mode,
        Some(FocusMode::Hover)
    );
    assert_eq!(
        backwards.window_management.as_ref().expect("management").focus_mode,
        Some(FocusMode::Click),
        "precedence follows load order, so swapping the layers swaps the winner"
    );
}

#[test]
fn a_later_list_replaces_an_earlier_one_instead_of_appending() {
    let defaults = table([(
        "workspaces",
        list(vec![
            table([("id", text("one")), ("name", text("One"))]),
            table([("id", text("two")), ("name", text("Two"))]),
        ]),
    )]);
    let user = table([(
        "workspaces",
        list(vec![table([("id", text("three")), ("name", text("Three"))])]),
    )]);
    let config = Config::from_value(merge_layers(vec![defaults, user])).expect("valid");
    let ids: Vec<&str> = config
        .workspaces
        .iter()
        .map(|workspace| workspace.id.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(
        ids,
        vec!["three"],
        "a list is replaced whole, so order and length come from one file"
    );
}

#[test]
fn an_inherit_marker_overrides_an_earlier_value_instead_of_disappearing() {
    let defaults = table([("input", table([("mouse", table([("speed", number(0.5))]))]))]);
    let user = table([("input", table([("mouse", table([("speed", text("inherit"))]))]))]);
    let config = Config::from_value(merge_layers(vec![defaults, user])).expect("valid");
    let mouse = config
        .input
        .as_ref()
        .and_then(|input| input.mouse.as_ref())
        .expect("mouse");
    assert_eq!(
        mouse.speed,
        Some(gnoblin_config::Inherit::Inherit),
        "inherit must not fall back to the value it was meant to clear"
    );
}

#[test]
fn salvage_keeps_the_sections_that_validate() {
    let document = table([
        ("compositor", table([("blur2", text("on"))])),
        ("cursor", table([("size", int(32))])),
        ("window-management", table([("edge-tiling", flag(true))])),
    ]);
    let salvaged = salvage(document);
    assert!(salvaged.config.compositor.is_none(), "the invalid section is dropped");
    assert_eq!(salvaged.config.cursor.as_ref().expect("cursor").size, Some(32));
    assert_eq!(
        salvaged
            .config
            .window_management
            .as_ref()
            .expect("management")
            .edge_tiling,
        Some(true)
    );
    let report = salvaged.report.expect("dropping a section is reported");
    assert_eq!(report.problems().len(), 1);
    assert_eq!(report.problems()[0].path.to_string(), "compositor.blur2");
    assert!(matches!(report.problems()[0].fault, Fault::UnknownKey { .. }));
}

#[test]
fn salvage_reports_every_section_it_drops() {
    let document = table([
        ("compositor", table([("blur", text("maybe"))])),
        ("cursor", table([("size", int(0))])),
        ("keyring", table([("enabled", flag(true))])),
    ]);
    let salvaged = salvage(document);
    let report = salvaged.report.expect("two sections were dropped");
    let paths: Vec<String> = report
        .problems()
        .iter()
        .map(|problem| problem.path.to_string())
        .collect();
    assert_eq!(paths, vec!["compositor.blur", "cursor.size"]);
    assert_eq!(salvaged.config.keyring.as_ref().expect("keyring").enabled, Some(true));
}

#[test]
fn a_document_that_validates_salvages_to_itself_without_a_report() {
    let document = table([("cursor", table([("size", int(24))]))]);
    let salvaged = salvage(document.clone());
    assert!(
        salvaged.report.is_none(),
        "nothing was dropped, so there is nothing to report"
    );
    assert_eq!(salvaged.config.document(), &document);
}

#[test]
fn salvage_of_a_document_that_is_not_a_table_keeps_nothing() {
    let salvaged = salvage(Value::List(vec![Value::Integer(1)]));
    assert!(salvaged.config.is_empty());
    let report = salvaged.report.expect("the refusal is reported");
    assert_eq!(report.problems()[0].path.to_string(), "<document>");
}

#[test]
fn salvage_drops_the_workspace_declaration_when_it_conflicts_with_dynamic_workspaces() {
    let document = table([
        ("window-management", table([("dynamic-workspaces", flag(true))])),
        (
            "workspaces",
            list(vec![table([("id", text("main")), ("name", text("Main"))])]),
        ),
    ]);
    let salvaged = salvage(document);
    assert!(
        salvaged.config.workspaces.is_empty(),
        "the declaration is the newer setting, so it is the one dropped"
    );
    assert_eq!(
        salvaged
            .config
            .window_management
            .as_ref()
            .expect("management")
            .dynamic_workspaces,
        Some(true)
    );
    let report = salvaged.report.expect("the conflict is reported");
    assert_eq!(report.problems()[0].path.to_string(), "workspaces");
}
