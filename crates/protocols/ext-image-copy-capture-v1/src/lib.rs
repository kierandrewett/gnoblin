//! XML protocol `ext_image_copy_capture_v1` (`<protocol name="ext_image_copy_capture_v1">`); interfaces served: `ext_image_copy_capture_manager_v1`.
//!
//! `ext_image_copy_capture_v1`: sessions that copy a source's pixels into client buffers.
//!
//! This crate is the protocol's own objects: the manager, the sessions it makes, the frames a session hands out,
//! the constraints a session announces and the check that a client's buffer meets them. What a session captures,
//! when a frame is answered and with which pixels are the compositor's capture rules, which the crate asks for
//! through [`Host`]. A refusal maps onto the protocol's failure reasons through [`ImageCopyFrame::fail`]. It exists because
//! Smithay's module of the same name cannot say what this compositor has to say.
//!
//! # Shared memory and GPU buffers
//!
//! A session always offers `wl_shm` (`argb8888`, `xrgb8888`). It also offers `dma_buf`, with the render node and the
//! format and modifier pairs, when the compositor can write the target into a client's GPU buffer: a software
//! session, a Vulkan session and a session with no GPU offer shared memory only, and a client that needs a GPU
//! buffer finds none offered. A frame into shared memory is copied when the request arrives. A frame into a GPU
//! buffer is queued, and the loop makes the copy where the renderer is.
//!
//! # Why this is not Smithay's `image_copy_capture`
//!
//! A session that offers `dma_buf` names the render node the buffers must be allocated on. Smithay's
//! `DmabufConstraints` holds that as a `DrmNode`, and a `DrmNode` can only be made for a device that has a
//! `/sys/dev/char/<major>:<minor>/device/drm` entry. That is right for a real GPU and makes the one thing the
//! constraint exists to say untestable on a machine with none: the Agent37 instances the compositor's tests run on
//! have no DRM device at all, so a stand-in importer's device number cannot be turned into a node and the whole
//! `dma_buf` half of the protocol could never run there. These objects state the device as the plain `dev_t` the
//! wire carries, which is all a `dmabuf_device` event is, and are otherwise the protocol: the same objects, events,
//! errors and lifetimes, so a client cannot tell them from Smithay's.
//!
//! # Lifetimes
//!
//! - A [`ImageCopySession`] is owned by the compositor. Dropping it sends `stopped` and fails the frame that is waiting.
//! - A [`ImageCopySessionRef`] is a cloneable handle for a frame handler.
//! - A [`ImageCopyFrame`] is a request for one copy. The compositor completes it with [`ImageCopyFrame::success`] or
//!   [`ImageCopyFrame::fail`]; a frame that is dropped without either is failed `unknown`, so a client never waits for ever.
//!   A frame may be held across loop turns: a copy into a GPU buffer is done where the renderer is, which is not
//!   in a protocol handler.
//!
//! # Failure modes
//!
//! 1. **A buffer that does not meet the constraints is written to.** [`validate_buffer`] runs before the compositor
//!    sees the frame: a shared-memory buffer must be at least the announced size in an offered format, a GPU buffer
//!    the same in an offered format *and modifier* with a plane count its modifier allows, and anything else is
//!    refused `buffer_constraints`: `crates/gnoblin-wayland/tests/dmabuf_capture.rs` (`a_gpu_buffer_in_an_unoffered_format_is_refused`,
//!    `a_gpu_buffer_that_is_too_small_is_refused`, `a_gpu_buffer_is_refused_when_none_was_offered`).
//! 2. **A second frame is created while one is waiting.** That is the protocol's `duplicate_frame` error.
//! 3. **A destroyed frame is answered.** Completing a frame whose object is gone sends nothing and cannot fail.
//! 4. **A stopped session is revived.** A session that stopped has no constraints, so any frame of it is refused.

