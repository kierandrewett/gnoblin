//! The closed band set: the ordering substrate of the desktop stack.
//!
//! A band is a position in the stack, not an identity. Two layers in the same
//! band are ordered by their `order` relation (a later phase), and a layer that
//! names no band cannot exist. The set is closed on purpose: a configuration file
//! or an extension can choose a band, but it cannot invent one, so the compositor
//! always knows where every surface can sit.

/// One band of the stack, from the bottom to the top.
///
/// The variants are declared bottom to top, so the derived `Ord` is the stacking
/// order: `LayerBand::Background < LayerBand::Cursor`. Code that needs "above"
/// or "below" should compare bands with `<` and `>` instead of matching on the
/// variants, so a future reordering of the declarations is one edit, not many.
///
/// The first four bands are the four wlr layer-shell layers, in the same order,
/// so a layer-shell surface maps onto a band without a lookup table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerBand {
    /// The desktop wallpaper and anything that must sit behind every window.
    Background,
    /// Below windows but above the wallpaper, such as desktop widgets.
    Bottom,
    /// The band that holds ordinary application windows. Layers attached to a
    /// window sit above their target inside this band's ordering.
    Windows,
    /// Above windows: panels and docks.
    Top,
    /// Above panels: overlays that must stay visible, such as popups.
    Overlay,
    /// Compositor-owned system UI. Only the compositor registers here.
    System,
    /// The session lock. Only the compositor registers here, and nothing sits
    /// above it except the cursor.
    Lock,
    /// The cursor. Only the compositor registers here; it is always on top.
    Cursor,
}

impl LayerBand {
    /// Every band in stacking order, bottom to top.
    ///
    /// Use this when iterating the set, for example to report every band to a
    /// client. The array order is the same as the `Ord` order, and a test pins
    /// that, so a future edit that breaks it fails loudly.
    pub const ALL: [LayerBand; 8] = [
        LayerBand::Background,
        LayerBand::Bottom,
        LayerBand::Windows,
        LayerBand::Top,
        LayerBand::Overlay,
        LayerBand::System,
        LayerBand::Lock,
        LayerBand::Cursor,
    ];

    /// The band's name as it appears in configuration and in the wire protocol.
    ///
    /// The names are fixed: a configuration written today must mean the same band
    /// in every later release, so renaming a band is a protocol change.
    pub const fn as_str(self) -> &'static str {
        match self {
            LayerBand::Background => "background",
            LayerBand::Bottom => "bottom",
            LayerBand::Windows => "windows",
            LayerBand::Top => "top",
            LayerBand::Overlay => "overlay",
            LayerBand::System => "system",
            LayerBand::Lock => "lock",
            LayerBand::Cursor => "cursor",
        }
    }
}
