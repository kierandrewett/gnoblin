//! gpg's passphrase prompt under the compositor, with the programs a desktop really runs.
//!
//! gnoblin-sn70 and gnoblin-yd5d reported that a `pinentry-gnome3` request never shows a prompt on a Gnoblin
//! session: `gpg-agent` runs `pinentry-gnome3`, which hands the request to `gcr-prompter` over the session bus
//! (`org.gnome.keyring.PrivatePrompter`), and `gcr-prompter` is meant to open its own dialog window.
//!
//! Gnoblin's keyring does not take part in that path: it serves `org.freedesktop.secrets` and asks for its own
//! unlock password through the `prompt.secret` slot, and the gcr `SystemPrompter` adapter that once routed these
//! requests is gone (design/rust-compositor.md, "the `prompt.secret` slot only shows a passphrase or confirm prompt
//! ... GPG's pinentry draws its own prompt"). So the prompt gpg shows is `gcr-prompter`'s own toplevel, and the
//! question is whether the compositor maps it.
//!
//! This test starts a private session bus and a headless compositor, runs the real `gcr-prompter` on the bus and
//! against the compositor's Wayland socket, then runs the real `pinentry-gnome3` and asks it for a passphrase the way
//! `gpg-agent` does (Assuan `GETPIN`). It asserts what a user would see: a focused toplevel window the compositor
//! lists, and that the keys a person types into it are the passphrase `gpg-agent` receives.
//!
//! The test is skipped **with a printed reason** only when the machine has no `gcr-prompter` or no `pinentry-gnome3`,
//! because a suite that passes silently on a host without them checks nothing.

mod support;

#[path = "../../xdg-desktop-portal-gnoblin/tests/support/bus.rs"]
mod bus;

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use bus::Bus;
use support::Session;
use support::ei::keyboard;

/// How long the prompt may take to appear: GTK starts, takes its bus name and maps a window.
const WINDOW_WAIT: Duration = Duration::from_secs(40);

