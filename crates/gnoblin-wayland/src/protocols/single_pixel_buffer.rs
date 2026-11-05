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

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;

use crate::State;

/// Creates the `wp_single_pixel_buffer_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> SinglePixelBufferState {
    SinglePixelBufferState::new::<State>(display)
}
