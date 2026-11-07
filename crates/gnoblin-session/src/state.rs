//! The states a child and the session are in.

use crate::{ChildId, ChildKind, ExitStatus};

/// Where one child is.
///
/// `Pending` and `Stopped` are not in the plan's list, and both are needed: a
/// child the plan declares and the guardian has not spawned must be observable
/// as something other than "running", and a stop the guardian asked for must be
/// distinguishable from a failure, otherwise a clean logout looks like a crash
/// in every log and every recovery count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildState {
    /// Declared by the plan and not spawned yet.
    Pending,
    /// Spawned, waiting for the readiness report its deadline covers.
    Starting,
    /// Reported ready while the start sequence still has children to launch.
    Ready,
    /// Usable, and either the session is up or the child needs no readiness.
    Running,
    /// Asked to exit: a graceful signal, a kill for a missed deadline, or the
    /// session's own shutdown.
    Stopping,
    /// Ended and waiting for its restart time, which only a caller tick reaches.
    Backoff,
    /// Ended unexpectedly and not restartable, or out of restart budget.
    Failed,
    /// Ended because the guardian asked it to.
    Stopped,
}

impl ChildState {
    /// Whether a process of this child may exist.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Starting | Self::Ready | Self::Running)
    }

    /// A stable lower-case name for logs and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Backoff => "backoff",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Where the session is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    /// Nothing has been started.
    Idle,
    /// The start sequence is running.
    Starting,
    /// Every planned child has been started and is usable.
    Up,
    /// The session runs, but a child exhausted its restart budget or the start
    /// sequence stopped at one that did. Whatever still works keeps working.
    Degraded,
    /// Children are stopping in reverse start order.
    Stopping,
    /// Every child has stopped; the caller ends the session.
    Ended,
}

impl SessionState {
    /// A stable lower-case name for logs and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Up => "up",
            Self::Degraded => "degraded",
            Self::Stopping => "stopping",
            Self::Ended => "ended",
        }
    }

    /// Whether children may be spawned in this state.
    #[must_use]
    pub const fn accepts_children(self) -> bool {
        matches!(self, Self::Idle | Self::Starting | Self::Up | Self::Degraded)
    }
}

/// Why the session is stopping or has ended.
///
/// The first two come from the caller; the last two are what the guardian
/// discovers, and they decide the exit status the caller reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// The user logged out.
    Logout,
    /// The guardian received a termination signal.
    Signal,
    /// The compositor exited, so every client lost its server.
    CompositorLost,
    /// The session never became usable: the compositor exited before it was
    /// ready, or it reported ready without the display the session needs.
    StartFailed,
}

impl StopReason {
    /// A stable lower-case name for logs and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Logout => "logout",
            Self::Signal => "signal",
            Self::CompositorLost => "compositor-lost",
            Self::StartFailed => "start-failed",
        }
    }
}

/// Why a child is `Stopping`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StopCause {
    /// It missed its readiness deadline, so it is unusable and killed.
    Unready,
    /// The session is ending and its turn in reverse start order has come.
    Session,
    /// The configuration no longer enables this autostart application, so it is stopped
    /// while the session keeps running. Its exit is expected and is not restarted.
    Retired,
}

/// A snapshot of one child, for diagnostics and tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildStatus {
    /// The planned identifier.
    pub id: ChildId,
    /// The name used in logs.
    pub name: String,
    /// What the child is.
    pub kind: ChildKind,
    /// Where the child is.
    pub state: ChildState,
    /// How many times this child has been spawned; a late event from an earlier
    /// instance carries a smaller number and is ignored.
    pub generation: u64,
    /// Restarts scheduled since the child was last stable.
    pub restarts: u32,
    /// The status of its process the last time it ended.
    pub last_exit: Option<ExitStatus>,
}