/// The first existing `name` in `directories`.
fn find(name: &str, directories: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(directories.iter().map(PathBuf::from))
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// A child that is killed when the test ends, whatever way it ends.
struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A command that talks to the private bus and the session's compositor, and to nothing of the machine's own: its
/// home, runtime directory and settings are the test's, and it has no X display to fall back on.
fn desktop_program(program: &Path, bus: &Bus, session: &Session, home: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("XDG_RUNTIME_DIR", session.directory())
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .env("WAYLAND_DISPLAY", session.wayland())
        .env("GDK_BACKEND", "wayland")
        .env("GSK_RENDERER", "cairo")
        .env("GSETTINGS_BACKEND", "memory")
        .env("NO_AT_BRIDGE", "1")
        .env("GTK_A11Y", "none");
    command
}

/// The bus name `pinentry-gnome3` asks for a passphrase on.
const PRIVATE_PROMPTER: &str = "org.gnome.keyring.PrivatePrompter";

/// Whether `name` gets an owner on `connection`'s bus within [`WINDOW_WAIT`].
fn wait_for_name(connection: &zbus::Connection, name: &'static str) -> bool {
    zbus::block_on(async {
        let bus = zbus::fdo::DBusProxy::new(connection).await.expect("the bus proxy");
        let name = zbus::names::BusName::try_from(name).expect("a bus name");
        let deadline = Instant::now() + WINDOW_WAIT;
        while Instant::now() < deadline {
            if bus.name_has_owner(name.clone()).await.unwrap_or(false) {
                return true;
            }
            async_io::Timer::after(Duration::from_millis(50)).await;
        }
        false
    })
}

/// The lines `window list` prints: one per toplevel the compositor holds.
fn windows(session: &Session) -> Vec<String> {
    session
        .ctl(&["window", "list"])
        .ok()
        .out
        .lines()
        .map(str::to_owned)
        .collect()
}

/// `KEY_S`, `KEY_E`, `KEY_C`, `KEY_R`, `KEY_E`, `KEY_T` and `KEY_ENTER` from `linux/input-event-codes.h`: "secret".
const PASSPHRASE_KEYS: [u32; 6] = [31, 18, 46, 19, 18, 20];
const KEY_ENTER: u32 = 28;

#[test]
fn a_pinentry_request_opens_a_window_the_compositor_maps_and_the_typed_passphrase_reaches_gpg_agent() {
    let Some(prompter) = find("gcr-prompter", &["/usr/libexec", "/usr/lib/gcr", "/usr/libexec/gcr4"]) else {
        eprintln!("skipped: no gcr-prompter on this machine, so gpg's prompt cannot be run");
        return;
    };
    let Some(pinentry) = find("pinentry-gnome3", &["/usr/bin"]) else {
        eprintln!("skipped: no pinentry-gnome3 on this machine, so gpg's prompt cannot be run");
        return;
    };

    let bus = Bus::start();
    let session = Session::start();
    let mut typing = keyboard(&session);
    let home = tempfile::tempdir().expect("a home directory");
    assert!(
        windows(&session).is_empty(),
        "a fresh session holds no windows: {:?}",
        windows(&session)
    );

    // The prompter owns `org.gnome.keyring.SystemPrompter` and `PrivatePrompter` on the private bus.
    let _prompter = Guard(
        desktop_program(&prompter, &bus, &session, home.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("gcr-prompter starts"),
    );
    // The pinentry falls back to a terminal prompt if the prompter has not taken its name yet, so a request is made only
    // once the name has an owner, as it has on a session where the prompter was started with the login.
    let connection =
        zbus::block_on(zbus::connection::Builder::address(bus.address()).build()).expect("a bus connection");
    assert!(
        wait_for_name(&connection, PRIVATE_PROMPTER),
        "gcr-prompter never took {PRIVATE_PROMPTER}"
    );

    // gpg-agent asks the pinentry for a passphrase in the Assuan protocol: requests on its standard input, replies on
    // its standard output.
    let mut ask = Guard(
        desktop_program(&pinentry, &bus, &session, home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("pinentry-gnome3 starts"),
    );
    let mut input = ask.0.stdin.take().expect("the pinentry's input is piped");
    let replies = BufReader::new(ask.0.stdout.take().expect("the pinentry's output is piped"));
    let (lines, heard) = mpsc::channel();
    std::thread::spawn(move || {
        for line in replies.lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    input
        .write_all(b"SETDESC Enter the passphrase for the test key\nSETPROMPT Passphrase:\nGETPIN\n")
        .expect("the request is written");
    input.flush().expect("the request is flushed");

    // The prompt is one window the compositor holds, and the keyboard is in it.
    let deadline = Instant::now() + WINDOW_WAIT;
    let seen = loop {
        let seen = windows(&session);
        if seen.iter().any(|line| line.contains("focused=true")) {
            break seen;
        }
        assert!(
            Instant::now() < deadline,
            "gpg's passphrase prompt never became a focused window the compositor holds: {seen:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(seen.len(), 1, "the prompt is one window: {seen:?}");
    assert!(
        seen[0].contains("app_id=\"gcr-prompter\"") && seen[0].contains("title=\"pinentry-gnome3\""),
        "the window is gcr-prompter's own prompt: {seen:?}"
    );

    // The person types the passphrase and presses Enter; the pinentry answers `gpg-agent` with it.
    typing.settle(Duration::from_millis(300));
    for key in PASSPHRASE_KEYS.into_iter().chain([KEY_ENTER]) {
        typing.typing(key, true);
        typing.typing(key, false);
    }
    let mut answer = None;
    let deadline = Instant::now() + WINDOW_WAIT;
    while answer.is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = heard
            .recv_timeout(remaining)
            .expect("the pinentry answered before the deadline");
        assert!(!line.starts_with("ERR"), "the pinentry refused the request: {line}");
        answer = line.strip_prefix("D ").map(str::to_owned);
    }
    assert_eq!(
        answer.as_deref(),
        Some("secret"),
        "the passphrase typed into the prompt is the one gpg-agent receives"
    );

    // The request is over: the prompt closes.
    drop(input);
    let deadline = Instant::now() + WINDOW_WAIT;
    while !windows(&session).is_empty() {
        assert!(
            Instant::now() < deadline,
            "the prompt window outlived its request: {:?}",
            windows(&session)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
