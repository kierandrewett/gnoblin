//! Design tokens for the portal dialogs: colours, radii, spacing and the type
//! scale, named once so every screen draws from the same values.
//!
//! The values are the ones the dialogs have always drawn with; this crate moves
//! them, it does not restyle anything. It depends on egui only and knows nothing
//! about portal interfaces.

use egui::{Color32, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// Which palette a dialog is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Dark,
}

/// Every colour a dialog paints with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub background: Color32,
    pub card: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub button: Color32,
    pub button_disarmed: Color32,
}

impl Palette {
    /// The palette for `mode`.
    #[must_use]
    pub const fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Dark => DARK,
        }
    }
}

/// The dark palette.
pub const DARK: Palette = Palette {
    background: Color32::from_rgb(20, 22, 27),
    card: Color32::from_rgb(34, 37, 44),
    text: Color32::from_rgb(235, 236, 240),
    muted: Color32::from_rgb(150, 155, 166),
    accent: Color32::from_rgb(78, 140, 230),
    button: Color32::from_rgb(58, 62, 72),
    button_disarmed: Color32::from_rgb(46, 49, 58),
};

/// Corner radii, in egui's integer pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Radii {
    /// Dialog card.
    pub card: u8,
    /// Buttons.
    pub button: u8,
    /// List rows and focus outlines on rows.
    pub row: u8,
}

/// The corner radii the dialogs use.
pub const RADII: Radii = Radii {
    card: 12,
    button: 8,
    row: 4,
};

/// Distances in frame pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// Space between the frame edge and the card.
    pub margin: f32,
    /// Space between two buttons.
    pub button_gap: f32,
    /// Space added under each list row and footer line, beyond the font size.
    pub row_padding: f32,
    /// Width of a button.
    pub button_width: f32,
    /// Height of a button.
    pub button_height: f32,
    /// Width of the focus outline.
    pub focus_width: f32,
}

/// The spacing the dialogs use.
pub const SPACING: Spacing = Spacing {
    margin: 24.0,
    button_gap: 16.0,
    row_padding: 10.0,
    button_width: 128.0,
    button_height: 40.0,
    focus_width: 2.0,
};

/// Font sizes in points. The body size is the one `Theme` carries; titles add
/// [`TypeScale::TITLE_STEP`] to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    pub body: f32,
}

impl TypeScale {
    /// How much larger a card title is than its body text.
    pub const TITLE_STEP: f32 = 4.0;

    /// The body font.
    #[must_use]
    pub fn body_font(self) -> FontId {
        FontId::proportional(self.body)
    }

    /// The card title font.
    #[must_use]
    pub fn title_font(self) -> FontId {
        FontId::proportional(self.body + Self::TITLE_STEP)
    }
}

/// The type scale the dialogs use.
pub const TYPE: TypeScale = TypeScale { body: 16.0 };

/// Corner radius for a card.
#[must_use]
pub const fn card_radius() -> CornerRadius {
    CornerRadius::same(RADII.card)
}

/// Corner radius for a button.
#[must_use]
pub const fn button_radius() -> CornerRadius {
    CornerRadius::same(RADII.button)
}

/// Corner radius for a list row.
#[must_use]
pub const fn row_radius() -> CornerRadius {
    CornerRadius::same(RADII.row)
}

/// The focus outline stroke, in the accent colour.
#[must_use]
pub fn focus_stroke(palette: &Palette) -> Stroke {
    Stroke::new(SPACING.focus_width, palette.accent)
}

/// Installs the palette and type scale on `ctx`, so egui widgets drawn outside
/// the portal painter match the dialogs.
pub fn apply(ctx: &Context, mode: Mode) {
    let palette = Palette::for_mode(mode);
    let mut visuals = Visuals::dark();
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.card;
    visuals.override_text_color = Some(palette.text);
    visuals.window_corner_radius = card_radius();
    visuals.widgets.inactive.weak_bg_fill = palette.button;
    visuals.widgets.inactive.corner_radius = button_radius();
    visuals.widgets.hovered.weak_bg_fill = palette.button;
    visuals.widgets.hovered.corner_radius = button_radius();
    visuals.widgets.active.weak_bg_fill = palette.accent;
    visuals.widgets.active.corner_radius = button_radius();
    visuals.selection.bg_fill = palette.accent;
    visuals.selection.stroke = focus_stroke(&palette);
    ctx.set_visuals(visuals);

    let body = TYPE.body_font();
    let title = TYPE.title_font();
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(TextStyle::Body, body.clone());
        style.text_styles.insert(TextStyle::Button, body.clone());
        style.text_styles.insert(TextStyle::Small, body.clone());
        style.text_styles.insert(TextStyle::Heading, title.clone());
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::new(TYPE.body, FontFamily::Monospace));
        style.spacing.item_spacing.x = SPACING.button_gap;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_palette_matches_the_values_the_dialogs_drew_with() {
        assert_eq!(Palette::for_mode(Mode::Dark), DARK);
        assert_eq!(DARK.background, Color32::from_rgb(20, 22, 27));
        assert_eq!(DARK.accent, Color32::from_rgb(78, 140, 230));
    }

    #[test]
    fn scale_values_are_the_dialog_metrics() {
        assert_eq!(RADII.card, 12);
        assert_eq!(RADII.button, 8);
        assert_eq!(SPACING.margin, 24.0);
        assert_eq!(SPACING.button_gap, 16.0);
        assert_eq!(TYPE.title_font().size, 20.0);
        assert_eq!(TYPE.body_font().size, 16.0);
    }

    #[test]
    fn apply_installs_the_palette_on_the_context() {
        let ctx = Context::default();
        apply(&ctx, Mode::Dark);
        let style = ctx.style_of(egui::Theme::Dark);
        assert_eq!(style.visuals.panel_fill, DARK.background);
        assert_eq!(style.text_styles[&TextStyle::Heading].size, 20.0);
    }
}
