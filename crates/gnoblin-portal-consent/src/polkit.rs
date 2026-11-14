//! The polkit authentication dialog: a lock, what is being authorized, who asks, who to authenticate as, PAM's own
//! prompt with a masked entry, and Cancel.
//!
//! The compositor draws this for `auth.polkit` and checks the password with polkit's helper itself
//! (design/js-research/polkit.md). A script never draws it, reads what is typed in it or decides whether the
//! password was right. This crate holds only the card's own state: what the helper asked is told to it
//! ([`PolkitUi::ask`], [`PolkitUi::info`], [`PolkitUi::error`]), and what the user did comes back as
//! [`PolkitEvent`]s for the owner to act on ([`PolkitUi::take_events`]). The owner runs the helper.
//!
//! Like [`SecretUi`](crate::SecretUi), characters reach it through [`PolkitUi::type_char`] and not as egui events,
//! because egui keeps a copy of every key event for the frame; egui only draws one bullet per typed character (or
//! the text itself when PAM says an answer may be echoed) and takes the clicks.
//!
//! - Cancel is the first and default button in every layout, and Escape cancels.
//! - Authenticate waits for a prompt from PAM and a typed character: an Enter that was a slip answers nothing.
//! - After a failure that cannot be retried ([`PolkitUi::fail`]) only Close is left.

use std::fmt;

use egui::{Frame, RichText, Ui};
use gnoblin_portal_core::model::Settled;
use gnoblin_portal_ui::style::decision_buttons;
use gnoblin_portal_ui::ui::DialogUi;
use gnoblin_secret::SecretText;
use gnoblin_types::Tick;

use crate::secret::lock_icon;

/// The most characters an answer takes, as in the passphrase dialog.
const MAX_CHARACTERS: usize = 1024;

/// The character that stands for each typed one of a masked answer.
const BULLET: char = '\u{2022}';

/// The most notes kept on screen: PAM's last few messages, oldest dropped first.
const MAX_NOTES: usize = 4;

/// What the user did, for the owner of the dialog to act on.
#[derive(Debug, PartialEq, Eq)]
pub enum PolkitEvent {
    /// The user answered the prompt PAM asked. The owner gives it to the helper and the text is wiped when it drops.
    Reply(SecretText),
    /// The user chose another identity to authenticate as, by its index in the list the dialog was made with. The
    /// owner starts a new helper for it.
    Identity(usize),
    /// The user cancelled, by Cancel, Close or Escape.
    Cancel,
}

/// One line the helper told the user.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Note {
    Info(String),
    Error(String),
}

/// What PAM asked, while it waits for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Prompt {
    text: String,
    echo: bool,
}

/// An authentication request on screen.
pub struct PolkitUi {
    message: String,
    requester: String,
    action_id: String,
    identities: Vec<String>,
    selected: usize,
    notes: Vec<Note>,
    prompt: Option<Prompt>,
    entry: SecretText,
    characters: usize,
    /// Set when nothing more can be typed: a helper that could not start, too many failures.
    failed: bool,
    events: Vec<PolkitEvent>,
}

impl PolkitUi {
    /// A dialog for `message` (polkitd's own words, never a script's), asked on behalf of `requester` when it is not
    /// empty, for the action `action_id`. `identities` are the names the user may authenticate as, and `selected`
    /// is the one the helper is started for.
    #[must_use]
    pub fn new(
        message: impl Into<String>,
        requester: impl Into<String>,
        action_id: impl Into<String>,
        identities: Vec<String>,
        selected: usize,
    ) -> Self {
        let selected = selected.min(identities.len().saturating_sub(1));
        Self {
            message: message.into(),
            requester: requester.into(),
            action_id: action_id.into(),
            identities,
            selected,
            notes: Vec::new(),
            prompt: None,
            entry: SecretText::default(),
            characters: 0,
            failed: false,
            events: Vec::new(),
        }
    }

    /// The identity the helper runs for, as an index.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// How many characters are typed.
    #[must_use]
    pub fn typed(&self) -> usize {
        self.characters
    }

    /// Whether PAM is waiting for an answer.
    #[must_use]
    pub fn is_asking(&self) -> bool {
        self.prompt.is_some() && !self.failed
    }

    /// Whether the dialog can take no more input but Close.
    #[must_use]
    pub fn has_failed(&self) -> bool {
        self.failed
    }

    /// PAM asks for an answer. `echo` says the answer may be shown as typed. Whatever was typed before is wiped: it
    /// answered an earlier prompt.
    pub fn ask(&mut self, text: impl Into<String>, echo: bool) {
        if self.failed {
            return;
        }
        self.clear_entry();
        self.prompt = Some(Prompt {
            text: text.into(),
            echo,
        });
    }

