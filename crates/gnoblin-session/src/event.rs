//! Everything the caller tells the guardian.

use crate::Tick;
use crate::ids::ChildId;

/// One thing that happened, at the moment the caller observed it.
///
/// Every event carries its own time, so the guardian needs no clock and a test
/// can cross a deadline in one call. A timestamp older than one already seen is
/// treated as no elapsed time rather than as a jump backwards.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    /// When the caller observed it, on the caller's monotonic clock.
    pub now: Tick,
    /// What happened.
    pub kind: EventKind,
}

impl Event {
    /// An event at `now`.
    #[must_use]
    pub const fn new(now: Tick, kind: EventKind) -> Self {
        Self { now, kind }
    }
}

/// What happened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventKind {
    /// Begin the session: export the environment and start the first child.
    Start,
    /// The caller's clock advanced. Deadlines and restart ladders are evaluated
    /// here and nowhere else, so a caller that never ticks never restarts
    /// anything.
    Tick,
    /// A child reported readiness over its own channel.
    ///
    /// Only the compositor publishes `environment`, which carries the display it
    /// created (`WAYLAND_DISPLAY`, `DISPLAY`, `XAUTHORITY`). Values from any
    /// other child are ignored.
    Ready {
        /// Which child reported.
        child: ChildId,
        /// Which of its instances reported; a stale one is ignored.
        generation: u64,
        /// What the compositor published, in its own order.
        environment: Vec<(String, String)>,
    },
    /// A child's process ended.
    Exited {
        /// Which child.
        child: ChildId,
        /// Which of its instances ended; a stale one is ignored.
        generation: u64,
        /// How it ended.
        status: ExitStatus,
    },
    /// The session should end: a logout, or a termination signal.
    Shutdown {
        /// Why the session is ending.
        reason: ShutdownReason,
    },
    /// The runtime committed a configuration document.
    ConfigAccepted {
        /// The revision it committed.
        revision: u64,
    },
    /// The runtime rejected a configuration document; the previous accepted
    /// document stays in force.
    ConfigRejected {
        /// The revision that was rejected.
        revision: u64,
        /// The diagnostic the runtime reported, kept for the recovery overlay.
        diagnostic: String,
    },
    /// The saved last-good document could not be used, so embedded defaults run.
    ConfigFallback {
        /// Why the saved document was unusable.
        reason: String,
    },
}

/// How a child's process ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    /// It exited with this status.
    Exited(i32),
    /// It was killed by this signal.
    Signalled(Signal),
}

impl ExitStatus {
    /// Whether the process exited successfully.
    #[must_use]
    pub const fn success(self) -> bool {
        matches!(self, Self::Exited(0))
    }
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Exited(code) => write!(formatter, "status {code}"),
            Self::Signalled(signal) => write!(formatter, "signal {}", signal.as_str()),
        }
    }
}

/// A signal the guardian asks the caller to deliver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    /// `SIGTERM`: stop gracefully, flushing what you must.
    Term,
    /// `SIGKILL`: die now. Always the second request for a child that ignored
    /// the first.
    Kill,
}

impl Signal {
    /// A stable name for logs and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Term => "SIGTERM",
            Self::Kill => "SIGKILL",
        }
    }
}

/// Why the caller is ending the session, as far as it knows.
///
/// The guardian adds the reasons it discovers itself (a lost compositor, a
/// start that cannot finish) to [`crate::StopReason`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownReason {
    /// The user logged out.
    Logout,
    /// The guardian received a termination signal.
    Signal,
}
