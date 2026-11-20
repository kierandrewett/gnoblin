//! XML protocol `single_pixel_buffer_v1` (`<protocol name="single_pixel_buffer_v1">`); interfaces served: `wp_single_pixel_buffer_manager_v1`.
//!
//! `wp_single_pixel_buffer_manager_v1`: a buffer that costs one pixel.
//!
//! A client that wants a solid-colour surface — a background, a cover, a cursor-sized
//! block — normally allocates a whole `wl_shm` buffer of that colour. This protocol lets
//! it name the colour instead, and the compositor provides the pixels. On a GL stack the
//! win is memory; here it is the same, plus one fewer object per surface.
//!
//! Smithay creates and parses these buffers: `get_single_pixel_buffer` answers the
//! colour in premultiplied RGBA. What a compositor has to add is the step where the frame
//! path recognises one, because the pixels are not in a shared-memory file and a renderer
//! that only understands `wl_shm` would find nothing to copy. That step belongs to
//! `gnoblin-backend`'s scene, not to this module: the global exists so a client can ask,
//! and a client that asks gets a buffer object it can attach.
//!
//! # Failure modes
//!
//! 1. **A single-pixel buffer is composed as if it had pixels.** The buffer carries its
//!    colour in user data and no memory; a reader that assumes a mapping fails loudly in
//!    `gnoblin-backend` rather than drawing uninitialised pixels.
//! 2. **A destroyed manager leaves buffers unusable.** The buffer objects outlive the
//!    manager object, as the protocol states, so destroying the manager does not destroy
//!    a buffer a surface still uses.

use smithay::reexports::wayland_protocols::wp::single_pixel_buffer::v1::server::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-single-pixel-buffer-v1",
    interfaces: &[ServedInterface::new("wp_single_pixel_buffer_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `wp_single_pixel_buffer_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> SinglePixelBufferState
where
    D: GlobalDispatch<WpSinglePixelBufferManagerV1, GlobalData>
        + Dispatch<WpSinglePixelBufferManagerV1, GlobalData>
        + 'static,
{
    SinglePixelBufferState::new::<D>(registrar.display())
}
