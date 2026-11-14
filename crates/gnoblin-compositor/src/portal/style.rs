//! The look the built-in dialogs are drawn with, from a committed configuration.
//!
//! `dialogs.style` is plain data in the document (`gnoblin-config`); what a dialog may do with it is decided here and
//! in `gnoblin_portal_ui::style`: every number is clamped to what a dialog can be drawn with and still be trusted,
//! and the accent is corrected against the dialog's colour when the dialog is drawn. This module only translates the
//! document's section into that crate's request, so the clamping rules live in exactly one place.

use gnoblin_config::appearance::Accent;
use gnoblin_config::{Config, DialogButtonLayout};
use gnoblin_portal_ui::style::{ButtonLayout, DialogStyle, StyleRequest};

use crate::log;

/// What `dialogs.style` asks for, before it is clamped. `None` when the document names no style.
fn request_of(config: &Config) -> Option<StyleRequest> {
    let style = config.dialogs.as_ref().and_then(|dialogs| dialogs.style.as_ref())?;
    // The document layer already refused a value that is not an accent, so a parse that fails here is a stale
    // document and draws no accent rather than a wrong one.
    let accent = style
        .accent
        .as_deref()
        .and_then(|text| Accent::parse(text).ok())
        .map(|accent| {
            let rgb = accent.rgb();
            [rgb.red, rgb.green, rgb.blue]
        });
    let font = style.font.as_ref();
    Some(StyleRequest {
        radius: style.radius,
        accent,
        font_family: font.and_then(|font| font.family.clone()),
        font_size: font.and_then(|font| font.size),
        scale: style.scale,
        button_layout: style.button_layout.map(layout_of),
    })
}

/// The style a configuration asks for, clamped. A document that names no `dialogs.style` draws the stock look.
pub(crate) fn dialog_style(config: &Config) -> DialogStyle {
    request_of(config).map_or_else(DialogStyle::default, |request| request.clamp())
}

/// Logs each value of `dialogs.style` that was moved to stay inside its range, once per commit, so a user who asked
/// for 8-point text can find out why the dialog uses 12. Both dialog services read the same style, so the commit path
/// reports once and not once per service.
pub(crate) fn report_dialog_style(config: &Config) {
    for moved in request_of(config)
        .map(|request| request.adjustments())
        .unwrap_or_default()
    {
        log::info!(area = "portal", "dialogs.style: {moved}");
    }
}

const fn layout_of(layout: DialogButtonLayout) -> ButtonLayout {
    match layout {
        DialogButtonLayout::Start => ButtonLayout::Start,
        DialogButtonLayout::End => ButtonLayout::End,
        DialogButtonLayout::Stretch => ButtonLayout::Stretch,
        DialogButtonLayout::Stacked => ButtonLayout::Stacked,
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_config::Value;
    use gnoblin_portal_ui::style::{FONT_SIZE_MAX, FONT_SIZE_MIN, RADIUS_MAX, SCALE_MAX, SCALE_MIN};

    use super::*;

    fn config_of(style: Value) -> Config {
        let document = Value::table([("dialogs", Value::table([("style", style)]))]);
        Config::from_value(document).expect("a valid document")
    }

    #[test]
    fn no_dialogs_section_is_the_stock_style() {
        let config = Config::from_value(Value::table([("cursor", Value::table([("size", Value::Integer(24))]))]))
            .expect("valid");
        assert_eq!(dialog_style(&config), DialogStyle::default());
    }

    #[test]
    fn a_style_in_the_document_reaches_the_dialog_style() {
        let config = config_of(Value::table([
            ("radius", Value::Integer(10)),
            ("accent", Value::String("#336699".to_owned())),
            (
                "font",
                Value::table([
                    ("family", Value::String("monospace".to_owned())),
                    ("size", Value::Integer(18)),
                ]),
            ),
            ("button-layout", Value::String("stacked".to_owned())),
            ("scale", Value::Float(1.5)),
        ]));
        let style = dialog_style(&config);
        assert_eq!(style.radius(), Some(10));
        assert_eq!(style.accent(), Some([0x33, 0x66, 0x99]));
        assert_eq!(style.font_size(), Some(18));
        assert!(style.monospace());
        assert_eq!(style.button_layout(), ButtonLayout::Stacked);
        assert!((style.scale() - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn a_named_accent_is_the_colour_it_names() {
        let config = config_of(Value::table([("accent", Value::String("blue".to_owned()))]));
        let rgb = Accent::parse("blue").expect("a named accent").rgb();
        assert_eq!(dialog_style(&config).accent(), Some([rgb.red, rgb.green, rgb.blue]));
    }

    #[test]
    fn a_file_cannot_ask_for_more_than_a_dialog_may_be() {
        // The document layer accepts these (they are numbers); the dialog never draws past its limits.
        let config = config_of(Value::table([
            ("radius", Value::Integer(1000)),
            ("font", Value::table([("size", Value::Integer(1))])),
            ("scale", Value::Float(100.0)),
        ]));
        let style = dialog_style(&config);
        assert_eq!(style.radius(), Some(RADIUS_MAX));
        assert_eq!(style.font_size(), Some(FONT_SIZE_MIN));
        assert!((f64::from(style.scale()) - SCALE_MAX).abs() < 1e-6);

        let config = config_of(Value::table([
            ("radius", Value::Integer(0)),
            ("font", Value::table([("size", Value::Integer(1000))])),
            ("scale", Value::Float(0.1)),
        ]));
        let style = dialog_style(&config);
        assert_eq!(style.radius(), Some(0));
        assert_eq!(style.font_size(), Some(FONT_SIZE_MAX));
        assert!((f64::from(style.scale()) - SCALE_MIN).abs() < 1e-6);
    }
}
