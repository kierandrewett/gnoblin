//! The extension namespaces a host has loaded.
//!
//! The runtime role fills this registry from the last accepted configuration
//! (`gnoblin_config::load_extensions`) before it runs the configuration. `require`
//! answers for a namespace only when it is both loaded here and granted to this
//! connection by the compositor (`gnoblin_client::Client::granted_extensions`). A loaded
//! namespace that is not granted is refused by name; a namespace that is not loaded is
//! an unknown module.

use std::collections::BTreeMap;
use std::fmt;

use gnoblin_schema::Schema;

/// The namespace the compositor's own object types live under. An extension cannot
/// claim it, because `gnoblin` is not something `require` returns.
const CORE_NAMESPACE: &str = "gnoblin";

/// The extension namespaces loaded into a host, each with its full schema.
#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    loaded: BTreeMap<String, Schema>,
}

/// Why a namespace was refused by the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The namespace is the core namespace, or is already loaded.
    InvalidNamespace(String),
    /// The namespace is already loaded.
    DuplicateNamespace(String),
    /// The namespace declares no object types, so there is nothing to bind.
    NoClasses(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::InvalidNamespace(name) => {
                write!(formatter, "`{name}` is not an extension namespace it may load")
            }
            RegistryError::DuplicateNamespace(name) => write!(formatter, "namespace `{name}` is already loaded"),
            RegistryError::NoClasses(name) => {
                write!(
                    formatter,
                    "namespace `{name}` declares no object types, so there is nothing to bind"
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

    /// Record that the schema's namespace is loaded. Refused when the namespace is the
    /// core namespace, is already loaded, or declares no object types.
    pub fn register_extension(&mut self, schema: Schema) -> Result<(), RegistryError> {
        let name = schema.namespace.as_str().to_owned();
        if name == CORE_NAMESPACE {
            return Err(RegistryError::InvalidNamespace(name));
        }
        if self.loaded.contains_key(&name) {
            return Err(RegistryError::DuplicateNamespace(name));
        }
        if schema.objects.is_empty() {
            return Err(RegistryError::NoClasses(name));
        }
        self.loaded.insert(name, schema);
        Ok(())
    }

    /// Whether `namespace` is loaded.
    pub fn is_loaded(&self, namespace: &str) -> bool {
        self.loaded.contains_key(namespace)
    }

    /// The schema of a loaded namespace, or `None` when it is not loaded.
    pub fn schema(&self, namespace: &str) -> Option<&Schema> {
        self.loaded.get(namespace)
    }

    /// Every loaded namespace with its schema, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Schema)> {
        self.loaded.iter().map(|(name, schema)| (name.as_str(), schema))
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_schema::parse_schema;

    use super::{ExtensionRegistry, RegistryError};

    fn schema(text: &str) -> gnoblin_schema::Schema {
        parse_schema("test.toml", text).expect("the fixture parses")
    }

    const ONE_OBJECT: &str =
        "namespace = \"demo.ext\"\nversion = \"0.1.0\"\n\n[[object]]\nname = \"thing\"\nsince = \"0.1.0\"\n";

    #[test]
    fn a_namespace_registers_once_with_its_schema() {
        let mut registry = ExtensionRegistry::new();
        registry.register_extension(schema(ONE_OBJECT)).expect("registers");
        assert!(registry.is_loaded("demo.ext"));
        assert_eq!(registry.schema("demo.ext").expect("loaded").objects.len(), 1);
        assert_eq!(
            registry.register_extension(schema(ONE_OBJECT)),
            Err(RegistryError::DuplicateNamespace("demo.ext".to_owned()))
        );
    }

    #[test]
    fn the_core_namespace_and_empty_namespaces_are_refused() {
        let mut registry = ExtensionRegistry::new();
        let empty = "namespace = \"demo.empty\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            registry.register_extension(schema(empty)),
            Err(RegistryError::NoClasses("demo.empty".to_owned()))
        );
        assert!(!registry.is_loaded("demo.empty"));
    }
}
