//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_shm`, `wl_shm_pool`.
//!
//! `wl_shm` and `wl_shm_pool` (XML `wayland.xml`), plus the `wl_buffer` release hook.
//!
//! Smithay owns the pools and buffers; this module answers its handler traits.
//!
//! # Failure modes
//!
//! A buffer whose size, stride or format does not match its pool is a protocol error for
//! that client only. Guarded by `tests/render.rs`
//! (`a_stride_padded_buffer_is_read_from_the_right_offsets`).

use smithay::reexports::wayland_server::protocol::wl_shm::WlShm;
use smithay::reexports::wayland_server::protocol::wl_shm_pool::WlShmPool;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::shm::{ShmHandler, ShmPoolUserData, ShmState};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wl-shm",
    interfaces: &[ServedInterface::new("wl_shm", 2)],
    privilege: Privilege::Core,
};

/// Creates the `wl_shm` global. The two formats the protocol requires are always offered; a renderer adds the rest.
pub fn register<D>(registrar: &Registrar<'_>) -> ShmState
where
    D: GlobalDispatch<WlShm, GlobalData>
        + Dispatch<WlShm, GlobalData>
        + Dispatch<WlShmPool, ShmPoolUserData>
        + BufferHandler
        + ShmHandler
        + 'static,
{
    ShmState::new::<D>(registrar.display(), Vec::new())
}
