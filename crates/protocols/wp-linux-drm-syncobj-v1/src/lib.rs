//! XML protocol `linux-drm-syncobj-v1` (`<protocol name="linux_drm_syncobj_v1">`); interfaces served: `wp_linux_drm_syncobj_manager_v1`, `wp_linux_drm_syncobj_surface_v1`, `wp_linux_drm_syncobj_timeline_v1`.
//!
//! `wp_linux_drm_syncobj_manager_v1`: explicit synchronisation of a client's GPU buffers.
//!
//! A client imports a DRM synchronisation object timeline and, with each commit that attaches a dmabuf, names two points
//! of it: an *acquire* point that signals when the buffer is ready to read, and a *release* point the compositor signals
//! when it has finished reading. Smithay owns the protocol objects and most of the validation; this module decides what
//! only the compositor can: whether the global exists, how a commit waits for its acquire point, and when a release point
//! signals.
//!
//! # When the global exists
//!
//! Only when the backend can keep the promise ([`Backend::explicit_sync`]): its renderer reports when the GPU has finished
//! each frame, and its DRM device waits on a timeline point through an eventfd. Vulkan on a device with a render node
//! does. GLES, the software renderer and a software Vulkan driver (lavapipe: no DRM node) do not, so a client there sees
//! no global and keeps implicit sync. A global that was advertised and could not be honoured would let a client write
//! into a buffer still being drawn.
//!
//! # Acquire: the commit waits, the renderer never does
//!
//! A pre-commit hook adds a blocker on the commit's acquire point: the kernel signals an eventfd the loop watches, and the
//! commit is applied when it fires. The compositor shows the previous frame meanwhile, so the renderer is only ever handed
//! a buffer that is ready. A commit with an acquire point is not held on the dmabuf's implicit fences as well
//! (`gnoblin_wayland_zwp_linux_dmabuf_v1::hold_commit_until_ready`): the client may attach none.
//!
//! # Release: after the GPU is done, not after the commit is replaced
//!
//! Smithay signals a release point when its `Buffer` is dropped, which happens at the next commit, while a frame that read
//! the buffer may still run on the GPU. Here the point is taken out of the surface's state as the commit is applied
//! ([`take_points`]) and kept in [`SyncobjServing`] with the surface's *generation*, a count of the commits that changed
//! what the surface shows. A point is due once the surface shows something newer (or is gone), and it is signalled when
//! the GPU has retired every frame submitted up to the moment that was seen ([`SyncobjServing::retire`], fed by
//! `Backend::buffers_retired`). A commit that is never applied (replaced in a synchronised subsurface's cache, or a surface
//! destroyed first) is released by Smithay at once, because nothing read it.
//!
//! # Protocol errors
//!
//! Smithay posts `surface_exists`, `invalid_timeline`, `no_buffer`, `no_acquire_point` (a release point alone),
//! `no_release_point`, `conflicting_points` and `unsupported_buffer`. It does not post `no_acquire_point` for a buffer
//! attached with no point at all, which the protocol requires of a surface that has the object; the pre-commit hook here
//! does ([`check_commit`]).
//!
//! # Failure modes
//!
//! 1. **The global is advertised where it cannot work.** A software, GLES or lavapipe session does not serve it:
//!    `a_session_without_a_retiring_gpu_renderer_does_not_serve_syncobj` and
//!    `a_session_on_a_software_vulkan_device_does_not_serve_syncobj` in `gnoblin-wayland/tests/syncobj.rs`.
//! 2. **A second object for a surface, a timeline that cannot be imported, a buffer with no points.** Protocol errors:
//!    `a_second_syncobj_surface_is_a_protocol_error`, `a_timeline_that_cannot_be_imported_is_a_protocol_error`,
//!    `a_buffer_with_no_points_is_a_protocol_error`.
//! 3. **A commit is applied before its buffer is ready.** It is held behind a blocker on the acquire point:
//!    `a_commit_waits_for_its_acquire_point` (needs a DRM device).
//! 4. **A release point signals under a draw.** It waits for the retirement of every frame that may have read the buffer:
//!    `a_release_point_signals_after_the_frame_retires` (needs a DRM device).
//! 5. **A client that reuses a buffer deadlocks on its own release.** A newer commit releases an older one even when it
//!    names the same `wl_buffer`: `a_buffer_attached_again_releases_the_earlier_commit` (needs a DRM device).
//! 6. **A rebuilt device leaves releases waiting for ever.** A retirement count that goes backwards releases the points
//!    of replaced content at once: `a_device_made_anew_releases_every_replaced_point` (needs a DRM device).
//!
//! [`Backend::explicit_sync`]: gnoblin_backend::Backend::explicit_sync

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::renderer::sync::Fence;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_manager_v1::WpLinuxDrmSyncobjManagerV1;
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_surface_v1::{
    Error as SurfaceError, WpLinuxDrmSyncobjSurfaceV1,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, GlobalDispatch, Resource, Weak};