use std::ops;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Fourcc, Modifier};
use smithay::backend::renderer::{BufferType, buffer_type};
use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::{
    ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use smithay::reexports::wayland_server::backend::{ClientId as WaylandClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::utils::user_data::UserDataMap;
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::image_capture_source::ImageCaptureSource;
use smithay::wayland::shm::with_buffer_contents;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_types::{Rect, Size};
use gnoblin_wayland_api::access::Global;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// Why a frame failed, as the protocol names it.
pub use ext_image_copy_capture_frame_v1::FailureReason;

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "ext-image-copy-capture-v1",
    interfaces: &[ServedInterface::new("ext_image_copy_capture_manager_v1", 1)],
    privilege: Privilege::Roster(Global::ImageCopyCapture),
};

/// What the protocol needs of the compositor: the capture rules behind each object.
pub trait Host: Sized + 'static {
    /// The constraints a new session of `source` must announce, or `None` when it may not capture at all. A session
    /// that may not capture learns nothing, not even how large its target is.
    fn image_copy_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints>;

    /// Takes ownership of the session a client made, and opens it in the compositor's capture table.
    fn image_copy_new_session(&mut self, session: ImageCopySession);

    /// Answers a client's request for one frame: a frame into shared memory now, a frame into a GPU buffer when the
    /// loop next draws.
    fn image_copy_frame(&mut self, session: &ImageCopySessionRef, frame: ImageCopyFrame);

    /// A client destroyed a frame object: a copy still queued for it is dropped.
    fn image_copy_frame_aborted(&mut self, frame: &ImageCopyFrameRef);

    /// A client destroyed a session object.
    fn image_copy_session_destroyed(&mut self, session: ImageCopySessionRef);

    /// Whether a buffer's plane count is one its modifier allows. Only the renderer's rules can say which counts a
    /// vendor modifier has, so the compositor answers.
    fn dmabuf_planes_fit(dmabuf: &Dmabuf) -> bool;
}

/// Creates the manager global, visible to the clients the roster calls privileged.
///
/// The identity it returns is what lets a configuration that turns the protocol off remove the global.
pub fn register<D>(registrar: &Registrar<'_>) -> GlobalId
where
    D: GlobalDispatch<ExtImageCopyCaptureManagerV1, ExtImageCopyCaptureV1ManagerGlobalData> + 'static,
{
    registrar.display().create_global::<D, ExtImageCopyCaptureManagerV1, _>(
        1,
        ExtImageCopyCaptureV1ManagerGlobalData {
            filter: Box::new(registrar.filter(Global::ImageCopyCapture)),
        },
    )
}

/// What a client's buffers must be like for a session.
#[derive(Clone, Debug)]
pub struct BufferConstraints {
    /// The size the buffer must be, or larger.
    pub size: Size,
    /// The shared-memory formats it may have.
    pub shm: Vec<wl_shm::Format>,
    /// The GPU buffers it may be, when the session offers any.
    pub dma: Option<DmabufConstraints>,
}

/// The GPU half of a session's constraints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmabufConstraints {
    /// The `dev_t` of the render node the buffers must be allocated on.
    pub device: u64,
    /// The formats, each with the modifiers the renderer can render into.
    pub formats: Vec<(Fourcc, Vec<Modifier>)>,
}

impl DmabufConstraints {
    /// The constraints the pairs a renderer can write into make, or `None` when no device is named.
    ///
    /// A client has to allocate on a particular device, so support that has none (a surfaceless context) is not a
    /// `dma_buf` constraint at all: a session that offers formats and no device is one no client can use.
    pub fn from_pairs(device: Option<u64>, pairs: impl IntoIterator<Item = (Fourcc, Modifier)>) -> Option<Self> {
        let device = device?;
        let mut formats: Vec<(Fourcc, Vec<Modifier>)> = Vec::new();
        for (code, modifier) in pairs {
            match formats.iter_mut().find(|(known, _)| *known == code) {
                Some((_, modifiers)) => modifiers.push(modifier),
                None => formats.push((code, vec![modifier])),
            }
        }
        (!formats.is_empty()).then_some(Self { device, formats })
    }

    /// Whether `dmabuf`'s format and modifier are among the offered ones.
    fn offers(&self, dmabuf: &Dmabuf) -> bool {
        let format = dmabuf.format();
        self.formats
            .iter()
            .any(|(code, modifiers)| *code == format.code && modifiers.contains(&format.modifier))
    }
}

