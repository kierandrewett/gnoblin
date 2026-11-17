//! A layer surface is on the screen: its pixels reach the frame in its layer's place, at the rectangle the protocol or the
//! shell chose, and the pointer reaches it where it is drawn.
//!
//! Everything here is end to end, as `render.rs` is for windows. A real `wayland-client` binds `zwlr_layer_shell_v1` on the
//! privileged socket, answers the compositor's configure with a buffer of the size it was told and one solid colour, and
//! the test reads the frame back through the screenshot path and probes it. No display and no GPU are involved: the
//! software renderer draws every test, and the GLES renderer draws it again where the machine can make a surfaceless
//! context.
//!
//! Every pixel assertion follows a marker. A test that asserts a surface is *not* drawn first waits for something that is
//! drawn after it (another surface committed later), so an absent surface means "the frame had it to draw and did not"
//! and not "the frame was not made yet".

mod session;
mod stock;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_render::transaction::TreeValue;
use gnoblin_types::{Point, Rect};
use gnoblin_wayland::TreeHooks;
use session::shell::Shell;
use session::{App, BTN_LEFT, Client, Handle, LayerWindow, PopupPlace, Session};
use wayland_client::protocol::wl_callback;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

type Layer = zwlr_layer_shell_v1::Layer;
type Anchor = zwlr_layer_surface_v1::Anchor;
type Interactivity = zwlr_layer_surface_v1::KeyboardInteractivity;

/// Red, green, blue, yellow and white in the order a probe reports them.
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
/// Opaque black, the desktop background.
const BACKGROUND: [u8; 4] = [0, 0, 0, 255];

/// A colour as the bytes of an `Argb8888` buffer on a little-endian machine: blue, green, red, alpha.
fn bgra(colour: [u8; 4]) -> [u8; 4] {
    [colour[2], colour[1], colour[0], colour[3]]
}

/// Runs `body` against every renderer this machine offers: the software path always, and the GLES path when a surfaceless
/// EGL context can be made. The same body serving both is what makes a difference between them a bug in one of them.
fn for_each_renderer(body: impl Fn(Headless)) {
    body(Headless::new());
    match Headless::with_config(HeadlessConfig::gles()) {
        Ok(backend) => body(backend),
        Err(error) => println!("skipping the GLES half: {error} (no surfaceless EGL context on this machine)"),
    }
}

/// A session with no shell, whose motion is off so a window that maps is drawn at once and not faded in.
fn still(backend: Headless) -> Session {
    let session = Session::start_on(backend, TreeHooks::default());
    session.control().set_reduced_motion(true);
    session
}

/// A session a shell holds the tree of, with its motion off.
fn shelled(backend: Headless) -> (Shell, Session) {
    let (shell, hooks) = Shell::new();
    let session = Session::start_on(backend, hooks);
    session.control().set_reduced_motion(true);
    (shell, session)
}

/// What a layer surface asks for: the attributes a client sets before its first commit, and the colour it then draws.
struct Spec {
    namespace: &'static str,
    layer: Layer,
    /// The size it asks for, as `set_size` states it: 0 in an axis anchored to both edges is the output's.
    size: (u32, u32),
    anchor: Anchor,
    zone: i32,
    /// Top, right, bottom and left.
    margin: (i32, i32, i32, i32),
    interactivity: Interactivity,
    colour: [u8; 4],
}

impl Spec {
    /// A surface in `layer` with no size, no anchor, no zone and no margin.
    fn new(namespace: &'static str, layer: Layer, colour: [u8; 4]) -> Self {
        Self {
            namespace,
            layer,
            size: (0, 0),
            anchor: Anchor::empty(),
            zone: 0,
            margin: (0, 0, 0, 0),
            interactivity: Interactivity::None,
            colour,
        }
    }

    /// A bar `height` high across the top of the output, which reserves its own height.
    fn bar(namespace: &'static str, layer: Layer, height: u32, colour: [u8; 4]) -> Self {
        Self {
            size: (0, height),
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            zone: i32::try_from(height).expect("a small height"),
            ..Self::new(namespace, layer, colour)
        }
    }

