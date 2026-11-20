//! What a layer-shell surface publishes of itself, and what a shell may write on its node.

use gnoblin_types::{Insets, Rect};

/// The layers of the layer-shell protocol, bottom first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerBand {
    /// The bottom of everything: a wallpaper.
    Background,
    /// Above the background and under the windows: a desktop widget or an icon grid.
    Bottom,
    /// Above the windows: a bar, a dock.
    Top,
    /// Above everything the protocol can draw: a lock prompt, a launcher.
    Overlay,
}

impl LayerBand {
    /// The layer's name as the protocol and the node's `layer` property give it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::Overlay => "overlay",
        }
    }

    /// Whether the layer is drawn above the windows.
    #[must_use]
    pub const fn above_windows(self) -> bool {
        matches!(self, Self::Top | Self::Overlay)
    }
}

/// What a layer-shell surface publishes of itself to its node: what the client asked for and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerNodeInfo {
    /// The namespace the client named.
    pub namespace: String,
    /// The layer the client asked for.
    pub layer: LayerBand,
    /// The anchored edges, among `top`, `bottom`, `left` and `right`.
    pub anchor: Vec<String>,
    /// The margins the client asked for: top, right, bottom, left.
    pub margin: [f64; 4],
    /// The size the client asked for: width, height.
    pub requested: [f64; 2],
    /// The exclusive zone the client asked for, as the protocol states it.
    pub zone: f64,
    /// Whether the surface has committed a buffer.
    pub mapped: bool,
}

/// What the shell wrote on a layer surface's node while it places the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerOverride {
    /// The rectangle the shell wants the surface at, on the output plane. A width or height of 0 is left to the protocol.
    pub rect: Rect,
    /// The insets the shell wants the surface to reserve instead of its own exclusive zone's strip, when it wrote four.
    pub reserve: Option<Insets>,
}
