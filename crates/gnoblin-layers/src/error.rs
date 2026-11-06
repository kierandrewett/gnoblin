//! Typed errors for every rule the layer registry enforces.
//!
//! Each variant is one rule, so a caller can match on the reason and map it to a
//! stable protocol error. The `Display` text is for logs and debugging; it is not
//! a contract, and nothing should parse it.

use std::fmt;

use gnoblin_types::OwnerId;

/// A layer operation was refused. Each variant names the rule that refused it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerError {
    /// The name is empty. A layer must have a name to be addressed at all.
    EmptyName,
    /// The name is already owned by another registration. Names are unique and
    /// belong to their first registrant, so a second registration is refused even
    /// when it comes from the same provider.
    NameTaken {
        /// The contested name.
        name: String,
    },
    /// The name is reserved for a built-in provider, and the caller is an
    /// extension. Reserved names are refused before the taken check, so an
    /// extension learns nothing about who owns a built-in name.
    ReservedName {
        /// The reserved name the extension asked for.
        name: String,
    },
    /// No layer with this name is registered.
    NotFound {
        /// The name that was looked up.
        name: String,
    },
    /// The layer exists but belongs to a different provider. Only the owner may
    /// remove a layer, so a provider cannot take down another provider's surface.
    NotOwner {
        /// The layer's name.
        name: String,
        /// The provider that owns the layer.
        owner: OwnerId,
        /// The provider that tried the change.
        caller: OwnerId,
    },
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayerError::EmptyName => write!(f, "layer name is empty"),
            LayerError::NameTaken { name } => write!(f, "layer name `{name}` is already registered"),
            LayerError::ReservedName { name } => {
                write!(f, "layer name `{name}` is reserved for a built-in provider")
            }
            LayerError::NotFound { name } => write!(f, "no layer named `{name}` is registered"),
            LayerError::NotOwner { name, owner, caller } => write!(
                f,
                "layer `{name}` belongs to owner {} and cannot be changed by owner {}",
                owner.get(),
                caller.get()
            ),
        }
    }
}

impl std::error::Error for LayerError {}