    /// A surface of a fixed `size` against the top left corner of the output, `left` and `top` away from it.
    fn corner(namespace: &'static str, layer: Layer, at: (i32, i32), size: (u32, u32), colour: [u8; 4]) -> Self {
        Self {
            size,
            anchor: Anchor::Top | Anchor::Left,
            margin: (at.1, 0, 0, at.0),
            ..Self::new(namespace, layer, colour)
        }
    }

    /// A surface that covers the whole output.
    fn cover(namespace: &'static str, layer: Layer, colour: [u8; 4]) -> Self {
        Self {
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            zone: -1,
            ..Self::new(namespace, layer, colour)
        }
    }
}

/// The layer shell, which only the privileged socket advertises.
fn layer_shell(client: &mut Client) -> zwlr_layer_shell_v1::ZwlrLayerShellV1 {
    client
        .globals
        .bind(&client.qh, 1..=5, ())
        .expect("the privileged connection sees zwlr_layer_shell_v1")
}

/// Maps a layer surface the way a toolkit does: its attributes and first commit, the configure the compositor answers
/// with, the acknowledgement, and a buffer of the configured size in one colour.
fn map_layer(client: &mut Client, spec: &Spec) -> LayerWindow {
    client.state.layer_size = None;
    client.state.layer_serial = None;
    let shell = layer_shell(client);
    let mut layer = client.layer_surface(&shell, spec.namespace, spec.layer);
    layer.layer_surface.set_size(spec.size.0, spec.size.1);
    layer.layer_surface.set_anchor(spec.anchor);
    layer.layer_surface.set_exclusive_zone(spec.zone);
    layer
        .layer_surface
        .set_margin(spec.margin.0, spec.margin.1, spec.margin.2, spec.margin.3);
    layer.layer_surface.set_keyboard_interactivity(spec.interactivity);
    layer.surface.commit();
    client.wait_until("the first configure", |state| state.layer_size.is_some());
    layer.ack(client);
    let (width, height) = client.state.layer_size.expect("a layer configure arrived");
    draw(client, &layer, (width, height), spec.colour);
    layer
}

/// Attaches a buffer of `size` in one colour to a layer surface and commits it.
fn draw(client: &mut Client, layer: &LayerWindow, size: (u32, u32), colour: [u8; 4]) {
    let (width, height) = (
        i32::try_from(size.0).expect("a small width"),
        i32::try_from(size.1).expect("a small height"),
    );
    let buffer = client.solid_buffer(width, height, bgra(colour));
    layer.surface.attach(Some(&buffer), 0, 0);
    layer.surface.damage(0, 0, width, height);
    layer.surface.commit();
    client.roundtrip();
}

/// Waits for the client's last configure to carry `size`.
#[track_caller]
fn wait_configure(client: &mut Client, size: (u32, u32)) {
    client.wait_until(&format!("a configure of {size:?}"), |state| {
        state.layer_size == Some(size)
    });
}

/// Maps an ordinary window of one colour and puts its frame at `rect`; returns the client that owns it.
fn place_window(session: &Handle, title: &str, rect: Rect, colour: [u8; 4]) -> Client {
    let mut client = Client::connect(session.socket());
    client.map_solid_window(
        "org.gnoblin.Window",
        title,
        (rect.width as i32, rect.height as i32),
        bgra(colour),
    );
    let id = session
        .wait_for(|observation| observation.titled(title).is_some_and(|window| window.mapped))
        .titled(title)
        .expect("the window")
        .id;
    session.control.move_window(id, rect);
    session.wait_for(|observation| observation.titled(title).is_some_and(|window| window.geometry == rect));
    client
}

/// How many frame callbacks a client has been answered.
type Answers = Arc<AtomicU32>;

