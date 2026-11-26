//! Checks a client's request has to pass before Smithay handles it.
//!
//! [`crate::dispatch`] hands every request of every protocol object to Smithay's handler. Most of
//! what the protocols forbid is refused in there. A few rules are not: Smithay accepts the request
//! and raises the error later, or never. A rule that Smithay's handler cannot be given (it is a
//! method on a type this crate does not own) is written here and applied by the generic dispatch,
//! which asks [`screen`] first and drops a request it refuses. The request's protocol error is
//! posted here, so the client is disconnected with the error the protocol names. A request whose
//! whole meaning Smithay gets wrong (a subsurface's position) is answered here and dropped the same way.
//!
//! # Failure modes
//!
//! 1. **A surface that cannot take an `xdg_surface`.** `xdg_wm_base.get_xdg_surface` on a `wl_surface`
//!    with another role is `xdg_wm_base.role`, and on one with a buffer attached or committed is
//!    `xdg_wm_base.invalid_surface_state`. Smithay raises the first only at `get_toplevel` and the
//!    second at commit, so a client that creates the object and never goes on is never told.
//!    Tests: the wlcs `XdgSurfaceStableTest` cases `creating_xdg_surface_from_wl_surface_with_*`.
//! 2. **A buffer attached before the first configure is acknowledged.** The protocol calls it
//!    `xdg_surface.unconfigured_buffer` and the client is told when it attaches; Smithay raises it at commit,
//!    and a client that attaches and never commits is never told. The error is raised on the `xdg_surface`,
//!    which Smithay creates inside its own handler and keeps in private state, so the object is found from its
//!    protocol id once the handler has made it ([`NewXdgSurface`]).
//!    Tests: the wlcs `XdgSurfaceStableTest.attaching_buffer_to_unconfigured_xdg_surface_is_an_error` and
//!    `tests/window_requests.rs`.
//! 3. **A subsurface position that lands with the subsurface's commit.** `set_position` is answered by
//!    [`crate::subsurface_position`], which applies it with the parent's commit as the protocol says.
//! 4. **A rule that costs every request.** Which object and which request a check is for is decided
//!    from the types, so a request that has no check pays a few type comparisons and nothing else.
//! 5. **A buffer larger than the compositor holds.** `wl_shm_pool.create_buffer` and
//!    `zwp_linux_buffer_params_v1.create`/`create_immed` name a width and a height, and nothing in Smithay bounds them
//!    but the pool's own size (a sparse file of any length is a valid pool) and the driver at the first draw. A buffer of
//!    more than 16384 pixels on an edge or 2^27 pixels is refused at the request, before anything is mapped or imported:
//!    `wl_shm.invalid_stride` for a pool, `zwp_linux_buffer_params_v1.invalid_dimensions` for a dmabuf.
//!    Tests: `tests/gpu_memory.rs` and `tests/dmabuf.rs`.
//! 6. **A client that holds more than its share.** `wl_compositor.create_surface` and `wl_shm_pool.create_buffer` are
//!    asked of the memory ledger ([`crate::gpu_memory::Ledger`]) first: a client past its surface or buffer-byte
//!    limit, or a buffer the session's budget cannot hold even after its caches are given back, is ended with
//!    `wl_display.error.no_memory` and the request goes no further. What is admitted is recorded once Smithay has made
//!    the object ([`Charge`]), so a request Smithay rejects leaves no entry behind, and is given back when the object
//!    is destroyed (`crate::dispatch`). A GPU buffer is admitted where the loop imports it, which is where the backend is.
//!    Tests: `tests/gpu_memory_budget.rs`.
//! 7. **A surface that makes its explicit sync object again.** Smithay keeps the object in a cell it makes once, and a
//!    `get_surface` after the object was destroyed leaves the cell empty: no `surface_exists` for a further object and no
//!    commit errors. The object Smithay made is found from its protocol id once its handler has run and stored
//!    ([`NewSyncobjSurface`]).
//!    Tests: `tests/syncobj.rs`, `a_syncobj_surface_made_again_is_still_unique` and
//!    `a_syncobj_surface_made_again_checks_its_commits`.

use std::any::Any;
use std::sync::{Mutex, PoisonError};

