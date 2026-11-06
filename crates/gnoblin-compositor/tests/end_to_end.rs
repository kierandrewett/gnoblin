//! The whole path, with real peers and no display: a session, a Wayland client and
//! the real `gnoblinctl` library entry.
//!
//! This is the acceptance story of the compositor card in `design/rust-compositor.md`
//! ("Process model", "One IPC", "Main data paths"): `gnoblin --role compositor`
//! owns a Wayland socket and the IPC socket; a shell's applications map windows;
//! `gnoblinctl` lists them, watches a focus change, and focuses a window through the
//! `window.focus` method; a client that exits takes its window with it.
//!
//! `gnoblinctl` is called through [`gnoblinctl::run`] — the function the binary's
//! `main` calls — with the session's socket, so what runs is the shipped parse, the
//! shipped command logic and the shipped exit codes. The only thing the test does not
//! do is fork: the session runs in a thread and the clients in this one, which is the
//! same arrangement with one less process to clean up.

mod support;

use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;

use gnoblin_model::ObjectId;

use support::{Client, Command, Session, wait_for, wait_for_listed_id};

/// A `window watch` running in a thread, and the command it printed.
struct Watcher {
    thread: JoinHandle<Command>,
    finished: Receiver<()>,
}

impl Watcher {
    /// Starts `gnoblinctl window watch <id> --count 1` against the session's socket.
    fn start(session: &Session, id: u64) -> Watcher {
        let socket = session.socket().to_path_buf();
        let (done, finished) = channel();
        let thread = std::thread::spawn(move || {
            let command = Command::run(&socket, &["window", "watch", &id.to_string(), "--count", "1"]);
            let _ = done.send(());
            command
        });
        Watcher { thread, finished }
    }

    /// Whether the watch has already printed its one observation.
    fn finished(&self) -> bool {
        self.finished.try_recv().is_ok()
    }
}

#[test]
fn a_shell_lists_watches_and_focuses_through_gnoblinctl() {
    let session = Session::start();

    // A real client maps a window: the Wayland half of the path.
    let mut alpha_client = Client::connect(session.wayland());
    let alpha = alpha_client.map_window("org.gnoblin.Alpha", "Alpha");
    let alpha_id = wait_for_listed_id(&session, "Alpha");

    // `window list` sees it, with the title and the app id the client sent.
    let list = session.ctl(&["window", "list"]).ok();
    assert!(list.out.contains("gnoblin.window"), "{}", list.out);
    assert!(list.out.contains("\"Alpha\""), "the title: {}", list.out);
    assert!(list.out.contains("\"org.gnoblin.Alpha\""), "the app id: {}", list.out);
    assert!(
        list.out.contains("focused=true"),
        "the first window takes focus: {}",
        list.out
    );

    // A second client maps, which takes focus from the first.
    let mut beta_client = Client::connect(session.wayland());
    beta_client.map_window("org.gnoblin.Beta", "Beta");
    let beta_id = wait_for_listed_id(&session, "Beta");

    // `window call <id> focus` changes focus through the window manager. The windows
    // alternate until the watcher has seen one change, which is what makes the watch
    // below independent of when its subscription was accepted.
    let watcher = Watcher::start(&session, alpha_id);
    let mut target = ObjectId::new(beta_id);
    wait_for("the watch prints a focus change", || {
        if watcher.finished() {
            return Some(());
        }
        let other = if target == ObjectId::new(alpha_id) {
            ObjectId::new(beta_id)
        } else {
            ObjectId::new(alpha_id)
        };
        session
            .ctl(&["window", "call", &target.get().to_string(), "focus"])
            .ok();
        target = other;
        None
    });
    let watch = watcher.thread.join().expect("the watcher thread");
    assert_eq!(watch.code, 0, "the watch exits 0: {}", watch.err);
    assert!(
        watch.out.contains(&format!("changed {alpha_id} focused = ")),
        "the focused diff for the watched window: {}",
        watch.out
    );

    // The method's effect is readable from the graph afterwards.
    let focus = session.ctl(&["window", "call", &alpha_id.to_string(), "focus"]).ok();
    assert!(focus.out.contains("done"), "the operation finished: {}", focus.out);
    let focused = session.ctl(&["window", "get", &alpha_id.to_string(), "focused"]).ok();
    assert_eq!(focused.out.trim(), "true", "alpha holds focus");
    let unfocused = session.ctl(&["window", "get", &beta_id.to_string(), "focused"]).ok();
    assert_eq!(unfocused.out.trim(), "false", "beta lost it");
    // The frame rectangle is published as four numbers. The window manager places a
    // window, so the test checks that the rectangle is there and is inside the output
    // rather than pinning a policy value here.
    for property in ["x", "y", "width", "height"] {
        let value = session.ctl(&["window", "get", &alpha_id.to_string(), property]).ok();
        let number: i64 = value
            .out
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("`{property}` is a number, got {:?}", value.out));
        assert!(number >= 0, "`{property}` is inside the output, got {number}");
    }
    let width = session.ctl(&["window", "get", &alpha_id.to_string(), "width"]).ok();
    assert!(width.out.trim() != "0", "the window has an extent: {}", width.out);

    // A title change is a property diff, not a new object.
    alpha_client.set_title(&alpha, "Renamed");
    wait_for("the new title is listed", || {
        let list = session.ctl(&["window", "list"]).ok();
        list.out.contains("\"Renamed\"").then_some(())
    });
    assert_eq!(wait_for_listed_id(&session, "Renamed"), alpha_id);

    // A client that exits takes its window with it: nothing outlives the surface.
    drop(alpha_client);
    wait_for("the crashed client's window is gone", || {
        let list = session.ctl(&["window", "list"]).ok();
        (!list.out.contains("\"Renamed\"")).then_some(())
    });
    let gone = session.ctl(&["window", "get", &alpha_id.to_string(), "title"]);
    assert_eq!(gone.code, 1, "a dead identity is an error response");
    assert!(gone.err.contains("object-unknown"), "{}", gone.err);
    // The other client is untouched by its neighbour's exit.
    assert_eq!(wait_for_listed_id(&session, "Beta"), beta_id);

    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_session_reports_its_version_through_the_short_lived_client() {
    let session = Session::start();
    let version = gnoblin_compositor::version().to_string();
    let list = session.ctl(&["session", "list"]).ok();
    assert!(list.out.contains(&format!("\"{version}\"")), "{}", list.out);
    // `gnoblin --version` prints the same triple, taken from the same catalog.
    assert_eq!(version.split('.').count(), 3);
    session.stop().expect("the session stops cleanly");
}
