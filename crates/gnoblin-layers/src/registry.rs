//! The registry: the one place layers are registered, looked up and ended.
//!
//! The registry owns every live layer and every rule about who may register
//! what. A provider never builds a full name itself. It hands over a
//! [`LayerSpec`] with a local part, and the registry prefixes the namespace from
//! the [`Provider`]. That is why two providers cannot collide: the prefix is not
//! a choice the caller makes.
//!
//! # What the registry checks, in order
//!
//! 1. The provider id is not the reserved `gnoblin` for an extension
//!    ([`RegisterError::ProviderIdReserved`]). Only built-ins use that namespace.
//! 2. The band is allowed for the provider's class ([`RegisterError::BandNotAllowed`]).
//!    Extensions stop at `Overlay`, other built-ins at `System`, and only the
//!    compositor may use `Lock` and `Cursor`.
//! 3. A window scope has a target ([`RegisterError::WindowScopeNeedsWindow`]).
//! 4. `Order::AboveTarget` is used only with a window scope
//!    ([`RegisterError::AboveTargetNeedsWindow`]).
//! 5. An `Above` or `Below` order names a live layer in the same band
//!    ([`RegisterError::OrderTargetMissing`], [`RegisterError::OrderTargetOtherBand`]).
//! 6. The full name is not already live ([`RegisterError::NameTaken`]).
//! 7. The provider is under its limit of live layers ([`RegisterError::LimitReached`]).
//!
//! The order of the checks is part of the contract. A caller that sends several
//! faults gets the first one, so the reason in a log is stable.
//!
//! # Handles
//!
//! [`LayerHandle`] is what a caller keeps. It is never reused, so a stale handle
//! names nothing rather than a later layer. A layer that ends releases its name,
//! but its handle stays dead for good.

use std::collections::BTreeMap;
use std::fmt;

use crate::band::Band;
use crate::name::{LayerName, NameError};
use crate::provider::Provider;
use crate::scope::{Order, Scope};
use crate::spec::LayerSpec;
use crate::state::{EndReason, LayerState};

/// The default number of live layers one provider may hold. It bounds a single
/// extension so it cannot fill the stack, and a built-in cannot leak layers
/// without anyone noticing.
pub const DEFAULT_LIMIT: usize = 32;

/// An opaque token for one registered layer.
///
/// The registry allocates handles from a counter that never goes back, so a
/// handle is never reused. The field is private: a caller can only obtain a handle
/// from [`LayerRegistry::register`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerHandle(u64);

impl fmt::Display for LayerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layer#{}", self.0)
    }
}

/// Why a registration was refused. Each variant is one rule, and callers match on
/// the variant, not on the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The extension id is `gnoblin`, which is reserved for the built-ins. This is
    /// the only reserved identifier.
    ProviderIdReserved,
    /// The extension id is outside the name grammar.
    BadProviderId(NameError),
    /// The band is not allowed for this provider's class. The `Band` says which.
    BandNotAllowed(Band),
    /// A window scope has no target window.
    WindowScopeNeedsWindow,
    /// `Order::AboveTarget` needs a window scope to name a target.
    AboveTargetNeedsWindow,
    /// An `Above` or `Below` order names no live layer, so there is nothing to sit
    /// next to.
    OrderTargetMissing(LayerName),
    /// An `Above` or `Below` order names a layer in a different band. The ordering
    /// is within one band, so the target must share it.
    OrderTargetOtherBand(LayerName),
    /// The provider already has a live layer with this full name.
    NameTaken(LayerName),
    /// The provider already holds its limit of live layers.
    LimitReached {
        /// The limit that was hit.
        limit: usize,
    },
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegisterError::ProviderIdReserved => {
                write!(f, "extension id `gnoblin` is reserved for the built-ins")
            }
            RegisterError::BadProviderId(e) => write!(f, "bad provider id: {e}"),
            RegisterError::BandNotAllowed(band) => {
                write!(f, "this provider may not register into {band:?}")
            }
            RegisterError::WindowScopeNeedsWindow => {
                write!(f, "a window scope needs a target window")
            }
            RegisterError::AboveTargetNeedsWindow => {
                write!(f, "`AboveTarget` needs a window scope")
            }
            RegisterError::OrderTargetMissing(name) => {
                write!(f, "order target {name} is not a live layer")
            }
            RegisterError::OrderTargetOtherBand(name) => {
                write!(f, "order target {name} is in another band")
            }
            RegisterError::NameTaken(name) => write!(f, "layer name {name} is already taken"),
            RegisterError::LimitReached { limit } => {
                write!(f, "provider already holds its limit of {limit} layers")
            }
        }
    }
}

impl std::error::Error for RegisterError {}

