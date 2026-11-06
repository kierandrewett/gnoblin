//! Proves that a PipeWire video stream round-trips through a private daemon.
//!
//! The test starts its own `pipewire` daemon in a temporary runtime directory and
//! connects one client to its socket. A producer output stream and a consumer input
//! stream share that client's main loop. The producer writes BGRx frames whose blue
//! channel encodes the frame number; the consumer links to the producer's node,
//! negotiates the same format and checks every frame it receives.

use std::cell::RefCell;
use std::fs::File;
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use pipewire::context::ContextRc;
use pipewire::loop_::Timeout;
use pipewire::main_loop::MainLoopRc;
use pipewire::properties::properties;
use pipewire::spa;
use pipewire::spa::param::ParamType;
use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pipewire::spa::param::format_utils;
use pipewire::spa::param::video::{VideoFormat, VideoInfoRaw};
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Object, Pod, Property, PropertyFlags, Value};
use pipewire::spa::utils::{Direction, Fraction, Rectangle, SpaTypes};
use pipewire::stream::{StreamBox, StreamFlags, StreamRc, StreamState};
use pipewire::types::ObjectType;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const FPS: u32 = 30;
const BYTES_PER_PIXEL: usize = 4;
const STRIDE: usize = WIDTH as usize * BYTES_PER_PIXEL;
const FRAME_BYTES: usize = STRIDE * HEIGHT as usize;
const FRAMES_TO_CHECK: usize = 5;
/// The blue channel cycles through this many distinct values, one per frame.
const CYCLE: u8 = 15;
const DEADLINE: Duration = Duration::from_secs(20);

/// A private `pipewire` daemon. Dropping it kills the daemon and removes its runtime directory.
struct Daemon {
    child: Child,
    runtime_dir: PathBuf,
    log: PathBuf,
}

