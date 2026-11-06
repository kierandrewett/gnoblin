//! One portal dialog on screen: the shell that runs egui for it, and the dialog
//! kind's UI behind the [`DialogUi`] trait.

use egui::Event;
use gnoblin_portal_appchooser::AppChooserUi;
use gnoblin_portal_consent::ConsentUi;
use gnoblin_portal_core::model::{Dialog, DirectoryListing, ScreenKey, Settled};
use gnoblin_portal_core::paint::Frame;
use gnoblin_portal_core::ui::{DialogShell, DialogUi, Setup, key_events, press_events, release_events};
use gnoblin_portal_filechooser::FileChooserUi;
use gnoblin_types::Tick;

/// The one place that maps a dialog kind to its UI crate. A new dialog kind adds
/// one arm here.
#[must_use]
pub fn ui_for(dialog: Dialog) -> Box<dyn DialogUi> {
    match dialog {
        Dialog::Consent(consent) => Box::new(ConsentUi::new(consent)),
        Dialog::Files(chooser) => Box::new(FileChooserUi::new(chooser)),
        Dialog::App(chooser) => Box::new(AppChooserUi::new(chooser)),
    }
}

/// One dialog on screen. The shell runs the frames; the UI crate draws the model.
pub struct PortalScreen {
    shell: DialogShell,
    ui: Box<dyn DialogUi>,
    now: Tick,
    settled: Option<Settled>,
}

impl PortalScreen {
    /// A screen for `dialog`, laid out once at `now`.
    #[must_use]
    pub fn new(dialog: Dialog, setup: Setup, now: Tick) -> Self {
        let mut shell = DialogShell::new(setup);
        let mut ui = ui_for(dialog);
        shell.layout(&mut *ui, now);
        Self {
            shell,
            ui,
            now,
            settled: None,
        }
    }

    /// The result, once the dialog has settled.
    #[must_use]
    pub const fn settled(&self) -> Option<&Settled> {
        self.settled.as_ref()
    }

    /// Moves the screen's clock. Animations and the arm delay read this value.
    pub const fn tick(&mut self, now: Tick) {
        self.now = now;
    }

    /// Feeds a key. Returns the result when this key settles the dialog.
    pub fn key(&mut self, key: ScreenKey, repeat: bool) -> Option<Settled> {
        self.run(key_events(key, repeat))
    }

    /// Feeds a pointer press at frame pixel `(x, y)`.
    pub fn pointer_press(&mut self, x: f32, y: f32) -> Option<Settled> {
        self.run(press_events(x, y))
    }

    /// Feeds a pointer release at frame pixel `(x, y)`.
    pub fn pointer_release(&mut self, x: f32, y: f32) -> Option<Settled> {
        self.run(release_events(x, y))
    }

    /// The compositor dismissed the dialog outside the UI. Returns `None` once settled.
    pub fn cancel(&mut self) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        self.settled = Some(Settled::Cancelled);
        self.settled.clone()
    }

    /// Gives a FileChooser the listing it asked for. Returns `false` when the
    /// dialog is settled or is not waiting for that folder.
    pub fn list(&mut self, listing: DirectoryListing) -> bool {
        self.settled.is_none() && self.ui.list(listing)
    }

    /// The folder a FileChooser needs listed, if it asked for one.
    #[must_use]
    pub fn listing_request(&self) -> Option<&str> {
        self.ui.listing_request()
    }

    /// Draws the dialog for this frame and rasterises it.
    /// The frame is returned for the caller to paint or ignore; calling this also runs the frame.
    pub fn render(&mut self) -> Frame {
        self.shell.render(&mut *self.ui, self.now)
    }

    fn run(&mut self, events: Vec<Event>) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        let settled = self.shell.run(&mut *self.ui, events, self.now);
        self.settled = settled.clone();
        settled
    }
}
