//! What a cover hides.
//!
//! The identifiers the lock names are the shared primitives: a client is a
//! [`gnoblin_types::ClientId`] and an output a [`gnoblin_types::OutputId`], the
//! same values the compositor routes input by and the window manager places
//! windows on. The lock never invents one and never reads a clock, so
//! `Lock::apply` replays, which the model tests in [`crate::model`] depend on.

use gnoblin_types::ClientId;

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