impl Dispatch<wl_callback::WlCallback, Answers> for App {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        answers: &Answers,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            answers.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn answered(answers: &Answers) -> u32 {
    answers.load(Ordering::SeqCst)
}

/// Creates a popup of a layer surface the way a bar's menu is made, takes its first configure and maps it with a buffer
/// of one colour at the size it asked for.
fn solid_popup(client: &mut Client, bar: &LayerWindow, place: &PopupPlace, colour: [u8; 4]) -> session::Popup {
    client.state.popup_placed = None;
    let positioner = client.positioner(place);
    let surface = client.compositor.create_surface(&client.qh, ());
    let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
    let popup = xdg_surface.get_popup(None, &positioner, &client.qh, ());
    positioner.destroy();
    bar.layer_surface.get_popup(&popup);
    surface.commit();
    client.wait_until("the popup's first configure", |state| state.popup_placed.is_some());
    let (width, height) = place.size;
    let buffer = client.solid_buffer(width, height, bgra(colour));
    surface.attach(Some(&buffer), 0, 0);
    surface.damage(0, 0, width, height);
    surface.commit();
    client.roundtrip();
    session::Popup {
        surface,
        xdg_surface,
        popup,
    }
}

#[test]
fn a_layer_surface_is_drawn_at_the_rectangle_the_protocol_computes() {
    for_each_renderer(|backend| {
        let session = still(backend);
        session.run(|session| {
            let mut client = Client::connect(session.privileged_socket());
            let _bar = map_layer(&mut client, &Spec::bar("org.gnoblin.PixelBar", Layer::Top, 40, RED));

            // A window mapped after the bar is the marker: once its pixels are on the output the frame the bar belongs
            // to has been made, so a bar that is not in it is not drawn and is not merely late.
            let marker = place_window(session, "Marker", Rect::new(300, 300, 100, 100), GREEN);
            session.wait_for_pixel(350, 350, GREEN);

            assert_eq!(session.probe(500, 20), RED, "the bar's pixels are on the output");
            assert_eq!(session.probe(0, 0), RED, "from the first pixel of its rectangle");
            assert_eq!(session.probe(1023, 39), RED, "to the last");
            assert_eq!(session.probe(500, 40), BACKGROUND, "and nothing is drawn below it");
            assert_eq!(session.probe(500, 200), BACKGROUND, "or anywhere else");
            drop(marker);
        });
    });
}

#[test]
fn layer_surfaces_are_drawn_in_the_order_the_protocol_names_around_the_windows() {
    for_each_renderer(|backend| {
        let session = still(backend);
        session.run(|session| {
            // The window sits in the middle of four layer surfaces that each overlap it and the ones next to them.
            let window = place_window(session, "Window", Rect::new(300, 200, 200, 150), BLUE);

            let mut clients: Vec<Client> = (0..4).map(|_| Client::connect(session.privileged_socket())).collect();
            let specs = [
                Spec::cover("org.gnoblin.Background", Layer::Background, GREEN),
                Spec::corner("org.gnoblin.Bottom", Layer::Bottom, (290, 190), (100, 100), YELLOW),
                Spec::corner("org.gnoblin.Top", Layer::Top, (450, 260), (100, 100), RED),
                Spec::corner("org.gnoblin.Overlay", Layer::Overlay, (470, 280), (60, 60), WHITE),
            ];
            let layers: Vec<LayerWindow> = clients
                .iter_mut()
                .zip(&specs)
                .map(|(client, spec)| map_layer(client, spec))
                .collect();
            session.wait_for_pixel(500, 320, WHITE);

            assert_eq!(
                session.probe(10, 10),
                GREEN,
                "the background layer is the bottom of everything"
            );
            assert_eq!(
                session.probe(295, 195),
                YELLOW,
                "the bottom layer is above the background"
            );
            assert_eq!(session.probe(350, 250), BLUE, "a window is above the bottom layer");
            assert_eq!(session.probe(400, 300), BLUE, "and shows where nothing is above it");
            assert_eq!(session.probe(460, 270), RED, "the top layer is above a window");
            assert_eq!(session.probe(520, 350), RED, "and shows past the window's edge");
            assert_eq!(
                session.probe(480, 290),
                WHITE,
                "the overlay is above the top layer and the window"
            );
            assert_eq!(
                session.probe(525, 335),
                WHITE,
                "and where only the top layer lies under it"
            );
            drop((layers, window));
        });
    });
}

#[test]
fn a_layer_surface_the_shell_places_is_drawn_at_the_shells_rectangle_and_nowhere_else() {
    for_each_renderer(|backend| {
        let (shell, session) = shelled(backend);
        session.run(move |session| {
            shell.attach();
            let mut client = Client::connect(session.privileged_socket());
            let mut layer = map_layer(&mut client, &Spec::bar("org.gnoblin.PixelBar", Layer::Top, 40, RED));
            let node = shell.wait_surface_node("layer");
            session.wait_for_pixel(500, 20, RED);

            // The shell puts the bar in the middle of the output at a size of its own. The client is configured at that
            // size and draws it; until it has, the surface keeps the pixels it last committed.
            shell.write(
                node,
                &[
                    ("placement_override", TreeValue::Bool(true)),
                    ("x", TreeValue::Double(200.0)),
                    ("y", TreeValue::Double(300.0)),
                    ("width", TreeValue::Double(400.0)),
                    ("height", TreeValue::Double(60.0)),
                ],
            );
            wait_configure(&mut client, (400, 60));
            layer.ack(&mut client);
            draw(&mut client, &layer, (400, 60), RED);

            session.wait_for_pixel(400, 330, RED);
            assert_eq!(
                session.probe(200, 300),
                RED,
                "the shell's rectangle starts where the shell put it"
            );
            assert_eq!(session.probe(599, 359), RED, "and ends where its size ends");
            assert_eq!(session.probe(600, 330), BACKGROUND, "nothing is drawn right of it");
            assert_eq!(session.probe(400, 361), BACKGROUND, "or below it");
            assert_eq!(
                session.probe(500, 20),
                BACKGROUND,
                "and the strip the protocol put it in is empty again"
            );

            // Moving it again leaves nothing at the place it left.
            shell.write(node, &[("x", TreeValue::Double(0.0)), ("y", TreeValue::Double(700.0))]);
            session.wait_for_pixel(100, 720, RED);
            assert_eq!(session.probe(400, 330), BACKGROUND, "the old rectangle is repainted");
        });
    });
}

#[test]
fn the_pointer_reaches_a_layer_surface_where_it_is_drawn_and_not_the_window_beneath() {
    let session = still(Headless::new());
    session.run(|session| {
        // A window whose top edge is under the bar's strip, and a bar above it.
        let mut app = place_window(session, "Window", Rect::new(300, 20, 200, 150), BLUE);
        let mut client = Client::connect(session.privileged_socket());
        let _bar = map_layer(&mut client, &Spec::bar("org.gnoblin.PixelBar", Layer::Top, 40, RED));
        session.wait_for_pixel(400, 30, RED);

        // A press where the bar covers the window is the bar's.
        session.click(Point::new(400, 30));
        client.wait_until("the bar's button", |state| state.pointer_buttons.len() >= 2);
        assert_eq!(
            client.state.pointer_buttons,
            [(BTN_LEFT, true), (BTN_LEFT, false)],
            "the bar heard the press and the release"
        );
        assert_eq!(
            client.state.pointer_at,
            Some((400.0, 30.0)),
            "at the point of the surface under the pointer"
        );
        app.roundtrip();
        assert_eq!(
            app.state.pointer_enters, 0,
            "the window under the bar never had the pointer"
        );
        assert!(app.state.pointer_buttons.is_empty(), "and heard no button");

        // A press on the window below the bar's strip is the window's.
        session.click(Point::new(400, 100));
        app.wait_until("the window's button", |state| state.pointer_buttons.len() >= 2);
        client.wait_until("the bar to lose the pointer", |state| state.pointer_leaves >= 1);
        assert_eq!(
            client.state.pointer_buttons.len(),
            2,
            "the bar heard nothing of a press that was not on it"
        );
    });
}

/// Waits long enough for an event the compositor means to send to have arrived, and checks that it has not.
#[track_caller]
fn assert_quiet(client: &mut Client, what: &str, quiet: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        client.roundtrip();
        assert!(quiet(&client.state), "{what}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_popup_of_a_bottom_layer_surface_is_drawn_above_the_windows() {
    let session = still(Headless::new());
    session.run(|session| {
        // The dock is on the bottom layer, under the window; its menu opens down and right over the window.
        let _window = place_window(session, "Window", Rect::new(300, 200, 200, 150), BLUE);
        let mut client = Client::connect(session.privileged_socket());
        let dock = map_layer(
            &mut client,
            &Spec::corner("org.gnoblin.Dock", Layer::Bottom, (250, 150), (300, 40), YELLOW),
        );
        let _menu = solid_popup(&mut client, &dock, &PopupPlace::at(50, 50, (80, 80)), GREEN);

        session.wait_for_pixel(340, 240, GREEN);
        assert_eq!(session.probe(260, 160), YELLOW, "the dock is drawn under the window");
        assert_eq!(
            session.probe(450, 320),
            BLUE,
            "and the window shows where nothing covers it"
        );
        assert_eq!(
            session.probe(379, 279),
            GREEN,
            "the menu is above the window to its last pixel, 50 right and 50 below the dock's corner"
        );
        assert_eq!(session.probe(380, 280), BLUE, "and ends there");
    });
}

#[test]
fn a_popup_of_a_layer_surface_follows_the_surface_the_shell_moves() {
    let (shell, session) = shelled(Headless::new());
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.privileged_socket());
        let bar = map_layer(
            &mut client,
            &Spec::corner("org.gnoblin.Bar", Layer::Top, (100, 100), (300, 40), YELLOW),
        );
        let node = shell.wait_surface_node("layer");
        let _menu = solid_popup(&mut client, &bar, &PopupPlace::at(50, 50, (80, 80)), GREEN);
        session.wait_for_pixel(190, 190, GREEN);

        // The shell puts the bar somewhere else at the size it has: the menu is drawn the same distance from its corner.
        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", TreeValue::Double(400.0)),
                ("y", TreeValue::Double(300.0)),
                ("width", TreeValue::Double(300.0)),
                ("height", TreeValue::Double(40.0)),
            ],
        );
        session.wait_for_pixel(490, 390, GREEN);
        assert_eq!(session.probe(410, 310), YELLOW, "the bar is at the shell's rectangle");
        assert_eq!(session.probe(190, 190), BACKGROUND, "the menu left where it was");
        assert_eq!(session.probe(110, 110), BACKGROUND, "and so did the bar");
    });
}

