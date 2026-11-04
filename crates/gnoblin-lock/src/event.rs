//! What the compositor tells the lock, and why a cover can fail.

use crate::{ClientId, OutputId, Subject};

/// Everything that can change the lock's mind.
///
/// The compositor translates protocol requests, render results, output
/// hotplug and its own process events into these. No Smithay type, no clock and
/// no protocol object appears here, so a test drives the machine exactly as the
/// compositor does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// A client sent `ext_session_lock_manager_v1.lock`.
    LockRequested {
        /// The requesting connection.
        locker: ClientId,
    },
    /// A frame newer than the barrier reached an output.
    ///
    /// The compositor echoes the generation from
    /// [`crate::Effect::AwaitPresentation`], so a frame queued before the cover
    /// was raised cannot prove it.
    CoverPresented {
        /// The barrier generation the presented frame belongs to.
        generation: u64,
        /// The output that presented it.
        output: OutputId,
    },
    /// The cover could not be produced for a barrier generation.
    CoverFailed {
        /// The barrier generation that failed.
        generation: u64,
        /// Why, for logs and for the reason a replacement is needed.
        reason: CoverFailure,
    },
    /// The output layout is now exactly this set: hotplug, unplug, monitor or
    /// mode changes all arrive here.
    OutputsChanged {
        /// Every output that exists now.
        outputs: Vec<OutputId>,
    },
    /// The active locker mapped a lock surface on an output.
    LockerSurfaceMapped {
        /// The locker that owns it.
        locker: ClientId,
        /// The output the surface covers.
        output: OutputId,
    },
    /// A lock surface stopped being usable: unmapped, destroyed, or its commit
    /// was refused.
    LockerSurfaceUnmapped {
        /// The locker that owned it.
        locker: ClientId,
        /// The output the surface covered.
        output: OutputId,
    },
    /// The locker's connection went away.
    LockerGone {
        /// The locker that disappeared.
        locker: ClientId,
    },
    /// The locker destroyed its lock object before it received `locked`.
    LockAbandoned {
        /// The locker that gave up.
        locker: ClientId,
    },
    /// The owner authenticated and asked to unlock.
    UnlockRequested {
        /// The requesting connection.
        locker: ClientId,
    },
    /// The cover is gone and every covered subject is on screen again.
    RevealComplete,
    /// A subject appeared while the session is covered.
    SubjectAdded {
        /// The subject that must be hidden.
        subject: Subject,
    },
    /// A subject no longer exists.
    SubjectRemoved {
        /// The subject that went away.
        subject: Subject,
    },
    /// The lock protocol was disabled by a configuration reload.
    ProtocolDisabled,
    /// The runtime process (the Lua host) restarted.
    RuntimeRestarted,
}

/// Why a cover did not happen, or why a covered session stays covered.
///
/// Every variant ends in the same place: [`crate::State::CoverFailed`] with the
/// embargo held and nothing revealed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverFailure {
    /// A lock was requested while no output exists, so no frame could ever
    /// prove the cover.
    NoOutputs,
    /// The compositor could not produce a frame for the cover.
    RenderFailed,
    /// The locker destroyed its lock object before `locked`.
    LockerAbandoned,
    /// The locker's connection died.
    LockerDied,
    /// A configuration reload withdrew the lock protocol while covered.
    ProtocolDisabled,
}

impl CoverFailure {
    /// A stable lower-case name for logs, events and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoOutputs => "no-outputs",
            Self::RenderFailed => "render-failed",
            Self::LockerAbandoned => "locker-abandoned",
            Self::LockerDied => "locker-died",
            Self::ProtocolDisabled => "protocol-disabled",
        }
    }
}
