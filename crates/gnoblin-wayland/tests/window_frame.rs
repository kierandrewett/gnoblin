//! `gnoblin_window_frame_v1` against real clients: a renderer and the app whose
//! window it decorates.
//!
//! The scenarios are the C suite's (`tests/frame-bind-no-buffer.c`,
//! `frame-renderer-policy.test.py` and `window-frames-input.js`) and the rules in
//! `src/protocols/gnoblin_window_frame_v1.rs`, in order. Two connections are used the way the
//! real arrangement has them: the renderer is promised `ClientRole::FrameRenderer`
//! and the app is an ordinary client that maps one window.

mod owned;

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_types::{Rect, WindowId};
use gnoblin_wayland::{ClientRole, GnoblinWindowFrameV1Action};
use owned::protocols::gnoblin_window_frame_v1::gnoblin_window_frame_manager_v1;
use owned::protocols::gnoblin_window_frame_v1::gnoblin_window_frame_v1;
use owned::{Connected, Handle, Session, TIMEOUT};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// One configure, as the renderer received it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Configure {
    serial: u32,
    width: i32,
    height: i32,
    extents: [i32; 4],
    state: u32,
    title: String,
    app_id: String,
    style: String,
}

/// What one connection records.
#[derive(Default)]
struct App {
    manager: Option<gnoblin_window_frame_manager_v1::GnoblinWindowFrameManagerV1>,
    frames: Vec<gnoblin_window_frame_v1::GnoblinWindowFrameV1>,
    configures: Vec<Configure>,
    interactions: Vec<(u32, bool)>,
    closed: u32,
}

/// The renderer's half: it binds the manager and drives a frame.
struct Renderer {
    client: Connected<App>,
    surface: Option<wl_surface::WlSurface>,
}

impl Renderer {
    fn start(handle: &Handle) -> Self {
        let mut client = Connected::connect(handle.socket(), App::default());
        let manager: gnoblin_window_frame_manager_v1::GnoblinWindowFrameManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("a renderer sees its own protocol");
        client.state.manager = Some(manager);
        client.roundtrip();
        Self { client, surface: None }
    }

    fn frame(&self) -> &gnoblin_window_frame_v1::GnoblinWindowFrameV1 {
        self.client.state.frames.first().expect("a frame was offered")
    }

    /// Attaches a fresh surface and returns it.
    fn attach(&mut self) -> wl_surface::WlSurface {
        let surface = self.client.compositor.create_surface(&self.client.qh, ());
        self.frame().attach_surface(&surface);
        self.client.roundtrip();
        self.surface = Some(surface.clone());
        surface
    }

    /// Acknowledges the last configure and commits a buffer of that size, without a
    /// round trip, for a commit the server is expected to refuse.
    fn present_invalid(&mut self, size: (i32, i32)) {
        let serial = self.client.state.configures.last().expect("a configure arrived").serial;
        let buffer = self.client.buffer(size.0, size.1);
        let surface = self.surface.clone().expect("a surface is attached");
        self.frame().ack_configure(serial);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, size.0, size.1);
        surface.commit();
        self.client.connection.flush().expect("the commit reaches the server");
    }

    /// Acknowledges the last configure and commits a buffer of that size.
    fn present(&mut self, size: (i32, i32)) {
        let serial = self.client.state.configures.last().expect("a configure arrived").serial;
        let buffer = self.client.buffer(size.0, size.1);
        let surface = self.surface.clone().expect("a surface is attached");
        self.frame().ack_configure(serial);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, size.0, size.1);
        surface.commit();
        self.client.roundtrip();
    }
}