    /// PAM says something, such as a fingerprint reader waiting.
    pub fn info(&mut self, text: impl Into<String>) {
        self.note(Note::Info(text.into()));
    }

    /// PAM reports an error, or an attempt failed and a new one starts.
    pub fn error(&mut self, text: impl Into<String>) {
        self.note(Note::Error(text.into()));
    }

    /// The helper is working: nothing can be typed until it asks again.
    pub fn wait(&mut self) {
        self.clear_entry();
        self.prompt = None;
    }

    /// Ends the attempt for good: `reason` is shown and only Close is left. The request fails closed when the user
    /// closes it, or at its deadline.
    pub fn fail(&mut self, reason: impl Into<String>) {
        self.wait();
        self.notes.clear();
        self.notes.push(Note::Error(reason.into()));
        self.failed = true;
    }

    /// Appends one typed character. Control characters and anything past the cap are ignored, and nothing is taken
    /// unless PAM is asking.
    pub fn type_char(&mut self, character: char) {
        if !self.is_asking() || character.is_control() || self.characters >= MAX_CHARACTERS {
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
        if !self.is_asking() || self.characters == 0 {
            return;
        }
        let kept = self.entry.expose();
        let end = kept.char_indices().last().map_or(0, |(index, _)| index);
        let mut next = String::with_capacity(end);
        next.push_str(&kept[..end]);
        self.replace(next);
        self.characters -= 1;
    }

    /// Answers the prompt. An empty entry answers nothing, so a slipped Enter does not spend an attempt.
    pub fn submit(&mut self) {
        if !self.is_asking() || self.characters == 0 {
            return;
        }
        let answer = std::mem::take(&mut self.entry);
        self.characters = 0;
        self.prompt = None;
        self.events.push(PolkitEvent::Reply(answer));
    }

    /// Chooses another identity to authenticate as. Nothing happens for the one already chosen or an index that is
    /// not in the list.
    pub fn select(&mut self, index: usize) {
        if self.failed || index >= self.identities.len() || index == self.selected {
            return;
        }
        self.selected = index;
        self.wait();
        self.notes.clear();
        self.events.push(PolkitEvent::Identity(index));
    }

    /// Cancels the request and wipes what was typed.
    pub fn cancel(&mut self) {
        self.wait();
        if !self.events.contains(&PolkitEvent::Cancel) {
            self.events.push(PolkitEvent::Cancel);
        }
    }

    /// What the user did since the last call.
    pub fn take_events(&mut self) -> Vec<PolkitEvent> {
        std::mem::take(&mut self.events)
    }

    fn note(&mut self, note: Note) {
        if self.failed {
            return;
        }
        self.notes.push(note);
        if self.notes.len() > MAX_NOTES {
            self.notes.remove(0);
        }
    }

    fn clear_entry(&mut self) {
        self.entry.wipe();
        self.characters = 0;
    }

    /// Swaps in `next` as the entry and wipes the value it replaces.
    fn replace(&mut self, next: String) {
        let mut old = std::mem::replace(&mut self.entry, SecretText::new(next));
        old.wipe();
    }
}

impl fmt::Debug for PolkitUi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PolkitUi")
            .field("action_id", &self.action_id)
            .field("requester", &self.requester)
            .field("selected", &self.selected)
            .field("typed", &self.characters)
            .field("asking", &self.is_asking())
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl DialogUi for PolkitUi {
    fn show(&mut self, ui: &mut Ui, _now: Tick) -> Option<Settled> {
        ui.horizontal(|ui| {
            lock_icon(ui);
            ui.heading("Authentication required");
        });
        ui.label(&self.message);
        if !self.requester.is_empty() {
            ui.label(format!("Requested by {}", self.requester));
        }
        ui.small(&self.action_id);
        if self.identities.len() > 1 && !self.failed {
            ui.label("Authenticate as:");
            ui.horizontal_wrapped(|ui| {
                for (index, name) in self.identities.clone().into_iter().enumerate() {
                    if ui.selectable_label(index == self.selected, name).clicked() {
                        self.select(index);
                    }
                }
            });
        } else if let Some(name) = self.identities.get(self.selected).filter(|_| !self.failed) {
            ui.label(format!("Authenticating as {name}"));
        }
        for note in &self.notes {
            match note {
                Note::Info(text) => ui.label(text),
                Note::Error(text) => ui.label(RichText::new(text).color(ui.visuals().error_fg_color)),
            };
        }
        if let Some(prompt) = self.prompt.as_ref().filter(|_| !self.failed) {
            ui.label(prompt.text.trim_end());
            let shown: String = if prompt.echo {
                self.entry.expose().to_owned()
            } else {
                std::iter::repeat_n(BULLET, self.characters).collect()
            };
            Frame::new()
                .stroke(ui.visuals().widgets.inactive.bg_stroke)
                .corner_radius(4.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.set_min_width(300.0);
                    ui.label(RichText::new(format!("{shown}|")).monospace());
                });
        } else if !self.failed {
            ui.label("Waiting for the authentication service...");
        }
        let deny = if self.failed { "Close" } else { "Cancel" };
        let buttons = decision_buttons(ui, deny, "Authenticate", self.is_asking() && self.characters > 0);
        if buttons.deny.clicked() {
            self.cancel();
        }
        if buttons.accept.clicked() {
            self.submit();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_portal_core::model::ScreenKey;
    use gnoblin_portal_ui::style::DialogStyle;
    use gnoblin_portal_ui::ui::{DialogScheme, DialogShell, Setup, key_events};

    fn setup() -> Setup {
        Setup {
            width: 520,
            height: 420,
            reduced_motion: true,
            scheme: DialogScheme::Dark,
            style: DialogStyle::default(),
        }
    }

    fn dialog() -> PolkitUi {
        PolkitUi::new(
            "Authentication is needed to run `/usr/bin/true' as the super user",
            "pkexec",
            "org.freedesktop.policykit.exec",
            vec!["alice".to_owned(), "root".to_owned()],
            0,
        )
    }

    fn typed(ui: &mut PolkitUi, text: &str) {
        for character in text.chars() {
            ui.type_char(character);
        }
    }

    #[test]
    fn nothing_is_typed_until_pam_asks() {
        let mut ui = dialog();
        typed(&mut ui, "early");
        assert_eq!(ui.typed(), 0);
        ui.submit();
        assert!(ui.take_events().is_empty());
        ui.ask("Password: ", false);
        typed(&mut ui, "ok");
        assert_eq!(ui.typed(), 2);
    }

    #[test]
    fn the_answer_is_an_event_and_the_dialog_waits_for_the_helper_again() {
        let mut ui = dialog();
        ui.ask("Password: ", false);
        typed(&mut ui, "correct horse");
        ui.submit();
        assert!(!ui.is_asking(), "the next prompt is the helper's to send");
        match ui.take_events().as_slice() {
            [PolkitEvent::Reply(secret)] => assert_eq!(secret.expose(), "correct horse"),
            other => panic!("expected the answer, got {other:?}"),
        }
        assert!(ui.take_events().is_empty(), "an event is taken once");
    }

    #[test]
    fn an_empty_entry_answers_nothing() {
        let mut ui = dialog();
        ui.ask("Password: ", false);
        ui.submit();
        typed(&mut ui, "a");
        ui.backspace();
        ui.submit();
        assert!(ui.take_events().is_empty());
        assert!(ui.is_asking());
    }

    #[test]
    fn a_new_prompt_wipes_what_answered_the_last_one() {
        let mut ui = dialog();
        ui.ask("Password: ", false);
        typed(&mut ui, "stale");
        ui.ask("Code: ", true);
        assert_eq!(ui.typed(), 0);
    }

    #[test]
    fn choosing_another_identity_is_an_event_and_forgets_the_attempt() {
        let mut ui = dialog();
        ui.ask("Password: ", false);
        typed(&mut ui, "half");
        ui.error("Authentication failure");
        ui.select(1);
        assert_eq!(ui.selected(), 1);
        assert_eq!(ui.typed(), 0);
        assert!(!ui.is_asking());
        assert_eq!(ui.take_events(), [PolkitEvent::Identity(1)]);
        ui.select(1);
        ui.select(9);
        assert!(
            ui.take_events().is_empty(),
            "the same identity and a missing one choose nothing"
        );
    }

    #[test]
    fn cancel_is_one_event_and_wipes_the_entry() {
        let mut ui = dialog();
        ui.ask("Password: ", false);
        typed(&mut ui, "hunter2");
        ui.cancel();
        ui.cancel();
        assert_eq!(ui.typed(), 0);
        assert_eq!(ui.take_events(), [PolkitEvent::Cancel]);
    }

    #[test]
    fn a_failed_dialog_takes_no_more_input_and_only_closes() {
        let mut ui = dialog();
        ui.fail("polkit-agent-helper-1 was not found");
        ui.ask("Password: ", false);
        typed(&mut ui, "x");
        ui.select(1);
        assert!(!ui.is_asking());
        assert_eq!(ui.typed(), 0);
        assert!(ui.take_events().is_empty());
        ui.cancel();
        assert_eq!(ui.take_events(), [PolkitEvent::Cancel]);
    }

    #[test]
    fn the_notes_are_bounded() {
        let mut ui = dialog();
        for number in 0..10 {
            ui.info(format!("note {number}"));
        }
        assert_eq!(ui.notes.len(), MAX_NOTES);
        assert_eq!(ui.notes.first(), Some(&Note::Info("note 6".to_owned())));
    }

    #[test]
    fn debug_never_shows_the_password() {
        let mut ui = dialog();
        ui.ask("Password: ", false);
        typed(&mut ui, "swordfish");
        let shown = format!("{ui:?}");
        assert!(!shown.contains("swordfish"), "{shown}");
        assert!(shown.contains("typed: 9"), "{shown}");
        ui.submit();
        let shown = format!("{:?}", ui.take_events());
        assert!(!shown.contains("swordfish"), "{shown}");
    }

    #[test]
    fn the_frame_never_draws_a_masked_answer_and_draws_pams_words() {
        let mut shell = DialogShell::new(setup());
        let mut ui = dialog();
        ui.info("Place your finger on the reader");
        ui.ask("Password: ", false);
        typed(&mut ui, "swordfish");
        let shown = shell.shown(&mut ui, Tick::new(0));
        assert!(
            shown.texts.iter().all(|text| !text.contains("swordfish")),
            "{:?}",
            shown.texts
        );
        for wanted in [
            "Authentication required",
            "Requested by pkexec",
            "org.freedesktop.policykit.exec",
            "Place your finger on the reader",
            "Password:",
            "Cancel",
            "Authenticate",
        ] {
            assert!(
                shown.texts.iter().any(|text| text == wanted),
                "{wanted:?} is missing: {:?}",
                shown.texts
            );
        }
        assert!(shown.texts.iter().any(|text| text.starts_with(BULLET)));
    }

    #[test]
    fn an_echoed_answer_is_shown_as_typed() {
        let mut shell = DialogShell::new(setup());
        let mut ui = dialog();
        ui.ask("Code: ", true);
        typed(&mut ui, "123456");
        let shown = shell.shown(&mut ui, Tick::new(0));
        assert!(
            shown.texts.iter().any(|text| text.contains("123456")),
            "{:?}",
            shown.texts
        );
    }

    #[test]
    fn a_dialog_with_several_identities_offers_each_and_one_with_a_single_names_it() {
        let mut shell = DialogShell::new(setup());
        let mut many = dialog();
        let shown = shell.shown(&mut many, Tick::new(0));
        for wanted in ["Authenticate as:", "alice", "root"] {
            assert!(
                shown.texts.iter().any(|text| text == wanted),
                "{wanted:?}: {:?}",
                shown.texts
            );
        }
        let mut one = PolkitUi::new("Message", "", "action", vec!["alice".to_owned()], 0);
        let shown = shell.shown(&mut one, Tick::new(0));
        assert!(
            shown.texts.iter().any(|text| text == "Authenticating as alice"),
            "{:?}",
            shown.texts
        );
        assert!(shown.texts.iter().all(|text| text != "Authenticate as:"));
    }

    #[test]
    fn authenticate_is_reached_by_keyboard() {
        let mut shell = DialogShell::new(setup());
        let mut ui = PolkitUi::new("Message", "", "action", vec!["alice".to_owned()], 0);
        ui.ask("Password: ", false);
        typed(&mut ui, "pw");
        shell.layout(&mut ui, Tick::new(0));
        // Tab walks Cancel then Authenticate, and Enter presses the focused button.
        for key in [ScreenKey::Tab, ScreenKey::Tab, ScreenKey::Enter] {
            shell.run(&mut ui, key_events(key, false), Tick::new(0));
        }
        assert!(matches!(ui.take_events().as_slice(), [PolkitEvent::Reply(_)]));
    }

    #[test]
    fn a_failed_dialog_says_why_and_offers_close() {
        let mut shell = DialogShell::new(setup());
        let mut ui = dialog();
        ui.fail("polkit-agent-helper-1 was not found");
        let shown = shell.shown(&mut ui, Tick::new(0));
        for wanted in ["polkit-agent-helper-1 was not found", "Close"] {
            assert!(
                shown.texts.iter().any(|text| text == wanted),
                "{wanted:?}: {:?}",
                shown.texts
            );
        }
    }
}
