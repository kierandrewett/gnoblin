//! XML protocol `linux-dmabuf-v1` (`<protocol name="linux_dmabuf_v1">`); interfaces served: `zwp_linux_dmabuf_v1`, `zwp_linux_buffer_params_v1`, `zwp_linux_dmabuf_feedback_v1`.
//!
//! `zwp_linux_dmabuf_v1`: a client shares a GPU buffer instead of copying pixels into shared memory.
//!
//! Browsers, games and video players render on the GPU. Without this protocol they fall back to
//! `wl_shm` (a copy per frame) or refuse to start. Smithay owns the protocol objects, the
//! parameter validation and the feedback; this module answers its handler trait and decides the
//! two things only the compositor can: whether the global exists, and whether one buffer is
//! accepted.
//!
//! # When the global exists
//!
//! Only when the backend reports a renderer that can import a dmabuf ([`Backend::dmabuf`]). The
//! GLES renderer on a display with `EGL_EXT_image_dma_buf_import` does; the software renderer, the
//! Vulkan renderer and a session with no output do not, and in those sessions the global is never
//! created, so a client sees no `zwp_linux_dmabuf_v1` and shares memory. A global that was
//! advertised and could not be drawn would be worse than none.
//!
//! With a render node the global is version 4 and up: clients get the main device and a format
//! table (`zwp_linux_dmabuf_feedback_v1`), so they allocate on the GPU the compositor draws with.
//! Without a node (a surfaceless context) it is version 3: format and modifier events and no
//! device. Per-surface feedback, with a scan-out tranche, is not offered: no surface is scanned
//! out directly, because every frame is composed.
//!
//! # How a buffer is accepted
//!
//! Smithay checks the parameters (a known format code, positive size, planes inside their file,
//! consecutive plane indices). `dmabuf_imported` then queues the buffer, and the loop answers it
//! once per turn ([`State::apply_dmabuf_imports`]) with the backend's renderer in hand, because a
//! protocol handler holds the state and not the backend:
//!
//! 1. the format *and modifier* must be among those offered (Smithay checks the code alone);
//! 2. a linear or implicit buffer must be one plane;
//! 3. the renderer imports it, so a buffer the GPU cannot read fails here and not on the draw.
//!
//! A refusal is the protocol's own failure: `failed` for `create`, and a protocol error
//! (`invalid_wl_buffer`) for `create_immed`, which Smithay posts when the notifier says failed.
//!
//! # Rendering, damage, release, unmap
//!
//! A surface whose buffer is a dmabuf is drawn from [`SurfaceSource::Dmabuf`]: the GLES renderer
//! samples the imported texture in place. Damage, `wl_buffer.release`, frame callbacks and unmap
//! are the surface machinery's and do not look at the buffer's kind. Two things differ from shm:
//!
//! - **Readiness.** A client's GPU work may be unfinished when it commits. A commit that attaches a
//!   dmabuf waits on a blocker for the buffer's fences ([`hold_commit_until_ready`]), so the
//!   compositor shows the previous frame until the new one is complete.
//! - **Close fade.** A window that closes fades out its last picture, read back from its buffer.
//!   A dmabuf has no CPU picture, so a window that shows one leaves at once.
//!
//! # Failure modes
//!
//! 1. **The global is advertised in a session that cannot draw it.** The software session has no
//!    `zwp_linux_dmabuf_v1`: `a_software_session_does_not_serve_dmabuf` in `tests/dmabuf.rs`.
//! 2. **A format outside the offer is imported.** The pair is checked against the offer, so an
//!    unadvertised modifier of an offered code is refused:
//!    `a_modifier_that_was_not_offered_is_refused`.
//! 3. **A buffer the GPU refuses is shown as a black window.** The import answers before the
//!    client may use the buffer: `a_buffer_the_importer_refuses_fails_the_request`.
//! 4. **A client sends a malformed buffer.** Smithay's own checks post the protocol errors:
//!    `a_zero_sized_buffer_is_a_protocol_error`, `an_unoffered_format_is_a_protocol_error`,
//!    `a_second_use_of_the_params_is_a_protocol_error`, `an_incomplete_params_is_a_protocol_error`.
//! 5. **A version 4 client gets no device.** With a render node, a bound client receives the main
//!    device and a format table: `a_version_4_client_is_told_the_main_device`.
//! 6. **A dmabuf commit is applied before the GPU is done.** The commit holds behind a blocker on
//!    the buffer's fences; a buffer that is already ready adds none: `a_commit_waits_for_the_buffers_fences`.
//!
//! [`Backend::dmabuf`]: gnoblin_backend::Backend::dmabuf
//! [`SurfaceSource::Dmabuf`]: gnoblin_render_frame::frame::SurfaceSource::Dmabuf

