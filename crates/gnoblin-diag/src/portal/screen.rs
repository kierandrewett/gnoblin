//! egui screens for the built-in portal dialogs (D5, D19).
//!
//! A [`Screen`] wraps one dialog model and draws it with egui. The models stay
//! the source of truth: a screen forwards key, pointer and tick input to the
//! model and reports the model's decision as a [`Settled`] value. It does no I/O
//! and reads no clock: every input carries its own [`Tick`].
//!
//! Failure modes, decided here rather than left to the caller:
//! - Input after a settled result is ignored and returns `None`.
//! - A pointer press on a button that is not under the pointer resolves to
//!   "elsewhere", which the model ignores.
//! - A disarmed Allow button ignores pointer and Enter input until the arm
//!   delay has passed; the model decides this, the screen only shows it.
//! - Reduced motion (the caller's copy of the settings value, in [`Setup`])
//!   makes every animated value jump to its target immediately.
//! - egui text that cannot be laid out is not drawn; no fallback glyph is
//!   invented. The bundled default font covers the Latin range used by prompts.

use std::fmt::Write as _;

use egui::epaint::ClippedPrimitive;
use egui::{Color32, Context, CornerRadius, FontId, Painter, Pos2, RawInput, Rect, Stroke, StrokeKind, Ui, Vec2};
use gnoblin_types::Tick;

use super::dialog::{Consent, ConsentFocus, ConsentInput, ConsentTarget, Key};
use super::interface::Grant;
use crate::paint::{Frame, Textures, rasterise};

/// One egui point is one frame pixel. The headless renderer does not scale.
const PIXELS_PER_POINT: f32 = 1.0;
/// Space between the frame edge and the card.
const MARGIN: f32 = 24.0;
const CARD_RADIUS: u8 = 12;
const BUTTON_SIZE: Vec2 = Vec2::new(128.0, 40.0);
const BUTTON_GAP: f32 = 16.0;
const BUTTON_RADIUS: u8 = 8;
const FOCUS_WIDTH: f32 = 2.0;
/// Seconds a state change takes to animate when motion is allowed.
const ANIMATION_SECONDS: f32 = 0.15;

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
        Self {
            background: Color32::from_rgb(20, 22, 27),
            card: Color32::from_rgb(34, 37, 44),
            text: Color32::from_rgb(235, 236, 240),
            muted: Color32::from_rgb(150, 155, 166),
            accent: Color32::from_rgb(78, 140, 230),
            button: Color32::from_rgb(58, 62, 72),
            button_disarmed: Color32::from_rgb(46, 49, 58),
            font_size: 16.0,
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

/// A key as the screens receive it. Keys a dialog does not use are ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKey {
    Escape,
    Enter,
    Tab,
    Up,
    Down,
    Space,
    Backspace,
    Text(char),
}

/// The result a screen reports once its model has decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    Granted(Grant),
    Denied,
    Cancelled,
}

/// A text run the last layout drew: its text and the rectangle its galley
/// occupies, in frame pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    pub rect: Rect,
}

/// The dialog a screen draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    Consent(Consent),
}

/// Pointer targets the last layout drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Allow,
    Deny,
}

/// What one layout pass produced besides the egui shapes.
#[derive(Default)]
struct Drawn {
    labels: Vec<Label>,
    hits: Vec<(Hit, Rect)>,
}

/// One dialog on screen. Holds the model, the egui context and the CPU textures.
pub struct Screen {
    ctx: Context,
    textures: Textures,
    dialog: Dialog,
    setup: Setup,
    now: Tick,
    settled: Option<Settled>,
    hits: Vec<(Hit, Rect)>,
    labels: Vec<Label>,
}

impl Screen {
    /// A screen for `dialog` at `now`. The first layout runs here, so pointer
    /// hit-testing works before the first [`Screen::render`].
    #[must_use]
    pub fn new(dialog: Dialog, setup: Setup, now: Tick) -> Self {
        let mut screen = Self {
            ctx: Context::default(),
            textures: Textures::default(),
            dialog,
            setup,
            now,
            settled: None,
            hits: Vec::new(),
            labels: Vec::new(),
        };
        screen.layout();
        screen
    }

    /// The settled result, once the model has decided or the screen was cancelled.
    #[must_use]
    pub const fn settled(&self) -> Option<&Settled> {
        self.settled.as_ref()
    }

