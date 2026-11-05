//! XML protocol `alpha_modifier_v1` (`<protocol name="alpha_modifier_v1">`); interfaces served: `wp_alpha_modifier_v1`.
//!
//! `wp_alpha_modifier_v1`: a client multiplies its surface's opacity by a factor.
//!
//! Implemented but NOT advertised. The global is not created in
//! [`crate::State::new`], so no client can bind it. The reason is that the composer
//! draws one group opacity per window, and the window manager's `SetOpacity` already
//! takes that slot. Nothing forms the product of the client's factor and the manager's
//! opacity, so advertising the global would accept a factor and never draw it. The
//! advertising rule is that a global is registered only when its behaviour is honoured
//! and tested. Registering it needs the composer to take the product per window, with a
//! test that a factor changes the rendered pixels; until then this module is kept and
//! not wired.
//!
//! # Failure modes (for when it is advertised)
//!
//! 1. **Two modifier objects for one surface.** Smithay posts
//!    `wp_alpha_modifier_v1.already_constructed`; the first object keeps the surface.
//! 2. **A modifier request after its surface is destroyed.** Smithay posts
//!    `wp_alpha_modifier_surface_v1.no_surface`; the surface is not changed.
//! 3. **A factor of zero.** The protocol means fully transparent.
//! 4. **A factor on a subsurface.** Not applied by the composer, which takes one opacity
//!    per window; this is one reason the global is not advertised.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::alpha_modifier::{AlphaModifierState, AlphaModifierSurfaceCachedState};
use smithay::wayland::compositor::with_states;

use crate::State;

/// Creates the `wp_alpha_modifier_v1` global. Not called: the global is not advertised.
#[allow(dead_code, reason = "implemented but not advertised; see the module docs")]
pub(crate) fn global(display: &DisplayHandle) -> AlphaModifierState {
    AlphaModifierState::new::<State>(display)
}

/// The factor the client last committed for a surface, or one when it set none.
///
/// Not called while the global is not advertised; see the module docs.
#[allow(dead_code, reason = "implemented but not advertised; see the module docs")]
pub(crate) fn committed_factor(surface: &WlSurface) -> f32 {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<AlphaModifierSurfaceCachedState>();
        cached.current().multiplier_f32().unwrap_or(1.0)
    })
}
