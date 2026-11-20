//! `gnoblin-wayland-zwp-linux-dmabuf-v1`: the import answer is the engine's, because it asks the backend's renderer and the
//! session's GPU memory budget.

use gnoblin_backend::{Backend, DmabufSupport};
use gnoblin_wayland_protocols::zwp_linux_dmabuf_v1::{DmabufServing, Host};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};

use crate::State;
use crate::gpu_memory::MemoryAdmission;
use crate::log;

impl Host for State {
    fn dmabuf(&mut self) -> &mut DmabufServing {
        &mut self.dmabuf
    }

    fn loop_handle(&self) -> Option<LoopHandle<'static, Self>> {
        self.loop_handle.clone()
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        self.dmabuf.state()
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        self.dmabuf.queue(dmabuf, notifier);
    }
}

impl State {
    /// Creates the global when the backend reports a renderer that can import a dmabuf.
    ///
    /// Called once, before any client connects, so a session that cannot draw a dmabuf never
    /// advertises the protocol.
    pub(crate) fn start_dmabuf(&mut self, support: Option<&DmabufSupport>, display: &DisplayHandle) {
        let Some(support) = support else {
            log::info!(
                area = "dmabuf",
                "the backend has no renderer that imports GPU buffers: zwp_linux_dmabuf_v1 is not served"
            );
            return;
        };
        let formats: Vec<_> = support.formats.iter().copied().collect();
        self.dmabuf.start::<State>(display, support.main_device, &formats);
    }

    /// Whether the session serves `zwp_linux_dmabuf_v1`.
    #[must_use]
    pub fn serves_dmabuf(&self) -> bool {
        self.dmabuf.served()
    }

    /// Answers every buffer a client sent since the last turn, with the backend's renderer.
    ///
    /// A buffer that imports is accepted; one that does not is failed, which the notifier turns
    /// into `failed` for `create` and a protocol error for `create_immed`. A client that went
    /// away meanwhile is not an error: its notifier answers nothing.
    pub(crate) fn apply_dmabuf_imports(&mut self, backend: &mut dyn Backend<State>) {
        for (dmabuf, notifier) in self.dmabuf.take_pending() {
            // The import is where a GPU buffer first costs the compositor anything, and where the backend is held, so
            // it is where the memory budget is asked: the client's own limit and the session's, with the caches given
            // back first when the buffer fits only that way. A client that is gone is not asked.
            let client = notifier.client();
            let bytes = gnoblin_wayland_protocols::zwp_linux_dmabuf_v1::dmabuf_bytes(&dmabuf);
            if let Some(client) = &client {
                match self.gpu_ledger.admit_buffer(&client.id(), bytes) {
                    Ok(MemoryAdmission::Fits) => {}
                    Ok(MemoryAdmission::Evict { .. }) => self.settle_gpu_memory(backend),
                    Err(refusal) => {
                        self.refuse_client(client, refusal);
                        notifier.failed();
                        continue;
                    }
                }
            }
            match backend.import_dmabuf(&dmabuf) {
                Ok(()) => {
                    // A client that disconnected has no buffer to hand the answer to.
                    if let Ok(buffer) = notifier.successful::<State>()
                        && let Some(client) = client
                        // A client that destroyed the buffer in the same turn it created it has nothing to count; its
                        // destruction was already seen, so recording now would never be given back.
                        && buffer.is_alive()
                    {
                        self.gpu_ledger
                            .record_buffer(&client.id(), buffer.id().protocol_id(), bytes);
                    }
                }
                Err(error) => {
                    log::warn!(area = "dmabuf", %error, "a client's GPU buffer is refused");
                    notifier.failed();
                }
            }
        }
    }
}
