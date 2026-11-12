//! The apply rules against the schema and against `design/config-reload.md`.
//!
//! A setting added to the schema without a rule would be accepted by a reload and silently not
//! applied, which is the failure the rules exist to name. These tests fail the build until the
//! new setting is classified, and until the design table says the same as the code.

use std::collections::BTreeSet;

use gnoblin_config::apply::{Applies, RULES, rule_of};
use gnoblin_config::{CARRIED_SECTIONS, TOP_LEVEL_KEYS, generated::SETTINGS};

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
    let ruled: BTreeSet<String> = RULES
        .iter()
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
    assert_eq!(ruled.len(), RULES.len(), "a setting has two rules");
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
