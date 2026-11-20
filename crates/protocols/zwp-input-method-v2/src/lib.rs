//! XML protocol `input_method_unstable_v2` (`<protocol name="input_method_unstable_v2">`); interfaces served: `zwp_input_method_manager_v2`.
//!
//! `zwp_input_method_manager_v2` and `zwp_input_method_v2` (XML `input-method-unstable-v2`).
//!
//! An input method (IBus, fcitx, an on-screen keyboard) holds a `zwp_input_method_v2` on the
//! seat and answers the focused text input with `commit_string`, `preedit_string` and
//! `delete_surrounding_text`. It is privileged: it sees what the user types everywhere, so
//! [`AccessPolicy`](gnoblin_wayland_api::access::AccessPolicy) decides who may bind the global.
//!
//! # Refusals and failure modes
//!
//! 1. **An unprivileged client.** The manager global is filtered by `AccessPolicy` at bind
//!    time; an ordinary or sandboxed client never sees the input method.
//! 2. **Popup placement.** Popups are positioned against the parent surface's geometry.
//!
//! # Guarding tests
//!
//! The privileged-global rules are guarded by `tests/access.rs`
//! (`an_unprivileged_client_sees_no_privileged_global`).

use gnoblin_wayland_api::access::Global;
use gnoblin_wayland_api::host::WindowHost;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use smithay::input::SeatHandler;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch, Resource};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::GlobalData;
use smithay::wayland::input_method::{
    InputMethodManagerGlobalData, InputMethodManagerState, InputMethodUserData, PopupSurface,
};
use wayland_protocols_misc::zwp_input_method_v2::server::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2, zwp_input_method_v2::ZwpInputMethodV2,
};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-input-method-v2",
    interfaces: &[ServedInterface::new("zwp_input_method_manager_v2", 1)],
    privilege: Privilege::Roster(Global::InputMethod),
};

/// What the protocol needs of the compositor: where the input method's popups are kept.
pub trait Host: WindowHost {
    /// The popups the input method created.
    fn input_method_popups(&mut self) -> &mut Vec<PopupSurface>;
}

/// Creates the `zwp_input_method_manager_v2` global.
///
/// The filter is the access policy: an input method sees every keystroke in the session, so
/// only a client the policy calls privileged may be one.
pub fn register<D>(registrar: &Registrar<'_>) -> InputMethodManagerState
where
    D: GlobalDispatch<ZwpInputMethodManagerV2, InputMethodManagerGlobalData>
        + Dispatch<ZwpInputMethodManagerV2, GlobalData>
        + Dispatch<ZwpInputMethodV2, InputMethodUserData<D>>
        + SeatHandler
        + 'static,
{
    InputMethodManagerState::new::<D, _>(registrar.display(), registrar.filter(Global::InputMethod))
}

/// The input method created a popup.
pub fn new_popup<H: Host>(host: &mut H, surface: PopupSurface) {
    log::debug!(area = "input-method", popup = ?surface.wl_surface().id(), "the input method created a popup");
    host.input_method_popups().push(surface);
}

/// The input method dismissed a popup.
pub fn dismiss_popup<H: Host>(host: &mut H, surface: &PopupSurface) {
    host.input_method_popups()
        .retain(|held| held.wl_surface() != surface.wl_surface());
}

/// The input method moved a popup.
pub fn popup_repositioned(surface: &PopupSurface) {
    log::debug!(area = "input-method", popup = ?surface.wl_surface().id(),
        "the input method moved its popup"
    );
}

/// The geometry the popup is placed against.
pub fn parent_geometry<H: Host>(host: &H, parent: &WlSurface) -> Rectangle<i32, Logical> {
    // The popup is placed against the focused window, which is the surface the input
    // method's text input belongs to. A surface with no window is at the origin, which
    // is what a popup with no parent gets.
    match host.window_of(parent).and_then(|window| host.geometry_of(window)) {
        Some(rect) => Rectangle::new(
            (rect.x, rect.y).into(),
            (
                i32::try_from(rect.width).unwrap_or(0),
                i32::try_from(rect.height).unwrap_or(0),
            )
                .into(),
        ),
        None => Rectangle::from_size((0, 0).into()),
    }
}
