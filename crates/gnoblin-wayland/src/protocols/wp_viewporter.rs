//! XML protocol `viewporter` (`<protocol name="viewporter">`); interfaces served: `wp_viewporter`, `wp_viewport`.
//!
//! `wp_viewporter`: a client crops a surface's buffer (`set_source`) and scales it (`set_destination`).
//!
//! The values live in Smithay's [`ViewportCachedState`], which a surface applies on commit and the renderer
//! reads for the source rectangle. The protocol's requests and its errors are this module's, because
//! Smithay's own `ViewporterState` gets three of them wrong for a conformance suite and keeps the pieces
//! that would fix them private:
//!
//! 1. A second `get_viewport` on a surface raises `viewport_exists` on the `wl_surface`. The error belongs
//!    to `wp_viewporter`.
//! 2. The `bad_size` check is a pre-commit hook that is only installed when the surface had no viewport
//!    data at all. A surface that has committed a buffer before it gets a viewport has that data (the
//!    renderer state creates it), so the hook is never installed and a source with a fractional size and
//!    no destination goes through.
//! 3. `out_of_buffer` is checked when a commit brings a new buffer. A commit that changes the source or
//!    the buffer scale of the buffer already attached never reaches the check.
//!
//! # Failure modes
//!
//! 1. **A second viewport on one surface.** `wp_viewporter.viewport_exists` on the `wp_viewporter`
//!    object, and no new viewport.
//! 2. **A source outside the buffer.** `wp_viewport.out_of_buffer` at commit, against the buffer at the
//!    scale and transform the commit applies, so a commit that only changes the scale is checked too.
//! 3. **A fractional source with no destination.** `wp_viewport.bad_size` at commit, from the state the
//!    commit applied, so it is raised whichever order the client set the two in. A source that is both
//!    outside the buffer and fractional is `out_of_buffer`: the suite reads the first as the more basic
//!    mistake, and it is the one a client can only fix by changing the source.
//! 4. **A viewport that outlives its surface.** Every request but `destroy` raises `no_surface`.
//! 5. **A subsurface that waits for its parent.** A synchronous subsurface's state applies with its
//!    parent's commit, so it is checked then, from the parent's tree walk.

use std::sync::Mutex;

use smithay::backend::renderer::buffer_dimensions;
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_protocols::wp::viewporter::server::{wp_viewport, wp_viewporter};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource, Weak};
use smithay::utils::{Logical, Rectangle, Size, Transform};
use smithay::wayland::compositor::{
    CompositorHandler, SurfaceAttributes, SurfaceData, TraversalAction, is_sync_subsurface, with_states,
    with_surface_tree_upward,
};
use smithay::wayland::viewporter::ViewportCachedState;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::State;

/// The version of `wp_viewporter` this compositor serves.
const VERSION: u32 = 1;

/// Creates the `wp_viewporter` global.
pub(crate) fn create_global(display: &DisplayHandle) {
    let _ = display.create_global::<State, wp_viewporter::WpViewporter, ViewporterGlobal>(VERSION, ViewporterGlobal);
}

/// The data of the `wp_viewporter` global.
pub(crate) struct ViewporterGlobal;

/// The data of a bound `wp_viewporter`.
pub(crate) struct Viewporter;

/// The data of a `wp_viewport`: the surface it was made for, until that goes.
pub(crate) struct Viewport {
    surface: Weak<WlSurface>,
}

/// Kept in a surface's data map: the surface's viewport object, while it has one.
struct ViewportSlot(Mutex<Option<Weak<wp_viewport::WpViewport>>>);

impl GlobalDispatch2<wp_viewporter::WpViewporter, State> for ViewporterGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wp_viewporter::WpViewporter>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, Viewporter);
    }
}

