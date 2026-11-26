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
//! Only when the backend can keep the promise (`Backend::explicit_sync`): its renderer reports when the GPU has finished
//! each frame, and its DRM device waits on a timeline point through an eventfd. Vulkan on a device with a render node
//! does, and the headless and nested backends answer for it. The DRM backend implements neither `explicit_sync` nor
//! `buffers_retired`, so a session on real outputs has no global. GLES, the software renderer and a software Vulkan
//! driver (lavapipe: no DRM node) do not either, so a client there sees no global and keeps implicit sync. A global that
//! was advertised and could not be honoured would let a client write into a buffer still being drawn.
//!
//! # Acquire: the commit waits, the renderer never does
//!
//! A pre-commit hook adds a blocker on the commit's acquire point: the kernel signals an eventfd the loop watches, and the
//! commit is applied when it fires. The compositor shows the previous frame meanwhile, so the renderer is only ever handed
//! a buffer that is ready. A commit with an acquire point is not held on the dmabuf's implicit fences as well
//! (`gnoblin_wayland_zwp_linux_dmabuf_v1::hold_commit_until_ready`): the client may attach none.
//!
//! # What a client may make the compositor hold
//!
//! Every wait holds an eventfd and a source of the loop for as long as the client leaves the point unsignalled, and
//! nothing obliges it to. The hook keeps each commit that names points in the data of its surface (`QueuedCommits`),
//! with the loop's token for its wait, until the commit is applied ([`take_points`]). The surface's destruction, which a
//! client's disconnect also causes for every surface it had, removes the sources and closes the eventfds.
//!
//! A surface may have [`SURFACE_COMMIT_LIMIT`] commits queued, and a client [`CLIENT_WAIT_LIMIT`] waits across its
//! surfaces. A commit past either ends the client with `wl_display.error.no_memory` ([`Host::refuse_waits`]). The protocol
//! has no error for a limit, a silently dropped commit would leave the client waiting for a release that never comes, and
//! `no_memory` is what the compositor already answers a client past its surface or buffer budget, so a client sees one
//! kind of refusal for every resource it overdraws. The limits are a swapchain's depth several times over: a client that
//! reaches one is not drawing.
//!
//! # Release: after the GPU is done, not after the commit is replaced
//!
//! Smithay signals a release point when its `Buffer` is dropped, which happens at the next commit, while a frame that read
//! the buffer may still run on the GPU. Here the point is taken out of the surface's state as the commit is applied
//! ([`take_points`]) and kept in [`SyncobjServing`] with the surface's *generation*, a count of the commits that changed
//! what the surface shows. A point is due once the surface shows something newer (or is gone), and it is signalled when
//! the GPU has retired every frame submitted up to the moment that was seen ([`SyncobjServing::retire`], fed by
//! `Backend::buffers_retired`). A commit that is never applied (replaced in a synchronised subsurface's cache, or a surface
//! destroyed first) is released at once, because nothing read it: by Smithay for the points it still holds as pending or
//! current, and for a commit still queued behind an acquire point by the surface's destruction hook here, which Smithay's
//! does not reach.
//!
//! The loop wakes every 100 ms when nothing else wakes it, and the GPU finishing a frame wakes nothing. While a point
//! waits for the GPU the loop waits at most [`SyncobjServing::POLL`] instead ([`SyncobjServing::awaiting_gpu`]), so an
//! idle session releases a buffer within a few milliseconds of its last frame retiring, and one with nothing to release
//! pays nothing.
//!
//! # Protocol errors
//!
//! Smithay posts `surface_exists`, `invalid_timeline`, `no_surface`, `no_buffer` (an acquire point with no buffer),
//! `no_acquire_point` (a release point alone), `no_release_point`, `conflicting_points` and `unsupported_buffer`. Here the
//! pre-commit hook (`check_commit`) raises what Smithay misses or names differently, before Smithay's own hook runs:
//!
//! - a buffer attached with no point at all is `no_acquire_point`, which the protocol requires of a surface that has the
//!   object and Smithay does not post;
//! - a point of either kind with no buffer attached in the commit is `no_buffer`, which is what the protocol says for a
//!   release point alone and Smithay answers `no_acquire_point`.
//!
//! Smithay stores the object a surface got in a cell it makes once, and a second `get_surface` after the object was
//! destroyed leaves the cell empty, so no error is raised for that surface any more. [`adopt_surface_object`] puts the new
//! object in, as the dispatch glue's request guard calls it.
//!
//! # Failure modes
//!
//! Every one has a test in `gnoblin-wayland/tests/syncobj.rs`; those that need real timelines run on a DRM render node.
//!
//! 1. **The global is advertised where it cannot work.** A software, GLES or lavapipe session does not serve it:
//!    `a_session_without_a_retiring_gpu_renderer_does_not_serve_syncobj` and
//!    `a_session_on_a_software_vulkan_device_does_not_serve_syncobj`.
//! 2. **A second object for a surface, a timeline that cannot be imported, a buffer with no points.** Protocol errors:
//!    `a_second_syncobj_surface_is_a_protocol_error`, `a_timeline_that_cannot_be_imported_is_a_protocol_error`,
//!    `a_buffer_with_no_points_is_a_protocol_error`.
//! 3. **A commit is applied before its buffer is ready.** It is held behind a blocker on the acquire point:
//!    `a_commit_waits_for_its_acquire_point`.
//! 4. **A release point signals under a draw.** It waits for the retirement of every frame that may have read the buffer:
//!    `a_release_point_signals_after_the_frame_retires`.
//! 5. **A client that reuses a buffer deadlocks on its own release.** A newer commit releases an older one even when it
//!    names the same `wl_buffer`: `a_buffer_attached_again_releases_the_earlier_commit`.
//! 6. **A rebuilt device leaves releases waiting for ever.** A retirement count that goes backwards releases the points
//!    of replaced content at once: `a_device_made_anew_releases_every_replaced_point`.
//! 7. **A client exhausts the compositor's descriptors with acquire points it never signals.** The limits end it:
//!    `a_surface_with_too_many_waiting_commits_ends_its_client`,
//!    `a_client_with_too_many_waits_across_surfaces_is_ended`; a client that signals its points is never counted past a
//!    limit: `a_client_that_signals_its_points_never_reaches_its_limits`.
//! 8. **The waits of a surface or a client that is gone outlive it.** `a_surface_destroyed_mid_wait_frees_its_waits`,
//!    `a_client_that_disconnects_mid_wait_frees_its_waits`.
//! 9. **The release points of commits still queued are dropped when the surface or client goes.**
//!    `a_surface_destroyed_mid_wait_signals_the_queued_release_points`,
//!    `a_client_that_disconnects_mid_wait_signals_the_queued_release_points`.
//! 10. **A surface that makes its object again is no longer checked.** `a_syncobj_surface_made_again_is_still_unique`,
//!     `a_syncobj_surface_made_again_checks_its_commits`.
//! 11. **A release point with no buffer is named `no_acquire_point`.** `a_release_point_with_no_buffer_is_no_buffer`;
//!     Smithay's own errors stay as they were, which the other `..._is_a_protocol_error` tests of the file check with a
//!     real client.
//! 12. **An idle session releases a buffer a loop timeout late.** `an_idle_session_releases_soon_after_the_gpu_retires`.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::renderer::sync::Fence;
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_manager_v1::WpLinuxDrmSyncobjManagerV1;
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_surface_v1::{
    Error as SurfaceError, WpLinuxDrmSyncobjSurfaceV1,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, GlobalDispatch, Resource, Weak};
