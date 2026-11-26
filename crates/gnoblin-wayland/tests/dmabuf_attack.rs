//! A client that sends GPU buffers no driver can map must not take the compositor down (bead gnoblin-992m).
//!
//! The session is the real compositor on the GLES renderer, and again on the Vulkan renderer. A client binds
//! `zwp_linux_dmabuf_v1`, shares a buffer whose plane is no GPU memory, attaches it to a window and commits, and the output
//! is read back while the window shows it. Mesa's llvmpipe on a machine with no `/dev/dri` imports such a plane without
//! complaint and takes the compositor down when the surface pass samples it (the bead saw SIGSEGV on its rasteriser thread
//! on `/dev/null`); the Vulkan renderer copies a linear plane's rows in the compositor's own process, so a file the client
//! resizes faults that copy with SIGBUS.
//!
//! The planes: `/dev/null` and a short file, which the protocol's own size check refuses; a regular file of the buffer's
//! size, one the client empties once the compositor holds the buffer, one a thread keeps resizing while the output is read
//! back, and a pipe, which pass that check (a pipe cannot be seeked, so the check never learns its size) and only the
//! compositor's own look at the descriptor stops.
//!
//! The compositor is right when it does either of two things: serves no `zwp_linux_dmabuf_v1` at all (a renderer with
//! no render node has no device a client could allocate on), or fails the client's buffer with a protocol error. What it
//! may never do is import the buffer and draw it. After every attack a fresh client must still get an answer and the
//! control handle must still read the output back, which is what separates a surviving compositor from one that merely
//! disconnected the attacker.
//!
//! It needs a GLES context, so where the machine has none the test prints why and returns, loudly.

use gnoblin_wayland_testkit::stock;

use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_v1 as dmabuf_global,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const TIMEOUT: Duration = Duration::from_secs(10);
/// `DRM_FORMAT_ARGB8888`.
const ARGB8888: u32 = 0x3432_5241;
/// `DRM_FORMAT_MOD_LINEAR` and `DRM_FORMAT_MOD_INVALID`, the modifiers a client with no GPU knowledge can name.
const MOD_LINEAR: u64 = 0;
const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

