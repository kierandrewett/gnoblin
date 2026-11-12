//! Test support for anything that streams through this crate: a private `pipewire` daemon, a second
//! client that watches what the daemon holds, and a consumer that reads the frames of one node.
//!
//! The recipe is `design/pipewire-capture-spike.md`. A private daemon runs no session manager, so
//! nothing links a producer to a consumer: [`FrameConsumer::connect`] makes the link a user session would.
//!
//! It is compiled for this crate's own tests and, behind the `testing` feature, for the integration
//! tests of the crates whose streams a test must read (`gnoblin-wayland`). Nothing else turns it on.

use std::cell::RefCell;
use std::fs::File;
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use pipewire::context::ContextRc;
use pipewire::core::CoreRc;
use pipewire::link::Link;
use pipewire::loop_::Timeout;
use pipewire::main_loop::MainLoopRc;
use pipewire::properties::properties;
use pipewire::registry::{Listener, RegistryRc};
use pipewire::spa;
use pipewire::spa::param::ParamType;
use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pipewire::spa::param::video::{VideoFormat, VideoInfoRaw};
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Pod, Value};
use pipewire::spa::utils::{Direction, Fraction, Rectangle, SpaTypes};
use pipewire::stream::{StreamFlags, StreamListener, StreamRc};
use pipewire::types::ObjectType;

/// How long a registry change may take to reach the observer, or a frame to reach a consumer, before a test
/// gives up.
pub const DEADLINE: Duration = Duration::from_secs(20);
/// How long the observer listens before it trusts that no node is coming.
const SETTLE: Duration = Duration::from_millis(400);

static NEXT_DAEMON: AtomicUsize = AtomicUsize::new(0);

/// A private `pipewire` daemon. Dropping it kills the daemon and removes its runtime directory.
pub struct Daemon {
    child: Child,
    runtime_dir: PathBuf,
    log: PathBuf,
}