use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_buffer_params_v1::{
    self, ZwpLinuxBufferParamsV1,
};
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_manager_v1::{
    self, WpLinuxDrmSyncobjManagerV1,
};
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_surface_v1::WpLinuxDrmSyncobjSurfaceV1;
use smithay::reexports::wayland_protocols::xdg::shell::server::{xdg_surface, xdg_wm_base};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_compositor::{self, WlCompositor};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_shm_pool::{self, WlShmPool};
use smithay::reexports::wayland_server::protocol::wl_subsurface::{self, WlSubsurface};
use smithay::reexports::wayland_server::protocol::wl_surface::{self, WlSurface};
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource, Weak, protocol::wl_buffer::WlBuffer};
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{
    BufferAssignment, CompositorHandler, SubsurfaceUserData, SurfaceAttributes, SurfaceData, get_role, with_states,
};
use smithay::wayland::shell::xdg::{XDG_POPUP_ROLE, XDG_TOPLEVEL_ROLE, XdgPopupSurfaceData, XdgToplevelSurfaceData};

use crate::State;
use crate::log;

/// What the generic dispatch does with a request.
pub(crate) enum Screened {
    /// Smithay's handler handles it.
    Forward,
    /// It was refused with its protocol error, or handled here; Smithay's handler never sees it.
    Consumed,
    /// Smithay's handler handles it, and the object it creates is recorded afterwards.
    ForwardThenTrack(NewXdgSurface),
    /// Smithay's handler handles it, and the memory the object it creates holds is recorded afterwards.
    ForwardThenCharge(Charge),
    /// Smithay's handler handles it, and the object it creates is put where Smithay's own checks find it afterwards.
    ForwardThenAdopt(NewSyncobjSurface),
}

/// What a request that was admitted to the memory ledger holds once Smithay has made its object.
pub(crate) enum Charge {
    /// A `wl_buffer` of `bytes`, by its protocol id.
    Buffer { client: ClientId, id: u32, bytes: u64 },
    /// A `wl_surface`, by its protocol id.
    Surface { client: ClientId, id: u32 },
}

impl Charge {
    /// Records the charge, if the object exists. A request Smithay rejected made no object and costs nothing.
    pub(crate) fn record(self, state: &State, client: &Client, handle: &DisplayHandle) {
        match self {
            Self::Buffer {
                client: id,
                id: object,
                bytes,
            } => {
                if client.object_from_protocol_id::<WlBuffer>(handle, object).is_ok() {
                    state.gpu_ledger.record_buffer(&id, object, bytes);
                }
            }
            Self::Surface { client: id, id: object } => {
                if client.object_from_protocol_id::<WlSurface>(handle, object).is_ok() {
                    state.gpu_ledger.record_surface(&id, object);
                }
            }
        }
    }
}

/// The protocol id of the object a `New<...>` request argument creates, read from its `Debug` form: the type hands
/// out nothing else. `None` when the form is not the one expected, in which case the caller does not record the
/// object and says so.
fn new_object_id(id: &dyn std::fmt::Debug, interface: &str) -> Option<u32> {
    let shown = format!("{id:?}");
    let digits = shown.split_once(&format!("{interface}@"))?.1;
    let end = digits
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse().ok()
}

/// A `get_xdg_surface` request that Smithay is about to handle.
pub(crate) struct NewXdgSurface {
    surface: WlSurface,
    protocol_id: u32,
}

/// The `xdg_surface` a `wl_surface` was given, kept on the surface for the checks that need the object to raise an
/// error on. A weak reference: the object's own data holds the surface, so a strong one would be a cycle.
#[derive(Default)]
struct XdgSurfaceOf(Mutex<Option<Weak<xdg_surface::XdgSurface>>>);

impl NewXdgSurface {
    /// Reads the id of the object a `get_xdg_surface` creates.
    ///
    /// The id is in the request as a `New<XdgSurface>`, which hands out nothing but its `Debug` form, and Smithay keeps
    /// the object it makes in private state, so the object is found from its protocol id once Smithay has made it.
    fn new(surface: &WlSurface, id: &dyn std::fmt::Debug) -> Option<Self> {
        let protocol_id = new_object_id(id, "xdg_surface")?;
        Some(Self {
            surface: surface.clone(),
            protocol_id,
        })
    }

