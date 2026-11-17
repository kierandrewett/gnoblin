//! [`ProviderSlots`]: the three places a provider is registered, and how each reaches the session.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_portal_ui::provider::UiProvider;
use gnoblin_wayland::ControlHandle;
use gnoblin_wm_api::WindowManager;

use super::settings::{HeldSettings, SettingEffect, SettingError, SettingValue};
use super::ui::{SlotUi, UiShared};

/// The identity of one registration, for [`ProviderSlots::remove`]. Never reused in the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(u64);

/// A kind of provider has one live registration, and it is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

impl std::fmt::Display for Busy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a provider of this kind is already registered")
    }
}

impl std::error::Error for Busy {}

/// What sees the accepted configuration document: the provider that turns its sections into settings.
pub trait ConfigObserver: Send {
    /// The accepted document as UTF-8 JSON, when the observer is registered and on every accepted commit.
    fn apply(&mut self, document: &str);
}

/// The observer a registration holds, shared with the thread that commits a document.
type SharedObserver = Arc<Mutex<Box<dyn ConfigObserver>>>;

/// Where a registration is, until the session is there to take it.
struct SlotsInner {
    control: Option<ControlHandle>,
    next_id: u64,
    /// The live window-manager registration, and the provider while it waits for the session.
    wm: Option<ProviderId>,
    pending_wm: Option<Box<dyn WindowManager + Send>>,
    ui: Option<ProviderId>,
    config: Option<(ProviderId, SharedObserver)>,
    /// The last accepted document, JSON, handed to an observer that registers after it.
    document: Option<String>,
    settings: HeldSettings,
}

/// The compositor's side of the providers. Cloning shares the slots.
///
/// See the [module](super) documentation. Every method may be called from any thread.
#[derive(Clone)]
pub struct ProviderSlots {
    inner: Arc<Mutex<SlotsInner>>,
    ui: Arc<UiShared>,
}

impl Default for ProviderSlots {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProviderSlots {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        formatter
            .debug_struct("ProviderSlots")
            .field("attached", &inner.control.is_some())
            .field("window_manager", &inner.wm.is_some())
            .field("ui", &inner.ui.is_some())
            .field("config", &inner.config.is_some())
            .finish_non_exhaustive()
    }
}

