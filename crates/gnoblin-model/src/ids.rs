//! The identities the graph hands out.
//!
//! Every identity is a plain counter with a type name around it, so an object id
//! cannot be passed where an operation id belongs.
//!
//! Identities are allocated by the graph and never reused. A closed object's id
//! stays dead for the life of the graph, so a subscription that outlives its
//! object can never receive a frame meant for a different one.

use std::fmt;

/// Declare one opaque identity type around a `u64`.
macro_rules! identity {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Wrap a raw number, for a wire decoder, a frame dump or a test.
            pub const fn new(raw: u64) -> $name {
                $name(raw)
            }

            /// The raw number.
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

identity!(
    /// The compositor's identity for one live object.
    ObjectId
);

identity!(
    /// One method call, from its reply until its single completion.
    OperationId
);

identity!(
    /// One offered request, from [`crate::RequestBroker::offer`] until its single
    /// finished outcome.
    RequestId
);

identity!(
    /// One subscription to the graph's observation stream.
    SubscriptionId
);

identity!(
    /// One connection's identity, as the request broker records it.
    ///
    /// The compositor assigns it per connection, so a request has exactly one
    /// owner and a claimant cannot choose who it claims to be.
    OwnerId
);
