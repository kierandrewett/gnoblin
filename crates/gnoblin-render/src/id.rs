//! Identities the scene graph and the tween player own.
//!
//! `NodeId` and `TrackId` stay local to the renderer because they name a scene
//! node and a player track, not a shared object: an output, a window and an
//! operation get their identities from `gnoblin-types`, and this crate uses
//! those where it hands an identity back. Both are `u64` newtypes for the same
//! reason the shared set is: the graph allocates unbounded counters and a
//! narrowing here would be a silent truncation somewhere later.

use std::fmt;

macro_rules! identity {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Wraps a token from the caller or the player's allocator.
            #[must_use]
            pub const fn new(raw: u64) -> Self {
                Self(raw)
            }

            /// The raw token, for a log line or a test.
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
    /// One node in the scene graph, allocated by the caller.
    NodeId
}

identity! {
    /// One tween track, allocated by the player.
    TrackId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_display_their_kind_and_token() {
        assert_eq!(NodeId::new(7).to_string(), "NodeId(7)");
        assert_eq!(TrackId::new(3).get(), 3);
    }
}
