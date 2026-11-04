//! The identifiers the compositor hands the lock, and the thing a cover hides.
//!
//! The lock never invents an identifier and never reads a clock: every id below
//! is allocated by the compositor and every `u64` generation is one the lock
//! itself counted. That is what makes `Lock::apply` replayable, which the model
//! tests in [`crate::model`] depend on.

/// A Wayland client connection, as the compositor names it.
///
/// Identity is a connection, not a process: a connection is the unit the
/// compositor routes input to, observes going away and can close. One process
/// may hold several connections and `fork` duplicates none of them, so a
/// connection id cannot be inherited by a child the way a session leader or a
/// process id can.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClientId(u64);

impl ClientId {
    /// Wraps the compositor's stable client index.
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

/// An output that the cover must reach before the session may be called locked.
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

/// Something the lock can hide, feed input to, or embargo.
///
/// The current C implementation covers "every surface": one implicit subject.
/// Nested sessions (design/rust-compositor.md, D4) make the covered set a set of
/// clients, so the machine names its subject instead of assuming the desktop is
/// the only thing under the cover. Nothing in the state machine branches on
/// which variant a subject is; only [`crate::Lock::covered`] readers do, and a
/// plain session uses [`Subject::Session`] alone.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Subject {
    /// The whole session: every ordinary client and every surface it draws.
    Session,
    /// One client on its own, such as a nested session hosted under D4.
    Client(ClientId),
}
