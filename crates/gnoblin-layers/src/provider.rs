//! Who registers a layer.
//!
//! A provider is either a Gnoblin built-in or an extension. The provider sets
//! the namespace of every layer it registers, through [`LayerProvider::namespace`],
//! so the name is never a free choice. Built-ins all share `gnoblin`; the
//! `&'static str` says which built-in it is, for logs and `layers.list`, and
//! does not change the namespace.

use crate::name::{LayerName, LayerNameError, LocalName, ProviderId};
use crate::registry::RegisterError;

/// The crate name of the compositor itself. It is the only built-in that may
/// register into the compositor-only bands, `Lock` and `Cursor`.
pub const COMPOSITOR_CRATE: &str = "gnoblin-compositor";

/// The party that registers and owns layers.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LayerProvider {
    /// A built-in Gnoblin crate, named by its crate name, such as `gnoblin-wayland`.
    Gnoblin(&'static str),
    /// An extension, named by its manifest id. The id is one segment, so the
    /// namespace `ext.<id>` is unambiguous.
    Extension(String),
}

impl LayerProvider {
    /// The namespace this provider registers under.
    ///
    /// # Errors
    ///
    /// Returns the [`LayerNameError`] when an extension id is outside the grammar. The
    /// reserved id `gnoblin` is not an error here; the registry refuses it with
    /// `RegisterError::ProviderIdReserved`, so the reason is named.
    pub fn namespace(&self) -> Result<ProviderId, LayerNameError> {
        match self {
            LayerProvider::Gnoblin(_) => Ok(ProviderId::gnoblin()),
            LayerProvider::Extension(id) => ProviderId::extension(id),
        }
    }

    /// Whether this is a Gnoblin built-in, which may use the bands above the
    /// extension ceiling.
    #[must_use]
    pub fn is_builtin(&self) -> bool {
        matches!(self, LayerProvider::Gnoblin(_))
    }

    /// Whether this is the compositor itself, the only provider that may register
    /// into the compositor-only bands.
    #[must_use]
    pub fn is_compositor(&self) -> bool {
        matches!(self, LayerProvider::Gnoblin(name) if *name == COMPOSITOR_CRATE)
    }

    /// Turns a name a caller asked for into the local part this provider may register.
    ///
    /// The provider is the one the compositor attributed to the connection, so a caller
    /// cannot choose it. A bare local name takes this provider's namespace. A full name
    /// is accepted only when its namespace is this provider's, so an extension cannot
    /// reach `gnoblin.` or another extension's `ext.<id>.`.
    ///
    /// # Errors
    ///
    /// Returns [`RegisterError::BadName`] for text outside the grammar, and
    /// [`RegisterError::ProviderMismatch`] for a full name in another namespace.
    pub fn resolve_local(&self, requested: &str) -> Result<LocalName, RegisterError> {
        if !requested.contains('.') {
            return LocalName::new(requested).map_err(RegisterError::BadName);
        }
        let full = LayerName::parse(requested).map_err(RegisterError::BadName)?;
        let own = self.namespace().map_err(RegisterError::BadProviderId)?;
        if full.provider() != &own {
            return Err(RegisterError::ProviderMismatch {
                requested: requested.to_owned(),
                attributed: own.as_str().to_owned(),
            });
        }
        Ok(full.local().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension(id: &str) -> LayerProvider {
        LayerProvider::Extension(id.to_owned())
    }

    #[test]
    fn a_local_name_takes_the_attributed_namespace() {
        let local = extension("a").resolve_local("note").expect("local name");
        assert_eq!(local.as_str(), "note");
        // The namespace is the provider's, so the full name is `ext.a.note`.
        let full = crate::LayerName::new(extension("a").namespace().unwrap(), local);
        assert_eq!(full.to_string(), "ext.a.note");
    }

    #[test]
    fn an_extension_cannot_name_another_extensions_namespace() {
        // `B.x` is not a full name: every full name starts `ext.<id>.` or `gnoblin.`.
        assert!(matches!(
            extension("a").resolve_local("B.x"),
            Err(RegisterError::BadName(_))
        ));
        // A well-formed full name under another extension is a mismatch, not a rename.
        assert_eq!(
            extension("a").resolve_local("ext.b.x"),
            Err(RegisterError::ProviderMismatch {
                requested: "ext.b.x".to_owned(),
                attributed: "ext.a".to_owned(),
            })
        );
    }

    #[test]
    fn an_extension_cannot_name_the_gnoblin_namespace() {
        assert_eq!(
            extension("a").resolve_local("gnoblin.x"),
            Err(RegisterError::ProviderMismatch {
                requested: "gnoblin.x".to_owned(),
                attributed: "ext.a".to_owned(),
            })
        );
    }

    #[test]
    fn a_full_name_with_the_attributed_prefix_is_accepted() {
        let local = extension("a").resolve_local("ext.a.note").expect("own full name");
        assert_eq!(local.as_str(), "note");
    }

    #[test]
    fn a_builtin_cannot_name_an_extension_namespace() {
        let builtin = LayerProvider::Gnoblin("gnoblin-wayland");
        assert_eq!(
            builtin.resolve_local("ext.a.note"),
            Err(RegisterError::ProviderMismatch {
                requested: "ext.a.note".to_owned(),
                attributed: "gnoblin".to_owned(),
            })
        );
        assert_eq!(builtin.resolve_local("portals").expect("local").as_str(), "portals");
    }

    #[test]
    fn a_malformed_local_name_is_refused_with_its_reason() {
        assert!(matches!(
            extension("a").resolve_local("Bad Name"),
            Err(RegisterError::BadName(_))
        ));
    }
}
