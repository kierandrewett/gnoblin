//! Failure-mode tests for the PipeWire export. Each test runs a private `pipewire` daemon. A
//! second client of that daemon lists the nodes it holds, as `pw-dump` would, so a node the
//! export should not have created, or should have removed, is seen exactly as a user would see it.
//!
//! Every test grants or refuses through [`Table::open`], the same call the portal consent flow
//! makes, never by reaching into the table.

use std::cell::RefCell;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_capture::{Copied, Format, PixelFormat, SessionId, Stop, Table, Target};
use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_types::{ClientId, OutputId, Rect, Size, Tick};
use pipewire::context::ContextRc;
use pipewire::core::CoreRc;
use pipewire::link::Link;
use pipewire::loop_::Timeout;
use pipewire::main_loop::MainLoopRc;
use pipewire::properties::properties;
use pipewire::registry::{Listener, RegistryRc};
use pipewire::spa::pod::Pod;
use pipewire::spa::utils::Direction;
use pipewire::stream::{StreamFlags, StreamRc};
use pipewire::types::ObjectType;

use super::{ExportError, PipewireExport, PlaneSource};

const SIZE: Size = Size::new(64, 48);
/// How long a registry change may take to reach the observer before a test gives up.
const DEADLINE: Duration = Duration::from_secs(20);
/// How long the observer listens before it trusts that no node is coming.
const SETTLE: Duration = Duration::from_millis(400);
/// The pixel the headless window paints, as the BGRx bytes of one stream pixel.
const WINDOW_PIXEL: [u8; 4] = [0x40, 0x80, 0xc0, 0x00];

static NEXT_DAEMON: AtomicUsize = AtomicUsize::new(0);
static INIT: Once = Once::new();

/// A private `pipewire` daemon. Dropping it kills the daemon and removes its runtime directory.
struct Daemon {
    child: Child,
    runtime_dir: PathBuf,
    log: PathBuf,
}