    /// Records the `xdg_surface` on its `wl_surface`. Called after Smithay's handler made the object.
    pub(crate) fn track(self, client: &Client, handle: &DisplayHandle) {
        let Ok(object) = client.object_from_protocol_id::<xdg_surface::XdgSurface>(handle, self.protocol_id) else {
            log::warn!(
                area = "xdg",
                protocol_id = self.protocol_id,
                "the new xdg_surface was not found, so a buffer attached early is caught at commit only"
            );
            return;
        };
        with_states(&self.surface, |states| {
            states.data_map.insert_if_missing_threadsafe(XdgSurfaceOf::default);
            if let Some(held) = states.data_map.get::<XdgSurfaceOf>() {
                *held.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(object.downgrade());
            }
        });
    }
}

/// A `get_surface` request of `wp_linux_drm_syncobj_manager_v1` that Smithay is about to handle.
pub(crate) struct NewSyncobjSurface {
    surface: WlSurface,
    protocol_id: u32,
}

impl NewSyncobjSurface {
    /// Stores the `wp_linux_drm_syncobj_surface_v1` Smithay made as the surface's object, when it could not.
    ///
    /// Smithay writes the object into a cell it makes once, so a surface that destroyed its object and asked for another
    /// gets an object its commit hook and its `surface_exists` check cannot see. A request Smithay refused made no
    /// object, and there is nothing to store.
    pub(crate) fn adopt(self, client: &Client, handle: &DisplayHandle) {
        let Ok(object) = client.object_from_protocol_id::<WpLinuxDrmSyncobjSurfaceV1>(handle, self.protocol_id) else {
            return;
        };
        gnoblin_wayland_protocols::wp_linux_drm_syncobj_v1::adopt_surface_object(&self.surface, &object);
    }
}