impl Dispatch<gnoblin_window_frame_manager_v1::GnoblinWindowFrameManagerV1, ()> for App {
    wayland_client::event_created_child!(App, gnoblin_window_frame_manager_v1::GnoblinWindowFrameManagerV1, [
        gnoblin_window_frame_manager_v1::EVT_FRAME_OPCODE =>
            (gnoblin_window_frame_v1::GnoblinWindowFrameV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _manager: &gnoblin_window_frame_manager_v1::GnoblinWindowFrameManagerV1,
        event: gnoblin_window_frame_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let gnoblin_window_frame_manager_v1::Event::Frame { frame } = event;
        state.frames.push(frame);
    }
}

impl Dispatch<gnoblin_window_frame_v1::GnoblinWindowFrameV1, ()> for App {
    fn event(
        state: &mut Self,
        _frame: &gnoblin_window_frame_v1::GnoblinWindowFrameV1,
        event: gnoblin_window_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            gnoblin_window_frame_v1::Event::Configure {
                serial,
                width,
                height,
                top,
                right,
                bottom,
                left,
                state: flags,
                title,
                app_id,
                style,
            } => state.configures.push(Configure {
                serial,
                width,
                height,
                extents: [top, right, bottom, left],
                state: match flags {
                    wayland_client::WEnum::Value(flags) => flags.bits(),
                    wayland_client::WEnum::Unknown(raw) => raw,
                },
                title,
                app_id,
                style,
            }),
            gnoblin_window_frame_v1::Event::Interaction { action, pressed } => {
                state.interactions.push((action, pressed != 0));
            }
            gnoblin_window_frame_v1::Event::Closed => state.closed += 1,
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _state: &mut Self,
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
        _state: &mut Self,
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

impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for App {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(App: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wayland_client::protocol::wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);

/// An ordinary client that maps one window, titled `decorated`.
fn map_app_window(handle: &Handle) -> Connected<App> {
    map_titled_window(handle, "decorated")
}

/// An ordinary client that maps one window with the given title, buffer 200 by 150.
fn map_titled_window(handle: &Handle, title: &str) -> Connected<App> {
    map_sized_window(handle, title, (200, 150))
}

/// An ordinary client that maps one window with the given title and buffer size.
fn map_sized_window(handle: &Handle, title: &str, size: (i32, i32)) -> Connected<App> {
    let mut client = Connected::connect(handle.socket(), App::default());
    let wm_base: xdg_wm_base::XdgWmBase = client
        .globals
        .bind(&client.qh, 1..=5, ())
        .expect("every session advertises xdg_wm_base");
    let surface = client.compositor.create_surface(&client.qh, ());
    let xdg = wm_base.get_xdg_surface(&surface, &client.qh, ());
    let toplevel = xdg.get_toplevel(&client.qh, ());
    toplevel.set_title(title.to_owned());
    toplevel.set_app_id("org.example.Decorated".to_owned());
    surface.commit();
    client.roundtrip();
    let buffer = client.buffer(size.0, size.1);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, size.0, size.1);
    surface.commit();
    client.roundtrip();
    client
}

/// Waits until the renderer's connection saw `predicate`.
fn wait_renderer(renderer: &mut Renderer, what: &str, predicate: impl Fn(&App) -> bool) {
    renderer.client.wait_until(what, predicate);
}

/// The window a test's app mapped; identities are handed out in creation order.
fn window() -> WindowId {
    WindowId::new(1)
}

#[test]
fn an_ordinary_client_sees_no_frame_protocol() {
    let session = Session::start(None);
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        client.roundtrip();
        assert!(
            !client.has("gnoblin_window_frame_manager_v1"),
            "an application must not be offered the frame protocol"
        );
    });
}

#[test]
fn a_renderer_is_offered_only_its_own_protocol() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut client = Connected::connect(handle.socket(), App::default());
        client.roundtrip();
        assert!(client.has("gnoblin_window_frame_manager_v1"));
        for hidden in [
            "gnoblin_session_host_v1",
            "zwlr_layer_shell_v1",
            "ext_data_control_manager_v1",
            "zwlr_screencopy_manager_v1",
        ] {
            assert!(!client.has(hidden), "a renderer must not see {hidden}");
        }
    });
}

