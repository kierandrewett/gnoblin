//! Opaque identities for windows, outputs and workspaces.
//!
//! The window manager never invents identity: the caller owns the lifetime of a
//! window (a Wayland surface), an output (a connected monitor) and a workspace (a
//! configuration declaration), so it supplies the token and the window manager
//! only refuses duplicates. Keeping the token opaque stops policy code from
//! depending on a slab index, a protocol object id or a connector name, which is
//! what lets the whole state machine run without a session.

use std::fmt;

/// Assigns one newtype per kind of identity.
macro_rules! identity {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(u32);

        impl $name {
            /// Wraps a token from the caller.
            #[must_use]
            pub const fn new(raw: u32) -> Self {
                Self(raw)
            }

            /// The token the caller supplied.
            #[must_use]
            pub const fn get(self) -> u32 {
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
    /// One connected output, as named by the backend.
    OutputId
}

identity! {
    /// One workspace: a position in the declared list, addressed by a stable id.
    ///
    /// Positions change when workspaces are added, removed or reordered. The id
    /// does not, which is what makes it safe for rules and for remembering where
    /// a window belongs.
    WorkspaceId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_keep_the_caller_token() {
        assert_eq!(WindowId::new(7).get(), 7);
        assert_eq!(OutputId::new(0), OutputId::new(0));
        assert_ne!(WorkspaceId::new(1), WorkspaceId::new(2));
    }

    #[test]
    fn display_names_the_kind() {
        assert_eq!(WindowId::new(3).to_string(), "WindowId(3)");
        assert_eq!(OutputId::new(4).to_string(), "OutputId(4)");
    }
}