/// One live layer, as the registry holds it.
///
/// Records are returned by reference from the registry, so callers read them but
/// cannot change them. A change goes through the registry's methods, which keep
/// the name index and the per-provider counts in step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerRecord {
    /// The handle the provider was given.
    pub handle: LayerHandle,
    /// The full name, `<namespace>.<local>`.
    pub name: LayerName,
    /// Who registered the layer.
    pub provider: Provider,
    /// The spec the provider asked for.
    pub spec: LayerSpec,
    /// The lifecycle state.
    pub state: LayerState,
    /// The registration sequence, from a counter that only grows. Within a band
    /// the newest registration with the default order sits on top, so this decides
    /// paint order. The stack is built later, see `design/layers.md` (Ordering).
    pub sequence: u64,
}

/// Owns every live layer and the rules for registering them.
///
/// Live means registered and not yet ended. An ended layer is removed from the
/// registry, so its full name is free for a new registration.
#[derive(Debug)]
pub struct LayerRegistry {
    /// The number of live layers one provider may hold.
    limit: usize,
    /// The next handle to hand out. It only grows.
    next_handle: u64,
    /// The next registration sequence number. It only grows.
    next_sequence: u64,
    /// Live layers by handle. A `BTreeMap` keeps iteration in handle order, which
    /// is registration order.
    by_handle: BTreeMap<LayerHandle, LayerRecord>,
    /// The name index. It maps each live full name to its handle, so the
    /// `NameTaken` check is one lookup.
    by_name: BTreeMap<LayerName, LayerHandle>,
}

impl Default for LayerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerRegistry {
    /// An empty registry with the default per-provider limit, [`DEFAULT_LIMIT`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_LIMIT)
    }

    /// An empty registry with a chosen per-provider limit. The compositor uses
    /// the default; a test uses a small limit to reach `LimitReached` quickly.
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            next_handle: 1,
            next_sequence: 1,
            by_handle: BTreeMap::new(),
            by_name: BTreeMap::new(),
        }
    }

    /// The per-provider limit this registry enforces.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Registers a layer for `provider` and returns its handle.
    ///
    /// The full name is built here from the provider's namespace and the spec's
    /// local part. The checks run in the order listed in the module docs, and the
    /// first failure is returned. A refused registration changes nothing.
    ///
    /// # Errors
    ///
    /// Returns the [`RegisterError`] for the first rule the registration breaks.
    pub fn register(&mut self, provider: &Provider, spec: LayerSpec) -> Result<LayerHandle, RegisterError> {
        // Rule 1: the reserved id. The namespace check below would accept it,
        // because `gnoblin` is a legal grammar word, so it is refused first.
        if matches!(provider, Provider::Extension(id) if id == crate::name::GNOBLIN_NAMESPACE) {
            return Err(RegisterError::ProviderIdReserved);
        }
        let namespace = provider.namespace().map_err(RegisterError::BadProviderId)?;

        // Rule 2: the band ceiling for this provider's class.
        if !band_allowed(provider, spec.band) {
            return Err(RegisterError::BandNotAllowed(spec.band));
        }

        // Rules 3 and 4: the scope and the order must agree.
        if let Scope::Window(None) = spec.scope {
            return Err(RegisterError::WindowScopeNeedsWindow);
        }
        if spec.order == Order::AboveTarget && !matches!(spec.scope, Scope::Window(_)) {
            return Err(RegisterError::AboveTargetNeedsWindow);
        }

        // Rule 5: an `Above` or `Below` target must be live and in this band.
        if let Order::Above(target) | Order::Below(target) = &spec.order {
            let handle = self
                .by_name
                .get(target)
                .ok_or_else(|| RegisterError::OrderTargetMissing(target.clone()))?;
            let record = &self.by_handle[handle];
            if record.spec.band != spec.band {
                return Err(RegisterError::OrderTargetOtherBand(target.clone()));
            }
        }

        // Rule 6: the full name must be free. The name is built from the namespace
        // so two providers never meet here.
        let name = LayerName::new(namespace, spec.local.clone());
        if self.by_name.contains_key(&name) {
            return Err(RegisterError::NameTaken(name));
        }

        // Rule 7: the provider's live count must stay under the limit.
        let held = self
            .by_handle
            .values()
            .filter(|record| &record.provider == provider)
            .count();
        if held >= self.limit {
            return Err(RegisterError::LimitReached { limit: self.limit });
        }

        let handle = LayerHandle(self.next_handle);
        self.next_handle += 1;
        let sequence = self.next_sequence;
        self.next_sequence += 1;

        let record = LayerRecord {
            handle,
            name: name.clone(),
            provider: provider.clone(),
            spec,
            state: LayerState::Mapped,
            sequence,
        };
        self.by_handle.insert(handle, record);
        self.by_name.insert(name, handle);
        Ok(handle)
    }

    /// Ends one live layer, removing it from the registry, and returns its record
    /// with the state set to `Ended(reason)`. Returns `None` when the handle is not
    /// live, so ending a layer twice is a no-op the caller can detect.
    pub fn end(&mut self, handle: LayerHandle, reason: EndReason) -> Option<LayerRecord> {
        let mut record = self.by_handle.remove(&handle)?;
        self.by_name.remove(&record.name);
        record.state = LayerState::Ended(reason);
        Some(record)
    }

    /// Ends every live layer that `provider` owns, with `reason`, and returns them
    /// in registration order. Used when a provider disconnects or an extension is
    /// unloaded.
    pub fn end_provider(&mut self, provider: &Provider, reason: EndReason) -> Vec<LayerRecord> {
        let handles: Vec<LayerHandle> = self
            .by_handle
            .values()
            .filter(|record| &record.provider == provider)
            .map(|record| record.handle)
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| self.end(handle, reason))
            .collect()
    }

    /// The live record for `handle`, or `None` when the layer has ended.
    #[must_use]
    pub fn get(&self, handle: LayerHandle) -> Option<&LayerRecord> {
        self.by_handle.get(&handle)
    }

    /// The live record with this full name, or `None`.
    #[must_use]
    pub fn by_name(&self, name: &LayerName) -> Option<&LayerRecord> {
        self.by_name.get(name).and_then(|handle| self.get(*handle))
    }

    /// Every live record, in registration order.
    pub fn records(&self) -> impl Iterator<Item = &LayerRecord> {
        self.by_handle.values()
    }

    /// The number of live layers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_handle.len()
    }

    /// Whether no layer is live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_handle.is_empty()
    }
}

