//! What the guardian asks the caller to do.

use crate::environment::{EnvTarget, Environment};
use crate::event::Signal;
use crate::ids::ChildId;
use crate::recovery::Recovery;

/// One imperative instruction, produced by [`crate::Guardian::apply`].
///
/// Effects are in the order they must happen. The environment is exported before
/// the first child that inherits it is spawned; a child is signalled before it is
/// killed; and every child is spawned at most once per generation, so the caller
/// never has two processes for one child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    /// Start one child in its own process group, with exactly this environment.
    ///
    /// The group matters on stop: the caller signals the group, so an autostart
    /// application's helpers go with it.
    Spawn {
        /// Which child to start.
        child: ChildId,
        /// Which instance this is. The caller reports it back in `Ready` and
        /// `Exited`, so an event from an earlier instance is recognisable.
        generation: u64,
        /// The name to log and to look up in the plan.
        name: String,
        /// The complete environment the child inherits.
        environment: Environment,
    },
    /// Ask a child's process group to exit gracefully.
    Signal {
        /// Which child to signal.
        child: ChildId,
        /// Which signal, which is always [`Signal::Term`] today: a graceful stop
        /// is the only thing the guardian asks for before it kills.
        signal: Signal,
    },
    /// Kill a child's process group. Nothing survives this, so the guardian
    /// stops waiting for the child once it has been sent.
    Kill {
        /// Which child to kill.
        child: ChildId,
    },
    /// Add one line to the session log.
    ///
    /// The message starts with the `[guardian]` tag and carries no environment
    /// value, no configuration document and no Lua source, matching the redaction
    /// rule in `design/startup-recovery.md`.
    Log {
        /// How urgent the line is.
        level: Level,
        /// The line itself.
        message: String,
    },
    /// Apply this environment to the session or to its activation environment.
    ExportEnv {
        /// Which environment to change.
        target: EnvTarget,
        /// What to set and unset.
        environment: Environment,
    },
    /// Persist the recovery state, which is the only state that outlives the
    /// session.
    SetRecovery(Recovery),
}

/// How urgent a log line is.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Level {
    /// Detail for a bug report.
    Debug,
    /// A session milestone.
    Info,
    /// Something recovered, or something the user should see.
    Warning,
    /// Something failed and the session is degraded or ending.
    Error,
}
