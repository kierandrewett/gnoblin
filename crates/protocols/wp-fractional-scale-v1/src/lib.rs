//! XML protocol `fractional_scale_v1` (`<protocol name="fractional_scale_v1">`); interfaces served: `wp_fractional_scale_manager_v1`.
//!
//! `wp_fractional_scale_manager_v1`: the scale the compositor draws a surface at.
//!
//! A client asks for a `wp_fractional_scale_v1` for one surface and waits for `preferred_scale`, which is the
//! compositor's whole part of the protocol. The value is in 1/120ths of a logical pixel, and what this module decides
//! is *which* scale: the output's, because a surface is drawn at the scale of the output it is shown on, and a scale
//! chosen anywhere else would tell a client to render at a resolution the compositor then has to resample.
//!
//! The manager is the compositor's own, as `wp_viewporter` is, because Smithay's raises `fractional_scale_exists` on
//! the `wl_surface`. The error belongs to the manager (`wp_fractional_scale_manager_v1.error`), so a client that reads
//! the code against the object that carries it was told about a `wl_surface` error it never made.
//!
//! # Failure modes
//!
//! 1. **A client renders at 1x on a scaled output.** The preferred scale is sent from the output the surface is on as
//!    soon as the object exists, so a client never has to guess: `a_surface_is_told_the_scale_of_its_output` in
//!    `tests/protocols.rs`.
//! 2. **A preferred scale arrives before the client committed anything.** The event is sent on the object's creation,
//!    which is what the protocol asks for, so nothing depends on a commit or a mapped window.
//! 3. **A second object on one surface is blamed on the wrong object.** `fractional_scale_exists` is raised on the
//!    manager the request was made on: `a_second_fractional_scale_object_is_an_error_on_the_manager`.
//! 4. **A destroyed object keeps its surface from having another.** Only a live object counts, so a client may make a
//!    new one once it has destroyed the old: `a_surface_may_have_a_fractional_scale_object_again_once_the_first_is_gone`.
//! 5. **A client whose output was re-scaled keeps drawing at the old scale.** An object is told again when its
//!    output's scale changes (`refresh_preferred_scales`), and only then: a refresh that finds the same value
//!    sends nothing.

use std::sync::{Mutex, PoisonError};

use smithay::reexports::wayland_protocols::wp::fractional_scale::v1::server::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
};
use smithay::wayland::compositor::with_states;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-fractional-scale-v1",
    interfaces: &[ServedInterface::new("wp_fractional_scale_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// The version of `wp_fractional_scale_manager_v1` this compositor serves.
const VERSION: u32 = 1;

/// What the protocol needs of the compositor.
pub trait Host {
    /// The scale `surface` is drawn at: the scale of the output it is shown on.
    fn surface_scale(&self, surface: &WlSurface) -> f64;

    /// The surfaces that have a `wp_fractional_scale_v1` object, kept as they come.
    fn fractional_surfaces(&mut self) -> &mut Vec<WlSurface>;
}

/// Creates the `wp_fractional_scale_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> GlobalId
where
    D: GlobalDispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, FractionalScaleManagerGlobal>
        + 'static,
{
    registrar
        .display()
        .create_global::<D, wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, FractionalScaleManagerGlobal>(
            VERSION,
            FractionalScaleManagerGlobal,
        )
}

/// The data of the `wp_fractional_scale_manager_v1` global.
pub struct FractionalScaleManagerGlobal;

/// The data of a bound `wp_fractional_scale_manager_v1`.
pub struct Manager;

/// The data of a `wp_fractional_scale_v1`. The surface keeps the record of its object, so the object keeps nothing.
pub struct FractionalScale;

/// What a surface's data map keeps: its fractional-scale object while it has one, and the scale that object was last
/// told.
#[derive(Default)]
struct ScaleRecord {
    object: Option<Weak<wp_fractional_scale_v1::WpFractionalScaleV1>>,
    sent: Option<u32>,
}

/// Kept in a surface's data map.
struct ScaleSlot(Mutex<ScaleRecord>);

impl<D> GlobalDispatch2<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, D> for FractionalScaleManagerGlobal
where
    D: Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, Manager> + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, Manager);
    }
}

impl<D> Dispatch2<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, D> for Manager
where
    D: Host + Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, FractionalScale> + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        resource: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        request: wp_fractional_scale_manager_v1::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_fractional_scale_manager_v1::Request::GetFractionalScale { id, surface } => {
                if has_live_object(&surface) {
                    resource.post_error(
                        wp_fractional_scale_manager_v1::Error::FractionalScaleExists,
                        "the surface already has a fractional_scale object associated",
                    );
                    return;
                }
                let object = data_init.init(id, FractionalScale);
                with_states(&surface, |states| {
                    states
                        .data_map
                        .insert_if_missing_threadsafe(|| ScaleSlot(Mutex::new(ScaleRecord::default())));
                    if let Some(slot) = states.data_map.get::<ScaleSlot>() {
                        *slot.0.lock().unwrap_or_else(PoisonError::into_inner) = ScaleRecord {
                            object: Some(object.downgrade()),
                            sent: None,
                        };
                    }
                });
                tell(&surface, state.surface_scale(&surface));
                if !state.fractional_surfaces().contains(&surface) {
                    state.fractional_surfaces().push(surface);
                }
            }
            wp_fractional_scale_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<D> Dispatch2<wp_fractional_scale_v1::WpFractionalScaleV1, D> for FractionalScale {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &wp_fractional_scale_v1::WpFractionalScaleV1,
        _request: wp_fractional_scale_v1::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // `destroy` is the only request, and a destroyed object stops counting by itself: the surface's record holds
        // a weak reference.
    }
}

/// Whether the surface has a fractional-scale object that is still alive.
fn has_live_object(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states.data_map.get::<ScaleSlot>().is_some_and(|slot| {
            let held = slot.0.lock().unwrap_or_else(PoisonError::into_inner);
            held.object.as_ref().is_some_and(|object| object.upgrade().is_ok())
        })
    })
}

/// Tells the surface's object `scale`, unless it already holds that value or there is no object.
///
/// The wire carries 1/120ths of a logical pixel.
fn tell(surface: &WlSurface, scale: f64) {
    let value = f64::round(scale * 120.0) as u32;
    with_states(surface, |states| {
        let Some(slot) = states.data_map.get::<ScaleSlot>() else {
            return;
        };
        let mut held = slot.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(object) = held.object.as_ref().and_then(|object| object.upgrade().ok()) else {
            return;
        };
        if held.sent != Some(value) {
            held.sent = Some(value);
            object.preferred_scale(value);
        }
    });
}

/// Re-sends the preferred scale of the surfaces that have an object.
pub trait FractionalScaleExt: Host {
    /// Tells every surface that has a `wp_fractional_scale_v1` object the scale of its output again, after an
    /// output's scale changed.
    ///
    /// An object is told its surface's scale when it is created and never asked again, so a client whose output was
    /// re-scaled (a new scale, or a supersample factor, which changes the scale clients are told) would keep drawing
    /// at the old one. The surfaces are the ones that asked, kept as they come, and the dead ones are dropped here.
    fn refresh_preferred_scales(&mut self) {
        self.fractional_surfaces().retain(|surface| surface.is_alive());
        for surface in self.fractional_surfaces().clone() {
            let scale = self.surface_scale(&surface);
            tell(&surface, scale);
        }
    }
}

impl<T: Host> FractionalScaleExt for T {}
