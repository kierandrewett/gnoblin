//! What the compositor tells a shell about how a window relates to others and to the launcher: a dialog's parent
//! (`parent_window`, from `xdg_toplevel.set_parent`) and a window that opened on a launcher's activation token
//! (`from_launcher`). Both are published on the window's node, so a shell can centre a dialog in its parent and keep a
//! launched window from stealing focus.

mod session;

use gnoblin_render::transaction::TreeValue;
use session::shell::Shell;
use session::{Client, Session};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1;

/// Asks for an activation token on a connection that is not given a click's serial.
fn mint_token(client: &mut Client) -> Option<String> {
    let activation: xdg_activation_v1::XdgActivationV1 = client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("every session advertises xdg_activation_v1");
    let token = activation.get_activation_token(&client.qh, ());
    token.commit();
    for _ in 0..20 {
        client.roundtrip();
        if client.state.token.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    client.state.token.clone()
}

#[test]
fn a_dialog_node_names_its_parent_window_node_and_clears_it_when_the_parent_closes() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let parent = client.map_window("org.example.editor", "Editor");
        let dialog = client.create_window("org.example.editor", "Save as");
        dialog.toplevel.set_parent(Some(&parent.toplevel));
        client.attach_buffer(&dialog.surface, 100, 80);
        let nodes = shell.wait_window_nodes(2);
        let (parent_node, dialog_node) = (nodes[0], nodes[1]);

        shell.wait_published(dialog_node, "parent_window", &TreeValue::Node(Some(parent_node)));
        assert_eq!(
            shell.published(parent_node, "parent_window"),
            None,
            "a window with no parent is never given one"
        );

        // The parent goes away: its dialog has no parent any more, and the shell is told.
        client.destroy(parent);
        shell.wait_published(dialog_node, "parent_window", &TreeValue::Node(None));
    });
}

#[test]
fn a_dialog_that_maps_before_its_parent_hears_of_the_parent_when_the_parent_maps() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        // The parent has a toplevel but no buffer, so it has no node yet.
        let parent = client.create_window("org.example.editor", "Editor");
        let dialog = client.create_window("org.example.editor", "Save as");
        dialog.toplevel.set_parent(Some(&parent.toplevel));
        client.attach_buffer(&dialog.surface, 100, 80);
        let dialog_node = shell.wait_window_nodes(1)[0];
        assert_eq!(shell.published(dialog_node, "parent_window"), None);

        client.attach_buffer(&parent.surface, 200, 150);
        let nodes = shell.wait_window_nodes(2);
        shell.wait_published(dialog_node, "parent_window", &TreeValue::Node(Some(nodes[1])));
    });
}

#[test]
fn a_window_that_redeems_a_launchers_token_is_from_the_launcher_and_one_that_does_not_is_not() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        // The launcher is a privileged connection: it earns a token without a click, and hands it to the program it
        // starts, which redeems it for its window.
        let mut launcher = Client::connect(session.privileged_socket());
        let token = mint_token(&mut launcher).expect("a privileged client earns a token");

        let mut app = Client::connect(session.socket());
        let launched = app.create_window("org.example.launched", "Launched");
        let activation: xdg_activation_v1::XdgActivationV1 = app
            .globals
            .bind(&app.qh, 1..=1, ())
            .expect("every session advertises xdg_activation_v1");
        activation.activate(token, &launched.surface);
        app.roundtrip();
        app.attach_buffer(&launched.surface, 200, 150);
        let _own = app.map_window("org.example.own", "Own");

        let nodes = shell.wait_window_nodes(2);
        shell.wait_published(nodes[0], "from_launcher", &TreeValue::Bool(true));
        assert_eq!(
            shell.published(nodes[1], "from_launcher"),
            None,
            "a window nobody launched is not marked"
        );
    });
}