use smithay::wayland::compositor::{
    BufferAssignment, CompositorHandler, SurfaceAttributes, TraversalAction, add_blocker, add_destruction_hook,
    add_pre_commit_hook, is_sync_subsurface, with_states, with_surface_tree_upward,
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

/// How many commits of one surface may be queued with points the compositor has not applied yet.
///
/// A commit queues when it waits on an acquire point that has not signalled, or sits behind one that waits. A client
/// that commits a swapchain's worth of frames ahead of the GPU has a handful; one past this is not drawing, it is filling
/// the compositor's tables.
pub const SURFACE_COMMIT_LIMIT: usize = 16;

/// How many acquire waits one client may have outstanding across all its surfaces.
///
/// Each wait holds an eventfd and a source of the loop for as long as the client's timeline point stays unsignalled,
/// which a client controls. The compositor's descriptors are shared with every other client, so one client may not hold
/// them all.
pub const CLIENT_WAIT_LIMIT: usize = 64;

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

    /// Ends `client` for asking the compositor to hold more than it will: the protocol has no error for a limit, so the
    /// session answers as it does a client past its memory limits (`wl_display.error.no_memory`).
    fn refuse_waits(&mut self, client: &Client, reason: &str);
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

/// A commit that names points and has not been applied: what the compositor owes its client if the commit never is.
#[derive(Debug)]
struct Queued {
    /// Tells the commit from the surface's others when the source of its wait fires.
    number: u64,
    /// The release point the commit named. A commit that is never applied never had its buffer read, so the point is
    /// signalled when the surface goes.
    release: Option<DrmSyncPoint>,
    /// The loop's token for the source that watches the commit's acquire point, until the source has fired.
    wait: Option<RegistrationToken>,
}

/// The commits of one surface that name points and have not been applied, oldest first, kept in the surface's data.
struct QueuedCommits {
    client: ClientId,
    commits: RefCell<VecDeque<Queued>>,
}

impl QueuedCommits {
    fn new(client: ClientId) -> Self {
        Self {
            client,
            commits: RefCell::new(VecDeque::new()),
        }
    }

    fn len(&self) -> usize {
        self.commits.borrow().len()
    }

    fn push(&self, queued: Queued) {
        self.commits.borrow_mut().push_back(queued);
    }

    /// Marks the wait of commit `number` as fired, and says whether it was one the client was still counted for.
    fn fired(&self, number: u64) -> bool {
        self.commits
            .borrow_mut()
            .iter_mut()
            .find(|queued| queued.number == number)
            .and_then(|queued| queued.wait.take())
            .is_some()
    }

    /// Drops the commit that named `release`, which was just applied, and every commit queued before it: commits apply in
    /// the order they were made, so an older one that is still here was replaced in a cache and Smithay released it. The
    /// waits among them that had not fired are returned, to be taken out of the loop.
    fn applied(&self, release: &DrmSyncPoint) -> Vec<Abandoned> {
        let mut commits = self.commits.borrow_mut();
        let Some(position) = commits.iter().position(|queued| {
            queued
                .release
                .as_ref()
                .is_some_and(|held| held.timeline() == release.timeline() && held.point() == release.point())
        }) else {
            return Vec::new();
        };
        commits
            .drain(..=position)
            .filter_map(|queued| queued.wait)
            .map(|token| Abandoned {
                client: self.client.clone(),
                token,
            })
            .collect()
    }

    /// Takes every commit still queued.
    fn drain(&self) -> Vec<Queued> {
        self.commits.borrow_mut().drain(..).collect()
    }
}

/// A wait that was queued and is not any more, though its source has not fired: the table takes it out of the loop and
/// stops counting it against its client.
#[derive(Debug)]
pub struct Abandoned {
    client: ClientId,
    token: RegistrationToken,
}

/// What a commit that was applied took out of its surface tree.
#[derive(Debug, Default)]
pub struct TakenPoints {
    released: Vec<ReleaseTaken>,
    abandoned: Vec<Abandoned>,
}

/// The explicit sync global, the release points waiting for the GPU and the acquire waits each client has outstanding.
#[derive(Default)]
pub struct SyncobjServing {
    state: Option<DrmSyncobjState>,
    holds: Vec<ReleaseHold>,
    /// The last submission count read, to see a device made anew.
    seen: u64,
    /// How many acquire waits each client has outstanding: each holds an eventfd and a source of the loop.
    waiting: HashMap<ClientId, usize>,
    /// Numbers the commits that are queued.
    commits: u64,
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

    /// How long the loop waits for a wake while a release point waits for the GPU: a frame retiring wakes nothing, so a
    /// loop with nothing else to do would notice it a whole idle timeout late.
    pub const POLL: Duration = Duration::from_millis(4);

    /// Whether a release point is waiting for the GPU to finish frames, so the loop must look again soon rather than
    /// sleep through its idle timeout.
    #[must_use]
    pub fn awaiting_gpu(&self) -> bool {
        self.holds.iter().any(|hold| hold.after.is_some())
    }

    /// How many acquire waits `client` has outstanding.
    fn waits(&self, client: &ClientId) -> usize {
        self.waiting.get(client).copied().unwrap_or(0)
    }

    fn wait_begun(&mut self, client: &ClientId) {
        *self.waiting.entry(client.clone()).or_default() += 1;
    }

    fn wait_ended(&mut self, client: &ClientId) {
        if let Some(count) = self.waiting.get_mut(client) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.waiting.remove(client);
            }
        }
    }

    fn next_commit(&mut self) -> u64 {
        self.commits += 1;
        self.commits
    }

    /// Creates the global, importing timelines on `device`. The backend vouched for the device
    /// (`gnoblin_backend::ExplicitSync`), so nothing is asked of the kernel here.
    ///
    /// Called once, before any client connects.
    pub fn start<D: Host>(&mut self, display: &DisplayHandle, device: DrmDeviceFd) {
        self.state = Some(DrmSyncobjState::new::<D>(display, device));
        log::info!(area = "syncobj", "wp_linux_drm_syncobj_manager_v1 is served");
    }

    /// Keeps the release points a commit took until the GPU is done with what they release, and takes out of `handle`'s
    /// loop the waits of commits that went before it without their sources firing.
    pub fn hold<D: 'static>(&mut self, applied: TakenPoints, handle: Option<&LoopHandle<'static, D>>) {
        for gone in applied.abandoned {
            if let Some(handle) = handle {
                handle.remove(gone.token);
            }
            self.wait_ended(&gone.client);
        }
        self.holds.extend(applied.released.into_iter().map(|taken| ReleaseHold {
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
///
/// The commit is also taken off its surface's queue of commits that have not been applied, with any that went before it.
#[must_use]
pub fn take_points(surface: &WlSurface) -> TakenPoints {
    let mut applied = TakenPoints::default();
    if is_sync_subsurface(surface) {
        return applied;
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
            if let Some(point) = &release
                && let Some(queue) = states.data_map.get::<QueuedCommits>()
            {
                applied.abandoned.extend(queue.applied(point));
            }
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
                applied.released.push(ReleaseTaken {
                    surface: tree_surface.downgrade(),
                    generation: number,
                    point,
                });
            }
        },
        |_, _, _| true,
    );
    applied
}

/// Checks a commit against what the protocol requires and Smithay does not say, or says under another name: a surface
/// that has the explicit sync object and attaches a buffer must name both points, and a commit that names a point must
/// attach a buffer (`no_buffer`, also for a release point alone, which Smithay answers with `no_acquire_point`).
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
        let named = pending.acquire_point.is_some() || pending.release_point.is_some();
        if named && !attaches {
            object.post_error(
                SurfaceError::NoBuffer,
                "a point was named and no buffer was attached".to_owned(),
            );
            return false;
        }
        if attaches && !named {
            object.post_error(
                SurfaceError::NoAcquirePoint,
                "a buffer was attached with no acquire point".to_owned(),
            );
            return false;
        }
        true
    })
}