#[test]
fn a_layer_surface_is_answered_its_frame_callbacks_until_the_shell_hides_it() {
    let (shell, session) = shelled(Headless::new());
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.privileged_socket());
        let bar = map_layer(&mut client, &Spec::bar("org.gnoblin.Bar", Layer::Top, 40, RED));
        let node = shell.wait_surface_node("layer");
        let answers: Answers = Arc::default();

        // A toolkit asks for a callback and draws; the compositor answers it with the frame that shows the surface.
        bar.surface.frame(&client.qh, Arc::clone(&answers));
        draw(&mut client, &bar, (1024, 40), RED);
        client.wait_until("the first frame callback", |_| answered(&answers) == 1);

        // A surface the shell hid is not shown, so its client is left waiting.
        shell.write(node, &[("visible", TreeValue::Bool(false))]);
        session.wait_for_pixel(500, 20, BACKGROUND);
        bar.surface.frame(&client.qh, Arc::clone(&answers));
        draw(&mut client, &bar, (1024, 40), RED);
        assert_quiet(&mut client, "a hidden surface was answered a frame", |_| {
            answered(&answers) == 1
        });

        // Shown again, it is answered the callback it was owed.
        shell.write(node, &[("visible", TreeValue::Bool(true))]);
        client.wait_until("the owed frame callback", |_| answered(&answers) == 2);
    });
}

