//! The session and frame objects of `ext_image_copy_capture_v1`.
//!
//! The compositor's capture rules live in [`ext_image_copy_capture_v1`](super::ext_image_copy_capture_v1) and
//! [`capture_shared`](super::capture_shared); this module is only the protocol's own objects: the manager, the
//! sessions it makes, the frames a session hands out, the constraints a session announces and the check that a
//! client's buffer meets them. It exists because Smithay's module of the same name cannot say what this compositor
//! has to say.
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
//! - A [`Session`] is owned by the compositor. Dropping it sends `stopped` and fails the frame that is waiting.
//! - A [`SessionRef`] is a cloneable handle for a frame handler.
//! - A [`Frame`] is a request for one copy. The compositor completes it with [`Frame::success`] or
//!   [`Frame::fail`]; a frame that is dropped without either is failed `unknown`, so a client never waits for ever.
//!   A frame may be held across loop turns: a copy into a GPU buffer is done where the renderer is, which is not
//!   in a protocol handler (see [`capture_shared`](super::capture_shared)).
//!
//! # Failure modes
//!
//! 1. **A buffer that does not meet the constraints is written to.** [`validate_buffer`] runs before the compositor
//!    sees the frame: a shared-memory buffer must be at least the announced size in an offered format, a GPU buffer
//!    the same in an offered format *and modifier* with a plane count its modifier allows, and anything else is
//!    refused `buffer_constraints`: `tests/dmabuf_capture.rs` (`a_gpu_buffer_in_an_unoffered_format_is_refused`,
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
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::utils::user_data::UserDataMap;
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::image_capture_source::ImageCaptureSource;
use smithay::wayland::shm::with_buffer_contents;
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use gnoblin_backend::DmabufSupport;
use gnoblin_types::{Rect, Size};

use crate::State;

/// Why a frame failed, as the protocol names it.
pub(crate) use ext_image_copy_capture_frame_v1::FailureReason;

/// What a client's buffers must be like for a session.
#[derive(Clone, Debug)]
pub(crate) struct BufferConstraints {
    /// The size the buffer must be, or larger.
    pub(crate) size: Size,
    /// The shared-memory formats it may have.
    pub(crate) shm: Vec<wl_shm::Format>,
    /// The GPU buffers it may be, when the session offers any.
    pub(crate) dma: Option<DmabufConstraints>,
}

/// The GPU half of a session's constraints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DmabufConstraints {
    /// The `dev_t` of the render node the buffers must be allocated on.
    pub(crate) device: u64,
    /// The formats, each with the modifiers the renderer can render into.
    pub(crate) formats: Vec<(Fourcc, Vec<Modifier>)>,
}