#[test]
fn a_mapped_window_is_offered_a_frame_and_presents_it() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());

        let surface = renderer.attach();
        wait_renderer(&mut renderer, "a configure", |state| !state.configures.is_empty());
        let configure = renderer.client.state.configures[0].clone();
        assert_eq!(
            (configure.width, configure.height),
            (200, 150),
            "the frame is configured to the window's geometry"
        );
        assert_eq!(configure.extents, [0; 4], "this slice computes no extents");
        assert_eq!(configure.title, "decorated");
        assert_eq!(configure.app_id, "org.example.Decorated");
        assert_eq!(configure.style, "");
        // Focus, and every action this slice allows on a normal window.
        let expected = gnoblin_window_frame_v1::State::Focused
            | gnoblin_window_frame_v1::State::CanClose
            | gnoblin_window_frame_v1::State::CanMaximize
            | gnoblin_window_frame_v1::State::CanMinimize
            | gnoblin_window_frame_v1::State::CanResize;
        assert_eq!(configure.state, expected.bits());

        // A matching buffer with one region is presented.
        let _ = surface;
        renderer
            .frame()
            .region(gnoblin_window_frame_v1::Action::Close, 0, 0, 40, 20);
        renderer.present((200, 150));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let frame = handle
                .control
                .window_frame(window())
                .flatten()
                .expect("the window has a frame");
            if frame.presented {
                assert_eq!(frame.regions.len(), 1);
                assert_eq!(frame.regions[0].action, GnoblinWindowFrameV1Action::Close);
                assert_eq!(frame.serial, configure.serial);
                break;
            }
            assert!(Instant::now() < deadline, "the frame was never presented");
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn attaching_a_roled_surface_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        // A surface that already has the renderer's own xdg role cannot become a
        // frame: a Wayland surface has one role for its life.
        let wm_base: xdg_wm_base::XdgWmBase = renderer
            .client
            .globals
            .bind(&renderer.client.qh, 1..=5, ())
            .expect("xdg_wm_base");
        let roled = renderer.client.compositor.create_surface(&renderer.client.qh, ());
        let xdg = wm_base.get_xdg_surface(&roled, &renderer.client.qh, ());
        let toplevel = xdg.get_toplevel(&renderer.client.qh, ());
        toplevel.set_app_id("org.example.Renderer".to_owned());
        roled.commit();
        renderer.client.roundtrip();
        let buffer = renderer.client.buffer(64, 64);
        roled.attach(Some(&buffer), 0, 0);
        roled.damage(0, 0, 64, 64);
        roled.commit();
        renderer.client.roundtrip();
        renderer.frame().attach_surface(&roled);
        renderer.client.expect_protocol_error("gnoblin_window_frame_v1", 0);
    });
}

#[test]
fn a_second_surface_for_one_frame_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _first = renderer.attach();
        let second = renderer.client.compositor.create_surface(&renderer.client.qh, ());
        renderer.frame().attach_surface(&second);
        renderer.client.expect_protocol_error("gnoblin_window_frame_v1", 0);
    });
}

#[test]
fn acknowledging_an_unknown_serial_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _surface = renderer.attach();
        wait_renderer(&mut renderer, "a configure", |state| !state.configures.is_empty());
        let serial = renderer.client.state.configures[0].serial;
        renderer.frame().ack_configure(serial + 1);
        renderer.client.expect_protocol_error("gnoblin_window_frame_v1", 1);
    });
}

#[test]
fn a_buffer_of_the_wrong_size_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _surface = renderer.attach();
        wait_renderer(&mut renderer, "a configure", |state| !state.configures.is_empty());
        renderer.present_invalid((100, 100));
        renderer.client.expect_protocol_error("gnoblin_window_frame_v1", 1);
    });
}

#[test]
fn an_out_of_range_region_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _surface = renderer.attach();
        renderer
            .frame()
            .region(gnoblin_window_frame_v1::Action::Close, 0, 0, 40000, 20);
        renderer.client.expect_protocol_error("gnoblin_window_frame_v1", 1);
    });
}

#[test]
fn more_than_sixty_four_regions_is_a_protocol_error() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _surface = renderer.attach();
        for index in 0..64 {
            renderer
                .frame()
                .region(gnoblin_window_frame_v1::Action::Close, 0, index, 10, 1);
        }
        renderer
            .frame()
            .region(gnoblin_window_frame_v1::Action::Close, 0, 64, 10, 1);
        renderer.client.expect_protocol_error("gnoblin_window_frame_v1", 1);
    });
}

