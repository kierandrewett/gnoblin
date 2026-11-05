//! The identifiers the machines key their tables by.
//!
//! [`OutputId`] and [`OwnerId`] are `gnoblin-types`' shared identities,
//! re-exported here so a consumer of this crate names one path for them.
//! [`KeyCode`] and [`Keysym`] are not identities: one is a hardware key number
//! and the other an XKB symbol, both of which `gnoblin-input` produces and only
//! this crate's own machines consume.
//!
//! Nothing here allocates an identifier or reads a clock: every keycode,
//! keysym, owner and output comes from the compositor, which is what makes
//! [`crate::A11y::apply`] replayable from an event log.

pub use gnoblin_types::{OutputId, OwnerId};

/// A hardware keycode, as libinput and XKB report it.
///
/// Bounce keys match presses by keycode rather than by keysym, so pressing `a`
/// and then `A` within the window is still a repeat: it is the same physical
/// key.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyCode(u32);

impl KeyCode {
    /// Wraps a hardware keycode, including the XKB `+8` offset if the caller
    /// already applied it.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw keycode, for logs and for the composition root's key table.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// An XKB keysym: the symbol a key produces in the current layout.
///
/// The screen reader grabs keys and modifiers by keysym
/// (`org.freedesktop.a11y.KeyboardMonitor.SetKeyGrabs`), so keysyms, not
/// keycodes, are what the watch and grab table stores.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Keysym(u32);

impl Keysym {
    /// Wraps an X11 keysym value.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw keysym, for the monitor's copy of a grab list.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}
