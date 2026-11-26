//! `wp_linux_drm_syncobj_manager_v1` against the real compositor: explicit sync of a client's GPU buffers.
//!
//! The protocol layer under test is the real one: Smithay's objects and validation, the compositor's own refusals, the
//! blocker that holds a commit until its acquire point signals, and the table that holds a release point until the GPU is
//! done. What is stood in is the GPU. The renderer's answers (which device, how many frames it has finished) come from a
//! `DmabufProvider`, so a test controls exactly when a "frame retires".
//!
//! # Where it runs
//!
//! Part of it needs no device and runs everywhere, the Agent37 boxes included (no `/dev/dri`, a gVisor sandbox):
//!
//! - the global is not offered by a software, GLES or lavapipe session;
//! - the protocol errors that need no working timeline (a second object for a surface, a timeline that cannot be
//!   imported, a buffer with no points, a surface that gets its object again after destroying it), against a stand-in
//!   device that imports nothing.
//!
//! The rest needs a DRM render node whose kernel supports `DRM_IOCTL_SYNCOBJ_EVENTFD` (Linux 6.6 and later): the tests make
//! real timelines on it, wait on a real acquire point and read the release point back from the kernel. Where there is none a
//! test prints why and returns: compiled and linked on every gate, *run* on a machine with a GPU or a virtual one (`vkms`,
//! `virtio-gpu`, `vgem`). A skip is deliberate and loud, never a silent pass.
//!
//! Failure modes, each with its test here (the list is in `protocols::wp_linux_drm_syncobj_v1`; the notes of the review
//! that added the later ones are in `docs/wayland-protocols.md`):
//!
//! - `a_session_without_a_retiring_gpu_renderer_does_not_serve_syncobj`,
//!   `a_session_on_a_software_vulkan_device_does_not_serve_syncobj`
//! - `a_second_syncobj_surface_is_a_protocol_error`, `a_timeline_that_cannot_be_imported_is_a_protocol_error`,
//!   `a_buffer_with_no_points_is_a_protocol_error`
//! - Smithay's own errors, kept with a real client because the compositor's hook runs before Smithay's:
//!   `a_point_for_a_destroyed_surface_is_a_protocol_error`, `an_acquire_point_with_no_buffer_is_a_protocol_error`,
//!   `an_acquire_point_with_no_release_point_is_a_protocol_error`,
//!   `a_release_point_not_after_the_acquire_point_is_a_protocol_error`,
//!   `points_with_a_shared_memory_buffer_are_a_protocol_error`
//! - `a_release_point_with_no_buffer_is_no_buffer`
//! - `a_syncobj_surface_made_again_is_still_unique`, `a_syncobj_surface_made_again_checks_its_commits`
//! - `a_commit_waits_for_its_acquire_point`
//! - `a_surface_with_too_many_waiting_commits_ends_its_client`,
//!   `a_client_with_too_many_waits_across_surfaces_is_ended`, and the guard that a client that signals its points is
//!   never counted past a limit, `a_client_that_signals_its_points_never_reaches_its_limits`
//! - `a_surface_destroyed_mid_wait_frees_its_waits`, `a_client_that_disconnects_mid_wait_frees_its_waits`
//! - `a_surface_destroyed_mid_wait_signals_the_queued_release_points`,
//!   `a_client_that_disconnects_mid_wait_signals_the_queued_release_points`
//! - `a_release_point_signals_after_the_frame_retires`, `an_idle_session_releases_soon_after_the_gpu_retires`
//! - `a_buffer_attached_again_releases_the_earlier_commit`
//! - `a_device_made_anew_releases_every_replaced_point`
//!
//! Every session runs one at a time ([`SESSIONS`]): the tests that count the process's eventfds would otherwise count
//! another test's.

use gnoblin_wayland_testkit::stock;

use std::fs::File;
use std::net::Shutdown;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_backend::{Backend, DmabufImportError, DmabufProvider, DmabufSupport, ExplicitSync, Retirement};
use gnoblin_wayland::{Config, ControlHandle, Server, SessionSnapshot, State};
use gnoblin_wayland_protocols::wp_linux_drm_syncobj_v1::{CLIENT_WAIT_LIMIT, SURFACE_COMMIT_LIMIT};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::backend::drm::DrmDeviceFd;
use smithay::reexports::drm::control::Device as _;
use smithay::reexports::drm::control::syncobj;
use smithay::utils::DeviceFd;
use smithay::wayland::drm_syncobj::supports_syncobj_eventfd;
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, DispatchError, EventQueue, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_v1 as dmabuf_global,
};
use wayland_protocols::wp::linux_drm_syncobj::v1::client::{
    wp_linux_drm_syncobj_manager_v1 as manager, wp_linux_drm_syncobj_surface_v1 as sync_surface,
    wp_linux_drm_syncobj_timeline_v1 as timeline,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const TIMEOUT: Duration = Duration::from_secs(10);
/// How long a commit that must stay held is watched.
const HELD: Duration = Duration::from_millis(400);
const MANAGER: &str = "wp_linux_drm_syncobj_manager_v1";

/// The `wp_linux_drm_syncobj_surface_v1` error codes.
const NO_ACQUIRE_POINT: u32 = 4;
const NO_SURFACE: u32 = 1;
const UNSUPPORTED_BUFFER: u32 = 2;
const NO_BUFFER: u32 = 3;
const NO_RELEASE_POINT: u32 = 5;
const CONFLICTING_POINTS: u32 = 6;
/// `wl_display.error.no_memory`, what a client past a limit is ended with.
const NO_MEMORY: u32 = 2;
/// Held while a session runs.
static SESSIONS: Mutex<()> = Mutex::new(());
/// The `wp_linux_drm_syncobj_manager_v1` error codes.
const SURFACE_EXISTS: u32 = 0;
const INVALID_TIMELINE: u32 = 1;

fn offered() -> Vec<Format> {
    vec![Format {
        code: Fourcc::Argb8888,
        modifier: Modifier::Linear,
    }]
}

/// The renderer's answers, as a test sets them.
struct Mock {
    device: Option<DrmDeviceFd>,
    retirement: Arc<Mutex<Retirement>>,
}

impl DmabufProvider for Mock {
    fn support(&mut self) -> Option<DmabufSupport> {
        Some(DmabufSupport::new(None, offered()))
    }

    fn import(&mut self, _: &Dmabuf) -> Result<(), DmabufImportError> {
        Ok(())
    }

    fn explicit_sync(&mut self) -> Option<ExplicitSync> {
        self.device.clone().map(ExplicitSync::unchecked)
    }

    fn buffers_retired(&mut self) -> Retirement {
        *self.retirement.lock().expect("the retirement")
    }
}

#[derive(Default)]
struct App;

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
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    xdg_toplevel::XdgToplevel,
    dmabuf_global::ZwpLinuxDmabufV1,
    params::ZwpLinuxBufferParamsV1,
    manager::WpLinuxDrmSyncobjManagerV1,
    sync_surface::WpLinuxDrmSyncobjSurfaceV1,
    timeline::WpLinuxDrmSyncobjTimelineV1,
);

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