    /// The text runs of the last layout, in draw order.
    #[must_use]
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Moves the screen's clock. Animations and the arm delay read this value.
    pub const fn tick(&mut self, now: Tick) {
        self.now = now;
    }

    /// Feeds one key press or repeat.
    pub fn key(&mut self, key: ScreenKey, repeat: bool) -> Option<Settled> {
        let key = match key {
            ScreenKey::Escape => Key::Escape,
            ScreenKey::Enter => Key::Enter,
            ScreenKey::Tab => Key::Tab,
            _ => return None,
        };
        self.consent_input(ConsentInput::Key { key, repeat })
    }

    /// Feeds a pointer press at frame pixel `(x, y)`.
    pub fn pointer_press(&mut self, x: f32, y: f32) -> Option<Settled> {
        let target = self.target_at(x, y);
        self.consent_input(ConsentInput::PointerPress { target })
    }

    /// Feeds a pointer release at frame pixel `(x, y)`.
    pub fn pointer_release(&mut self, x: f32, y: f32) -> Option<Settled> {
        let target = self.target_at(x, y);
        self.consent_input(ConsentInput::PointerRelease { target })
    }

    /// Settles the screen as cancelled: the requester went away or the user
    /// dismissed the dialog outside the model. Returns `None` once settled.
    pub fn cancel(&mut self) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        self.settled = Some(Settled::Cancelled);
        self.settled.clone()
    }

    /// Lays out and rasterises the current state.
    #[must_use]
    pub fn render(&mut self) -> Frame {
        let output = self.layout();
        let primitives: Vec<ClippedPrimitive> = self.ctx.tessellate(output.shapes, PIXELS_PER_POINT);
        let mut frame = Frame::filled(
            self.setup.width,
            self.setup.height,
            self.setup.theme.background.to_array(),
        );
        rasterise(&mut frame, &self.textures, &primitives, PIXELS_PER_POINT);
        frame
    }

    /// A plain-text description of what the model holds and what is on screen.
    /// Stable for the same inputs; used as the golden structure check.
    #[must_use]
    pub fn structure(&self) -> String {
        let mut out = String::new();
        match &self.dialog {
            Dialog::Consent(consent) => {
                let _ = writeln!(out, "dialog consent");
                let _ = writeln!(out, "app {}", consent.app_id());
                let _ = writeln!(out, "prompt {}", consent.prompt());
                let _ = writeln!(out, "focus {:?}", consent.focus());
                let _ = writeln!(out, "armed {}", consent.armed(self.now));
                let _ = writeln!(out, "buttons Deny Allow");
                let _ = writeln!(out, "decision {:?}", consent.decision());
            }
        }
        let _ = writeln!(out, "settled {:?}", self.settled);
        out
    }

    fn consent_input(&mut self, input: ConsentInput) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        let now = self.now;
        let decision = match &mut self.dialog {
            Dialog::Consent(consent) => consent.input(input, now),
        }?;
        let settled = match Consent::grant(decision) {
            Some(grant) => Settled::Granted(grant),
            None => Settled::Denied,
        };
        self.settled = Some(settled.clone());
        Some(settled)
    }

    fn target_at(&mut self, x: f32, y: f32) -> ConsentTarget {
        self.layout();
        let point = Pos2::new(x, y);
        match self.hits.iter().find(|(_, rect)| rect.contains(point)) {
            Some((Hit::Allow, _)) => ConsentTarget::Allow,
            Some((Hit::Deny, _)) => ConsentTarget::Deny,
            None => ConsentTarget::Elsewhere,
        }
    }

    /// Runs egui over the current state, records the hit rectangles and labels,
    /// and applies the texture delta. The delta is applied and its frees are
    /// processed here, before any mesh of this pass is painted: no mesh of this
    /// pass references a freed texture, and the CPU painter has no earlier
    /// frame still in flight.
    fn layout(&mut self) -> egui::FullOutput {
        let frame_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(self.setup.width as f32, self.setup.height as f32));
        let raw = RawInput {
            screen_rect: Some(frame_rect),
            time: Some(self.now.get() as f64 / 1000.0),
            ..RawInput::default()
        };
        let mut drawn = Drawn::default();
        let dialog = &self.dialog;
        let setup = &self.setup;
        let now = self.now;
        let mut output = self.ctx.run_ui(raw, |ui| match dialog {
            Dialog::Consent(consent) => draw_consent(ui, consent, setup, now, &mut drawn),
        });
        self.textures.update(&output.textures_delta);
        self.textures.free(&output.textures_delta);
        // egui panics when a TexturesDelta is dropped with unapplied deltas.
        output.textures_delta.clear();
        self.hits = drawn.hits;
        self.labels = drawn.labels;
        output
    }
}

