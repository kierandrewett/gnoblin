//! XML protocol `alpha_modifier_v1` (`<protocol name="alpha_modifier_v1">`); interfaces served: `wp_alpha_modifier_v1`.
//!
//! `wp_alpha_modifier_v1`: a client multiplies its surface's opacity by a factor.
//!
//! The factor is a committed, double-buffered property of the surface. The composer
//! takes the product of it and the window's group opacity when it draws the window, so
//! a factor of one half over a black background paints half the colour. Only the root
//! surface is drawn, so the factor applies to the whole window; a factor set on a
//! subsurface has no effect because subsurfaces are not drawn separately.
//!
//! # Failure modes
//!
//! 1. **Two modifier objects for one surface.** Smithay posts
//!    `wp_alpha_modifier_v1.already_constructed`; the first object keeps the surface.
//! 2. **A modifier request after its surface is destroyed.** Smithay posts
//!    `wp_alpha_modifier_surface_v1.no_surface`; the surface is not changed.
//! 3. **A factor of zero.** The protocol means fully transparent; the window is not drawn.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::alpha_modifier::{AlphaModifierState, AlphaModifierSurfaceCachedState};
use smithay::wayland::compositor::with_states;

use crate::State;

/// Creates the `wp_alpha_modifier_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> AlphaModifierState {
    AlphaModifierState::new::<State>(display)
}

/// The factor the client last committed for a surface, or one when it set none.
pub(crate) fn committed_factor(surface: &WlSurface) -> f32 {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<AlphaModifierSurfaceCachedState>();
        cached.current().multiplier_f32().unwrap_or(1.0)
    })
}