/// What a client of the session sees.
struct Client {
    globals: GlobalList,
    queue: EventQueue<App>,
    app: App,
    observation: Arc<Mutex<SessionSnapshot>>,
    /// A second handle on the socket, to hang up with.
    hangup: UnixStream,
}

impl Client {
    fn qh(&self) -> QueueHandle<App> {
        self.queue.handle()
    }

    /// Ends the connection as a client that died would: the compositor reads end of file and destroys its objects.
    fn hang_up(&self) {
        self.hangup.shutdown(Shutdown::Both).expect("the socket shuts down");
    }

    fn roundtrip(&mut self) -> Result<(), DispatchError> {
        self.queue.roundtrip(&mut self.app).map(|_| ())
    }

    fn mapped(&self) -> bool {
        self.observation
            .lock()
            .expect("the observation lock")
            .windows()
            .any(|window| window.mapped)
    }

    /// A toplevel that has had its first commit and its configure answered, ready for a buffer.
    fn toplevel(&mut self) -> (wl_surface::WlSurface, Vec<Box<dyn std::any::Any>>) {
        let qh = self.qh();
        let compositor: wl_compositor::WlCompositor = self.globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
        let wm_base: xdg_wm_base::XdgWmBase = self.globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg.get_toplevel(&qh, ());
        toplevel.set_title("syncobj window".to_owned());
        surface.commit();
        self.roundtrip().expect("the first commit is accepted");
        (
            surface,
            vec![
                Box::new(compositor),
                Box::new(wm_base),
                Box::new(xdg),
                Box::new(toplevel),
            ],
        )
    }

    /// A buffer that has the shape of a linear dmabuf, shared through `zwp_linux_buffer_params_v1`.
    fn dmabuf_buffer(&mut self) -> wl_buffer::WlBuffer {
        let qh = self.qh();
        let global: dmabuf_global::ZwpLinuxDmabufV1 = self.globals.bind(&qh, 2..=3, ()).expect("zwp_linux_dmabuf_v1");
        let (width, height) = (64u32, 48u32);
        let file = tempfile::tempfile().expect("a temporary file");
        file.set_len(u64::from(width * height * 4)).expect("a sized file");
        let params = global.create_params(&qh, ());
        params.add(OwnedFd::from(file).as_fd(), 0, 0, width * 4, 0, 0);
        params.create_immed(
            width as i32,
            height as i32,
            Fourcc::Argb8888 as u32,
            params::Flags::empty(),
            &qh,
            (),
        )
    }

    /// A shared-memory buffer: a client that names no points for it is the case the protocol refuses.
    fn shm_buffer(&mut self) -> wl_buffer::WlBuffer {
        let qh = self.qh();
        let shm: wl_shm::WlShm = self.globals.bind(&qh, 1..=2, ()).expect("wl_shm");
        let file = tempfile::tempfile().expect("a temporary file");
        file.set_len(4 * 4 * 4).expect("a sized file");
        let pool = shm.create_pool(file.as_fd(), 4 * 4 * 4, &qh, ());
        pool.create_buffer(0, 4, 4, 16, wl_shm::Format::Argb8888, &qh, ())
    }

    fn manager(&mut self) -> manager::WpLinuxDrmSyncobjManagerV1 {
        let qh = self.qh();
        self.globals.bind(&qh, 1..=1, ()).expect(MANAGER)
    }
}

/// Runs `client` against a session on `backend`, and returns what it returns.
fn session<T: Send + 'static>(backend: Headless, client: impl FnOnce(Client) -> T + Send + 'static) -> T {
    let _serial = SESSIONS.lock().unwrap_or_else(PoisonError::into_inner);
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = directory.path().join("wayland-0.sock");
    let config = Config {
        socket_path: Some(socket.clone()),
        ..crate::stock::config()
    };
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    let control = server.control();
    let observation = server.observation();
    let worker = thread::spawn(move || {
        let _stop = StopOnDrop(control);
        let stream = UnixStream::connect(&socket).expect("the session is listening");
        let hangup = stream.try_clone().expect("the socket clones");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        client(Client {
            globals,
            queue,
            app: App,
            observation,
            hangup,
        })
    });
    let outcome = server.run();
    let result = match worker.join() {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    };
    outcome.expect("the loop stops cleanly");
    result
}

fn advertises_syncobj(client: &Client) -> bool {
    client
        .globals
        .contents()
        .with_list(|list| list.iter().any(|global| global.interface == MANAGER))
}

/// A device that is not a GPU, for the errors that need none. It imports no timeline and waits on nothing.
fn stand_in_device() -> DrmDeviceFd {
    let file = File::open("/dev/null").expect("/dev/null opens");
    DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)))
}