/// Puts `object` in the cell Smithay keeps a surface's explicit sync object in, when `get_surface` made the object and
/// could not store it.
///
/// Smithay makes the cell once and writes it with `insert_if_missing`, which leaves a cell that a destroyed object emptied
/// as it was. The surface then has an object Smithay's checks cannot see: no `surface_exists` for a further `get_surface`,
/// and none of the commit errors. Called after Smithay handled the request, with the object it created.
pub fn adopt_surface_object(surface: &WlSurface, object: &WpLinuxDrmSyncobjSurfaceV1) {
    with_states(surface, |states| {
        if let Some(held) = states.data_map.get::<RefCell<Option<WpLinuxDrmSyncobjSurfaceV1>>>() {
            let mut held = held.borrow_mut();
            if held.is_none() {
                *held = Some(object.clone());
            }
        }
    });
}

/// Holds a surface's commits until the acquire points they name have signalled, refuses a commit that breaks the protocol,
/// and keeps what the compositor owes for each commit that has not been applied.
///
/// The kernel signals an eventfd when the point does, and the loop clears the blocker. A point that has already signalled
/// adds no wait, and so does a surface whose client is gone or a session with no loop to wait on. A surface past
/// [`SURFACE_COMMIT_LIMIT`] queued commits, or a client past [`CLIENT_WAIT_LIMIT`] waits, is ended instead. When the
/// surface is destroyed (a client that disconnects destroys all of its surfaces) the sources of its waits come out of the
/// loop and the release points of its queued commits are signalled.
pub fn hold_commit_until_acquired<D: Host>(surface: &WlSurface) {
    add_pre_commit_hook::<D, _>(surface, |state, _display, surface| {
        if !check_commit(surface) {
            return;
        }
        let (acquire, release) = with_states(surface, |states| {
            let mut cached = states.cached_state.get::<DrmSyncobjCachedState>();
            let pending = cached.pending();
            (pending.acquire_point.clone(), pending.release_point.clone())
        });
        if acquire.is_none() && release.is_none() {
            return;
        }
        let Some(client) = surface.client() else {
            return;
        };
        queue_commit(state, surface, &client, acquire.as_ref(), release);
    });
    add_destruction_hook::<D, _>(surface, forget_commits::<D>);
}

