//! XML protocol `ext_background_effect_v1` (`<protocol name="ext_background_effect_v1">`); interfaces served: `ext_background_effect_manager_v1`, `ext_background_effect_surface_v1`.
//!
//! `ext_background_effect_manager_v1` (staging): a blur behind a client's own surface.
//!
//! Smithay implements the requests. A bind of the manager reports the capability, and
//! `get_background_effect` attaches an effect object per surface. `set_blur_region` and
//! `unset_blur_region` are staged in the surface's cached state by Smithay and take
//! effect on the surface's next commit. This module gates the bind through the access
//! roster and exposes the committed region to the renderer's blur pass.
//!
//! ## Failure modes first
//!
//! 1. **A client blurs outside its own surface.** The region is per surface and is only
//!    read for that surface, so no client reads another client's pixels through this
//!    protocol. Covered by `tests/background_effect.rs`.
//! 2. **A staged region applies before commit.** Smithay keeps it pending until commit.
//! 3. **The global is advertised when the roster refuses it.** The bind is refused by
//!    `can_view`, so the client never sees the global.
//! 4. **An unset region still reads back as set.** Unsetting clears the committed region.

use std::sync::Arc;

use gnoblin_types::{Rect, WindowId};
use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New};
use smithay::wayland::background_effect::{BackgroundEffectSurfaceCachedState, ExtBackgroundEffectHandler};
use smithay::wayland::compositor::{RectangleKind, RegionAttributes, with_states};
use smithay::wayland::{GlobalData, GlobalDispatch2};

use crate::State;
use crate::access::{Access, AccessPolicy, Global};

/// The version this session serves: the staging protocol's first version.
const MANAGER_VERSION: u32 = 1;

/// One piece of a committed blur region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExtBackgroundEffectV1ObservedBlurRect {
    /// A rectangle the region covers.
    Add(Rect),
    /// A rectangle the region removes from what it covers so far.
    Subtract(Rect),
}

impl ExtBackgroundEffectHandler for State {}

/// Creates the `ext_background_effect_manager_v1` global gated by the access roster.
pub(crate) fn global(display_handle: &DisplayHandle, access: &Arc<Access>) -> GlobalId {
    display_handle.create_global::<State, ExtBackgroundEffectManagerV1, _>(
        MANAGER_VERSION,
        ExtBackgroundEffectV1ManagerGlobalData {
            access: Arc::clone(access),
        },
    )
}

/// The global's data: the roster, consulted at every bind.
struct ExtBackgroundEffectV1ManagerGlobalData {
    access: Arc<Access>,
}

impl GlobalDispatch2<ExtBackgroundEffectManagerV1, State> for ExtBackgroundEffectV1ManagerGlobalData {
    fn bind(
        &self,
        state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        // Smithay's own request handling takes the manager object from here, and the
        // manager announces the capability it serves, as Smithay's own bind does.
        let manager = data_init.init(resource, GlobalData);
        manager.capabilities(state.capabilities());
    }

    fn can_view(&self, client: &Client) -> bool {
        AccessPolicy::allows(&*self.access, &client.id(), Global::BackgroundEffect)
    }
}

impl State {
    /// The committed blur region of a surface, or [`None`] when it has none.
    ///
    /// The renderer's blur pass calls this per surface; the region is read under the
    /// surface's state lock and nothing is copied.
    pub fn with_blur_region<T>(&self, surface: &WlSurface, f: impl FnOnce(&RegionAttributes) -> T) -> Option<T> {
        with_states(surface, |states| {
            let mut cached = states.cached_state.get::<BackgroundEffectSurfaceCachedState>();
            cached.current().blur_region.as_ref().map(f)
        })
    }

    /// The committed blur region of a window's surface, as the control seam reads it.
    ///
    /// [`None`] means the window has no surface or its surface committed no region.
    pub(crate) fn window_blur_region(&self, window: WindowId) -> Option<Vec<ExtBackgroundEffectV1ObservedBlurRect>> {
        let surface = self.windows.get(&window)?.surface.wl_surface()?;
        self.with_blur_region(&surface, observed)
    }
}

/// The pieces of a committed region, in the order the client sent them.
fn observed(region: &RegionAttributes) -> Vec<ExtBackgroundEffectV1ObservedBlurRect> {
    region
        .rects
        .iter()
        .map(|(kind, rect)| {
            let rect = Rect {
                x: rect.loc.x,
                y: rect.loc.y,
                width: u32::try_from(rect.size.w).unwrap_or(0),
                height: u32::try_from(rect.size.h).unwrap_or(0),
            };
            match kind {
                RectangleKind::Add => ExtBackgroundEffectV1ObservedBlurRect::Add(rect),
                RectangleKind::Subtract => ExtBackgroundEffectV1ObservedBlurRect::Subtract(rect),
            }
        })
        .collect()
}