impl Daemon {
    /// Starts the daemon named by `GNOBLIN_PIPEWIRE_BIN` (default `pipewire`) in a runtime directory of its own and
    /// waits for its socket.
    pub fn start() -> Self {
        let index = NEXT_DAEMON.fetch_add(1, Ordering::Relaxed);
        let runtime_dir = std::env::temp_dir().join(format!("gnoblin-pipewire-export-{}-{index}", std::process::id()));
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

    /// The daemon's socket.
    pub fn socket(&self) -> PathBuf {
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

/// One node the daemon holds, as the registry announced it.
#[derive(Debug)]
struct ObservedNode {
    id: u32,
    name: String,
    class: String,
    description: String,
}

/// One port the daemon holds: the node it belongs to and whether it is an output.
#[derive(Debug)]
struct SeenPort {
    id: u32,
    node: u32,
    output: bool,
}

/// A second client of the daemon. It records every node the daemon holds and forgets a node when the daemon
/// removes it.
pub struct Observer {
    // Fields drop in declaration order, and the listener goes before the registry it listens to. A `Listener`
    // holds only a `spa_hook` that is linked into the registry proxy's list: dropping the registry frees that list,
    // and a hook removed afterwards writes into freed memory (valgrind: "Invalid write of size 8" in
    // `spa_hook_remove`, "free'd" by `pw_core_disconnect`). The same rule holds for every listener in this file.
    _listener: Listener,
    _registry: RegistryRc,
    /// The client's loop, which every wait here iterates.
    pub mainloop: MainLoopRc,
    /// The client's connection to the daemon.
    pub core: CoreRc,
    /// Every node currently in the registry.
    nodes: Rc<RefCell<Vec<ObservedNode>>>,
    /// Every port currently in the registry.
    ports: Rc<RefCell<Vec<SeenPort>>>,
}

impl Observer {
    /// Connects to the daemon listening on `socket`.
    pub fn connect(socket: &Path) -> Self {
        crate::init_pipewire();
        let mainloop = MainLoopRc::new(None).expect("create the observer main loop");
        let core = connect_client(&mainloop, socket);
        let registry = core.get_registry_rc().expect("get the observer registry");
        let nodes = Rc::new(RefCell::new(Vec::<ObservedNode>::new()));
        let ports = Rc::new(RefCell::new(Vec::<SeenPort>::new()));
        let listener = registry
            .add_listener_local()
            .global({
                let nodes = Rc::clone(&nodes);
                let ports = Rc::clone(&ports);
                move |global| {
                    let props = global.props;
                    let prop = |key: &str| props.and_then(|dict| dict.get(key)).unwrap_or_default().to_owned();
                    if global.type_ == ObjectType::Port {
                        ports.borrow_mut().push(SeenPort {
                            id: global.id,
                            node: prop("node.id").parse().unwrap_or_default(),
                            output: prop("port.direction") == "out",
                        });
                        return;
                    }
                    if global.type_ != ObjectType::Node {
                        return;
                    }
                    nodes.borrow_mut().push(ObservedNode {
                        id: global.id,
                        name: prop("node.name"),
                        class: prop("media.class"),
                        description: prop("node.description"),
                    });
                }
            })
            .global_remove({
                let nodes = Rc::clone(&nodes);
                let ports = Rc::clone(&ports);
                move |id| {
                    nodes.borrow_mut().retain(|seen| seen.id != id);
                    ports.borrow_mut().retain(|port| port.id != id);
                }
            })
            .register();
        Self {
            mainloop,
            core,
            _registry: registry,
            _listener: listener,
            nodes,
            ports,
        }
    }

    /// The daemon's id of the node named `name`, if the registry holds one.
    pub fn node(&self, name: &str) -> Option<u32> {
        self.nodes
            .borrow()
            .iter()
            .find(|seen| seen.name == name)
            .map(|seen| seen.id)
    }

    /// The daemon's id of the port on `node` that is an output (`output`) or an input.
    fn port(&self, node: u32, output: bool) -> Option<u32> {
        self.ports
            .borrow()
            .iter()
            .find(|port| port.node == node && port.output == output)
            .map(|port| port.id)
    }

    /// Links the output of node `from` to the input of node `to`. The private daemon runs no session manager to
    /// make the link a user session would make, so the test makes it.
    pub fn link(&self, from: u32, to: u32) -> Link {
        let linked = drive(&self.mainloop, || {
            self.port(from, true).is_some() && self.port(to, false).is_some()
        });
        assert!(
            linked,
            "the ports of nodes {from} and {to} never reached the observer; it saw {}",
            self.describe()
        );
        let output = self.port(from, true).expect("the producer has an output port");
        let input = self.port(to, false).expect("the consumer has an input port");
        self.core
            .create_object::<Link>(
                "link-factory",
                &properties! {
                    "link.output.port" => output.to_string(),
                    "link.input.port" => input.to_string()
                },
            )
            .expect("link the producer to the consumer")
    }

    /// Listens for a short while, so a node that was going to appear has had time to.
    pub fn settle(&self) {
        let started = Instant::now();
        while started.elapsed() < SETTLE {
            self.mainloop
                .loop_()
                .iterate(Timeout::Finite(Duration::from_millis(10)));
        }
    }

    /// Waits until the registry holds node `id`, or panics listing every node the observer saw.
    pub fn wait_for(&self, id: u32) {
        let seen = drive(&self.mainloop, || self.nodes.borrow().iter().any(|seen| seen.id == id));
        assert!(
            seen,
            "node {id} never reached the observer within {DEADLINE:?}; it saw {}",
            self.describe()
        );
    }

    /// Every node the observer holds, one per line, for a failing test's message.
    pub fn describe(&self) -> String {
        let nodes = self.nodes.borrow();
        if nodes.is_empty() {
            return "no nodes at all".to_owned();
        }
        let lines: Vec<String> = nodes
            .iter()
            .map(|seen| {
                format!(
                    "id {} node.name {:?} media.class {:?} node.description {:?}",
                    seen.id, seen.name, seen.class, seen.description
                )
            })
            .collect();
        format!("{} nodes:\n  {}", lines.len(), lines.join("\n  "))
    }

    /// Waits until the registry holds no node named `name`.
    pub fn wait_gone(&self, name: &str) -> bool {
        drive(&self.mainloop, || self.node(name).is_none())
    }
}

fn connect_client(mainloop: &MainLoopRc, socket: &Path) -> CoreRc {
    let context = ContextRc::new(mainloop, None).expect("create the client context");
    let stream = UnixStream::connect(socket).expect("connect to the daemon socket");
    context
        .connect_fd_rc(OwnedFd::from(stream), None)
        .expect("connect the client to the daemon")
}

/// Iterates `mainloop` until `done` holds or the deadline passes. Returns whether `done` held.
pub fn drive(mainloop: &MainLoopRc, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while !done() {
        if started.elapsed() >= DEADLINE {
            return false;
        }
        mainloop.loop_().iterate(Timeout::Finite(Duration::from_millis(10)));
    }
    true
}

/// One frame a consumer received: the picture as the stream's own `BGRA` or `BGRx` bytes, at the size the consumer
/// had negotiated when it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedFrame {
    /// The width of the picture, in pixels.
    pub width: u32,
    /// The height of the picture, in pixels.
    pub height: u32,
    /// The bytes the producer filled: one `BGRA` (or `BGRx`) pixel after another, `width` of them to a row.
    pub bytes: Vec<u8>,
}

impl ReceivedFrame {
    /// Whether the bytes are exactly one picture of the size the consumer negotiated.
    pub fn is_whole(&self) -> bool {
        self.width > 0 && self.height > 0 && self.bytes.len() == self.width as usize * self.height as usize * 4
    }

    /// The pixel at `(x, y)` as `[red, green, blue, alpha]`, or `None` outside the picture.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if !self.is_whole() || x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        let slice = self.bytes.get(at..at + 4)?;
        Some([slice[2], slice[1], slice[0], slice[3]])
    }

    /// How many pixels have exactly the colour `[red, green, blue]`, whatever their alpha.
    pub fn count_of(&self, colour: [u8; 3]) -> usize {
        if !self.is_whole() {
            return 0;
        }
        self.bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[2] == colour[0] && pixel[1] == colour[1] && pixel[0] == colour[2])
            .count()
    }