/// Whether `buffer` meets `constraints`, or why it does not.
///
/// Both kinds are at least the announced size. A shared-memory buffer must be in an offered format. A GPU buffer
/// must be in an offered format and modifier, and its plane count must be one its modifier allows. A buffer of any
/// other kind, such as a single colour, is not a place a capture can be written.
pub fn validate_buffer<H: Host>(buffer: &WlBuffer, constraints: &BufferConstraints) -> Result<(), FailureReason> {
    let wanted = constraints.size;
    match buffer_type(buffer) {
        Some(BufferType::Shm) => with_buffer_contents(buffer, |_, _, data| {
            let big_enough = u32::try_from(data.width).is_ok_and(|width| width >= wanted.width)
                && u32::try_from(data.height).is_ok_and(|height| height >= wanted.height);
            if big_enough && constraints.shm.contains(&data.format) {
                Ok(())
            } else {
                Err(FailureReason::BufferConstraints)
            }
        })
        .map_err(|_| FailureReason::BufferConstraints)?,
        Some(BufferType::Dma) => {
            let dmabuf = get_dmabuf(buffer).map_err(|_| FailureReason::BufferConstraints)?;
            let offered = constraints.dma.as_ref().ok_or(FailureReason::BufferConstraints)?;
            let size = dmabuf.size();
            let big_enough = u32::try_from(size.w).is_ok_and(|width| width >= wanted.width)
                && u32::try_from(size.h).is_ok_and(|height| height >= wanted.height);
            if big_enough && offered.offers(dmabuf) && H::dmabuf_planes_fit(dmabuf) {
                Ok(())
            } else {
                Err(FailureReason::BufferConstraints)
            }
        }
        _ => Err(FailureReason::BufferConstraints),
    }
}

/// The state behind one session object.
#[derive(Debug)]
struct SessionInner {
    stopped: bool,
    constraints: Option<BufferConstraints>,
    source: ImageCaptureSource,
    /// The frame created and not yet destroyed, of which a session has at most one.
    active_frame: Option<ImageCopyFrameRef>,
}

/// A cloneable handle to a session.
#[derive(Clone, Debug)]
pub struct ImageCopySessionRef {
    obj: ExtImageCopyCaptureSessionV1,
    inner: Arc<Mutex<SessionInner>>,
    user_data: Arc<UserDataMap>,
}

impl PartialEq for ImageCopySessionRef {
    fn eq(&self, other: &Self) -> bool {
        self.obj == other.obj
    }
}

