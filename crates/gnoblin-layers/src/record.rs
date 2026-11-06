//! The data a provider registers: a [`LayerRecord`] and the enums it is made of.
//!
//! A record is plain data. Building one does not check anything, because the
//! rules that matter (name uniqueness, reserved names) depend on what else is
//! registered, and only the registry knows that. A record that the registry
//! accepts is the one the compositor will act on.

use gnoblin_types::{OutputId, OwnerId, WindowId};

use crate::band::LayerBand;

/// Whether a provider is a built-in part of the compositor or an extension.
///
/// The kind decides which names a provider may claim. Built-in providers may
/// claim the reserved names; an extension may not. The kind is set by the
/// compositor when it creates the provider, never by the provider itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerProviderKind {
    /// Part of the compositor or one of its shipped services, such as the portal
    /// or the lock screen.
    Builtin,
    /// A Lua extension loaded from the user's configuration.
    Extension,
}

/// The identity of the provider that registered a layer.
///
/// `id` is the connection's [`OwnerId`], assigned by the compositor, so a provider
/// cannot choose to be another one. The `kind` travels with the identity so the
/// registry can apply the name rules without a second lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerProvider {
    /// The connection that owns this provider's layers.
    pub id: OwnerId,
    /// Whether the provider is built in or an extension.
    pub kind: LayerProviderKind,
}

/// What a layer is attached to, and so what its lifetime follows.
///
/// An output-scoped layer lives as long as its output. A window-scoped layer
/// follows its target window: it moves, minimises and changes workspace with it,
/// and ends when the window closes. The follow rules live in a later phase; this
/// type only records which target applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerScope {
    /// Attached to one output. The layer is shown on that output only.
    Output(OutputId),
    /// Attached to one window. The layer tracks that window.
    Window(WindowId),
}

/// How a layer takes pointer and keyboard input.
///
/// The variants are ordered from least to most intrusive. Only `Passive` is the
/// default for a provider that does not ask for input; the others are the modal
/// forms that stop input reaching what is underneath them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayerInput {
    /// The layer never takes input. Pointer and keys pass to whatever is under it.
    /// This is the default: a layer that does not say otherwise is passive.
    #[default]
    Passive,
    /// While the layer is up, input is limited to its target window.
    ModalToTarget,
    /// While the layer is up, input is limited to its output.
    ModalToOutput,
    /// The layer takes all input until it is dismissed. Only a built-in provider
    /// may use this, and the rule is enforced in a later phase.
    Grab,
}

/// What the layer's pixels come from.
///
/// A provider either supplies its own surface, or asks the compositor to paint a
/// named element. Painting is used for elements the compositor must draw itself,
/// such as alerts, so the content cannot be spoofed by a provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerContent {
    /// A Wayland surface the provider owns and draws into.
    Surface,
    /// An element the compositor paints itself. The provider chooses only which
    /// element; it does not supply pixels.
    CompositorPainted,
}

/// One registered layer: the data the registry stores and the compositor reads.
///
/// Every field is public because the record is plain data. The invariants that
/// matter are checked when the record is registered, not when it is read, and the
/// registry gives out only shared references, so a record cannot change after it
/// is accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerRecord {
    /// The unique name of the layer. The first provider to register a name owns
    /// it; no other provider can register, re-register or remove it.
    pub name: String,
    /// The provider that registered the layer. Only this provider may remove it.
    pub provider: LayerProvider,
    /// The output or window the layer is attached to.
    pub scope: LayerScope,
    /// The band the layer sits in.
    pub band: LayerBand,
    /// How the layer takes input. Defaults to [`LayerInput::Passive`] in
    /// [`LayerRecord::new`].
    pub input: LayerInput,
    /// Whether the layer's pixels may be captured. Defaults to `false`, so
    /// content is hidden from screenshots and recordings unless its owner or the
    /// user says otherwise. The rules for changing it arrive in a later phase.
    pub secure: bool,
    /// Where the pixels come from.
    pub content: LayerContent,
}

impl LayerRecord {
    /// Builds a record with the safe defaults: passive input and `secure` false.
    ///
    /// The defaults are the safe choice for a record nobody has reviewed. Passive
    /// input cannot trap the user, and a `secure` of false is the only value a
    /// provider can set without the owner or the user's consent. A caller that
    /// wants anything else sets the field after construction, and the registry
    /// checks it.
    pub fn new(
        name: impl Into<String>,
        provider: LayerProvider,
        scope: LayerScope,
        band: LayerBand,
        content: LayerContent,
    ) -> Self {
        Self {
            name: name.into(),
            provider,
            scope,
            band,
            input: LayerInput::Passive,
            secure: false,
            content,
        }
    }
}
