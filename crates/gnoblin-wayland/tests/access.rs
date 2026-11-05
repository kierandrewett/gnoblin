//! `wp_security_context_v1` and the per-client access policy, from both sides of the
//! socket.
//!
//! The plan's rule is that access is decided per client, not only per global. That is
//! only provable with two connections: one on the privileged socket, one on the
//! ordinary socket, and one inside an application sandbox. Each test connects a real
//! client on a real socket and asks what the registry answered.

mod session;

use std::os::fd::AsFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use session::{Client, Session};
use wayland_protocols::wp::security_context::v1::client::{wp_security_context_manager_v1, wp_security_context_v1};

/// The globals only a privileged or trusted connection may see.
const PRIVILEGED: &[&str] = &[
    "zwlr_layer_shell_v1",
    "zwlr_foreign_toplevel_manager_v1",
    "ext_foreign_toplevel_list_v1",
    "ext_data_control_manager_v1",
    "zwlr_data_control_manager_v1",
];

/// The globals every application needs, so a test can tell "denied" from "broken".
const ORDINARY: &[&str] = &[
    "wl_compositor",
    "wl_shm",
    "wl_seat",
    "wl_data_device_manager",
    "xdg_wm_base",
    "xdg_activation_v1",
];

/// Binds the security-context manager on a connection that may create sandboxes.
fn sandbox_manager(client: &mut Client) -> wp_security_context_manager_v1::WpSecurityContextManagerV1 {
    client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("a connection that is not already sandboxed may create one")
}

/// Creates an application sandbox and returns the socket inside it.
///
/// The compositor is handed a listening socket through the protocol and then accepts
/// the connections that arrive on it, so the socket path is the test's own endpoint for
/// the sandboxed client.
fn create_sandbox(
    shell: &mut Client,
    directory: &tempfile::TempDir,
) -> (
    PathBuf,
    wp_security_context_v1::WpSecurityContextV1,
    std::io::PipeWriter,
) {
    let path = directory.path().join("wayland-0-sandbox.sock");
    let listener = UnixListener::bind(&path).expect("a sandbox listener");
    let (close_reader, close_writer) = std::io::pipe().expect("a pipe");
    let manager = sandbox_manager(shell);
    let context = manager.create_listener(listener.as_fd(), close_reader.as_fd(), &shell.qh, ());
    context.set_sandbox_engine("org.gnoblin.Test".to_owned());
    context.set_app_id("org.gnoblin.Sandboxed".to_owned());
    // The listener only exists once the context is committed: the protocol makes the
    // registration atomic so a half-built sandbox cannot be connected to.
    context.commit();
    shell.roundtrip();
    // The close end is held for as long as the sandbox exists: closing it is how the
    // engine tells the compositor the sandbox is gone, and the compositor stops
    // accepting connections on that listener when it does.
    (path, context, close_writer)
}

/// Asserts that a connection sees exactly the ordinary globals and none of the others.
fn assert_unprivileged(client: &Client, who: &str) {
    for interface in ORDINARY {
        assert!(
            client.has_global(interface),
            "{who} needs {interface} to be a working client"
        );
    }
    for interface in PRIVILEGED {
        assert!(!client.has_global(interface), "{who} must not see {interface}");
    }
}

#[test]
fn a_sandboxed_client_sees_no_privileged_global() {
    let session = Session::start();
    session.run(|session| {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let mut shell = Client::connect(session.privileged_socket());
        let (sandbox_path, _context, _close) = create_sandbox(&mut shell, &directory);

        // The sandbox engine itself is privileged; what it creates is not.
        for interface in PRIVILEGED {
            assert!(
                shell.has_global(interface),
                "the privileged connection sees {interface}"
            );
        }

        let mut sandboxed = Client::connect(Path::new(&sandbox_path));
        assert_unprivileged(&sandboxed, "a sandboxed client");
        assert!(
            !sandboxed.has_global("wp_security_context_manager_v1"),
            "a sandbox may not create another sandbox"
        );

        // The marking is a policy, not a broken connection: the sandboxed client maps a
        // window and the session serves it like any other.
        let _window = sandboxed.map_window("org.gnoblin.Sandboxed", "Sandboxed");
        let observation = session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        assert_eq!(observation.only().expect("one window").app_id, "org.gnoblin.Sandboxed");
    });
}

#[test]
fn an_unprivileged_client_sees_no_privileged_global() {
    let session = Session::start();
    session.run(|session| {
        let application = Client::connect(session.socket());
        assert_unprivileged(&application, "an application");
    });
}

#[test]
fn a_privileged_client_sees_every_privileged_global() {
    let session = Session::start();
    session.run(|session| {
        let privileged = Client::connect(session.privileged_socket());
        for interface in PRIVILEGED {
            assert!(
                privileged.has_global(interface),
                "the privileged socket advertises {interface}"
            );
        }
        for interface in ORDINARY {
            assert!(
                privileged.has_global(interface),
                "and the ordinary ones as well: {interface}"
            );
        }
    });
}

#[test]
fn a_disabled_protocol_is_advertised_to_nobody() {
    let mut protocols = gnoblin_config::Protocols::default();
    protocols.toggles.insert(gnoblin_config::Protocol::WlrLayerShell, false);
    let session = Session::start_with(protocols);
    session.run(|session| {
        let privileged = Client::connect(session.privileged_socket());
        assert!(
            !privileged.has_global("zwlr_layer_shell_v1"),
            "the configuration gate comes before the roster's: nobody sees it"
        );
        assert!(
            privileged.has_global("zwlr_foreign_toplevel_manager_v1"),
            "and every other protocol is untouched"
        );
    });
}

#[test]
fn a_sandboxed_client_still_uses_the_ordinary_clipboard() {
    let session = Session::start();
    session.run(|session| {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let mut shell = Client::connect(session.privileged_socket());
        let (sandbox_path, _context, _close) = create_sandbox(&mut shell, &directory);
        let mut sandboxed = Client::connect(Path::new(&sandbox_path));

        // The sandbox flag denies the *managers*, not the clipboard an application
        // needs: `wl_data_device_manager` and the primary selection stay.
        assert!(sandboxed.has_global("wl_data_device_manager"));
        assert!(sandboxed.has_global("zwp_primary_selection_device_manager_v1"));
        let manager: wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager = sandboxed
            .globals
            .bind(&sandboxed.qh, 1..=3, ())
            .expect("and binding it works");
        let _device = manager.get_data_device(&sandboxed.seat, &sandboxed.qh, ());
        sandboxed.roundtrip();

        for interface in PRIVILEGED {
            assert!(
                !sandboxed.has_global(interface),
                "the manager {interface} is not part of the sandbox's registry"
            );
        }
    });
}
