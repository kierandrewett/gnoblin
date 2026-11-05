//! The identifiers the machines key their tables by.
//!
//! Nothing here allocates an identifier or reads a clock: every keycode,
//! keysym, owner and output comes from the compositor, which is what makes
//! [`crate::A11y::apply`] replayable from an event log.

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

/// A keyboard-monitor consumer: one assistive technology connection.
///
/// The C tree watches the D-Bus sender name and drops the grabber when the name
/// vanishes. The compositor has the same identity as a connection id, so the
/// table is keyed by it and [`crate::Event::MonitorDropped`] is the only way an
/// owner leaves.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OwnerId(u64);

impl OwnerId {
    /// Wraps the compositor's stable connection index.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw index, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// An output, as the compositor names it.
///
/// The magnifier publishes which output its view belongs to, because the render
/// path needs it and because a locked session must be able to prove the view
/// never left that output.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OutputId(u32);

impl OutputId {
    /// Wraps the compositor's stable output index.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw index, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}