/// Whether `request` on `resource` may go on to Smithay's handler.
///
/// A request that is refused has already had its protocol error posted.
pub(crate) fn screen<I>(state: &State, client: &Client, resource: &I, request: &I::Request) -> Screened
where
    I: Resource + 'static,
    I::Request: 'static,
{
    let request: &dyn Any = request;
    let resource: &dyn Any = resource;
    if let Some(request) = request.downcast_ref::<xdg_wm_base::Request>() {
        let (Some(wm_base), xdg_wm_base::Request::GetXdgSurface { id, surface }) =
            (resource.downcast_ref::<xdg_wm_base::XdgWmBase>(), request)
        else {
            return Screened::Forward;
        };
        if let Some((error, message)) = refuse_xdg_surface(surface) {
            wm_base.post_error(error, message);
            return Screened::Consumed;
        }
        return match NewXdgSurface::new(surface, id) {
            Some(created) => Screened::ForwardThenTrack(created),
            None => {
                log::warn!(
                    area = "xdg",
                    "the id of a new xdg_surface could not be read, so a buffer attached early is caught at commit only"
                );
                Screened::Forward
            }
        };
    }
    if let Some(request) = request.downcast_ref::<wl_compositor::Request>() {
        let (Some(_), wl_compositor::Request::CreateSurface { id }) =
            (resource.downcast_ref::<WlCompositor>(), request)
        else {
            return Screened::Forward;
        };
        if let Err(refusal) = state.gpu_ledger.admit_surface(&client.id()) {
            state.refuse_client(client, refusal);
            return Screened::Consumed;
        }
        return match new_object_id(id, "wl_surface") {
            Some(object) => Screened::ForwardThenCharge(Charge::Surface {
                client: client.id(),
                id: object,
            }),
            None => {
                log::warn!(
                    area = "memory",
                    "the id of a new wl_surface could not be read, so it is not counted"
                );
                Screened::Forward
            }
        };
    }
    if let Some(request) = request.downcast_ref::<wl_surface::Request>() {
        return match (resource.downcast_ref::<WlSurface>(), request) {
            (Some(surface), wl_surface::Request::Attach { buffer: Some(_), .. })
                if refuse_unconfigured_attach(surface) =>
            {
                Screened::Consumed
            }
            _ => Screened::Forward,
        };
    }
    if let Some(request) = request.downcast_ref::<wl_subsurface::Request>() {
        let Some(subsurface) = resource.downcast_ref::<WlSubsurface>().and_then(subsurface_surface) else {
            return Screened::Forward;
        };
        return match request {
            wl_subsurface::Request::SetPosition { x, y } => {
                let scale = state.client_compositor_state(client).client_scale();
                // Smithay's own conversion of a position from the client's scale: the same division and rounding.
                let at = Point::<i32, Logical>::from((from_client_scale(*x, scale), from_client_scale(*y, scale)));
                crate::subsurface_position::request(subsurface, at);
                Screened::Consumed
            }
            wl_subsurface::Request::Destroy => {
                crate::subsurface_position::forget(subsurface);
                Screened::Forward
            }
            _ => Screened::Forward,
        };
    }
    if let Some(request) = request.downcast_ref::<wl_shm_pool::Request>() {
        let (
            Some(pool),
            wl_shm_pool::Request::CreateBuffer {
                id,
                width,
                height,
                stride,
                ..
            },
        ) = (resource.downcast_ref::<WlShmPool>(), request)
        else {
            return Screened::Forward;
        };
        if let Some(message) = oversized(*width, *height) {
            // The protocol has no error for a buffer that is too large; `invalid_stride` is the one `wl_shm` names for
            // a width, height or stride the pool cannot describe, and the message says which limit was passed.
            pool.post_error(wl_shm::Error::InvalidStride, message);
            return Screened::Consumed;
        }
        // A size or stride the pool cannot describe is Smithay's to refuse with the error the protocol names; it
        // holds nothing, so it is not asked of the ledger.
        let (Ok(rows), Ok(stride)) = (u64::try_from(*height), u64::try_from(*stride)) else {
            return Screened::Forward;
        };
        let bytes = rows.saturating_mul(stride);
        if bytes == 0 {
            return Screened::Forward;
        }
        if let Err(refusal) = state.gpu_ledger.admit_buffer(&client.id(), bytes) {
            state.refuse_client(client, refusal);
            return Screened::Consumed;
        }
        return match new_object_id(id, "wl_buffer") {
            Some(object) => Screened::ForwardThenCharge(Charge::Buffer {
                client: client.id(),
                id: object,
                bytes,
            }),
            None => {
                log::warn!(
                    area = "memory",
                    "the id of a new wl_buffer could not be read, so it is not counted"
                );
                Screened::Forward
            }
        };
    }
    if let Some(request) = request.downcast_ref::<zwp_linux_buffer_params_v1::Request>() {
        let (width, height) = match request {
            zwp_linux_buffer_params_v1::Request::Create { width, height, .. }
            | zwp_linux_buffer_params_v1::Request::CreateImmed { width, height, .. } => (*width, *height),
            _ => return Screened::Forward,
        };
        if let (Some(params), Some(message)) = (
            resource.downcast_ref::<ZwpLinuxBufferParamsV1>(),
            oversized(width, height),
        ) {
            params.post_error(zwp_linux_buffer_params_v1::Error::InvalidDimensions, message);
            return Screened::Consumed;
        }
        return Screened::Forward;
    }
    if let Some(request) = request.downcast_ref::<wp_linux_drm_syncobj_manager_v1::Request>() {
        let (Some(_), wp_linux_drm_syncobj_manager_v1::Request::GetSurface { id, surface }) =
            (resource.downcast_ref::<WpLinuxDrmSyncobjManagerV1>(), request)
        else {
            return Screened::Forward;
        };
        return match new_object_id(id, "wp_linux_drm_syncobj_surface_v1") {
            Some(protocol_id) => Screened::ForwardThenAdopt(NewSyncobjSurface {
                surface: surface.clone(),
                protocol_id,
            }),
            None => {
                log::warn!(
                    area = "syncobj",
                    "the id of a new wp_linux_drm_syncobj_surface_v1 could not be read, so a surface that makes its object again is not checked"
                );
                Screened::Forward
            }
        };
    }
    Screened::Forward
}

/// The widest or tallest buffer a client may create, in pixels: the largest texture edge the drivers this compositor
/// draws with commonly report (`GL_MAX_TEXTURE_SIZE` is 16384 on Mesa's).
const MAX_BUFFER_EDGE: i32 = 16384;

/// The most pixels one buffer may hold: 2^27, half a GiB of RGBA. A window as large as the compositor's own largest
/// output surface (8192 by 8192, `gnoblin_output::supersample`) holds half of that.
const MAX_BUFFER_PIXELS: i64 = 1 << 27;

