//! The small enums a layer spec is built from.
//!
//! Each type is one field of a layer's contract in `design/layers.md`. They are
//! plain values: this module decides nothing. The rules that use them live in the
//! registry and in the follow functions.

use gnoblin_types::{OutputId, WindowId};

use crate::name::LayerName;

/// What a layer is attached to.
///
/// `Output` follows its output and ends when the output is removed. `Window`
/// follows its target window through move, minimise and workspace change, and
/// ends when the window closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Attached to an output.
    Output(OutputId),
    /// Attached to a window. `None` names no target yet, and the registry refuses
    /// it as `WindowScopeNeedsWindow`. The `Option` is the typed form of "no
    /// window": there is no sentinel id, so a valid window can never be confused
    /// with a missing one.
    Window(Option<WindowId>),
}

/// Where a layer sits inside its band, relative to other layers.
///
/// The default is the newest claim on top of its band. `Above` and `Below` name
/// a full layer name, which may belong to another provider. The band ceiling
/// still applies, so naming a layer cannot lift a layer into a band it may not use.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Order {
    /// The newest registration sits on top of its band.
    #[default]
    Default,
    /// Directly above the target window. Only meaningful for window scope.
    AboveTarget,
    /// Directly above the named layer, in the same band.
    Above(LayerName),
    /// Directly below the named layer, in the same band.
    Below(LayerName),
}

/// Whether a layer takes pointer and keyboard input.
///
/// The default is `Passive`, which takes no input and lets clicks pass through.
/// The other policies are grants. An extension may request only `Passive`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum InputPolicy {
    /// Takes no input. Clicks pass through to whatever is under the layer.
    #[default]
    Passive,
    /// While mapped, the target window gets no key or button. Other windows still do.
    ModalToTarget,
    /// While mapped, no client on the output gets input.
    ModalToOutput,
    /// Takes the pointer and keyboard until dismissed. A click outside is consumed.
    Grab,
}

/// Whether a layer may take keyboard focus.
///
/// The default is `None`. A layer that is not asked for focus must not take it,
/// so a bar or an overlay cannot steal the keyboard from the window the user is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Focus {
    /// The layer may take keyboard focus when shown.
    Takes,
    /// The layer never takes keyboard focus.
    #[default]
    None,
}

/// What happens to a layer when the session locks.
///
/// Gnoblin providers only. The default is `Hide`: a layer is removed on lock
/// unless its provider asks to keep it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum LockBehaviour {
    /// Removed when the session locks.
    #[default]
    Hide,
    /// Stays mapped while the session is locked.
    Keep,
}

/// Whether the compositor's effects (such as window transforms) reach a layer.
///
/// The default is `Inherit`, so a layer shares the effects of the target it
/// follows. `None` opts a layer out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum EffectsPolicy {
    /// Effects of the target or output apply to the layer.
    #[default]
    Inherit,
    /// Effects never apply to the layer.
    None,
}

/// What a layer paints.
///
/// `ProviderSurface` is a surface the provider owns. `Painter` names a painter
/// the compositor implements, for example `alerts`, so the compositor draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ContentKind {
    /// A surface the provider supplies.
    #[default]
    ProviderSurface,
    /// A named compositor painter.
    Painter(&'static str),
}