/// The `(format, modifier)` pairs the compositor offered a version 3 client.
#[derive(Default)]
struct App {
    offered: Vec<(u32, u64)>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
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

macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for App {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_buffer::WlBuffer,
    xdg_toplevel::XdgToplevel,
    params::ZwpLinuxBufferParamsV1,
);

impl Dispatch<dmabuf_global::ZwpLinuxDmabufV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &dmabuf_global::ZwpLinuxDmabufV1,
        event: dmabuf_global::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let dmabuf_global::Event::Modifier {
            format,
            modifier_hi,
            modifier_lo,
        } = event
        {
            state
                .offered
                .push((format, u64::from(modifier_hi) << 32 | u64::from(modifier_lo)));
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A plane the attack names, and why a driver cannot map it.
struct Attack {
    name: &'static str,
    plane: OwnedFd,
    /// A handle on the same file, which the client empties once the compositor has taken the buffer.
    shrink: Option<File>,
    /// A handle on the same file, which the client resizes in a loop for as long as the compositor draws the buffer.
    flap: Option<File>,
}

fn attacks() -> Vec<Attack> {
    let full = u64::from(WIDTH) * u64::from(HEIGHT) * 4;
    let small = tempfile::tempfile().expect("a temporary file");
    small.set_len(16).expect("a short file");
    let regular = tempfile::tempfile().expect("a temporary file");
    regular.set_len(full).expect("a file of the buffer's size");
    let shrinking = tempfile::tempfile().expect("a temporary file");
    shrinking.set_len(full).expect("a file of the buffer's size");
    let flapping = tempfile::tempfile().expect("a temporary file");
    flapping.set_len(full).expect("a file of the buffer's size");
    let (pipe, writer) = std::io::pipe().expect("a pipe");
    // The writer is kept open by leaking it for the test's life: a closed pipe is a different attack.
    std::mem::forget(writer);
    vec![
        // Both of these the protocol's own size check (Smithay's, on `lseek`) already refuses.
        Attack {
            name: "/dev/null",
            plane: File::open("/dev/null").expect("/dev/null opens").into(),
            shrink: None,
            flap: None,
        },
        Attack {
            name: "a file far smaller than the buffer",
            plane: small.into(),
            shrink: None,
            flap: None,
        },
        // These pass it: a file of the right size, one that is emptied after the buffer is taken, and a descriptor that
        // cannot be seeked, whose size the check never learns.
        Attack {
            name: "a regular file of the buffer's size, which is no GPU memory",
            plane: regular.into(),
            shrink: None,
            flap: None,
        },
        Attack {
            name: "a regular file emptied once the compositor holds the buffer",
            plane: shrinking.try_clone().expect("a second handle").into(),
            shrink: Some(shrinking),
            flap: None,
        },
        Attack {
            name: "a regular file a thread keeps resizing while the compositor draws it",
            plane: flapping.try_clone().expect("a second handle").into(),
            shrink: None,
            flap: Some(flapping),
        },
        Attack {
            name: "a pipe, whose size no check can read",
            plane: pipe.into(),
            shrink: None,
            flap: None,
        },
    ]
}

/// Connects a client and binds what an attack needs, or `None` when the compositor serves no dmabuf.
fn connect(
    socket: &std::path::Path,
) -> (
    Connection,
    GlobalList,
    EventQueue<App>,
    App,
    Option<dmabuf_global::ZwpLinuxDmabufV1>,
) {
    let stream = UnixStream::connect(socket).expect("the session is listening");
    let connection = Connection::from_socket(stream).expect("a client connection");
    let (globals, mut queue) = registry_queue_init::<App>(&connection).expect("the global list");
    let qh = queue.handle();
    let mut app = App::default();
    // Version 3 names formats and no device: it is what a session with no render node serves, and a version 4 global
    // answers a version 3 bind the same way.
    let global = globals
        .bind::<dmabuf_global::ZwpLinuxDmabufV1, _, _>(&qh, 1..=3, ())
        .ok();
    queue.roundtrip(&mut app).expect("the compositor answers a round trip");
    (connection, globals, queue, app, global)
}

/// What the compositor did with an attack.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// It serves no `zwp_linux_dmabuf_v1`, so there was nothing to send the buffer through.
    NoGlobal,
    /// It failed the buffer: the connection died of a protocol error.
    Refused,
    /// The buffer was taken, attached and drawn.
    Accepted,
}

/// Shares `plane` as a buffer, attaches it to a window and commits. The protocol error that follows, if any, is the
/// compositor refusing the buffer.
fn attack(socket: &std::path::Path, attack: &Attack, draw: &dyn Fn()) -> Result<Answer, String> {
    use std::os::fd::AsFd as _;
    let (connection, globals, mut queue, mut app, global) = connect(socket);
    let Some(global) = global else {
        return Ok(Answer::NoGlobal);
    };
    let qh = queue.handle();
    let (format, modifier) = *app
        .offered
        .iter()
        .find(|pair| **pair == (ARGB8888, MOD_LINEAR))
        .or_else(|| app.offered.iter().find(|pair| **pair == (ARGB8888, MOD_INVALID)))
        .or_else(|| app.offered.first())
        .ok_or_else(|| "the compositor offered no format".to_owned())?;
    let params = global.create_params(&qh, ());
    params.add(
        attack.plane.as_fd(),
        0,
        0,
        WIDTH * 4,
        (modifier >> 32) as u32,
        modifier as u32,
    );
    let buffer = params.create_immed(WIDTH as i32, HEIGHT as i32, format, params::Flags::empty(), &qh, ());

    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_title(attack.name.to_owned());
    surface.commit();
    // A roundtrip that fails is the compositor ending the connection over the buffer (a protocol error).
    let mut accepted = queue.roundtrip(&mut app).is_ok();
    if accepted {
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, WIDTH as i32, HEIGHT as i32);
        surface.commit();
        accepted = accepted && queue.roundtrip(&mut app).is_ok();
        if let Some(file) = &attack.shrink {
            file.set_len(0).expect("the client empties its own file");
        }
        // The output is read back while the window shows the buffer, which is when the renderer reads it. A buffer whose
        // file is being resized is read many times, so a read that lands past the new end is likely to happen.
        let stop = Arc::new(AtomicBool::new(false));
        let flapper = attack.flap.as_ref().map(|file| {
            let file = file.try_clone().expect("a handle for the resizing thread");
            let stop = Arc::clone(&stop);
            let full = u64::from(WIDTH) * u64::from(HEIGHT) * 4;
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = file.set_len(0);
                    let _ = file.set_len(full);
                }
            })
        });
        let rounds = if flapper.is_some() { 400 } else { 3 };
        for _ in 0..rounds {
            accepted = accepted && queue.roundtrip(&mut app).is_ok();
            if accepted {
                draw();
            }
            if flapper.is_none() {
                thread::sleep(Duration::from_millis(20));
            }
        }
        stop.store(true, Ordering::Relaxed);
        if let Some(flapper) = flapper {
            flapper.join().expect("the resizing thread ends");
        }
    }
    drop((connection, buffer));
    Ok(if accepted { Answer::Accepted } else { Answer::Refused })
}

