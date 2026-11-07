//! A method call's handle.

use gnoblin_types::{ErrorCode, ObjectId, OperationId};

use crate::Value;

/// What a method call returns: a pending operation and the object it was called on.
///
/// The handle is a value, not a capability. The graph holds the pending entry and
/// only [`crate::Graph::complete`] finishes it, so a client cannot complete an
/// operation someone else owns, and completing one twice is refused instead of
/// delivered twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Operation {
    id: OperationId,
    object: ObjectId,
}

impl Operation {
    /// Record an operation, for a graph that issued it or a client that received
    /// the reply naming it.
    pub const fn new(id: OperationId, object: ObjectId) -> Operation {
        Operation { id, object }
    }

    /// The operation's identity, which is what a completion is matched on.
    pub const fn id(self) -> OperationId {
        self.id
    }

    /// The object the method was called on.
    pub const fn object(self) -> ObjectId {
        self.object
    }
}

/// How a method call ended. Exactly one of these reaches a subscriber per
/// operation, and the graph refuses a second one.
///
/// `Value` is the one outcome that carries data: a read such as `store.get` returns
/// what it read here, so the reply and its completion are one event. It is bounded by
/// the wire frame cap, so a read that would not fit is refused with a typed error
/// instead of truncated; list-like reads page by cursor rather than return everything.
#[derive(Clone, Debug, PartialEq)]
pub enum OperationOutcome {
    /// The method did what it was asked.
    Done,
    /// The method refused, with the schema's error code.
    Failed(ErrorCode),
    /// The method did what it was asked and returns this value.
    Value(Value),
}

impl OperationOutcome {
    /// Whether the call succeeded.
    pub fn is_done(&self) -> bool {
        matches!(self, OperationOutcome::Done | OperationOutcome::Value(_))
    }
}
