//! The X11 session a compositor starts from the `xwayland` settings.
//!
//! A real compositor session is started with the settings the `xwayland` table maps to,
//! and a real `x11rb` client connects to the X display the session's `Xwayland` took. The
//! test does not reach into the session: it finds the display by its socket under
//! `/tmp/.X11-unix`, which is where the X server listens whatever number it was given.
//!
//! The test is skipped **with a printed reason** only when the machine has no `Xwayland`
//! binary on `PATH`, because a suite that passes silently on a host without the feature
//! checks nothing.

mod support;

use std::collections::BTreeSet;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_compositor::SessionConfig;
use support::{Session, TIMEOUT};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// The X11 socket directory the X server listens in.
const X11_SOCKETS: &str = "/tmp/.X11-unix";

/// The X display numbers with a listening socket in [`X11_SOCKETS`].
fn displays() -> BTreeSet<u32> {
    let Ok(entries) = std::fs::read_dir(Path::new(X11_SOCKETS)) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.strip_prefix('X')?.parse().ok())
        .collect()
}

#[test]
fn a_session_started_with_xwayland_accepts_an_x11_client() {
    if gnoblin_xwayland::probe(std::env::var_os("PATH").as_deref()).is_err() {
        eprintln!("skipped: no Xwayland binary on PATH, so no X11 session can start here");
        return;
    }

    let before = displays();
    // The default refuses byte-swapped clients, which an Xwayland without the
    // `-byteswappedclients` switch cannot do, so the session would not start there.
    // The test allows them to exercise the start on any Xwayland.
    let (session, _roles) = Session::start_privileged_configured(&[], |config: &mut SessionConfig| {
        config.xwayland = Some(gnoblin_xwayland::Settings {
            allow_byte_swapped_clients: true,
            ..gnoblin_xwayland::Settings::default()
        });
    });

    let deadline = Instant::now() + TIMEOUT;
    let display = loop {
        if let Some(display) = displays().difference(&before).next().copied() {
            break display;
        }
        assert!(Instant::now() < deadline, "no X11 socket appeared under {X11_SOCKETS}");
        thread::sleep(Duration::from_millis(50));
    };

    let name = format!(":{display}");
    let (connection, screen) = loop {
        match RustConnection::connect(Some(&name)) {
            Ok(pair) => break pair,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "the X11 client could not connect to {name}: {error}"
                );
                thread::sleep(Duration::from_millis(50));
            }
        }
    };
    let root = connection.setup().roots[screen].root;
    let geometry = connection
        .get_geometry(root)
        .expect("a request on the connection")
        .reply()
        .expect("the root window's geometry");
    assert!(geometry.width > 0 && geometry.height > 0, "the root window has no size");

    drop(connection);
    drop(session);
}