#[test]
fn a_buffer_no_driver_can_map_never_reaches_the_gles_driver() {
    never_reaches_the_renderer("GLES", HeadlessConfig::gles());
}

#[test]
fn a_buffer_no_driver_can_map_never_reaches_the_vulkan_renderer() {
    never_reaches_the_renderer("Vulkan", HeadlessConfig::vulkan());
}

fn never_reaches_the_renderer(name: &str, config: HeadlessConfig) {
    let backend = match Headless::with_config(config) {
        Ok(backend) => backend,
        Err(error) => {
            println!("skipping the dmabuf attack test: no {name} renderer on this machine ({error})");
            return;
        }
    };
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = directory.path().join("wayland-0.sock");
    let config = Config {
        socket_path: Some(socket.clone()),
        ..crate::stock::config()
    };
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    let served = server.state().serves_dmabuf();
    let control = server.control();
    let observation = server.observation();
    let report = Arc::new(Mutex::new(Vec::<String>::new()));

    let worker = thread::spawn({
        let control = control.clone();
        let report = Arc::clone(&report);
        move || {
            let _stop = StopOnDrop(control.clone());
            let output = *observation
                .lock()
                .expect("the observation lock")
                .outputs()
                .first()
                .expect("an output");
            let draw = || {
                let _ = control.screenshot(output);
            };
            let mut accepted = Vec::new();
            for case in attacks() {
                let answer = attack(&socket, &case, &draw).unwrap_or_else(|error| panic!("{}: {error}", case.name));
                assert_eq!(
                    answer == Answer::NoGlobal,
                    !served,
                    "{}: the session {} dmabuf, and a client {}",
                    case.name,
                    if served { "serves" } else { "does not serve" },
                    if answer == Answer::NoGlobal {
                        "found no global"
                    } else {
                        "found one"
                    },
                );
                report
                    .lock()
                    .expect("the report")
                    .push(format!("{}: {answer:?}", case.name));
                if answer == Answer::Accepted {
                    accepted.push(case.name);
                }
            }
            // The compositor is still up: a new client is answered and the output reads back.
            let (_connection, _globals, _queue, _app, _global) = connect(&socket);
            let deadline = Instant::now() + TIMEOUT;
            while control.screenshot(output).is_none() {
                assert!(Instant::now() < deadline, "the compositor never read its output back");
                thread::sleep(Duration::from_millis(10));
            }
            // Every attack ran to the end before this is judged, so one that takes the compositor down is seen as that.
            assert!(
                accepted.is_empty(),
                "the compositor accepted buffers no driver can map, and drew them: {accepted:?}"
            );
        }
    });
    let outcome: Result<(), LoopError> = server.run();
    if let Err(payload) = worker.join() {
        std::panic::resume_unwind(payload);
    }
    outcome.expect("the loop stops cleanly");
    for line in report.lock().expect("the report").iter() {
        println!("{line}");
    }
}
