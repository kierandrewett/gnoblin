//! Who registers a layer.
//!
//! A provider is either a Gnoblin built-in or an extension. The provider sets
//! the namespace of every layer it registers, through [`Provider::namespace`],
//! so the name is never a free choice. Built-ins all share `gnoblin`; the
//! `&'static str` says which built-in it is, for logs and `layers.list`, and
//! does not change the namespace.

use crate::name::{NameError, ProviderId};

/// The crate name of the compositor itself. It is the only built-in that may
/// register into the compositor-only bands, `Lock` and `Cursor`.
pub const COMPOSITOR_CRATE: &str = "gnoblin-compositor";

/// The party that registers and owns layers.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    /// A built-in Gnoblin crate, named by its crate name, such as `gnoblin-wayland`.
    Gnoblin(&'static str),
    /// An extension, named by its manifest id. The id is one segment, so the
    /// namespace `ext.<id>` is unambiguous.
    Extension(String),
}

impl Provider {
    /// The namespace this provider registers under.
    ///
    /// # Errors
    ///
    /// Returns the [`NameError`] when an extension id is outside the grammar. The
    /// reserved id `gnoblin` is not an error here; the registry refuses it with
    /// `RegisterError::ProviderIdReserved`, so the reason is named.
    pub fn namespace(&self) -> Result<ProviderId, NameError> {
        match self {
            Provider::Gnoblin(_) => Ok(ProviderId::gnoblin()),
            Provider::Extension(id) => ProviderId::extension(id),
        }
    }

    /// Whether this is a Gnoblin built-in, which may use the bands above the
    /// extension ceiling.
    #[must_use]
    pub fn is_builtin(&self) -> bool {
        matches!(self, Provider::Gnoblin(_))
    }

    /// Whether this is the compositor itself, the only provider that may register
    /// into the compositor-only bands.
    #[must_use]
    pub fn is_compositor(&self) -> bool {
        matches!(self, Provider::Gnoblin(name) if *name == COMPOSITOR_CRATE)
    }
}
