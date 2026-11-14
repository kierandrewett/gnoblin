//! `xdg-decoration`: who draws a window's frame. The compositor tells a client that asks for server-side that the
//! server draws only while a shell that holds the tree has declared that it draws decorations (the main seat's
//! `decorations_available`), and publishes the ask and the answer on the window's node (`decoration_wanted`,
//! `decoration`). Every test runs a real client over a real socket and reads what the client was told and what the
//! shell was told.

mod session;

use gnoblin_render::transaction::TreeValue;
use session::shell::Shell;
use session::{Client, Session};
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1::Mode;

fn text(value: &str) -> TreeValue {
    TreeValue::Text(value.to_owned())
}

#[test]
fn with_no_shell_a_client_that_asks_for_server_side_is_told_client_side() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (_window, decoration) = client.create_decorated_window("org.example.gtk", "GTK", Some(Mode::ServerSide));
        assert_eq!(
            client.wait_decoration_modes(&decoration, 1),
            vec![Mode::ClientSide],
            "nobody draws a frame, so the client must keep its own: a window with neither would have no way to be moved"
        );
    });
}

#[test]
fn a_shell_that_holds_the_tree_but_has_not_declared_decorations_leaves_clients_client_side() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_window, decoration) = client.create_decorated_window("org.example.gtk", "GTK", Some(Mode::ServerSide));
        assert_eq!(client.wait_decoration_modes(&decoration, 1), vec![Mode::ClientSide]);
    });
}

#[test]
fn a_client_that_asks_for_server_side_is_told_server_side_while_the_shell_declares_decorations() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (window, decoration) = client.create_decorated_window("org.example.gtk", "GTK", Some(Mode::ServerSide));
        assert_eq!(
            client.wait_decoration_modes(&decoration, 1),
            vec![Mode::ServerSide],
            "the first configure already carries the answer"
        );
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        shell.wait_published(node, "decoration", &text("server"));
        shell.wait_published(node, "decoration_wanted", &text("server"));
    });
}

#[test]
fn a_client_that_asks_for_client_side_is_told_client_side_even_when_the_shell_draws() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (window, decoration) = client.create_decorated_window("org.example.foot", "foot", Some(Mode::ClientSide));
        assert_eq!(client.wait_decoration_modes(&decoration, 1), vec![Mode::ClientSide]);
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        shell.wait_published(node, "decoration_wanted", &text("client"));
        assert_eq!(
            shell.published(node, "decoration"),
            None,
            "the window keeps the schema's default, `client`: nothing changed for the shell to hear"
        );
    });
}

#[test]
fn a_client_that_states_no_preference_is_told_client_side_and_is_unset() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (window, decoration) = client.create_decorated_window("org.example.qt", "Qt", None);
        assert_eq!(client.wait_decoration_modes(&decoration, 1), vec![Mode::ClientSide]);
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        assert_eq!(shell.published(node, "decoration"), None);
        assert_eq!(
            shell.published(node, "decoration_wanted"),
            None,
            "unset is the schema's default"
        );
    });
}

#[test]
fn a_window_whose_client_never_binds_the_manager_draws_its_own_frame() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let _window = client.map_window("org.example.plain", "Plain");
        let node = shell.wait_window_node();
        assert_eq!(shell.published(node, "decoration"), None, "`client`, the default");
        assert_eq!(shell.published(node, "decoration_wanted"), None, "`unset`, the default");
        assert!(
            client.state.decoration_modes.is_empty(),
            "a client that never bound the manager was told nothing"
        );
    });
}

#[test]
fn a_shell_that_turns_decorations_off_and_on_sends_every_server_side_window_the_new_answer() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (window, decoration) = client.create_decorated_window("org.example.gtk", "GTK", Some(Mode::ServerSide));
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        assert_eq!(client.wait_decoration_modes(&decoration, 1), vec![Mode::ServerSide]);
        shell.wait_published(node, "decoration", &text("server"));

        // A reload that turns decorations off: the shell clears the declaration while it stays attached.
        shell.declare_decorations(false);
        assert_eq!(
            client.wait_decoration_modes(&decoration, 2),
            vec![Mode::ServerSide, Mode::ClientSide],
            "the window is given its own frame back, so it is never left with none"
        );
        shell.wait_published(node, "decoration", &text("client"));
        assert_eq!(
            shell.published(node, "decoration_wanted"),
            Some(text("server")),
            "what the client asked for does not change"
        );

        // And on again: the ask it still holds is answered.
        shell.declare_decorations(true);
        assert_eq!(
            client.wait_decoration_modes(&decoration, 3),
            vec![Mode::ServerSide, Mode::ClientSide, Mode::ServerSide]
        );
        shell.wait_published(node, "decoration", &text("server"));
    });
}

#[test]
fn a_shell_whose_connection_drops_clears_its_declaration_and_its_windows_get_their_frames_back() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (window, decoration) = client.create_decorated_window("org.example.gtk", "GTK", Some(Mode::ServerSide));
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        assert_eq!(client.wait_decoration_modes(&decoration, 1), vec![Mode::ServerSide]);

        // The shell crashes: it no longer holds the tree and writes nothing.
        shell.detach();
        shell.wait_decorations_declared(false);
        assert_eq!(
            client.wait_decoration_modes(&decoration, 2),
            vec![Mode::ServerSide, Mode::ClientSide]
        );
        shell.wait_published(node, "decoration", &text("client"));

        // The next shell starts from a seat nobody declared on: until it declares, the answer stays client-side.
        shell.attach();
        let (second, second_decoration) =
            client.create_decorated_window("org.example.gtk", "Second", Some(Mode::ServerSide));
        client.attach_buffer(&second.surface, 200, 150);
        assert_eq!(
            client.wait_decoration_modes(&second_decoration, 1),
            vec![Mode::ClientSide]
        );
        shell.declare_decorations(true);
        assert_eq!(
            client.wait_decoration_modes(&second_decoration, 2),
            vec![Mode::ClientSide, Mode::ServerSide]
        );
        assert_eq!(
            client.wait_decoration_modes(&decoration, 3),
            vec![Mode::ServerSide, Mode::ClientSide, Mode::ServerSide]
        );
    });
}

#[test]
fn a_client_that_changes_its_mind_is_answered_again() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        shell.declare_decorations(true);
        let mut client = Client::connect(session.socket());
        let (window, decoration) = client.create_decorated_window("org.example.gtk", "GTK", Some(Mode::ServerSide));
        client.attach_buffer(&window.surface, 200, 150);
        let node = shell.wait_window_node();
        assert_eq!(client.wait_decoration_modes(&decoration, 1), vec![Mode::ServerSide]);

        decoration.set_mode(Mode::ClientSide);
        assert_eq!(
            client.wait_decoration_modes(&decoration, 2),
            vec![Mode::ServerSide, Mode::ClientSide]
        );
        shell.wait_published(node, "decoration", &text("client"));
        shell.wait_published(node, "decoration_wanted", &text("client"));

        decoration.unset_mode();
        client.roundtrip();
        shell.wait_published(node, "decoration_wanted", &text("unset"));
        assert_eq!(
            shell.published(node, "decoration"),
            Some(text("client")),
            "a client with no preference keeps its own frame"
        );
    });
}
