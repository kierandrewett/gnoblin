//! XML protocol `idle_inhibit_unstable_v1` (`<protocol name="idle_inhibit_unstable_v1">`); interfaces served: `zwp_idle_inhibit_manager_v1`.
//!
//! `zwp_idle_inhibit_manager_v1`: a client asks that the session never idle.
//!
//! A video player and a presentation tool both need it: the screen must not blank while
//! the user is watching something and not touching the input devices. The protocol's
//! object is a surface, so the inhibitor's lifetime is the surface's, and the answer —
//! "is this session idle?" — is decided in `gnoblin-wayland-ext-idle-notify-v1`, because a
//! second place deciding it would be a second answer to one question.
//!
//! # Failure modes
//!
//! 1. **An inhibitor leaks past its client.** The surface is checked for liveness on
//!    every idle evaluation, so a killed client's inhibitor stops counting even though
//!    its `destroy` request never arrived:
//!    `an_idle_inhibitor_suppresses_idle_and_never_leaks` in `tests/protocols.rs`.
//! 2. **An inhibitor outlives the object that asked for it.** Destroying the inhibitor
//!    object calls `uninhibit` for that surface through Smithay's request handler, so
//!    the session idles again on the next advance.

use smithay::reexports::wayland_protocols::wp::idle_inhibit::zv1::server::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use smithay::reexports::wayland_server::GlobalDispatch;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::GlobalData;
use smithay::wayland::idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-idle-inhibit-v1",
    interfaces: &[ServedInterface::new("zwp_idle_inhibit_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// What the protocol needs of the compositor: where the inhibitors are recorded.
pub trait Host {
    /// Records or clears the inhibitor of `surface`. The session's idle answer is decided where
    /// the idle notifications are, so this protocol only says which surfaces want the session awake.
    fn set_idle_inhibited(&mut self, surface: &WlSurface, inhibited: bool);
}

/// Creates the `zwp_idle_inhibit_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> IdleInhibitManagerState
where
    D: GlobalDispatch<ZwpIdleInhibitManagerV1, GlobalData> + IdleInhibitHandler + 'static,
{
    IdleInhibitManagerState::new::<D>(registrar.display())
}

/// A client created an inhibitor for `surface`.
pub fn inhibit<H: Host>(host: &mut H, surface: &WlSurface) {
    host.set_idle_inhibited(surface, true);
}

/// A client destroyed the inhibitor of `surface`.
pub fn uninhibit<H: Host>(host: &mut H, surface: &WlSurface) {
    host.set_idle_inhibited(surface, false);
}
