//! The panel's physical extent.

/// The panel's physical extent, in millimetres.
///
/// A zero axis means the display did not report one, which `EDID` allows and
/// projectors commonly do, so zero is preserved instead of being turned into a
/// plausible-looking number. This is display identity rather than a shared
/// primitive: nothing outside output discovery needs a millimetre measurement.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
pub struct PhysicalSize {
    /// Horizontal extent in millimetres, or zero when unknown.
    pub width_mm: u32,
    /// Vertical extent in millimetres, or zero when unknown.
    pub height_mm: u32,
}

impl PhysicalSize {
    /// A physical extent in millimetres, zero meaning unknown.
    #[must_use]
    pub const fn new(width_mm: u32, height_mm: u32) -> Self {
        Self { width_mm, height_mm }
    }
}
