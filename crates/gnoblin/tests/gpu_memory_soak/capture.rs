//! The capture client the soak leaves open: one `ext_image_copy_capture` session on the first output that asks for a
//! frame whenever the previous one is done, for the whole run, through window churn, supersample changes, output
//! hotplug and session locks.
//!
//! A screenshot over the control socket is one request that ends; a screen recorder is a session that stays and is
//! fed a frame for every frame the compositor presents. The second is the one that keeps a compositor's capture
//! cache, readback and per-session state alive, so it is the one a leak would show in (design/gpu-memory.md).

use std::fs::File;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::path::Path;

use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};

/// What the server told this client, which the step reads.
#[derive(Default)]
struct State {
    /// The buffer size the session asked for, and how many constraint rounds ended (`done`).
    size: Option<(u32, u32)>,
    done: u32,
    /// How many times the server ended the session.
    stopped: u32,
    /// Frames that were delivered and frames the server refused.
    ready: u64,
    failed: u64,
}

/// The pixels a frame is written into: a file the compositor maps through a `wl_shm` pool.
struct Buffer {
    _file: File,
    buffer: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
}

/// A session on the first output and the frame it has asked for.
pub struct Capture {
    _connection: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    globals: GlobalList,
    shm: wl_shm::WlShm,
    session: ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    buffer: Option<Buffer>,
    /// The frame in flight and how many have been asked for.
    frame: Option<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1>,
    asked: u64,
    /// How many sessions this client has opened, the first one included.
    sessions: u32,
    /// Steps to wait before opening another session after the server ended one. A session asked for while the
    /// session is locked is ended at once, so a recorder that reopened on every step would open a session a step.
    cooldown: u32,
}

/// How many steps a recorder waits after its session ended before it asks for another.
const REOPEN_AFTER: u32 = 10;

impl Capture {
    /// Connects to the privileged socket and opens a session on the first output, or `None` when the compositor does not
    /// offer capture there.
    pub fn open(socket: &Path) -> Option<Self> {
        let stream = UnixStream::connect(socket).ok()?;
        let connection = Connection::from_socket(stream).ok()?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection).ok()?;
        let qh = queue.handle();
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).ok()?;
        let mut state = State::default();
        let session = open_session(&globals, &qh)?;
        // The constraints arrive before the first frame can be asked for.
        for _ in 0..50 {
            queue.roundtrip(&mut state).ok()?;
            if state.done > 0 {
                break;
            }
        }
        let mut capture = Self {
            _connection: connection,
            queue,
            qh,
            state,
            globals,
            shm,
            session,
            buffer: None,
            frame: None,
            asked: 0,
            sessions: 1,
            cooldown: 0,
        };
        (capture.state.done > 0).then(|| {
            capture.settle();
            capture
        })
    }

    /// Takes what the server sent, retires the frame that ended, and asks for the next one when none is in flight.
    ///
    /// Never blocks on a frame: a frame is delivered when the output next has damage, and a step that waited for one
    /// would hold the churn for as long as the compositor is idle.
    pub fn step(&mut self) {
        if self.queue.roundtrip(&mut self.state).is_err() {
            return;
        }
        self.settle();
    }

    fn settle(&mut self) {
        if self.state.stopped >= self.sessions {
            // The server ended the session (the output went away or the lock ended it): a recorder opens another,
            // not at once, because while the lock holds the new one is ended as it is made.
            if self.cooldown > 0 {
                self.cooldown -= 1;
                return;
            }
            self.cooldown = REOPEN_AFTER;
            if let Some(session) = open_session(&self.globals, &self.qh) {
                self.state.size = None;
                self.sessions += 1;
                self.session.destroy();
                self.session = session;
                let done = self.state.done;
                for _ in 0..50 {
                    if self.queue.roundtrip(&mut self.state).is_err() || self.state.done > done {
                        break;
                    }
                }
            }
            return;
        }
        if self.state.ready + self.state.failed >= self.asked
            && let Some(frame) = self.frame.take()
        {
            frame.destroy();
        }
        if self.frame.is_some() {
            return;
        }
        let Some((width, height)) = self.state.size else { return };
        if self
            .buffer
            .as_ref()
            .is_none_or(|buffer| (buffer.width, buffer.height) != (width, height))
        {
            if let Some(old) = self.buffer.take() {
                old.buffer.destroy();
            }
            self.buffer = self.allocate(width, height);
        }
        let Some(buffer) = &self.buffer else { return };
        let frame = self.session.create_frame(&self.qh, ());
        frame.attach_buffer(&buffer.buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        self.asked += 1;
        self.frame = Some(frame);
    }

    /// A shared-memory buffer of `width` by `height` pixels, or `None` when the file cannot be made.
    fn allocate(&self, width: u32, height: u32) -> Option<Buffer> {
        let stride = width.checked_mul(4)?;
        let length = u64::from(stride) * u64::from(height);
        let file = tempfile::tempfile().ok()?;
        file.set_len(length).ok()?;
        let pool: wl_shm_pool::WlShmPool =
            self.shm
                .create_pool(file.as_fd(), i32::try_from(length).ok()?, &self.qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            &self.qh,
            (),
        );
        // The buffer keeps the memory alive; the pool object is not needed again.
        pool.destroy();
        Some(Buffer {
            _file: file,
            buffer,
            width,
            height,
        })
    }

    /// Frames delivered, frames refused, and sessions opened so far.
    pub fn counts(&self) -> (u64, u64, u32) {
        (self.state.ready, self.state.failed, self.sessions)
    }
}

/// A session on the first output the server lists.
fn open_session(
    globals: &GlobalList,
    qh: &QueueHandle<State>,
) -> Option<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1> {
    let manager: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 = globals.bind(qh, 1..=1, ()).ok()?;
    let sources: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 =
        globals.bind(qh, 1..=1, ()).ok()?;
    let name = globals
        .contents()
        .clone_list()
        .into_iter()
        .find(|global| global.interface == "wl_output")?;
    let output = globals
        .registry()
        .bind::<wl_output::WlOutput, _, _>(name.name, name.version.min(4), qh, ());
    let source = sources.create_source(&output, qh, ());
    Some(manager.create_session(&source, ext_image_copy_capture_manager_v1::Options::empty(), qh, ()))
}

macro_rules! ignore_events {
    ($($proxy:ty),* $(,)?) => {$(
        impl Dispatch<$proxy, ()> for State {
            fn event(_: &mut Self, _: &$proxy, _: <$proxy as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}

ignore_events!(
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_output::WlOutput,
    ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
);

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

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.size = Some((width, height))
            }
            ext_image_copy_capture_session_v1::Event::Done => state.done += 1,
            ext_image_copy_capture_session_v1::Event::Stopped => state.stopped += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.ready += 1,
            // A frame refused for a buffer that no longer matches is answered by the next constraints round.
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => state.failed += 1,
            _ => {}
        }
    }
}
