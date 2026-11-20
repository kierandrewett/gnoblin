//! Who an input or clipboard event is for.
//!
//! The identifiers the lock names are the shared primitives: a client is a
//! [`gnoblin_types::ClientId`] and an output a [`gnoblin_types::OutputId`], the
//! same values the compositor routes input by and the window manager places
//! windows on. The lock never invents one and never reads a clock, so
//! `Lock::apply` replays, which the model tests in [`crate::model`] depend on.

use gnoblin_types::ClientId;

/// Who an input or clipboard event is for, so the lock can tell its owner from everything else.
///
/// The covered set is always the whole session: a lock hides every client the same way and the machine does not
/// branch on which one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Subject {
    /// The whole session: every ordinary client and every surface it draws.
    Session,
    /// One client on its own: the locker is the only one that may receive input while the session is covered.
    Client(ClientId),
}