fn stand_in_backend(device: Option<DrmDeviceFd>) -> (Headless, Arc<Mutex<Retirement>>) {
    let retirement = Arc::new(Mutex::new(Retirement::default()));
    let backend = Headless::new().with_dmabuf_provider(Box::new(Mock {
        device,
        retirement: Arc::clone(&retirement),
    }));
    (backend, retirement)
}

/// The protocol error a failed round trip carries: the interface of the object and the code.
fn protocol_error(result: Result<(), DispatchError>) -> (String, u32) {
    match result {
        Err(DispatchError::Backend(WaylandError::Protocol(error))) => (error.object_interface, error.code),
        other => panic!("the round trip should end in a protocol error, not {other:?}"),
    }
}

#[test]
fn a_session_without_a_retiring_gpu_renderer_does_not_serve_syncobj() {
    // The software renderer reports no retirement, so no device is offered.
    let served = session(Headless::new(), |client| advertises_syncobj(&client));
    assert!(!served, "a software session advertises explicit sync");
    // A stand-in that offers no device behaves the same, and so does one that imports buffers only.
    let (backend, _) = stand_in_backend(None);
    let served = session(backend, |client| advertises_syncobj(&client));
    assert!(
        !served,
        "a session whose backend offers no device advertises explicit sync"
    );
    // GLES draws synchronously and reports no retirement: it is never offered the protocol.
    match Headless::with_config(HeadlessConfig::gles()) {
        Ok(backend) => {
            let served = session(backend, |client| advertises_syncobj(&client));
            assert!(!served, "a GLES session advertises explicit sync");
        }
        Err(error) => println!("not running the GLES half: no GLES renderer on this machine ({error})"),
    }
}

#[test]
fn a_session_on_a_software_vulkan_device_does_not_serve_syncobj() {
    let mut backend = match Headless::with_config(HeadlessConfig::vulkan()) {
        Ok(backend) => backend,
        Err(error) => {
            println!("skipping: no Vulkan renderer on this machine ({error})");
            return;
        }
    };
    // A software driver (lavapipe) names no DRM render node; the offer must follow the device.
    let support = Backend::<State>::dmabuf(&mut backend);
    let node = support.as_ref().and_then(|support| support.main_device);
    let expected = ExplicitSync::on_node(node).is_ok();
    println!(
        "the Vulkan session reports dmabuf support {} and render node {node:?}; explicit sync should be served: {expected}",
        if support.is_some() { "yes" } else { "none" }
    );
    if node.is_none() {
        assert!(!expected, "a device with no DRM node is offered explicit sync");
    }
    let served = session(backend, |client| advertises_syncobj(&client));
    println!("the session advertises {MANAGER}: {served}");
    assert_eq!(served, expected, "the session's offer differs from the device's");
}

#[test]
fn a_second_syncobj_surface_is_a_protocol_error() {
    let (backend, _) = stand_in_backend(Some(stand_in_device()));
    let (interface, code) = session(backend, |mut client| {
        let manager = client.manager();
        let qh = client.qh();
        let surface = {
            let compositor: wl_compositor::WlCompositor = client.globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
            compositor.create_surface(&qh, ())
        };
        let _first = manager.get_surface(&surface, &qh, ());
        client.roundtrip().expect("one object per surface is fine");
        let _second = manager.get_surface(&surface, &qh, ());
        protocol_error(client.roundtrip())
    });
    assert_eq!((interface.as_str(), code), (MANAGER, SURFACE_EXISTS));
}

#[test]
fn a_timeline_that_cannot_be_imported_is_a_protocol_error() {
    let (backend, _) = stand_in_backend(Some(stand_in_device()));
    let (interface, code) = session(backend, |mut client| {
        let manager = client.manager();
        let qh = client.qh();
        // A file that is no syncobj.
        let file = tempfile::tempfile().expect("a temporary file");
        let _timeline = manager.import_timeline(file.as_fd(), &qh, ());
        protocol_error(client.roundtrip())
    });
    assert_eq!((interface.as_str(), code), (MANAGER, INVALID_TIMELINE));
}

#[test]
fn a_buffer_with_no_points_is_a_protocol_error() {
    let (backend, _) = stand_in_backend(Some(stand_in_device()));
    let (interface, code) = session(backend, |mut client| {
        let manager = client.manager();
        let (surface, _keep) = client.toplevel();
        let qh = client.qh();
        let _sync = manager.get_surface(&surface, &qh, ());
        let buffer = client.shm_buffer();
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        protocol_error(client.roundtrip())
    });
    assert_eq!(
        (interface.as_str(), code),
        ("wp_linux_drm_syncobj_surface_v1", NO_ACQUIRE_POINT)
    );
}

/// A render node that supports timeline eventfds, opened, or why there is none.
fn drm_device() -> Result<DrmDeviceFd, String> {
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")
        .map_err(|error| format!("/dev/dri cannot be read ({error}): no DRM device"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("renderD"))
        })
        .collect();
    nodes.sort();
    let mut why = "/dev/dri has no render node".to_owned();
    for path in nodes {
        let file = match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                why = format!("{} cannot be opened: {error}", path.display());
                continue;
            }
        };
        let device = DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file)));
        if supports_syncobj_eventfd(&device) {
            return Ok(device);
        }
        why = format!(
            "{} does not support syncobj eventfds (Linux 6.6 and later)",
            path.display()
        );
    }
    Err(why)
}

/// What a test holds of a timeline it made on the device: the kernel's handle and the descriptor the client imports.
struct Timeline {
    handle: syncobj::Handle,
}

impl Timeline {
    fn new(device: &DrmDeviceFd) -> (Self, OwnedFd) {
        let handle = device.create_syncobj(false).expect("a syncobj");
        let fd = device.syncobj_to_fd(handle, false).expect("a syncobj descriptor");
        (Self { handle }, fd)
    }

    fn signal(&self, device: &DrmDeviceFd, point: u64) {
        device
            .syncobj_timeline_signal(&[self.handle], &[point])
            .expect("the point signals");
    }

