//! `org_kde_kwin_appmenu_manager`: the DBus address of a window's application menu, from a real client.
//!
//! The scenarios are the failure modes listed at the top of `src/protocols/kde_appmenu.rs`. The compositor draws
//! no menu: it publishes the address on the window's node, which is what a shell that shows a global menu reads, so
//! each test reads the node's `appmenu_service` and `appmenu_path` through the shell stand-in.

mod session;
mod stock;

use gnoblin_render::transaction::TreeValue;
use gnoblin_types::NodeId;
use session::shell::Shell;
use session::{Client, Session};
use wayland_protocols_plasma::appmenu::client::{
    org_kde_kwin_appmenu::OrgKdeKwinAppmenu, org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager,
};

wayland_client::delegate_noop!(session::App: ignore OrgKdeKwinAppmenuManager);
wayland_client::delegate_noop!(session::App: ignore OrgKdeKwinAppmenu);

fn text(value: &str) -> TreeValue {
    TreeValue::Text(value.to_owned())
}

fn manager(client: &Client) -> OrgKdeKwinAppmenuManager {
    client
        .globals
        .bind(&client.qh, 1..=2, ())
        .expect("every client may bind org_kde_kwin_appmenu_manager")
}

/// Waits for the window's node to hold `service` and `path`.
#[track_caller]
fn wait_address(shell: &Shell, node: NodeId, service: &str, path: &str) {
    shell.wait_published(node, "appmenu_service", &text(service));
    shell.wait_published(node, "appmenu_path", &text(path));
}

#[test]
fn every_client_sees_the_appmenu_manager() {
    Session::start().run(|session| {
        let application = Client::connect(session.socket());
        assert_eq!(application.state.globals.get("org_kde_kwin_appmenu_manager"), Some(&2));
    });
}

#[test]
fn a_set_address_is_published_on_the_window_node() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.menu", "Menu");
        let appmenu = manager(&client).create(&window.surface, &client.qh, ());
        appmenu.set_address("org.example.Menu".to_owned(), "/MenuBar/1".to_owned());
        client.roundtrip();
        let node = shell.wait_window_node();
        wait_address(&shell, node, "org.example.Menu", "/MenuBar/1");
    });
}

#[test]
fn a_second_address_replaces_the_first() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.menu", "Menu");
        let appmenu = manager(&client).create(&window.surface, &client.qh, ());
        appmenu.set_address("org.example.Menu".to_owned(), "/MenuBar/1".to_owned());
        client.roundtrip();
        let node = shell.wait_window_node();
        wait_address(&shell, node, "org.example.Menu", "/MenuBar/1");

        appmenu.set_address(":1.42".to_owned(), "/MenuBar/2".to_owned());
        client.roundtrip();
        wait_address(&shell, node, ":1.42", "/MenuBar/2");
    });
}

#[test]
fn releasing_the_appmenu_clears_the_node() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.menu", "Menu");
        let appmenu = manager(&client).create(&window.surface, &client.qh, ());
        appmenu.set_address("org.example.Menu".to_owned(), "/MenuBar/1".to_owned());
        client.roundtrip();
        let node = shell.wait_window_node();
        wait_address(&shell, node, "org.example.Menu", "/MenuBar/1");

        appmenu.release();
        client.roundtrip();
        wait_address(&shell, node, "", "");
    });
}

#[test]
fn releasing_one_of_two_objects_keeps_the_others_address() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.menu", "Menu");
        let manager = manager(&client);
        let first = manager.create(&window.surface, &client.qh, ());
        first.set_address("org.example.First".to_owned(), "/First".to_owned());
        let second = manager.create(&window.surface, &client.qh, ());
        second.set_address("org.example.Second".to_owned(), "/Second".to_owned());
        client.roundtrip();
        let node = shell.wait_window_node();
        wait_address(&shell, node, "org.example.Second", "/Second");

        // The one that set its address last is the one published, and its going leaves the other's in force.
        second.release();
        client.roundtrip();
        wait_address(&shell, node, "org.example.First", "/First");
        first.release();
        client.roundtrip();
        wait_address(&shell, node, "", "");
    });
}

#[test]
fn an_address_set_before_the_toplevel_is_applied_to_it() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        // The object is made for a surface that has no role yet.
        let surface = client.compositor.create_surface(&client.qh, ());
        let appmenu = manager(&client).create(&surface, &client.qh, ());
        appmenu.set_address("org.example.Early".to_owned(), "/Early".to_owned());
        client.roundtrip();

        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        let toplevel = xdg_surface.get_toplevel(&client.qh, ());
        toplevel.set_app_id("org.example.early".to_owned());
        surface.commit();
        client.roundtrip();
        client.attach_buffer(&surface, 200, 150);
        let node = shell.wait_window_node();
        wait_address(&shell, node, "org.example.Early", "/Early");
    });
}

#[test]
fn an_invalid_address_is_dropped() {
    let (shell, hooks) = Shell::new();
    Session::start_shelled(hooks).run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.example.menu", "Menu");
        let appmenu = manager(&client).create(&window.surface, &client.qh, ());
        appmenu.set_address("org.example.Menu".to_owned(), "/MenuBar/1".to_owned());
        client.roundtrip();
        let node = shell.wait_window_node();
        wait_address(&shell, node, "org.example.Menu", "/MenuBar/1");

        // Neither a name with a space nor a path without its slash is an address; the connection survives them and
        // the object keeps the address it had.
        appmenu.set_address("not a name".to_owned(), "/MenuBar/9".to_owned());
        appmenu.set_address("org.example.Menu".to_owned(), "MenuBar".to_owned());
        client.roundtrip();
        // A later valid address still lands, which also proves the invalid ones were handled before it.
        appmenu.set_address("org.example.Menu".to_owned(), "/MenuBar/3".to_owned());
        client.roundtrip();
        wait_address(&shell, node, "org.example.Menu", "/MenuBar/3");
        assert_ne!(
            shell.published(node, "appmenu_path"),
            Some(text("/MenuBar/9")),
            "the invalid address was never published"
        );
    });
}
