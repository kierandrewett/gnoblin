//! PipeWire video streams for capture sessions the table has granted.
//!
//! A portal ScreenCast client reads a PipeWire node. That node exists only for a session the
//! capture table holds with its consent grant ([`CaptureStream::Granted`]) and that has not
//! stopped. Three rules follow, and each has a failure-mode test in `tests.rs`:
//!
//! - **No grant, no stream.** [`PipewireExport::start`] refuses a session without a grant before
//!   it connects to PipeWire, so no stream is created and no node id exists.
//! - **A node id is the owner's.** [`PipewireExport::node_id`] answers only the connection that
//!   opened the session; any other caller is refused.
//! - **The stream ends with its authority.** When the owner disconnects, the session is stopped
//!   (by the lock embargo or a target going away), or it is closed, the next
//!   [`PipewireExport::pump`] drops the stream and its node. The lock is applied to every stream on
//!   every pump ([`Table::enforce`]), so a stream nobody reads is cut too.
//!
//! Frames are pulled from the same [`Table`] rules the capture protocols use, so a stream is
//! paced and refused by the same code, and a refused frame carries no pixels.
//!
//! - **A buffer is delivered in the pump that fills it.** The producer fills a buffer from the picture the compositor
//!   holds, queues it and runs the stream's cycle at once, so no buffer the producer filled waits on its side across
//!   a `secure` mark or the lock. A consumer that took nothing since before the mark is not handed a buffer filled
//!   before it. What a consumer had already taken is the consumer's own. The tests are
//!   `no_buffer_queued_before_a_mark_is_delivered_after_it_on_a_real_cast` and
//!   `no_buffer_queued_before_the_lock_is_delivered_after_it_on_a_real_cast` in `gnoblin-wayland`'s
//!   `tests/view_capture_e2e.rs`, on a real daemon.
//!
//! A stream of a [`Target::View`] or a [`Target::Window`] is that target at the size it is drawn at: when the size
//! changes the format is offered again at the new size, the table's session takes the new shape, and the consumer
//! is renegotiated. An output's stream keeps the shape it started with. A frame carries only the pixels of the
//! target's own rectangle, so a window that shrank below the size the stream negotiated is not padded with the
//! desktop around it: what lies outside the window is black until the renegotiation lands.
//!
//! The `testing` feature (and this crate's own tests) compile `testing.rs`: a private daemon, a second
//! client that watches it, and a consumer that reads a node's frames.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Once;
use std::time::{Duration, Instant};

use gnoblin_capture::{BufferId, CaptureOutcome, CaptureSource, Copied, Format, PixelFormat, SessionId, Table, Target};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_render_frame::image::Image;
use gnoblin_types::{ClientId, Point, Rect, Tick};
use pipewire::context::ContextRc;
use pipewire::core::CoreRc;
use pipewire::loop_::Timeout;
use pipewire::main_loop::MainLoopRc;
use pipewire::properties::properties;
use pipewire::spa;
use pipewire::spa::param::ParamType;
use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pipewire::spa::param::video::VideoFormat;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Object, Pod, Property, PropertyFlags, Value};
use pipewire::spa::utils::{Direction, Fraction, Rectangle, SpaTypes};
use pipewire::stream::{StreamFlags, StreamListener, StreamRc, StreamState};

/// The frame rate the stream advertises. A stream produces a frame each [`FRAME_INTERVAL`].
const FRAME_RATE: u32 = 30;
/// The time between frames: one frame per 33 ms, about 30 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// How many PipeWire buffers the stream offers. Each holds one frame.
const BUFFER_COUNT: i32 = 4;
/// How long a new stream may take to appear in the daemon before the start fails.
const START_DEADLINE: Duration = Duration::from_secs(5);
/// The one buffer id a stream uses. A stream holds one buffer at a time.
const BUFFER: u64 = 1;