impl Daemon {
    fn start() -> Self {
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

/// A second client of the daemon. It records every node the daemon holds and forgets a node
/// when the daemon removes it.
struct Observer {
    mainloop: MainLoopRc,
    core: CoreRc,
    _registry: RegistryRc,
    _listener: Listener,
    /// Every node currently in the registry.
    nodes: Rc<RefCell<Vec<ObservedNode>>>,
    /// Every port currently in the registry.
    ports: Rc<RefCell<Vec<SeenPort>>>,
}

impl Observer {
    fn connect(socket: &Path) -> Self {
        INIT.call_once(pipewire::init);
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
    fn node(&self, name: &str) -> Option<u32> {
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

    /// Links the output of node `from` to the input of node `to`. The private daemon runs no
    /// session manager to make the link a user session would make, so the test makes it.
    fn link(&self, from: u32, to: u32) -> Link {
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

    /// Listens for `SETTLE`, so a node that was going to appear has had time to.
    fn settle(&self) {
        let started = Instant::now();
        while started.elapsed() < SETTLE {
            self.mainloop
                .loop_()
                .iterate(Timeout::Finite(Duration::from_millis(10)));
        }
    }

    /// Waits until the registry holds node `id`, or panics listing every node the observer saw.
    fn wait_for(&self, id: u32) {
        let seen = drive(&self.mainloop, || self.nodes.borrow().iter().any(|seen| seen.id == id));
        assert!(
            seen,
            "node {id} never reached the observer within {DEADLINE:?}; it saw {}",
            self.describe()
        );
    }

    /// Every node the observer holds, one per line, for a failing test's message.
    fn describe(&self) -> String {
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
    fn wait_gone(&self, name: &str) -> bool {
        drive(&self.mainloop, || self.node(name).is_none())
    }
}

/// A source with nothing to copy. The lifecycle tests never reach a frame.
struct Nothing;

impl PlaneSource for Nothing {
    fn bounds(&self, _target: Target) -> Option<Rect> {
        Some(Rect::from_size(SIZE))
    }

    fn copy_into(&mut self, _target: Target, _region: Rect, _format: Format, _plane: &mut [u8]) -> Copied {
        Copied::Nothing
    }
}

/// A source that paints every pixel the window colour, standing in for a rendered output.
struct SolidSource;

impl PlaneSource for SolidSource {
    fn bounds(&self, _target: Target) -> Option<Rect> {
        Some(Rect::from_size(SIZE))
    }

    fn copy_into(&mut self, _target: Target, _region: Rect, _format: Format, plane: &mut [u8]) -> Copied {
        for pixel in plane.as_chunks_mut::<4>().0 {
            *pixel = WINDOW_PIXEL;
        }
        Copied::Copied
    }
}

fn format() -> Format {
    Format::new(SIZE, SIZE.width * 4, PixelFormat::Xrgb8888).expect("a valid format")
}

fn output() -> Target {
    Target::Output(OutputId::new(1))
}

fn owner() -> ClientId {
    ClientId::new(1)
}

fn intruder() -> ClientId {
    ClientId::new(2)
}

/// The `node.name` the export gives the stream of `session`.
fn stream_name(session: SessionId) -> String {
    format!("gnoblin-capture-{}", session.get())
}

fn connect_client(mainloop: &MainLoopRc, socket: &Path) -> CoreRc {
    let context = ContextRc::new(mainloop, None).expect("create the client context");
    let stream = UnixStream::connect(socket).expect("connect to the daemon socket");
    context
        .connect_fd_rc(OwnedFd::from(stream), None)
        .expect("connect the client to the daemon")
}

/// Iterates `mainloop` until `done` holds or the deadline passes. Returns whether `done` held.
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
fn ungranted_session_is_refused_and_no_node_appears() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Requested, format());
    let mut export = PipewireExport::with_socket(daemon.socket());

    assert_eq!(export.start(&table, session, owner()), Err(ExportError::NotGranted));
    assert_eq!(export.node_id(&table, session, owner()), Err(ExportError::NotExported));
    assert!(
        export.connection.is_none(),
        "a refused session must not connect to PipeWire"
    );
    observer.settle();
    assert_eq!(
        observer.node(&stream_name(session)),
        None,
        "a refused session must not create a node"
    );
}

#[test]
fn granted_session_gets_the_node_the_observer_sees() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());

    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    assert_eq!(export.node_id(&table, session, owner()), Ok(node));
    observer.wait_for(node);
}

#[test]
fn unknown_and_closed_sessions_are_refused() {
    let daemon = Daemon::start();
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    assert!(table.close(session).is_some());
    let mut export = PipewireExport::with_socket(daemon.socket());

    assert_eq!(export.start(&table, session, owner()), Err(ExportError::Unknown));
    assert_eq!(
        export.start(&table, SessionId::new(42), owner()),
        Err(ExportError::Unknown)
    );
    assert!(
        export.connection.is_none(),
        "a refused session must not connect to PipeWire"
    );
}

#[test]
fn stopped_session_is_refused() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    assert!(table.stop(session, Stop::TargetGone));
    let mut export = PipewireExport::with_socket(daemon.socket());

    assert_eq!(export.start(&table, session, owner()), Err(ExportError::Stopped));
    assert!(
        export.connection.is_none(),
        "a stopped session must not connect to PipeWire"
    );
    observer.settle();
    assert_eq!(
        observer.node(&stream_name(session)),
        None,
        "a stopped session must not create a node"
    );
}

#[test]
fn only_the_owner_reads_the_node_id_and_starts_the_stream() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("the owner starts the stream");
    observer.wait_for(node);

    assert_eq!(export.node_id(&table, session, intruder()), Err(ExportError::NotOwner));
    assert_eq!(export.start(&table, session, intruder()), Err(ExportError::NotOwner));
    assert_eq!(export.node_id(&table, session, owner()), Ok(node));
}

#[test]
fn a_second_caller_gets_its_own_node_and_never_another_callers() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let first = table.open(owner(), output(), CaptureStream::Granted, format());
    let second = table.open(intruder(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let first_node = export
        .start(&table, first, owner())
        .expect("the owner starts its stream");

    assert_eq!(export.node_id(&table, first, intruder()), Err(ExportError::NotOwner));
    let second_node = export
        .start(&table, second, intruder())
        .expect("the second caller starts its own stream");
    assert_ne!(first_node, second_node, "each caller's stream is a separate node");
    assert_eq!(export.node_id(&table, second, intruder()), Ok(second_node));
    assert_eq!(export.node_id(&table, second, owner()), Err(ExportError::NotOwner));
    assert_eq!(export.node_id(&table, first, owner()), Ok(first_node));
    observer.wait_for(second_node);
}

#[test]
fn owner_disconnect_tears_the_stream_down() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("the owner starts the stream");
    observer.wait_for(node);

    assert_eq!(table.close_owner(owner()), 1);
    export.pump(&mut table, Tick::new(1), Capture::Desktop, &mut Nothing);

    assert!(!export.is_live(), "the stream ends with its owner");
    assert!(export.connection.is_none(), "no connection outlives the last stream");
    assert!(
        observer.wait_gone(&stream_name(session)),
        "the node leaves the registry"
    );
    assert_eq!(export.node_id(&table, session, owner()), Err(ExportError::Unknown));
}

