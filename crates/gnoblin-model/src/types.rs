//! Where an object type sits in the schema.

use std::collections::BTreeMap;
use std::fmt;

use gnoblin_schema::{Catalog, ObjectType};

/// An object type's namespace and name.
///
/// The namespace is part of the identity because the schema is namespaced:
/// `gnoblin.window` and an extension's `bsm.greeter.prompt` are different types
/// with different members, even if both were called `prompt`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeKey {
    namespace: String,
    name: String,
}

impl TypeKey {
    /// A type in any namespace.
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> TypeKey {
        TypeKey {
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    /// A type in the compositor's own `gnoblin` namespace, which is where every
    /// object the compositor itself creates lives.
    pub fn gnoblin(name: impl Into<String>) -> TypeKey {
        TypeKey::new("gnoblin", name)
    }

    /// The schema namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The object type's name inside its namespace.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for TypeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.namespace, self.name)
    }
}

/// The object types a graph was built from, keyed by namespace and name.
#[derive(Debug)]
pub(crate) struct Types {
    by_key: BTreeMap<TypeKey, ObjectType>,
}

impl Types {
    /// Index every object type in a validated catalog.
    pub(crate) fn index(catalog: &Catalog) -> Types {
        let mut by_key = BTreeMap::new();
        for namespace in catalog.namespaces() {
            for object in catalog.objects(&namespace) {
                by_key.insert(TypeKey::new(namespace.as_str(), object.name.clone()), object.clone());
            }
        }
        Types { by_key }
    }

    /// The declaration for one object type.
    pub(crate) fn get(&self, key: &TypeKey) -> Option<&ObjectType> {
        self.by_key.get(key)
    }
}
