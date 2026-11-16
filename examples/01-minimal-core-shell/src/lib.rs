//! Example 1: a minimal shell on `gnoblin-core` alone.
//!
//! What it does, in thirty lines:
//!
//! - every window gets a place the moment it opens (a cascade of 800 by 600 rectangles), including the ones that were open before
//!   the shell started;
//! - `Super+Return` starts a terminal (`foot`);
//! - `Super+q` closes the window that has the keyboard.
//!
//! What it is not: it draws nothing, owns no workspaces and links nothing but the core library (`readelf -d` shows
//! `libgnoblin-core.so.1` as the only library of ours, and `nm -D` shows one defined symbol, `gnoshell_entry_v1`).
//!
//! Build it with `cargo build -p gnoblin-example-minimal-core-shell` and run it with `gnoblin ./target/debug/libminimal_core_shell.so`.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{Gnoblin, Rect, Window};

/// Where the `n`th window goes: a cascade that wraps after eight, so no window is ever placed off the screen.
fn cascade(n: usize) -> Rect {
    let step = 40.0 * (n % 8) as f64;
    Rect::new(60.0 + step, 60.0 + step, 800.0, 600.0)
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    // The windows that are open already, then the ones that open later, take the next place of the cascade.
    let placed = Rc::new(Cell::new(0usize));
    let place = {
        let placed = Rc::clone(&placed);
        move |window: &Window| {
            let n = placed.get();
            placed.set(n + 1);
            if let Err(error) = window.set_geometry(cascade(n)) {
                window
                    .gnoblin()
                    .log(&format!("cannot place window {}: {error}", window.id().0));
            }
        }
    };
    for window in g.windows()? {
        place(&window);
    }
    g.on_client_created(place)?;

    g.bind("Super+Return", move || {
        if let Err(error) = g.spawn("foot") {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;

    g.bind("Super+q", move || {
        let focused = g
            .windows()
            .ok()
            .and_then(|all| all.into_iter().find(|w| w.is_focused().unwrap_or(false)));
        if let Some(window) = focused {
            let _ = window.close();
        }
    })?;

    g.log("minimal core shell is running");
    Ok(())
}