/// Draws a consent card: prompt, requester, and the Deny and Allow buttons.
fn draw_consent(ui: &Ui, consent: &Consent, setup: &Setup, now: Tick, drawn: &mut Drawn) {
    let theme = &setup.theme;
    let painter = ui.painter();
    let frame_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(setup.width as f32, setup.height as f32));
    let card = frame_rect.shrink(MARGIN);
    painter.rect_filled(card, CornerRadius::same(CARD_RADIUS), theme.card);

    let wrap = card.width() - 2.0 * MARGIN;
    let left = card.left() + MARGIN;
    let title_font = FontId::proportional(theme.font_size + 4.0);
    let body_font = FontId::proportional(theme.font_size);
    let title = draw_text(
        painter,
        drawn,
        consent.prompt(),
        title_font,
        theme.text,
        wrap,
        Pos2::new(left, card.top() + MARGIN),
    );
    draw_text(
        painter,
        drawn,
        &format!("Requested by {}", consent.app_id()),
        body_font.clone(),
        theme.muted,
        wrap,
        Pos2::new(left, title.bottom() + 8.0),
    );

    let row_top = card.bottom() - MARGIN - BUTTON_SIZE.y;
    let allow_left = card.right() - MARGIN - BUTTON_SIZE.x;
    let deny_rect = Rect::from_min_size(Pos2::new(allow_left - BUTTON_GAP - BUTTON_SIZE.x, row_top), BUTTON_SIZE);
    let allow_rect = Rect::from_min_size(Pos2::new(allow_left, row_top), BUTTON_SIZE);

    // The arm animation is keyed on the app id, so two dialogs never share it.
    let arm_time = if setup.reduced_motion { 0.0 } else { ANIMATION_SECONDS };
    let arm_progress = ui.ctx().animate_bool_with_time(
        ui.scope_id().with(("consent-arm", consent.app_id())),
        consent.armed(now),
        arm_time,
    );
    let allow_fill = theme.button_disarmed.lerp_to_gamma(theme.accent, arm_progress);

    let radius = CornerRadius::same(BUTTON_RADIUS);
    painter.rect_filled(deny_rect, radius, theme.button);
    painter.rect_filled(allow_rect, radius, allow_fill);
    draw_centred(painter, drawn, "Deny", deny_rect, body_font.clone(), theme.text);
    draw_centred(painter, drawn, "Allow", allow_rect, body_font, theme.text);

    let focused = match consent.focus() {
        ConsentFocus::Deny => deny_rect,
        ConsentFocus::Allow => allow_rect,
    };
    painter.rect_stroke(
        focused.expand(FOCUS_WIDTH),
        radius,
        Stroke::new(FOCUS_WIDTH, theme.accent),
        StrokeKind::Outside,
    );

    drawn.hits.push((Hit::Deny, deny_rect));
    drawn.hits.push((Hit::Allow, allow_rect));
}

/// Lays out and draws one text run at `pos`, recording its label. Returns the
/// rectangle the galley occupies.
fn draw_text(
    painter: &Painter,
    drawn: &mut Drawn,
    text: &str,
    font: FontId,
    color: Color32,
    wrap: f32,
    pos: Pos2,
) -> Rect {
    let galley = painter.layout(text.to_owned(), font, color, wrap);
    let rect = Rect::from_min_size(pos, galley.size());
    painter.galley(pos, galley, color);
    drawn.labels.push(Label {
        text: text.to_owned(),
        rect,
    });
    rect
}

/// Draws `text` centred in `area`.
fn draw_centred(painter: &Painter, drawn: &mut Drawn, text: &str, area: Rect, font: FontId, color: Color32) {
    let galley = painter.layout(text.to_owned(), font, color, area.width());
    let pos = area.center() - galley.size() / 2.0;
    let rect = Rect::from_min_size(pos, galley.size());
    painter.galley(pos, galley, color);
    drawn.labels.push(Label {
        text: text.to_owned(),
        rect,
    });
}
