//! Painter helpers the portal dialogs share: text runs that record their label
//! for headless checks, centred button labels, and focus outlines. They draw
//! with the tokens of `crate::theme` and know nothing about portal interfaces.

use crate::theme::{RADII, SPACING};
use egui::{Color32, CornerRadius, FontId, Painter, Pos2, Rect, Stroke, StrokeKind};

/// A text run the last layout drew: its text and the rectangle its galley
/// occupies, in frame pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    pub rect: Rect,
}

/// Lays out and draws one text run at `pos`, recording its label in `labels`.
/// Returns the rectangle the galley occupies.
pub fn text(
    painter: &Painter,
    labels: &mut Vec<Label>,
    text: &str,
    font: FontId,
    color: Color32,
    wrap: f32,
    pos: Pos2,
) -> Rect {
    let galley = painter.layout(text.to_owned(), font, color, wrap);
    let rect = Rect::from_min_size(pos, galley.size());
    painter.galley(pos, galley, color);
    labels.push(Label {
        text: text.to_owned(),
        rect,
    });
    rect
}

/// Draws `text` centred in `area`, recording its label in `labels`.
pub fn centred(painter: &Painter, labels: &mut Vec<Label>, text: &str, area: Rect, font: FontId, color: Color32) {
    let galley = painter.layout(text.to_owned(), font, color, area.width());
    let pos = area.center() - galley.size() / 2.0;
    let rect = Rect::from_min_size(pos, galley.size());
    painter.galley(pos, galley, color);
    labels.push(Label {
        text: text.to_owned(),
        rect,
    });
}

/// Outlines a list row or filter line that has focus, inside its bounds.
pub fn focus_outline(painter: &Painter, rect: Rect, accent: Color32) {
    painter.rect_stroke(
        rect,
        CornerRadius::same(RADII.row),
        Stroke::new(SPACING.focus_width, accent),
        StrokeKind::Inside,
    );
}

/// Outlines a button or card from the outside, by the focus width.
pub fn focus_ring(painter: &Painter, rect: Rect, radius: CornerRadius, accent: Color32) {
    painter.rect_stroke(
        rect.expand(SPACING.focus_width),
        radius,
        Stroke::new(SPACING.focus_width, accent),
        StrokeKind::Outside,
    );
}