/// Why a PipeWire stream was not started, or why a node id was not given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// The session is not in the table.
    Unknown,
    /// The session holds no consent grant, so no stream may exist for it.
    NotGranted,
    /// The session has stopped and cannot be exported.
    Stopped,
    /// The caller did not open the session, so it may not read the node id.
    NotOwner,
    /// The session has no stream.
    NotExported,
    /// PipeWire could not be reached or refused the stream.
    Pipewire(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => formatter.write_str("the capture session is unknown"),
            Self::NotGranted => formatter.write_str("the capture session holds no consent grant"),
            Self::Stopped => formatter.write_str("the capture session has stopped"),
            Self::NotOwner => formatter.write_str("the caller does not own the capture session"),
            Self::NotExported => formatter.write_str("the capture session has no PipeWire stream"),
            Self::Pipewire(message) => write!(formatter, "PipeWire: {message}"),
        }
    }
}

/// Where a PipeWire frame's pixels come from. The compositor implements it over the same
/// targets the capture protocols copy from, so a stream and a screencopy agree on pixels.
pub trait PlaneSource {
    /// What `target` covers now, or [`None`] when it is gone.
    fn bounds(&self, target: Target) -> Option<Rect>;

    /// Writes `region` of `target` into `plane`, laid out as `format` describes.
    fn copy_into(&mut self, target: Target, region: Rect, format: Format, plane: &mut [u8]) -> Copied;
}

/// Adapts a [`PlaneSource`] and one buffer's plane to the [`CaptureSource`] the table copies
/// through.
struct Plane<'a> {
    source: &'a mut dyn PlaneSource,
    plane: &'a mut [u8],
}

impl CaptureSource for Plane<'_> {
    fn bounds(&self, target: Target) -> Option<Rect> {
        self.source.bounds(target)
    }

    fn copy(&mut self, target: Target, region: Rect, format: Format) -> Copied {
        self.source.copy_into(target, region, format, self.plane)
    }
}

/// Writes `region` of `image` into `plane` as the packed 32-bit layout `format` names, from the
/// plane's top-left corner. A pixel the image does not hold, or that lies past the extent of `region` (the target is
/// smaller than the format the stream negotiated), is written black, so no byte of an earlier frame survives into a
/// later one and no pixel the target does not own is copied.
pub fn write_region(image: &Image, region: Rect, format: Format, plane: &mut [u8]) -> Copied {
    let Some(length) = format.bytes() else {
        return Copied::Nothing;
    };
    if plane.len() < length {
        return Copied::Nothing;
    }
    let stride = format.stride() as usize;
    let size = format.size();
    let row_bytes = size.width as usize * BYTES_PER_PIXEL;
    for row in 0..size.height as usize {
        let Some(line) = plane.get_mut(row * stride..row * stride + row_bytes) else {
            return Copied::Nothing;
        };
        let (pixels, _) = line.as_chunks_mut::<BYTES_PER_PIXEL>();
        for (column, slot) in pixels.iter_mut().enumerate() {
            let inside = (column as u64) < u64::from(region.width) && (row as u64) < u64::from(region.height);
            let x = region.x.saturating_add_unsigned(column as u32);
            let y = region.y.saturating_add_unsigned(row as u32);
            let [red, green, blue, alpha] = if inside {
                image.pixel(Point::new(x, y)).unwrap_or([0, 0, 0, 0])
            } else {
                [0, 0, 0, 0]
            };
            let alpha = match format.pixel() {
                PixelFormat::Argb8888 => alpha,
                PixelFormat::Xrgb8888 => u8::MAX,
            };
            // The stream layouts are BGRA and BGRx: blue is the first byte.
            slot.copy_from_slice(&[blue, green, red, alpha]);
        }
    }
    Copied::Copied
}

const BYTES_PER_PIXEL: usize = 4;

