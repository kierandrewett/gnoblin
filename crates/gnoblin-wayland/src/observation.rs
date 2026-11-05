//! The window state a test or a diagnostics reader observes.
//!
//! The server owns its policy state on its own thread, so a test cannot borrow it.
//! What it can do is read a snapshot: the server republishes [`SessionSnapshot`] after
//! every change and a caller holds the [`std::sync::Arc`] the server was built
//! with. That is the in-process observation recorder `gnoblin-testkit` will build
//! on, kept here so the first real client test needs nothing else.

use std::collections::BTreeMap;

use gnoblin_types::{OutputId, Rect, WindowId};

/// One window, as the server last saw it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ObservedWindow {
    /// The policy identity of the window.
    pub id: WindowId,
    /// The title the client last sent.
    pub title: String,
    /// The application id the client last sent.
    pub app_id: String,
    /// Whether the client has committed a buffer, so the window is drawn.
    pub mapped: bool,
    /// Whether the window manager considers the window visible now.
    pub visible: bool,
    /// Whether the window holds keyboard focus.
    pub focused: bool,
    /// The window's frame rectangle in global logical coordinates.
    pub geometry: Rect,
}

/// The window state at one instant.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct SessionSnapshot {
    pub(crate) windows: BTreeMap<WindowId, ObservedWindow>,
    pub(crate) outputs: Vec<OutputId>,
    pub(crate) focus: Option<WindowId>,
}

impl SessionSnapshot {
    /// Every managed window, in policy identity order.
    pub fn windows(&self) -> impl Iterator<Item = &ObservedWindow> {
        self.windows.values()
    }

    /// One window, if it is managed.
    #[must_use]
    pub fn window(&self, id: WindowId) -> Option<&ObservedWindow> {
        self.windows.get(&id)
    }

    /// How many windows are managed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no window is managed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The window holding keyboard focus.
    #[must_use]
    pub fn focused(&self) -> Option<&ObservedWindow> {
        self.focus.and_then(|id| self.windows.get(&id))
    }

    /// The identity of the focused window.
    #[must_use]
    pub fn focus(&self) -> Option<WindowId> {
        self.focus
    }

    /// The outputs the server presents on.
    #[must_use]
    pub fn outputs(&self) -> &[OutputId] {
        &self.outputs
    }

    /// The one managed window, when a test knows there is exactly one.
    #[must_use]
    pub fn only(&self) -> Option<&ObservedWindow> {
        if self.windows.len() == 1 {
            self.windows.values().next()
        } else {
            None
        }
    }

    /// The window with this title, if it is managed.
    #[must_use]
    pub fn titled(&self, title: &str) -> Option<&ObservedWindow> {
        self.windows.values().find(|window| window.title == title)
    }
}
