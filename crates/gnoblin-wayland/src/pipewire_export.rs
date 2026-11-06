//! PipeWire video streams for capture sessions the table has granted.
//!
//! A portal ScreenCast client reads a PipeWire node. That node exists only for a session the
//! capture table holds with its consent grant ([`CaptureStream::Granted`]) and that has not
//! stopped. Three rules follow, and each has a failure-mode test in `pipewire_export/tests.rs`:
//!
//! - **No grant, no stream.** [`PipewireExport::start`] refuses a session without a grant before
//!   it connects to PipeWire, so no stream is created and no node id exists.
//! - **A node id is the owner's.** [`PipewireExport::node_id`] answers only the connection that
//!   opened the session; any other caller is refused.
//! - **The stream ends with its authority.** When the owner disconnects, the session is stopped
//!   (by the lock embargo or a target going away), or it is closed, the next
//!   [`PipewireExport::pump`] drops the stream and its node.
//!
//! Frames are pulled from the same [`Table`] rules the capture protocols use, so a stream is
//! paced and refused by the same code, and a refused frame carries no pixels.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, Instant};

use gnoblin_backend::render::Image;
use gnoblin_capture::{BufferId, CaptureOutcome, CaptureSource, Copied, Format, PixelFormat, SessionId, Table, Target};
use gnoblin_lock::{Capture, CaptureStream};
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
pub(crate) enum ExportError {
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
pub(crate) trait PlaneSource {
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
/// plane's top-left corner. A pixel the image does not hold is written black, so no byte of an
/// earlier frame survives into a later one.
pub(crate) fn write_region(image: &Image, region: Rect, format: Format, plane: &mut [u8]) -> Copied {
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
            let x = region.x.saturating_add_unsigned(column as u32);
            let y = region.y.saturating_add_unsigned(row as u32);
            let [red, green, blue, alpha] = image.pixel(Point::new(x, y)).unwrap_or([0, 0, 0, 0]);
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

impl PipewireConnection {
    fn open(socket: Option<&Path>) -> Result<Self, ExportError> {
        static INIT: Once = Once::new();
        INIT.call_once(pipewire::init);
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
    /// Answers the daemon's format negotiation with the buffers. Declared before the stream so it drops first.
    _listener: StreamListener<()>,
    /// The stream. Dropping it destroys the node.
    stream: StreamRc,
    /// The earliest moment the stream may produce its next frame.
    next_frame: Instant,
}

impl PipewireExportStream {
    /// Produces one frame if the table allows one and the stream has a free buffer.
    fn frame(&mut self, table: &mut Table, tick: Tick, capture: Capture, source: &mut dyn PlaneSource) {
        let now = Instant::now();
        if now < self.next_frame {
            return;
        }
        self.next_frame = now + FRAME_INTERVAL;
        // A DRIVER stream is its own graph driver, so nothing else schedules it: tick it even while
        // Paused, because a Paused stream never reaches Streaming on its own.
        let _ = self.stream.trigger_process();
        if self.stream.state() != StreamState::Streaming {
            return;
        }
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
        // Dropping the buffer queues it back to the stream.
    }
}

/// The PipeWire streams of the granted capture sessions.
///
/// It has no connection until the first stream starts, and it holds none once the last stream
/// has ended. Field order matters: the streams drop before the connection they belong to.
pub(crate) struct PipewireExport {
    /// The streams, by the session each serves.
    exports: BTreeMap<SessionId, PipewireExportStream>,
    /// The connection the streams share, if any stream is live.
    connection: Option<PipewireConnection>,
    /// The daemon socket to connect to. [`None`] uses the environment's default.
    socket: Option<PathBuf>,
}

impl PipewireExport {
    /// An export that connects to the daemon the environment names.
    pub(crate) fn new() -> Self {
        Self {
            exports: BTreeMap::new(),
            connection: None,
            socket: None,
        }
    }

    /// An export that connects to the daemon listening on `socket`.
    #[cfg(test)]
    pub(crate) fn with_socket(socket: PathBuf) -> Self {
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
    pub(crate) fn start(&mut self, table: &Table, session: SessionId, caller: ClientId) -> Result<u32, ExportError> {
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
        let answer = buffer_pod.clone();
        let listener = stream
            .add_local_listener_with_user_data(())
            .param_changed(move |output, _, id, param| {
                if id != ParamType::Format.as_raw() || param.is_none() {
                    return;
                }
                if let Some(buffers) = Pod::from_bytes(&answer) {
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
                _listener: listener,
                stream,
                next_frame: Instant::now(),
            },
        );
        Ok(node)
    }

    /// The node id of `session`, if `caller` opened it and it has a stream.
    pub(crate) fn node_id(&self, table: &Table, session: SessionId, caller: ClientId) -> Result<u32, ExportError> {
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
    pub(crate) fn is_live(&self) -> bool {
        !self.exports.is_empty()
    }

    /// Advances every live stream by one iteration of the connection, ends the streams whose
    /// authority has gone, and produces a frame for each streaming session the table allows.
    pub(crate) fn pump(&mut self, table: &mut Table, tick: Tick, capture: Capture, source: &mut dyn PlaneSource) {
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

#[cfg(test)]
mod tests;
