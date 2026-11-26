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
//! once per turn (`State::apply_dmabuf_imports`, in the compositor) with the backend's renderer in hand, because a
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
//! - **Close fade.** A window that closes or unmaps fades out its last picture. A shared-memory window keeps a
//!   copy of its pixels; a dmabuf window keeps the buffer's handle, which owns the plane descriptors and so the
//!   memory, and the renderer draws it again for the fade. A client that rewrites the buffer while the fade runs
//!   shows through it.
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

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Format};
use smithay::reexports::calloop::{Interest, LoopHandle};
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, GlobalDispatch, Resource};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker, add_pre_commit_hook, with_states,
};
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufGlobalData, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf,
};
use smithay::wayland::drm_syncobj::DrmSyncobjCachedState;

use gnoblin_wayland_api::host::CoreHost;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises. The global exists only when the backend reports a renderer that imports GPU buffers, so the
/// table lists it as conditional, and version 4 needs a render node: a session without one serves version 3.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-linux-dmabuf-v1",
    interfaces: &[ServedInterface::new("zwp_linux_dmabuf_v1", 4)],
    privilege: Privilege::Core,
};

/// What the protocol needs of the compositor.
pub trait Host:
    CoreHost
    + CompositorHandler
    + DmabufHandler
    + BufferHandler
    + GlobalDispatch<ZwpLinuxDmabufV1, DmabufGlobalData>
    + Sized
    + 'static
{
    /// The global and the buffers waiting for the loop's answer.
    fn dmabuf(&mut self) -> &mut DmabufServing;

    /// The loop a buffer's fences are watched on, once the session has one.
    fn loop_handle(&self) -> Option<LoopHandle<'static, Self>>;
}

/// The dmabuf global and the buffers waiting for the loop's answer.
pub struct DmabufServing {
    state: DmabufState,
    /// The global, once the backend reported support; `None` for a session that does not serve it.
    global: Option<DmabufGlobal>,
    /// The buffers clients sent, each with the notifier that tells its client the outcome.
    pending: Vec<(Dmabuf, ImportNotifier)>,
}

impl Default for DmabufServing {
    fn default() -> Self {
        Self::new()
    }
}

impl DmabufServing {
    /// A table with no global.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: DmabufState::new(),
            global: None,
            pending: Vec::new(),
        }
    }

    /// Whether the session serves `zwp_linux_dmabuf_v1`.
    #[must_use]
    pub const fn served(&self) -> bool {
        self.global.is_some()
    }

    /// Smithay's state, for the compositor's `DmabufHandler`.
    pub fn state(&mut self) -> &mut DmabufState {
        &mut self.state
    }

    /// Queues a buffer a client sent, with the notifier that answers it.
    pub fn queue(&mut self, dmabuf: Dmabuf, notifier: ImportNotifier) {
        self.pending.push((dmabuf, notifier));
    }

    /// Takes the buffers waiting for an answer, leaving none.
    pub fn take_pending(&mut self) -> Vec<(Dmabuf, ImportNotifier)> {
        std::mem::take(&mut self.pending)
    }

    /// Creates the global when the renderer can import a dmabuf: version 4 with a feedback when it names a render node
    /// (`main_device`), version 3 without. A renderer that offers no format serves nothing.
    ///
    /// Called once, before any client connects, so a session that cannot draw a dmabuf never advertises the protocol.
    pub fn start<D: Host>(&mut self, display: &DisplayHandle, main_device: Option<u64>, formats: &[Format]) {
        if formats.is_empty() {
            log::info!(
                area = "dmabuf",
                "the renderer offers no importable format: zwp_linux_dmabuf_v1 is not served"
            );
            return;
        }
        let global = match main_device {
            Some(device) => match DmabufFeedbackBuilder::new(device, formats.iter().copied()).build() {
                Ok(feedback) => self.state.create_global_with_default_feedback::<D>(display, &feedback),
                Err(error) => {
                    log::warn!(
                        area = "dmabuf",
                        %error,
                        "the format table could not be built; serving version 3 without feedback"
                    );
                    self.state.create_global::<D>(display, formats.iter().copied())
                }
            },
            None => self.state.create_global::<D>(display, formats.iter().copied()),
        };
        log::info!(
            area = "dmabuf",
            formats = formats.len(),
            device = ?main_device,
            "zwp_linux_dmabuf_v1 is served"
        );
        self.global = Some(global);
    }
}

/// What a GPU buffer is counted as holding: its width times height times the bytes per pixel of its format, which is
/// an estimate (a tiled or padded buffer holds more, a multi-planar one reports its average), and four bytes a pixel
/// for a format the table does not know.
pub fn dmabuf_bytes(dmabuf: &Dmabuf) -> u64 {
    let size = dmabuf.size();
    let pixels = u64::try_from(size.w)
        .unwrap_or(0)
        .saturating_mul(u64::try_from(size.h).unwrap_or(0));
    let bits = smithay::backend::allocator::format::get_bpp(dmabuf.format().code).unwrap_or(32);
    pixels.saturating_mul(u64::try_from(bits).unwrap_or(32)) / 8
}

/// Holds a surface's commit until the dmabuf it attaches is ready to be read.
///
/// A client's GPU work is queued, not done, when it commits: the buffer's fences say when it
/// finishes. A commit applied early would show a half-drawn frame. The hook adds a blocker that
/// clears when the buffer's fences signal, and the compositor keeps showing the previous frame
/// until then. A buffer that is already ready adds nothing, and so does a surface whose client
/// is gone or a session with no loop to wait on. A commit with an explicit acquire point is not held here: the point is
/// its wait.
pub fn hold_commit_until_ready<D: Host>(surface: &WlSurface) {
    add_pre_commit_hook::<D, _>(surface, |state, _display, surface| {
        let attached = with_states(surface, |states| {
            // A commit that names an acquire point waits on that point and not on the buffer's implicit fences
            // (`gnoblin-wayland-wp-linux-drm-syncobj-v1`): the client may not attach any, and the protocol lets the
            // compositor ignore them.
            if states
                .cached_state
                .get::<DrmSyncobjCachedState>()
                .pending()
                .acquire_point
                .is_some()
            {
                return None;
            }
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
        let (Some(client), Some(handle)) = (surface.client(), state.loop_handle()) else {
            return;
        };
        let inserted = handle.insert_source(source, move |_, _, state: &mut D| {
            let display = state.display_handle().clone();
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
