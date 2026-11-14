//! The `dialogs.style` key: what it accepts, and what it refuses with which path.
//!
//! The ranges the schema accepts are wider than the ranges a dialog draws with (a dialog clamps what it is given,
//! `gnoblin_portal_ui::style`), so these tests cover the document layer only: types, spellings and paths.

mod support;

use gnoblin_config::{Config, DialogButtonLayout, Fault, Value};

use support::{int, number, table, text};

fn style(entries: [(&str, Value); 1]) -> Value {
    table([("dialogs", table([("style", table(entries))]))])
}

fn problems(document: Value) -> Vec<(String, Fault)> {
    let report = Config::from_value(document).expect_err("the document must be refused");
    report
        .problems()
        .iter()
        .map(|problem| (problem.path.to_string(), problem.fault.clone()))
        .collect()
}

#[test]
fn a_complete_style_is_accepted_with_typed_values() {
    let document = table([(
        "dialogs",
        table([(
            "style",
            table([
                ("radius", int(14)),
                ("accent", text("#89b4fa")),
                ("font", table([("family", text("Inter")), ("size", int(16))])),
                ("button-layout", text("stretch")),
                ("scale", number(1.5)),
            ]),
        )]),
    )]);
    let config = Config::from_value(document).expect("a style document validates");
    let style = config
        .dialogs
        .as_ref()
        .and_then(|dialogs| dialogs.style.as_ref())
        .expect("the style");
    assert_eq!(style.radius, Some(14));
    assert_eq!(style.accent.as_deref(), Some("#89b4fa"));
    let font = style.font.as_ref().expect("the font");
    assert_eq!(font.family.as_deref(), Some("Inter"));
    assert_eq!(font.size, Some(16));
    assert_eq!(style.button_layout, Some(DialogButtonLayout::Stretch));
    assert_eq!(style.scale, Some(1.5));
}

#[test]
fn no_dialogs_section_is_no_style() {
    let config = Config::from_value(table([("cursor", table([("size", int(24))]))])).expect("valid");
    assert!(config.dialogs.is_none());
}

#[test]
fn a_named_accent_is_accepted_like_the_appearance_accent() {
    for accent in ["blue", "slate", "#fff", "#00ff7f"] {
        Config::from_value(style([("accent", text(accent))])).unwrap_or_else(|_| panic!("{accent} is an accent"));
    }
}

#[test]
fn a_value_that_is_not_a_colour_is_refused_at_its_path() {
    for accent in ["", "chartreuse", "#12", "#12345678", "89b4fa"] {
        let found = problems(style([("accent", text(accent))]));
        assert!(
            found.iter().any(|(path, _)| path == "dialogs.style.accent"),
            "{accent:?} is not an accent: {found:?}"
        );
    }
}

#[test]
fn an_unknown_key_names_the_keys_a_style_has() {
    let found = problems(style([("colour", text("blue"))]));
    let (path, fault) = &found[0];
    assert_eq!(path, "dialogs.style.colour");
    let Fault::UnknownKey { accepted } = fault else {
        panic!("expected an unknown key, got {fault:?}");
    };
    for key in ["radius", "accent", "font", "button-layout", "scale"] {
        assert!(accepted.contains(&key), "{key} is accepted: {accepted:?}");
    }
}

#[test]
fn a_wrong_type_or_spelling_is_refused_at_its_path() {
    let found = problems(style([("radius", text("round"))]));
    assert_eq!(found[0].0, "dialogs.style.radius");
    let found = problems(style([("scale", text("big"))]));
    assert_eq!(found[0].0, "dialogs.style.scale");
    let found = problems(style([("button-layout", text("sideways"))]));
    assert_eq!(found[0].0, "dialogs.style.button-layout");
    assert!(matches!(found[0].1, Fault::UnsupportedValue { .. }), "{:?}", found[0].1);
}

#[test]
fn numbers_past_the_sanity_bounds_are_refused_and_numbers_past_the_safe_range_are_not() {
    // The document layer refuses nonsense; the dialog crate clamps the rest.
    for (key, value) in [("radius", int(5_000)), ("radius", int(-1)), ("scale", number(0.0))] {
        let found = problems(style([(key, value)]));
        assert_eq!(found[0].0, format!("dialogs.style.{key}"));
    }
    Config::from_value(style([("radius", int(100))]))
        .expect("an oversized radius is clamped by the dialog, not refused");
    Config::from_value(style([("scale", number(0.5))])).expect("a small scale is clamped by the dialog, not refused");
}

#[test]
fn a_font_family_is_printable_text_of_bounded_length() {
    let family = |family: &str| style([("font", table([("family", text(family))]))]);
    Config::from_value(family("monospace")).expect("a family name");
    for bad in ["bad\nname", &"x".repeat(65)] {
        let found = problems(family(bad));
        assert!(
            found.iter().any(|(path, _)| path == "dialogs.style.font.family"),
            "{bad:?} is not a family name: {found:?}"
        );
    }
    let found = problems(style([("font", table([("size", int(0))]))]));
    assert_eq!(found[0].0, "dialogs.style.font.size");
}
