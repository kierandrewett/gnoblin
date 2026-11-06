//! Opaque identities for windows, outputs, workspaces, objects and everything a
//! caller hands in.
//!
//! Every identity is a plain [`u64`] token with a type name around it, so an
//! object id cannot be passed where an operation id belongs. The width is 64
//! bits for all of them because the object graph allocates unbounded counters
//! and an engine crate must be able to hold one without narrowing it; a backend
//! that numbers outputs from zero widens trivially, where the reverse would
//! silently truncate a graph counter.
//!
//! The caller owns the token: the compositor never invents identity, it only
//! refuses a duplicate. That is what keeps policy code free of a slab index, a
//! protocol object id or a connector name.

use std::fmt;

/// Declares one opaque identity type around a `u64`.
macro_rules! identity {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Wraps a token the caller supplied or the graph allocated.
            #[must_use]
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            /// The raw token, for a wire decoder, a log line or a test.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, concat!(stringify!($name), "({})"), self.0)
            }
        }
    };
}

identity! {
    /// One managed toplevel, popup host or dialog.
    WindowId
}

identity! {
    /// One connected output, as the backend names it.
    OutputId
}

identity! {
    /// One workspace: a stable name for a position in the declared list.
    ///
    /// Positions change when workspaces are added, removed or reordered. The id
    /// does not, which is what makes it safe for rules and for remembering where
    /// a window belongs.
    WorkspaceId
}

identity! {
    /// The compositor's identity for one live object in the graph.
    ObjectId
}

identity! {
    /// One method call, from its reply until its single completion.
    OperationId
}

identity! {
    /// One offered request, from [`crate::Tick`] it was offered until its single
    /// finished outcome.
    RequestId
}

identity! {
    /// One subscription to the graph's observation stream.
    SubscriptionId
}

identity! {
    /// One connection's identity, as the request broker records it.
    ///
    /// The compositor assigns it per connection, so a request has exactly one
    /// owner and a claimant cannot choose who it claims to be.
    OwnerId
}

identity! {
    /// One client connection, as the compositor numbers it.
    ///
    /// A connection, not a process: it is what input is routed to, what can be
    /// observed going away and what can be closed, and a forked child never
    /// shares one. Authority decides who may act per connection and the locker
    /// covers and embargoes a connection, so both name the same value.
    ClientId
}
