//! The registry: the one place layers are registered, looked up and ended.
//!
//! The registry owns every live layer and every rule about who may register
//! what. A provider never builds a full name itself. It hands over a
//! [`LayerSpec`] with a local part, and the registry prefixes the namespace from
//! the [`LayerProvider`]. That is why two providers cannot collide: the prefix is not
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
use crate::name::{LayerName, LayerNameError};
use crate::provider::LayerProvider;
use crate::scope::{LayerScope, Order};
use crate::spec::LayerSpec;
use crate::state::{EndReason, LayerState};
use gnoblin_types::{OutputId, WindowId};

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
    BadProviderId(LayerNameError),
    /// The requested name is outside the name grammar, so it cannot be a layer.
    BadName(LayerNameError),
    /// The requested full name names a namespace other than the provider's. The
    /// provider comes from the authenticated connection, so the request cannot
    /// choose another one; `attributed` is the namespace that was enforced.
    ProviderMismatch {
        /// The name the caller asked for.
        requested: String,
        /// The namespace the compositor attributed to the caller.
        attributed: String,
    },
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
            RegisterError::BadName(e) => write!(f, "bad layer name: {e}"),
            RegisterError::ProviderMismatch { requested, attributed } => write!(
                f,
                "layer name {requested} is outside the attributed namespace {attributed}"
            ),
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

/// Why [`LayerRegistry::set_secure`] refused a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SecureError {
    /// No live layer has that handle, so there is nothing to change.
    NotFound,
    /// The caller is not the provider that registered the layer. Only the owner
    /// may change its `secure` flag.
    NotOwner,
    /// The layer is in the lock band, which is always secure.
    LockIsFixed,
}

impl fmt::Display for SecureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("no live layer has that handle"),
            Self::NotOwner => f.write_str("only the owning provider may change a layer's secure flag"),
            Self::LockIsFixed => f.write_str("a lock-band layer is always secure"),
        }
    }
}

