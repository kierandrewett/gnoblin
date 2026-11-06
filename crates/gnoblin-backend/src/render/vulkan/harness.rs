//! A real client's `wl_shm` buffers, in process, for the tests that draw a client surface.
//!
//! A surface layer holds a `wl_buffer` that only a Wayland server can hold, so the tests start
//! one: a `wayland-server` display with Smithay's `ShmState`, and a `wayland-client` connection
//! to it over a socket pair. The client lives on a worker thread because binding a global and
//! creating a pool are round trips that the test's own thread answers by pumping the display.
//! Both renderers then read the same buffers through the same Smithay mapping a session uses.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::{Client, Display, Resource};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::shm::{ShmHandler, ShmState};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

/// One buffer the client creates, with the bytes in its pool.
pub(super) struct BufferSpec {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub format: wl_shm::Format,
    /// The pool's bytes, `stride * height` of them, in the buffer's own byte order.
    pub bytes: Vec<u8>,
}

impl BufferSpec {
    /// A packed `argb8888` buffer from straight-order `[r, g, b, a]` pixels, rows top first.
    ///
    /// The pixels are stored as a client stores them on a little-endian machine, blue first.
    pub(super) fn argb(width: i32, height: i32, pixel: impl Fn(i32, i32) -> [u8; 4]) -> Self {
        Self::packed(width, height, wl_shm::Format::Argb8888, pixel)
    }

    /// The same for `xrgb8888`, whose fourth byte is padding.
    pub(super) fn xrgb(width: i32, height: i32, pixel: impl Fn(i32, i32) -> [u8; 4]) -> Self {
        Self::packed(width, height, wl_shm::Format::Xrgb8888, pixel)
    }

    fn packed(width: i32, height: i32, format: wl_shm::Format, pixel: impl Fn(i32, i32) -> [u8; 4]) -> Self {
        let stride = width * 4;
        let mut bytes = Vec::with_capacity((stride * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let [red, green, blue, alpha] = pixel(x, y);
                bytes.extend_from_slice(&[blue, green, red, alpha]);
            }
        }
        Self {
            width,
            height,
            stride,
            format,
            bytes,
        }
    }
}

struct ServerState {
    shm: ShmState,
}

impl BufferHandler for ServerState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for ServerState {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

smithay::delegate_dispatch2!(ServerState);

struct Permissive;

impl ClientData for Permissive {}

enum Command {
    Destroy(usize),
    Finish,
}

/// The server, the client and the buffers the client made.
pub(super) struct ShmClient {
    display: Display<ServerState>,
    state: ServerState,
    buffers: Vec<WlBuffer>,
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl ShmClient {
    /// Starts a server and a client that creates one buffer per spec.
    pub(super) fn new(specs: Vec<BufferSpec>) -> Self {
        let display = Display::<ServerState>::new().expect("a wayland display");
        let state = ServerState {
            shm: ShmState::new::<ServerState>(&display.handle(), Vec::new()),
        };
        let (server_stream, client_stream) = UnixStream::pair().expect("a socket pair");
        let client: Client = display
            .handle()
            .insert_client(server_stream, Arc::new(Permissive))
            .expect("the client is accepted");
        let (ids_sender, ids) = mpsc::channel();
        let (commands, command_receiver) = mpsc::channel();
        let worker = thread::spawn(move || run_client(client_stream, &specs, &ids_sender, &command_receiver));
        let mut this = Self {
            display,
            state,
            buffers: Vec::new(),
            commands,
            worker: Some(worker),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        let protocol_ids: Vec<u32> = loop {
            this.pump();
            match ids.try_recv() {
                Ok(found) => break found,
                Err(TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "the client never created its buffers");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(TryRecvError::Disconnected) => panic!("the client thread ended before it made its buffers"),
            }
        };
        let handle = this.display.handle();
        this.buffers = protocol_ids
            .into_iter()
            .map(|id| {
                client
                    .object_from_protocol_id::<WlBuffer>(&handle, id)
                    .expect("the server knows the client's buffer")
            })
            .collect();
        this
    }

    /// The server's handle to the `index`th buffer.
    pub(super) fn buffer(&self, index: usize) -> WlBuffer {
        self.buffers[index].clone()
    }

    /// Serves whatever the client has sent.
    fn pump(&mut self) {
        self.display.dispatch_clients(&mut self.state).expect("a dispatch");
        self.display.flush_clients().expect("a flush");
    }

    /// Has the client destroy a buffer and its pool, and waits until the server has seen it.
    pub(super) fn destroy(&mut self, index: usize) {
        self.commands
            .send(Command::Destroy(index))
            .expect("the client is listening");
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.buffers[index].is_alive() {
            self.pump();
            assert!(Instant::now() < deadline, "the client never destroyed its buffer");
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for ShmClient {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Finish);
        if let Some(worker) = self.worker.take() {
            // The client's last round trip needs the server to answer, so keep pumping.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !worker.is_finished() && Instant::now() < deadline {
                let _ = self.display.dispatch_clients(&mut self.state);
                let _ = self.display.flush_clients();
                thread::sleep(Duration::from_millis(1));
            }
            let _ = worker.join();
        }
    }
}

struct App;

wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The client: binds `wl_shm`, makes a pool and a buffer per spec, reports the buffers' ids and
/// then obeys commands until told to finish.
fn run_client(stream: UnixStream, specs: &[BufferSpec], ids: &Sender<Vec<u32>>, commands: &Receiver<Command>) {
    let connection = Connection::from_socket(stream).expect("a client connection");
    let (globals, mut queue) = registry_queue_init::<App>(&connection).expect("the global list");
    let qh = queue.handle();
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm is advertised");
    let mut app = App;
    let mut live = Vec::new();
    for spec in specs {
        let mut file = tempfile::tempfile().expect("a shared memory file");
        file.write_all(&spec.bytes).expect("the pool's bytes");
        let pool = shm.create_pool(file.as_fd(), spec.bytes.len() as i32, &qh, ());
        let buffer = pool.create_buffer(0, spec.width, spec.height, spec.stride, spec.format, &qh, ());
        live.push((buffer, pool, file));
    }
    queue.roundtrip(&mut app).expect("the buffers are created");
    let protocol_ids = live.iter().map(|(buffer, _, _)| buffer.id().protocol_id()).collect();
    if ids.send(protocol_ids).is_err() {
        return;
    }
    while let Ok(Command::Destroy(index)) = commands.recv() {
        let (buffer, pool, _file) = &live[index];
        buffer.destroy();
        pool.destroy();
        let _ = queue.roundtrip(&mut app);
    }
}