/// Admits a commit that names points, and watches its acquire point when it has not signalled.
fn queue_commit<D: Host>(
    state: &mut D,
    surface: &WlSurface,
    client: &Client,
    acquire: Option<&DrmSyncPoint>,
    release: Option<DrmSyncPoint>,
) {
    let owner = client.id();
    let queued = with_states(surface, |states| {
        states.data_map.insert_if_missing(|| QueuedCommits::new(owner.clone()));
        states.data_map.get::<QueuedCommits>().map_or(0, QueuedCommits::len)
    });
    if queued >= SURFACE_COMMIT_LIMIT {
        state.refuse_waits(
            client,
            &format!("a surface has {queued} commits waiting on explicit sync points, which is its limit"),
        );
        return;
    }
    let unsignalled = acquire.filter(|point| !point.is_signaled());
    if unsignalled.is_some() {
        let held = state.syncobj().waits(&owner);
        if held >= CLIENT_WAIT_LIMIT {
            state.refuse_waits(
                client,
                &format!("a client has {held} acquire waits outstanding, which is its limit"),
            );
            return;
        }
    }
    let number = state.syncobj().next_commit();
    let wait = unsignalled.and_then(|point| watch(state, surface, client, point, number));
    if wait.is_none() && release.is_none() {
        return;
    }
    with_states(surface, |states| {
        if let Some(queue) = states.data_map.get::<QueuedCommits>() {
            queue.push(Queued { number, release, wait });
        }
    });
}