/// The table has no revoke call: a grant is fixed when the session opens. Withdrawing consent
/// therefore reaches the export the only way the table allows, by stopping the session (here
/// the lock embargo), and the export must end the stream on that stop.
#[test]
fn revoked_grant_tears_the_stream_down() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("the owner starts the stream");
    observer.wait_for(node);

    assert!(table.stop(session, Stop::Embargoed));
    export.pump(&mut table, Tick::new(1), Capture::Desktop, &mut Nothing);

    assert!(!export.is_live(), "a stopped session's stream ends");
    assert!(
        observer.wait_gone(&stream_name(session)),
        "the node leaves the registry"
    );
    assert_eq!(
        export.start(&table, session, owner()),
        Err(ExportError::Stopped),
        "a stopped session cannot be restarted"
    );
}

/// A consumer of the export's node receives frames of the window colour, not empty or stale buffers.
#[test]
fn consumer_receives_frames_of_the_window_colour() {
    let daemon = Daemon::start();
    let observer = Observer::connect(&daemon.socket());
    let mut table = Table::new();
    let session = table.open(owner(), output(), CaptureStream::Granted, format());
    let mut export = PipewireExport::with_socket(daemon.socket());
    let node = export
        .start(&table, session, owner())
        .expect("a granted session starts");
    observer.wait_for(node);

    let received = Arc::new(Mutex::new(Vec::new()));
    let consumer = StreamRc::new(
        observer.core.clone(),
        "gnoblin-capture-test-consumer",
        properties! {
            "media.type" => "Video",
            "media.category" => "Capture"
        },
    )
    .expect("create the consumer stream");
    let states = Arc::new(Mutex::new(Vec::new()));
    let _states_listener = consumer
        .add_local_listener_with_user_data(Arc::clone(&states))
        .state_changed(|_, states, old, new| {
            states
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{old:?}->{new:?}"));
        })
        .register()
        .expect("register the consumer state callback");
    // The callbacks run on PipeWire's data thread, so the frame and state logs are shared through a
    // Mutex. Never hold a guard across a call that pumps the loop: the callback would block on it.
    let _listener = consumer
        .add_local_listener_with_user_data(Arc::clone(&received))
        .process(|stream, received| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.chunk().size() as usize;
            let matches = data.data().is_some_and(|bytes| {
                let Some(plane) = bytes.get(..size) else {
                    return false;
                };
                !plane.is_empty() && plane.as_chunks::<4>().0.iter().all(|pixel| *pixel == WINDOW_PIXEL)
            });
            received.lock().unwrap_or_else(PoisonError::into_inner).push(matches);
        })
        .register()
        .expect("register the consumer callbacks");
    let format_bytes = super::format_pod(format()).expect("the format pod serialises");
    let mut params = [Pod::from_bytes(&format_bytes).expect("the format pod is valid")];
    consumer
        .connect(
            Direction::Input,
            Some(node),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .expect("connect the consumer to the export's node");
    // The private daemon runs no session manager, so nothing links the streams. The test links them.
    let consumer_node = super::wait_for_node(&observer.mainloop, &consumer).expect("the consumer gets a node");
    let _link = observer.link(node, consumer_node);

    let mut tick = 0;
    let delivered = drive(&observer.mainloop, || {
        tick += 1;
        export.pump(&mut table, Tick::new(tick), Capture::Desktop, &mut SolidSource);
        received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|matches| **matches)
            .count()
            >= 3
    });
    let frames = received.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let producer: Vec<_> = export.exports.values().map(|export| export.stream.state()).collect();
    assert!(
        delivered,
        "three frames of the window colour arrive within the deadline, got {frames:?}; consumer states {:?}; producer {producer:?}",
        states.lock().unwrap_or_else(PoisonError::into_inner).clone()
    );
    assert!(
        frames.iter().all(|matches| *matches),
        "every frame is the window colour, got {frames:?}"
    );
}