#[test]
fn a_layer_surface_the_shell_hides_is_not_drawn_and_takes_no_pointer() {
    let (shell, session) = shelled(Headless::new());
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.privileged_socket());
        let _bar = map_layer(&mut client, &Spec::bar("org.gnoblin.Bar", Layer::Top, 40, RED));
        let node = shell.wait_surface_node("layer");
        session.wait_for_pixel(500, 20, RED);

        shell.write(node, &[("visible", TreeValue::Bool(false))]);
        session.wait_for_pixel(500, 20, BACKGROUND);
        session.click(Point::new(500, 20));
        assert_quiet(&mut client, "a hidden surface heard the pointer", |state| {
            state.pointer_enters == 0
        });

        shell.write(node, &[("visible", TreeValue::Bool(true))]);
        session.wait_for_pixel(500, 20, RED);
    });
}

#[test]
fn a_layer_surface_that_unmaps_or_goes_leaves_no_pixels_behind() {
    let session = still(Headless::new());
    session.run(|session| {
        let mut bar_client = Client::connect(session.privileged_socket());
        let bar = map_layer(&mut bar_client, &Spec::bar("org.gnoblin.Bar", Layer::Top, 40, RED));
        let mut note_client = Client::connect(session.privileged_socket());
        let note = map_layer(
            &mut note_client,
            &Spec::corner("org.gnoblin.Note", Layer::Overlay, (500, 300), (100, 100), BLUE),
        );
        session.wait_for_pixel(550, 350, BLUE);
        assert_eq!(session.probe(500, 20), RED);

        // A client that attaches no buffer has unmapped its surface: nothing of it is left on the output.
        bar.surface.attach(None, 0, 0);
        bar.surface.commit();
        bar_client.roundtrip();
        session.wait_for_pixel(500, 20, BACKGROUND);
        assert_eq!(session.probe(550, 350), BLUE, "the other surface is not affected");

        note.layer_surface.destroy();
        note.surface.destroy();
        note_client.roundtrip();
        session.wait_for_pixel(550, 350, BACKGROUND);
    });
}

