//! `gnoblin-wayland-wl-shm`: Smithay owns the pools and buffers; the state answers its handler traits.

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
