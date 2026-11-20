//! XML protocol `alpha_modifier_v1` (`<protocol name="alpha_modifier_v1">`); interfaces served: `wp_alpha_modifier_v1`.
//!
//! `wp_alpha_modifier_v1`: a client multiplies its surface's opacity by a factor.
//!
//! The factor is a committed, double-buffered property of the surface. The composer
//! takes the product of it and the window's group opacity when it draws the window, so
//! a factor of one half over a black background paints half the colour. The factor of the
//! window's own surface applies to the window; the factor of a subsurface or a popup applies
//! to that surface and to everything that hangs from it, on top of its window's opacity
//! (`surface_tree.rs`).
//!
//! # Failure modes
//!
//! 1. **Two modifier objects for one surface.** Smithay posts
//!    `wp_alpha_modifier_v1.already_constructed`; the first object keeps the surface.
//! 2. **A modifier request after its surface is destroyed.** Smithay posts
//!    `wp_alpha_modifier_surface_v1.no_surface`; the surface is not changed.
//! 3. **A factor of zero.** The protocol means fully transparent; the window is not drawn.

use smithay::reexports::wayland_protocols::wp::alpha_modifier::v1::server::{
    wp_alpha_modifier_surface_v1::WpAlphaModifierSurfaceV1, wp_alpha_modifier_v1::WpAlphaModifierV1,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::alpha_modifier::{
    AlphaModifierState, AlphaModifierSurfaceCachedState, AlphaModifierSurfaceUserData,
};
use smithay::wayland::compositor::with_states;

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-alpha-modifier-v1",
    interfaces: &[ServedInterface::new("wp_alpha_modifier_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `wp_alpha_modifier_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> AlphaModifierState
where
    D: GlobalDispatch<WpAlphaModifierV1, GlobalData>
        + Dispatch<WpAlphaModifierV1, GlobalData>
        + Dispatch<WpAlphaModifierSurfaceV1, AlphaModifierSurfaceUserData>
        + 'static,
{
    AlphaModifierState::new::<D>(registrar.display())
}

/// The factor a surface last committed, one when it committed none.
#[must_use]
pub fn committed_factor(surface: &WlSurface) -> f32 {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<AlphaModifierSurfaceCachedState>();
        cached.current().multiplier_f32().unwrap_or(1.0)
    })
}
