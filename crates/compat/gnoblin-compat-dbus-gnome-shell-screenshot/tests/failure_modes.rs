//! One test per failure mode that needs no bus: the embargo the adapter reads.
//!
//! The only thing this adapter takes from the graph is the lock state, so the interesting
//! failure is a tree that does not publish it: a released embargo inferred from an absent
//! node is the one guess a capture adapter must not make.

mod support;

use gnoblin_client::Server;
use gnoblin_compat_dbus_gnome_shell_screenshot::{ScreenshotError, ScreenshotLink};
use support::{VERSION, graph};

#[test]
fn a_tree_without_a_lock_node_is_refused() {
    // The failure mode: the adapter starts with an embargo it invented, and screenshots a
    // session that may be locked.
    let mut server = Server::new(graph(), VERSION);
    let client = server.connect(VERSION).expect("the versions match");

    match ScreenshotLink::connect(client) {
        Err(ScreenshotError::NoLock) => {}
        other => panic!("a tree with no lock node is refused, got {other:?}"),
    }
}
