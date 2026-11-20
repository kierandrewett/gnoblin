//! XML protocol `ext_session_lock_v1` (`<protocol name="ext_session_lock_v1">`); interfaces served: `ext_session_lock_manager_v1`.
//!
//! `ext_session_lock_manager_v1`: the lock request, the cover and the embargo.
//!
//! The crate creates the global and says what it advertises. The lock itself is the engine's work: a lock covers every output,
//! stops capture, takes the input and answers the locker only when the cover has been presented, which needs the frame path, the
//! lock card and the lock's authentication, so the handler (`SessionLockHandler`) and the cover's drawing are in
//! `gnoblin-wayland`'s `session_lock` module. The global is advertised to every client: the protocol makes the socket the
//! admission boundary, and `docs/session-lock.md` states that the first request owns the lock.

use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1, ext_session_lock_v1::ExtSessionLockV1,
};
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::session_lock::{
    SessionLockHandler, SessionLockManagerGlobalData, SessionLockManagerState, SessionLockState,
};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "ext-session-lock-v1",
    interfaces: &[ServedInterface::new("ext_session_lock_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `ext_session_lock_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> SessionLockManagerState
where
    D: GlobalDispatch<ExtSessionLockManagerV1, SessionLockManagerGlobalData>
        + Dispatch<ExtSessionLockManagerV1, GlobalData>
        + Dispatch<ExtSessionLockV1, SessionLockState>
        + SessionLockHandler
        + 'static,
{
    SessionLockManagerState::new::<D, _>(registrar.display(), |_| true)
}