/// One PipeWire connection, shared by every stream. The main loop drives the connection.
struct PipewireConnection {
    // Fields drop in declaration order: the core before the context, the context before the loop.
    core: CoreRc,
    _context: ContextRc,
    mainloop: MainLoopRc,
}

/// Initialises libpipewire once for the process. `pw_init` is not safe to call from two threads at once, so
/// every connection in this crate, and the test helpers, go through this one `Once`.
pub(crate) fn init_pipewire() {
    static INIT: Once = Once::new();
    INIT.call_once(pipewire::init);
}

impl PipewireConnection {
    fn open(socket: Option<&Path>) -> Result<Self, ExportError> {
        init_pipewire();
        let mainloop = MainLoopRc::new(None).map_err(pipewire_error)?;
        let context = ContextRc::new(&mainloop, None).map_err(pipewire_error)?;
        let core = match socket {
            Some(path) => {
                let stream = UnixStream::connect(path).map_err(pipewire_error)?;
                context.connect_fd_rc(OwnedFd::from(stream), None)
            }
            None => context.connect_rc(None),
        }
        .map_err(pipewire_error)?;
        Ok(Self {
            core,
            _context: context,
            mainloop,
        })
    }
}

fn pipewire_error(error: impl Display) -> ExportError {
    ExportError::Pipewire(error.to_string())
}

/// One granted session's stream.
struct PipewireExportStream {
    /// The session the stream serves. Its grant is checked on every pump.
    session: SessionId,
    /// The format the stream was negotiated in.
    format: Format,
    /// The node the daemon gave the stream.
    node: u32,
    /// The buffer layout the stream answers a format negotiation with. It is replaced when the format changes.
    buffers: Rc<RefCell<Vec<u8>>>,
    /// Answers the daemon's format negotiation with the buffers. Declared before the stream so it drops first.
    _listener: StreamListener<()>,
    /// The stream. Dropping it destroys the node.
    stream: StreamRc,
    /// The earliest moment the stream may produce its next frame.
    next_frame: Instant,
}

impl PipewireExportStream {
    /// Offers the format again when the stream serves a View or a window whose size changed.
    ///
    /// A View is sized by its owner and a window by its user, and what the stream carries is the target at the size
    /// it is drawn, so a View that grew or a window that was dragged larger is not cropped to the size it had when
    /// the stream started. The session takes the new shape, the next negotiation answers with buffers of the new
    /// size, and the consumer hears the format change. At most one renegotiation happens per frame interval, so a
    /// drag costs its consumer no more than 30 a second. An output is not followed: its size is the monitor's.
    fn follow_target_size(&mut self, table: &mut Table, source: &dyn PlaneSource) {
        let Some(target) = table.get(self.session).map(|entry| entry.target()) else {
            return;
        };
        if !matches!(target, Target::View(_) | Target::Window(_)) {
            return;
        }
        let Some(size) = source.bounds(target).map(|bounds| bounds.size()) else {
            return;
        };
        if size == self.format.size() {
            return;
        }
        let Some(format) = Format::new(size, size.width.saturating_mul(4), self.format.pixel()) else {
            return;
        };
        let (Ok(format_bytes), Ok(buffer_bytes)) = (format_pod(format), buffer_pod(format)) else {
            return;
        };
        let Some(pod) = Pod::from_bytes(&format_bytes) else {
            return;
        };
        *self.buffers.borrow_mut() = buffer_bytes;
        if self.stream.update_params(&mut [pod]).is_err() {
            // The daemon did not take the new format: the stream keeps the old one and tries again on the next frame.
            return;
        }
        table.reformat(self.session, format);
        self.format = format;
    }

