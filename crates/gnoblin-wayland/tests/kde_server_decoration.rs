//! `org_kde_kwin_server_decoration_manager`: KDE's older decoration negotiation, from a real client.
//!
//! The scenarios are the failure modes listed at the top of `src/protocols/kde_server_decoration.rs`. The protocol
//! is answered by the policy `xdg-decoration` already has, so each test reads what the client was told (`mode` and
//! `default_mode`) and, where a shell is attached, what the shell was told (`decoration_wanted` and `decoration`
//! on the window's node), the same two sides `tests/decoration.rs` reads.

use gnoblin_wayland_testkit::session;

use gnoblin_render::transaction::TreeValue;
use session::shell::Shell;
use session::{Client, Session, Window};
use wayland_protocols_plasma::server_decoration::client::{
    org_kde_kwin_server_decoration::OrgKdeKwinServerDecoration,
    org_kde_kwin_server_decoration_manager::OrgKdeKwinServerDecorationManager,
};

/// The protocol's `mode` values.
const CLIENT: u32 = 1;
const SERVER: u32 = 2;

fn text(value: &str) -> TreeValue {
    TreeValue::Text(value.to_owned())
}

/// Binds the manager and waits for the default mode it announces.
#[track_caller]
fn bind(client: &mut Client) -> (OrgKdeKwinServerDecorationManager, u32) {
    let manager: OrgKdeKwinServerDecorationManager = client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("the session advertises org_kde_kwin_server_decoration_manager");
    client.wait_until("the default mode", |app| app.kde_default_mode.is_some());
    let default = client.state.kde_default_mode.expect("waited for it");
    (manager, default)
}

/// A toplevel the way a Qt 5 or GTK 3 client makes one: the KDE decoration object exists before the first commit.
#[track_caller]
fn decorated_window(
    client: &mut Client,
    manager: &OrgKdeKwinServerDecorationManager,
) -> (Window, OrgKdeKwinServerDecoration) {
    let surface = client.compositor.create_surface(&client.qh, ());
    let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
    let toplevel = xdg_surface.get_toplevel(&client.qh, ());
    toplevel.set_app_id("org.example.kde".to_owned());
    toplevel.set_title("KDE".to_owned());
    let object = manager.create(&surface, &client.qh, ());
    surface.commit();
    client.roundtrip();
    (
        Window {
            surface,
            xdg_surface,
            toplevel,
        },
        object,
    )
}

/// Waits until the client has heard `count` modes and returns every one.
#[track_caller]
fn modes(client: &mut Client, count: usize) -> Vec<u32> {
    client.wait_until(&format!("{count} KDE decoration mode(s)"), |app| {
        app.kde_modes.len() >= count
    });
    client.state.kde_modes.clone()
}

#[test]
fn a_disabled_protocol_is_not_advertised() {
    let mut protocols = gnoblin_config::Protocols::default();
    protocols
        .toggles
        .insert(gnoblin_config::Protocol::KdeServerDecoration, false);
    Session::start_with(protocols).run(|session| {
        let client = Client::connect(session.socket());
        assert!(!client.has_global("org_kde_kwin_server_decoration_manager"));
        assert!(
            client.has_global("zxdg_decoration_manager_v1"),
            "and the other decoration protocol is untouched"
        );
    });
}

#[test]
fn a_client_is_told_client_side_while_no_shell_draws_decorations() {
    Session::start().run(|session| {
        let mut client = Client::connect(session.socket());
        assert!(client.has_global("org_kde_kwin_server_decoration_manager"));
        let (manager, default) = bind(&mut client);
        assert_eq!(default, CLIENT, "nobody draws a frame, so the default is the client's");
        let (_window, object) = decorated_window(&mut client, &manager);
        assert_eq!(modes(&mut client, 1), [CLIENT]);

        // Asking for a frame nobody draws is answered with the client's own, and the answer is sent.
        object.request_mode(SERVER);
        assert_eq!(modes(&mut client, 2), [CLIENT, CLIENT]);
    });
}

#[test]
fn a_shell_that_draws_decorations_makes_it_server_side() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (manager, default) = bind(&mut client);
        assert_eq!(
            default, SERVER,
            "a shell draws a frame, so the server's default is server-side"
        );

        let (window, _object) = decorated_window(&mut client, &manager);
        assert_eq!(
            modes(&mut client, 1),
            [SERVER],
            "a client that asked for nothing wants the server's default"
        );
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        shell.wait_published(node, "decoration", &text("server"));

        // The shell stops drawing: the manager hears the new default and the object is moved to the client's frame.
        shell.declare_decorations(false);
        client.wait_until("the default to change", |app| app.kde_default_mode == Some(CLIENT));
        assert_eq!(modes(&mut client, 2), [SERVER, CLIENT]);
    });
}

#[test]
fn a_request_for_client_side_is_honoured_and_acknowledged() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (manager, _) = bind(&mut client);
        let (window, object) = decorated_window(&mut client, &manager);
        assert_eq!(modes(&mut client, 1), [SERVER]);

        object.request_mode(CLIENT);
        assert_eq!(modes(&mut client, 2), [SERVER, CLIENT], "the request is acknowledged");
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        shell.wait_published(node, "decoration_wanted", &text("client"));

        // Asking for the frame again is the same answer the other way, and an unchanged mode is still acknowledged.
        object.request_mode(CLIENT);
        assert_eq!(modes(&mut client, 3), [SERVER, CLIENT, CLIENT]);
        object.request_mode(SERVER);
        assert_eq!(modes(&mut client, 4), [SERVER, CLIENT, CLIENT, SERVER]);
        shell.wait_published(node, "decoration", &text("server"));
    });
}

#[test]
fn a_request_made_before_the_toplevel_is_applied_to_it() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (manager, _) = bind(&mut client);

        // The decoration is made for a surface that has no role yet, and asks for the client's own frame.
        let surface = client.compositor.create_surface(&client.qh, ());
        let object = manager.create(&surface, &client.qh, ());
        object.request_mode(CLIENT);
        assert_eq!(
            modes(&mut client, 2),
            [SERVER, CLIENT],
            "the object is told the default when it is made and the answer when it asks"
        );

        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        let toplevel = xdg_surface.get_toplevel(&client.qh, ());
        toplevel.set_app_id("org.example.kde".to_owned());
        surface.commit();
        client.roundtrip();
        client.attach_buffer(&surface, 200, 150);
        let node = shell.wait_window_node();
        shell.wait_published(node, "decoration_wanted", &text("client"));
        assert_eq!(
            shell.published(node, "decoration"),
            None,
            "the window keeps the schema's default, `client`"
        );
    });
}
