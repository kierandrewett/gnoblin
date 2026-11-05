//! A session on a socket of its own, and a real client, for the Gnoblin-owned
//! protocols.
//!
//! The three test files (`window_frame.rs`, `blur_fade.rs` and `session_host.rs`)
//! share this harness rather than the `session` one, because these protocols need
//! two things the general harness does not offer: a connection whose role the
//! compositor promised *before* it connected, and a client with no globals bound
//! except the ones a test names.
//!
//! The two threads are the same arrangement as the other protocol tests: the
//! session runs on the test's thread and the client choreography on a worker,
//! which is also how a real session and a real client are related.

#![allow(dead_code)]

use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessHandle};
use gnoblin_backend::render::Image;
use gnoblin_backend::{Backend, BackendError, BackendEvent, Host, InputSource, Output, Present, Target};
use gnoblin_types::OutputId;
use gnoblin_wayland::{ClientRole, Config, ControlHandle, LoopError, Server};
use smithay::reexports::calloop::LoopHandle;
use wayland_client::DispatchError;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

/// How long a test waits for a state before failing.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The protocol error a client's connection ended with, if it ended with one.
pub struct ProtocolFailure {
    /// The code, which is the interface's own error enum value.
    pub code: u32,
    /// The interface the error was posted on.
    pub interface: String,
}

/// What a worker thread can reach of a running session.
#[derive(Clone)]
pub struct Handle {
    socket: PathBuf,
    pub control: ControlHandle,
    pub backend: HeadlessHandle,
}

impl Handle {
    /// The socket a client connects to.
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

/// A headless backend that reports no gamma or power control.
///
/// Its outputs are the headless ones; only the controls are missing, which is the case
/// a backend that cannot change its outputs presents. The protocols that need the
/// controls must not be advertised.
pub struct WithoutOutputControls(pub Headless);

impl<S: Host + 'static> Backend<S> for WithoutOutputControls {
    fn name(&self) -> &'static str {
        <Headless as Backend<S>>::name(&self.0)
    }

    fn outputs(&self) -> &[Output] {
        <Headless as Backend<S>>::outputs(&self.0)
    }

    fn output(&self, id: OutputId) -> Option<&Output> {
        <Headless as Backend<S>>::output(&self.0, id)
    }

    fn target(&mut self, id: OutputId) -> Option<&mut dyn Target> {
        <Headless as Backend<S>>::target(&mut self.0, id)
    }

    fn input(&mut self) -> &mut dyn InputSource {
        <Headless as Backend<S>>::input(&mut self.0)
    }

    fn register(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        <Headless as Backend<S>>::register(&mut self.0, loop_handle)
    }

    fn render(&mut self, host: &mut S, output: OutputId) -> Result<Present, BackendError> {
        <Headless as Backend<S>>::render(&mut self.0, host, output)
    }

    fn screenshot(&mut self, output: OutputId) -> Option<Image> {
        <Headless as Backend<S>>::screenshot(&mut self.0, output)
    }

    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent> {
        <Headless as Backend<S>>::dispatch(&mut self.0, host)
    }
}

/// Stops the session when the worker finishes, including when it panics.
pub struct StopOnDrop(pub ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A session that has been created but is not running yet.
pub struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink the socket.
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    server: Server,
    handle: Handle,
}

impl Session {
    /// A session that has promised its own process a role.
    ///
    /// A promise is keyed by process and the test binary is one process, so a client
    /// the test connects is recorded with the role. That is the same seam the
    /// composition root uses: promise, then start the renderer or the session.
    pub fn start(role: Option<ClientRole>) -> Self {
        Self::boot(role, true)
    }

    /// A session whose backend reports no gamma or power control, so neither protocol
    /// is advertised.
    pub fn start_without_output_controls() -> Self {
        Self::boot(None, false)
    }

