//! The look a dialog may be given, and the limits that keep it safe.
//!
//! A user, a config file or a script can ask for a corner radius, an accent colour, a font size and family, a scale
//! and a button layout (`dialogs.style`). The dialogs are the compositor's own, drawn where only the compositor can
//! draw, so a restyle must never turn one into a disguise: a request is **clamped**, not trusted.
//!
//! - Text and its background keep at least [`MIN_TEXT_CONTRAST`]; the accent keeps at least
//!   [`MIN_CONTROL_CONTRAST`] against the dialog, and the text on it keeps [`MIN_TEXT_CONTRAST`]. An accent that
//!   fails is moved toward white or black until it passes ([`correct_accent`]) and the label on it is black or white,
//!   whichever reads better.
//! - A button is at least [`BUTTON_MIN_WIDTH`] by [`BUTTON_MIN_HEIGHT`] pixels, grows with the font, and Deny is
//!   never smaller than Allow: both take the width of the wider label.
//! - Font size is [`FONT_SIZE_MIN`] to [`FONT_SIZE_MAX`], the scale [`SCALE_MIN`] to [`SCALE_MAX`] (a dialog is never
//!   drawn smaller than stock), the radius at most [`RADIUS_MAX`].
//! - What no style reaches: the 500 ms arm delay (the consent model's), the requester's name, the lock icon, Deny
//!   first and holding the initial focus. There is no field for any of them, so there is nothing to clamp.
//!
//! [`StyleRequest`] is what a caller asks for and [`StyleRequest::clamp`] is the only way to a [`DialogStyle`], so a
//! dialog never sees an unclamped value.

use egui::{
    Button, Color32, CornerRadius, FontFamily, FontId, Id, Response, RichText, Stroke, Style, TextStyle, Ui, Vec2,
};

/// The least contrast ratio between text and the colour behind it (WCAG AA for body text).
pub const MIN_TEXT_CONTRAST: f64 = 4.5;
/// The least contrast ratio between a control and the dialog it sits on (WCAG AA for components).
pub const MIN_CONTROL_CONTRAST: f64 = 3.0;
/// The largest corner radius, in points.
pub const RADIUS_MAX: u8 = 24;
/// The smallest font size, in points.
pub const FONT_SIZE_MIN: u8 = 12;
/// The largest font size, in points.
pub const FONT_SIZE_MAX: u8 = 32;
/// The smallest scale: a dialog is never drawn smaller than stock.
pub const SCALE_MIN: f64 = 1.0;
/// The largest scale.
pub const SCALE_MAX: f64 = 3.0;
/// The narrowest a Deny or Allow button is drawn, in points.
pub const BUTTON_MIN_WIDTH: f32 = 88.0;
/// The shortest a Deny or Allow button is drawn, in points.
pub const BUTTON_MIN_HEIGHT: f32 = 32.0;

/// Where the resolved style sits in the egui context's temporary data, for the dialogs to read.
const STYLE_ID: &str = "gnoblin-dialog-style";

/// How the two decision buttons sit. Deny is first in every layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonLayout {
    /// Side by side at the start of the row. Today's look.
    #[default]
    Start,
    /// Side by side at the end of the row.
    End,
    /// Side by side, each taking half the row.
    Stretch,
    /// One above the other, each the full width of the row.
    Stacked,
}

impl ButtonLayout {
    /// The layout a config spelling names: `start`, `end`, `stretch` or `stacked`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            "stretch" => Some(Self::Stretch),
            "stacked" => Some(Self::Stacked),
            _ => None,
        }
    }

    /// The config spelling of the layout.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
            Self::Stretch => "stretch",
            Self::Stacked => "stacked",
        }
    }
}

/// What a caller asks for. Nothing here is trusted: [`StyleRequest::clamp`] decides what is drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleRequest {
    /// The corner radius in points.
    pub radius: Option<i64>,
    /// The accent colour, red green blue.
    pub accent: Option<[u8; 3]>,
    /// The font family name. `monospace`, `mono` and `monospaced` ask for the monospace face; any other name is the
    /// proportional face, because the dialogs draw with the fonts egui embeds and look up no system font.
    pub font_family: Option<String>,
    /// The font size in points.
    pub font_size: Option<i64>,
    /// The scale factor.
    pub scale: Option<f64>,
    /// How the decision buttons sit.
    pub button_layout: Option<ButtonLayout>,
}

