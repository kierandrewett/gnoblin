//! What a provider asks for when it registers a layer.
//!
//! A [`LayerSpec`] is plain data. It carries the local part of the name, not the
//! full name: the registry adds the namespace from the provider, so a spec cannot
//! choose its prefix. Every field other than the name, scope and band has a
//! default, set by [`LayerSpec::new`], and that default is the design's default.

use crate::band::Band;
use crate::name::LocalName;
use crate::scope::{ContentKind, EffectsPolicy, Focus, InputPolicy, LayerScope, LockBehaviour, Order};

/// A request to register one layer, before the registry checks it.
///
/// The `secure` flag is `false` by default. A provider sets `true` on a layer
/// whose pixels must never be captured. Only the owner, or a user override in
/// config, changes it later (see the invariants in `design/layers.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerSpec {
    /// The local part. The registry prefixes it with the provider's namespace.
    pub local: LocalName,
    /// The output or window the layer is attached to.
    pub scope: LayerScope,
    /// The band the layer sits in.
    pub band: Band,
    /// Where the layer sits inside its band.
    pub order: Order,
    /// Whether the layer takes pointer and keyboard input.
    pub input: InputPolicy,
    /// Whether the layer may take keyboard focus.
    pub focus: Focus,
    /// Whether the layer's pixels are kept out of every capture.
    pub secure: bool,
    /// What happens to the layer on lock.
    pub lock: LockBehaviour,
    /// Whether compositor effects reach the layer.
    pub effects: EffectsPolicy,
    /// What the layer paints.
    pub content: ContentKind,
}

impl LayerSpec {
    /// A spec with the design's defaults for everything but the three required
    /// fields: `Order::Default`, `Passive`, `Focus::None`, `secure = false`,
    /// `Hide`, `Inherit` and `ProviderSurface`.
    ///
    /// Each default is the safe choice: a new layer takes no input, no focus and
    /// no capture, and is removed on lock.
    #[must_use]
    pub fn new(local: LocalName, scope: LayerScope, band: Band) -> Self {
        Self {
            local,
            scope,
            band,
            order: Order::Default,
            input: InputPolicy::Passive,
            focus: Focus::None,
            secure: false,
            lock: LockBehaviour::Hide,
            effects: EffectsPolicy::Inherit,
            content: ContentKind::ProviderSurface,
        }
    }
}
