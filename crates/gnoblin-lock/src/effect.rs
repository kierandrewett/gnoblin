//! What the lock asks the compositor to do.

use gnoblin_types::{ClientId, OutputId};

use crate::{CoverFailure, Embargo, State, Subject};

/// One imperative instruction, produced by [`crate::Lock::apply`].
///
/// Effects are in the order they must happen: a cover is always raised before a
/// barrier is armed, and the embargo is announced before any state that would
/// admit input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    /// Raise (or keep) the opaque cover over exactly these subjects, with the
    /// active locker's own surfaces above it.
    Cover {
        /// The barrier generation this cover belongs to.
        generation: u64,
        /// Every subject that must be hidden, in identifier order.
        subjects: Vec<Subject>,
    },
    /// Arm the presentation barrier: nothing is locked until a frame newer than
    /// this generation reaches every listed output.
    AwaitPresentation {
        /// The generation a proving frame must carry.
        generation: u64,
        /// Every output that must present, in identifier order.
        outputs: Vec<OutputId>,
    },
    /// Send `locked` on the locker's lock object. Emitted once per owner, even
    /// when hotplug re-proves the cover later.
    SendLocked {
        /// The owner to notify.
        locker: ClientId,
    },
    /// Send `finished`: the request was not granted.
    Reject {
        /// The refused requester.
        locker: ClientId,
        /// Why it was refused.
        reason: RejectReason,
    },
    /// Send a fresh configure to the locker's surfaces for the current layout.
    ConfigureLockSurfaces {
        /// The owner whose surfaces to reconfigure.
        locker: ClientId,
        /// Every output that needs a surface, in identifier order.
        outputs: Vec<OutputId>,
    },
    /// Take the cover down. The session stays covered until
    /// [`crate::Event::RevealComplete`].
    Reveal,
    /// Keep the cover and the embargo although no owner can proceed.
    KeepCovered {
        /// Why nothing may be revealed.
        reason: CoverFailure,
    },
    /// The embargo now has exactly this value.
    Embargo(Embargo),
    /// Publish the state to observers.
    Publish(Published),
}

/// The state observers see, matching `gnoblin.session.lock-state-changed` and
/// `org.freedesktop.ScreenSaver.ActiveChanged`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Published {
    /// The state after the event.
    pub state: State,
    /// Whether `ScreenSaver.GetActive` flips. It stays false through
    /// `Locking` to `Locked` to `CoverFailed`, so those transitions publish no
    /// `ActiveChanged`, only a lock-state change.
    pub active_changed: bool,
}

/// Why a lock request received `finished`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectReason {
    /// Another lock is already active or being established. The first request
    /// owns it; concurrent ones are refused.
    AlreadyLocked,
    /// No output exists, so the cover could never be proven.
    CoverUnavailable,
    /// The lock is not `locked` yet, so `unlock_and_destroy` is a protocol
    /// error.
    NotLocked,
    /// Only the owner may unlock.
    NotOwner,
}

impl RejectReason {
    /// A stable lower-case name for logs and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyLocked => "already-locked",
            Self::CoverUnavailable => "cover-unavailable",
            Self::NotLocked => "not-locked",
            Self::NotOwner => "not-owner",
        }
    }
}