    /// Whether the picture is whole and every pixel is `[red, green, blue]`.
    pub fn is_solid(&self, colour: [u8; 3]) -> bool {
        self.is_whole() && self.count_of(colour) == self.width as usize * self.height as usize
    }
}

/// What the consumer's callbacks record, shared with the thread that reads it.
#[derive(Default)]
struct Received {
    /// The size the producer last negotiated, if it has negotiated one.
    size: Option<(u32, u32)>,
    /// The frames that carried bytes, oldest first.
    frames: Vec<ReceivedFrame>,
    /// How many buffers arrived with no bytes: a frame the producer refused.
    empty: usize,
    /// Every state the consumer's stream went through.
    states: Vec<String>,
}

/// A consumer of one node's frames, linked to it the way a session manager would link it.
///
/// It accepts any size and either `BGRA` or `BGRx`, so a producer that renegotiates a new size is followed
/// rather than refused.
pub struct FrameConsumer {
    // Fields drop in declaration order: the link and the stream go before the client they belong to.
    _link: Link,
    _stream_listener: StreamListener<Arc<Mutex<Received>>>,
    stream: StreamRc,
    received: Arc<Mutex<Received>>,
    /// The consumer's own client of the daemon, which every wait iterates.
    pub observer: Observer,
}

impl FrameConsumer {
    /// Connects a consumer to `node` on the daemon at `socket` and links the two.
    pub fn connect(socket: &Path, node: u32) -> Self {
        let observer = Observer::connect(socket);
        observer.wait_for(node);
        let received = Arc::new(Mutex::new(Received::default()));
        let stream = StreamRc::new(
            observer.core.clone(),
            "gnoblin-capture-test-consumer",
            properties! {
                "media.type" => "Video",
                "media.category" => "Capture"
            },
        )
        .expect("create the consumer stream");
        // The callbacks run on PipeWire's data thread, so what they record is shared through a Mutex. Never hold a
        // guard across a call that iterates the loop: a callback would block on it.
        let listener = stream
            .add_local_listener_with_user_data(Arc::clone(&received))
            .state_changed(|_, received, old, new| {
                lock(received).states.push(format!("{old:?}->{new:?}"));
            })
            .param_changed(|_, received, id, param| {
                if id != ParamType::Format.as_raw() {
                    return;
                }
                let Some(param) = param else {
                    return;
                };
                let mut info = VideoInfoRaw::new();
                if info.parse(param).is_ok() {
                    let size = info.size();
                    lock(received).size = Some((size.width, size.height));
                }
            })
            .process(|stream, received| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let length = data.chunk().size() as usize;
                let mut guard = lock(received);
                if length == 0 {
                    guard.empty += 1;
                    return;
                }
                let Some(bytes) = data.data().and_then(|bytes| bytes.get(..length)).map(<[u8]>::to_vec) else {
                    return;
                };
                let (width, height) = guard.size.unwrap_or_default();
                guard.frames.push(ReceivedFrame { width, height, bytes });
            })
            .register()
            .expect("register the consumer callbacks");
        let format = consumer_format();
        let mut params = [Pod::from_bytes(&format).expect("the format pod is valid")];
        stream
            .connect(
                Direction::Input,
                Some(node),
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .expect("connect the consumer to the producer's node");
        let consumer_node = crate::wait_for_node(&observer.mainloop, &stream).expect("the consumer gets a node");
        let link = observer.link(node, consumer_node);
        Self {
            _link: link,
            _stream_listener: listener,
            stream,
            received,
            observer,
        }
    }