impl Dispatch2<wp_viewporter::WpViewporter, State> for Viewporter {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        resource: &wp_viewporter::WpViewporter,
        request: wp_viewporter::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            wp_viewporter::Request::GetViewport { id, surface } => {
                if live_viewport_of(&surface).is_some() {
                    resource.post_error(
                        wp_viewporter::Error::ViewportExists,
                        "the surface already has a viewport object associated",
                    );
                    return;
                }
                let viewport = data_init.init(
                    id,
                    Viewport {
                        surface: surface.downgrade(),
                    },
                );
                with_states(&surface, |states| {
                    states
                        .data_map
                        .insert_if_missing_threadsafe(|| ViewportSlot(Mutex::new(None)));
                    if let Some(binding) = states.data_map.get::<ViewportSlot>() {
                        *binding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(viewport.downgrade());
                    }
                });
            }
            wp_viewporter::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch2<wp_viewport::WpViewport, State> for Viewport {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        resource: &wp_viewport::WpViewport,
        request: wp_viewport::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            wp_viewport::Request::Destroy => {
                if let Ok(surface) = self.surface.upgrade() {
                    with_states(&surface, |states| {
                        if let Some(binding) = states.data_map.get::<ViewportSlot>() {
                            binding
                                .0
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take();
                        }
                        *states.cached_state.get::<ViewportCachedState>().pending() = ViewportCachedState::default();
                    });
                }
            }
            wp_viewport::Request::SetSource { x, y, width, height } => {
                let unset = x == -1.0 && y == -1.0 && width == -1.0 && height == -1.0;
                if !unset && !valid_source(x, y, width, height) {
                    resource.post_error(
                        wp_viewport::Error::BadValue,
                        "negative or zero values in width or height or negative values in x or y",
                    );
                    return;
                }
                let Ok(surface) = self.surface.upgrade() else {
                    resource.post_error(wp_viewport::Error::NoSurface, "the wl_surface was destroyed");
                    return;
                };
                with_states(&surface, |states| {
                    states.cached_state.get::<ViewportCachedState>().pending().src =
                        (!unset).then(|| Rectangle::new((x, y).into(), (width, height).into()));
                });
            }
            wp_viewport::Request::SetDestination { width, height } => {
                let unset = width == -1 && height == -1;
                if !unset && !(width > 0 && height > 0) {
                    resource.post_error(
                        wp_viewport::Error::BadValue,
                        "negative or zero values in width or height",
                    );
                    return;
                }
                let Ok(surface) = self.surface.upgrade() else {
                    resource.post_error(wp_viewport::Error::NoSurface, "the wl_surface was destroyed");
                    return;
                };
                let scale = state.client_compositor_state(client).client_scale();
                with_states(&surface, |states| {
                    // The client's units are its buffer-scale-free pixels: the destination is that many
                    // of them divided by the scale the client's compositor state keeps for it.
                    states.cached_state.get::<ViewportCachedState>().pending().dst = (!unset).then(|| {
                        Size::<f64, Logical>::from((f64::from(width) / scale, f64::from(height) / scale)).to_i32_round()
                    });
                });
            }
            _ => {}
        }
    }
}

/// Whether a source rectangle is one the protocol allows: a corner that is not negative and a size
/// that is more than nothing.
fn valid_source(x: f64, y: f64, width: f64, height: f64) -> bool {
    x >= 0.0 && y >= 0.0 && width > 0.0 && height > 0.0
}

/// Whether a size has no fraction: what a source needs to be when no destination says how big the
/// surface is.
fn is_whole(size: Size<f64, Logical>) -> bool {
    size.w.fract() == 0.0 && size.h.fract() == 0.0
}

/// Whether `source` lies inside a buffer of `buffer`, both in the surface's logical units.
fn source_fits(source: Rectangle<f64, Logical>, buffer: Size<i32, Logical>) -> bool {
    Rectangle::from_size(buffer.to_f64()).contains_rect(source)
}

/// The surface's live viewport object, if it has one.
fn live_viewport_of(surface: &WlSurface) -> Option<wp_viewport::WpViewport> {
    with_states(surface, viewport_in)
}

fn viewport_in(states: &SurfaceData) -> Option<wp_viewport::WpViewport> {
    let binding = states.data_map.get::<ViewportSlot>()?;
    let held = binding.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    held.as_ref()?.upgrade().ok()
}

