//! Example 10: a shell you can rebuild while it runs.
//!
//! Edit this file, build it (`cargo build`), and the running compositor loads the new library without a restart and without
//! disturbing a single client: every window stays open on the same Wayland connection and surface, and the node ids the shell
//! sees do not change. The reload protocol is `design/modular-framework.md` section 8.1; what makes this shell reload well is
//! four habits, each a few lines below:
//!
//! 1. **State goes through `keep_state`.** The count of windows placed so far lives in the kept state, under a name and a
//!    version, and the next generation reads it back. The shell's own variables die with its generation.
//! 2. **Nodes are found by name.** The badge is made with [`Gnoblin::adopt_node`], so the next generation gets the same node
//!    (same id, same children, same values) instead of making a second one. The toast is made with
//!    [`Gnoblin::adopt_ephemeral_node`]: it belongs to this generation and the compositor removes it at the next reload unless
//!    the new code adopts it again.
//! 3. **Registrations are made in the entry function, and only there.** Listeners, the chords and the timer are removed with the
//!    generation that made them, so a reload never leaves two of anything running. A program started with the same arguments
//!    by two generations is one program.
//! 4. **Start-up work is done once.** The windows open when the shell first starts are placed by the first generation; a
//!    generation that finds kept state leaves them alone.
//!
//! What it does, so that a test can see each of the above: new windows cascade by `STEP` pixels; `Super+n` counts a press on the
//! badge's `z`; a 50 ms timer counts on its `y`; `Super+Shift+r` asks for a reload; a helper program is started once.
//!
//! The test of this example (`crates/gnoblin-core-impl/tests/reload.rs`) builds it twice from the same source, once as it is and
//! once with the two constants below edited, exactly as a developer's edit-and-rebuild loop does, and loads one after the other.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{Gnoblin, Rect, Window};

/// What this build says it is. The badge's `x` shows it (ten times the number), so a reload is visible in the tree.
const VERSION: u32 = 1;

/// Pixels between cascaded windows. Edit it and the next window opens somewhere else; the windows that are open stay put.
const STEP: f64 = 40.0;

/// The kept state's key and the version of its layout. A generation that finds another version starts from zero instead of
/// reading bytes it does not understand.
const OPENED: &str = "opened";
const OPENED_VERSION: u32 = 1;

/// The windows placed so far, from the kept state.
fn windows_placed(g: &Gnoblin) -> Option<u32> {
    let (bytes, version) = g.kept_state(OPENED).ok().flatten()?;
    (version == OPENED_VERSION)
        .then(|| bytes.try_into().ok().map(u32::from_le_bytes))
        .flatten()
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    let kept = windows_placed(g);
    let placed = Rc::new(Cell::new(kept.unwrap_or(0)));

    // Habit 1 and 4: the first generation places the windows that are open; a reload finds state and leaves them alone.
    let place = {
        let placed = Rc::clone(&placed);
        move |window: &Window| {
            let n = placed.get();
            placed.set(n + 1);
            let offset = STEP * f64::from(n % 8);
            if let Err(error) = window.set_geometry(Rect::new(60.0 + offset, 60.0 + offset, 640.0, 480.0)) {
                window
                    .gnoblin()
                    .log(&format!("cannot place window {}: {error}", window.id().0));
            }
            if let Err(error) = window
                .gnoblin()
                .keep_state(OPENED, &placed.get().to_le_bytes(), OPENED_VERSION)
            {
                window.gnoblin().log(&format!("cannot keep the window count: {error}"));
            }
        }
    };
    if kept.is_none() {
        for window in g.windows()? {
            place(&window);
        }
    }
    g.on_client_created(place)?;

    // Habit 2: the badge outlives generations; the toast does not.
    let badge = g.adopt_node("canvas", "reload-badge")?;
    badge.node.set("x", f64::from(VERSION) * 10.0)?;
    if VERSION == 1 {
        g.adopt_ephemeral_node("canvas", "reload-toast")?;
    }

    // Habit 3: registrations belong to the generation. `Super+n` counts on the badge's `z`, and the count is kept state too, so
    // it goes on from where the last generation left it.
    let presses = Rc::new(Cell::new(
        g.kept_state("presses")?
            .and_then(|(bytes, _)| bytes.try_into().ok().map(u32::from_le_bytes))
            .unwrap_or(0),
    ));
    {
        let badge = badge.node.clone();
        let presses = Rc::clone(&presses);
        g.bind("Super+n", move || {
            presses.set(presses.get() + 1);
            let result = badge
                .set("z", f64::from(presses.get()))
                .and_then(|()| badge.gnoblin().keep_state("presses", &presses.get().to_le_bytes(), 1));
            if let Err(error) = result {
                badge.gnoblin().log(&format!("cannot count a press: {error}"));
            }
        })?;
    }
    g.bind("Super+Shift+r", move || {
        if let Err(error) = g.request_reload() {
            g.log(&format!("cannot ask for a reload: {error}"));
        }
    })?;

    // A 50 ms timer counts on the badge's `y`: one tick per period however many times the shell reloaded.
    {
        let badge = badge.node.clone();
        let ticks = Rc::new(Cell::new(0u32));
        g.every(50, move || {
            ticks.set(ticks.get() + 1);
            if let Err(error) = badge.set("y", f64::from(ticks.get())) {
                badge.gnoblin().log(&format!("cannot count a tick: {error}"));
            }
        })?;
    }

    // A helper the shell keeps running (a status bar, say). The next generation starts the same program with the same arguments,
    // and the host adopts the one that is running instead of starting a second.
    if let Err(error) = g.spawn("hot-reload-helper") {
        g.log(&format!("cannot start the helper: {error}"));
    }

    let reloads = g.info().map_or(0, |info| info.reload_count);
    g.log(&format!(
        "hot-reload version {VERSION} is running (reload {reloads}, {} window(s) placed so far)",
        placed.get()
    ));
    Ok(())
}