    fn signalled(&self, device: &DrmDeviceFd, point: u64) -> bool {
        let mut last = [0u64];
        device
            .syncobj_timeline_query(&[self.handle], &mut last, false)
            .expect("the timeline answers");
        last[0] >= point
    }
}

/// Waits until `condition` holds, or fails with `what`.
fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// A session on a stand-in renderer that offers the real device, with the controls a DRM test needs.
fn drm_session<T: Send + 'static>(
    test: &str,
    run: impl FnOnce(Client, DrmDeviceFd, Arc<Mutex<Retirement>>) -> T + Send + 'static,
) -> Option<T> {
    let device = match drm_device() {
        Ok(device) => device,
        Err(reason) => {
            println!("skipping {test}: {reason}");
            return None;
        }
    };
    let (backend, retirement) = stand_in_backend(Some(device.clone()));
    Some(session(backend, move |client| run(client, device, retirement)))
}

/// Commits `buffer` on `surface` with the acquire point `acquire` of `acquire_timeline` and the release point `release` of
/// `release_timeline`.
fn commit_with_points(
    surface: &wl_surface::WlSurface,
    sync: &sync_surface::WpLinuxDrmSyncobjSurfaceV1,
    buffer: &wl_buffer::WlBuffer,
    acquire: (&timeline::WpLinuxDrmSyncobjTimelineV1, u32),
    release: (&timeline::WpLinuxDrmSyncobjTimelineV1, u32),
) {
    sync.set_acquire_point(acquire.0, 0, acquire.1);
    sync.set_release_point(release.0, 0, release.1);
    surface.attach(Some(buffer), 0, 0);
    surface.damage(0, 0, 64, 48);
    surface.commit();
}

#[test]
fn a_commit_waits_for_its_acquire_point() {
    let Some(()) = drm_session("a_commit_waits_for_its_acquire_point", |mut client, device, _| {
        let manager = client.manager();
        let (surface, _keep) = client.toplevel();
        let qh = client.qh();
        let sync = manager.get_surface(&surface, &qh, ());
        let (acquire, acquire_fd) = Timeline::new(&device);
        let (release, release_fd) = Timeline::new(&device);
        let acquire_timeline = manager.import_timeline(acquire_fd.as_fd(), &qh, ());
        let release_timeline = manager.import_timeline(release_fd.as_fd(), &qh, ());
        let buffer = client.dmabuf_buffer();
        commit_with_points(&surface, &sync, &buffer, (&acquire_timeline, 1), (&release_timeline, 1));
        client.roundtrip().expect("the commit is accepted");
        // The buffer is not ready: the window must not map while the acquire point is unsignalled.
        let until = Instant::now() + HELD;
        while Instant::now() < until {
            assert!(
                !client.mapped(),
                "the commit was applied before its acquire point signalled"
            );
            client.roundtrip().expect("the round trip");
            thread::sleep(Duration::from_millis(20));
        }
        acquire.signal(&device, 1);
        wait_for("the window to map once the acquire point signalled", || {
            client.roundtrip().expect("the round trip");
            client.mapped()
        });
        // The release point is not signalled while the buffer is the surface's content.
        assert!(
            !release.signalled(&device, 1),
            "the release point signalled while the buffer is shown"
        );
    }) else {
        return;
    };
}

#[test]
fn a_release_point_signals_after_the_frame_retires() {
    let Some(()) = drm_session(
        "a_release_point_signals_after_the_frame_retires",
        |mut client, device, retirement| {
            let manager = client.manager();
            let (surface, _keep) = client.toplevel();
            let qh = client.qh();
            let sync = manager.get_surface(&surface, &qh, ());
            let (acquire, acquire_fd) = Timeline::new(&device);
            let acquire_timeline = manager.import_timeline(acquire_fd.as_fd(), &qh, ());
            let (first, first_fd) = Timeline::new(&device);
            let (second, second_fd) = Timeline::new(&device);
            let first_release = manager.import_timeline(first_fd.as_fd(), &qh, ());
            let second_release = manager.import_timeline(second_fd.as_fd(), &qh, ());
            let one = client.dmabuf_buffer();
            let two = client.dmabuf_buffer();
            // Three frames are in flight when the second buffer replaces the first.
            *retirement.lock().expect("the retirement") = Retirement {
                submitted: 3,
                retired: 0,
            };
            acquire.signal(&device, 2);
            commit_with_points(&surface, &sync, &one, (&acquire_timeline, 1), (&first_release, 1));
            commit_with_points(&surface, &sync, &two, (&acquire_timeline, 2), (&second_release, 1));
            client.roundtrip().expect("the commits are accepted");
            wait_for("the window to map", || {
                client.roundtrip().expect("the round trip");
                client.mapped()
            });
            // The first buffer is replaced, but the GPU has not finished the frames that may have read it.
            let until = Instant::now() + HELD;
            while Instant::now() < until {
                assert!(
                    !first.signalled(&device, 1),
                    "the release point signalled before the GPU retired the frames that may have read the buffer"
                );
                client.roundtrip().expect("the round trip");
                thread::sleep(Duration::from_millis(20));
            }
            // Two of three frames retire: still not enough.
            retirement.lock().expect("the retirement").retired = 2;
            thread::sleep(HELD);
            assert!(
                !first.signalled(&device, 1),
                "the release point signalled with a frame still running"
            );
            retirement.lock().expect("the retirement").retired = 3;
            wait_for("the release point to signal once every frame retired", || {
                first.signalled(&device, 1)
            });
            // The buffer still shown is not released.
            assert!(!second.signalled(&device, 1), "the shown buffer was released");
        },
    ) else {
        return;
    };
}

