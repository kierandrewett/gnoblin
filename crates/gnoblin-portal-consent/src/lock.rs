//! The lock card: a padlock, the time of day, a masked password entry and one line that says how the last attempt went.
//!
//! The compositor draws this over its lock cover when no shell holds the lock screen (none attached, or one that died
//! or never engaged), so a locked session can always be unlocked (design/js-research/popups-portals.md, row 9). It is
//! the same kind of card as [`SecretUi`](crate::SecretUi) and shares its rules for the text:
//!
//! - Typed characters, Backspace and Enter are given to [`LockUi`] by its owner through [`LockUi::type_char`] and
//!   friends, never as egui events, because egui keeps a copy of every key event for the frame and a text widget
//!   keeps undo history, neither of which is ever wiped. egui only draws one mark per character.
//! - The text lives in a [`SecretText`]. Each edit builds the new value and wipes the old one, and the submitted
//!   password moves out of the card whole, so no earlier state of it outlives the keystroke that replaced it.
//! - `Debug` shows how many characters were typed, never which.
//!
//! The card decides nothing. It hands the password to its owner, who asks the compositor's PAM check and tells the card
//! the answer with [`LockUi::set_notice`]. While a check runs the entry takes nothing, so an Enter pressed meanwhile is
//! not a second attempt. There is no way to dismiss the card: no Escape, no button.

use std::fmt;

use egui::{Color32, Frame, RichText, Ui};
use gnoblin_portal_core::model::Settled;
use gnoblin_portal_ui::ui::DialogUi;
use gnoblin_secret::SecretText;
use gnoblin_types::Tick;

use crate::secret::lock_icon;

/// The most characters the entry takes. The compositor refuses a password over 1024 bytes, and the cap keeps the
/// entry from growing without bound under a held key.
const MAX_CHARACTERS: usize = 1024;

/// The character that stands for each typed one.
const BULLET: char = '\u{2022}';

/// What the line under the entry says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockNotice {
    /// The card is waiting for a password.
    Ready,
    /// A password is being checked.
    Checking,
    /// The password was wrong.
    Denied,
    /// Too many wrong passwords: the next attempt can be made in this many milliseconds.
    RateLimited(u64),
    /// The previous attempt is still being checked.
    Busy,
    /// The check could not be run, or the password could not be checked.
    Failed,
}

impl LockNotice {
    /// The line, in the words the stock shell's lock screen uses.
    #[must_use]
    pub fn line(self) -> String {
        match self {
            Self::Ready => "Enter your password.".to_owned(),
            Self::Checking => "Checking...".to_owned(),
            Self::Denied => "Wrong password.".to_owned(),
            Self::RateLimited(milliseconds) => {
                format!("Too many attempts. Try again in {} s.", milliseconds.div_ceil(1000))
            }
            Self::Busy => "Checking the previous attempt.".to_owned(),
            Self::Failed => "The password could not be checked.".to_owned(),
        }
    }

    /// Whether the line reports a failure, which is drawn in the error colour.
    #[must_use]
    pub fn is_error(self) -> bool {
        matches!(self, Self::Denied | Self::RateLimited(_) | Self::Busy | Self::Failed)
    }
}

/// The lock card on screen. Owns the entry and, once Enter is pressed, the password until its owner takes it.
pub struct LockUi {
    clock: String,
    notice: LockNotice,
    entry: SecretText,
    characters: usize,
    submitted: Option<SecretText>,
}