impl StyleRequest {
    /// The style that is safe to draw: every number inside its range, an unset value left at stock.
    ///
    /// A scale that is not a number is the stock scale, not the smallest or the largest, so a document that
    /// cannot say what it wants gets no surprise.
    #[must_use]
    pub fn clamp(&self) -> DialogStyle {
        let scale = match self.scale {
            Some(scale) if scale.is_finite() => scale.clamp(SCALE_MIN, SCALE_MAX),
            _ => 1.0,
        };
        DialogStyle {
            radius: self.radius.map(|radius| clamp_u8(radius, 0, RADIUS_MAX)),
            accent: self.accent,
            font_size: self.font_size.map(|size| clamp_u8(size, FONT_SIZE_MIN, FONT_SIZE_MAX)),
            monospace: self.font_family.as_deref().is_some_and(is_monospace_name),
            scale_permille: scale_to_permille(scale),
            button_layout: self.button_layout.unwrap_or_default(),
        }
    }

    /// One sentence for each value [`clamp`](Self::clamp) moved, so a log can say why a dialog does not look as it was
    /// asked to. An accent is not listed: its correction depends on the dialog colour it is drawn on, and is
    /// [`correct_accent`]'s.
    #[must_use]
    pub fn adjustments(&self) -> Vec<String> {
        let style = self.clamp();
        let mut moved = Vec::new();
        if let (Some(asked), Some(drawn)) = (self.radius, style.radius())
            && asked != i64::from(drawn)
        {
            moved.push(format!("radius {asked} is drawn as {drawn} (at most {RADIUS_MAX})"));
        }
        if let (Some(asked), Some(drawn)) = (self.font_size, style.font_size())
            && asked != i64::from(drawn)
        {
            moved.push(format!(
                "font size {asked} is drawn as {drawn} ({FONT_SIZE_MIN} to {FONT_SIZE_MAX})"
            ));
        }
        if let Some(asked) = self.scale
            && (!asked.is_finite() || (asked - f64::from(style.scale())).abs() > 1e-6)
        {
            moved.push(format!(
                "scale {asked} is drawn as {} ({SCALE_MIN} to {SCALE_MAX})",
                style.scale()
            ));
        }
        moved
    }
}

fn clamp_u8(value: i64, low: u8, high: u8) -> u8 {
    u8::try_from(value.clamp(i64::from(low), i64::from(high))).unwrap_or(low)
}

fn scale_to_permille(scale: f64) -> u16 {
    // `scale` is inside 1.0..=3.0, so the product is inside 1000..=3000 and the cast cannot truncate.
    (scale * 1000.0).round() as u16
}

fn is_monospace_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "monospace" | "mono" | "monospaced"
    )
}

/// A clamped style. Only [`StyleRequest::clamp`] and [`DialogStyle::default`] make one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogStyle {
    radius: Option<u8>,
    accent: Option<[u8; 3]>,
    font_size: Option<u8>,
    monospace: bool,
    scale_permille: u16,
    button_layout: ButtonLayout,
}

impl Default for DialogStyle {
    /// The stock look: nothing restyled, scale 1, buttons at the start of the row.
    fn default() -> Self {
        Self {
            radius: None,
            accent: None,
            font_size: None,
            monospace: false,
            scale_permille: 1000,
            button_layout: ButtonLayout::Start,
        }
    }
}

impl DialogStyle {
    /// The corner radius in points, when restyled.
    #[must_use]
    pub const fn radius(&self) -> Option<u8> {
        self.radius
    }

    /// The requested accent, before it is corrected against the dialog's background.
    #[must_use]
    pub const fn accent(&self) -> Option<[u8; 3]> {
        self.accent
    }

    /// The font size in points, when restyled.
    #[must_use]
    pub const fn font_size(&self) -> Option<u8> {
        self.font_size
    }

