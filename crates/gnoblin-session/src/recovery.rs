//! The recovery state: the only state that outlives one session.
//!
//! `design/startup-recovery.md` describes what the C guardian keeps on disk:
//! which configuration document was last accepted, and enough evidence for the
//! next login to decide whether to run the user's files at all. This crate keeps
//! that state as data and reports each change with
//! [`crate::Effect::SetRecovery`]; the caller writes it under
//! `$XDG_STATE_HOME/gnoblin/`.
//!
//! # On-disk format
//!
//! The file is `$XDG_STATE_HOME/gnoblin/recovery`, or `$HOME/.local/state/gnoblin/recovery`
//! when `XDG_STATE_HOME` is unset, and `/tmp` stands in for the home directory when
//! `HOME` is unset too. It is four `key=value` lines, written in this order:
//!
//! ```text
//! failed_starts=<decimal count>
//! safe_mode=<0 or 1>
//! last_good=<revision, or empty when none>
//! status=<idle, accepted, rejected or defaults>
//! ```
//!
//! The writer creates the directory, writes `recovery.new` and renames it over
//! `recovery`. The rename makes the replacement atomic for a reader, but nothing is
//! flushed to disk first, so a power cut can leave an empty or stale file. The reader
//! treats that as a hint, not an error: a missing or unreadable file yields the default
//! state, and an unknown key or an unparsable value is skipped.
//!
//! The reader restores `status=accepted` only. `rejected` and `defaults` are written
//! but read back as `idle`, so the diagnostic and the reason are not kept across a
//! login. `last_good` survives, which is what the next session needs to decide what to run.

/// The configuration transaction outcomes the guardian tracks.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ConfigStatus {
    /// No configuration transaction has reported yet.
    #[default]
    Idle,
    /// The runtime committed a document.
    Accepted,
    /// The runtime rejected a candidate; the last good document stays in force
    /// and the diagnostic is kept for the recovery overlay.
    Rejected {
        /// The revision that was rejected.
        revision: u64,
        /// What the runtime said about it.
        diagnostic: String,
    },
    /// The saved last-good document could not be used, so embedded defaults run.
    Defaults {
        /// Why the saved document was unusable.
        reason: String,
    },
}

/// Which configuration the session is running, and how the last attempt went.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigRecovery {
    /// The revision of the last document the runtime committed, if any. It
    /// survives a later rejection, which is why it is separate from
    /// [`Self::status`].
    pub last_good: Option<u64>,
    /// The outcome of the most recent transaction.
    pub status: ConfigStatus,
}

/// Everything the next session needs to know about the last ones.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Recovery {
    /// Session starts that ended before the session became usable, counted in a
    /// row. Reset when a start succeeds, so an intermittent fault does not
    /// accumulate into safe mode over months.
    pub failed_starts: u32,
    /// Whether this session runs the embedded defaults instead of the user's
    /// configuration.
    ///
    /// Set when [`Self::failed_starts`] reaches the configured threshold and
    /// cleared when the user's next configuration transaction is accepted, so a
    /// repaired configuration leaves safe mode without a second flag file.
    pub safe_mode: bool,
    /// The last-good document and the outcome of the last transaction.
    pub config: ConfigRecovery,
}