impl LockUi {
    /// A card that shows no time yet and waits for a password.
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: String::new(),
            notice: LockNotice::Ready,
            entry: SecretText::default(),
            characters: 0,
            submitted: None,
        }
    }

    /// Sets the time of day the card shows, as the owner formats it.
    pub fn set_clock(&mut self, clock: impl Into<String>) {
        self.clock = clock.into();
    }

    /// The time of day the card shows.
    #[must_use]
    pub fn clock(&self) -> &str {
        &self.clock
    }

    /// Sets the line under the entry. An answer to a check also ends the check, so the entry takes text again.
    pub fn set_notice(&mut self, notice: LockNotice) {
        self.notice = notice;
    }

    /// The line under the entry.
    #[must_use]
    pub fn notice(&self) -> LockNotice {
        self.notice
    }

    /// How many characters are typed.
    #[must_use]
    pub fn typed(&self) -> usize {
        self.characters
    }

    /// Whether a check runs, which is when the entry takes nothing.
    fn checking(&self) -> bool {
        self.notice == LockNotice::Checking
    }

    /// Appends one typed character. Control characters and anything past the cap are ignored, and so is everything
    /// while a check runs.
    pub fn type_char(&mut self, character: char) {
        if self.checking() || character.is_control() || self.characters >= MAX_CHARACTERS {
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
        if self.checking() || self.characters == 0 {
            return;
        }
        let kept = self.entry.expose();
        let end = kept.char_indices().last().map_or(0, |(index, _)| index);
        let mut next = String::with_capacity(end);
        next.push_str(&kept[..end]);
        self.replace(next);
        self.characters -= 1;
    }

    /// Confirms the entry: the password moves out of the card for [`LockUi::take_submitted`] and the card shows that
    /// it is checking. An empty entry confirms nothing, and nothing is confirmed while a check runs.
    pub fn submit(&mut self) {
        if self.checking() || self.characters == 0 {
            return;
        }
        self.submitted = Some(std::mem::take(&mut self.entry));
        self.characters = 0;
        self.notice = LockNotice::Checking;
    }

    /// Takes the password the last Enter confirmed, once.
    pub fn take_submitted(&mut self) -> Option<SecretText> {
        self.submitted.take()
    }

    /// Wipes everything typed and any password not yet taken.
    pub fn wipe(&mut self) {
        self.entry.wipe();
        self.characters = 0;
        if let Some(submitted) = self.submitted.as_mut() {
            submitted.wipe();
        }
        self.submitted = None;
    }

    /// Swaps in `next` as the entry and wipes the value it replaces.
    fn replace(&mut self, next: String) {
        let mut old = std::mem::replace(&mut self.entry, SecretText::new(next));
        old.wipe();
    }
}

impl Default for LockUi {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LockUi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockUi")
            .field("notice", &self.notice)
            .field("typed", &self.characters)
            .field("submitted", &self.submitted.is_some())
            .finish_non_exhaustive()
    }
}