impl Daemon {
    fn start() -> Self {
        let runtime_dir = std::env::temp_dir().join(format!("gnoblin-pipewire-spike-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime_dir);
        std::fs::create_dir_all(&runtime_dir).expect("create the daemon runtime directory");
        let log = runtime_dir.join("daemon.log");
        let log_file = File::create(&log).expect("create the daemon log");
        let log_err = log_file.try_clone().expect("clone the daemon log handle");
        let binary = std::env::var_os("GNOBLIN_PIPEWIRE_BIN").unwrap_or_else(|| "pipewire".into());
        let child = Command::new(&binary)
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("PIPEWIRE_RUNTIME_DIR", &runtime_dir)
            .stdin(Stdio::null())
            .stdout(log_file)
            .stderr(log_err)
            .spawn()
            .unwrap_or_else(|error| panic!("start {binary:?}: {error}"));
        let mut daemon = Self {
            child,
            runtime_dir,
            log,
        };
        daemon.wait_for_socket();
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.runtime_dir.join("pipewire-0")
    }

    fn wait_for_socket(&mut self) {
        let started = Instant::now();
        while !self.socket().exists() {
            if let Some(status) = self.child.try_wait().expect("poll the daemon") {
                panic!("pipewire exited early ({status}): {}", self.log_tail());
            }
            assert!(
                started.elapsed() < DEADLINE,
                "pipewire socket never appeared at {}: {}",
                self.socket().display(),
                self.log_tail()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn log_tail(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

/// The format both streams offer: fixed BGRx, 64x48, 30 fps.
fn format_params() -> Vec<u8> {
    let object = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(FormatProperties::VideoFormat, Id, VideoFormat::BGRx),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Rectangle,
            Rectangle {
                width: WIDTH,
                height: HEIGHT
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Fraction,
            Fraction { num: FPS, denom: 1 }
        ),
    );
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .expect("serialise the format")
        .0
        .into_inner()
}

/// The buffer layout the producer needs: four memfd planes of one frame each. Without this the
/// daemon has no buffers to hand out and the consumer maps a zero-sized plane.
fn buffer_params() -> Vec<u8> {
    let int = |key: u32, value: i32| Property {
        key,
        flags: PropertyFlags::empty(),
        value: Value::Int(value),
    };
    let object = Object {
        type_: SpaTypes::ObjectParamBuffers.as_raw(),
        id: ParamType::Buffers.as_raw(),
        properties: vec![
            int(spa::sys::SPA_PARAM_BUFFERS_buffers, 4),
            int(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(spa::sys::SPA_PARAM_BUFFERS_size, FRAME_BYTES as i32),
            int(spa::sys::SPA_PARAM_BUFFERS_stride, STRIDE as i32),
            int(spa::sys::SPA_PARAM_BUFFERS_align, 16),
            int(spa::sys::SPA_PARAM_BUFFERS_dataType, 1 << 2),
        ],
    };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .expect("serialise the buffer params")
        .0
        .into_inner()
}

/// The blue byte for frame `index`. Every frame is uniform, so one pixel names the frame.
fn blue_for(index: u32) -> u8 {
    ((index % u32::from(CYCLE)) as u8 + 1) * 16
}

fn fill_frame(bytes: &mut [u8], index: u32) {
    let blue = blue_for(index);
    for pixel in bytes.as_chunks_mut::<BYTES_PER_PIXEL>().0 {
        *pixel = [blue, 0x40, 0x80, 0x00];
    }
}

/// What the consumer saw, filled in by its callbacks.
#[derive(Default)]
struct Sink {
    format: Option<VideoInfoRaw>,
    /// The blue byte of each frame, in arrival order.
    blues: Vec<u8>,
    errors: Vec<String>,
    /// Frames the producer has written, and how often its process callback ran.
    produced: u32,
    producer_calls: u32,
    /// Consumer state transitions, for diagnosing a stalled link.
    states: Vec<String>,
}

fn check_frame(bytes: &[u8], size: u32, stride: i32, offset: u32, sink: &mut Sink) {
    if size as usize != FRAME_BYTES {
        sink.errors.push(format!("chunk size {size}, expected {FRAME_BYTES}"));
        return;
    }
    if stride as usize != STRIDE {
        sink.errors.push(format!("chunk stride {stride}, expected {STRIDE}"));
        return;
    }
    let start = offset as usize;
    let Some(frame) = bytes.get(start..start + FRAME_BYTES) else {
        sink.errors
            .push(format!("frame at offset {offset} is outside the mapped buffer"));
        return;
    };
    let blue = frame[0];
    let mut mismatches = 0usize;
    for pixel in frame.as_chunks::<BYTES_PER_PIXEL>().0 {
        if pixel[0] != blue || pixel[1] != 0x40 || pixel[2] != 0x80 {
            mismatches += 1;
        }
    }
    if mismatches > 0 {
        sink.errors
            .push(format!("{mismatches} pixels differ from frame blue {blue:#04x}"));
    }
    sink.blues.push(blue);
}

fn connect_client(mainloop: &MainLoopRc, socket: &Path) -> pipewire::core::CoreRc {
    let context = ContextRc::new(mainloop, None).expect("create the client context");
    let stream = UnixStream::connect(socket).expect("connect to the daemon socket");
    context
        .connect_fd_rc(OwnedFd::from(stream), None)
        .expect("connect the client to the daemon")
}

/// Iterate the loop until `done` holds or the deadline passes. Returns whether `done` held.
fn drive(mainloop: &MainLoopRc, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while !done() {
        if started.elapsed() >= DEADLINE {
            return false;
        }
        mainloop.loop_().iterate(Timeout::Finite(Duration::from_millis(10)));
    }
    true
}

#[test]
fn pipewire_video_stream_round_trips_frames() {
    let daemon = Daemon::start();
    pipewire::init();

    let mainloop = MainLoopRc::new(None).expect("create the main loop");
    let core = connect_client(&mainloop, &daemon.socket());

    // Producer: a video source that writes numbered BGRx frames into the buffers PipeWire hands it.
    let producer = StreamRc::new(
        core.clone(),
        "gnoblin-spike-source",
        properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_CLASS => "Video/Source",
            *pipewire::keys::NODE_ALWAYS_PROCESS => "true",
            "node.want-driver" => "true",
        },
    )
    .expect("create the producer stream");
    let producer_trace = Rc::new(RefCell::new(Sink::default()));
    let _producer_listener = producer
        .add_local_listener_with_user_data(Rc::clone(&producer_trace))
        .process(|stream, trace| {
            trace.borrow_mut().producer_calls += 1;
            let Some(mut buffer) = stream.dequeue_buffer() else {
                trace.borrow_mut().errors.push("producer: no buffer to fill".to_owned());
                return;
            };
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else {
                trace
                    .borrow_mut()
                    .errors
                    .push("producer: buffer has no data plane".to_owned());
                return;
            };
            let Some(bytes) = data.data() else {
                trace
                    .borrow_mut()
                    .errors
                    .push("producer: data plane is not mapped".to_owned());
                return;
            };
            let index = trace.borrow().produced;
            fill_frame(bytes, index);
            let chunk = data.chunk_mut();
            *chunk.size_mut() = FRAME_BYTES as u32;
            *chunk.stride_mut() = STRIDE as i32;
            *chunk.offset_mut() = 0;
            trace.borrow_mut().produced += 1;
        })
        .register()
        .expect("register the producer listener");
    let format = format_params();
    let buffers = buffer_params();
    let mut producer_params = [
        Pod::from_bytes(&format).expect("valid format pod"),
        Pod::from_bytes(&buffers).expect("valid buffer pod"),
    ];
    producer
        .connect(
            Direction::Output,
            None,
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::DRIVER,
            &mut producer_params,
        )
        .expect("connect the producer stream");
    // A DRIVER stream is its own graph driver: nothing else schedules its process callback,
    // so tick it from a timer at 30 fps.
    let weak_producer = producer.downgrade();
    let timer_trace = Rc::clone(&producer_trace);
    let frame_timer = mainloop.loop_().add_timer(move |_| {
        let Some(producer) = weak_producer.upgrade() else {
            return;
        };
        if let Err(error) = producer.trigger_process() {
            timer_trace
                .borrow_mut()
                .errors
                .push(format!("trigger_process: {error}"));
        }
    });
    frame_timer
        .update_timer(Some(Duration::from_millis(33)), Some(Duration::from_millis(33)))
        .into_result()
        .expect("arm the frame timer");

    let producer_ready = drive(&mainloop, || {
        matches!(
            producer.state(),
            StreamState::Paused | StreamState::Streaming | StreamState::Error(_)
        )
    });
    assert!(
        producer_ready,
        "producer never reached Paused: state {:?}; daemon log: {}",
        producer.state(),
        daemon.log_tail()
    );
    if let StreamState::Error(message) = producer.state() {
        panic!("producer stream failed: {message}; daemon log: {}", daemon.log_tail());
    }
    let node_id = producer.node_id();
    assert_ne!(node_id, u32::MAX, "producer has no node id");

    // Consumer: an input stream that links to the producer's node by id.
    let sink = Rc::new(RefCell::new(Sink::default()));
    let consumer = StreamBox::new(
        &core,
        "gnoblin-spike-sink",
        properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Camera",
        },
    )
    .expect("create the consumer stream");
    let _consumer_listener = consumer
        .add_local_listener_with_user_data(Rc::clone(&sink))
        .param_changed(|_, sink, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
                return;
            }
            let mut info = VideoInfoRaw::default();
            match info.parse(param) {
                Ok(_) => sink.borrow_mut().format = Some(info),
                Err(error) => sink.borrow_mut().errors.push(format!("parse format: {error}")),
            }
        })
        .state_changed(|_, sink, old, new| {
            // The consumer is dropped while the test still holds the sink, so do not panic here.
            if let Ok(mut sink) = sink.try_borrow_mut() {
                sink.states.push(format!("{old:?}->{new:?}"));
            }
        })
        .process(|stream, sink| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else {
                return;
            };
            let chunk = data.chunk();
            let (size, stride, offset) = (chunk.size(), chunk.stride(), chunk.offset());
            let Some(bytes) = data.data() else {
                sink.borrow_mut()
                    .errors
                    .push("consumer buffer is not mapped".to_owned());
                return;
            };
            check_frame(bytes, size, stride, offset, &mut sink.borrow_mut());
        })
        .register()
        .expect("register the consumer listener");
    let mut consumer_params = [Pod::from_bytes(&format).expect("valid format pod")];
    consumer
        .connect(
            Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut consumer_params,
        )
        .expect("connect the consumer stream");
    // Without a session manager nothing links the streams, so create the link ourselves
    // from the port ids the registry advertises.
    let ports = Rc::new(RefCell::new(Vec::<(u32, String, u32)>::new()));
    let registry = core.get_registry_rc().expect("get the registry");
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let ports = Rc::clone(&ports);
            move |global| {
                if global.type_ != ObjectType::Port {
                    return;
                }
                let Some(props) = global.props else {
                    return;
                };
                let (Some(node), Some(direction)) = (props.get("node.id"), props.get("port.direction")) else {
                    return;
                };
                if let Ok(node) = node.parse::<u32>() {
                    ports.borrow_mut().push((node, direction.to_owned(), global.id));
                }
            }
        })
        .register();
    let port_of = |node: u32, direction: &str| {
        ports
            .borrow()
            .iter()
            .find(|(n, d, _)| *n == node && d == direction)
            .map(|(_, _, id)| *id)
    };
    let consumer_node = || consumer.node_id();
    let ready = drive(&mainloop, || {
        consumer_node() != u32::MAX && port_of(node_id, "out").is_some() && port_of(consumer_node(), "in").is_some()
    });
    assert!(
        ready,
        "ports never appeared: producer {node_id} out {:?}, consumer {} in {:?}; daemon log: {}",
        port_of(node_id, "out"),
        consumer_node(),
        port_of(consumer_node(), "in"),
        daemon.log_tail()
    );
    let link_properties = properties! {
        "link.output.node" => node_id.to_string(),
        "link.output.port" => port_of(node_id, "out").expect("producer output port").to_string(),
        "link.input.node" => consumer_node().to_string(),
        "link.input.port" => port_of(consumer_node(), "in").expect("consumer input port").to_string(),
    };
    let _link = core
        .create_object::<pipewire::link::Link>("link-factory", &link_properties)
        .expect("create the link between the streams");

