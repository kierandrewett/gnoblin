//! XML protocol `fractional_scale_v1` (`<protocol name="fractional_scale_v1">`); interfaces served: `wp_fractional_scale_manager_v1`.
//!
//! `wp_fractional_scale_manager_v1`: the scale the compositor draws a surface at.
//!
//! The global has no requests worth answering: a client asks for a
//! `wp_fractional_scale_v1` for one surface and waits for `preferred_scale`, which is
//! the compositor's whole part of the protocol. So the handler's one job is to answer
//! a new fractional-scale object with the scale of the output that surface is on.
//!
//! The value is in 1/120ths of a logical pixel, which Smithay encodes; what this module
//! decides is *which* scale. It is the output's, because a surface is drawn at the
//! scale of the output it is shown on: a scale chosen anywhere else would tell a client
//! to render at a resolution the compositor then has to resample.
//!
//! # Failure modes
//!
//! 1. **A client renders at 1x on a scaled output.** The preferred scale is sent from
//!    the output the surface is on as soon as the object exists, so a client never has
//!    to guess: `a_surface_is_told_the_scale_of_its_output` in `tests/protocols.rs`.
//! 2. **A preferred scale arrives before the client committed anything.** The event is
//!    sent on the object's creation, which is what the protocol asks for, so nothing
//!    depends on a commit or a mapped window.

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;
use smithay::wayland::fractional_scale::{self, FractionalScaleHandler, FractionalScaleManagerState};

use crate::State;

/// Creates the `wp_fractional_scale_manager_v1` global.
pub(crate) fn global(display: &smithay::reexports::wayland_server::DisplayHandle) -> FractionalScaleManagerState {
    FractionalScaleManagerState::new::<State>(display)
}

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self.surface_scale(&surface);
        with_states(&surface, |states| {
            fractional_scale::with_fractional_scale(states, |state| state.set_preferred_scale(scale));
        });
        self.fractional_surfaces.push(surface);
    }
}

impl State {
    /// Tells every surface that has a `wp_fractional_scale_v1` object the scale of its output again, after an
    /// output's scale changed.
    ///
    /// An object is told its surface's scale when it is created and never asked again, so a client whose output was
    /// re-scaled (a new scale, or a supersample factor, which changes the scale clients are told) would keep drawing
    /// at the old one. The surfaces are the ones that asked, kept as they come, and the dead ones are dropped here.
    pub(crate) fn refresh_preferred_scales(&mut self) {
        self.fractional_surfaces.retain(|surface| surface.is_alive());
        for surface in self.fractional_surfaces.clone() {
            let scale = self.surface_scale(&surface);
            with_states(&surface, |states| {
                fractional_scale::with_fractional_scale(states, |state| state.set_preferred_scale(scale));
            });
        }
    }
}