#[test]
fn a_click_gives_the_keyboard_to_a_surface_that_asked_for_it_on_demand_and_a_click_elsewhere_takes_it_back() {
    let session = still(Headless::new());
    session.run(|session| {
        let mut app = place_window(session, "Window", Rect::new(300, 300, 200, 150), BLUE);
        let mut client = Client::connect(session.privileged_socket());
        let mut spec = Spec::bar("org.gnoblin.Launcher", Layer::Top, 40, RED);
        spec.interactivity = Interactivity::OnDemand;
        let _bar = map_layer(&mut client, &spec);
        session.wait_for_pixel(500, 20, RED);

        // The default session never focuses a window that maps, so the window has the keyboard once it is clicked.
        session.click(Point::new(400, 350));
        app.wait_until("the window's keyboard", |state| state.keyboard_enters >= 1);
        assert_eq!(
            client.state.keyboard_enters, 0,
            "the bar has no keyboard before it is clicked"
        );

        session.click(Point::new(500, 20));
        client.wait_until("the bar's keyboard", |state| state.keyboard_enters >= 1);

        session.click(Point::new(400, 350));
        app.wait_until("the window's keyboard again", |state| state.keyboard_enters >= 2);
        client.roundtrip();
        assert_eq!(client.state.keyboard_enters, 1, "the bar had the keyboard once");
    });
}

#[test]
fn a_second_bar_is_placed_against_the_first_and_a_surface_with_no_zone_avoids_both() {
    let session = still(Headless::new());
    session.run(|session| {
        let mut first = Client::connect(session.privileged_socket());
        let mut second = Client::connect(session.privileged_socket());
        let mut third = Client::connect(session.privileged_socket());
        let _a = map_layer(&mut first, &Spec::bar("org.gnoblin.First", Layer::Top, 30, RED));
        let _b = map_layer(&mut second, &Spec::bar("org.gnoblin.Second", Layer::Top, 20, BLUE));
        // A notification with no zone of its own, against the top left corner.
        let _c = map_layer(
            &mut third,
            &Spec::corner("org.gnoblin.Notice", Layer::Top, (0, 0), (100, 50), GREEN),
        );

        session.wait_for_pixel(50, 75, GREEN);
        assert_eq!(session.probe(500, 10), RED, "the first bar is against the top edge");
        assert_eq!(
            session.probe(500, 40),
            BLUE,
            "the second sits against the first and not over it"
        );
        assert_eq!(session.probe(500, 55), BACKGROUND, "and nothing is drawn below the two");
        assert_eq!(
            session.probe(50, 49),
            BLUE,
            "the notification is placed in what the bars left"
        );
        assert_eq!(session.probe(99, 99), GREEN, "to its last pixel");
        assert_eq!(session.probe(100, 99), BACKGROUND, "and no further");
    });
}