use smithay::wayland::compositor::{
    BufferAssignment, CompositorHandler, SurfaceAttributes, TraversalAction, add_blocker, add_pre_commit_hook,
    is_sync_subsurface, with_states, with_surface_tree_upward,
};
use smithay::wayland::drm_syncobj::{
    DrmSyncPoint, DrmSyncobjCachedState, DrmSyncobjGlobalData, DrmSyncobjHandler, DrmSyncobjState,
};

use gnoblin_wayland_api::host::CoreHost;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises. Only a session whose backend can keep the promise creates the global, so the table
/// lists it as conditional.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-linux-drm-syncobj-v1",
    interfaces: &[ServedInterface::new("wp_linux_drm_syncobj_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// What the protocol needs of the compositor.
pub trait Host:
    CoreHost
    + CompositorHandler
    + DrmSyncobjHandler
    + GlobalDispatch<WpLinuxDrmSyncobjManagerV1, DrmSyncobjGlobalData>
    + Sized
    + 'static
{
    /// The global and the release points waiting for the GPU.
    fn syncobj(&mut self) -> &mut SyncobjServing;

    /// The loop an acquire point's eventfd is watched on, once the session has one.
    fn loop_handle(&self) -> Option<LoopHandle<'static, Self>>;
}

/// How many commits of a surface changed what it shows.
///
/// Stored in the surface's data, and read by [`SyncobjServing::retire`] to tell whether a held release point is of the
/// surface's current content.
struct ContentGeneration(AtomicU64);

/// A release point taken from a commit that was applied, with the generation of the content it belongs to.
#[derive(Debug)]
pub struct ReleaseTaken {
    surface: Weak<WlSurface>,
    generation: u64,
    point: DrmSyncPoint,
}

/// A release point held until the GPU is done.
#[derive(Debug)]
struct ReleaseHold {
    surface: Weak<WlSurface>,
    generation: u64,
    point: DrmSyncPoint,
    /// The number of the last submission when the content was found replaced: every frame that may have read the buffer
    /// is at or before it. `None` while the content is current.
    after: Option<u64>,
}

/// The explicit sync global and the release points waiting for the GPU.
#[derive(Default)]
pub struct SyncobjServing {
    state: Option<DrmSyncobjState>,
    holds: Vec<ReleaseHold>,
    /// The last submission count read, to see a device made anew.
    seen: u64,
}

impl SyncobjServing {
    /// A table with no global.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the session serves `wp_linux_drm_syncobj_manager_v1`.
    #[must_use]
    pub const fn served(&self) -> bool {
        self.state.is_some()
    }

    /// Smithay's state, for the compositor's `DrmSyncobjHandler`.
    pub fn state_mut(&mut self) -> Option<&mut DrmSyncobjState> {
        self.state.as_mut()
    }

    /// How many release points wait for the GPU.
    #[must_use]
    pub fn held(&self) -> usize {
        self.holds.len()
    }

    /// Creates the global, importing timelines on `device`. The backend vouched for the device
    /// (`gnoblin_backend::ExplicitSync`), so nothing is asked of the kernel here.
    ///
    /// Called once, before any client connects.
    pub fn start<D: Host>(&mut self, display: &DisplayHandle, device: DrmDeviceFd) {
        self.state = Some(DrmSyncobjState::new::<D>(display, device));
        log::info!(area = "syncobj", "wp_linux_drm_syncobj_manager_v1 is served");
    }

    /// Keeps the release points a commit took, until the GPU is done with what they release.
    pub fn hold(&mut self, taken: Vec<ReleaseTaken>) {
        self.holds.extend(taken.into_iter().map(|taken| ReleaseHold {
            surface: taken.surface,
            generation: taken.generation,
            point: taken.point,
            after: None,
        }));
    }

    /// Signals the release points the GPU is done with.
    ///
    /// `submitted` is the number of the last submission and `retired` how many of them have finished (see
    /// `gnoblin_backend::Retirement`). A point whose content was replaced is stamped with `submitted`, the last frame
    /// that may have read it, and signalled once `retired` reaches the stamp. A `submitted` below the last one seen is a
    /// device made anew: the frames that read the replaced content are gone, so those points are signalled at once. A
    /// point of the content a surface still shows stays held, because the new device reads it again.
    pub fn retire(&mut self, submitted: u64, retired: u64) {
        if self.holds.is_empty() {
            self.seen = submitted;
            return;
        }
        let renewed = submitted < self.seen;
        self.seen = submitted;
        for hold in &mut self.holds {
            if hold.after.is_none() && replaced(hold) {
                hold.after = Some(submitted);
            }
        }
        self.holds.retain(|hold| {
            let done = hold.after.is_some_and(|after| renewed || after <= retired);
            if done && let Err(error) = hold.point.signal() {
                // The client's timeline or the device is gone: nobody waits on the point.
                log::debug!(area = "syncobj", %error, "a release point could not be signalled");
            }
            !done
        });
    }
}

/// Whether the surface shows something newer than the content `hold` releases, or is gone.
fn replaced(hold: &ReleaseHold) -> bool {
    let Ok(surface) = hold.surface.upgrade() else {
        return true;
    };
    with_states(&surface, |states| {
        states
            .data_map
            .get::<ContentGeneration>()
            .is_none_or(|current| current.0.load(Ordering::Acquire) != hold.generation)
    })
}

/// Takes the release points the commit just applied named, and counts the commit when it changed what the surface shows.
///
/// Called right before Smithay's `on_commit_buffer_handler`, which would otherwise own the point and signal it when the
/// next commit replaces the buffer, whatever the GPU is doing. The tree is walked as that handler walks it, because a
/// commit of a parent applies the cached commits of its synchronised subsurfaces. A synchronised subsurface's own commit
/// applies nothing and takes nothing.
#[must_use]
pub fn take_points(surface: &WlSurface) -> Vec<ReleaseTaken> {
    let mut taken = Vec::new();
    if is_sync_subsurface(surface) {
        return taken;
    }
    with_surface_tree_upward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |tree_surface, states, _| {
            let changed = states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer
                .is_some();
            let (release, _acquire) = {
                let mut cached = states.cached_state.get::<DrmSyncobjCachedState>();
                let current = cached.current();
                // The acquire point was waited on before the commit applied.
                (current.release_point.take(), current.acquire_point.take())
            };
            let known = states.data_map.get::<ContentGeneration>().is_some();
            if release.is_none() && !(changed && known) {
                return;
            }
            states
                .data_map
                .insert_if_missing_threadsafe(|| ContentGeneration(AtomicU64::new(0)));
            let Some(generation) = states.data_map.get::<ContentGeneration>() else {
                return;
            };
            // A commit that attaches or removes a buffer replaces what the surface showed, so the points of the earlier
            // content are due; the point this commit names belongs to the new content.
            let number = if changed {
                generation.0.fetch_add(1, Ordering::AcqRel) + 1
            } else {
                generation.0.load(Ordering::Acquire)
            };
            if let Some(point) = release {
                taken.push(ReleaseTaken {
                    surface: tree_surface.downgrade(),
                    generation: number,
                    point,
                });
            }
        },
        |_, _, _| true,
    );
    taken
}

/// Checks a commit against what the protocol requires and Smithay does not: a surface that has the explicit sync object
/// and attaches a buffer must name both points.
fn check_commit(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        let Some(object) = states.data_map.get::<RefCell<Option<WpLinuxDrmSyncobjSurfaceV1>>>() else {
            return true;
        };
        let object = object.borrow();
        let Some(object) = object.as_ref() else {
            return true;
        };
        let attaches = matches!(
            states.cached_state.get::<SurfaceAttributes>().pending().buffer,
            Some(BufferAssignment::NewBuffer(_))
        );
        let mut cached = states.cached_state.get::<DrmSyncobjCachedState>();
        let pending = cached.pending();
        if attaches && pending.acquire_point.is_none() && pending.release_point.is_none() {
            object.post_error(
                SurfaceError::NoAcquirePoint,
                "a buffer was attached with no acquire point".to_owned(),
            );
            return false;
        }
        true
    })
}

/// Holds a surface's commit until the acquire point it names has signalled, and refuses a commit that breaks the protocol.
///
/// The kernel signals an eventfd when the point does, and the loop clears the blocker. A point that has already signalled
/// adds nothing, and so does a surface whose client is gone or a session with no loop to wait on.
pub fn hold_commit_until_acquired<D: Host>(surface: &WlSurface) {
    add_pre_commit_hook::<D, _>(surface, |state, _display, surface| {
        if !check_commit(surface) {
            return;
        }
        let acquire = with_states(surface, |states| {
            states
                .cached_state
                .get::<DrmSyncobjCachedState>()
                .pending()
                .acquire_point
                .clone()
        });
        let Some(point) = acquire else {
            return;
        };
        if point.is_signaled() {
            return;
        }
        let (blocker, source) = match point.generate_blocker() {
            Ok(pair) => pair,
            Err(error) => {
                log::warn!(
                    area = "syncobj",
                    %error,
                    "an acquire point could not be watched; the commit is not held"
                );
                return;
            }
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
                    area = "syncobj",
                    ?error,
                    "an acquire point could not be watched; the commit is not held"
                );
            }
        }
    });
}
