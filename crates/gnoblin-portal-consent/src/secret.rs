//! The passphrase dialog: a lock, the question, who asks, a masked entry, Cancel and Unlock.
//!
//! The compositor draws this when no shell has claimed `prompt.secret`, or the claimant went away,
//! so the keyring can always be unlocked (design/js-research/popups-portals.md, row 5). Its one
//! difference from [`ConsentUi`](crate::ConsentUi) is that it holds a secret, which shapes how keys
//! reach it:
//!
//! - Typed characters, Backspace, Enter and Escape are given to [`SecretUi`] by its owner through
//!   [`SecretUi::type_char`] and friends, never as egui events. egui keeps a copy of every key event
//!   for the frame and a text widget keeps undo history, neither of which is ever wiped, so the
//!   entry is not an egui text edit: egui only draws one bullet per character and takes the clicks
//!   on the two buttons.
//! - The text lives in a [`SecretText`]. Each edit builds the new value and wipes the old one, so no
//!   earlier state of the passphrase outlives the keystroke that replaced it.
//! - `Debug` shows how many characters were typed, never which.

use std::f32::consts::PI;
use std::fmt;

use egui::{Frame, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2};
use gnoblin_portal_core::model::Settled;
use gnoblin_portal_ui::style::decision_buttons;
use gnoblin_portal_ui::ui::DialogUi;
use gnoblin_secret::SecretText;
use gnoblin_types::Tick;

/// The most characters the entry takes. A passphrase longer than this is not typed by a person, and
/// the cap keeps the entry from growing without bound under a held key.
const MAX_CHARACTERS: usize = 1024;

/// The character that stands for each typed one.
const BULLET: char = '\u{2022}';

/// How a passphrase dialog ended.
#[derive(Debug, PartialEq, Eq)]
pub enum SecretResult {
    /// The user confirmed. The passphrase is wiped when this drops.
    Answered(SecretText),
    /// The user declined: Cancel or Escape.
    Declined,
}

/// A passphrase request on screen. Owns the entry and, once it ends, the result.
pub struct SecretUi {
    title: String,
    message: String,
    requester: String,
    entry: SecretText,
    characters: usize,
    result: Option<SecretResult>,
}

impl SecretUi {
    /// A dialog titled `title` that asks `message` on behalf of `requester`, which is shown when it is
    /// not empty. All three come from the request, never from the user's own configuration.
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>, requester: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            requester: requester.into(),
            entry: SecretText::default(),
            characters: 0,
            result: None,
        }
    }

    /// Whether the dialog ended and its result is waiting for [`SecretUi::take_result`].
    #[must_use]
    pub fn ended(&self) -> bool {
        self.result.is_some()
    }

    /// How many characters are typed.
    #[must_use]
    pub fn typed(&self) -> usize {
        self.characters
    }

    /// Appends one typed character. Control characters and anything past the cap are ignored, and
    /// nothing is taken once the dialog ended.
    pub fn type_char(&mut self, character: char) {
        if self.ended() || character.is_control() || self.characters >= MAX_CHARACTERS {
            return;
        }
        let mut next = String::with_capacity(self.entry.expose().len() + character.len_utf8());
        next.push_str(self.entry.expose());
        next.push(character);
        self.replace(next);
        self.characters += 1;
    }

    /// Removes the last typed character.
    pub fn backspace(&mut self) {
        if self.ended() || self.characters == 0 {
            return;
        }
        let kept = self.entry.expose();
        let end = kept.char_indices().last().map_or(0, |(index, _)| index);
        let mut next = String::with_capacity(end);
        next.push_str(&kept[..end]);
        self.replace(next);
        self.characters -= 1;
    }

    /// Confirms the entry. An empty entry confirms nothing: an Enter that was a slip must not send
    /// an empty passphrase as the answer.
    pub fn submit(&mut self) {
        if self.ended() || self.characters == 0 {
            return;
        }
        self.result = Some(SecretResult::Answered(std::mem::take(&mut self.entry)));
        self.characters = 0;
    }

    /// Declines the request and wipes what was typed.
    pub fn cancel(&mut self) {
        if self.ended() {
            return;
        }
        self.entry.wipe();
        self.characters = 0;
        self.result = Some(SecretResult::Declined);
    }

    /// Takes how the dialog ended, once.
    pub fn take_result(&mut self) -> Option<SecretResult> {
        self.result.take()
    }

    /// Swaps in `next` as the entry and wipes the value it replaces.
    fn replace(&mut self, next: String) {
        let mut old = std::mem::replace(&mut self.entry, SecretText::new(next));
        old.wipe();
    }
}

