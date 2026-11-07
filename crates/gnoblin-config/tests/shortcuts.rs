//! The `shortcuts` section: a map of named chord-to-command bindings.
//!
//! The section is validated here so the compositor only ever receives shortcuts
//! that have a name, a chord and a runnable command. Duplicate chords are not
//! checked here: they depend on the keymap and are refused where the bindings
//! are installed.

mod support;

use gnoblin_config::{Config, ShortcutTrigger, Value};

use support::{flag, list, table, text};

fn shortcut(binding: &str, command: &[&str]) -> Value {
    table([
        ("binding", text(binding)),
        ("command", list(command.iter().map(|word| text(word)).collect())),
    ])
}

fn with_field(mut value: Value, key: &str, field: Value) -> Value {
    if let Value::Map(map) = &mut value {
        map.insert(key.to_owned(), field);
    }
    value
}

fn document(name: &str, shortcut: Value) -> Value {
    table([("shortcuts", table([(name, shortcut)]))])
}

fn refused(document: Value) -> Vec<String> {
    let report = Config::from_value(document).expect_err("the document must be refused");
    report
        .problems()
        .iter()
        .map(|problem| problem.path.to_string())
        .collect()
}

#[test]
fn a_named_shortcut_parses_into_its_typed_fields() {
    let config = Config::from_value(document("terminal", shortcut("<Super>Return", &["foot"]))).expect("valid");
    let shortcuts = config.shortcuts.as_ref().expect("the shortcuts section is present");
    let terminal = shortcuts
        .shortcuts
        .get("terminal")
        .expect("the shortcut is keyed by its name");
    assert_eq!(terminal.binding.as_deref(), Some("<Super>Return"));
    assert_eq!(terminal.command.as_deref(), Some(&["foot".to_owned()][..]));
    assert_eq!(
        terminal.trigger, None,
        "an absent trigger is left to the consumer, which applies press"
    );
    assert_eq!(terminal.enable, None);
}

#[test]
fn trigger_and_enable_are_read_when_present() {
    let value = with_field(shortcut("<Super>k", &["true"]), "trigger", text("release"));
    let value = with_field(value, "enable", flag(false));
    let config = Config::from_value(document("unlock", value)).expect("valid");
    let unlock = &config.shortcuts.as_ref().expect("section").shortcuts["unlock"];
    assert_eq!(unlock.trigger, Some(ShortcutTrigger::Release));
    assert_eq!(unlock.enable, Some(false));
}

#[test]
fn a_shortcut_name_outside_the_allowed_shape_is_refused() {
    let problems = refused(document("has space", shortcut("<Super>Return", &["foot"])));
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with("shortcuts"),
        "the path names the section: {problems:?}"
    );
}

#[test]
fn an_empty_command_is_refused() {
    let problems = refused(document("terminal", shortcut("<Super>Return", &[])));
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].contains("command"),
        "the path names the command: {problems:?}"
    );
}

#[test]
fn an_unknown_trigger_is_refused() {
    let value = with_field(shortcut("<Super>Return", &["foot"]), "trigger", text("hold"));
    let problems = refused(document("terminal", value));
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].contains("trigger"),
        "the path names the trigger: {problems:?}"
    );
}
