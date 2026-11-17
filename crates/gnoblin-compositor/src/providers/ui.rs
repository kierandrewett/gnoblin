//! [`SlotUi`]: the portal service's UI, which is whatever provider is registered, or none.
//!
//! The portal service drives one [`UiProvider`] on the compositor's IPC thread. The provider is registered from another
//! thread, and ends when its registrant does, so the service holds this handle instead: every call goes to the provider that is
//! registered now, under a lock the registering thread takes only to swap it. With none registered every request is refused
//! `no-ui` and nothing is held, drawn or settled, which is [`NoUi`](crate::portal::NoUi)'s behaviour. A provider that ends leaves
//! no request waiting: each one it held settles `error(no-ui)`.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_portal_core::model::{Dialog, Interface, ScreenKey};
use gnoblin_portal_core::{PortalError, Verdict};
use gnoblin_portal_ui::paint::Frame;
use gnoblin_portal_ui::provider::{SubmitOutcome, UiProvider};
use gnoblin_portal_ui::style::DialogStyle;
use gnoblin_portal_ui::ui::DialogScheme;
use gnoblin_types::{RequestId, Tick};

use super::slots::ProviderId;

/// What the slot keeps across providers: what the session last told the UI, to give the next one.
#[derive(Default)]
struct UiState {
    provider: Option<(ProviderId, Box<dyn UiProvider + Send>)>,
    scheme: DialogScheme,
    style: DialogStyle,
    bounds: Option<(usize, usize)>,
    locked: bool,
    /// The requests the provider admitted and has not settled.
    admitted: BTreeSet<RequestId>,
    /// Verdicts of requests a provider that ended left, for the next `take_outcomes`.
    orphaned: Vec<(RequestId, Verdict)>,
}

/// The shared half of the slot.
#[derive(Default)]
pub(super) struct UiShared(Mutex<UiState>);

impl UiShared {
    fn lock(&self) -> MutexGuard<'_, UiState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes `ui` the provider, after telling it what the session last told its predecessor.
    pub(super) fn install(&self, id: ProviderId, mut ui: Box<dyn UiProvider + Send>) {
        let mut state = self.lock();
        ui.set_scheme(state.scheme);
        ui.set_style(state.style);
        if let Some((width, height)) = state.bounds {
            ui.set_bounds(width, height);
        }
        if state.locked {
            ui.set_locked(true);
        }
        state.provider = Some((id, ui));
    }

    /// Ends the provider `id`: what it held settles `error(no-ui)`, and it is dropped.
    pub(super) fn remove(&self, id: ProviderId) {
        let mut state = self.lock();
        if state.provider.as_ref().is_none_or(|(held, _)| *held != id) {
            return;
        }
        let Some((_, mut ui)) = state.provider.take() else {
            return;
        };
        let mut settled = ui.close();
        // A provider that holds nothing past its drop has settled nothing; what it admitted still needs an answer.
        let admitted = std::mem::take(&mut state.admitted);
        for request in admitted {
            if !settled.iter().any(|(done, _)| *done == request) {
                settled.push((request, Verdict::Error(PortalError::NoUi)));
            }
        }
        state.orphaned.extend(settled);
        drop(state);
        // The provider drops here, outside the lock: for one registered through the C ABI that runs the registrant's destructor.
        drop(ui);
    }
}

/// The portal service's handle on the UI slot. Cloning shares the slot.
#[derive(Clone)]
pub struct SlotUi(Arc<UiShared>);

impl SlotUi {
    pub(super) fn new(shared: Arc<UiShared>) -> Self {
        Self(shared)
    }
}

impl UiProvider for SlotUi {
    fn size(&self) -> (usize, usize) {
        let state = self.0.lock();
        state
            .provider
            .as_ref()
            .map_or_else(|| state.bounds.unwrap_or((0, 0)), |(_, ui)| ui.size())
    }

    fn set_scheme(&mut self, scheme: DialogScheme) {
        let mut state = self.0.lock();
        state.scheme = scheme;
        if let Some((_, ui)) = state.provider.as_mut() {
            ui.set_scheme(scheme);
        }
    }

    fn set_style(&mut self, style: DialogStyle) {
        let mut state = self.0.lock();
        state.style = style;
        if let Some((_, ui)) = state.provider.as_mut() {
            ui.set_style(style);
        }
    }

    fn set_bounds(&mut self, width: usize, height: usize) {
        let mut state = self.0.lock();
        state.bounds = Some((width, height));
        if let Some((_, ui)) = state.provider.as_mut() {
            ui.set_bounds(width, height);
        }
    }

    fn active_request(&self) -> Option<RequestId> {
        self.0.lock().provider.as_ref().and_then(|(_, ui)| ui.active_request())
    }

    fn waiting(&self) -> usize {
        self.0.lock().provider.as_ref().map_or(0, |(_, ui)| ui.waiting())
    }

    fn submit(
        &mut self,
        request: RequestId,
        app_id: &str,
        interface: Interface,
        dialog: Dialog,
        timeout_ms: u64,
        now: Tick,
    ) -> Result<SubmitOutcome, PortalError> {
        let mut state = self.0.lock();
        let Some((_, ui)) = state.provider.as_mut() else {
            return Err(PortalError::NoUi);
        };
        let outcome = ui.submit(request, app_id, interface, dialog, timeout_ms, now)?;
        state.admitted.insert(request);
        Ok(outcome)
    }