impl fmt::Debug for SecretUi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretUi")
            .field("title", &self.title)
            .field("requester", &self.requester)
            .field("typed", &self.characters)
            .field("ended", &self.ended())
            .finish_non_exhaustive()
    }
}

impl DialogUi for SecretUi {
    fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
        ui.horizontal(|ui| {
            lock_icon(ui);
            ui.heading(&self.title);
        });
        ui.label(&self.message);
        if !self.requester.is_empty() {
            ui.label(format!("Requested by {}", self.requester));
        }
        let mask: String = std::iter::repeat_n(BULLET, self.characters).collect();
        Frame::new()
            .stroke(ui.visuals().widgets.inactive.bg_stroke)
            .corner_radius(4.0)
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.set_min_width(300.0);
                ui.label(RichText::new(format!("{mask}|")).monospace());
            });
        // Cancel is the Deny side: first in every layout and never smaller than Unlock. Unlock waits for a character.
        let buttons = decision_buttons(ui, "Cancel", "Unlock", self.characters > 0);
        if buttons.deny.clicked() {
            self.cancel();
        }
        if buttons.accept.clicked() {
            self.submit();
        }
        None
    }
}

/// A padlock, drawn from shapes so it needs no icon font. It is part of the dialog the compositor
/// draws, so a prompt that shows it is the compositor's own.
fn lock_icon(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(22.0, 26.0), Sense::hover());
    let color = ui.visuals().strong_text_color();
    let painter = ui.painter();
    let shoulder = rect.top() + 11.0;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(rect.left(), shoulder), rect.right_bottom()),
        3.0,
        color,
    );
    let (centre, radius) = (Pos2::new(rect.center().x, shoulder), 6.5_f32);
    let shackle: Vec<Pos2> = (0..=16_u8)
        .map(|step| {
            let angle = PI * f32::from(step) / 16.0;
            Pos2::new(centre.x - radius * angle.cos(), centre.y - radius * angle.sin())
        })
        .collect();
    painter.add(Shape::line(shackle, Stroke::new(2.5, color)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_portal_core::model::ScreenKey;
    use gnoblin_portal_ui::style::{ButtonLayout, DialogStyle, StyleRequest};
    use gnoblin_portal_ui::ui::{DialogScheme, DialogShell, Setup, key_events};

    fn setup() -> Setup {
        Setup {
            width: 480,
            height: 300,
            reduced_motion: true,
            scheme: DialogScheme::Dark,
            style: DialogStyle::default(),
        }
    }

    fn dialog() -> SecretUi {
        SecretUi::new("Unlock the keyring", "Enter your passphrase", "keyring")
    }

    fn typed(ui: &mut SecretUi, text: &str) {
        for character in text.chars() {
            ui.type_char(character);
        }
    }

    #[test]
    fn typed_text_is_the_answer_on_submit() {
        let mut ui = dialog();
        typed(&mut ui, "correct horse");
        ui.submit();
        match ui.take_result() {
            Some(SecretResult::Answered(secret)) => assert_eq!(secret.expose(), "correct horse"),
            other => panic!("expected the passphrase, got {other:?}"),
        }
        assert!(ui.take_result().is_none(), "the result is taken once");
    }

    #[test]
    fn backspace_removes_whole_characters() {
        let mut ui = dialog();
        typed(&mut ui, "caf\u{e9}");
        ui.backspace();
        ui.backspace();
        typed(&mut ui, "t");
        ui.submit();
        match ui.take_result() {
            Some(SecretResult::Answered(secret)) => assert_eq!(secret.expose(), "cat"),
            other => panic!("expected the passphrase, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_entry_confirms_nothing() {
        let mut ui = dialog();
        ui.submit();
        assert!(!ui.ended(), "an Enter on an empty entry is a slip, not an answer");
        typed(&mut ui, "a");
        ui.backspace();
        ui.submit();
        assert!(!ui.ended());
    }

    #[test]
    fn cancel_declines_and_forgets_the_entry() {
        let mut ui = dialog();
        typed(&mut ui, "hunter2");
        ui.cancel();
        assert_eq!(ui.typed(), 0);
        ui.type_char('x');
        assert_eq!(ui.typed(), 0, "an ended dialog takes no more text");
        assert_eq!(ui.take_result(), Some(SecretResult::Declined));
    }

    #[test]
    fn control_characters_are_not_text() {
        let mut ui = dialog();
        ui.type_char('\n');
        ui.type_char('\u{8}');
        assert_eq!(ui.typed(), 0);
    }

    #[test]
    fn the_entry_stops_growing_at_the_cap() {
        let mut ui = dialog();
        for _ in 0..MAX_CHARACTERS + 10 {
            ui.type_char('a');
        }
        assert_eq!(ui.typed(), MAX_CHARACTERS);
    }

    #[test]
    fn debug_never_shows_the_passphrase() {
        let mut ui = dialog();
        typed(&mut ui, "swordfish");
        let shown = format!("{ui:?}");
        assert!(!shown.contains("swordfish"), "{shown}");
        assert!(shown.contains("typed: 9"), "{shown}");
        ui.submit();
        let result = ui.take_result();
        let shown = format!("{result:?}");
        assert!(!shown.contains("swordfish"), "{shown}");
    }

    #[test]
    fn the_frame_shows_one_mark_per_typed_character_and_never_the_text() {
        let mut shell = DialogShell::new(setup());
        let mut ui = dialog();
        let empty = shell.render(&mut ui, Tick::new(0));
        typed(&mut ui, "swordfish");
        let full = shell.render(&mut ui, Tick::new(0));
        assert_ne!(empty.bytes(), full.bytes(), "the entry draws what was typed");
        assert_eq!(ui.typed(), 9);
    }

    /// Every style the tests care about: stock, and the strangest one a caller can reach after clamping.
    fn styles() -> [DialogStyle; 2] {
        let extreme = StyleRequest {
            radius: Some(i64::MAX),
            accent: Some([27, 27, 27]),
            font_family: Some("monospace".to_owned()),
            font_size: Some(i64::MIN),
            scale: Some(1.0e9),
            button_layout: Some(ButtonLayout::Stacked),
        }
        .clamp();
        [DialogStyle::default(), extreme]
    }

    #[test]
    fn the_lock_the_title_and_the_requester_are_drawn_under_any_style() {
        for style in styles() {
            let mut shell = DialogShell::new(Setup { style, ..setup() });
            let mut ui = dialog();
            let shown = shell.shown(&mut ui, Tick::new(0));
            for wanted in ["Unlock the keyring", "Requested by keyring", "Cancel", "Unlock"] {
                assert!(
                    shown.texts.iter().any(|text| text == wanted),
                    "{wanted:?} is missing under {style:?}: {:?}",
                    shown.texts
                );
            }
            // The padlock's body is the 22 by 15 point rectangle `lock_icon` fills.
            assert!(
                shown
                    .rects
                    .iter()
                    .any(|rect| (rect.width() - 22.0).abs() < 0.5 && (rect.height() - 15.0).abs() < 0.5),
                "the lock icon is missing under {style:?}"
            );
        }
    }

    #[test]
    fn a_styled_dialog_still_never_draws_the_typed_text() {
        for style in styles() {
            let mut shell = DialogShell::new(Setup { style, ..setup() });
            let mut ui = dialog();
            typed(&mut ui, "swordfish");
            let shown = shell.shown(&mut ui, Tick::new(0));
            assert!(
                shown.texts.iter().all(|text| !text.contains("swordfish")),
                "{:?}",
                shown.texts
            );
            assert!(
                shown.texts.iter().any(|text| text.starts_with(BULLET)),
                "one mark per typed character is drawn"
            );
        }
    }

    #[test]
    fn unlock_is_reached_by_keyboard_under_any_style() {
        for style in styles() {
            let mut shell = DialogShell::new(Setup { style, ..setup() });
            let mut ui = dialog();
            typed(&mut ui, "pw");
            shell.layout(&mut ui, Tick::new(0));
            // Tab walks Cancel then Unlock in every layout, and Enter presses the focused button.
            for key in [ScreenKey::Tab, ScreenKey::Tab, ScreenKey::Enter] {
                shell.run(&mut ui, key_events(key, false), Tick::new(0));
            }
            assert!(
                matches!(ui.take_result(), Some(SecretResult::Answered(_))),
                "Unlock answers under {style:?}"
            );
        }
    }
}
