//! The closed set of bands: the coarse slots a layer sits in, bottom to top.
//!
//! A band is an ordering substrate, not a layer's identity. A layer's identity is
//! its full name, and its place inside a band is its `Order`. The set is closed:
//! the compositor owns it, and configuration cannot add to it. That closure is
//! the trust boundary. No extension can sort above the lock or the cursor,
//! because no extension can name a band that is not in this list.
//!
//! Declaration order is paint order. `Ord` is derived from it, so sorting bands
//! sorts them from the bottom of the stack to the top.

/// One band, from the bottom of the stack to the top.
///
/// `Lock` and `Cursor` accept registrations only from the compositor itself. The
/// lock keeps the unlock prompt above everything else, and the cursor is never
/// covered by a layer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Band {
    /// The bottom of the stack, under every window. Wayland `background` layers.
    Background,
    /// Above the background, still under windows. Wayland `bottom` layers.
    Bottom,
    /// Ordinary windows. Toplevels, X11 windows and the layers attached to them.
    Windows,
    /// Above windows. Wayland `top` layers, such as bars.
    Top,
    /// Above `Top`. Wayland `overlay` layers and extension output layers.
    Overlay,
    /// Compositor-chosen system UI, such as an output-scope portal with no parent.
    System,
    /// The session lock. Compositor-only, so nothing draws over the unlock prompt.
    Lock,
    /// The pointer. Compositor-only, so no layer can cover it.
    Cursor,
}

impl Band {
    /// Every band in paint order, bottom to top. A test checks this matches `Ord`.
    pub const ALL: [Band; 8] = [
        Band::Background,
        Band::Bottom,
        Band::Windows,
        Band::Top,
        Band::Overlay,
        Band::System,
        Band::Lock,
        Band::Cursor,
    ];

    /// Whether only the compositor may register into this band.
    ///
    /// The check is done by the registry, against the provider's kind, not by the
    /// caller. Keeping the rule on the band means a new compositor-only band
    /// cannot be forgotten at a call site.
    #[must_use]
    pub const fn compositor_only(self) -> bool {
        matches!(self, Band::Lock | Band::Cursor)
    }
}