/// Why a buffer of `width` by `height` pixels is larger than the compositor holds, if it is.
///
/// A size with no pixels is Smithay's to refuse, with the error the protocol names for it.
fn oversized(width: i32, height: i32) -> Option<String> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let pixels = i64::from(width) * i64::from(height);
    (width > MAX_BUFFER_EDGE || height > MAX_BUFFER_EDGE || pixels > MAX_BUFFER_PIXELS).then(|| {
        format!(
            "a buffer of {width}x{height} pixels is larger than the compositor holds (at most {MAX_BUFFER_EDGE} on an \
             edge and {MAX_BUFFER_PIXELS} pixels)"
        )
    })
}

/// One coordinate of a client that draws at `scale` (an Xwayland client's) in logical pixels.
fn from_client_scale(coordinate: i32, scale: f64) -> i32 {
    (f64::from(coordinate) / scale).round() as i32
}

/// The surface a `wl_subsurface` is the role of.
fn subsurface_surface(subsurface: &WlSubsurface) -> Option<&WlSurface> {
    subsurface.data::<SubsurfaceUserData>().map(SubsurfaceUserData::surface)
}

/// Raises `xdg_surface.unconfigured_buffer` when a buffer is attached to a surface whose xdg_surface has not had its
/// first configure acknowledged. Returns whether it did.
///
/// A surface that is not an xdg surface is not asked, and neither is one whose `xdg_surface` is gone.
fn refuse_unconfigured_attach(surface: &WlSurface) -> bool {
    let object = with_states(surface, |states| {
        let held = states.data_map.get::<XdgSurfaceOf>()?;
        let object = held
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()?
            .upgrade()
            .ok()?;
        (!configured(states)).then_some(object)
    });
    let Some(object) = object else {
        return false;
    };
    object.post_error(
        xdg_surface::Error::UnconfiguredBuffer,
        format!(
            "{} had no configure acknowledged before a buffer was attached",
            surface.id()
        ),
    );
    true
}

/// Whether the role of the surface has had a configure acknowledged: what a buffer may be attached after.
fn configured(states: &SurfaceData) -> bool {
    let toplevel = states
        .data_map
        .get::<XdgToplevelSurfaceData>()
        .is_some_and(|role| role.lock().unwrap_or_else(PoisonError::into_inner).last_acked.is_some());
    toplevel
        || states
            .data_map
            .get::<XdgPopupSurfaceData>()
            .is_some_and(|role| role.lock().unwrap_or_else(PoisonError::into_inner).last_acked.is_some())
}

/// Why `surface` cannot have an `xdg_surface`, if it cannot.
///
/// A surface that already has an xdg role keeps it, so a new `xdg_surface` on it is not a different
/// role. A buffer counts whether it is only attached or already committed.
fn refuse_xdg_surface(surface: &WlSurface) -> Option<(xdg_wm_base::Error, String)> {
    let id = surface.id();
    if let Some(role) = get_role(surface)
        && role != XDG_TOPLEVEL_ROLE
        && role != XDG_POPUP_ROLE
    {
        return Some((
            xdg_wm_base::Error::Role,
            format!("{id} already has the role {role}, so it cannot have an xdg_surface"),
        ));
    }
    has_buffer(surface).then(|| {
        (
            xdg_wm_base::Error::InvalidSurfaceState,
            format!("{id} already has a buffer attached or committed, so it cannot have an xdg_surface"),
        )
    })
}

/// Whether a surface has a buffer attached and not yet committed, or one it shows.
fn has_buffer(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        let attached = states
            .cached_state
            .get::<SurfaceAttributes>()
            .pending()
            .buffer
            .as_ref()
            .is_some_and(|buffer| matches!(buffer, BufferAssignment::NewBuffer(_)));
        attached || shown_buffer(states).is_some()
    })
}

/// The buffer the surface's last commit left it showing.
fn shown_buffer(states: &SurfaceData) -> Option<WlBuffer> {
    let renderer = states.data_map.get::<RendererSurfaceStateUserData>()?;
    let renderer = renderer.lock().ok()?;
    renderer.buffer().map(|buffer| (**buffer).clone())
}
