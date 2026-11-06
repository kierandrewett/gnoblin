//! What the configuration model asks the backend to do, and what it refuses.

use crate::engine::LayoutError;
use crate::layout::Layout;
use crate::transaction::TransactionToken;

/// An action the backend must perform, in the order it must happen.
///
/// The model returns these instead of touching the hardware so the same sequence
/// can be replayed in a test. Every effect carries the whole layout it means,
/// never a delta: applying a delta twice would leave the outputs in a state that
/// depends on how many times the effect was delivered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Set this layout provisionally and report the result.
    ///
    /// Until the verdict arrives the backend owns the test: it must not answer
    /// `Passed` unless every output accepted its mode, because the model treats a
    /// half-applied layout as a working one until the confirmation timeout.
    TestRequested {
        /// The transaction this test belongs to.
        token: TransactionToken,
        /// The layout under test.
        layout: Layout,
    },
    /// The test passed, so the layout is the session's configuration now.
    Committed {
        /// The transaction.
        token: TransactionToken,
        /// The layout that is now applied.
        layout: Layout,
    },
    /// The layout was confirmed and no longer needs a revert.
    Confirmed {
        /// The transaction.
        token: TransactionToken,
        /// The confirmed layout.
        layout: Layout,
    },
    /// The previous layout is in force again.
    RolledBack {
        /// The transaction that ended. A hotplug that abandons a test reports the
        /// same token it was testing.
        token: TransactionToken,
        /// The layout to restore.
        layout: Layout,
        /// Why the proposal was abandoned.
        reason: RollbackReason,
    },
    /// The layout changed outside any transaction, after a hotplug or a
    /// configuration change.
    Applied {
        /// The layout that is now applied.
        layout: Layout,
    },
    /// The request was refused and nothing changed.
    Rejected(Rejection),
}

/// Why a proposal was abandoned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RollbackReason {
    /// The caller tested the layout and reported a failure.
    TestFailed,
    /// Nobody confirmed it before the timeout expired.
    Timeout,
    /// The caller withdrew it.
    Requested,
    /// The connected set changed, so the proposal no longer describes the hardware.
    Hotplug,
}

/// Why a request changed nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Rejection {
    /// A transaction is already in flight; the caller must finish it first.
    Busy,
    /// The configuration did not resolve to a layout.
    Invalid(LayoutError),
    /// The token does not belong to the transaction the model is holding.
    StaleToken,
    /// Nothing is committed, so there is nothing to confirm.
    NotCommitted,
    /// Nothing is in flight, so there is nothing to roll back.
    NoTransaction,
}
