//! The apply rules against the schema and against `design/config-reload.md`.
//!
//! A setting added to the schema without a rule would be accepted by a reload and silently not
//! applied, which is the failure the rules exist to name. These tests fail the build until the
//! new setting is classified, and until the design table says the same as the code.

use std::collections::BTreeSet;

use gnoblin_config::apply::{Applies, RULES, rule_of};
use gnoblin_config::{CARRIED_SECTIONS, TOP_LEVEL_KEYS, generated::SETTINGS};
use gnoblin_config::{Config, Fault, Value};

const DESIGN: &str = include_str!("../../../design/config-reload.md");

/// Every setting the schema declares, as `section.key`, with the carried sections as `section.*`.
fn declared() -> BTreeSet<String> {
    let mut names: BTreeSet<String> = SETTINGS
        .iter()
        .map(|setting| format!("{}.{}", setting.section, setting.key))
        .collect();
    names.extend(CARRIED_SECTIONS.iter().map(|section| format!("{section}.*")));
    names
}

#[test]
fn every_declared_setting_has_a_rule_and_every_rule_names_a_declared_setting() {
    // A rule for one leaf of a record (`mouse.double-click-time`) is checked by the test below.
    let ruled: BTreeSet<String> = RULES
        .iter()
        .filter(|rule| !rule.key.contains('.'))
        .map(|rule| format!("{}.{}", rule.section, rule.key))
        .collect();
    let declared = declared();
    let missing: Vec<&String> = declared.difference(&ruled).collect();
    let unknown: Vec<&String> = ruled.difference(&declared).collect();
    assert!(missing.is_empty(), "settings with no apply rule: {missing:?}");
    assert!(
        unknown.is_empty(),
        "rules for settings the schema does not declare: {unknown:?}"
    );
    let all: BTreeSet<String> = RULES
        .iter()
        .map(|rule| format!("{}.{}", rule.section, rule.key))
        .collect();
    assert_eq!(all.len(), RULES.len(), "a setting has two rules");
}

/// A document that sets the leaf a rule names to a string, nested as the rule's path nests.
fn leaf_document(rule: &gnoblin_config::apply::ApplyRule) -> Value {
    let mut path = vec![rule.section];
    path.extend(rule.key.split('.'));
    let mut value = Value::string("leaf");
    for key in path.iter().rev() {
        value = Value::table([(*key, value)]);
    }
    value
}

#[test]
fn a_rule_for_one_leaf_names_a_leaf_the_schema_has_inside_a_section_with_a_rule() {
    let mut leaves = 0;
    for rule in RULES.iter().filter(|rule| rule.key.contains('.')) {
        leaves += 1;
        let named = format!("{}.{}", rule.section, rule.key);
        let (record, _) = rule.key.split_once('.').expect("a dotted key");
        assert!(
            rule_of(rule.section, record).is_some(),
            "{named} is a leaf of a setting with no rule of its own"
        );
        // A string is the wrong value for these leaves, but the schema must refuse it for the value and not
        // for the key: an unknown key means the rule names something the document does not have.
        if let Err(report) = Config::from_value(leaf_document(rule)) {
            let unknown = report.problems().iter().any(|problem| {
                matches!(problem.fault, Fault::UnknownKey { .. }) && problem.path.to_string().starts_with(&named)
            });
            assert!(!unknown, "{named} is not a setting the schema declares");
        }
    }
    assert!(leaves > 0, "the input section's leaves that nothing applies are listed");
}

#[test]
fn every_top_level_section_is_covered() {
    let covered: BTreeSet<&str> = RULES.iter().map(|rule| rule.section).collect();
    for key in TOP_LEVEL_KEYS {
        assert!(covered.contains(key), "the section {key} has no apply rule");
    }
}

#[test]
fn a_rule_that_waits_or_does_nothing_says_why() {
    for rule in RULES {
        let named = format!("{}.{}", rule.section, rule.key);
        match rule.applies {
            Applies::Live => assert!(rule.detail.is_empty(), "{named} is live and needs no reason"),
            Applies::Restart(_) | Applies::Unused => {
                assert!(!rule.detail.is_empty(), "{named} must say why it is not applied");
            }
        }
        assert!(rule_of(rule.section, rule.key).is_some());
    }
}

/// The table row of `setting` in the design document, if there is one. Prettier pads the
/// cells, so the setting is the trimmed first cell.
fn design_row(setting: &str) -> Option<&'static str> {
    let cell = format!("`{setting}`");
    DESIGN
        .lines()
        .find(|line| line.starts_with('|') && line.split('|').nth(1).map(str::trim) == Some(cell.as_str()))
}

#[test]
fn the_design_table_says_what_the_rules_say() {
    for rule in RULES {
        let setting = format!("{}.{}", rule.section, rule.key);
        let row = design_row(&setting).unwrap_or_else(|| panic!("design/config-reload.md has no row for {setting}"));
        let status = row.split('|').nth(2).map(str::trim).unwrap_or_default();
        let expected = match rule.applies {
            Applies::Live => "live",
            Applies::Restart(_) => "restart",
            Applies::Unused => "unused",
        };
        assert_eq!(status, expected, "design/config-reload.md disagrees about {setting}");
    }
}