fn locked<T>(shared: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ImageCopySessionRef {
    /// Announces `constraints` to the client and keeps them to check frames against.
    ///
    /// The `dmabuf_device` and `dmabuf_format` events are sent only when the session offers GPU buffers, which is
    /// what lets a client that finds none fall back to shared memory.
    pub fn update_constraints(&self, constraints: BufferConstraints) {
        let mut inner = locked(&self.inner);
        if !self.obj.is_alive() || inner.stopped {
            return;
        }
        self.obj.buffer_size(constraints.size.width, constraints.size.height);
        for format in &constraints.shm {
            self.obj.shm_format(*format);
        }
        if let Some(dma) = &constraints.dma {
            self.obj.dmabuf_device(dma.device.to_ne_bytes().to_vec());
            for (code, modifiers) in &dma.formats {
                let modifiers = modifiers
                    .iter()
                    .flat_map(|modifier| u64::from(*modifier).to_ne_bytes())
                    .collect();
                self.obj.dmabuf_format(*code as u32, modifiers);
            }
        }
        self.obj.done();
        inner.constraints = Some(constraints);
    }

    /// What the session captures.
    pub fn source(&self) -> ImageCaptureSource {
        locked(&self.inner).source.clone()
    }

    /// The compositor's own data for the session.
    pub fn user_data(&self) -> &UserDataMap {
        &self.user_data
    }

    /// The client that made the session, until it disconnects.
    pub fn client(&self) -> Option<Client> {
        self.obj.client()
    }
}

/// A session the compositor owns: dropping it stops it.
#[derive(Debug)]
pub struct ImageCopySession(ImageCopySessionRef);

impl ops::Deref for ImageCopySession {
    type Target = ImageCopySessionRef;
    fn deref(&self) -> &ImageCopySessionRef {
        &self.0
    }
}

impl PartialEq<ImageCopySessionRef> for ImageCopySession {
    fn eq(&self, other: &ImageCopySessionRef) -> bool {
        self.0 == *other
    }
}

impl Drop for ImageCopySession {
    fn drop(&mut self) {
        let mut inner = locked(&self.0.inner);
        if !self.0.obj.is_alive() || inner.stopped {
            return;
        }
        if let Some(frame) = inner.active_frame.take() {
            locked(&frame.inner).fail(&frame.obj, FailureReason::Stopped);
        }
        self.0.obj.stopped();
        inner.constraints.take();
        inner.stopped = true;
    }
}

impl ImageCopySession {
    /// Stops the session, telling its client.
    pub fn stop(self) {
        drop(self);
    }
}

/// The state behind one frame object.
#[derive(Debug)]
struct FrameInner {
    constraints: Option<BufferConstraints>,
    buffer: Option<WlBuffer>,
    capture_requested: bool,
    failed: Option<FailureReason>,
    ready: bool,
}

impl FrameInner {
    /// Fails the frame once, telling the client when it has already asked for the copy.
    fn fail(&mut self, frame: &ExtImageCopyCaptureFrameV1, reason: FailureReason) {
        if self.ready || self.failed.is_some() {
            return;
        }
        self.failed = Some(reason);
        if self.capture_requested {
            frame.failed(reason);
        }
    }
}

/// A cloneable handle to a frame.
#[derive(Clone, Debug)]
pub struct ImageCopyFrameRef {
    obj: ExtImageCopyCaptureFrameV1,
    inner: Arc<Mutex<FrameInner>>,
}

impl PartialEq for ImageCopyFrameRef {
    fn eq(&self, other: &Self) -> bool {
        self.obj == other.obj
    }
}

impl ImageCopyFrameRef {
    /// The buffer the client attached.
    pub fn buffer(&self) -> Option<WlBuffer> {
        locked(&self.inner).buffer.clone()
    }
}

/// One copy the client asked for, which the compositor has to answer.
#[derive(Debug)]
pub struct ImageCopyFrame {
    frame: ImageCopyFrameRef,
    completed: bool,
}

impl ops::Deref for ImageCopyFrame {
    type Target = ImageCopyFrameRef;
    fn deref(&self) -> &ImageCopyFrameRef {
        &self.frame
    }
}

impl PartialEq<ImageCopyFrameRef> for ImageCopyFrame {
    fn eq(&self, other: &ImageCopyFrameRef) -> bool {
        self.frame == *other
    }
}

impl ImageCopyFrame {
    /// Tells the client the buffer holds the frame: what changed since its last one, and when it was presented.
    pub fn success(mut self, damage: &[Rect], presented: Duration) {
        {
            let inner = locked(&self.frame.inner);
            if !inner.capture_requested || inner.failed.is_some() {
                return;
            }
        }
        let obj = &self.frame.obj;
        obj.transform(wl_output::Transform::Normal);
        for rect in damage {
            obj.damage(
                rect.x,
                rect.y,
                i32::try_from(rect.width).unwrap_or(i32::MAX),
                i32::try_from(rect.height).unwrap_or(i32::MAX),
            );
        }
        let seconds = presented.as_secs();
        obj.presentation_time(
            u32::try_from(seconds >> 32).unwrap_or(0),
            u32::try_from(seconds & 0xFFFF_FFFF).unwrap_or(0),
            presented.subsec_nanos(),
        );
        locked(&self.frame.inner).ready = true;
        obj.ready();
        self.completed = true;
    }

    /// Tells the client the copy was not made, and why.
    pub fn fail(mut self, reason: FailureReason) {
        locked(&self.frame.inner).fail(&self.frame.obj, reason);
        self.completed = true;
    }
}

impl Drop for ImageCopyFrame {
    fn drop(&mut self) {
        if !self.completed {
            locked(&self.frame.inner).fail(&self.frame.obj, FailureReason::Unknown);
        }
    }
}

/// The manager global's data: who may see it.
pub struct ExtImageCopyCaptureV1ManagerGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

impl std::fmt::Debug for ExtImageCopyCaptureV1ManagerGlobalData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExtImageCopyCaptureV1ManagerGlobalData")
            .finish_non_exhaustive()
    }
}

impl<D> GlobalDispatch2<ExtImageCopyCaptureManagerV1, D> for ExtImageCopyCaptureV1ManagerGlobalData
where
    D: Dispatch<ExtImageCopyCaptureManagerV1, ExtImageCopyCaptureV1ManagerUserData>,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ExtImageCopyCaptureManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ExtImageCopyCaptureV1ManagerUserData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

/// The manager object carries no state of its own.
#[derive(Debug)]
pub struct ExtImageCopyCaptureV1ManagerUserData;