    fn key(&mut self, key: ScreenKey, repeat: bool) {
        if let Some((_, ui)) = self.0.lock().provider.as_mut() {
            ui.key(key, repeat);
        }
    }

    fn pointer_press(&mut self, x: f32, y: f32) {
        if let Some((_, ui)) = self.0.lock().provider.as_mut() {
            ui.pointer_press(x, y);
        }
    }

    fn pointer_release(&mut self, x: f32, y: f32) {
        if let Some((_, ui)) = self.0.lock().provider.as_mut() {
            ui.pointer_release(x, y);
        }
    }

    fn tick(&mut self, now: Tick) {
        if let Some((_, ui)) = self.0.lock().provider.as_mut() {
            ui.tick(now);
        }
    }

    fn take_frame(&mut self) -> Option<Frame> {
        self.0.lock().provider.as_mut().and_then(|(_, ui)| ui.take_frame())
    }

    fn redraw(&mut self) {
        if let Some((_, ui)) = self.0.lock().provider.as_mut() {
            ui.redraw();
        }
    }

    fn take_outcomes(&mut self) -> Vec<(RequestId, Verdict)> {
        let mut state = self.0.lock();
        let mut settled = std::mem::take(&mut state.orphaned);
        if let Some((_, ui)) = state.provider.as_mut() {
            settled.extend(ui.take_outcomes());
        }
        for (request, _) in &settled {
            state.admitted.remove(request);
        }
        settled
    }

    fn cancel(&mut self, request: RequestId) -> bool {
        self.0
            .lock()
            .provider
            .as_mut()
            .is_some_and(|(_, ui)| ui.cancel(request))
    }

    fn set_locked(&mut self, locked: bool) {
        let mut state = self.0.lock();
        state.locked = locked;
        if let Some((_, ui)) = state.provider.as_mut() {
            ui.set_locked(locked);
        }
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_portal_core::model::Consent;

    use super::super::ProviderSlots;
    use super::*;

    /// A UI that holds the one request it is given until it is told its verdict.
    #[derive(Default)]
    struct Holder {
        held: Option<RequestId>,
        scheme: Option<DialogScheme>,
        locked: bool,
    }

    impl UiProvider for Holder {
        fn size(&self) -> (usize, usize) {
            (10, 10)
        }
        fn set_scheme(&mut self, scheme: DialogScheme) {
            self.scheme = Some(scheme);
        }
        fn set_style(&mut self, _style: DialogStyle) {}
        fn set_bounds(&mut self, _width: usize, _height: usize) {}
        fn active_request(&self) -> Option<RequestId> {
            self.held
        }
        fn waiting(&self) -> usize {
            0
        }
        fn submit(
            &mut self,
            request: RequestId,
            _app_id: &str,
            _interface: Interface,
            _dialog: Dialog,
            _timeout_ms: u64,
            _now: Tick,
        ) -> Result<SubmitOutcome, PortalError> {
            if self.locked {
                return Err(PortalError::Locked);
            }
            self.held = Some(request);
            Ok(SubmitOutcome::Started)
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
        fn set_locked(&mut self, locked: bool) {
            self.locked = locked;
        }
    }

    fn consent() -> Dialog {
        Dialog::Consent(Consent::open("org.example.App", "Share?", Tick::ZERO))
    }

    fn submit(ui: &mut SlotUi, request: u64) -> Result<SubmitOutcome, PortalError> {
        ui.submit(
            RequestId::new(request),
            "org.example.App",
            Interface::Screenshot,
            consent(),
            1000,
            Tick::ZERO,
        )
    }

    #[test]
    fn with_no_provider_every_request_is_refused_no_ui_and_nothing_is_held() {
        let slots = ProviderSlots::new();
        let mut ui = slots.ui_handle();
        assert_eq!(submit(&mut ui, 1), Err(PortalError::NoUi));
        assert_eq!(ui.active_request(), None);
        assert_eq!(ui.waiting(), 0);
        assert!(ui.take_frame().is_none());
        assert!(ui.take_outcomes().is_empty());
        assert!(!ui.cancel(RequestId::new(1)));
    }

    #[test]
    fn a_provider_registered_later_is_given_what_the_session_already_set() {
        let slots = ProviderSlots::new();
        let mut ui = slots.ui_handle();
        ui.set_scheme(DialogScheme::Light);
        ui.set_locked(true);
        slots.set_ui(|| Box::<Holder>::default()).expect("free");
        assert_eq!(
            submit(&mut ui, 1),
            Err(PortalError::Locked),
            "the lock reached the new provider"
        );
        ui.set_locked(false);
        assert_eq!(submit(&mut ui, 2), Ok(SubmitOutcome::Started));
    }

    #[test]
    fn a_provider_that_ends_settles_what_it_held_no_ui_and_the_next_request_is_refused() {
        let slots = ProviderSlots::new();
        let mut ui = slots.ui_handle();
        let id = slots.set_ui(|| Box::<Holder>::default()).expect("free");
        assert_eq!(submit(&mut ui, 7), Ok(SubmitOutcome::Started));
        assert!(slots.remove(id));
        assert_eq!(
            ui.take_outcomes(),
            vec![(RequestId::new(7), Verdict::Error(PortalError::NoUi))]
        );
        assert!(ui.take_outcomes().is_empty(), "a verdict is taken once");
        assert_eq!(submit(&mut ui, 8), Err(PortalError::NoUi));
    }
}