    /// Forgets every frame received so far, so the next wait sees only frames that arrive after this call.
    pub fn forget(&self) {
        let mut received = lock(&self.received);
        received.frames.clear();
        received.empty = 0;
    }

    /// The frames received since the last [`FrameConsumer::forget`] that carried bytes, oldest first.
    pub fn frames(&self) -> Vec<ReceivedFrame> {
        lock(&self.received).frames.clone()
    }

    /// How many buffers arrived empty since the last [`FrameConsumer::forget`]: frames the producer refused.
    pub fn empty_frames(&self) -> usize {
        lock(&self.received).empty
    }

    /// The size the producer negotiated last, if it has negotiated one.
    pub fn size(&self) -> Option<(u32, u32)> {
        lock(&self.received).size
    }

    /// Waits for a frame `accept` takes, and answers it. Panics, saying what `what` was and what did arrive, when
    /// none does within [`DEADLINE`].
    #[track_caller]
    pub fn wait_for_frame(&self, what: &str, accept: impl Fn(&ReceivedFrame) -> bool) -> ReceivedFrame {
        self.wait_for_frame_pumping(what, || {}, accept)
    }

    /// [`FrameConsumer::wait_for_frame`] for a producer that runs on this thread: `pump` runs once per turn of the wait,
    /// which is what lets the producer make its frames.
    #[track_caller]
    pub fn wait_for_frame_pumping(
        &self,
        what: &str,
        mut pump: impl FnMut(),
        accept: impl Fn(&ReceivedFrame) -> bool,
    ) -> ReceivedFrame {
        let mut found = None;
        drive(&self.observer.mainloop, || {
            pump();
            found = lock(&self.received)
                .frames
                .iter()
                .rev()
                .find(|frame| accept(frame))
                .cloned();
            found.is_some()
        });
        if let Some(frame) = found {
            return frame;
        }
        let received = lock(&self.received);
        let seen: Vec<String> = received
            .frames
            .iter()
            .rev()
            .take(3)
            .map(|frame| {
                format!(
                    "{}x{} whole={} first pixel {:?}",
                    frame.width,
                    frame.height,
                    frame.is_whole(),
                    frame.pixel(0, 0)
                )
            })
            .collect();
        panic!(
            "no frame {what} arrived within {DEADLINE:?}: {} frames in all, {} empty, last {seen:?}; negotiated {:?}; \
             consumer states {:?}; producer sees {}",
            received.frames.len(),
            received.empty,
            received.size,
            received.states,
            self.observer.describe()
        );
    }

    /// Iterates the consumer's loop for `how_long`, so what the producer sent in that time has arrived.
    pub fn listen(&self, how_long: Duration) {
        self.listen_pumping(how_long, || {});
    }

    /// [`FrameConsumer::listen`] for a producer that runs on this thread: `pump` runs once per turn.
    pub fn listen_pumping(&self, how_long: Duration, mut pump: impl FnMut()) {
        let started = Instant::now();
        while started.elapsed() < how_long {
            pump();
            self.observer
                .mainloop
                .loop_()
                .iterate(Timeout::Finite(Duration::from_millis(10)));
        }
    }

    /// The consumer stream's state, for a failing test's message.
    pub fn state(&self) -> String {
        format!("{:?}", self.stream.state())
    }
}

fn lock(received: &Arc<Mutex<Received>>) -> std::sync::MutexGuard<'_, Received> {
    received.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The format a consumer accepts: `BGRA` or `BGRx`, any size, any rate.
fn consumer_format() -> Vec<u8> {
    let object = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle { width: 64, height: 64 },
            Rectangle { width: 1, height: 1 },
            Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 30, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction { num: 1000, denom: 1 }
        ),
    );
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .map(|(cursor, _)| cursor.into_inner())
        .expect("the consumer's format pod serialises")
}