    /// Produces one frame if the table allows one and the stream has a free buffer.
    fn frame(&mut self, table: &mut Table, tick: Tick, capture: Capture, source: &mut dyn PlaneSource) {
        // The lock ends a stream whether or not anyone is reading it: a stream nobody reads is never asked for a frame,
        // and a frame is where the lock would otherwise be applied.
        if !table.enforce(self.session, capture) {
            return;
        }
        let now = Instant::now();
        if now < self.next_frame {
            return;
        }
        self.next_frame = now + FRAME_INTERVAL;
        self.follow_target_size(table, source);
        // A DRIVER stream is its own graph driver, so nothing else schedules it: tick it even while
        // Paused, because a Paused stream never reaches Streaming on its own.
        if self.stream.state() != StreamState::Streaming {
            let _ = self.stream.trigger_process();
            if self.stream.state() != StreamState::Streaming {
                return;
            }
        }
        self.fill(table, tick, capture, source);
        // The cycle of this pump runs after the fill, so the buffer just queued is delivered now and not in the next
        // pump: a buffer the producer filled would otherwise wait in the daemon across whatever the compositor does in
        // between (a `secure` mark, the lock), and a consumer would be handed the picture from before it. It is the
        // pump's one cycle: it also gives the consumer's buffers back, so a stalled consumer cannot hold the stream.
        let _ = self.stream.trigger_process();
    }

    /// Fills one buffer if the table allows a frame and the stream has a free one. The buffer is queued to the stream
    /// when this returns.
    fn fill(&mut self, table: &mut Table, tick: Tick, capture: Capture, source: &mut dyn PlaneSource) {
        let Ok(claim) = table.request(self.session, BufferId::new(BUFFER), tick) else {
            return;
        };
        let Some(mut buffer) = self.stream.dequeue_buffer() else {
            table.cancel(claim);
            return;
        };
        let Some(data) = buffer.datas_mut().first_mut() else {
            table.cancel(claim);
            return;
        };
        let Some(length) = self.format.bytes() else {
            table.cancel(claim);
            return;
        };
        let Some(bytes) = data.data() else {
            table.cancel(claim);
            return;
        };
        let Some(plane) = bytes.get_mut(..length) else {
            table.cancel(claim);
            return;
        };
        let outcome = {
            let mut plane_source = Plane { source, plane };
            table.capture(claim, tick, capture, &mut plane_source)
        };
        let chunk = data.chunk_mut();
        match outcome {
            CaptureOutcome::Copied(_) => {
                *chunk.size_mut() = length as u32;
                *chunk.stride_mut() = self.format.stride() as i32;
                *chunk.offset_mut() = 0;
            }
            CaptureOutcome::Refused(_) => {
                *chunk.size_mut() = 0;
            }
        }
        // Dropping the buffer queues it to the stream.
    }
}

/// The PipeWire streams of the granted capture sessions.
///
/// It has no connection until the first stream starts, and it holds none once the last stream
/// has ended. Field order matters: the streams drop before the connection they belong to.
pub struct PipewireExport {
    /// The streams, by the session each serves.
    exports: BTreeMap<SessionId, PipewireExportStream>,
    /// The connection the streams share, if any stream is live.
    connection: Option<PipewireConnection>,
    /// The daemon socket to connect to. [`None`] uses the environment's default.
    socket: Option<PathBuf>,
}

impl Default for PipewireExport {
    fn default() -> Self {
        Self::new()
    }
}

impl PipewireExport {
    /// An export that connects to the daemon the environment names.
    pub fn new() -> Self {
        Self {
            exports: BTreeMap::new(),
            connection: None,
            socket: None,
        }
    }

    /// An export that connects to the daemon listening on `socket`, instead of the one the
    /// environment names. A session names its daemon this way when it must not reach the
    /// user's own, and a test names its private one.
    pub fn with_socket(socket: PathBuf) -> Self {
        Self {
            exports: BTreeMap::new(),
            connection: None,
            socket: Some(socket),
        }
    }

