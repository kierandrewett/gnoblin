//! The extension namespaces a host has loaded.
//!
//! `require` answers for a namespace only when it is both loaded here and granted by
//! the compositor (`gnoblin_client::Client::granted_extensions`). The host-config
//! loader fills this registry; until it does, the stock registry is empty and `require`
//! answers for no extension at all.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The namespace the compositor's own object types live under. An extension cannot
/// claim it, because `gnoblin` is not something `require` returns.
const CORE_NAMESPACE: &str = "gnoblin";

/// The extension namespaces loaded into a host, each with the object types it declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionRegistry {
    loaded: BTreeMap<String, BTreeSet<String>>,
}

/// Why a namespace or class was refused by the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The namespace is empty, has an empty segment, or is the core namespace.
    InvalidNamespace(String),
    /// The namespace is already loaded.
    DuplicateNamespace(String),
    /// The namespace declares no object types, so there is nothing to bind.
    NoClasses(String),
    /// A class name is empty or is listed twice under one namespace.
    InvalidClass {
        /// The namespace the class was listed under.
        namespace: String,
        /// The class name that was refused.
        class: String,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::InvalidNamespace(namespace) => {
                write!(formatter, "invalid extension namespace `{namespace}`")
            }
            RegistryError::DuplicateNamespace(namespace) => {
                write!(formatter, "extension namespace `{namespace}` is already loaded")
            }
            RegistryError::NoClasses(namespace) => {
                write!(formatter, "extension namespace `{namespace}` declares no object types")
            }
            RegistryError::InvalidClass { namespace, class } => {
                write!(
                    formatter,
                    "extension `{namespace}` lists the object type `{class}` invalidly"
                )
            }
        }
    }
}

impl std::error::Error for RegistryError {}

impl ExtensionRegistry {
    /// An empty registry: no extension is loaded, so `require` answers for none.
    pub fn new() -> ExtensionRegistry {
        ExtensionRegistry::default()
    }

    /// Record that `namespace` is loaded and declares the object types `classes`.
    ///
    /// Refused when the namespace is empty, has an empty segment, is `gnoblin`, is
    /// already loaded, or lists no classes, and when a class name is empty or repeated.
    pub fn register_extension(&mut self, namespace: &str, classes: &[&str]) -> Result<(), RegistryError> {
        if namespace == CORE_NAMESPACE || namespace.is_empty() || namespace.split('.').any(str::is_empty) {
            return Err(RegistryError::InvalidNamespace(namespace.to_owned()));
        }
        if self.loaded.contains_key(namespace) {
            return Err(RegistryError::DuplicateNamespace(namespace.to_owned()));
        }
        if classes.is_empty() {
            return Err(RegistryError::NoClasses(namespace.to_owned()));
        }
        let mut declared = BTreeSet::new();
        for class in classes {
            if class.is_empty() || !declared.insert((*class).to_owned()) {
                return Err(RegistryError::InvalidClass {
                    namespace: namespace.to_owned(),
                    class: (*class).to_owned(),
                });
            }
        }
        self.loaded.insert(namespace.to_owned(), declared);
        Ok(())
    }

    /// Whether `namespace` is loaded.
    pub fn is_loaded(&self, namespace: &str) -> bool {
        self.loaded.contains_key(namespace)
    }

    /// The object types a loaded namespace declares, in name order, or `None` when the
    /// namespace is not loaded.
    pub fn classes(&self, namespace: &str) -> Option<impl Iterator<Item = &str>> {
        self.loaded
            .get(namespace)
            .map(|classes| classes.iter().map(String::as_str))
    }

    /// Every loaded namespace, in name order.
    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.loaded.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::{ExtensionRegistry, RegistryError};

    #[test]
    fn the_stock_registry_loads_no_extension() {
        let registry = ExtensionRegistry::new();
        assert!(!registry.is_loaded("example.ext"));
        assert_eq!(registry.namespaces().count(), 0);
    }

    #[test]
    fn a_registered_namespace_lists_its_classes_in_name_order() {
        let mut registry = ExtensionRegistry::new();
        registry
            .register_extension("example.ext", &["probe", "counter"])
            .expect("registers");
        assert!(registry.is_loaded("example.ext"));
        let classes: Vec<&str> = registry.classes("example.ext").expect("loaded").collect();
        assert_eq!(classes, ["counter", "probe"]);
        assert_eq!(registry.namespaces().collect::<Vec<_>>(), ["example.ext"]);
    }

    #[test]
    fn malformed_or_reserved_namespaces_are_refused() {
        let mut registry = ExtensionRegistry::new();
        for namespace in ["", "gnoblin", "example.", ".ext", "example..ext"] {
            assert_eq!(
                registry.register_extension(namespace, &["probe"]),
                Err(RegistryError::InvalidNamespace(namespace.to_owned())),
                "{namespace}"
            );
        }
        assert!(registry.namespaces().next().is_none());
    }

    #[test]
    fn a_namespace_loads_once_and_declares_at_least_one_class() {
        let mut registry = ExtensionRegistry::new();
        registry
            .register_extension("example.ext", &["probe"])
            .expect("registers");
        assert_eq!(
            registry.register_extension("example.ext", &["probe"]),
            Err(RegistryError::DuplicateNamespace("example.ext".to_owned()))
        );
        assert_eq!(
            registry.register_extension("other.ext", &[]),
            Err(RegistryError::NoClasses("other.ext".to_owned()))
        );
    }

    #[test]
    fn an_empty_or_repeated_class_is_refused_and_leaves_nothing_loaded() {
        let mut registry = ExtensionRegistry::new();
        assert_eq!(
            registry.register_extension("example.ext", &["probe", "probe"]),
            Err(RegistryError::InvalidClass {
                namespace: "example.ext".to_owned(),
                class: "probe".to_owned(),
            })
        );
        assert_eq!(
            registry.register_extension("example.ext", &[""]),
            Err(RegistryError::InvalidClass {
                namespace: "example.ext".to_owned(),
                class: String::new(),
            })
        );
        assert!(!registry.is_loaded("example.ext"));
    }
}