impl ProviderSlots {
    /// Slots with nothing registered, which is the core-only session: `NullWm`, no dialog UI, default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SlotsInner {
                control: None,
                next_id: 0,
                wm: None,
                pending_wm: None,
                ui: None,
                config: None,
                document: None,
                settings: HeldSettings::default(),
            })),
            ui: Arc::new(UiShared::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, SlotsInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next(inner: &mut SlotsInner) -> ProviderId {
        inner.next_id += 1;
        ProviderId(inner.next_id)
    }

    /// The UI the portal service draws and answers through: the registered provider, or none.
    #[must_use]
    pub fn ui_handle(&self) -> SlotUi {
        SlotUi::new(Arc::clone(&self.ui))
    }

    /// The session is up: a window manager registered before it is made the session's, and every setting set before it is
    /// applied. Called once by the composition root, on the thread that starts the session.
    pub fn attach(&self, control: ControlHandle) {
        let (pending, actions) = {
            let mut inner = self.lock();
            inner.control = Some(control.clone());
            (inner.pending_wm.take(), inner.settings.all())
        };
        if let Some(provider) = pending {
            control.set_window_manager(Some(provider));
        }
        for action in actions {
            action.deliver(&control);
        }
    }

    /// The session has ended: the window manager it held is gone with it, so a registration ending later asks nothing of a loop
    /// that no longer answers. Called by the composition root when the loop returns.
    pub fn detach(&self) {
        let mut inner = self.lock();
        inner.control = None;
        inner.wm = None;
        inner.pending_wm = None;
    }

    /// Registers the window manager `make` builds. `make` runs only when the slot is free, so a registration that is refused
    /// builds nothing and adopts nothing.
    ///
    /// # Errors
    /// [`Busy`] when one is registered.
    pub fn set_window_manager(&self, make: impl FnOnce() -> Box<dyn WindowManager + Send>) -> Result<ProviderId, Busy> {
        let (id, control, wm) = {
            let mut inner = self.lock();
            if inner.wm.is_some() {
                return Err(Busy);
            }
            let id = Self::next(&mut inner);
            inner.wm = Some(id);
            let wm = make();
            match inner.control.clone() {
                Some(control) => (id, control, wm),
                None => {
                    inner.pending_wm = Some(wm);
                    return Ok(id);
                }
            }
        };
        control.set_window_manager(Some(wm));
        Ok(id)
    }

    /// Registers the portal dialog UI `make` builds, which runs only when the slot is free. The scheme, the style, the
    /// bounds and the lock the session last set are given to it.
    ///
    /// # Errors
    /// [`Busy`] when one is registered.
    pub fn set_ui(&self, make: impl FnOnce() -> Box<dyn UiProvider + Send>) -> Result<ProviderId, Busy> {
        let id = {
            let mut inner = self.lock();
            if inner.ui.is_some() {
                return Err(Busy);
            }
            let id = Self::next(&mut inner);
            inner.ui = Some(id);
            id
        };
        self.ui.install(id, make());
        Ok(id)
    }

    /// Registers the configuration observer `make` builds, which runs only when the slot is free. The observer is told the
    /// last accepted document at once, on this thread.
    ///
    /// # Errors
    /// [`Busy`] when one is registered.
    pub fn set_config(&self, make: impl FnOnce() -> Box<dyn ConfigObserver>) -> Result<ProviderId, Busy> {
        let (id, observer, document) = {
            let mut inner = self.lock();
            if inner.config.is_some() {
                return Err(Busy);
            }
            let id = Self::next(&mut inner);
            let observer = Arc::new(Mutex::new(make()));
            inner.config = Some((id, Arc::clone(&observer)));
            (id, observer, inner.document.clone())
        };
        if let Some(document) = document {
            observer.lock().unwrap_or_else(PoisonError::into_inner).apply(&document);
        }
        Ok(id)
    }

    /// Ends a registration: the window manager is replaced by `NullWm` (the windows it held are kept), the UI settles what it
    /// held `error(no-ui)` and refuses from then on, the observer is dropped and its settings keep their last values. A
    /// provider is dropped here, which is where its `drop_user` runs. `false` when `id` is not registered.
    pub fn remove(&self, id: ProviderId) -> bool {
        let (control, was_wm, pending, was_ui, observer) = {
            let mut inner = self.lock();
            let was_wm = inner.wm == Some(id);
            let was_ui = inner.ui == Some(id);
            if was_wm {
                inner.wm = None;
            }
            if was_ui {
                inner.ui = None;
            }
            let observer = match inner.config.take() {
                Some((held, observer)) if held == id => Some(observer),
                other => {
                    inner.config = other;
                    None
                }
            };
            (
                inner.control.clone(),
                was_wm,
                inner.pending_wm.take_if(|_| was_wm),
                was_ui,
                observer,
            )
        };
        if was_wm
            && pending.is_none()
            && let Some(control) = control
        {
            control.set_window_manager(None);
        }
        if was_ui {
            self.ui.remove(id);
        }
        was_wm || was_ui || observer.is_some()
    }

    /// Records the accepted document (as JSON) and tells the observer, on this thread.
    pub fn commit_document(&self, document: String) {
        let observer = {
            let mut inner = self.lock();
            inner.document = Some(document.clone());
            inner.config.as_ref().map(|(_, observer)| Arc::clone(observer))
        };
        if let Some(observer) = observer {
            observer.lock().unwrap_or_else(PoisonError::into_inner).apply(&document);
        }
    }

    /// Sets a compositor setting by key (`design/modular-framework.md`, 28.2). Applied to the session at once when it is
    /// attached, and when it attaches otherwise.
    ///
    /// # Errors
    /// [`SettingError`]: no such key, a value of the wrong kind, or one the compositor refuses.
    pub fn setting_set(&self, key: &str, value: &SettingValue) -> Result<(), SettingError> {
        let (action, control): (SettingEffect, Option<ControlHandle>) = {
            let mut inner = self.lock();
            let action = inner.settings.set(key, value)?;
            (action, inner.control.clone())
        };
        if let Some(control) = control {
            action.deliver(&control);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_wm_api::NullWm;

    use super::*;

    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl ConfigObserver for Recorder {
        fn apply(&mut self, document: &str) {
            self.0.lock().expect("lock").push(document.to_owned());
        }
    }

    #[test]
    fn one_provider_of_a_kind_at_a_time_and_a_removed_slot_is_free_again() {
        let slots = ProviderSlots::new();
        let first = slots.set_window_manager(|| Box::<NullWm>::default()).expect("free");
        assert_eq!(
            slots.set_window_manager(|| unreachable!("a busy slot builds nothing")),
            Err(Busy)
        );
        assert!(slots.remove(first));
        assert!(!slots.remove(first), "a registration ends once");
        slots
            .set_window_manager(|| Box::<NullWm>::default())
            .expect("free again");
    }

    #[test]
    fn an_observer_hears_the_document_it_registered_after_and_every_commit_after_that() {
        let slots = ProviderSlots::new();
        slots.commit_document("{\"a\":1}".to_owned());
        let heard = Arc::new(Mutex::new(Vec::new()));
        let id = slots
            .set_config(|| Box::new(Recorder(Arc::clone(&heard))))
            .expect("free");
        slots.commit_document("{\"b\":2}".to_owned());
        assert_eq!(*heard.lock().expect("lock"), ["{\"a\":1}", "{\"b\":2}"]);
        assert!(slots.remove(id));
        slots.commit_document("{\"c\":3}".to_owned());
        assert_eq!(heard.lock().expect("lock").len(), 2, "a removed observer hears nothing");
    }

    #[test]
    fn a_setting_set_before_the_session_is_kept_for_it() {
        let slots = ProviderSlots::new();
        slots
            .setting_set("bell.flash", &SettingValue::Text("frame".to_owned()))
            .expect("sets");
        assert_eq!(
            slots.setting_set("nope", &SettingValue::Bool(true)),
            Err(SettingError::UnknownKey)
        );
    }
}