#[test]
fn a_buffer_attached_again_releases_the_earlier_commit() {
    let Some(()) = drm_session(
        "a_buffer_attached_again_releases_the_earlier_commit",
        |mut client, device, retirement| {
            let manager = client.manager();
            let (surface, _keep) = client.toplevel();
            let qh = client.qh();
            let sync = manager.get_surface(&surface, &qh, ());
            let (acquire, acquire_fd) = Timeline::new(&device);
            let acquire_timeline = manager.import_timeline(acquire_fd.as_fd(), &qh, ());
            let (first, first_fd) = Timeline::new(&device);
            let (second, second_fd) = Timeline::new(&device);
            let first_release = manager.import_timeline(first_fd.as_fd(), &qh, ());
            let second_release = manager.import_timeline(second_fd.as_fd(), &qh, ());
            let buffer = client.dmabuf_buffer();
            *retirement.lock().expect("the retirement") = Retirement {
                submitted: 1,
                retired: 1,
            };
            acquire.signal(&device, 2);
            commit_with_points(&surface, &sync, &buffer, (&acquire_timeline, 1), (&first_release, 1));
            client.roundtrip().expect("the first commit is accepted");
            wait_for("the window to map", || {
                client.roundtrip().expect("the round trip");
                client.mapped()
            });
            // A client that draws into one buffer waits for its release before it commits it again; the release of the first
            // commit must not wait for a different buffer that never comes.
            commit_with_points(&surface, &sync, &buffer, (&acquire_timeline, 2), (&second_release, 1));
            client.roundtrip().expect("the second commit is accepted");
            wait_for(
                "the first commit's release point, with the same buffer attached again",
                || {
                    client.roundtrip().expect("the round trip");
                    first.signalled(&device, 1)
                },
            );
            assert!(!second.signalled(&device, 1), "the commit that is shown was released");
        },
    ) else {
        return;
    };
}

#[test]
fn a_device_made_anew_releases_every_replaced_point() {
    let Some(()) = drm_session(
        "a_device_made_anew_releases_every_replaced_point",
        |mut client, device, retirement| {
            let manager = client.manager();
            let (surface, _keep) = client.toplevel();
            let qh = client.qh();
            let sync = manager.get_surface(&surface, &qh, ());
            let (acquire, acquire_fd) = Timeline::new(&device);
            let acquire_timeline = manager.import_timeline(acquire_fd.as_fd(), &qh, ());
            let (first, first_fd) = Timeline::new(&device);
            let (second, second_fd) = Timeline::new(&device);
            let first_release = manager.import_timeline(first_fd.as_fd(), &qh, ());
            let second_release = manager.import_timeline(second_fd.as_fd(), &qh, ());
            let one = client.dmabuf_buffer();
            let two = client.dmabuf_buffer();
            // Frames are running on a device that is then lost and made anew: its count starts again from zero.
            *retirement.lock().expect("the retirement") = Retirement {
                submitted: 9,
                retired: 0,
            };
            acquire.signal(&device, 2);
            commit_with_points(&surface, &sync, &one, (&acquire_timeline, 1), (&first_release, 1));
            commit_with_points(&surface, &sync, &two, (&acquire_timeline, 2), (&second_release, 1));
            client.roundtrip().expect("the commits are accepted");
            wait_for("the window to map", || {
                client.roundtrip().expect("the round trip");
                client.mapped()
            });
            thread::sleep(HELD);
            assert!(
                !first.signalled(&device, 1),
                "the release point signalled with nine frames running"
            );
            *retirement.lock().expect("the retirement") = Retirement {
                submitted: 0,
                retired: 0,
            };
            wait_for(
                "the replaced content's release point after the device was made anew",
                || first.signalled(&device, 1),
            );
            // What the surface still shows is read again by the new device, so it stays held.
            assert!(
                !second.signalled(&device, 1),
                "the shown buffer was released when the device was made anew"
            );
        },
    ) else {
        return;
    };
}

/// The interface of the `wp_linux_drm_syncobj_surface_v1` object, which Smithay and the compositor raise its errors on.
const SYNC_SURFACE: &str = "wp_linux_drm_syncobj_surface_v1";

/// The eventfds the process holds open. The compositor keeps one for each acquire point a commit waits on, and the
/// session shares the process with the test, so a test compares counts before and after rather than reading one.
fn eventfds() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("the process's descriptors can be listed")
        .filter_map(Result::ok)
        .filter(|entry| {
            std::fs::read_link(entry.path()).is_ok_and(|target| target.to_str() == Some("anon_inode:[eventfd]"))
        })
        .count()
}

/// A surface with no role, for the tests that destroy it or need many: nothing about explicit sync needs a window.
fn plain_surface(client: &mut Client) -> (wl_surface::WlSurface, wl_compositor::WlCompositor) {
    let qh = client.qh();
    let compositor: wl_compositor::WlCompositor = client.globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
    (compositor.create_surface(&qh, ()), compositor)
}

/// A timeline made on the device and imported by the client: the kernel's handle, to signal and read it, and the client's
/// object, to name points of it.
fn import(
    client: &mut Client,
    manager: &manager::WpLinuxDrmSyncobjManagerV1,
    device: &DrmDeviceFd,
) -> (Timeline, timeline::WpLinuxDrmSyncobjTimelineV1) {
    let qh = client.qh();
    let (kernel, fd) = Timeline::new(device);
    let imported = manager.import_timeline(fd.as_fd(), &qh, ());
    client.roundtrip().expect("the timeline is imported");
    (kernel, imported)
}

/// Runs `build` on a surface that has the explicit sync object, and returns the protocol error the round trip after it
/// ends in. `None` where there is no DRM device.
fn commit_error(
    test: &str,
    build: impl FnOnce(
        &mut Client,
        &DrmDeviceFd,
        &manager::WpLinuxDrmSyncobjManagerV1,
        &wl_surface::WlSurface,
        &sync_surface::WpLinuxDrmSyncobjSurfaceV1,
    ) + Send
    + 'static,
) -> Option<(String, u32)> {
    drm_session(test, move |mut client, device, _| {
        let manager = client.manager();
        let (surface, _compositor) = plain_surface(&mut client);
        let qh = client.qh();
        let sync = manager.get_surface(&surface, &qh, ());
        build(&mut client, &device, &manager, &surface, &sync);
        protocol_error(client.roundtrip())
    })
}