/// The data of a session object.
#[derive(Debug)]
pub struct ExtImageCopyCaptureV1SessionUserData {
    inner: Arc<Mutex<SessionInner>>,
    user_data: Arc<UserDataMap>,
}

impl ExtImageCopyCaptureV1SessionUserData {
    /// The data of a session that is stopped from the start: one made for a source that is not one, or one a
    /// cursor session hands out, which this compositor does not capture.
    fn stopped(source: ImageCaptureSource) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionInner {
                stopped: true,
                constraints: None,
                source,
                active_frame: None,
            })),
            user_data: Arc::new(UserDataMap::new()),
        }
    }
}

/// The data of a pointer cursor session object. Cursor capture is not served, so it only ever hands out a
/// session that is stopped.
#[derive(Debug)]
pub struct ExtImageCopyCaptureV1CursorSessionUserData {
    handed_out: Mutex<bool>,
}

/// The data of a frame object.
#[derive(Debug)]
pub struct ExtImageCopyCaptureV1FrameUserData {
    inner: Arc<Mutex<FrameInner>>,
    session: ImageCopySessionRef,
}

impl<D> Dispatch2<ExtImageCopyCaptureManagerV1, D> for ExtImageCopyCaptureV1ManagerUserData
where
    D: Host
        + Dispatch<ExtImageCopyCaptureSessionV1, ExtImageCopyCaptureV1SessionUserData>
        + Dispatch<ExtImageCopyCaptureCursorSessionV1, ExtImageCopyCaptureV1CursorSessionUserData>,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _manager: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession { session, source, .. } => {
                let Some(source) = ImageCaptureSource::from_resource(&source) else {
                    let data = ExtImageCopyCaptureV1SessionUserData::stopped(ImageCaptureSource::new());
                    data_init.init(session, data).stopped();
                    return;
                };
                let inner = Arc::new(Mutex::new(SessionInner {
                    stopped: false,
                    constraints: None,
                    source: source.clone(),
                    active_frame: None,
                }));
                let user_data = Arc::new(UserDataMap::new());
                let obj = data_init.init(
                    session,
                    ExtImageCopyCaptureV1SessionUserData {
                        inner: Arc::clone(&inner),
                        user_data: Arc::clone(&user_data),
                    },
                );
                let session = ImageCopySessionRef { obj, inner, user_data };
                match state.image_copy_constraints(&source) {
                    Some(constraints) => {
                        session.update_constraints(constraints);
                        state.image_copy_new_session(ImageCopySession(session));
                    }
                    None => {
                        session.obj.stopped();
                        locked(&session.inner).stopped = true;
                    }
                }
            }
            ext_image_copy_capture_manager_v1::Request::CreatePointerCursorSession { session, .. } => {
                data_init.init(
                    session,
                    ExtImageCopyCaptureV1CursorSessionUserData {
                        handed_out: Mutex::new(false),
                    },
                );
            }
            _ => {}
        }
    }
}

impl<D> Dispatch2<ExtImageCopyCaptureSessionV1, D> for ExtImageCopyCaptureV1SessionUserData
where
    D: Host + Dispatch<ExtImageCopyCaptureFrameV1, ExtImageCopyCaptureV1FrameUserData>,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        resource: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            let mut session = locked(&self.inner);
            if session.active_frame.is_some() {
                resource.post_error(
                    ext_image_copy_capture_session_v1::Error::DuplicateFrame,
                    "create_frame sent before the previous frame was destroyed",
                );
                return;
            }
            let inner = Arc::new(Mutex::new(FrameInner {
                constraints: session.constraints.clone(),
                buffer: None,
                capture_requested: false,
                failed: None,
                ready: false,
            }));
            let obj = data_init.init(
                frame,
                ExtImageCopyCaptureV1FrameUserData {
                    inner: Arc::clone(&inner),
                    session: ImageCopySessionRef {
                        obj: resource.clone(),
                        inner: Arc::clone(&self.inner),
                        user_data: Arc::clone(&self.user_data),
                    },
                },
            );
            session.active_frame = Some(ImageCopyFrameRef { obj, inner });
        }
    }

    fn destroyed(&self, state: &mut D, _client: WaylandClientId, resource: &ExtImageCopyCaptureSessionV1) {
        state.image_copy_session_destroyed(ImageCopySessionRef {
            obj: resource.clone(),
            inner: Arc::clone(&self.inner),
            user_data: Arc::clone(&self.user_data),
        });
    }
}

