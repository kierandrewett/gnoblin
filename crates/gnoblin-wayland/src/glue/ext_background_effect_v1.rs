//! `gnoblin-wayland-ext-background-effect-v1`: the committed blur region of a window's surface.

use gnoblin_types::WindowId;
use gnoblin_wayland_protocols::ext_background_effect_v1::{
    ExtBackgroundEffectV1ObservedBlurRect, observed, with_blur_region,
};
use smithay::wayland::background_effect::ExtBackgroundEffectHandler;

use crate::State;

impl ExtBackgroundEffectHandler for State {}

impl State {
    /// The committed blur region of a window's surface, as the control seam reads it.
    ///
    /// [`None`] means the window has no surface or its surface committed no region.
    pub(crate) fn window_blur_region(&self, window: WindowId) -> Option<Vec<ExtBackgroundEffectV1ObservedBlurRect>> {
        let surface = self.windows.get(&window)?.surface.wl_surface()?;
        with_blur_region(&surface, observed)
    }
}