    /// Whether the monospace face is asked for.
    #[must_use]
    pub const fn monospace(&self) -> bool {
        self.monospace
    }

    /// The scale: egui points per frame pixel.
    #[must_use]
    pub fn scale(&self) -> f32 {
        f32::from(self.scale_permille) / 1000.0
    }

    /// How the decision buttons sit.
    #[must_use]
    pub const fn button_layout(&self) -> ButtonLayout {
        self.button_layout
    }

    /// Applies the style to `style` and returns what the dialogs read back: the accent after correction.
    ///
    /// Besides the requested values this is where the contrast floor is enforced for whatever the visuals hold, so
    /// a palette that falls below it is repaired here and not trusted.
    pub fn apply(&self, style: &mut Style) -> ResolvedStyle {
        if let Some(radius) = self.radius {
            let radius = CornerRadius::same(radius);
            let visuals = &mut style.visuals;
            visuals.window_corner_radius = radius;
            visuals.menu_corner_radius = radius;
            let widgets = &mut visuals.widgets;
            for widget in [
                &mut widgets.noninteractive,
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                widget.corner_radius = radius;
            }
        }
        self.apply_fonts(style);
        guard_text_contrast(style);
        let background = rgb_of(style.visuals.window_fill);
        let accent = self.accent.map(|accent| correct_accent(accent, background));
        if let Some(accent) = accent {
            style.visuals.selection.bg_fill = accent.fill;
            style.visuals.selection.stroke = Stroke::new(1.0, accent.text);
        }
        ResolvedStyle { style: *self, accent }
    }

    fn apply_fonts(&self, style: &mut Style) {
        if self.font_size.is_none() && !self.monospace {
            return;
        }
        let family = if self.monospace {
            FontFamily::Monospace
        } else {
            FontFamily::Proportional
        };
        let stock_body = style.text_styles.get(&TextStyle::Body).map_or(14.0, |font| font.size);
        for text_style in [
            TextStyle::Body,
            TextStyle::Button,
            TextStyle::Heading,
            TextStyle::Small,
            TextStyle::Monospace,
        ] {
            let stock = style.text_styles.get(&text_style).map_or(stock_body, |font| font.size);
            let size = match (self.font_size, text_style.clone()) {
                // Small text is not drawn smaller than the body: the floor is for every line.
                (Some(base), TextStyle::Heading) => f32::from(base) * stock / stock_body,
                (Some(base), _) => f32::from(base),
                (None, _) => stock,
            };
            let family = if text_style == TextStyle::Monospace {
                FontFamily::Monospace
            } else {
                family.clone()
            };
            style.text_styles.insert(text_style, FontId::new(size, family));
        }
    }
}

/// A style as the dialogs read it from the egui context: what was asked for, and the accent that is safe to draw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResolvedStyle {
    /// The clamped style.
    pub style: DialogStyle,
    /// The accent after correction, when one was asked for.
    pub accent: Option<AccentColours>,
}

/// An accent that is safe to draw: a fill that stands out from the dialog, and the colour of the text on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentColours {
    /// The fill of a primary button and of a selection.
    pub fill: Color32,
    /// The text on the fill.
    pub text: Color32,
}

/// Puts `resolved` where the dialogs read it with [`resolved`].
pub fn publish(ctx: &egui::Context, resolved: ResolvedStyle) {
    ctx.data_mut(|data| data.insert_temp(Id::unique(STYLE_ID), resolved));
}

/// The style the shell drew this frame with: the stock one when none was published.
#[must_use]
pub fn resolved(ui: &Ui) -> ResolvedStyle {
    ui.ctx()
        .data(|data| data.get_temp::<ResolvedStyle>(Id::unique(STYLE_ID)))
        .unwrap_or_default()
}

/// The colour as red green blue; alpha is not part of a contrast ratio and a dialog's colours are opaque.
#[must_use]
pub fn rgb_of(color: Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

fn linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The relative luminance of a colour, 0 for black and 1 for white (WCAG 2).
#[must_use]
pub fn luminance(rgb: [u8; 3]) -> f64 {
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}

/// The contrast ratio of two colours, from 1 (the same) to 21 (black on white), in either order.
#[must_use]
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    let (high, low) = if a >= b { (a, b) } else { (b, a) };
    (high + 0.05) / (low + 0.05)
}