    let frames_seen = drive(&mainloop, || {
        sink.borrow().blues.len() >= FRAMES_TO_CHECK || !sink.borrow().errors.is_empty()
    });
    if !frames_seen {
        panic!(
            "no frames at the consumer: consumer {:?}, producer {:?}, states {:?}, errors {:?}; daemon log: {}",
            consumer.state(),
            producer.state(),
            sink.borrow().states,
            (
                producer_trace.borrow().produced,
                producer_trace.borrow().producer_calls,
                &producer_trace.borrow().errors
            ),
            daemon.log_tail()
        );
    }

    let sink = sink.borrow();
    assert!(
        sink.errors.is_empty(),
        "consumer errors: {:?}; daemon log: {}",
        sink.errors,
        daemon.log_tail()
    );
    let info = sink.format.as_ref().expect("consumer negotiated a video format");
    assert_eq!(info.format(), VideoFormat::BGRx);
    assert_eq!(info.size().width, WIDTH);
    assert_eq!(info.size().height, HEIGHT);
    assert_eq!(info.framerate().num, FPS);
    assert_eq!(info.framerate().denom, 1);
    assert!(
        sink.blues.len() >= FRAMES_TO_CHECK,
        "received {} frames",
        sink.blues.len()
    );

    // The producer numbers frames consecutively, so the blue values advance by one step each time.
    for pair in sink.blues.windows(2) {
        let (previous, next) = (pair[0] / 16 - 1, pair[1] / 16 - 1);
        assert_eq!(
            next,
            (previous + 1) % CYCLE,
            "frames arrived out of order or with gaps: {:?}",
            sink.blues
        );
    }
    drop(consumer);
    drop(producer);
}
