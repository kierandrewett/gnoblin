//! The kind of a layout, and what a window's placement means under it
//! (design/rust-compositor.md, D25).
//!
//! The layout of a workspace is the authority on whether its windows are tiled.
//! Every layout declares a [`LayoutKind`]; a window's [`WindowPlacement`] says
//! whether the layout assigned its geometry or the window keeps its own.

/// The kind a layout declares. It decides who owns window geometry and which
/// standard commands the host accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutKind {
    /// The layout gives initial placement only. Every window is [`WindowPlacement::Free`].
    Floating,
    /// The layout assigns every managed window a rectangle. Windows the layout
    /// frees (see [`WindowPlacement::Free`]) keep their own geometry.
    Tiling,
    /// One window is shown fullscreen and the others are hidden. Toggling
    /// placement is refused.
    Kiosk,
}

impl LayoutKind {
    /// The wire name of the kind, as a layout manifest spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Floating => "floating",
            Self::Tiling => "tiling",
            Self::Kiosk => "kiosk",
        }
    }

    /// The kind a manifest names, or `None` for an unknown name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "floating" => Some(Self::Floating),
            "tiling" => Some(Self::Tiling),
            "kiosk" => Some(Self::Kiosk),
            _ => None,
        }
    }

    /// Whether a window can be moved between managed and free under this kind.
    #[must_use]
    pub const fn allows_toggle_float(self) -> bool {
        matches!(self, Self::Tiling)
    }

    /// Whether a layout of this kind may arrange in `extent`. Only a tiling
    /// layout may leave the output; floating and kiosk layouts stay inside it.
    #[must_use]
    pub const fn allows_extent(self, extent: LayoutExtent) -> bool {
        matches!(self, Self::Tiling) || matches!(extent, LayoutExtent::Output)
    }

    /// The placement a window takes when it enters a layout of this kind.
    /// Transients always start free, whatever the kind.
    #[must_use]
    pub const fn default_placement(self, transient: bool) -> WindowPlacement {
        match self {
            Self::Floating => WindowPlacement::Free,
            Self::Tiling | Self::Kiosk if transient => WindowPlacement::Free,
            Self::Tiling | Self::Kiosk => WindowPlacement::Managed,
        }
    }
}

/// The space a layout arranges its windows in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayoutExtent {
    /// Placements lie inside the output's work area.
    #[default]
    Output,
    /// Placements may lie anywhere in layout space; the layout's viewport picks
    /// the part that is shown.
    Unbounded,
}

impl LayoutExtent {
    /// The wire name of the extent, as a layout manifest spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Unbounded => "unbounded",
        }
    }

    /// The extent a manifest names, or `None` for an unknown name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "output" => Some(Self::Output),
            "unbounded" => Some(Self::Unbounded),
            _ => None,
        }
    }
}

/// Whether the layout assigned a window its geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowPlacement {
    /// The window keeps its own geometry; the layout does not place it.
    Free,
    /// The layout assigns the window its geometry.
    Managed,
}

impl WindowPlacement {
    /// The wire name of the placement.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Managed => "managed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_floating_layout_never_manages_a_window() {
        assert_eq!(LayoutKind::Floating.default_placement(false), WindowPlacement::Free);
        assert_eq!(LayoutKind::Floating.default_placement(true), WindowPlacement::Free);
    }

    #[test]
    fn a_transient_is_free_under_a_tiling_or_kiosk_layout() {
        assert_eq!(LayoutKind::Tiling.default_placement(true), WindowPlacement::Free);
        assert_eq!(LayoutKind::Kiosk.default_placement(true), WindowPlacement::Free);
        assert_eq!(LayoutKind::Tiling.default_placement(false), WindowPlacement::Managed);
    }

    #[test]
    fn only_a_tiling_layout_allows_toggling_placement() {
        assert!(LayoutKind::Tiling.allows_toggle_float());
        assert!(!LayoutKind::Floating.allows_toggle_float());
        assert!(!LayoutKind::Kiosk.allows_toggle_float());
    }

    #[test]
    fn only_a_tiling_layout_may_leave_the_output() {
        assert!(LayoutKind::Tiling.allows_extent(LayoutExtent::Unbounded));
        assert!(LayoutKind::Tiling.allows_extent(LayoutExtent::Output));
        assert!(!LayoutKind::Floating.allows_extent(LayoutExtent::Unbounded));
        assert!(!LayoutKind::Kiosk.allows_extent(LayoutExtent::Unbounded));
        assert!(LayoutKind::Kiosk.allows_extent(LayoutExtent::Output));
    }
}
