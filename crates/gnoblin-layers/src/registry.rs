//! The registry: the single owner of every registered layer and its name.
//!
//! A name is a claim, and the registry is the only place claims are recorded. The
//! rules are simple to state and easy to break by accident, so each one is a
//! typed error and a test:
//!
//! 1. A name is unique. The first registrant owns it.
//! 2. Reserved names are for built-in providers only. An extension is refused
//!    before the taken check, so it cannot probe who owns a built-in name.
//! 3. Only the owner may remove a layer.
//!
//! The registry stores records in registration order. That order is the one
//! stable thing about the list before ordering exists, so [`LayerRegistry::list`]
//! is deterministic and a test can rely on it.

use crate::error::LayerError;
use crate::record::{LayerProvider, LayerProviderKind, LayerRecord};

/// Names the compositor reserves for its built-in providers.
///
/// An extension cannot register any of these, and cannot re-register one after a
/// built-in provider has released it. `surface_` is a prefix rule, applied in
/// [`is_reserved_name`], because the built-in surface kinds form a family.
const RESERVED_NAMES: &[&str] = &["portals", "alerts", "window_menu", "lock"];

/// The prefix that reserves a family of built-in surface names.
///
/// Only built-in kinds use this prefix, so any name starting with it is reserved
/// as a whole rather than one name at a time. A new built-in surface kind therefore
/// needs no change here, and an extension cannot squat on a future one.
const RESERVED_PREFIX: &str = "surface_";

/// Whether `name` is reserved for built-in providers.
///
/// A reserved name is refused to an extension. A built-in provider may still use
/// it; the built-in names are reserved against extensions, not against the
/// compositor itself.
pub fn is_reserved_name(name: &str) -> bool {
    RESERVED_NAMES.contains(&name) || name.starts_with(RESERVED_PREFIX)
}

/// The owner of every registered layer, in registration order.
///
/// The registry holds plain records and enforces the name rules. It knows nothing
/// about bands, scopes or the compositor, so it can be tested as a value. Later
/// phases add ordering and limits on top; they do not change how names are owned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerRegistry {
    records: Vec<LayerRecord>,
}

impl LayerRegistry {
    /// An empty registry with no layers and no owned names.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a layer, taking ownership of its name.
    ///
    /// The checks run in a fixed order, and the first failure is returned:
    /// the name must be non-empty, an extension may not use a reserved name, and
    /// the name must not already be registered. The order matters: a reserved
    /// name is refused before the taken check, so an extension cannot learn
    /// whether a built-in name is in use.
    pub fn register(&mut self, record: LayerRecord) -> Result<(), LayerError> {
        if record.name.is_empty() {
            return Err(LayerError::EmptyName);
        }
        if record.provider.kind == LayerProviderKind::Extension && is_reserved_name(&record.name) {
            return Err(LayerError::ReservedName { name: record.name });
        }
        if self.get(&record.name).is_some() {
            return Err(LayerError::NameTaken { name: record.name });
        }
        self.records.push(record);
        Ok(())
    }

    /// Removes a layer, returning its record. Only the owner may remove it.
    ///
    /// The owner is compared by [`gnoblin_types::OwnerId`], which the compositor assigns per
    /// connection, so a provider cannot remove another provider's layer by
    /// naming it. A missing name and a name owned by someone else are different
    /// errors, so the caller can tell a typo from an access refusal.
    pub fn remove(&mut self, name: &str, caller: LayerProvider) -> Result<LayerRecord, LayerError> {
        let index = self
            .records
            .iter()
            .position(|record| record.name == name)
            .ok_or_else(|| LayerError::NotFound { name: name.to_owned() })?;
        let owner = self.records[index].provider;
        if owner.id != caller.id {
            return Err(LayerError::NotOwner {
                name: name.to_owned(),
                owner: owner.id,
                caller: caller.id,
            });
        }
        Ok(self.records.remove(index))
    }

    /// The record registered under `name`, if any.
    pub fn get(&self, name: &str) -> Option<&LayerRecord> {
        self.records.iter().find(|record| record.name == name)
    }

    /// Every registered layer, in the order they were registered.
    pub fn list(&self) -> impl Iterator<Item = &LayerRecord> {
        self.records.iter()
    }

    /// The number of registered layers.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no layers are registered.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}
