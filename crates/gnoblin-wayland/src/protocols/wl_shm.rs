//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_shm`, `wl_shm_pool`.
//!
//! `wl_shm` and `wl_shm_pool` (XML `wayland.xml`), plus the `wl_buffer` release hook.
//!
//! Smithay owns the pools and buffers; this module answers its handler traits.
//!
//! # Failure modes
//!
//! A buffer whose size, stride or format does not match its pool is a protocol error for
//! that client only. Guarded by `tests/window_frame.rs`
//! (`a_buffer_of_the_wrong_size_is_a_protocol_error`) and `tests/render.rs`
//! (`a_stride_padded_buffer_is_read_from_the_right_offsets`).

use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::shm::{ShmHandler, ShmState};

use crate::State;

/// A buffer is gone: Smithay has already released whatever it tracked.
impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
