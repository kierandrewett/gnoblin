//! Shared drawing for the portal dialogs: the frame geometry, the theme, and the text,
//! list and button helpers the consent and chooser screens draw with.

use egui::{Color32, CornerRadius, FontId, Painter, Pos2, Rect, Vec2};

use crate::model::{ConsentTarget, DialogError};
use crate::theme::{Mode, Palette, RADII, SPACING, TYPE};
use crate::widgets::{self, Label};

/// Space between the frame edge and the card.
pub const MARGIN: f32 = SPACING.margin;
pub const CARD_RADIUS: u8 = RADII.card;
pub const BUTTON_SIZE: Vec2 = Vec2::new(SPACING.button_width, SPACING.button_height);
pub const BUTTON_GAP: f32 = SPACING.button_gap;
pub const BUTTON_RADIUS: u8 = RADII.button;
pub const FOCUS_WIDTH: f32 = SPACING.focus_width;
/// Seconds a state change takes to animate when motion is allowed.
pub const ANIMATION_SECONDS: f32 = 0.15;
/// Space added under each list row and footer line, beyond the font size.
pub const ROW_PADDING: f32 = SPACING.row_padding;

/// Colours and type size. Config wiring that fills this from the settings is a
/// later lane; until then the screens use [`Theme::default`].
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub background: Color32,
    pub card: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub button: Color32,
    pub button_disarmed: Color32,
    pub font_size: f32,
}

impl Default for Theme {
    fn default() -> Self {
        let palette = Palette::for_mode(Mode::Dark);
        Self {
            background: palette.background,
            card: palette.card,
            text: palette.text,
            muted: palette.muted,
            accent: palette.accent,
            button: palette.button,
            button_disarmed: palette.button_disarmed,
            font_size: TYPE.body,
        }
    }
}

/// How a screen is drawn: frame size in pixels, theme, and the caller's copy of
/// the `reduced_motion` setting.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub width: usize,
    pub height: usize,
    pub theme: Theme,
    pub reduced_motion: bool,
}

/// The control a chooser's keys reach. Tab moves between the ones a dialog has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The entries or applications. Up and Down move the cursor.
    List,
    /// The save name of a FileChooser in Save mode. Text keys edit it.
    Name,
    /// A FileChooser's filter (Up and Down choose a filter), or an AppChooser's
    /// filter text (text keys edit it).
    Filter,
    /// The "remember this choice" checkbox of an AppChooser. Space toggles it.
    Remember,
}

/// What a chooser draws beyond its model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChooserView {
    pub cursor: usize,
    pub focus: Focus,
    pub remember: bool,
    pub error: Option<DialogError>,
}

/// What one layout pass produced besides the egui shapes.
#[derive(Default)]
pub struct Drawn {
    pub labels: Vec<Label>,
    pub hits: Vec<(ConsentTarget, Rect)>,
}

/// Lays out and draws one text run at `pos`, recording its label.
pub fn draw_text(
    painter: &Painter,
    drawn: &mut Drawn,
    text: &str,
    font: FontId,
    color: Color32,
    wrap: f32,
    pos: Pos2,
) -> Rect {
    widgets::text(painter, &mut drawn.labels, text, font, color, wrap, pos)
}

/// Draws `text` centred in `area`.
pub fn draw_centred(painter: &Painter, drawn: &mut Drawn, text: &str, area: Rect, font: FontId, color: Color32) {
    widgets::centred(painter, &mut drawn.labels, text, area, font, color);
}

/// The words a refusal shows. The model names the refusal; these are its text.
pub fn error_text(refused: DialogError) -> &'static str {
    match refused {
        DialogError::EmptyName => "Enter a name first.",
        DialogError::InvalidName => "That name is not allowed.",
        DialogError::NoSelection => "Select something first.",
        DialogError::NotSelectable => "That cannot be chosen here.",
        DialogError::NotAFile => "That name is a folder.",
        DialogError::OutOfRange => "That entry is not shown.",
        DialogError::TooMany => "Only one item can be chosen.",
        DialogError::NotOffered => "That application was not offered.",
    }
}

pub fn card_rect(setup: &Setup) -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(setup.width as f32, setup.height as f32)).shrink(MARGIN)
}

/// The Cancel and accept buttons of a chooser, bottom right of the card.
pub fn button_rects(card: Rect) -> (Rect, Rect) {
    let row_top = card.bottom() - MARGIN - BUTTON_SIZE.y;
    let accept_left = card.right() - MARGIN - BUTTON_SIZE.x;
    let accept = Rect::from_min_size(Pos2::new(accept_left, row_top), BUTTON_SIZE);
    let cancel = Rect::from_min_size(
        Pos2::new(accept_left - BUTTON_GAP - BUTTON_SIZE.x, row_top),
        BUTTON_SIZE,
    );
    (cancel, accept)
}

/// The rows of a list and the area they sit in.
pub struct ListView<'a> {
    pub rows: &'a [String],
    pub area: Rect,
    pub cursor: usize,
    pub focused: bool,
    pub empty: &'a str,
}

/// Draws the rows that fit in the list area, scrolled so the cursor row shows.
/// The cursor row is filled only while the list has focus.
pub fn draw_list(painter: &Painter, drawn: &mut Drawn, theme: &Theme, font: &FontId, view: &ListView) {
    if view.rows.is_empty() {
        let at = view.area.left_top() + Vec2::new(6.0, 5.0);
        draw_text(
            painter,
            drawn,
            view.empty,
            font.clone(),
            theme.muted,
            view.area.width(),
            at,
        );
        return;
    }
    let row = theme.font_size + ROW_PADDING;
    let fit = ((view.area.height() / row).floor() as usize).max(1);
    let first = if view.cursor >= fit { view.cursor + 1 - fit } else { 0 };
    for (offset, text) in view.rows.iter().enumerate().skip(first).take(fit) {
        let top = view.area.top() + (offset - first) as f32 * row;
        let line = Rect::from_min_size(Pos2::new(view.area.left(), top), Vec2::new(view.area.width(), row));
        if offset == view.cursor && view.focused {
            painter.rect_filled(line, CornerRadius::same(BUTTON_RADIUS / 2), theme.button);
        }
        let at = Pos2::new(view.area.left() + 6.0, top + 5.0);
        draw_text(painter, drawn, text, font.clone(), theme.text, view.area.width(), at);
    }
}

/// Outlines the control that has focus.
pub fn focus_outline(painter: &Painter, theme: &Theme, rect: Rect) {
    widgets::focus_outline(painter, rect, theme.accent);
}