/// The buffer a surface shows, in logical units at the scale and transform its last commit set.
///
/// The renderer's own record of the buffer's size holds the scale of the commit that attached the
/// buffer, so a commit that only changes the scale would be read at the old one.
fn committed_buffer_size(states: &SurfaceData) -> Option<Size<i32, Logical>> {
    let renderer = states.data_map.get::<RendererSurfaceStateUserData>()?;
    let renderer = renderer.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let buffer = renderer.buffer()?;
    let dimensions = buffer_dimensions(buffer)?;
    let mut attributes = states.cached_state.get::<SurfaceAttributes>();
    let attributes = attributes.current();
    Some(dimensions.to_logical(attributes.buffer_scale, Transform::from(attributes.buffer_transform)))
}

/// Raises `out_of_buffer` or `bad_size` for a surface, or any of its subsurfaces, whose committed viewport is
/// wrong.
///
/// Called after a commit has applied the surface's state. A surface that is a synchronous subsurface
/// waits for its parent's commit, which walks the tree and reaches it.
pub(crate) fn validate_commit(surface: &WlSurface) {
    if is_sync_subsurface(surface) {
        return;
    }
    with_surface_tree_upward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_, states, _| {
            let Some(viewport) = viewport_in(states) else {
                return;
            };
            let (source, destination) = {
                let mut cached = states.cached_state.get::<ViewportCachedState>();
                let current = cached.current();
                (current.src, current.dst)
            };
            let Some(source) = source else {
                return;
            };
            if let Some(buffer) = committed_buffer_size(states)
                && !source_fits(source, buffer)
            {
                viewport.post_error(
                    wp_viewport::Error::OutOfBuffer,
                    format!(
                        "source rectangle x={},y={},w={},h={} extends outside of the buffer, {}x{}",
                        source.loc.x, source.loc.y, source.size.w, source.size.h, buffer.w, buffer.h
                    ),
                );
            } else if destination.is_none() && !is_whole(source.size) {
                viewport.post_error(wp_viewport::Error::BadSize, "destination size is not integer");
            }
        },
        |_, _, _| true,
    );
}

#[cfg(test)]
mod tests {
    use smithay::utils::{Logical, Rectangle, Size};

    use super::{is_whole, source_fits, valid_source};

    /// A source that has a corner below zero or no area is refused, and one that has both is allowed.
    #[test]
    fn a_source_needs_a_corner_that_is_not_negative_and_an_area() {
        assert!(valid_source(0.0, 0.0, 1.0, 1.0));
        assert!(valid_source(0.5, 0.25, 0.5, 0.5));
        assert!(!valid_source(-1.0, 0.0, 1.0, 1.0), "a negative x");
        assert!(!valid_source(0.0, -0.5, 1.0, 1.0), "a negative y");
        assert!(!valid_source(0.0, 0.0, 0.0, 1.0), "no width");
        assert!(!valid_source(0.0, 0.0, 1.0, -2.0), "a negative height");
    }

    /// The size a source needs without a destination is whole in both directions.
    #[test]
    fn a_size_with_a_fraction_in_either_direction_is_not_whole() {
        assert!(is_whole(Size::from((23.0, 100.0))));
        assert!(!is_whole(Size::from((23.2, 100.0))));
        assert!(!is_whole(Size::from((23.0, 99.5))));
    }

    /// A buffer of 200x100 pixels at scale 2 is 100x50 in the surface's units, so a source that ends at
    /// 100.5 is outside it while one that ends at 100 is not.
    #[test]
    fn a_source_must_end_inside_the_buffer_in_logical_units() {
        let buffer: Size<i32, Logical> = Size::from((100, 50));
        let source = |x: f64, width: f64| Rectangle::<f64, Logical>::new((x, 0.0).into(), (width, 50.0).into());
        assert!(source_fits(source(50.0, 50.0), buffer));
        assert!(!source_fits(source(50.0, 50.5), buffer));
        assert!(source_fits(source(0.0, 100.0), buffer), "the whole buffer fits");
    }
}
