//! The consent dialog: the prompt, the requester, and the Deny and Allow buttons. Beside it, the
//! passphrase dialog ([`SecretUi`]), which is the same kind of Rust-drawn card for `prompt.secret`, the polkit
//! authentication dialog ([`PolkitUi`]) for `auth.polkit`, and the lock card ([`LockUi`]), which the compositor draws
//! over its lock cover when no shell holds the lock screen.
//!
//! The model ([`Consent`]) decides. Allow stays disabled until the arm delay has
//! passed, and Deny takes initial focus, so Enter denies unless the user moves.

mod lock;
mod polkit;
mod secret;

pub use lock::{LockNotice, LockUi};
pub use polkit::{PolkitEvent, PolkitUi};
pub use secret::{SecretResult, SecretUi};

use egui::{Key, Ui};
use gnoblin_portal_core::model::{Consent, ConsentDecision, ConsentInput, ConsentTarget, Settled};
use gnoblin_portal_ui::style::decision_buttons;
use gnoblin_portal_ui::ui::DialogUi;
use gnoblin_types::Tick;

/// A consent request on screen. Owns the model, which owns the decision.
pub struct ConsentUi {
    consent: Consent,
    focused: bool,
}

impl ConsentUi {
    /// A view of `consent` for one frame sequence.
    #[must_use]
    pub const fn new(consent: Consent) -> Self {
        Self {
            consent,
            focused: false,
        }
    }

    /// Feeds a click on `target` to the model as a press and a release.
    fn click(&mut self, target: ConsentTarget, now: Tick) -> Option<ConsentDecision> {
        self.consent.input(ConsentInput::PointerPress { target }, now);
        self.consent.input(ConsentInput::PointerRelease { target }, now)
    }
}

impl DialogUi for ConsentUi {
    fn show(&mut self, ui: &mut Ui, now: Tick) -> Option<Settled> {
        let armed = self.consent.armed(now);
        let mut decision = None;
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            decision = self.consent.input(ConsentInput::Escape, now);
        }

        ui.heading(self.consent.prompt());
        ui.label(format!("Requested by {}", self.consent.app_id()));
        // Deny is first in every layout and holds the initial focus, so Enter denies unless the user moves. The
        // arm delay is the model's: no style reaches it.
        let buttons = decision_buttons(ui, "Deny", "Allow", armed);
        if !self.focused {
            buttons.deny.request_focus();
            self.focused = true;
        }
        if decision.is_none() && buttons.deny.clicked() {
            decision = self.click(ConsentTarget::Deny, now);
        }
        if decision.is_none() && buttons.accept.clicked() {
            decision = self.click(ConsentTarget::Allow, now);
        }
        decision.map(|decision| Consent::grant(decision).map_or(Settled::Denied, Settled::Granted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_portal_core::model::{ARM_DELAY_MS, ScreenKey};
    use gnoblin_portal_ui::style::{ButtonLayout, DialogStyle, StyleRequest};
    use gnoblin_portal_ui::ui::{DialogScheme, DialogShell, Setup, key_events};

    fn setup_with(style: DialogStyle) -> Setup {
        Setup {
            width: 480,
            height: 240,
            reduced_motion: true,
            scheme: DialogScheme::Dark,
            style,
        }
    }

    /// The most a caller can ask for: every number past its limit, an accent that is the dialog's own colour, a
    /// layout that moves the buttons and the largest font and scale. After clamping it is the strangest style a dialog
    /// is ever drawn with.
    fn extreme() -> DialogStyle {
        StyleRequest {
            radius: Some(i64::MAX),
            accent: Some([27, 27, 27]),
            font_family: Some("monospace".to_owned()),
            font_size: Some(i64::MIN),
            scale: Some(1.0e9),
            button_layout: Some(ButtonLayout::Stacked),
        }
        .clamp()
    }

    /// Opens a consent at `opened`, lays it out and feeds `keys` at `at`. Returns the
    /// first settlement.
    fn settle_with(style: DialogStyle, keys: Vec<ScreenKey>, opened: u64, at: u64) -> Option<Settled> {
        let mut shell = DialogShell::new(setup_with(style));
        let mut ui = ConsentUi::new(Consent::open(
            "org.example.App",
            "Share your screen?",
            Tick::new(opened),
        ));
        shell.layout(&mut ui, Tick::new(at));
        let mut settled = None;
        for key in keys {
            settled = settled.or(shell.run(&mut ui, key_events(key, false), Tick::new(at)));
        }
        settled
    }

    fn settle(keys: Vec<ScreenKey>, opened: u64, at: u64) -> Option<Settled> {
        settle_with(DialogStyle::default(), keys, opened, at)
    }

    #[test]
    fn tab_then_enter_allows_once_armed() {
        let settled = settle(vec![ScreenKey::Tab, ScreenKey::Enter], 0, 1_000);
        assert!(
            matches!(settled, Some(Settled::Granted(_))),
            "Tab moves focus to Allow and Enter presses it once armed: {settled:?}"
        );
    }

    #[test]
    fn enter_on_initial_focus_denies() {
        assert_eq!(
            settle(vec![ScreenKey::Enter], 0, 1_000),
            Some(Settled::Denied),
            "Deny holds initial focus"
        );
    }

    #[test]
    fn tab_then_enter_before_arm_does_not_allow() {
        assert_eq!(
            settle(vec![ScreenKey::Tab, ScreenKey::Enter], 0, 100),
            Some(Settled::Denied),
            "Allow is disabled until the arm delay has passed"
        );
    }

    #[test]
    fn tab_alone_after_arm_does_not_settle() {
        assert_eq!(
            settle(vec![ScreenKey::Tab], 0, 1_000),
            None,
            "Tab only moves focus; it must not answer the consent"
        );
    }

    #[test]
    fn the_arm_delay_survives_any_style() {
        for style in [DialogStyle::default(), extreme()] {
            assert_eq!(
                settle_with(style, vec![ScreenKey::Tab, ScreenKey::Enter], 0, 100),
                Some(Settled::Denied),
                "Allow stays disabled until the arm delay passed, whatever the style"
            );
            assert!(
                matches!(
                    settle_with(style, vec![ScreenKey::Tab, ScreenKey::Enter], 0, ARM_DELAY_MS + 1),
                    Some(Settled::Granted(_))
                ),
                "and opens once it has"
            );
        }
    }

    #[test]
    fn deny_keeps_the_initial_focus_in_every_layout() {
        for layout in [
            ButtonLayout::Start,
            ButtonLayout::End,
            ButtonLayout::Stretch,
            ButtonLayout::Stacked,
        ] {
            let style = StyleRequest {
                button_layout: Some(layout),
                ..StyleRequest::default()
            }
            .clamp();
            assert_eq!(
                settle_with(style, vec![ScreenKey::Enter], 0, 1_000),
                Some(Settled::Denied),
                "{layout:?}: Enter on the initial focus denies"
            );
        }
    }

    #[test]
    fn the_requester_and_the_question_are_always_drawn() {
        for style in [DialogStyle::default(), extreme()] {
            let mut shell = DialogShell::new(setup_with(style));
            let mut ui = ConsentUi::new(Consent::open("org.example.App", "Share your screen?", Tick::new(0)));
            let shown = shell.shown(&mut ui, Tick::new(1_000));
            for wanted in ["Requested by org.example.App", "Share your screen?", "Deny", "Allow"] {
                assert!(
                    shown.texts.iter().any(|text| text == wanted),
                    "{wanted:?} is missing under {style:?}: {:?}",
                    shown.texts
                );
            }
        }
    }
}