#[test]
fn a_point_for_a_destroyed_surface_is_a_protocol_error() {
    let Some(error) = drm_session(
        "a_point_for_a_destroyed_surface_is_a_protocol_error",
        |mut client, device, _| {
            let manager = client.manager();
            let (surface, _compositor) = plain_surface(&mut client);
            let qh = client.qh();
            let sync = manager.get_surface(&surface, &qh, ());
            let (_kernel, imported) = import(&mut client, &manager, &device);
            surface.destroy();
            client.roundtrip().expect("the surface is destroyed");
            sync.set_acquire_point(&imported, 0, 1);
            protocol_error(client.roundtrip())
        },
    ) else {
        return;
    };
    assert_eq!((error.0.as_str(), error.1), (SYNC_SURFACE, NO_SURFACE));
}

#[test]
fn an_acquire_point_with_no_buffer_is_a_protocol_error() {
    let Some(error) = commit_error(
        "an_acquire_point_with_no_buffer_is_a_protocol_error",
        |client, device, manager, surface, sync| {
            let (_kernel, imported) = import(client, manager, device);
            sync.set_acquire_point(&imported, 0, 1);
            surface.commit();
        },
    ) else {
        return;
    };
    assert_eq!((error.0.as_str(), error.1), (SYNC_SURFACE, NO_BUFFER));
}

#[test]
fn an_acquire_point_with_no_release_point_is_a_protocol_error() {
    let Some(error) = commit_error(
        "an_acquire_point_with_no_release_point_is_a_protocol_error",
        |client, device, manager, surface, sync| {
            let (kernel, imported) = import(client, manager, device);
            kernel.signal(device, 1);
            let buffer = client.dmabuf_buffer();
            sync.set_acquire_point(&imported, 0, 1);
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
        },
    ) else {
        return;
    };
    assert_eq!((error.0.as_str(), error.1), (SYNC_SURFACE, NO_RELEASE_POINT));
}

#[test]
fn a_release_point_not_after_the_acquire_point_is_a_protocol_error() {
    let Some(error) = commit_error(
        "a_release_point_not_after_the_acquire_point_is_a_protocol_error",
        |client, device, manager, surface, sync| {
            let (kernel, imported) = import(client, manager, device);
            kernel.signal(device, 5);
            let buffer = client.dmabuf_buffer();
            // One timeline, and a release point that is the acquire point.
            sync.set_acquire_point(&imported, 0, 5);
            sync.set_release_point(&imported, 0, 5);
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
        },
    ) else {
        return;
    };
    assert_eq!((error.0.as_str(), error.1), (SYNC_SURFACE, CONFLICTING_POINTS));
}

#[test]
fn points_with_a_shared_memory_buffer_are_a_protocol_error() {
    let Some(error) = commit_error(
        "points_with_a_shared_memory_buffer_are_a_protocol_error",
        |client, device, manager, surface, sync| {
            let (acquire, acquire_timeline) = import(client, manager, device);
            let (_release, release_timeline) = import(client, manager, device);
            acquire.signal(device, 1);
            let buffer = client.shm_buffer();
            sync.set_acquire_point(&acquire_timeline, 0, 1);
            sync.set_release_point(&release_timeline, 0, 1);
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
        },
    ) else {
        return;
    };
    assert_eq!((error.0.as_str(), error.1), (SYNC_SURFACE, UNSUPPORTED_BUFFER));
}

#[test]
fn a_release_point_with_no_buffer_is_no_buffer() {
    // A release point alone, with nothing attached and with the buffer removed: the protocol names `no_buffer` for both,
    // and Smithay names `no_acquire_point` for a release point with no acquire point.
    for removes_buffer in [false, true] {
        let Some(error) = commit_error(
            "a_release_point_with_no_buffer_is_no_buffer",
            move |client, device, manager, surface, sync| {
                let (_kernel, imported) = import(client, manager, device);
                sync.set_release_point(&imported, 0, 1);
                if removes_buffer {
                    surface.attach(None, 0, 0);
                }
                surface.commit();
            },
        ) else {
            return;
        };
        assert_eq!(
            (error.0.as_str(), error.1),
            (SYNC_SURFACE, NO_BUFFER),
            "a release point with no buffer (buffer removed: {removes_buffer})"
        );
    }
}

#[test]
fn a_syncobj_surface_made_again_is_still_unique() {
    let (backend, _) = stand_in_backend(Some(stand_in_device()));
    let (interface, code) = session(backend, |mut client| {
        let manager = client.manager();
        let (surface, _compositor) = plain_surface(&mut client);
        let qh = client.qh();
        let first = manager.get_surface(&surface, &qh, ());
        client.roundtrip().expect("the first object is made");
        first.destroy();
        client.roundtrip().expect("the object is destroyed");
        let _second = manager.get_surface(&surface, &qh, ());
        client
            .roundtrip()
            .expect("a surface that destroyed its object may have another");
        let _third = manager.get_surface(&surface, &qh, ());
        protocol_error(client.roundtrip())
    });
    assert_eq!((interface.as_str(), code), (MANAGER, SURFACE_EXISTS));
}

#[test]
fn a_syncobj_surface_made_again_checks_its_commits() {
    let (backend, _) = stand_in_backend(Some(stand_in_device()));
    let (interface, code) = session(backend, |mut client| {
        let manager = client.manager();
        let (surface, _compositor) = plain_surface(&mut client);
        let qh = client.qh();
        let first = manager.get_surface(&surface, &qh, ());
        client.roundtrip().expect("the first object is made");
        first.destroy();
        client.roundtrip().expect("the object is destroyed");
        // With no object the surface takes a buffer as any surface does.
        let plain = client.shm_buffer();
        surface.attach(Some(&plain), 0, 0);
        surface.commit();
        client
            .roundtrip()
            .expect("a surface with no object commits a buffer with no points");
        // With the new object it must name its points.
        let _second = manager.get_surface(&surface, &qh, ());
        client.roundtrip().expect("the second object is made");
        let again = client.shm_buffer();
        surface.attach(Some(&again), 0, 0);
        surface.commit();
        protocol_error(client.roundtrip())
    });
    assert_eq!((interface.as_str(), code), (SYNC_SURFACE, NO_ACQUIRE_POINT));
}