impl DmabufConstraints {
    /// The constraints a backend's capture support makes, or `None` when it names no device.
    ///
    /// A client has to allocate on a particular device, so support that has none (a surfaceless context) is not a
    /// `dma_buf` constraint at all: a session that offers formats and no device is one no client can use.
    pub(crate) fn from_support(support: &DmabufSupport) -> Option<Self> {
        let device = support.main_device?;
        let mut formats: Vec<(Fourcc, Vec<Modifier>)> = Vec::new();
        for format in support.formats.iter() {
            match formats.iter_mut().find(|(code, _)| *code == format.code) {
                Some((_, modifiers)) => modifiers.push(format.modifier),
                None => formats.push((format.code, vec![format.modifier])),
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
pub(crate) fn validate_buffer(buffer: &WlBuffer, constraints: &BufferConstraints) -> Result<(), FailureReason> {
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
            if big_enough && offered.offers(dmabuf) && DmabufSupport::plane_count_fits(dmabuf) {
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
    active_frame: Option<FrameRef>,
}

/// A cloneable handle to a session.
#[derive(Clone, Debug)]
pub(crate) struct SessionRef {
    obj: ExtImageCopyCaptureSessionV1,
    inner: Arc<Mutex<SessionInner>>,
    user_data: Arc<UserDataMap>,
}

impl PartialEq for SessionRef {
    fn eq(&self, other: &Self) -> bool {
        self.obj == other.obj
    }
}

fn locked<T>(shared: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SessionRef {
    /// Announces `constraints` to the client and keeps them to check frames against.
    ///
    /// The `dmabuf_device` and `dmabuf_format` events are sent only when the session offers GPU buffers, which is
    /// what lets a client that finds none fall back to shared memory.
    pub(crate) fn update_constraints(&self, constraints: BufferConstraints) {
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
    pub(crate) fn source(&self) -> ImageCaptureSource {
        locked(&self.inner).source.clone()
    }

    /// The compositor's own data for the session.
    pub(crate) fn user_data(&self) -> &UserDataMap {
        &self.user_data
    }

    /// The client that made the session, until it disconnects.
    pub(crate) fn client(&self) -> Option<Client> {
        self.obj.client()
    }
}

/// A session the compositor owns: dropping it stops it.
#[derive(Debug)]
pub(crate) struct Session(SessionRef);

impl ops::Deref for Session {
    type Target = SessionRef;
    fn deref(&self) -> &SessionRef {
        &self.0
    }
}

impl PartialEq<SessionRef> for Session {
    fn eq(&self, other: &SessionRef) -> bool {
        self.0 == *other
    }
}

impl Drop for Session {
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

impl Session {
    /// Stops the session, telling its client.
    pub(crate) fn stop(self) {
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
pub(crate) struct FrameRef {
    obj: ExtImageCopyCaptureFrameV1,
    inner: Arc<Mutex<FrameInner>>,
}

impl PartialEq for FrameRef {
    fn eq(&self, other: &Self) -> bool {
        self.obj == other.obj
    }
}

impl FrameRef {
    /// The buffer the client attached.
    pub(crate) fn buffer(&self) -> Option<WlBuffer> {
        locked(&self.inner).buffer.clone()
    }
}

/// One copy the client asked for, which the compositor has to answer.
#[derive(Debug)]
pub(crate) struct Frame {
    frame: FrameRef,
    completed: bool,
}

impl ops::Deref for Frame {
    type Target = FrameRef;
    fn deref(&self) -> &FrameRef {
        &self.frame
    }
}

impl PartialEq<FrameRef> for Frame {
    fn eq(&self, other: &FrameRef) -> bool {
        self.frame == *other
    }
}

impl Frame {
    /// Tells the client the buffer holds the frame: what changed since its last one, and when it was presented.
    pub(crate) fn success(mut self, damage: &[Rect], presented: Duration) {
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
    pub(crate) fn fail(mut self, reason: FailureReason) {
        locked(&self.frame.inner).fail(&self.frame.obj, reason);
        self.completed = true;
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if !self.completed {
            locked(&self.frame.inner).fail(&self.frame.obj, FailureReason::Unknown);
        }
    }
}

/// Creates the manager global, visible to the clients `filter` allows.
///
/// The identity it returns is what lets a configuration that turns the protocol off remove the global.
pub(crate) fn create_image_copy_global<F>(display: &DisplayHandle, filter: F) -> GlobalId
where
    F: Fn(&Client) -> bool + Send + Sync + 'static,
{
    display.create_global::<State, ExtImageCopyCaptureManagerV1, _>(
        1,
        ManagerGlobal {
            filter: Box::new(filter),
        },
    )
}

/// The manager global's data: who may see it.
struct ManagerGlobal {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

impl std::fmt::Debug for ManagerGlobal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ManagerGlobal").finish_non_exhaustive()
    }
}

impl GlobalDispatch2<ExtImageCopyCaptureManagerV1, State> for ManagerGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ExtImageCopyCaptureManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

/// The manager object carries no state of its own.
#[derive(Debug)]
pub(crate) struct ManagerData;

/// The data of a session object.
#[derive(Debug)]
pub(crate) struct SessionData {
    inner: Arc<Mutex<SessionInner>>,
    user_data: Arc<UserDataMap>,
}

impl SessionData {
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
pub(crate) struct CursorSessionData {
    handed_out: Mutex<bool>,
}

/// The data of a frame object.
#[derive(Debug)]
pub(crate) struct FrameData {
    inner: Arc<Mutex<FrameInner>>,
    session: SessionRef,
}

impl Dispatch2<ExtImageCopyCaptureManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _manager: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession { session, source, .. } => {
                let Some(source) = ImageCaptureSource::from_resource(&source) else {
                    let data = SessionData::stopped(ImageCaptureSource::new());
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
                    SessionData {
                        inner: Arc::clone(&inner),
                        user_data: Arc::clone(&user_data),
                    },
                );
                let session = SessionRef { obj, inner, user_data };
                match state.image_copy_constraints(&source) {
                    Some(constraints) => {
                        session.update_constraints(constraints);
                        state.image_copy_new_session(Session(session));
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
                    CursorSessionData {
                        handed_out: Mutex::new(false),
                    },
                );
            }
            _ => {}
        }
    }
}

impl Dispatch2<ExtImageCopyCaptureSessionV1, State> for SessionData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        resource: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
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
                FrameData {
                    inner: Arc::clone(&inner),
                    session: SessionRef {
                        obj: resource.clone(),
                        inner: Arc::clone(&self.inner),
                        user_data: Arc::clone(&self.user_data),
                    },
                },
            );
            session.active_frame = Some(FrameRef { obj, inner });
        }
    }

    fn destroyed(&self, state: &mut State, _client: WaylandClientId, resource: &ExtImageCopyCaptureSessionV1) {
        state.image_copy_session_destroyed(SessionRef {
            obj: resource.clone(),
            inner: Arc::clone(&self.inner),
            user_data: Arc::clone(&self.user_data),
        });
    }
}

impl Dispatch2<ExtImageCopyCaptureCursorSessionV1, State> for CursorSessionData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } = request {
            let mut handed_out = self.handed_out.lock().unwrap_or_else(PoisonError::into_inner);
            if *handed_out {
                return;
            }
            *handed_out = true;
            data_init
                .init(session, SessionData::stopped(ImageCaptureSource::new()))
                .stopped();
        }
    }
}

impl Dispatch2<ExtImageCopyCaptureFrameV1, State> for FrameData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
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

    fn destroyed(&self, state: &mut State, _client: WaylandClientId, resource: &ExtImageCopyCaptureFrameV1) {
        let frame = FrameRef {
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

impl FrameData {
    /// Answers `capture`: the buffer is checked against the session's constraints, and a buffer that meets them
    /// is handed to the compositor to be copied into.
    fn capture(&self, state: &mut State, resource: &ExtImageCopyCaptureFrameV1) {
        let frame = FrameRef {
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
                (Some(buffer), Some(constraints)) => validate_buffer(buffer, constraints).err(),
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
            Frame {
                frame,
                completed: false,
            },
        );
    }
}
