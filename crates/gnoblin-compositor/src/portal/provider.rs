//! The seam between the portal's negotiation and whatever draws its dialogs, from the compositor's side.
//!
//! Core queues and validates portal requests and settles every one of them with exactly one verdict
//! ([`super::validate_reply`]). It draws nothing itself: the screen the user answers is a [`UiProvider`]'s, a trait
//! `gnoblin-portal-ui` defines beside the frame and the style it speaks in. The stock provider is std's
//! (`gnoblin-std-impl`), registered through the C ABI; with none, [`NoUi`] is what the session runs, and it refuses every
//! request `no-ui`. A request the user cannot be asked about is therefore never granted: the consent flow fails safe.

use gnoblin_portal_core::model::{Dialog, Interface, ScreenKey};
use gnoblin_portal_core::{PortalError, Verdict};
use gnoblin_portal_ui::paint::Frame;
use gnoblin_portal_ui::provider::{SubmitOutcome, UiProvider};
use gnoblin_portal_ui::style::DialogStyle;
use gnoblin_portal_ui::ui::DialogScheme;
use gnoblin_types::{RequestId, Tick};

/// The provider of a core with none: there is nothing to ask the user with, so every request is refused `no-ui`.
///
/// It admits nothing, so it never holds a request, never draws a frame and never settles one: the refusal is the
/// reply to the submit. A consent request that cannot be shown to the user is not granted.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoUi;

impl UiProvider for NoUi {
    fn size(&self) -> (usize, usize) {
        (0, 0)
    }

    fn set_scheme(&mut self, _scheme: DialogScheme) {}

    fn set_style(&mut self, _style: DialogStyle) {}

    fn set_bounds(&mut self, _width: usize, _height: usize) {}

    fn active_request(&self) -> Option<RequestId> {
        None
    }

    fn waiting(&self) -> usize {
        0
    }

    fn submit(
        &mut self,
        _request: RequestId,
        _app_id: &str,
        _interface: Interface,
        _dialog: Dialog,
        _timeout_ms: u64,
        _now: Tick,
    ) -> Result<SubmitOutcome, PortalError> {
        Err(PortalError::NoUi)
    }

    fn key(&mut self, _key: ScreenKey, _repeat: bool) {}

    fn pointer_press(&mut self, _x: f32, _y: f32) {}

    fn pointer_release(&mut self, _x: f32, _y: f32) {}

    fn tick(&mut self, _now: Tick) {}

    fn take_frame(&mut self) -> Option<Frame> {
        None
    }

    fn redraw(&mut self) {}

    fn take_outcomes(&mut self) -> Vec<(RequestId, Verdict)> {
        Vec::new()
    }

    fn cancel(&mut self, _request: RequestId) -> bool {
        false
    }

    fn set_locked(&mut self, _locked: bool) {}
}

#[cfg(test)]
mod tests {
    use gnoblin_portal_core::model::Consent;

    use super::*;

    fn consent() -> Dialog {
        Dialog::Consent(Consent::open("org.example.App", "Share your screen?", Tick::ZERO))
    }

    #[test]
    fn with_no_ui_every_interface_is_refused_no_ui() {
        let mut ui = NoUi;
        for interface in [
            Interface::ScreenCast,
            Interface::Screenshot,
            Interface::RemoteDesktop,
            Interface::Secret,
        ] {
            let refused = ui.submit(
                RequestId::new(1),
                "org.example.App",
                interface,
                consent(),
                60_000,
                Tick::ZERO,
            );
            assert_eq!(refused, Err(PortalError::NoUi), "{interface:?}");
        }
    }

    #[test]
    fn with_no_ui_nothing_is_held_drawn_or_settled() {
        let mut ui = NoUi;
        let _ = ui.submit(
            RequestId::new(1),
            "org.example.App",
            Interface::Screenshot,
            consent(),
            60_000,
            Tick::ZERO,
        );
        ui.tick(Tick::new(1));
        assert_eq!(ui.active_request(), None);
        assert_eq!(ui.waiting(), 0);
        assert!(ui.take_frame().is_none());
        assert!(ui.take_outcomes().is_empty());
        assert!(
            !ui.cancel(RequestId::new(1)),
            "a request it refused is not one it holds"
        );
    }
}