/// A client that has made `count` commits on `surface`, each waiting on an acquire point that never signals. `next` is the
/// number of the first point to name, and is left past the last.
fn commit_waiting(
    surface: &wl_surface::WlSurface,
    sync: &sync_surface::WpLinuxDrmSyncobjSurfaceV1,
    buffer: &wl_buffer::WlBuffer,
    timelines: (
        &timeline::WpLinuxDrmSyncobjTimelineV1,
        &timeline::WpLinuxDrmSyncobjTimelineV1,
    ),
    next: &mut u32,
    count: usize,
) {
    for _ in 0..count {
        commit_with_points(surface, sync, buffer, (timelines.0, *next), (timelines.1, *next));
        *next += 1;
    }
}

#[test]
fn a_surface_with_too_many_waiting_commits_ends_its_client() {
    let Some(error) = drm_session(
        "a_surface_with_too_many_waiting_commits_ends_its_client",
        |mut client, device, _| {
            let manager = client.manager();
            let (surface, _compositor) = plain_surface(&mut client);
            let qh = client.qh();
            let sync = manager.get_surface(&surface, &qh, ());
            let (_never, acquire) = import(&mut client, &manager, &device);
            let (_released, release) = import(&mut client, &manager, &device);
            let buffer = client.dmabuf_buffer();
            let mut next = 1;
            commit_waiting(
                &surface,
                &sync,
                &buffer,
                (&acquire, &release),
                &mut next,
                SURFACE_COMMIT_LIMIT,
            );
            client
                .roundtrip()
                .expect("a surface with as many waiting commits as it may have is not refused");
            commit_waiting(&surface, &sync, &buffer, (&acquire, &release), &mut next, 1);
            protocol_error(client.roundtrip())
        },
    ) else {
        return;
    };
    assert_eq!((error.0.as_str(), error.1), ("wl_display", NO_MEMORY));
}

#[test]
fn a_client_with_too_many_waits_across_surfaces_is_ended() {
    let Some(error) = drm_session(
        "a_client_with_too_many_waits_across_surfaces_is_ended",
        |mut client, device, _| {
            let manager = client.manager();
            let qh = client.qh();
            let (_never, acquire) = import(&mut client, &manager, &device);
            let (_released, release) = import(&mut client, &manager, &device);
            let buffer = client.dmabuf_buffer();
            // Enough surfaces that no one of them reaches its own limit before the client reaches its.
            let surfaces: Vec<_> = (0..=CLIENT_WAIT_LIMIT / SURFACE_COMMIT_LIMIT)
                .map(|_| {
                    let (surface, compositor) = plain_surface(&mut client);
                    let sync = manager.get_surface(&surface, &qh, ());
                    (surface, sync, compositor)
                })
                .collect();
            let mut next = 1;
            for held in 0..CLIENT_WAIT_LIMIT {
                let (surface, sync, _) = &surfaces[held / SURFACE_COMMIT_LIMIT];
                commit_waiting(surface, sync, &buffer, (&acquire, &release), &mut next, 1);
            }
            client
                .roundtrip()
                .expect("a client with as many waits as it may have is not refused");
            let (surface, sync, _) = &surfaces[CLIENT_WAIT_LIMIT / SURFACE_COMMIT_LIMIT];
            commit_waiting(surface, sync, &buffer, (&acquire, &release), &mut next, 1);
            protocol_error(client.roundtrip())
        },
    ) else {
        return;
    };
    assert_eq!((error.0.as_str(), error.1), ("wl_display", NO_MEMORY));
}

#[test]
fn a_client_that_signals_its_points_never_reaches_its_limits() {
    let Some(()) = drm_session(
        "a_client_that_signals_its_points_never_reaches_its_limits",
        |mut client, device, _| {
            let manager = client.manager();
            let (surface, _compositor) = plain_surface(&mut client);
            let qh = client.qh();
            let sync = manager.get_surface(&surface, &qh, ());
            let (acquire, acquire_timeline) = import(&mut client, &manager, &device);
            let (_released, release_timeline) = import(&mut client, &manager, &device);
            let buffer = client.dmabuf_buffer();
            client.roundtrip().expect("the buffer is made");
            let baseline = eventfds();
            // Three times what a client may have outstanding, one commit at a time: a wait that fired and a commit that
            // was applied are not counted against the client or its surface any more, or the limits would be reached.
            let commits = u32::try_from(3 * CLIENT_WAIT_LIMIT.max(SURFACE_COMMIT_LIMIT)).expect("a small count");
            for point in 1..=commits {
                commit_with_points(
                    &surface,
                    &sync,
                    &buffer,
                    (&acquire_timeline, point),
                    (&release_timeline, point),
                );
                client
                    .roundtrip()
                    .expect("a commit of a client that signals its points is accepted");
                acquire.signal(&device, u64::from(point));
                wait_for("the wait of the commit to fire and close its eventfd", || {
                    eventfds() <= baseline
                });
            }
            client.roundtrip().expect("the last commit is accepted");
        },
    ) else {
        return;
    };
}

/// What a test holds of a surface with two commits queued: the first waits on an acquire point that never signals, the
/// second's acquire point signalled and it waits only behind the first.
struct QueuedPair {
    surface: wl_surface::WlSurface,
    first: Timeline,
    second: Timeline,
    /// The eventfds the process held before the first commit.
    baseline: usize,
    _keep: Vec<Box<dyn std::any::Any>>,
}