impl<D> Dispatch2<ExtImageCopyCaptureCursorSessionV1, D> for ExtImageCopyCaptureV1CursorSessionUserData
where
    D: Dispatch<ExtImageCopyCaptureSessionV1, ExtImageCopyCaptureV1SessionUserData>,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } = request {
            let mut handed_out = self.handed_out.lock().unwrap_or_else(PoisonError::into_inner);
            if *handed_out {
                return;
            }
            *handed_out = true;
            data_init
                .init(
                    session,
                    ExtImageCopyCaptureV1SessionUserData::stopped(ImageCaptureSource::new()),
                )
                .stopped();
        }
    }
}

impl<D: Host> Dispatch2<ExtImageCopyCaptureFrameV1, D> for ExtImageCopyCaptureV1FrameUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        resource: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            ext_image_copy_capture_frame_v1::Request::AttachBuffer { buffer } => {
                let mut inner = locked(&self.inner);
                if !inner.capture_requested {
                    inner.buffer = Some(buffer);
                }
            }
            // The damage a client already knows of narrows what the compositor need report. It reports what changed
            // since the session's last frame either way, which is the whole of it or less, so it is not kept.
            ext_image_copy_capture_frame_v1::Request::DamageBuffer { .. } => {}
            ext_image_copy_capture_frame_v1::Request::Capture => self.capture(state, resource),
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut D, _client: WaylandClientId, resource: &ExtImageCopyCaptureFrameV1) {
        let frame = ImageCopyFrameRef {
            obj: resource.clone(),
            inner: Arc::clone(&self.inner),
        };
        {
            let mut session = locked(&self.session.inner);
            if session.active_frame.as_ref() == Some(&frame) {
                session.active_frame = None;
            }
        }
        state.image_copy_frame_aborted(&frame);
    }
}

impl ExtImageCopyCaptureV1FrameUserData {
    /// Answers `capture`: the buffer is checked against the session's constraints, and a buffer that meets them
    /// is handed to the compositor to be copied into.
    fn capture<D: Host>(&self, state: &mut D, resource: &ExtImageCopyCaptureFrameV1) {
        let frame = ImageCopyFrameRef {
            obj: resource.clone(),
            inner: Arc::clone(&self.inner),
        };
        {
            let mut inner = locked(&self.inner);
            if inner.capture_requested || inner.failed.is_some() {
                return;
            }
            inner.capture_requested = true;
            let refusal = match (&inner.buffer, &inner.constraints) {
                (Some(buffer), Some(constraints)) => validate_buffer::<D>(buffer, constraints).err(),
                _ => Some(FailureReason::BufferConstraints),
            };
            if let Some(reason) = refusal {
                inner.fail(resource, reason);
                return;
            }
        }
        if locked(&self.session.inner).active_frame.as_ref() != Some(&frame) {
            locked(&self.inner).fail(resource, FailureReason::Unknown);
            return;
        }
        state.image_copy_frame(
            &self.session,
            ImageCopyFrame {
                frame,
                completed: false,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::DmabufConstraints;
    use smithay::backend::allocator::{Fourcc, Modifier};

    #[test]
    fn the_modifiers_of_one_format_are_grouped_under_it() {
        let constraints = DmabufConstraints::from_pairs(
            Some(7),
            [
                (Fourcc::Argb8888, Modifier::Linear),
                (Fourcc::Xrgb8888, Modifier::Linear),
                (Fourcc::Argb8888, Modifier::Invalid),
            ],
        )
        .expect("a device and formats make constraints");
        assert_eq!(constraints.device, 7);
        assert_eq!(
            constraints.formats,
            vec![
                (Fourcc::Argb8888, vec![Modifier::Linear, Modifier::Invalid]),
                (Fourcc::Xrgb8888, vec![Modifier::Linear]),
            ]
        );
    }

    #[test]
    fn support_that_names_no_device_makes_no_constraints() {
        assert_eq!(
            DmabufConstraints::from_pairs(None, [(Fourcc::Argb8888, Modifier::Linear)]),
            None
        );
    }

    #[test]
    fn a_device_with_no_formats_makes_no_constraints() {
        assert_eq!(DmabufConstraints::from_pairs(Some(7), []), None);
    }
}