use gnoblin_backend::{Backend, DmabufSupport};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::reexports::calloop::Interest;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::wayland::compositor::{
    BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker, add_pre_commit_hook, with_states,
};
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf,
};

use crate::State;
use crate::log;

/// The dmabuf global and the buffers waiting for the loop's answer.
pub(crate) struct DmabufServing {
    state: DmabufState,
    /// The global, once the backend reported support; `None` for a session that does not serve it.
    global: Option<DmabufGlobal>,
    /// The buffers clients sent, each with the notifier that tells its client the outcome.
    pending: Vec<(Dmabuf, ImportNotifier)>,
}

impl DmabufServing {
    pub(crate) fn new() -> Self {
        Self {
            state: DmabufState::new(),
            global: None,
            pending: Vec::new(),
        }
    }

    /// Whether the session serves `zwp_linux_dmabuf_v1`.
    pub(crate) fn served(&self) -> bool {
        self.global.is_some()
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf.state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        self.dmabuf.pending.push((dmabuf, notifier));
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
        if formats.is_empty() {
            log::info!(
                area = "dmabuf",
                "the renderer offers no importable format: zwp_linux_dmabuf_v1 is not served"
            );
            return;
        }
        let global = match support.main_device {
            Some(device) => match DmabufFeedbackBuilder::new(device, formats.iter().copied()).build() {
                Ok(feedback) => self
                    .dmabuf
                    .state
                    .create_global_with_default_feedback::<State>(display, &feedback),
                Err(error) => {
                    log::warn!(
                        area = "dmabuf",
                        %error,
                        "the format table could not be built; serving version 3 without feedback"
                    );
                    self.dmabuf
                        .state
                        .create_global::<State>(display, formats.iter().copied())
                }
            },
            None => self
                .dmabuf
                .state
                .create_global::<State>(display, formats.iter().copied()),
        };
        log::info!(
            area = "dmabuf",
            formats = formats.len(),
            device = ?support.main_device,
            "zwp_linux_dmabuf_v1 is served"
        );
        self.dmabuf.global = Some(global);
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
        for (dmabuf, notifier) in std::mem::take(&mut self.dmabuf.pending) {
            match backend.import_dmabuf(&dmabuf) {
                Ok(()) => {
                    // A client that disconnected has no buffer to hand the answer to.
                    let _ = notifier.successful::<State>();
                }
                Err(error) => {
                    log::warn!(area = "dmabuf", %error, "a client's GPU buffer is refused");
                    notifier.failed();
                }
            }
        }
    }
}

/// Holds a surface's commit until the dmabuf it attaches is ready to be read.
///
/// A client's GPU work is queued, not done, when it commits: the buffer's fences say when it
/// finishes. A commit applied early would show a half-drawn frame. The hook adds a blocker that
/// clears when the buffer's fences signal, and the compositor keeps showing the previous frame
/// until then. A buffer that is already ready adds nothing, and so does a surface whose client
/// is gone or a session with no loop to wait on.
pub(crate) fn hold_commit_until_ready(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _display, surface| {
        let attached = with_states(surface, |states| {
            let mut attributes = states.cached_state.get::<SurfaceAttributes>();
            match &attributes.pending().buffer {
                Some(BufferAssignment::NewBuffer(buffer)) => get_dmabuf(buffer).ok().cloned(),
                _ => None,
            }
        });
        let Some(dmabuf) = attached else {
            return;
        };
        // An error here means the fences are already signalled: nothing to wait for.
        let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) else {
            return;
        };
        let (Some(client), Some(handle)) = (surface.client(), state.loop_handle.clone()) else {
            return;
        };
        let inserted = handle.insert_source(source, move |_, _, state: &mut State| {
            let display = state.display_handle.clone();
            state.client_compositor_state(&client).blocker_cleared(state, &display);
            Ok(())
        });
        match inserted {
            Ok(_) => add_blocker(surface, blocker),
            Err(error) => {
                log::warn!(
                    area = "dmabuf",
                    ?error,
                    "a buffer's fences could not be watched; the commit is not held"
                );
            }
        }
    });
}
