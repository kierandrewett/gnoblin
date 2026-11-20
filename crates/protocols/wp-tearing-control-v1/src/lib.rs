//! XML protocol `tearing_control_v1` (`<protocol name="tearing_control_v1">`); interfaces served: `wp_tearing_control_v1`.
//!
//! `wp_tearing_control_v1`: a client says it would accept a torn frame.
//!
//! A game with an uncapped frame rate prefers a frame shown the instant it is ready to
//! one shown a refresh later, and accepts the tear that comes with it. The protocol is the
//! client's half of that offer: `set_presentation_hint(async)` says "tearing is fine",
//! `vsync` says "it is not", and a compositor that cannot tear ignores both. Nothing in the
//! protocol is a promise, so ignoring it is a correct implementation; honouring it is an
//! optimisation.
//!
//! What the compositor owes the client is that the hint is *kept*, so a presentation
//! decision made when the frame is composed can be made from the hint the client last set
//! on the surface being composed. `tearing_hint` is that read, and the hint lives
//! in the surface's own record so it cannot outlive the surface.
//!
//! # Failure modes
//!
//! 1. **A hint outlives its surface.** Hints are keyed by surface and pruned when the
//!    surface is gone, so a recycled surface id cannot inherit another client's choice.
//! 2. **A client's hint is lost when it re-creates its control object.** A new control
//!    object for the same surface replaces the old hint, as the protocol states: the last
//!    request wins and the record is keyed by surface rather than by object.
//! 3. **A destroyed manager destroys a live control object.** The control objects outlive
//!    the manager, so the hint keeps working after the manager is destroyed.

use smithay::reexports::wayland_protocols::wp::tearing_control::v1::server::{
    wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
    wp_tearing_control_v1::{self, WpTearingControlV1},
};
use smithay::reexports::wayland_server::backend::{ClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-tearing-control-v1",
    interfaces: &[ServedInterface::new("wp_tearing_control_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// What a client asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WpTearingControlV1Hint {
    /// No hint was given, so the compositor presents tear-free.
    #[default]
    Unset,
    /// Tearing is acceptable.
    Async,
    /// Tearing is not acceptable.
    Sync,
}

/// What the protocol needs of the compositor: the place the hints are kept.
pub trait Host {
    /// The hints, one per surface that set one.
    fn tearing_hints(&self) -> &[(WlSurface, WpTearingControlV1Hint)];

    /// The same, to change.
    fn tearing_hints_mut(&mut self) -> &mut Vec<(WlSurface, WpTearingControlV1Hint)>;
}

/// User data of a `wp_tearing_control_manager_v1`.
#[derive(Debug)]
pub struct WpTearingControlV1TearingManagerUserData;

/// User data of a `wp_tearing_control_v1`.
#[derive(Debug)]
pub struct WpTearingControlV1TearingControlUserData {
    surface: WlSurface,
}

/// Creates the `wp_tearing_control_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> GlobalId
where
    D: GlobalDispatch<WpTearingControlManagerV1, WpTearingControlV1TearingManagerUserData> + 'static,
{
    registrar
        .display()
        .create_global::<D, WpTearingControlManagerV1, _>(1, WpTearingControlV1TearingManagerUserData)
}

impl<D> GlobalDispatch2<WpTearingControlManagerV1, D> for WpTearingControlV1TearingManagerUserData
where
    D: Dispatch<WpTearingControlManagerV1, WpTearingControlV1TearingManagerUserData> + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WpTearingControlManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, WpTearingControlV1TearingManagerUserData);
    }
}

impl<D> Dispatch2<WpTearingControlManagerV1, D> for WpTearingControlV1TearingManagerUserData
where
    D: Host + Dispatch<WpTearingControlV1, WpTearingControlV1TearingControlUserData> + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &WpTearingControlManagerV1,
        request: wp_tearing_control_manager_v1::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_tearing_control_manager_v1::Request::GetTearingControl { id, surface } => {
                if !surface.is_alive() {
                    return;
                }
                data_init.init(
                    id,
                    WpTearingControlV1TearingControlUserData {
                        surface: surface.clone(),
                    },
                );
                // A new control object starts with no hint, which is the protocol's
                // starting state: the surface asks nothing until it says so.
                state.set_tearing_hint(&surface, WpTearingControlV1Hint::Unset);
            }
            wp_tearing_control_manager_v1::Request::Destroy => {}
            _ => unreachable!("wp_tearing_control_manager_v1 has two requests"),
        }
    }
}

impl<D: Host> Dispatch2<WpTearingControlV1, D> for WpTearingControlV1TearingControlUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &WpTearingControlV1,
        request: wp_tearing_control_v1::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_tearing_control_v1::Request::SetPresentationHint { hint } => {
                let hint = match hint {
                    WEnum::Value(wp_tearing_control_v1::PresentationHint::Async) => WpTearingControlV1Hint::Async,
                    WEnum::Value(wp_tearing_control_v1::PresentationHint::Vsync) => WpTearingControlV1Hint::Sync,
                    // A value from a newer revision is answered the way the protocol
                    // asks: ignore it rather than guess what it means.
                    _ => return,
                };
                log::debug!(area = "tearing", surface = ?self.surface.id(), ?hint, "a client set a hint");
                state.set_tearing_hint(&self.surface, hint);
            }
            wp_tearing_control_v1::Request::Destroy => {
                state.set_tearing_hint(&self.surface, WpTearingControlV1Hint::Unset);
            }
            _ => unreachable!("wp_tearing_control_v1 has two requests"),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, _resource: &WpTearingControlV1) {
        state.set_tearing_hint(&self.surface, WpTearingControlV1Hint::Unset);
    }
}

/// The surfaces' tearing hints, read and written through the host.
pub trait TearingExt: Host {
    /// Records a surface's tearing hint.
    fn set_tearing_hint(&mut self, surface: &WlSurface, hint: WpTearingControlV1Hint) {
        self.tearing_hints_mut().retain(|(held, _)| held != surface);
        if hint != WpTearingControlV1Hint::Unset {
            self.tearing_hints_mut().push((surface.clone(), hint));
        }
    }

    /// The tearing hint a surface's client last set.
    #[must_use]
    fn tearing_hint(&self, surface: &WlSurface) -> WpTearingControlV1Hint {
        self.tearing_hints()
            .iter()
            .find(|(held, _)| held == surface && held.is_alive())
            .map_or(WpTearingControlV1Hint::Unset, |(_, hint)| *hint)
    }
}

impl<T: Host + ?Sized> TearingExt for T {}