const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

/// Black or white, whichever reads better on `background`. Either one reaches at least 4.58 on any colour.
#[must_use]
pub fn readable_on(background: [u8; 3]) -> [u8; 3] {
    if contrast_ratio(WHITE, background) >= contrast_ratio(BLACK, background) {
        WHITE
    } else {
        BLACK
    }
}

fn blend(from: [u8; 3], to: [u8; 3], toward: f64) -> [u8; 3] {
    let mix = |a: u8, b: u8| {
        let value = f64::from(a) + (f64::from(b) - f64::from(a)) * toward;
        // The value is inside 0..=255 because both ends are.
        value.round() as u8
    };
    [mix(from[0], to[0]), mix(from[1], to[1]), mix(from[2], to[2])]
}

/// The accent a dialog on `background` may draw: `accent` when it already stands out by [`MIN_CONTROL_CONTRAST`],
/// otherwise the nearest colour on the line from `accent` toward white or black (whichever contrasts more with the
/// background) that does. The text on it is black or white, so it reaches [`MIN_TEXT_CONTRAST`] for any accent.
#[must_use]
pub fn correct_accent(accent: [u8; 3], background: [u8; 3]) -> AccentColours {
    let toward = readable_on(background);
    let mut fill = accent;
    let steps = 20_u8;
    for step in 1..=steps {
        if contrast_ratio(fill, background) >= MIN_CONTROL_CONTRAST {
            break;
        }
        fill = blend(accent, toward, f64::from(step) / f64::from(steps));
    }
    let text = readable_on(fill);
    AccentColours {
        fill: Color32::from_rgb(fill[0], fill[1], fill[2]),
        text: Color32::from_rgb(text[0], text[1], text[2]),
    }
}

/// Repairs any text colour of `style` that falls below [`MIN_TEXT_CONTRAST`] on what it is drawn over: body text on
/// the dialog, and button text on each button state. The stock palettes pass, so this only acts on one that does not.
fn guard_text_contrast(style: &mut Style) {
    let window = rgb_of(style.visuals.window_fill);
    if contrast_ratio(rgb_of(style.visuals.text_color()), window) < MIN_TEXT_CONTRAST {
        let fixed = readable_on(window);
        style.visuals.override_text_color = Some(Color32::from_rgb(fixed[0], fixed[1], fixed[2]));
    }
    let widgets = &mut style.visuals.widgets;
    for widget in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active] {
        let behind = rgb_of(widget.weak_bg_fill);
        if contrast_ratio(rgb_of(widget.fg_stroke.color), behind) < MIN_TEXT_CONTRAST {
            let fixed = readable_on(behind);
            widget.fg_stroke.color = Color32::from_rgb(fixed[0], fixed[1], fixed[2]);
        }
    }
}

/// The two decision buttons of a dialog, Deny first, as the style lays them out.
pub struct DecisionButtons {
    /// The Deny (or Cancel) button.
    pub deny: Response,
    /// The Allow (or Unlock) button.
    pub accept: Response,
}