    /// Starts the session; `controls` says whether the headless backend keeps its controls.
    fn boot(role: Option<ClientRole>, controls: bool) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            ..Config::default()
        };
        let headless = Headless::new();
        let outputs = headless.handle();
        let started = if controls {
            Server::new(Box::new(headless), config)
        } else {
            Server::new(Box::new(WithoutOutputControls(headless)), config)
        };
        let mut server = started.expect("the session starts");
        if let Some(role) = role {
            server.promise_role(std::process::id(), role);
        }
        let handle = Handle {
            socket,
            control: server.control(),
            backend: outputs,
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the session on this thread and `body` on a worker, then joins both.
    pub fn run(self, body: impl FnOnce(&Handle) + Send + 'static) {
        let handle = self.handle.clone();
        let worker = thread::spawn({
            let handle = handle.clone();
            move || {
                let _stop = StopOnDrop(handle.control.clone());
                body(&handle);
            }
        });
        let outcome: Result<(), LoopError> = self.server.run();
        if let Err(payload) = worker.join() {
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// One real Wayland client, with the core globals bound.
pub struct Connected<S>
where
    S: Dispatch<wl_registry::WlRegistry, GlobalListContents>
        + Dispatch<wl_compositor::WlCompositor, ()>
        + Dispatch<wl_shm::WlShm, ()>
        + Dispatch<wl_shm_pool::WlShmPool, ()>
        + Dispatch<wl_buffer::WlBuffer, ()>
        + 'static,
{
    /// The client's connection, which closes when it is dropped.
    pub connection: Connection,
    pub globals: GlobalList,
    pub queue: EventQueue<S>,
    pub qh: QueueHandle<S>,
    pub state: S,
    pub compositor: wl_compositor::WlCompositor,
    pub shm: wl_shm::WlShm,
}

impl<S> Connected<S>
where
    S: Dispatch<wl_registry::WlRegistry, GlobalListContents>
        + Dispatch<wl_compositor::WlCompositor, ()>
        + Dispatch<wl_shm::WlShm, ()>
        + Dispatch<wl_shm_pool::WlShmPool, ()>
        + Dispatch<wl_buffer::WlBuffer, ()>
        + 'static,
{
    /// Connects to a socket and binds `wl_compositor` and `wl_shm`.
    pub fn connect(socket: &Path, state: S) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        stream
            .set_read_timeout(Some(TIMEOUT))
            .expect("a read timeout on the client socket");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<S>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor = globals
            .bind(&qh, 1..=6, ())
            .expect("every session advertises wl_compositor");
        let shm = globals.bind(&qh, 1..=1, ()).expect("every session advertises wl_shm");
        Self {
            connection,
            globals,
            queue,
            qh,
            state,
            compositor,
            shm,
        }
    }

    /// Processes every event the server has sent.
    pub fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// Processes whatever has arrived, then waits for the predicate to hold.
    pub fn wait_until(&mut self, what: &str, predicate: impl Fn(&S) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.roundtrip();
            if predicate(&self.state) {
                return;
            }
            assert!(Instant::now() < deadline, "the client never saw {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The interfaces the session advertised to this connection.
    pub fn interfaces(&self) -> Vec<String> {
        let mut names = Vec::new();
        self.globals.contents().with_list(|list| {
            for global in list {
                names.push(global.interface.clone());
            }
        });
        names
    }

    /// Whether the session advertised a global to this connection.
    pub fn has(&self, interface: &str) -> bool {
        self.interfaces().iter().any(|name| name == interface)
    }

    /// Creates a shared-memory ARGB8888 buffer.
    pub fn buffer(&self, width: i32, height: i32) -> wl_buffer::WlBuffer {
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ())
    }

    /// Round trips until the server ends the connection with a protocol error, and
    /// answers which one.
    pub fn expect_protocol_error(&mut self, interface: &str, code: u32) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.queue.roundtrip(&mut self.state) {
                Ok(_) => {}
                Err(DispatchError::Backend(_)) => {
                    let error = self
                        .connection
                        .protocol_error()
                        .expect("a backend failure that names no protocol error");
                    assert_eq!(
                        (error.object_interface.as_str(), error.code),
                        (interface, code),
                        "the error the server posted"
                    );
                    return;
                }
                Err(other) => panic!("expected a protocol error, got {other:?}"),
            }
            assert!(Instant::now() < deadline, "the server never posted an error");
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// The client-side bindings for the Gnoblin-owned protocols.
///
/// Generated from the same XML the server is, at compile time, so a client here
/// cannot speak a wire the server does not.
pub mod protocols {
    /// `gnoblin_window_frame_v1`.
    pub mod gnoblin_window_frame_v1 {
        use wayland_client;
        use wayland_client::protocol::wl_surface;

        pub mod __interfaces {
            use super::wayland_client::backend as wayland_backend;
            use super::wayland_client::protocol::__interfaces::*;

            wayland_scanner::generate_interfaces!("../../protocols/gnoblin-window-frame-v1.xml");
        }

        #[allow(unused_imports)]
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("../../protocols/gnoblin-window-frame-v1.xml");
    }

    /// `gnoblin_blur_fade_v1`.
    pub mod gnoblin_blur_fade_v1 {
        use wayland_client;
        use wayland_client::protocol::wl_surface;

        pub mod __interfaces {
            use super::wayland_client::backend as wayland_backend;
            use super::wayland_client::protocol::__interfaces::*;

            wayland_scanner::generate_interfaces!("../../protocols/gnoblin-blur-fade-v1.xml");
        }

        #[allow(unused_imports)]
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("../../protocols/gnoblin-blur-fade-v1.xml");
    }

    /// `gnoblin_session_host_v1`.
    pub mod gnoblin_session_host_v1 {
        use wayland_client;

        pub mod __interfaces {
            use super::wayland_client::backend as wayland_backend;

            wayland_scanner::generate_interfaces!("../../protocols/gnoblin-session-host-v1.xml");
        }

        #[allow(unused_imports)]
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("../../protocols/gnoblin-session-host-v1.xml");
    }
}

/// How many distinct colours a screenshot contains, so a test can tell "one
/// window on the background" from "only the background".
pub fn distinct_colours(image: &gnoblin_backend::render::Image) -> usize {
    let mut seen = std::collections::BTreeSet::new();
    let size = image.size();
    let stride = image.stride();
    for y in 0..i32::try_from(size.height).unwrap_or(0) {
        for x in 0..i32::try_from(size.width).unwrap_or(0) {
            let offset = usize::try_from(y).unwrap_or(0) * stride + usize::try_from(x).unwrap_or(0) * 4;
            if let Some(pixel) = image.data().get(offset..offset + 4) {
                seen.insert([pixel[0], pixel[1], pixel[2], pixel[3]]);
            }
        }
    }
    seen.len()
}