#[test]
fn hovering_a_region_is_reported_to_the_renderer() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _surface = renderer.attach();
        wait_renderer(&mut renderer, "a configure", |state| !state.configures.is_empty());
        renderer
            .frame()
            .region(gnoblin_window_frame_v1::Action::Close, 0, 0, 40, 20);
        renderer.present((200, 150));

        // The frame covers the window, so a global position inside a region is a
        // frame-local one too once the window is placed.
        wait_presented(handle, window());
        handle.control.move_window(window(), Rect::new(50, 50, 200, 150));
        handle.control.inject_input(gnoblin_backend::InputEvent::PointerMotion {
            output: gnoblin_types::OutputId::new(0),
            position: gnoblin_types::Point::new(60, 60),
        });
        wait_renderer(&mut renderer, "a hovered region", |state| {
            state
                .interactions
                .iter()
                .any(|(action, pressed)| *action == 2 && !*pressed)
        });
        // A press in the same region is reported as pressed.
        handle.control.inject_input(gnoblin_backend::InputEvent::PointerButton {
            button: 0x110,
            pressed: true,
        });
        wait_renderer(&mut renderer, "a pressed region", |state| {
            state
                .interactions
                .iter()
                .any(|(action, pressed)| *action == 2 && *pressed)
        });
    });
}

/// Attaches, configures and presents the frame of the window titled `title`, with one
/// region at frame-local `(x, y, width, height)` and a 200 by 150 buffer.
fn present_titled(
    renderer: &mut Renderer,
    frame: &gnoblin_window_frame_v1::GnoblinWindowFrameV1,
    title: &str,
    action: gnoblin_window_frame_v1::Action,
    region: (i32, i32, i32, i32),
) {
    present_titled_sized(renderer, frame, title, action, region, (200, 150));
}

/// As `present_titled`, with a buffer of `size`.
fn present_titled_sized(
    renderer: &mut Renderer,
    frame: &gnoblin_window_frame_v1::GnoblinWindowFrameV1,
    title: &str,
    action: gnoblin_window_frame_v1::Action,
    region: (i32, i32, i32, i32),
    size: (i32, i32),
) {
    let surface = renderer.client.compositor.create_surface(&renderer.client.qh, ());
    frame.attach_surface(&surface);
    renderer.client.roundtrip();
    wait_renderer(renderer, "a configure for the frame", |state| {
        state.configures.iter().any(|configure| configure.title == title)
    });
    let serial = renderer
        .client
        .state
        .configures
        .iter()
        .rev()
        .find(|configure| configure.title == title)
        .expect("the window's frame was configured")
        .serial;
    frame.region(action, region.0, region.1, region.2, region.3);
    let buffer = renderer.client.buffer(size.0, size.1);
    frame.ack_configure(serial);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, size.0, size.1);
    surface.commit();
    renderer.client.roundtrip();
}

/// Waits until the frame of `window` is presented.
fn wait_presented(handle: &Handle, window: WindowId) {
    let deadline = Instant::now() + TIMEOUT;
    while handle
        .control
        .window_frame(window)
        .flatten()
        .is_none_or(|frame| !frame.presented)
    {
        assert!(Instant::now() < deadline, "a frame was never presented");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_pointer_over_overlapping_windows_reaches_the_topmost_frame() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        // The second window is mapped last, so it is the topmost of the two.
        let _bottom = map_titled_window(handle, "bottom");
        wait_renderer(&mut renderer, "the first frame", |state| state.frames.len() == 1);
        let _top = map_titled_window(handle, "top");
        wait_renderer(&mut renderer, "the second frame", |state| state.frames.len() == 2);
        let bottom_frame = renderer.client.state.frames[0].clone();
        let top_frame = renderer.client.state.frames[1].clone();
        // Both frames cover their windows. The bottom frame's region covers the whole
        // window, so a pointer that reached it would report Minimize, not Close.
        present_titled(
            &mut renderer,
            &bottom_frame,
            "bottom",
            gnoblin_window_frame_v1::Action::Minimize,
            (0, 0, 200, 150),
        );
        present_titled(
            &mut renderer,
            &top_frame,
            "top",
            gnoblin_window_frame_v1::Action::Close,
            (0, 0, 40, 20),
        );
        wait_presented(handle, WindowId::new(1));
        wait_presented(handle, WindowId::new(2));

        handle.control.move_window(WindowId::new(1), Rect::new(0, 0, 200, 150));
        handle
            .control
            .move_window(WindowId::new(2), Rect::new(50, 50, 200, 150));
        // (60, 60) lies in both windows. In the top window that is frame-local (10, 10),
        // inside its Close region; the bottom window's Minimize region covers it too.
        handle.control.inject_input(gnoblin_backend::InputEvent::PointerMotion {
            output: gnoblin_types::OutputId::new(0),
            position: gnoblin_types::Point::new(60, 60),
        });
        wait_renderer(&mut renderer, "a hovered region", |state| {
            state
                .interactions
                .iter()
                .any(|(action, pressed)| *action == 2 && !*pressed)
        });
        assert!(
            !renderer
                .client
                .state
                .interactions
                .iter()
                .any(|(action, _)| *action == 4),
            "the window underneath must not receive the pointer"
        );
    });
}

