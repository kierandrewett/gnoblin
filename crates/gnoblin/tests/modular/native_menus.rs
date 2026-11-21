//! Example 29 run for real from the three languages: the shipped binary serving a shell that starts the window menu of `libgnoblin-std`,
//! with a real Wayland client that asks for its window menu.
//!
//! A client asks with `xdg_toplevel.show_window_menu` and the serial of a button press, which the test earns with a click of a virtual
//! pointer. What is asserted is what the shell logs of the menu and what the compositor's tree holds of it: the card opens at the client's
//! request, the keyboard (a virtual keyboard) moves over its entries with the arrow keys and a letter, Return runs the entry the
//! keyboard is on after the card has closed, a press anywhere else closes it, and the card of a secure window is secure. One core and one
//! std are mapped.

use gnoblin_proto::tree::snapshot_node::Properties;

use crate::support::native_shell::{
    BTN_LEFT, Gnoblin, KEY_DOWN, KEY_ENTER, KEY_G, Pointer, Start, Typist, start_script, std_shells, wait_for_nodes,
};
use crate::support::{Mapper, Window, scene};
use crate::{copy_example, screen_of, serving};

use super::native_std::{assert_one_core_one_std, start_std};

/// How many lines of the log hold `needle`.
fn count(session: &Gnoblin, needle: &str) -> usize {
    session.log().iter().filter(|line| line.text.contains(needle)).count()
}

/// The window of `app` as the compositor holds it.
fn window_of(session: &Gnoblin, app: &str) -> Window {
    scene(&session.ipc())
        .into_iter()
        .find(|window| window.app_id == app)
        .unwrap_or_else(|| panic!("no window of {app}"))
}

/// Clicks the middle of `window` with the virtual pointer, so the client that owns it holds the serial of a button press.
#[track_caller]
fn click_on(session: &mut Gnoblin, mapper: &mut Mapper, pointer: &mut Pointer, app: &str) {
    let window = window_of(session, app);
    let held = mapper.pointer();
    pointer.move_to(
        (window.x + window.width / 2.0).round() as u32,
        (window.y + window.height / 2.0).round() as u32,
    );
    session.wait_for("the pointer over the window", |_| {
        mapper.roundtrip();
        held.over()
    });
    let before = held.presses();
    pointer.click(BTN_LEFT);
    session.wait_for("the click reaches the client", |_| {
        mapper.roundtrip();
        held.presses() > before
    });
}

/// The canvases the shell made, as the tree holds them.
fn canvases(nodes: &[Properties]) -> usize {
    nodes.iter().filter(|node| node.class == "canvas").count()
}

/// Whether the tree holds the menu's layer: a canvas as big as the screen, which takes every press outside the cards. The shell logs a card
/// when it makes one, and the compositor takes what the shell wrote in two transactions of its own, the canvases and then, from the toolkit's
/// next turn, their places and sizes. A press that goes out before the second lands on what lies under the layer, and the card stays open.
fn has_layer(nodes: &[Properties], screen: (f64, f64)) -> bool {
    nodes.iter().any(|node| {
        let size = |key: &str| {
            node.props
                .iter()
                .find(|prop| prop.key == key)
                .map_or(0.0, |prop| prop.value_double)
        };
        node.class == "canvas" && size("width") >= screen.0 && size("height") >= screen.1
    })
}

#[track_caller]
fn a_client_asks_for_its_menu_and_the_card_answers(session: &mut Gnoblin) {
    session.wait_log("the shell started the menu", |line| line.contains("menus: running"));
    let (width, height) = screen_of(session);
    let mut mapper = Mapper::connect(&session.wayland());
    let mut pointer = Pointer::connect(&session.wayland_privileged(), (width as u32, height as u32));
    mapper.map_window("plain", "plain");
    mapper.map_window("secret", "secret");
    session.wait_for("both windows are in the tree", |session| {
        scene(&session.ipc()).len() == 2
    });
    let plain = window_of(session, "plain");
    let secret = window_of(session, "secret");

    // The first window asks for its menu where it was clicked.
    click_on(session, &mut mapper, &mut pointer, "plain");
    let quiet = canvases(&wait_for_nodes(session, "the tree is there", |nodes| !nodes.is_empty()));
    mapper.show_window_menu(0, 10, 10);
    session.wait_log("the card opens for the plain window", |line| {
        line.contains(&format!("menus: a card opened for window {}, secure: false", plain.id))
    });
    let open = wait_for_nodes(session, "the card's canvases are in the tree", |nodes| {
        canvases(nodes) > quiet + 4
    });
    assert!(canvases(&open) > quiet, "the card is canvases of the tree");

    // The arrows and a letter move the keyboard over the entries the shell gave the menu.
    let mut typist = Typist::connect(&session.wayland());
    typist.tap(KEY_DOWN);
    session.wait_log("the keyboard is on Alpha", |line| {
        line.contains("menus: the keyboard is on Alpha")
    });
    typist.tap(KEY_DOWN);
    session.wait_log("the keyboard is on Beta", |line| {
        line.contains("menus: the keyboard is on Beta")
    });
    typist.tap(KEY_G);
    session.wait_log("a letter jumps to Gamma", |line| {
        line.contains("menus: the keyboard is on Gamma")
    });

    // Return runs the entry after the card has closed, for the window the card was opened on.
    typist.tap(KEY_ENTER);
    session.wait_log("Return runs Gamma", |line| {
        line.contains(&format!("menus: ran Gamma on window {}", plain.id))
    });
    session.wait_for("the card closed", |session| {
        count(session, "menus: the card closed") == 1
    });
    wait_for_nodes(session, "the card's canvases are gone", |nodes| {
        canvases(nodes) <= quiet
    });

    // A press anywhere else closes it. The second request names the serial of the same click.
    mapper.show_window_menu(0, 10, 10);
    session.wait_for("the card opens again", |session| {
        count(session, &format!("menus: a card opened for window {}", plain.id)) == 2
    });
    wait_for_nodes(session, "the card's layer is in the tree again", |nodes| {
        has_layer(nodes, (width, height))
    });
    pointer.move_to(width as u32 - 20, height as u32 - 20);
    pointer.click(BTN_LEFT);
    session.wait_for("a press elsewhere closes it", |session| {
        count(session, "menus: the card closed") == 2
    });
    assert_eq!(
        count(session, "menus: ran "),
        1,
        "closing the card ran no entry: {}",
        session.log_text()
    );

    // The window that is secure gets a secure card.
    click_on(session, &mut mapper, &mut pointer, "secret");
    mapper.show_window_menu(1, 5, 5);
    session.wait_log("the card of the secure window is secure", |line| {
        line.contains(&format!("menus: a card opened for window {}, secure: true", secret.id))
    });
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_29_a_std_shell_starts_the_window_menu_and_a_client_asks_for_it() {
    let mut session = start_std(&std_shells().menus);
    a_client_asks_for_its_menu_and_the_card_answers(&mut session);
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_29_a_typescript_shell_starts_the_window_menu_and_a_client_asks_for_it() {
    let (_dir, mut session) = script_session("29-native-menus-ts", "shell.ts");
    a_client_asks_for_its_menu_and_the_card_answers(&mut session);
}

#[test]
fn example_29_a_python_shell_starts_the_window_menu_and_a_client_asks_for_it() {
    let (_dir, mut session) = script_session("29-native-menus-py", "shell.py");
    a_client_asks_for_its_menu_and_the_card_answers(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