    /// Starts the stream for `session` and answers its node id.
    ///
    /// The session must hold [`CaptureStream::Granted`] and must not have stopped. Any other
    /// session is refused before PipeWire is touched, so no stream and no node id come into
    /// being for it. Starting an already-exported session answers its existing node.
    pub fn start(&mut self, table: &Table, session: SessionId, caller: ClientId) -> Result<u32, ExportError> {
        let entry = table.get(session).ok_or(ExportError::Unknown)?;
        if entry.owner() != caller {
            return Err(ExportError::NotOwner);
        }
        if entry.stream() != CaptureStream::Granted {
            return Err(ExportError::NotGranted);
        }
        if entry.stopped().is_some() {
            return Err(ExportError::Stopped);
        }
        if let Some(export) = self.exports.get(&session) {
            return Ok(export.node);
        }
        let format = entry.format();

        if self.connection.is_none() {
            self.connection = Some(PipewireConnection::open(self.socket.as_deref())?);
        }
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| ExportError::Pipewire("no connection".to_owned()))?;
        let core = connection.core.clone();
        let mainloop = connection.mainloop.clone();

        let name = format!("gnoblin-capture-{}", session.get());
        let stream = StreamRc::new(
            core,
            &name,
            properties! {
                *pipewire::keys::NODE_NAME => name.as_str(),
                *pipewire::keys::MEDIA_TYPE => "Video",
                *pipewire::keys::MEDIA_CATEGORY => "Capture",
                *pipewire::keys::MEDIA_CLASS => "Video/Source",
                *pipewire::keys::NODE_ALWAYS_PROCESS => "true",
                "node.want-driver" => "true",
            },
        )
        .map_err(pipewire_error)?;
        let format_pod = format_pod(format)?;
        let buffer_pod = buffer_pod(format)?;
        let mut params = [
            Pod::from_bytes(&format_pod).ok_or_else(|| ExportError::Pipewire("format pod".to_owned()))?,
            Pod::from_bytes(&buffer_pod).ok_or_else(|| ExportError::Pipewire("buffer pod".to_owned()))?,
        ];
        // The daemon fixes the format, then waits for the buffers this stream answers with; without
        // them the link never leaves Paused.
        let buffers = Rc::new(RefCell::new(buffer_pod.clone()));
        let answer = Rc::clone(&buffers);
        let listener = stream
            .add_local_listener_with_user_data(())
            .param_changed(move |output, _, id, param| {
                if id != ParamType::Format.as_raw() || param.is_none() {
                    return;
                }
                let bytes = answer.borrow();
                if let Some(buffers) = Pod::from_bytes(&bytes) {
                    let _ = output.update_params(&mut [buffers]);
                }
            })
            .register()
            .map_err(pipewire_error)?;
        stream
            .connect(
                Direction::Output,
                None,
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::DRIVER,
                &mut params,
            )
            .map_err(pipewire_error)?;
        let node = wait_for_node(&mainloop, &stream)?;

        self.exports.insert(
            session,
            PipewireExportStream {
                session,
                format,
                node,
                buffers,
                _listener: listener,
                stream,
                next_frame: Instant::now(),
            },
        );
        Ok(node)
    }

    /// The node id of `session`, if `caller` opened it and it has a stream.
    pub fn node_id(&self, table: &Table, session: SessionId, caller: ClientId) -> Result<u32, ExportError> {
        let entry = table.get(session).ok_or(ExportError::Unknown)?;
        if entry.owner() != caller {
            return Err(ExportError::NotOwner);
        }
        self.exports
            .get(&session)
            .map(|export| export.node)
            .ok_or(ExportError::NotExported)
    }
    /// Whether any stream is live, so the compositor only pumps when one is.
    pub fn is_live(&self) -> bool {
        !self.exports.is_empty()
    }

    /// Advances every live stream by one iteration of the connection, ends the streams whose
    /// authority has gone, and produces a frame for each streaming session the table allows.
    pub fn pump(&mut self, table: &mut Table, tick: Tick, capture: Capture, source: &mut dyn PlaneSource) {
        self.reap(table);
        if let Some(connection) = &self.connection {
            connection.mainloop.loop_().iterate(Timeout::Finite(Duration::ZERO));
        }
        for export in self.exports.values_mut() {
            export.frame(table, tick, capture, source);
        }
    }

    /// Drops every stream whose session is gone, stopped, or no longer granted, and the
    /// connection when no stream is left.
    fn reap(&mut self, table: &Table) {
        self.exports.retain(|session, _| {
            table
                .get(*session)
                .is_some_and(|entry| entry.stream() == CaptureStream::Granted && entry.stopped().is_none())
        });
        if self.exports.is_empty() {
            self.connection = None;
        }
    }
}