impl std::error::Error for SecureError {}

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
    pub provider: LayerProvider,
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
    /// The number of live layers one provider may hold, for any provider without
    /// its own entry in `provider_limits`.
    limit: usize,
    /// Per-provider limits that differ from `limit`. A provider with an entry here
    /// may hold up to that many live layers instead. Only the compositor's own
    /// window mirror uses one, because its count is the window cap.
    provider_limits: BTreeMap<LayerProvider, usize>,
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
    /// The per-name secure override. A compositor config may force a layer
    /// secure or insecure by its full name; `None` means the layer's own
    /// `spec.secure` decides. Kept separate from the record so an override set
    /// before a layer registers still applies.
    secure_overrides: BTreeMap<LayerName, bool>,
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
            provider_limits: BTreeMap::new(),
            next_handle: 1,
            next_sequence: 1,
            by_handle: BTreeMap::new(),
            by_name: BTreeMap::new(),
            secure_overrides: BTreeMap::new(),
        }
    }

    /// Gives `provider` its own limit instead of the registry default. Other
    /// providers keep the default, so a built-in can hold more than an extension
    /// without raising the bound for extensions.
    #[must_use]
    pub fn with_provider_limit(mut self, provider: LayerProvider, limit: usize) -> Self {
        self.provider_limits.insert(provider, limit);
        self
    }

    /// The default per-provider limit this registry enforces, for any provider
    /// without its own limit.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// The number of live layers `provider` may hold: its own limit if it has one,
    /// otherwise the registry default.
    #[must_use]
    pub fn limit_for(&self, provider: &LayerProvider) -> usize {
        self.provider_limits.get(provider).copied().unwrap_or(self.limit)
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
    pub fn register(&mut self, provider: &LayerProvider, spec: LayerSpec) -> Result<LayerHandle, RegisterError> {
        // Rule 1: the reserved id. The namespace check below would accept it,
        // because `gnoblin` is a legal grammar word, so it is refused first.
        if matches!(provider, LayerProvider::Extension(id) if id == crate::name::GNOBLIN_NAMESPACE) {
            return Err(RegisterError::ProviderIdReserved);
        }
        let namespace = provider.namespace().map_err(RegisterError::BadProviderId)?;

        // Rule 2: the band ceiling for this provider's class.
        if !band_allowed(provider, spec.band) {
            return Err(RegisterError::BandNotAllowed(spec.band));
        }

        // Rules 3 and 4: the scope and the order must agree.
        if let LayerScope::Window(None) = spec.scope {
            return Err(RegisterError::WindowScopeNeedsWindow);
        }
        if spec.order == Order::AboveTarget && !matches!(spec.scope, LayerScope::Window(_)) {
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
        let limit = self.limit_for(provider);
        if held >= limit {
            return Err(RegisterError::LimitReached { limit });
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
    pub fn end_provider(&mut self, provider: &LayerProvider, reason: EndReason) -> Vec<LayerRecord> {
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

    /// Ends every live layer whose scope is the window `window`, with
    /// [`EndReason::TargetGone`], and returns them. Called when a window closes.
    pub fn end_target(&mut self, window: WindowId) -> Vec<LayerRecord> {
        self.end_matching(
            |scope| *scope == LayerScope::Window(Some(window)),
            EndReason::TargetGone,
        )
    }

    /// Ends every live layer attached to `output`, with [`EndReason::OutputGone`],
    /// and returns them. Called when an output is removed. Window-scoped layers
    /// on that output are not touched here; they follow their window.
    pub fn end_output(&mut self, output: OutputId) -> Vec<LayerRecord> {
        self.end_matching(|scope| *scope == LayerScope::Output(output), EndReason::OutputGone)
    }

    /// Moves every live layer on `window` between `Mapped` and `Hidden`, so a
    /// minimised window hides its layers and a restored one shows them again.
    /// Layers already in the requested state are left alone. Returns how many
    /// changed.
    pub fn set_target_visible(&mut self, window: WindowId, visible: bool) -> usize {
        let wanted = if visible {
            LayerState::Mapped
        } else {
            LayerState::Hidden
        };
        let mut changed = 0;
        for record in self.by_handle.values_mut() {
            if record.spec.scope != LayerScope::Window(Some(window)) {
                continue;
            }
            if let Ok(next) = record.state.transition(wanted) {
                record.state = next;
                changed += 1;
            }
        }
        changed
    }

    /// Ends every live layer whose scope satisfies `matches`, in registration
    /// order. Shared by the follow functions so each one is a single rule.
    fn end_matching(&mut self, matches: impl Fn(&LayerScope) -> bool, reason: EndReason) -> Vec<LayerRecord> {
        let handles: Vec<LayerHandle> = self
            .by_handle
            .values()
            .filter(|record| matches(&record.spec.scope))
            .map(|record| record.handle)
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| self.end(handle, reason))
            .collect()
    }

    /// Sets the compositor config override for `name`, so the layer's effective
    /// secure value is `secure` whatever the provider asked for. Only the
    /// compositor may call this; the provider path has no caller for it.
    pub fn set_secure_override(&mut self, name: LayerName, secure: bool) {
        self.secure_overrides.insert(name, secure);
    }

    /// Removes the config override for `name`, returning the layer to its own
    /// `spec.secure`.
    pub fn clear_secure_override(&mut self, name: &LayerName) {
        self.secure_overrides.remove(name);
    }

    /// Whether the layer `record` is secure right now. The config override wins
    /// when set; otherwise the provider's `spec.secure` decides. A lock-band layer
    /// is always secure, whatever either says, because the lock surface must not
    /// leak.
    #[must_use]
    pub fn effective_secure(&self, record: &LayerRecord) -> bool {
        if record.spec.band == Band::Lock {
            return true;
        }
        self.secure_overrides
            .get(&record.name)
            .copied()
            .unwrap_or(record.spec.secure)
    }

    /// Changes the `secure` flag of a live layer on behalf of its owner.
    ///
    /// Only the provider that registered the layer may change it, so a second
    /// provider cannot flip a neighbour's layer. Returns the new effective value,
    /// or [`SecureError`] when the layer is missing, not owned by `provider`, or
    /// in the lock band (always secure).
    ///
    /// # Errors
    ///
    /// [`SecureError::NotFound`] when the handle is not live,
    /// [`SecureError::NotOwner`] when `provider` did not register it, and
    /// [`SecureError::LockIsFixed`] for a lock-band layer.
    pub fn set_secure(
        &mut self,
        provider: &LayerProvider,
        handle: LayerHandle,
        secure: bool,
    ) -> Result<bool, SecureError> {
        let record = self.by_handle.get_mut(&handle).ok_or(SecureError::NotFound)?;
        if &record.provider != provider {
            return Err(SecureError::NotOwner);
        }
        if record.spec.band == Band::Lock {
            return Err(SecureError::LockIsFixed);
        }
        record.spec.secure = secure;
        Ok(secure)
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
fn band_allowed(provider: &LayerProvider, band: Band) -> bool {
    if band.compositor_only() {
        return provider.is_compositor();
    }
    match provider {
        LayerProvider::Gnoblin(_) => true,
        LayerProvider::Extension(_) => band <= Band::Overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::band::Band;
    use crate::name::LocalName;
    use crate::scope::LayerScope;
    use gnoblin_types::{OutputId, WindowId};

    fn gnoblin() -> LayerProvider {
        LayerProvider::Gnoblin("gnoblin-wayland")
    }

    fn ext(id: &str) -> LayerProvider {
        LayerProvider::Extension(id.to_owned())
    }

    fn spec(local: &str, band: Band) -> LayerSpec {
        LayerSpec::new(
            LocalName::new(local).unwrap(),
            LayerScope::Output(OutputId::new(1)),
            band,
        )
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
        let compositor = LayerProvider::Gnoblin(crate::provider::COMPOSITOR_CRATE);
        reg.register(&compositor, spec("lock", Band::Lock)).unwrap();
    }

    #[test]
    fn window_scope_without_a_target_is_refused() {
        let mut reg = LayerRegistry::new();
        let mut s = spec("x", Band::Top);
        s.scope = LayerScope::Window(None);
        assert_eq!(reg.register(&ext("bar"), s), Err(RegisterError::WindowScopeNeedsWindow));
        let mut s = spec("y", Band::Top);
        s.scope = LayerScope::Window(Some(WindowId::new(7)));
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
    fn a_provider_limit_overrides_the_default_for_that_provider_only() {
        let mut reg = LayerRegistry::with_limit(2).with_provider_limit(gnoblin(), 4);
        assert_eq!(reg.limit_for(&gnoblin()), 4);
        assert_eq!(reg.limit_for(&ext("bar")), 2);
        for i in 0..4 {
            reg.register(&gnoblin(), spec(&format!("w{i}"), Band::Windows)).unwrap();
        }
        assert_eq!(
            reg.register(&gnoblin(), spec("w4", Band::Windows)),
            Err(RegisterError::LimitReached { limit: 4 })
        );
        // The override does not lift an extension's limit.
        reg.register(&ext("bar"), spec("a", Band::Top)).unwrap();
        reg.register(&ext("bar"), spec("b", Band::Top)).unwrap();
        assert_eq!(
            reg.register(&ext("bar"), spec("c", Band::Top)),
            Err(RegisterError::LimitReached { limit: 2 })
        );
    }

    #[test]
    fn an_extension_still_stops_at_the_default_limit() {
        let mut reg = LayerRegistry::new();
        for i in 0..DEFAULT_LIMIT {
            reg.register(&ext("bar"), spec(&format!("l{i}"), Band::Top)).unwrap();
        }
        assert_eq!(
            reg.register(&ext("bar"), spec("over", Band::Top)),
            Err(RegisterError::LimitReached { limit: DEFAULT_LIMIT })
        );
        assert_eq!(reg.len(), DEFAULT_LIMIT);
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

#[cfg(test)]
mod follow_tests {
    use super::*;
    use crate::name::LocalName;

    fn gnoblin() -> LayerProvider {
        LayerProvider::Gnoblin("gnoblin-wayland")
    }

    fn on_window(local: &str, window: WindowId) -> LayerSpec {
        let mut spec = LayerSpec::new(
            LocalName::new(local).unwrap(),
            LayerScope::Window(Some(window)),
            Band::Windows,
        );
        spec.order = Order::Default;
        spec
    }

    fn on_output(local: &str, output: OutputId) -> LayerSpec {
        LayerSpec::new(LocalName::new(local).unwrap(), LayerScope::Output(output), Band::Top)
    }

    #[test]
    fn closing_a_window_ends_its_layers_with_target_gone() {
        let window = WindowId::new(1);
        let mut reg = LayerRegistry::new();
        reg.register(&gnoblin(), on_window("a", window)).unwrap();
        reg.register(&gnoblin(), on_window("b", WindowId::new(2))).unwrap();
        let ended = reg.end_target(window);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].state, LayerState::Ended(EndReason::TargetGone));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn removing_an_output_ends_its_layers_and_leaves_window_layers() {
        let output = OutputId::new(1);
        let mut reg = LayerRegistry::new();
        reg.register(&gnoblin(), on_output("bar", output)).unwrap();
        reg.register(&gnoblin(), on_window("w", WindowId::new(1))).unwrap();
        let ended = reg.end_output(output);
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].state, LayerState::Ended(EndReason::OutputGone));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn hiding_and_showing_a_window_moves_only_its_layers() {
        let window = WindowId::new(1);
        let mut reg = LayerRegistry::new();
        let h = reg.register(&gnoblin(), on_window("a", window)).unwrap();
        let other = reg.register(&gnoblin(), on_window("b", WindowId::new(2))).unwrap();
        assert_eq!(reg.set_target_visible(window, false), 1);
        assert_eq!(reg.get(h).unwrap().state, LayerState::Hidden);
        assert_eq!(reg.get(other).unwrap().state, LayerState::Mapped);
        // Repeating the change is a no-op, so the count is zero.
        assert_eq!(reg.set_target_visible(window, false), 0);
        assert_eq!(reg.set_target_visible(window, true), 1);
        assert_eq!(reg.get(h).unwrap().state, LayerState::Mapped);
    }

    #[test]
    fn transitions_follow_the_table() {
        use crate::state::TransitionError;
        assert_eq!(
            LayerState::Mapped.transition(LayerState::Hidden),
            Ok(LayerState::Hidden)
        );
        assert_eq!(
            LayerState::Hidden.transition(LayerState::Mapped),
            Ok(LayerState::Mapped)
        );
        assert_eq!(
            LayerState::Mapped.transition(LayerState::Mapped),
            Err(TransitionError::NoChange)
        );
        let ended = LayerState::Ended(EndReason::Removed);
        assert_eq!(
            ended.transition(LayerState::Mapped),
            Err(TransitionError::EndedIsTerminal)
        );
    }
}

#[cfg(test)]
mod secure_tests {
    use super::*;
    use crate::name::LocalName;

    fn spec(local: &str, band: Band, secure: bool) -> LayerSpec {
        let mut s = LayerSpec::new(
            LocalName::new(local).unwrap(),
            LayerScope::Output(OutputId::new(1)),
            band,
        );
        s.secure = secure;
        s
    }

    #[test]
    fn override_beats_the_spec_and_clears_back() {
        let provider = LayerProvider::Gnoblin("gnoblin-wayland");
        let mut reg = LayerRegistry::new();
        let h = reg.register(&provider, spec("a", Band::Top, false)).unwrap();
        let name = reg.get(h).unwrap().name.clone();
        assert!(!reg.effective_secure(reg.get(h).unwrap()));
        reg.set_secure_override(name.clone(), true);
        assert!(reg.effective_secure(reg.get(h).unwrap()));
        reg.clear_secure_override(&name);
        assert!(!reg.effective_secure(reg.get(h).unwrap()));
    }

    #[test]
    fn lock_band_is_always_secure() {
        let provider = LayerProvider::Gnoblin(crate::provider::COMPOSITOR_CRATE);
        let mut reg = LayerRegistry::new();
        let h = reg.register(&provider, spec("lock", Band::Lock, false)).unwrap();
        let name = reg.get(h).unwrap().name.clone();
        reg.set_secure_override(name, false);
        assert!(reg.effective_secure(reg.get(h).unwrap()));
    }

    #[test]
    fn only_the_owner_may_set_secure() {
        let owner = LayerProvider::Extension("alice".into());
        let other = LayerProvider::Extension("bob".into());
        let mut reg = LayerRegistry::new();
        let h = reg.register(&owner, spec("a", Band::Top, false)).unwrap();
        assert_eq!(reg.set_secure(&other, h, true), Err(SecureError::NotOwner));
        assert_eq!(reg.set_secure(&owner, h, true), Ok(true));
        assert!(reg.get(h).unwrap().spec.secure);
    }

    #[test]
    fn set_secure_refuses_lock_and_missing_handles() {
        let owner = LayerProvider::Gnoblin(crate::provider::COMPOSITOR_CRATE);
        let mut reg = LayerRegistry::new();
        let lock = reg.register(&owner, spec("lock", Band::Lock, true)).unwrap();
        assert_eq!(reg.set_secure(&owner, lock, false), Err(SecureError::LockIsFixed));
        reg.end(lock, EndReason::Removed);
        assert_eq!(reg.set_secure(&owner, lock, true), Err(SecureError::NotFound));
    }
}