impl DialogUi for LockUi {
    fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
        ui.horizontal(|ui| {
            lock_icon(ui);
            ui.heading("Locked");
        });
        if !self.clock.is_empty() {
            ui.label(RichText::new(&self.clock).size(40.0).strong());
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
        let line = RichText::new(self.notice.line());
        ui.label(if self.notice.is_error() {
            line.color(Color32::from_rgb(0xf3, 0x8b, 0xa8))
        } else {
            line
        });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_portal_ui::style::{DialogStyle, StyleRequest};
    use gnoblin_portal_ui::ui::{DialogScheme, DialogShell, Setup};

    fn setup() -> Setup {
        Setup {
            width: 480,
            height: 300,
            reduced_motion: true,
            scheme: DialogScheme::Dark,
            style: DialogStyle::default(),
        }
    }

    fn typed(ui: &mut LockUi, text: &str) {
        for character in text.chars() {
            ui.type_char(character);
        }
    }

    #[test]
    fn typed_text_is_the_password_on_submit_and_the_card_shows_it_is_checking() {
        let mut ui = LockUi::new();
        typed(&mut ui, "correct horse");
        ui.submit();
        assert_eq!(ui.notice(), LockNotice::Checking);
        assert_eq!(ui.typed(), 0, "the card keeps nothing once the password is out");
        let password = ui.take_submitted().expect("the password");
        assert_eq!(password.expose(), "correct horse");
        assert!(ui.take_submitted().is_none(), "the password is taken once");
    }

    #[test]
    fn backspace_removes_whole_characters() {
        let mut ui = LockUi::new();
        typed(&mut ui, "caf\u{e9}");
        ui.backspace();
        ui.backspace();
        typed(&mut ui, "t");
        ui.submit();
        assert_eq!(ui.take_submitted().expect("the password").expose(), "cat");
    }

    #[test]
    fn an_empty_entry_is_not_an_attempt() {
        let mut ui = LockUi::new();
        ui.submit();
        assert_eq!(ui.notice(), LockNotice::Ready);
        assert!(ui.take_submitted().is_none());
    }

    #[test]
    fn nothing_is_taken_while_a_check_runs_and_an_answer_opens_the_entry_again() {
        let mut ui = LockUi::new();
        typed(&mut ui, "one");
        ui.submit();
        typed(&mut ui, "two");
        ui.backspace();
        ui.submit();
        assert_eq!(ui.typed(), 0, "keys during the check are dropped");
        assert_eq!(
            ui.take_submitted().expect("the first password").expose(),
            "one",
            "and Enter is not a second attempt"
        );
        ui.set_notice(LockNotice::Denied);
        typed(&mut ui, "three");
        assert_eq!(ui.typed(), 5, "after the answer the entry takes text again");
    }

    #[test]
    fn control_characters_are_not_text_and_the_entry_stops_at_the_cap() {
        let mut ui = LockUi::new();
        ui.type_char('\n');
        ui.type_char('\u{8}');
        assert_eq!(ui.typed(), 0);
        for _ in 0..MAX_CHARACTERS + 10 {
            ui.type_char('a');
        }
        assert_eq!(ui.typed(), MAX_CHARACTERS);
    }

    #[test]
    fn the_notice_lines_are_the_stock_screens() {
        assert_eq!(LockNotice::Denied.line(), "Wrong password.");
        assert_eq!(
            LockNotice::RateLimited(1500).line(),
            "Too many attempts. Try again in 2 s.",
            "1500 ms rounds up to 2 s"
        );
        assert_eq!(LockNotice::Busy.line(), "Checking the previous attempt.");
        assert_eq!(LockNotice::Failed.line(), "The password could not be checked.");
        assert!(LockNotice::Denied.is_error() && !LockNotice::Ready.is_error() && !LockNotice::Checking.is_error());
    }

    #[test]
    fn debug_never_shows_the_password() {
        let mut ui = LockUi::new();
        typed(&mut ui, "swordfish");
        let shown = format!("{ui:?}");
        assert!(!shown.contains("swordfish"), "{shown}");
        assert!(shown.contains("typed: 9"), "{shown}");
        ui.submit();
        let shown = format!("{ui:?}");
        assert!(!shown.contains("swordfish"), "{shown}");
    }

    #[test]
    fn wipe_forgets_the_entry_and_a_password_not_yet_taken() {
        let mut ui = LockUi::new();
        typed(&mut ui, "hunter2");
        ui.submit();
        ui.wipe();
        assert!(ui.take_submitted().is_none());
        assert_eq!(ui.typed(), 0);
    }

    #[test]
    fn the_frame_draws_the_title_the_clock_the_notice_and_one_mark_per_character_never_the_text() {
        for style in [
            DialogStyle::default(),
            StyleRequest {
                radius: Some(i64::MAX),
                accent: Some([27, 27, 27]),
                font_family: Some("monospace".to_owned()),
                font_size: Some(i64::MIN),
                scale: Some(1.0e9),
                button_layout: None,
            }
            .clamp(),
        ] {
            let mut shell = DialogShell::new(Setup { style, ..setup() });
            let mut ui = LockUi::new();
            ui.set_clock("09:41");
            typed(&mut ui, "swordfish");
            ui.set_notice(LockNotice::Denied);
            let shown = shell.shown(&mut ui, Tick::new(0));
            for wanted in ["Locked", "09:41", "Wrong password."] {
                assert!(
                    shown.texts.iter().any(|text| text == wanted),
                    "{wanted:?} is missing under {style:?}: {:?}",
                    shown.texts
                );
            }
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
}
