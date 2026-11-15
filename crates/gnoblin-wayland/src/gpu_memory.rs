//! The memory an output holds, published on its screen node (`gpu_memory_*`, design/gpu-memory.md).
//!
//! The numbers come from two places that only this crate sees together: the renderer's own footprint (the backend
//! states it) and the frames capture keeps. They are published after every iteration, and the tree announces a value
//! only when it differs from the node's, so a steady session publishes nothing. Read-only: nothing here limits or
//! changes an allocation.

use gnoblin_backend::Backend;

use crate::State;

impl State {
    /// Publishes, for every output, the bytes its renderer holds and the bytes capture keeps of its last frame.
    pub(crate) fn publish_gpu_memory(&mut self, backend: &dyn Backend<State>) {
        for output in self.output_ids() {
            let footprint = backend.footprint(output);
            let capture = self.capture_cache_bytes(output);
            self.composer.set_output_memory(output, &footprint, capture);
        }
    }
}