/// Iterates the loop until the daemon has given `stream` a node and the stream is paused or
/// streaming, or until the deadline passes.
fn wait_for_node(mainloop: &MainLoopRc, stream: &StreamRc) -> Result<u32, ExportError> {
    let started = Instant::now();
    loop {
        let state = stream.state();
        if let StreamState::Error(message) = &state {
            return Err(ExportError::Pipewire(message.clone()));
        }
        let node = stream.node_id();
        if node != u32::MAX && matches!(state, StreamState::Paused | StreamState::Streaming) {
            return Ok(node);
        }
        if started.elapsed() >= START_DEADLINE {
            return Err(ExportError::Pipewire(
                "the stream never appeared in the daemon".to_owned(),
            ));
        }
        mainloop.loop_().iterate(Timeout::Finite(Duration::from_millis(10)));
    }
}

/// The video format a stream offers: the session's pixel layout, size and the advertised rate.
fn format_pod(format: Format) -> Result<Vec<u8>, ExportError> {
    let video = match format.pixel() {
        PixelFormat::Argb8888 => VideoFormat::BGRA,
        PixelFormat::Xrgb8888 => VideoFormat::BGRx,
    };
    let size = format.size();
    let object = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(FormatProperties::VideoFormat, Id, video),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Rectangle,
            Rectangle {
                width: size.width,
                height: size.height
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Fraction,
            Fraction {
                num: FRAME_RATE,
                denom: 1
            }
        ),
    );
    serialise(&Value::Object(object), "format")
}

/// The buffer layout a stream needs: [`BUFFER_COUNT`] memfd planes, each holding one frame.
fn buffer_pod(format: Format) -> Result<Vec<u8>, ExportError> {
    let bytes = format
        .bytes()
        .ok_or_else(|| ExportError::Pipewire("the format has no size".to_owned()))?;
    let bytes = i32::try_from(bytes).map_err(pipewire_error)?;
    let stride = i32::try_from(format.stride()).map_err(pipewire_error)?;
    let int = |key: u32, value: i32| Property {
        key,
        flags: PropertyFlags::empty(),
        value: Value::Int(value),
    };
    let object = Object {
        type_: SpaTypes::ObjectParamBuffers.as_raw(),
        id: ParamType::Buffers.as_raw(),
        properties: vec![
            int(spa::sys::SPA_PARAM_BUFFERS_buffers, BUFFER_COUNT),
            int(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(spa::sys::SPA_PARAM_BUFFERS_size, bytes),
            int(spa::sys::SPA_PARAM_BUFFERS_stride, stride),
            int(spa::sys::SPA_PARAM_BUFFERS_align, 16),
            // One bit per memory type: 1 << 2 is memfd, the only type this producer maps.
            int(spa::sys::SPA_PARAM_BUFFERS_dataType, 1 << 2),
        ],
    };
    serialise(&Value::Object(object), "buffers")
}

fn serialise(value: &Value, what: &str) -> Result<Vec<u8>, ExportError> {
    PodSerializer::serialize(Cursor::new(Vec::new()), value)
        .map(|(cursor, _)| cursor.into_inner())
        .map_err(|error| ExportError::Pipewire(format!("serialise the {what} params: {error}")))
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod tests;
