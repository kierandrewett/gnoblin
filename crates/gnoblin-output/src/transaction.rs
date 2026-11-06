//! The transactional apply protocol: propose, test, commit, confirm or roll back.

use crate::effect::{Effect, Rejection, RollbackReason};
use crate::engine::LayoutError;
use crate::layout::Layout;

/// A handle for the transaction a caller is answering.
///
/// Tokens exist because a verdict, a confirmation and a rollback all arrive from
/// outside this crate, possibly late. A token that no longer names the transaction
/// in flight is refused instead of being applied to whatever replaced it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TransactionToken(u64);

impl TransactionToken {
    /// The token's value, for logging.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Where a transaction is in its lifecycle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Nothing is in flight; the applied layout is stable.
    Idle,
    /// The layout has been handed to the backend and is waiting for a verdict.
    Testing,
    /// The backend accepted the layout; it stays until it is confirmed or the
    /// timeout expires.
    Committed {
        /// Ticks elapsed since the commit.
        elapsed_ticks: u64,
    },
}

/// What the caller's test of a proposed layout reported.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Every output accepted the layout.
    Passed,
    /// The hardware did not take it, so the previous layout must come back.
    Failed,
}

/// The propose-commit-confirm protocol of an output configuration.
///
/// The shape exists because a mode set that the user cannot see is worse than no
/// change at all: the layout is applied provisionally, and unless it is confirmed
/// within the timeout it is replaced by the layout that was in force when the
/// proposal arrived. Keeping the previous [`Layout`] inside the transaction, rather
/// than recomputing it, is what makes the restore exact through a mode change that
/// altered every logical size.
///
/// The clock is the caller's: [`Transaction::tick`] advances time, so a test can
/// reach the timeout without sleeping and the same sequence always produces the
/// same effects.
#[derive(Debug)]
pub struct Transaction {
    timeout_ticks: u64,
    phase: Phase,
    next_token: u64,
    pending: Option<(TransactionToken, Layout)>,
    previous: Layout,
}

impl Transaction {
    /// A transaction that rolls back after `confirm_timeout_ticks` ticks.
    ///
    /// A zero timeout disables the automatic rollback, which is what a headless
    /// test session wants; a session with a person in front of it always uses a
    /// non-zero value, because otherwise a black screen never comes back.
    #[must_use]
    pub fn new(confirm_timeout_ticks: u64) -> Self {
        Self {
            timeout_ticks: confirm_timeout_ticks,
            phase: Phase::Idle,
            next_token: 0,
            pending: None,
            previous: Layout::default(),
        }
    }

    /// Where the transaction is.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether nothing is in flight.
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    /// Offers `layout` for testing, remembering `previous` as the layout to restore.
    ///
    /// A proposal that presents nothing is refused here as well as in the layout
    /// engine, because this is the last place before the backend would turn every
    /// output off.
    pub fn propose(&mut self, layout: Layout, previous: Layout) -> Result<Vec<Effect>, Rejection> {
        if !self.is_idle() {
            return Err(Rejection::Busy);
        }
        if layout.is_empty() {
            return Err(Rejection::Invalid(LayoutError::NoEnabledOutputs));
        }
        let token = TransactionToken(self.next_token);
        self.next_token = self.next_token.saturating_add(1);
        self.previous = previous;
        self.pending = Some((token, layout.clone()));
        self.phase = Phase::Testing;
        Ok(vec![Effect::TestRequested { token, layout }])
    }

    /// Records what the caller's test reported.
    ///
    /// A passing verdict commits and starts the confirmation timeout; a failing one
    /// rolls back immediately, before anything was committed.
    pub fn verdict(&mut self, token: TransactionToken, verdict: Verdict) -> Result<Vec<Effect>, Rejection> {
        let Some((pending_token, layout)) = self.pending.take() else {
            return Err(Rejection::NoTransaction);
        };
        if pending_token != token || !matches!(self.phase, Phase::Testing) {
            self.pending = Some((pending_token, layout));
            return Err(Rejection::StaleToken);
        }
        match verdict {
            Verdict::Passed => {
                self.phase = Phase::Committed { elapsed_ticks: 0 };
                self.pending = Some((token, layout.clone()));
                Ok(vec![Effect::Committed { token, layout }])
            }
            Verdict::Failed => Ok(self.abandon(token, RollbackReason::TestFailed)),
        }
    }

    /// Advances the clock, rolling back a commit that was never confirmed.
    pub fn tick(&mut self, ticks: u64) -> Vec<Effect> {
        let Phase::Committed { elapsed_ticks } = self.phase else {
            return Vec::new();
        };
        let elapsed = elapsed_ticks.saturating_add(ticks);
        if self.timeout_ticks == 0 || elapsed < self.timeout_ticks {
            self.phase = Phase::Committed { elapsed_ticks: elapsed };
            return Vec::new();
        }
        match self.pending.as_ref().map(|(token, _)| *token) {
            Some(token) => self.abandon(token, RollbackReason::Timeout),
            None => Vec::new(),
        }
    }

    /// Accepts the committed layout and ends the transaction.
    pub fn confirm(&mut self, token: TransactionToken) -> Result<Vec<Effect>, Rejection> {
        if !matches!(self.phase, Phase::Committed { .. }) {
            return Err(Rejection::NotCommitted);
        }
        let Some((pending_token, layout)) = self.pending.take() else {
            return Err(Rejection::NotCommitted);
        };
        if pending_token != token {
            self.pending = Some((pending_token, layout));
            return Err(Rejection::StaleToken);
        }
        self.phase = Phase::Idle;
        self.previous = Layout::default();
        Ok(vec![Effect::Confirmed { token, layout }])
    }

    /// Restores the previous layout, whatever phase the transaction is in.
    ///
    /// A rollback during the test is as valid as one after the commit: it is how a
    /// person cancels a change they can see is wrong before answering.
    pub fn rollback(&mut self, token: TransactionToken) -> Result<Vec<Effect>, Rejection> {
        let Some(pending_token) = self.pending.as_ref().map(|(token, _)| *token) else {
            return Err(Rejection::NoTransaction);
        };
        if pending_token != token {
            return Err(Rejection::StaleToken);
        }
        Ok(self.abandon(token, RollbackReason::Requested))
    }

    /// Abandons an in-flight transaction because the hardware changed.
    ///
    /// The caller owns the layout that replaces it: only it knows what is still
    /// connected, so this returns the token and nothing else.
    pub fn abort(&mut self) -> Option<TransactionToken> {
        let (token, _) = self.pending.take()?;
        self.phase = Phase::Idle;
        self.previous = Layout::default();
        Some(token)
    }

    /// Ends the transaction and emits the restore of the previous layout.
    fn abandon(&mut self, token: TransactionToken, reason: RollbackReason) -> Vec<Effect> {
        self.phase = Phase::Idle;
        self.pending = None;
        let layout = std::mem::take(&mut self.previous);
        vec![Effect::RolledBack { token, layout, reason }]
    }
}
