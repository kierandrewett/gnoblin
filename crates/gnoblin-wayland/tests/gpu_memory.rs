//! A hostile client against the real compositor: what it can make the session hold (design/gpu-memory.md).
//!
//! Every test runs the real session on a headless backend and a real `wayland-client` connection. Two things are
//! proven here and one is measured:
//!
//! - **A buffer larger than the compositor holds is refused at the request.** A sparse pool of a gigabyte is a valid
//!   pool, so nothing but the compositor's own limit stands between a 16384 x 16384 buffer and the renderer: the
//!   request is `wl_shm.invalid_stride` and the client is disconnected (`an_shm_buffer_past_the_limit_is_a_protocol_error`).
//!   The limit is the largest size that is accepted, exactly (`the_largest_buffer_is_accepted_and_one_pixel_more_is_not`).
//!   The dmabuf twin is in `tests/dmabuf.rs`.
//! - **A thousand windows stop at the cap and end with their client.** A client that makes a thousand toplevels gets
//!   exactly as many managed as the session's record allows (256), stays connected and leaves the session with none
//!   of them when it drops, which frees the room for the next client
//!   (`a_thousand_tiny_windows_stop_at_the_cap_and_end_with_their_client`).
//!
//! What it does not prove: the memory a driver holds for a buffer that is accepted (see `tests/gpu_memory_soak.rs` in
//! the `gnoblin` crate for the series), and a real GPU's dmabuf import.

mod session;

use std::thread;
use std::time::{Duration, Instant};

use session::{Client, Handle, Session, TIMEOUT};
use wayland_client::DispatchError;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_shm;

/// Round-trips until the server closes the connection with a protocol error, and returns the error's interface and code.
fn protocol_error(client: &mut Client) -> (String, u32) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match client.queue.roundtrip(&mut client.state) {
            Ok(_) => {}
            Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
                return (error.object_interface.to_string(), error.code);
            }
            Err(other) => panic!("the round trip failed for another reason: {other}"),
        }
        assert!(Instant::now() < deadline, "the server never posted a protocol error");
        thread::sleep(Duration::from_millis(5));
    }
}

/// The resident set of this process, in bytes: the session and the client share it, which is what a bound on the
/// session's growth has to be measured against.
fn resident() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|statm| statm.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4096)
}

#[test]
fn an_shm_buffer_past_the_limit_is_a_protocol_error() {
    let session = Session::start();
    session.run(|handle| {
        let mut client = Client::connect(handle.socket());
        let before = resident();
        // A sparse pool of a gigabyte is a valid pool: the buffer's size is the client's claim.
        let _buffer = client.buffer(16384, 16384);
        let (interface, code) = protocol_error(&mut client);
        assert_eq!(interface, "wl_shm_pool");
        assert_eq!(code, wl_shm::Error::InvalidStride as u32);
        let grown = resident().saturating_sub(before);
        assert!(
            grown < 256 << 20,
            "refusing the buffer must not map or copy it: the process grew by {grown} bytes"
        );
    });
}

#[test]
fn the_largest_buffer_is_accepted_and_one_pixel_more_is_not() {
    let session = Session::start();
    session.run(|handle| {
        // 16384 x 8192 is 2^27 pixels, the most one buffer may hold: it is created and the connection stays up.
        let mut client = Client::connect(handle.socket());
        let _largest = client.buffer(16384, 8192);
        client.roundtrip();
        // One row more passes the pixel limit, and a width of 16385 passes the edge limit.
        for (width, height) in [(16384, 8193), (16385, 1)] {
            let mut client = Client::connect(handle.socket());
            let _too_large = client.buffer(width, height);
            let (_, code) = protocol_error(&mut client);
            assert_eq!(code, wl_shm::Error::InvalidStride as u32, "{width}x{height} is refused");
        }
    });
}

/// The most windows the session manages at once: `MAX_WINDOWS` of `src/fallback.rs`, which is private to the crate. A
/// window past it is left unmanaged, so the number is what bounds the record a client that opens toplevels in a loop
/// can make the compositor keep. The test fails, rather than quietly follows, if the cap is moved.
const WINDOW_CAP: usize = 256;

/// Maps `count` 1x1 windows on `client` in two round trips: every toplevel is created and committed first, then every
/// one is given the same shared buffer, which a client may attach to any number of surfaces.
///
/// One round trip per window is what a toolkit does and what made two hundred windows take a minute in a debug build:
/// each commit presents a frame of everything mapped so far. Here the requests queue up and the session handles them
/// as fast as it reads them, so a thousand windows are cheap enough for every run. The test client acknowledges every
/// configure as it arrives, which is what lets the second round attach a buffer.
fn map_tiny_windows(client: &mut Client, count: usize) -> Vec<session::Window> {
    let windows: Vec<session::Window> = (0..count)
        .map(|index| {
            let surface = client.compositor.create_surface(&client.qh, ());
            let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
            let toplevel = xdg_surface.get_toplevel(&client.qh, ());
            toplevel.set_app_id(format!("org.example.tiny{index}"));
            toplevel.set_title("tiny".to_owned());
            surface.commit();
            session::Window {
                surface,
                xdg_surface,
                toplevel,
            }
        })
        .collect();
    client.roundtrip();
    let buffer = client.buffer(1, 1);
    for window in &windows {
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage(0, 0, 1, 1);
        window.surface.commit();
    }
    client.roundtrip();
    windows
}

/// Waits up to ten seconds for the session to hold a number of windows `done` accepts.
fn wait_len(handle: &Handle, done: impl Fn(usize) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done(handle.windows().len()) {
        assert!(
            Instant::now() < deadline,
            "the session holds {} windows",
            handle.windows().len()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_thousand_tiny_windows_stop_at_the_cap_and_end_with_their_client() {
    const COUNT: usize = 1000;
    let session = Session::start();
    session.run(|handle| {
        let before = resident();
        let started = Instant::now();
        let mut hostile = Client::connect(handle.socket());
        let windows = map_tiny_windows(&mut hostile, COUNT);
        assert_eq!(windows.len(), COUNT, "the client made every window it asked for");

        // The session manages as many as its cap and no more, however many the client keeps making: the rest are left
        // unmanaged (not in the record, not placed, not listed) while the client stays connected and answered.
        wait_len(handle, |len| len == WINDOW_CAP);
        hostile.roundtrip();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(
            handle.windows().len(),
            WINDOW_CAP,
            "{COUNT} toplevels from one client must leave exactly the cap managed"
        );
        let held = resident().saturating_sub(before);
        println!(
            "{COUNT} tiny windows ({WINDOW_CAP} managed) in {:?}: the process grew by {held} bytes",
            started.elapsed()
        );
        assert!(
            held < 256 << 20,
            "a thousand 1x1 windows grew the process by {held} bytes"
        );

        // The session still answers someone else while it holds them.
        let mut other = Client::connect(handle.socket());
        other.roundtrip();

        // The connection goes, and every window goes with it.
        drop(windows);
        drop(hostile);
        wait_len(handle, |len| len == 0);
        other.roundtrip();
        let left = resident().saturating_sub(before);
        println!("after the client left: the process holds {left} bytes more than before");
        assert!(
            left < 256 << 20,
            "a thousand 1x1 windows must not leave a quarter of a gigabyte behind: {left} bytes"
        );

        // Room again: a window of a new client is managed once the hostile one is gone.
        let mut polite = Client::connect(handle.socket());
        let _window = polite.map_window("org.example.polite", "polite");
        wait_len(handle, |len| len == 1);
    });
}
