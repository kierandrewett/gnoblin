//! The XSETTINGS owner seam.
//!
//! X11 applications do not read the configuration document; they read the XSETTINGS
//! property on the root window. Publishing it is the compositor's job, because it owns
//! the X connection the property is written through, and deciding what goes into it is
//! `gnoblin-compat-x11-xsettings`' job, because the values are the desktop's settings
//! translated for GTK.
//!
//! This module is the seam between the two: an owner supplies the values, the compositor
//! publishes them once the X11 window manager is up, and the owner hears whether it
//! worked. The trait is an *extension* trait in the sense the plan uses: nothing in this
//! crate implements it for real, and the adapter plugs in without this crate depending on
//! it.
//!
//! # Why the owner pushes and does not write
//!
//! A second writer would need the X connection, the manager window and the selection
//! owner, which is the compositor's state; handing them out would let two processes
//! fight over one property. The owner therefore answers [`XSettings::settings`] and is
//! told the outcome, and the exchange stays in the compositor's loop where the rest of
//! the X11 state lives.
//!
//! # Failure modes first
//!
//! 1. **An owner that publishes nothing leaves the previous values in place.** The list
//!    is replaced whole rather than merged, so an owner that removes a setting removes
//!    it from the clients too: `an_owner_that_empties_its_list_replaces_the_previous_one`.
//! 2. **A failed publish is silent.** [`XSettings::published`] receives the error, so an
//!    adapter can log it or retry: `a_failed_publish_reaches_the_owner`.

use std::sync::{Arc, Mutex};

pub use gnoblin_backend::x11::Setting;

/// An XSETTINGS owner.
///
/// The compositor asks for the values when the X11 window manager is up and after
/// every restart, because a new connection has no property yet.
pub trait XSettings: Send {
    /// The settings to publish, in name order.
    ///
    /// The whole list replaces what is published: a name that is absent is removed from
    /// the clients, which is what makes a setting the document no longer declares
    /// disappear instead of lingering from the previous session.
    fn settings(&mut self) -> Vec<(String, Setting)>;

    /// Records what publishing did.
    ///
    /// `Ok(())` means every setting reached the root window; an error is the reason it
    /// did not, and is logged by the caller as it stands.
    fn published(&mut self, result: Result<(), String>);
}

/// What a [`Recording`] owner holds.
#[derive(Debug, Default)]
struct Recorded {
    settings: Vec<(String, Setting)>,
    reported: Vec<Result<(), String>>,
    /// How many times the compositor asked for the values.
    reads: u32,
}

/// A test double: an owner whose values and answers a test can read.
///
/// It exists so the seam can be exercised without the compatibility adapter, which is
/// the whole point of a seam: the compositor's side is tested against a stand-in that
/// records what it was asked and told.
#[derive(Clone, Debug, Default)]
pub struct RecordingOwner {
    recorded: Arc<Mutex<Recorded>>,
}

impl RecordingOwner {
    /// An owner with no settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets one value, replacing a value of the same name.
    pub fn set(&self, name: &str, value: Setting) {
        let mut recorded = self.lock();
        recorded.settings.retain(|(existing, _)| existing != name);
        recorded.settings.push((name.to_owned(), value));
        recorded.settings.sort_by(|left, right| left.0.cmp(&right.0));
    }

    /// Forgets every value.
    pub fn clear(&self) {
        self.lock().settings.clear();
    }

    /// The answers the compositor gave, oldest first.
    #[must_use]
    pub fn reported(&self) -> Vec<Result<(), String>> {
        self.lock().reported.clone()
    }

    /// How many times the compositor asked for the values.
    #[must_use]
    pub fn reads(&self) -> u32 {
        self.lock().reads
    }
}

impl XSettings for RecordingOwner {
    fn settings(&mut self) -> Vec<(String, Setting)> {
        let mut recorded = self.lock();
        recorded.reads = recorded.reads.saturating_add(1);
        recorded.settings.clone()
    }

    fn published(&mut self, result: Result<(), String>) {
        self.lock().reported.push(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's list is what the compositor publishes, in name order, and asking
    /// twice is not a second set of values.
    #[test]
    fn an_owner_reports_its_values_in_name_order() {
        let owner = RecordingOwner::new();
        owner.set("gtk-theme", Setting::Text("Adwaita".to_owned()));
        owner.set("gtk-double-click-time", Setting::Integer(400));
        owner.set("gtk-theme", Setting::Text("HighContrast".to_owned()));

        let mut boxed: Box<dyn XSettings> = Box::new(owner);
        assert_eq!(
            boxed.settings(),
            vec![
                ("gtk-double-click-time".to_owned(), Setting::Integer(400)),
                ("gtk-theme".to_owned(), Setting::Text("HighContrast".to_owned())),
            ]
        );
    }

    /// An empty list is the answer that removes every setting from the clients, so it
    /// must not read as "no change".
    #[test]
    fn an_owner_that_empties_its_list_replaces_the_previous_one() {
        let owner = RecordingOwner::new();
        owner.set("gtk-theme", Setting::Text("Adwaita".to_owned()));
        let mut boxed: Box<dyn XSettings> = Box::new(owner.clone());
        assert_eq!(boxed.settings().len(), 1);

        owner.clear();
        assert!(boxed.settings().is_empty());
    }

    /// The error of a failed publish reaches the owner instead of being swallowed by the
    /// compositor.
    #[test]
    fn a_failed_publish_reaches_the_owner() {
        let owner = RecordingOwner::new();
        let mut boxed: Box<dyn XSettings> = Box::new(owner.clone());

        boxed.published(Err("the X connection is gone".to_owned()));
        boxed.published(Ok(()));

        assert_eq!(
            owner.reported(),
            vec![Err("the X connection is gone".to_owned()), Ok(())]
        );
    }
}
