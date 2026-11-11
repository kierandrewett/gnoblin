//! The Wayland side of view capture.
//!
//! [`Client`] connects to the owner's view socket and binds the capture globals. [`Capture`] is
//! one `ext_image_copy_capture` session on a View node: its frames are copied into a memfd-backed
//! `wl_shm` buffer, which [`Capture::pump`] reads into a [`Frame`]. One frame is in flight at a
//! time. Reads go through `pread`, so this module needs no `unsafe`.

use std::env;
use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rustix::event::{Nsecs, PollFd, PollFlags, Secs, Timespec, poll};
use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{
    ConnectError, Connection, Dispatch, DispatchError, EventQueue, QueueHandle, WEnum, delegate_noop,
};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1 as frame, ext_image_copy_capture_manager_v1 as manager,
    ext_image_copy_capture_session_v1 as session,
};

use crate::protocols::gnoblin_view_source_v1::gnoblin_view_source_manager_v1::GnoblinViewSourceManagerV1;
use crate::{LayoutError, ShmFormat, ShmLayout, shm_layout};

/// How long opening a capture waits for the session's first `done`.
const OPEN_WAIT: Duration = Duration::from_secs(2);

/// One captured picture of a View: `bytes` is `height` rows of `stride` bytes in `format`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: ShmFormat,
    pub bytes: Vec<u8>,
    /// Counts the frames of one capture from 1.
    pub serial: u64,
}

/// Why connecting, opening or capturing failed.
#[derive(Debug)]
pub enum CaptureError {
    /// `GNOBLIN_VIEW_DISPLAY` is set but `XDG_RUNTIME_DIR` is not.
    NoRuntimeDir,
    Connect(ConnectError),
    Wayland(WaylandError),
    Dispatch(DispatchError),
    Global(GlobalError),
    Bind(BindError),
    Io(io::Error),
    Layout(LayoutError),
    /// The node id does not fit the protocol's `uint`.
    NodeOutOfRange(u64),
    /// The session did not send `done` in time, or sent no buffer size or format.
    SessionNotReady,
    /// The session ended. The compositor stops a View's session when the View leaves the scene,
    /// and while the session is locked.
    Stopped,
    /// A frame failed. The reason is the protocol's `failure_reason`.
    FrameFailed(WEnum<frame::FailureReason>),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRuntimeDir => f.write_str("GNOBLIN_VIEW_DISPLAY is set but XDG_RUNTIME_DIR is not"),
            Self::Connect(error) => write!(f, "connecting to the view socket: {error}"),
            Self::Wayland(error) => write!(f, "view socket: {error}"),
            Self::Dispatch(error) => write!(f, "dispatching view socket events: {error}"),
            Self::Global(error) => write!(f, "view socket globals: {error}"),
            Self::Bind(error) => write!(f, "binding a view capture global: {error}"),
            Self::Io(error) => write!(f, "view capture I/O: {error}"),
            Self::Layout(error) => write!(f, "view capture buffer: {error}"),
            Self::NodeOutOfRange(node) => write!(f, "node {node} does not fit the protocol's uint"),
            Self::SessionNotReady => f.write_str("the capture session sent no usable buffer"),
            Self::Stopped => f.write_str("the capture session stopped"),
            Self::FrameFailed(reason) => write!(f, "a capture frame failed: {reason:?}"),
        }
    }
}

impl std::error::Error for CaptureError {}

macro_rules! from_error {
    ($($source:ty => $variant:ident),* $(,)?) => {
        $(impl From<$source> for CaptureError {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        })*
    };
}

from_error! {
    ConnectError => Connect,
    WaylandError => Wayland,
    DispatchError => Dispatch,
    GlobalError => Global,
    BindError => Bind,
    io::Error => Io,
    LayoutError => Layout,
}

/// The dispatch state of a [`Client`]. Every object that carries events has its own user data,
/// so the queue itself keeps no state.
struct State;

delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore manager::ExtImageCopyCaptureManagerV1);
delegate_noop!(State: ignore ExtImageCaptureSourceV1);
delegate_noop!(State: ignore GnoblinViewSourceManagerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Locks a mutex, recovering the data when a holder panicked: the events are plain values.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a session has announced so far: its buffer and whether it is ready or has stopped.
#[derive(Debug, Clone, Copy, Default)]
struct SessionEvents {
    size: Option<(u32, u32)>,
    format: Option<ShmFormat>,
    done: bool,
    stopped: bool,
}

#[derive(Default)]
struct SessionShared {
    events: Mutex<SessionEvents>,
}

impl SessionShared {
    fn snapshot(&self) -> SessionEvents {
        *lock(&self.events)
    }
}

/// How one frame ended.
#[derive(Debug, Clone)]
enum Outcome {
    Ready,
    Failed(WEnum<frame::FailureReason>),
}

#[derive(Default)]
struct FrameShared {
    outcome: Mutex<Option<Outcome>>,
}

impl FrameShared {
    fn outcome(&self) -> Option<Outcome> {
        lock(&self.outcome).clone()
    }
}

fn shm_format_of(format: wl_shm::Format) -> Option<ShmFormat> {
    match format {
        wl_shm::Format::Argb8888 => Some(ShmFormat::Argb8888),
        wl_shm::Format::Xrgb8888 => Some(ShmFormat::Xrgb8888),
        _ => None,
    }
}

fn wl_shm_format(format: ShmFormat) -> wl_shm::Format {
    match format {
        ShmFormat::Argb8888 => wl_shm::Format::Argb8888,
        ShmFormat::Xrgb8888 => wl_shm::Format::Xrgb8888,
    }
}

/// A layout value as the protocol's `int32`. `shm_layout` has already bounded every value it
/// returns to `i32::MAX`, so the fallback never applies.
fn int(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

impl Dispatch<session::ExtImageCopyCaptureSessionV1, Arc<SessionShared>> for State {
    fn event(
        _: &mut Self,
        _: &session::ExtImageCopyCaptureSessionV1,
        event: session::Event,
        data: &Arc<SessionShared>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut events = lock(&data.events);
        match event {
            session::Event::BufferSize { width, height } => events.size = Some((width, height)),
            session::Event::ShmFormat { format } => {
                if let WEnum::Value(format) = format
                    && events.format.is_none()
                {
                    events.format = shm_format_of(format);
                }
            }
            session::Event::Done => events.done = true,
            session::Event::Stopped => events.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<frame::ExtImageCopyCaptureFrameV1, Arc<FrameShared>> for State {
    fn event(
        _: &mut Self,
        _: &frame::ExtImageCopyCaptureFrameV1,
        event: frame::Event,
        data: &Arc<FrameShared>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let outcome = match event {
            frame::Event::Ready => Outcome::Ready,
            frame::Event::Failed { reason } => Outcome::Failed(reason),
            _ => return,
        };
        *lock(&data.outcome) = Some(outcome);
    }
}

/// A connection to the owner's view socket, with the capture globals bound.
pub struct Client {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    shm: wl_shm::WlShm,
    copy_manager: manager::ExtImageCopyCaptureManagerV1,
    view_manager: GnoblinViewSourceManagerV1,
}

impl Client {
    /// Connects from the environment, or returns `Ok(None)` when neither variable is set.
    ///
    /// `WAYLAND_SOCKET` (an inherited connection) wins. Otherwise `GNOBLIN_VIEW_DISPLAY` names a
    /// socket under `XDG_RUNTIME_DIR`.
    pub fn connect_from_env() -> Result<Option<Self>, CaptureError> {
        let connection = if env::var_os("WAYLAND_SOCKET").is_some() {
            Connection::connect_to_env()?
        } else if let Some(name) = env::var_os("GNOBLIN_VIEW_DISPLAY") {
            let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or(CaptureError::NoRuntimeDir)?;
            Connection::from_socket(UnixStream::connect(PathBuf::from(runtime).join(name))?)?
        } else {
            return Ok(None);
        };
        Self::from_connection(connection).map(Some)
    }

    /// Binds the capture globals on an established connection. A missing global is an error: the
    /// socket is not the view role's.
    pub fn from_connection(connection: Connection) -> Result<Self, CaptureError> {
        let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
        let handle = queue.handle();
        let mut state = State;
        let shm = globals.bind::<wl_shm::WlShm, _, _>(&handle, 1..=1, ())?;
        let copy_manager = globals.bind::<manager::ExtImageCopyCaptureManagerV1, _, _>(&handle, 1..=1, ())?;
        let view_manager = globals.bind::<GnoblinViewSourceManagerV1, _, _>(&handle, 1..=1, ())?;
        queue.roundtrip(&mut state)?;
        Ok(Self {
            connection,
            queue,
            state,
            shm,
            copy_manager,
            view_manager,
        })
    }

    /// Flushes requests, then waits up to `timeout` for the socket to become readable and
    /// dispatches what arrived. A zero timeout only dispatches what is already read.
    pub fn dispatch(&mut self, timeout: Duration) -> Result<(), CaptureError> {
        self.connection.flush()?;
        self.queue.dispatch_pending(&mut self.state)?;
        if let Some(guard) = self.connection.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let wait = Timespec {
                tv_sec: Secs::try_from(timeout.as_secs()).unwrap_or(Secs::MAX),
                tv_nsec: Nsecs::from(timeout.subsec_nanos()),
            };
            poll(&mut fds, Some(&wait)).map_err(io::Error::from)?;
            if !fds[0].revents().is_empty() {
                guard.read()?;
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }

    /// Dispatches until `done` holds or `timeout` passes.
    fn wait_until(&mut self, timeout: Duration, done: impl Fn() -> bool) -> Result<(), CaptureError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.dispatch(Duration::from_millis(10))?;
            if done() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(CaptureError::SessionNotReady);
            }
        }
    }

    /// Opens a capture of the View node `node`. Waits for the session's buffer constraints and
    /// allocates the buffer it will fill.
    pub fn open(&mut self, node: u64) -> Result<Capture, CaptureError> {
        let node_id = u32::try_from(node).map_err(|_| CaptureError::NodeOutOfRange(node))?;
        let handle = self.queue.handle();
        let source = self.view_manager.create_source(node_id, &handle, ());
        let shared = Arc::new(SessionShared::default());
        let session =
            self.copy_manager
                .create_session(&source, manager::Options::empty(), &handle, Arc::clone(&shared));
        match self.negotiate(&shared, &handle, node, &source, &session) {
            Ok(capture) => Ok(capture),
            Err(error) => {
                session.destroy();
                source.destroy();
                Err(error)
            }
        }
    }

    fn negotiate(
        &mut self,
        shared: &Arc<SessionShared>,
        handle: &QueueHandle<State>,
        node: u64,
        source: &ExtImageCaptureSourceV1,
        session: &session::ExtImageCopyCaptureSessionV1,
    ) -> Result<Capture, CaptureError> {
        self.wait_until(OPEN_WAIT, || {
            let events = shared.snapshot();
            events.done || events.stopped
        })?;
        let events = shared.snapshot();
        if events.stopped {
            return Err(CaptureError::Stopped);
        }
        let (width, height) = events.size.ok_or(CaptureError::SessionNotReady)?;
        let format = events.format.ok_or(CaptureError::SessionNotReady)?;
        let layout = shm_layout(width, height, format)?;

        let memfd = memfd_create("gnoblin-view-capture", MemfdFlags::CLOEXEC).map_err(io::Error::from)?;
        let file = File::from(memfd);
        file.set_len(u64::from(layout.size))?;
        let pool = self.shm.create_pool(file.as_fd(), int(layout.size), handle, ());
        let buffer = pool.create_buffer(
            0,
            int(layout.width),
            int(layout.height),
            int(layout.stride),
            wl_shm_format(format),
            handle,
            (),
        );
        pool.destroy();

        Ok(Capture {
            node,
            source: source.clone(),
            session: session.clone(),
            shared: Arc::clone(shared),
            file,
            buffer,
            layout,
            format,
            in_flight: None,
            latest: None,
            serial: 0,
        })
    }
}

/// One capture session on a View node.
pub struct Capture {
    node: u64,
    source: ExtImageCaptureSourceV1,
    session: session::ExtImageCopyCaptureSessionV1,
    shared: Arc<SessionShared>,
    file: File,
    buffer: wl_buffer::WlBuffer,
    layout: ShmLayout,
    format: ShmFormat,
    in_flight: Option<(frame::ExtImageCopyCaptureFrameV1, Arc<FrameShared>)>,
    latest: Option<Frame>,
    serial: u64,
}

impl Capture {
    /// The View node this capture is of.
    pub fn node(&self) -> u64 {
        self.node
    }

    /// The most recent frame read, if any.
    pub fn latest(&self) -> Option<&Frame> {
        self.latest.as_ref()
    }

    /// Advances the capture after the connection was dispatched. A ready frame is read into
    /// [`Capture::latest`] and the next frame is requested. An unfinished frame is left alone.
    ///
    /// # Errors
    ///
    /// The session stopped, a frame failed, or the buffer could not be read. The caller drops
    /// the capture.
    pub fn pump(&mut self, client: &Client) -> Result<(), CaptureError> {
        if self.shared.snapshot().stopped {
            return Err(CaptureError::Stopped);
        }
        let handle = client.queue.handle();
        let Some((frame, shared)) = self.in_flight.take() else {
            self.request(&handle);
            return Ok(());
        };
        match shared.outcome() {
            None => {
                self.in_flight = Some((frame, shared));
                Ok(())
            }
            Some(Outcome::Ready) => {
                frame.destroy();
                let read = self.read_frame();
                self.request(&handle);
                self.latest = Some(read?);
                Ok(())
            }
            Some(Outcome::Failed(reason)) => {
                frame.destroy();
                self.request(&handle);
                Err(CaptureError::FrameFailed(reason))
            }
        }
    }

    fn request(&mut self, handle: &QueueHandle<State>) {
        let shared = Arc::new(FrameShared::default());
        let frame = self.session.create_frame(handle, Arc::clone(&shared));
        frame.attach_buffer(&self.buffer);
        frame.damage_buffer(0, 0, int(self.layout.width), int(self.layout.height));
        frame.capture();
        self.in_flight = Some((frame, shared));
    }

    fn read_frame(&mut self) -> Result<Frame, CaptureError> {
        let mut bytes = vec![0u8; self.layout.size as usize];
        self.file.read_exact_at(&mut bytes, 0)?;
        self.serial += 1;
        Ok(Frame {
            width: self.layout.width,
            height: self.layout.height,
            stride: self.layout.stride,
            format: self.format,
            bytes,
            serial: self.serial,
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some((frame, _)) = self.in_flight.take() {
            frame.destroy();
        }
        self.buffer.destroy();
        self.session.destroy();
        self.source.destroy();
    }
}