#[test]
fn a_pointer_over_a_tiled_window_reaches_its_own_frame() {
    let session = Session::start_tiled(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        // Master tiling puts the first window on the left half of the output and the
        // second on the right, so the second window starts at x 512.
        let _left = map_sized_window(handle, "left", (512, 768));
        wait_renderer(&mut renderer, "the first frame", |state| state.frames.len() == 1);
        let _right = map_sized_window(handle, "right", (512, 768));
        wait_renderer(&mut renderer, "the second frame", |state| state.frames.len() == 2);
        let left_frame = renderer.client.state.frames[0].clone();
        let right_frame = renderer.client.state.frames[1].clone();
        // The left frame's region covers its whole window, so a pointer that reached it
        // would report Minimize, which the test forbids.
        present_titled_sized(
            &mut renderer,
            &left_frame,
            "left",
            gnoblin_window_frame_v1::Action::Minimize,
            (0, 0, 512, 768),
            (512, 768),
        );
        present_titled_sized(
            &mut renderer,
            &right_frame,
            "right",
            gnoblin_window_frame_v1::Action::Close,
            (80, 90, 40, 20),
            (512, 768),
        );
        wait_presented(handle, WindowId::new(1));
        wait_presented(handle, WindowId::new(2));
        // (600, 100) is frame-local (88, 100) in the right window: inside its Close region,
        // and outside the left window altogether.
        handle.control.inject_input(gnoblin_backend::InputEvent::PointerMotion {
            output: gnoblin_types::OutputId::new(0),
            position: gnoblin_types::Point::new(600, 100),
        });
        wait_renderer(&mut renderer, "a hovered region", |state| {
            state
                .interactions
                .iter()
                .any(|(action, pressed)| *action == 2 && !*pressed)
        });
        assert!(
            !renderer
                .client
                .state
                .interactions
                .iter()
                .any(|(action, _)| *action == 4),
            "the left window underneath must not receive the pointer"
        );
    });
}

#[test]
fn a_renderer_crash_closes_its_frames() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let _app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _surface = renderer.attach();
        let deadline = Instant::now() + TIMEOUT;
        while handle.control.window_frame(window()).flatten().is_none() {
            assert!(Instant::now() < deadline, "the frame was never created");
            thread::sleep(Duration::from_millis(5));
        }
        // The renderer's connection goes away without destroying anything.
        drop(renderer);
        let deadline = Instant::now() + TIMEOUT;
        while handle.control.window_frame(window()).flatten().is_some() {
            assert!(Instant::now() < deadline, "the crashed renderer left a frame behind");
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn a_closed_window_closes_its_frame() {
    let session = Session::start(Some(ClientRole::FrameRenderer));
    session.run(|handle| {
        let mut renderer = Renderer::start(handle);
        let app = map_app_window(handle);
        wait_renderer(&mut renderer, "a frame", |state| !state.frames.is_empty());
        let _surface = renderer.attach();
        drop(app);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            renderer.client.roundtrip();
            if renderer.client.state.closed >= 1 {
                break;
            }
            assert!(Instant::now() < deadline, "the renderer never heard `closed`");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            handle.control.window_frame(window()).flatten().is_none(),
            "a closed window keeps no frame"
        );
    });
}