/// Draws the Deny and the Allow button.
///
/// Both are at least [`BUTTON_MIN_WIDTH`] by [`BUTTON_MIN_HEIGHT`], taller with the font, and the same width: the
/// wider label sets it, so Deny is never smaller than Allow. Deny is first in every layout. The Allow button takes the
/// accent when the style has one, with text that stays legible on it; `accept_enabled` is false while the dialog is
/// not armed.
pub fn decision_buttons(ui: &mut Ui, deny: &str, accept: &str, accept_enabled: bool) -> DecisionButtons {
    let resolved = resolved(ui);
    let font = TextStyle::Button.resolve(ui.style());
    let widest = [deny, accept]
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    let padding = ui.spacing().button_padding.x * 2.0;
    let height = BUTTON_MIN_HEIGHT.max(font.size * 2.4);
    let width = BUTTON_MIN_WIDTH.max(widest + padding);
    let gap = ui.spacing().item_spacing.x;
    let row = ui.available_width();

    let mut buttons: Option<(Response, Response)> = None;
    let mut draw = |ui: &mut Ui, size: Vec2| {
        let denied = ui.add(Button::new(deny).min_size(size));
        let accepted = ui.add_enabled(accept_enabled, accept_button(accept, resolved.accent, size));
        buttons = Some((denied, accepted));
    };
    match resolved.style.button_layout() {
        ButtonLayout::Start => {
            ui.horizontal(|ui| draw(ui, Vec2::new(width, height)));
        }
        ButtonLayout::End => {
            ui.horizontal(|ui| {
                ui.add_space((row - 2.0 * width - gap).max(0.0));
                draw(ui, Vec2::new(width, height));
            });
        }
        ButtonLayout::Stretch => {
            let each = ((row - gap) / 2.0).max(width);
            ui.horizontal(|ui| draw(ui, Vec2::new(each, height)));
        }
        ButtonLayout::Stacked => {
            ui.vertical(|ui| draw(ui, Vec2::new(row.max(width), height)));
        }
    }
    // Every arm above draws both buttons, so there is always a pair.
    let (deny, accept) = buttons.unwrap_or_else(|| {
        let nothing = ui.allocate_response(Vec2::ZERO, egui::Sense::hover());
        (nothing.clone(), nothing)
    });
    DecisionButtons { deny, accept }
}

