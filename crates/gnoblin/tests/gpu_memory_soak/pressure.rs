//! The client that leans on the memory budget in a soak: it asks for one large shared-memory buffer, as a client that
//! is about to draw a huge window does, and finds out whether the compositor accepted it (design/gpu-memory.md,
//! section 6).
//!
//! A buffer the budget cannot hold ends the connection with `wl_display.error.no_memory`, so the answer is the connection
//! itself: still up after a round trip, or ended with that error. Nothing is drawn into the pool, which is sparse: what
//! is measured is the compositor's own accounting, not the test machine's memory.

use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::path::Path;

use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, DispatchError, QueueHandle};

/// `wl_display.error.no_memory`.
const NO_MEMORY: u32 = 2;

/// What the compositor did with the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The connection is up after the buffer was created: it was admitted.
    Accepted,
    /// The connection was ended with `wl_display.error.no_memory`.
    Refused,
}

/// What the client hears; none of it matters here.
struct Quiet;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Quiet {
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

impl Dispatch<wl_shm::WlShm, ()> for Quiet {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for Quiet {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for Quiet {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Connects, asks for a buffer of `width` by `height` pixels and holds it for as long as the returned guard lives.
///
/// The guard keeps the connection, so the buffer counts against the budget until it is dropped.
pub fn lean(socket: &Path, width: i32, height: i32) -> (Answer, Option<Connection>) {
    let stream = UnixStream::connect(socket).expect("the session is listening");
    let connection = Connection::from_socket(stream).expect("a client connection");
    let (globals, mut queue) = registry_queue_init::<Quiet>(&connection).expect("the global list");
    let qh = queue.handle();
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_shm");
    let stride = width * 4;
    let length = stride * height;
    let file = tempfile::tempfile().expect("a shared memory file");
    file.set_len(u64::try_from(length).expect("a positive length"))
        .expect("a sparse file");
    let pool = shm.create_pool(file.as_fd(), length, &qh, ());
    let _buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &qh, ());
    // The server answers on its next loop turn, so a refusal can arrive after the first round trip.
    let mut state = Quiet;
    for _ in 0..3 {
        match queue.roundtrip(&mut state) {
            Ok(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(DispatchError::Backend(WaylandError::Protocol(error))) if error.code == NO_MEMORY => {
                return (Answer::Refused, None);
            }
            Err(other) => panic!("the pressure client failed for another reason: {other}"),
        }
    }
    (Answer::Accepted, Some(connection))
}