fn queue_two_commits(client: &mut Client, device: &DrmDeviceFd) -> QueuedPair {
    let manager = client.manager();
    let (surface, compositor) = plain_surface(client);
    let qh = client.qh();
    let sync = manager.get_surface(&surface, &qh, ());
    let (_never, never_timeline) = import(client, &manager, device);
    let (ready, ready_timeline) = import(client, &manager, device);
    let (first, first_release) = import(client, &manager, device);
    let (second, second_release) = import(client, &manager, device);
    ready.signal(device, 1);
    let buffer = client.dmabuf_buffer();
    client.roundtrip().expect("the buffer is made");
    let baseline = eventfds();
    commit_with_points(&surface, &sync, &buffer, (&never_timeline, 1), (&first_release, 1));
    client.roundtrip().expect("the first commit is accepted");
    commit_with_points(&surface, &sync, &buffer, (&ready_timeline, 1), (&second_release, 1));
    client.roundtrip().expect("the second commit is accepted");
    // The first commit holds an eventfd for its wait; the second's point has signalled and holds none.
    wait_for("the first commit's wait to hold an eventfd", || eventfds() > baseline);
    QueuedPair {
        surface,
        first,
        second,
        baseline,
        _keep: vec![
            Box::new(manager),
            Box::new(compositor),
            Box::new(sync),
            Box::new(never_timeline),
            Box::new(ready_timeline),
            Box::new(first_release),
            Box::new(second_release),
            Box::new(buffer),
        ],
    }
}

#[test]
fn a_surface_destroyed_mid_wait_frees_its_waits() {
    let Some(()) = drm_session(
        "a_surface_destroyed_mid_wait_frees_its_waits",
        |mut client, device, _| {
            let queued = queue_two_commits(&mut client, &device);
            queued.surface.destroy();
            client.roundtrip().expect("the surface is destroyed");
            wait_for("the destroyed surface's wait to close its eventfd", || {
                eventfds() <= queued.baseline
            });
        },
    ) else {
        return;
    };
}

#[test]
fn a_client_that_disconnects_mid_wait_frees_its_waits() {
    let Some(()) = drm_session(
        "a_client_that_disconnects_mid_wait_frees_its_waits",
        |mut client, device, _| {
            let queued = queue_two_commits(&mut client, &device);
            client.hang_up();
            wait_for("the dead client's wait to close its eventfd", || {
                eventfds() <= queued.baseline
            });
        },
    ) else {
        return;
    };
}

#[test]
fn a_surface_destroyed_mid_wait_signals_the_queued_release_points() {
    let Some(()) = drm_session(
        "a_surface_destroyed_mid_wait_signals_the_queued_release_points",
        |mut client, device, _| {
            let queued = queue_two_commits(&mut client, &device);
            assert!(
                !queued.first.signalled(&device, 1) && !queued.second.signalled(&device, 1),
                "a release point signalled while its commit was queued"
            );
            queued.surface.destroy();
            client.roundtrip().expect("the surface is destroyed");
            wait_for("the release points of both queued commits, which nothing read", || {
                queued.first.signalled(&device, 1) && queued.second.signalled(&device, 1)
            });
        },
    ) else {
        return;
    };
}

#[test]
fn a_client_that_disconnects_mid_wait_signals_the_queued_release_points() {
    let Some(()) = drm_session(
        "a_client_that_disconnects_mid_wait_signals_the_queued_release_points",
        |mut client, device, _| {
            let queued = queue_two_commits(&mut client, &device);
            client.hang_up();
            wait_for("the release points of both queued commits, which nothing read", || {
                queued.first.signalled(&device, 1) && queued.second.signalled(&device, 1)
            });
        },
    ) else {
        return;
    };
}

#[test]
fn an_idle_session_releases_soon_after_the_gpu_retires() {
    /// The loop's own timeout is 100 ms; a release that waits for it is slower than this.
    const PROMPT: Duration = Duration::from_millis(60);
    const TRIALS: u32 = 11;
    let Some(worst) = drm_session(
        "an_idle_session_releases_soon_after_the_gpu_retires",
        |mut client, device, retirement| {
            let manager = client.manager();
            let (surface, _compositor) = plain_surface(&mut client);
            let qh = client.qh();
            let sync = manager.get_surface(&surface, &qh, ());
            let (acquire, acquire_timeline) = import(&mut client, &manager, &device);
            let (release, release_timeline) = import(&mut client, &manager, &device);
            acquire.signal(&device, u64::from(TRIALS) + 1);
            let buffer = client.dmabuf_buffer();
            *retirement.lock().expect("the retirement") = Retirement {
                submitted: 1,
                retired: 1,
            };
            commit_with_points(&surface, &sync, &buffer, (&acquire_timeline, 1), (&release_timeline, 1));
            client.roundtrip().expect("the first commit is accepted");
            let mut worst = Duration::ZERO;
            for trial in 2..=TRIALS + 1 {
                // `trial` frames are in flight when the commit replaces the content, and the last one finishes while the
                // session is idle: nothing the client does wakes the loop.
                *retirement.lock().expect("the retirement") = Retirement {
                    submitted: u64::from(trial),
                    retired: u64::from(trial) - 1,
                };
                commit_with_points(
                    &surface,
                    &sync,
                    &buffer,
                    (&acquire_timeline, trial),
                    (&release_timeline, trial),
                );
                client.roundtrip().expect("the commit is accepted");
                thread::sleep(Duration::from_millis(30));
                assert!(
                    !release.signalled(&device, u64::from(trial) - 1),
                    "the release point signalled with a frame still running"
                );
                retirement.lock().expect("the retirement").retired = u64::from(trial);
                let started = Instant::now();
                while !release.signalled(&device, u64::from(trial) - 1) {
                    assert!(started.elapsed() < TIMEOUT, "the release point never signalled");
                    thread::sleep(Duration::from_millis(1));
                }
                worst = worst.max(started.elapsed());
            }
            worst
        },
    ) else {
        return;
    };
    println!("the worst release latency of {TRIALS} idle retirements: {worst:?}");
    assert!(
        worst < PROMPT,
        "a release point waited {worst:?} for an idle loop to notice the GPU had finished"
    );
}