fn accept_button(label: &str, accent: Option<AccentColours>, size: Vec2) -> Button<'_> {
    match accent {
        Some(accent) => Button::new(RichText::new(label).color(accent.text))
            .fill(accent.fill)
            .min_size(size),
        None => Button::new(label).min_size(size),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::ui::{DialogScheme, DialogShell, DialogUi, Setup, press_events, release_events};
    use gnoblin_portal_core::model::Settled;
    use gnoblin_types::Tick;

    fn request() -> StyleRequest {
        StyleRequest::default()
    }

    #[test]
    fn numbers_are_clamped_into_their_ranges() {
        let style = StyleRequest {
            radius: Some(10_000),
            font_size: Some(3),
            scale: Some(0.1),
            ..request()
        }
        .clamp();
        assert_eq!(style.radius(), Some(RADIUS_MAX));
        assert_eq!(
            style.font_size(),
            Some(FONT_SIZE_MIN),
            "a dialog never reads smaller than 12"
        );
        assert!(
            (style.scale() - 1.0).abs() < f32::EPSILON,
            "never drawn smaller than stock"
        );

        let style = StyleRequest {
            radius: Some(-5),
            font_size: Some(500),
            scale: Some(40.0),
            ..request()
        }
        .clamp();
        assert_eq!(style.radius(), Some(0));
        assert_eq!(style.font_size(), Some(FONT_SIZE_MAX));
        assert!((style.scale() - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_scale_that_is_not_a_number_is_stock() {
        for scale in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let style = StyleRequest {
                scale: Some(scale),
                ..request()
            }
            .clamp();
            assert!((style.scale() - 1.0).abs() < f32::EPSILON, "{scale} is not a scale");
        }
    }

    #[test]
    fn nothing_asked_is_the_stock_style() {
        assert_eq!(request().clamp(), DialogStyle::default());
    }

    #[test]
    fn the_monospace_names_pick_the_monospace_face() {
        for name in ["monospace", "Mono", " MONOSPACED "] {
            let style = StyleRequest {
                font_family: Some(name.to_owned()),
                ..request()
            }
            .clamp();
            assert!(style.monospace(), "{name}");
        }
        let style = StyleRequest {
            font_family: Some("Inter".to_owned()),
            ..request()
        }
        .clamp();
        assert!(!style.monospace());
    }

    #[test]
    fn contrast_ratio_is_the_wcag_formula() {
        assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 1e-9);
        assert!((contrast_ratio(BLACK, WHITE) - contrast_ratio(WHITE, BLACK)).abs() < 1e-12);
    }

    #[test]
    fn black_or_white_reads_on_every_colour() {
        for red in (0..=255).step_by(17) {
            for green in (0..=255).step_by(17) {
                for blue in (0..=255).step_by(17) {
                    let background = [red, green, blue];
                    let text = readable_on(background);
                    assert!(
                        contrast_ratio(text, background) >= MIN_TEXT_CONTRAST,
                        "{background:?} has no legible black or white text"
                    );
                }
            }
        }
    }

    #[test]
    fn a_low_contrast_accent_is_corrected_on_a_dark_and_a_light_dialog() {
        let dark = rgb_of(egui::Visuals::dark().window_fill);
        let light = rgb_of(egui::Visuals::light().window_fill);
        // Nearly the dialog's own colour: invisible as a control until corrected.
        for (background, accent) in [(dark, [30, 30, 36]), (light, [245, 245, 240])] {
            assert!(
                contrast_ratio(accent, background) < MIN_CONTROL_CONTRAST,
                "the test accent must fail"
            );
            let corrected = correct_accent(accent, background);
            let fill = rgb_of(corrected.fill);
            assert!(
                contrast_ratio(fill, background) >= MIN_CONTROL_CONTRAST,
                "{fill:?} on {background:?}"
            );
            assert!(contrast_ratio(rgb_of(corrected.text), fill) >= MIN_TEXT_CONTRAST);
        }
    }

    #[test]
    fn an_accent_that_already_stands_out_is_left_alone() {
        let dark = rgb_of(egui::Visuals::dark().window_fill);
        let blue = [60, 120, 220];
        assert!(contrast_ratio(blue, dark) >= MIN_CONTROL_CONTRAST);
        assert_eq!(rgb_of(correct_accent(blue, dark).fill), blue);
    }

    #[test]
    fn every_accent_is_legible_with_its_label_on_either_scheme() {
        let backgrounds = [
            rgb_of(egui::Visuals::dark().window_fill),
            rgb_of(egui::Visuals::light().window_fill),
        ];
        for red in (0..=255).step_by(51) {
            for green in (0..=255).step_by(51) {
                for blue in (0..=255).step_by(51) {
                    for background in backgrounds {
                        let corrected = correct_accent([red, green, blue], background);
                        let fill = rgb_of(corrected.fill);
                        assert!(contrast_ratio(fill, background) >= MIN_CONTROL_CONTRAST);
                        assert!(contrast_ratio(rgb_of(corrected.text), fill) >= MIN_TEXT_CONTRAST);
                    }
                }
            }
        }
    }

    #[test]
    fn the_stock_palettes_meet_the_text_floor_and_a_broken_one_is_repaired() {
        for mut style in [Style::default(), {
            let mut light = Style::default();
            light.visuals = egui::Visuals::light();
            light
        }] {
            let before = style.visuals.clone();
            guard_text_contrast(&mut style);
            assert_eq!(
                style.visuals.override_text_color, None,
                "a stock palette is left as it is"
            );
            assert_eq!(
                style.visuals.widgets.inactive.fg_stroke,
                before.widgets.inactive.fg_stroke
            );
        }
        let mut broken = Style::default();
        broken.visuals.override_text_color = Some(broken.visuals.window_fill);
        guard_text_contrast(&mut broken);
        let fixed = broken.visuals.text_color();
        assert!(contrast_ratio(rgb_of(fixed), rgb_of(broken.visuals.window_fill)) >= MIN_TEXT_CONTRAST);
    }

    #[test]
    fn applying_a_style_sets_radius_fonts_and_the_corrected_accent() {
        let mut style = Style::default();
        let resolved = StyleRequest {
            radius: Some(14),
            accent: Some([30, 30, 36]),
            font_size: Some(18),
            font_family: Some("monospace".to_owned()),
            ..request()
        }
        .clamp()
        .apply(&mut style);
        assert_eq!(style.visuals.window_corner_radius, CornerRadius::same(14));
        assert_eq!(style.visuals.widgets.inactive.corner_radius, CornerRadius::same(14));
        let body = &style.text_styles[&TextStyle::Body];
        assert!((body.size - 18.0).abs() < f32::EPSILON);
        assert_eq!(body.family, FontFamily::Monospace);
        let accent = resolved.accent.expect("an accent was asked for");
        assert!(
            contrast_ratio(rgb_of(accent.fill), rgb_of(style.visuals.window_fill)) >= MIN_CONTROL_CONTRAST,
            "the near-black accent was corrected against the dark dialog"
        );
        assert_eq!(style.visuals.selection.bg_fill, accent.fill);
    }

    #[test]
    fn a_style_with_nothing_set_leaves_the_egui_style_as_it_was() {
        let stock = Style::default();
        let mut style = Style::default();
        let resolved = DialogStyle::default().apply(&mut style);
        assert_eq!(resolved, ResolvedStyle::default());
        assert_eq!(style.text_styles, stock.text_styles);
        assert_eq!(style.visuals.window_corner_radius, stock.visuals.window_corner_radius);
    }

    #[test]
    fn layout_names_round_trip() {
        for layout in [
            ButtonLayout::Start,
            ButtonLayout::End,
            ButtonLayout::Stretch,
            ButtonLayout::Stacked,
        ] {
            assert_eq!(ButtonLayout::from_name(layout.name()), Some(layout));
        }
        assert_eq!(ButtonLayout::from_name("sideways"), None);
    }

    /// Where the probe's two buttons were drawn, shared with the test that reads them.
    type SharedRects = Arc<Mutex<Option<(egui::Rect, egui::Rect)>>>;

    /// A dialog that is nothing but the decision buttons, so a test can read where they sit.
    struct Probe {
        rects: SharedRects,
        denied: bool,
    }

    impl DialogUi for Probe {
        fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
            let buttons = decision_buttons(ui, "Deny", "Allow every request from this program", true);
            *self.rects.lock().expect("the probe's lock") = Some((buttons.deny.rect, buttons.accept.rect));
            self.denied |= buttons.deny.clicked();
            self.denied.then_some(Settled::Denied)
        }
    }

    fn probe() -> (Probe, SharedRects) {
        let rects = Arc::new(Mutex::new(None));
        (
            Probe {
                rects: Arc::clone(&rects),
                denied: false,
            },
            rects,
        )
    }

    fn setup_with(style: DialogStyle) -> Setup {
        Setup {
            width: 1200,
            height: 800,
            reduced_motion: true,
            scheme: DialogScheme::Dark,
            style,
        }
    }

    fn rects_of(rects: &SharedRects) -> (egui::Rect, egui::Rect) {
        rects.lock().expect("the probe's lock").expect("the dialog was shown")
    }

    #[test]
    fn the_buttons_keep_their_floor_deny_comes_first_and_is_never_smaller_than_allow() {
        for layout in [
            ButtonLayout::Start,
            ButtonLayout::End,
            ButtonLayout::Stretch,
            ButtonLayout::Stacked,
        ] {
            for (font_size, scale) in [(None, None), (Some(12), Some(1.0)), (Some(30), Some(2.0))] {
                let style = StyleRequest {
                    font_size,
                    scale,
                    button_layout: Some(layout),
                    ..request()
                }
                .clamp();
                let mut shell = DialogShell::new(setup_with(style));
                let (mut dialog, rects) = probe();
                shell.layout(&mut dialog, Tick::new(0));
                let (deny, allow) = rects_of(&rects);
                let what = format!("{layout:?} {font_size:?} {scale:?}");
                assert!(
                    deny.width() + 0.5 >= BUTTON_MIN_WIDTH,
                    "{what}: Deny is {:?}",
                    deny.size()
                );
                assert!(
                    deny.height() + 0.5 >= BUTTON_MIN_HEIGHT,
                    "{what}: Deny is {:?}",
                    deny.size()
                );
                assert!(
                    allow.width() + 0.5 >= BUTTON_MIN_WIDTH,
                    "{what}: Allow is {:?}",
                    allow.size()
                );
                assert!(deny.width() + 0.5 >= allow.width(), "{what}: Deny narrower than Allow");
                assert!(deny.height() + 0.5 >= allow.height(), "{what}: Deny shorter than Allow");
                if layout == ButtonLayout::Stacked {
                    assert!(deny.max.y <= allow.min.y + 0.5, "{what}: Deny is above Allow");
                } else {
                    assert!(deny.max.x <= allow.min.x + 0.5, "{what}: Deny is before Allow");
                }
            }
        }
    }

    #[test]
    fn a_larger_font_makes_taller_buttons_and_never_shorter_ones() {
        let height_at = |size: i64| {
            let style = StyleRequest {
                font_size: Some(size),
                ..request()
            }
            .clamp();
            let mut shell = DialogShell::new(setup_with(style));
            let (mut dialog, rects) = probe();
            shell.layout(&mut dialog, Tick::new(0));
            rects_of(&rects).0.height()
        };
        assert!(height_at(32) > height_at(12));
        assert!(
            height_at(1) + 0.5 >= BUTTON_MIN_HEIGHT,
            "a font below the floor is raised to it"
        );
    }

    #[test]
    fn a_scaled_dialog_is_larger_in_pixels_and_a_click_lands_on_the_scaled_button() {
        let style = StyleRequest {
            scale: Some(2.0),
            ..request()
        }
        .clamp();
        let mut stock = DialogShell::new(setup_with(DialogStyle::default()));
        let mut scaled = DialogShell::new(setup_with(style));
        let (mut a, _) = probe();
        let (mut b, rects) = probe();
        stock.layout(&mut a, Tick::new(0));
        scaled.layout(&mut b, Tick::new(0));
        assert!(
            scaled.size().0 > stock.size().0,
            "the minimum width doubles with the scale: {:?} against {:?}",
            scaled.size(),
            stock.size()
        );
        // The button's rectangle is in points; the pointer is in frame pixels, two to the point.
        let centre = rects_of(&rects).0.center();
        let (x, y) = (centre.x * 2.0, centre.y * 2.0);
        assert_eq!(scaled.run(&mut b, press_events(x, y), Tick::new(0)), None);
        assert_eq!(
            scaled.run(&mut b, release_events(x, y), Tick::new(0)),
            Some(Settled::Denied)
        );
    }

    #[test]
    fn a_scaled_frame_is_rasterised_at_its_size() {
        let style = StyleRequest {
            scale: Some(2.0),
            ..request()
        }
        .clamp();
        let mut shell = DialogShell::new(setup_with(style));
        let (mut dialog, _) = probe();
        let frame = shell.render(&mut dialog, Tick::new(0));
        assert_eq!((frame.width(), frame.height()), shell.size());
        assert!(frame.width() >= 720, "360 points at two pixels to the point");
    }

    #[test]
    fn adjustments_name_each_value_that_was_moved_and_nothing_else() {
        assert!(request().adjustments().is_empty());
        let inside = StyleRequest {
            radius: Some(10),
            font_size: Some(16),
            scale: Some(1.5),
            ..request()
        };
        assert!(inside.adjustments().is_empty(), "{:?}", inside.adjustments());
        let outside = StyleRequest {
            radius: Some(99),
            font_size: Some(4),
            scale: Some(0.2),
            ..request()
        };
        let moved = outside.adjustments();
        assert_eq!(moved.len(), 3, "{moved:?}");
        assert!(moved[0].contains("radius 99"), "{moved:?}");
        assert!(moved[1].contains("font size 4"), "{moved:?}");
        assert!(moved[2].contains("scale 0.2"), "{moved:?}");
    }

    #[test]
    fn a_scale_the_bounds_cannot_hold_is_lowered_until_the_dialog_fits() {
        let style = StyleRequest {
            scale: Some(3.0),
            ..request()
        }
        .clamp();
        let mut shell = DialogShell::new(Setup {
            width: 480,
            height: 120,
            ..setup_with(style)
        });
        let (mut dialog, _) = probe();
        let shown = shell.shown(&mut dialog, Tick::new(0));
        let (width, height) = shell.size();
        assert!(width <= 480 && height <= 120, "{width}x{height} outgrew its bounds");
        assert!(shown.texts.iter().any(|text| text == "Deny"), "{:?}", shown.texts);
        assert!(
            shown.texts.iter().any(|text| text.starts_with("Allow")),
            "the second button is not clipped away: {:?}",
            shown.texts
        );
    }
}