/// Watches `point` on the loop and blocks the commit until it signals. `None` when it could not be watched, in which case
/// the commit is not held.
fn watch<D: Host>(
    state: &mut D,
    surface: &WlSurface,
    client: &Client,
    point: &DrmSyncPoint,
    number: u64,
) -> Option<RegistrationToken> {
    let (blocker, source) = match point.generate_blocker() {
        Ok(pair) => pair,
        Err(error) => {
            log::warn!(
                area = "syncobj",
                %error,
                "an acquire point could not be watched; the commit is not held"
            );
            return None;
        }
    };
    let handle = state.loop_handle()?;
    let weak = surface.downgrade();
    let waiter = client.clone();
    let inserted = handle.insert_source(source, move |_, _, state: &mut D| {
        wait_fired(state, &weak, &waiter, number);
        let display = state.display_handle().clone();
        state.client_compositor_state(&waiter).blocker_cleared(state, &display);
        Ok(())
    });
    match inserted {
        Ok(token) => {
            add_blocker(surface, blocker);
            state.syncobj().wait_begun(&client.id());
            Some(token)
        }
        Err(error) => {
            log::warn!(
                area = "syncobj",
                ?error,
                "an acquire point could not be watched; the commit is not held"
            );
            None
        }
    }
}

/// The source of a wait fired: its eventfd is closed as the loop drops the source, so the client is no longer counted for it.
fn wait_fired<D: Host>(state: &mut D, surface: &Weak<WlSurface>, client: &Client, number: u64) {
    let Ok(surface) = surface.upgrade() else {
        return;
    };
    let counted = with_states(&surface, |states| {
        states
            .data_map
            .get::<QueuedCommits>()
            .is_some_and(|queue| queue.fired(number))
    });
    if counted {
        state.syncobj().wait_ended(&client.id());
    }
}

/// A surface is destroyed, by its client or because the client went: its waits come out of the loop, closing their
/// eventfds, and the release points of the commits that never applied are signalled, because nothing read their buffers.
/// Smithay's destruction hook reaches only the points it still holds as pending or current.
fn forget_commits<D: Host>(state: &mut D, surface: &WlSurface) {
    let Some((client, commits)) = with_states(surface, |states| {
        states
            .data_map
            .get::<QueuedCommits>()
            .map(|queue| (queue.client.clone(), queue.drain()))
    }) else {
        return;
    };
    let handle = state.loop_handle();
    for queued in commits {
        if let Some(token) = queued.wait {
            if let Some(handle) = &handle {
                handle.remove(token);
            }
            state.syncobj().wait_ended(&client);
        }
        if let Some(point) = queued.release
            && let Err(error) = point.signal()
        {
            // The client's timeline or the device is gone: nobody waits on the point.
            log::debug!(area = "syncobj", %error, "a release point of a commit that never applied could not be signalled");
        }
    }
}