/// Whether `provider` may register into `band`.
///
/// The rule is per class, so a new band cannot be forgotten at a call site:
/// - the compositor may use every band;
/// - another built-in may use every band except the compositor-only ones;
/// - an extension may use up to `Overlay`, so it cannot sit above system UI.
fn band_allowed(provider: &Provider, band: Band) -> bool {
    if band.compositor_only() {
        return provider.is_compositor();
    }
    match provider {
        Provider::Gnoblin(_) => true,
        Provider::Extension(_) => band <= Band::Overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::band::Band;
    use crate::name::LocalName;
    use crate::scope::Scope;
    use gnoblin_types::{OutputId, WindowId};

    fn gnoblin() -> Provider {
        Provider::Gnoblin("gnoblin-wayland")
    }

    fn ext(id: &str) -> Provider {
        Provider::Extension(id.to_owned())
    }

    fn spec(local: &str, band: Band) -> LayerSpec {
        LayerSpec::new(LocalName::new(local).unwrap(), Scope::Output(OutputId::new(1)), band)
    }

    #[test]
    fn register_builds_the_full_name_from_the_provider() {
        let mut reg = LayerRegistry::new();
        let h = reg.register(&gnoblin(), spec("portals", Band::Overlay)).unwrap();
        let record = reg.get(h).unwrap();
        assert_eq!(record.name.to_string(), "gnoblin.portals");
        assert_eq!(record.state, LayerState::Mapped);
        let h2 = reg.register(&ext("bar"), spec("status", Band::Top)).unwrap();
        assert_eq!(reg.get(h2).unwrap().name.to_string(), "ext.bar.status");
    }

    #[test]
    fn same_local_name_is_allowed_across_providers() {
        // The prefix differs, so the names differ and nothing collides.
        let mut reg = LayerRegistry::new();
        reg.register(&gnoblin(), spec("alerts", Band::Top)).unwrap();
        reg.register(&ext("bar"), spec("alerts", Band::Top)).unwrap();
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn same_local_name_twice_in_one_provider_is_name_taken() {
        let mut reg = LayerRegistry::new();
        reg.register(&ext("bar"), spec("status", Band::Top)).unwrap();
        let err = reg.register(&ext("bar"), spec("status", Band::Top)).unwrap_err();
        assert!(matches!(err, RegisterError::NameTaken(_)));
    }

    #[test]
    fn reserved_extension_id_is_refused() {
        let mut reg = LayerRegistry::new();
        let err = reg.register(&ext("gnoblin"), spec("x", Band::Top)).unwrap_err();
        assert_eq!(err, RegisterError::ProviderIdReserved);
    }

    #[test]
    fn extensions_stop_at_overlay() {
        let mut reg = LayerRegistry::new();
        let err = reg.register(&ext("bar"), spec("x", Band::System)).unwrap_err();
        assert_eq!(err, RegisterError::BandNotAllowed(Band::System));
        // Overlay itself is the ceiling and is allowed.
        reg.register(&ext("bar"), spec("y", Band::Overlay)).unwrap();
    }

    #[test]
    fn only_the_compositor_uses_lock_and_cursor() {
        let mut reg = LayerRegistry::new();
        let err = reg.register(&gnoblin(), spec("x", Band::Lock)).unwrap_err();
        assert_eq!(err, RegisterError::BandNotAllowed(Band::Lock));
        let err = reg.register(&ext("bar"), spec("x", Band::Cursor)).unwrap_err();
        assert_eq!(err, RegisterError::BandNotAllowed(Band::Cursor));
        let compositor = Provider::Gnoblin(crate::provider::COMPOSITOR_CRATE);
        reg.register(&compositor, spec("lock", Band::Lock)).unwrap();
    }

    #[test]
    fn window_scope_without_a_target_is_refused() {
        let mut reg = LayerRegistry::new();
        let mut s = spec("x", Band::Top);
        s.scope = Scope::Window(None);
        assert_eq!(reg.register(&ext("bar"), s), Err(RegisterError::WindowScopeNeedsWindow));
        let mut s = spec("y", Band::Top);
        s.scope = Scope::Window(Some(WindowId::new(7)));
        reg.register(&ext("bar"), s).unwrap();
    }

    #[test]
    fn above_target_needs_a_window_scope() {
        let mut reg = LayerRegistry::new();
        let mut s = spec("x", Band::Top);
        s.order = Order::AboveTarget;
        assert_eq!(reg.register(&ext("bar"), s), Err(RegisterError::AboveTargetNeedsWindow));
    }

    #[test]
    fn order_target_must_be_live_and_in_the_same_band() {
        let mut reg = LayerRegistry::new();
        let base = reg.register(&gnoblin(), spec("base", Band::Top)).unwrap();
        let base_name = reg.get(base).unwrap().name.clone();

        let mut s = spec("above", Band::Top);
        s.order = Order::Above(base_name.clone());
        reg.register(&ext("bar"), s).unwrap();

        let mut s = spec("other", Band::Bottom);
        s.order = Order::Below(base_name.clone());
        assert_eq!(
            reg.register(&ext("bar"), s),
            Err(RegisterError::OrderTargetOtherBand(base_name))
        );

        let missing = LayerName::parse("gnoblin.nope").unwrap();
        let mut s = spec("z", Band::Top);
        s.order = Order::Above(missing.clone());
        assert_eq!(
            reg.register(&ext("bar"), s),
            Err(RegisterError::OrderTargetMissing(missing))
        );
    }

    #[test]
    fn the_limit_is_per_provider() {
        let mut reg = LayerRegistry::with_limit(2);
        reg.register(&ext("bar"), spec("a", Band::Top)).unwrap();
        reg.register(&ext("bar"), spec("b", Band::Top)).unwrap();
        assert_eq!(
            reg.register(&ext("bar"), spec("c", Band::Top)),
            Err(RegisterError::LimitReached { limit: 2 })
        );
        // Another provider still has room.
        reg.register(&ext("other"), spec("a", Band::Top)).unwrap();
    }

    #[test]
    fn a_refused_registration_changes_nothing() {
        let mut reg = LayerRegistry::with_limit(1);
        reg.register(&ext("bar"), spec("a", Band::Top)).unwrap();
        let _ = reg.register(&ext("bar"), spec("b", Band::Top));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn ending_releases_the_name_and_the_handle_stays_dead() {
        let mut reg = LayerRegistry::new();
        let h = reg.register(&ext("bar"), spec("a", Band::Top)).unwrap();
        let ended = reg.end(h, EndReason::Removed).unwrap();
        assert_eq!(ended.state, LayerState::Ended(EndReason::Removed));
        assert!(reg.get(h).is_none());
        assert!(reg.end(h, EndReason::Removed).is_none());
        // The name is free again, and the new handle is different.
        let h2 = reg.register(&ext("bar"), spec("a", Band::Top)).unwrap();
        assert_ne!(h, h2);
    }

    #[test]
    fn end_provider_ends_only_that_providers_layers() {
        let mut reg = LayerRegistry::new();
        reg.register(&ext("bar"), spec("a", Band::Top)).unwrap();
        reg.register(&ext("bar"), spec("b", Band::Top)).unwrap();
        reg.register(&ext("other"), spec("a", Band::Top)).unwrap();
        let ended = reg.end_provider(&ext("bar"), EndReason::ProviderGone);
        assert_eq!(ended.len(), 2);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn sequence_grows_in_registration_order() {
        let mut reg = LayerRegistry::new();
        let a = reg.register(&ext("bar"), spec("a", Band::Top)).unwrap();
        let b = reg.register(&ext("bar"), spec("b", Band::Top)).unwrap();
        assert!(reg.get(a).unwrap().sequence < reg.get(b).unwrap().sequence);
    }
}
