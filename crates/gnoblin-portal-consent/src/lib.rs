//! The consent dialog: the prompt, the requester, and the Deny and Allow buttons.
//!
//! The model ([`Consent`]) decides. Allow stays disabled until the arm delay has
//! passed, and Deny takes initial focus, so Enter denies unless the user moves.

use egui::{Button, Key, Ui};
use gnoblin_portal_core::model::{Consent, ConsentDecision, ConsentInput, ConsentTarget, Settled};
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
        ui.horizontal(|ui| {
            let deny = ui.button("Deny");
            if !self.focused {
                deny.request_focus();
                self.focused = true;
            }
            let allow = ui.add_enabled(armed, Button::new("Allow"));
            if decision.is_none() && deny.clicked() {
                decision = self.click(ConsentTarget::Deny, now);
            }
            if decision.is_none() && allow.clicked() {
                decision = self.click(ConsentTarget::Allow, now);
            }
        });
        decision.map(|decision| Consent::grant(decision).map_or(Settled::Denied, Settled::Granted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_portal_core::model::ScreenKey;
    use gnoblin_portal_ui::ui::{DialogScheme, DialogShell, Setup, key_events};

    fn setup() -> Setup {
        Setup {
            width: 480,
            height: 240,
            reduced_motion: true,
            scheme: DialogScheme::Dark,
        }
    }

    /// Opens a consent at `opened`, lays it out and feeds `keys` at `at`. Returns the
    /// first settlement.
    fn settle(keys: Vec<ScreenKey>, opened: u64, at: u64) -> Option<Settled> {
        let mut shell = DialogShell::new(setup());
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
}
