//! A shell that misbehaves when a test tells it to, for the failure tests of an in-process native shell (`crates/gnoblin/tests/native_shell_failures.rs`).
//!
//! The test and the shell talk through a directory (`GNOBLIN_FIXTURE_DIR`) and nothing else, so the shell needs no protocol:
//!
//! - The shell places every window at `(300 + 20n, 200)`, so a test can tell a window the shell placed from one the compositor's own
//!   window management did.
//! - When the session lock is taken (`lock.state` leaves `not-locked`) it engages its lock screen and renews its lease every 400 ms,
//!   as the stock lock screen does, and stops renewing when the shell hangs.
//! - Every 50 ms a timer writes its count to `ticks` and looks for a command file: `hang` (the callback never returns; the shell writes
//!   `hanging` with the time first), `panic` (the callback panics once), `exit` (the shell asks to exit with the code the file holds).
//! - A thread the shell started waits for the generation to retire and then calls the core, writing what came back to `late-call`.
//! - `fail-init` in the directory makes the entry fail the load.
//!
//! Every file the shell writes holds a line of text, written whole.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use gnoblin_core::host::Rect;

/// The time as the test reads it: milliseconds since the epoch.
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

/// Writes `text` to `name` in `dir` by writing a sibling and renaming it, so a test never reads half a line.
fn note(dir: &Path, name: &str, text: &str) {
    let part = dir.join(format!(".{name}.part"));
    if fs::write(&part, format!("{text}\n")).is_ok() {
        let _ = fs::rename(part, dir.join(name));
    }
}

#[gnoblin::main]
fn main(root: &Gnoblin) -> gnoblin::host::Result<()> {
    let g = root.host();
    let dir = PathBuf::from(
        std::env::var_os("GNOBLIN_FIXTURE_DIR")
            .expect("GNOBLIN_FIXTURE_DIR names the directory the test commands the shell through"),
    );
    assert!(!dir.join("fail-init").exists(), "the fixture was told to fail its init");

    // Windows: placed where only this shell would put them.
    let placed = Rc::new(Cell::new(0u32));
    let place = {
        let placed = Rc::clone(&placed);
        move |window: &gnoblin::host::Window| {
            let n = placed.get();
            placed.set(n + 1);
            let _ = window.set_geometry(Rect::new(300.0 + 20.0 * f64::from(n), 200.0, 400.0, 300.0));
        }
    };
    for window in g.windows()? {
        place(&window);
    }
    g.on_client_created(place)?;

    // The lock: when it is taken, put up a screen (a node under the lock node that has the keyboard), engage it and renew the lease
    // on it, as the stock lock screen does. The compositor counts the screen as held only while all three are true.
    let engaged = Rc::new(Cell::new(false));
    let root = g.root()?;
    let find = |class: &str| {
        root.children().ok().and_then(|nodes| {
            nodes
                .into_iter()
                .find(|node| node.class_name().is_ok_and(|name| name == class))
        })
    };
    if let (Some(lock), Some(seat)) = (find("lock"), find("seat")) {
        let engage = {
            let engaged = Rc::clone(&engaged);
            let lock = lock.clone();
            move |state: &str| {
                if state == "not-locked" {
                    engaged.set(false);
                } else if !engaged.get() {
                    engaged.set(true);
                    if let Err(error) = raise_screen(g, &lock, &seat) {
                        g.log(&format!("cannot put up the lock screen: {error}"));
                    }
                }
            }
        };
        engage(&lock.get_text("state").unwrap_or_default());
        g.on_property_changed(&lock, "state", move |_, value| {
            if let Some(state) = value.as_str() {
                engage(state);
            }
        })?;
        let engaged = Rc::clone(&engaged);
        g.every(400, move || {
            if engaged.get() {
                let _ = lock.call("heartbeat", &[]);
            }
        })?;
    }

    // A thread of the shell's own calls the core after the generation retired.
    let retired = Arc::new(AtomicBool::new(false));
    {
        let retired = Arc::clone(&retired);
        g.on_shutdown(move || retired.store(true, Ordering::Release))?;
    }
    let poster = g.poster();
    let late = dir.clone();
    let started = dir.clone();
    std::thread::spawn(move || {
        while !retired.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
        }
        // The core marks the generation retired after the shutdown hooks ran, so give it a moment.
        std::thread::sleep(Duration::from_millis(300));
        let result = poster.post(|_| {});
        note(&late, "late-call", &format!("{result:?}"));
    });

    // The commands.
    let ticks = Rc::new(Cell::new(0u64));
    g.every(50, move || {
        ticks.set(ticks.get() + 1);
        note(&dir, "ticks", &ticks.get().to_string());
        if dir.join("hang").exists() {
            note(&dir, "hanging", &now_ms().to_string());
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        if dir.join("panic").exists() {
            let _ = fs::remove_file(dir.join("panic"));
            note(&dir, "panicking", &ticks.get().to_string());
            panic!("the fixture was told to panic");
        }
        if let Ok(text) = fs::read_to_string(dir.join("slow")) {
            // A callback that returns, but late: it holds the shell thread for the milliseconds the file holds.
            let _ = fs::remove_file(dir.join("slow"));
            let ms: u64 = text.trim().parse().unwrap_or(0);
            note(&dir, "slowing", &now_ms().to_string());
            std::thread::sleep(Duration::from_millis(ms));
            note(&dir, "slowed", &now_ms().to_string());
        }
        if let Ok(text) = fs::read_to_string(dir.join("exit")) {
            let _ = fs::remove_file(dir.join("exit"));
            let code = text.trim().parse().unwrap_or(1);
            note(&dir, "exiting", &code.to_string());
            let _ = g.request_exit(code);
        }
    })?;

    note(&started, "started", &now_ms().to_string());
    Ok(())
}

/// Puts up the lock screen: a canvas under the lock node, the keyboard on it, and the lock engaged.
fn raise_screen(
    g: &gnoblin::host::Host,
    lock: &gnoblin::host::Node,
    seat: &gnoblin::host::Node,
) -> gnoblin::host::Result<()> {
    let screen = g.create_node("canvas")?;
    screen.set("width", 300.0)?;
    screen.set("height", 100.0)?;
    screen.insert_into(lock, None)?;
    seat.set("focus", screen.id())?;
    lock.set("engaged", true)
}
