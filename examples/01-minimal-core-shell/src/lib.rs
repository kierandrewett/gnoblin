//! Example 1: a minimal shell on `gnoblin-core` alone.
//!
//! What it does, in thirty lines:
//!
//! - every window gets a place the moment it opens (a cascade of 800 by 600 rectangles), including the ones that were open before
//!   the shell started;
//! - a window that opens takes the keyboard: the core moves the keyboard only when a shell asks, so with no line like this one no window ever
//!   has it and `<Super>q` has nothing to close (click focus and raise on click are policy, which is `gnoblin-std`'s: see example 2);
//! - `<Super>Return` starts a terminal (`foot`);
//! - `<Super>q` closes the window that has the keyboard.
//!
//! What it is not: it draws nothing, owns no workspaces and links nothing but the core library (`readelf -d` shows
//! `libgnoblin-core.so.1` as the only library of ours, and `nm -D` shows one defined symbol, `gnoshell_entry_v1`).
//!
//! Build it with `cargo build -p gnoblin-example-minimal-core-shell` and run it with `gnoblin ./target/debug/libminimal_core_shell.so`.

use std::cell::Cell;
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{Client, Gnoblin, Rect};

/// Where the `n`th window goes: a cascade that wraps after eight, so no window is ever placed off the screen.
fn cascade(n: usize) -> Rect {
    let step = 40.0 * (n % 8) as f64;
    Rect {
        x: 60.0 + step,
        y: 60.0 + step,
        width: 800.0,
        height: 600.0,
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    // Every window takes the next place of the cascade.
    let placed = Rc::new(Cell::new(0usize));
    let place = {
        let placed = Rc::clone(&placed);
        move |window: &Client| {
            let n = placed.get();
            placed.set(n + 1);
            if let Err(error) = window.set_bounds(&cascade(n)) {
                g.log(&format!(
                    "cannot place window {}: {error}",
                    window.id().unwrap_or_default()
                ));
            }
        }
    };
    // The listener hears the windows that are open already as well as the ones that open later.
    g.on_client_created(move |window, _| {
        place(&window);
        if let Err(error) = window.focus() {
            g.log(&format!(
                "cannot focus window {}: {error}",
                window.id().unwrap_or_default()
            ));
        }
    })?
    .keep();

    g.bind("<Super>Return", move || {
        if let Err(error) = g.spawn(&["foot".to_owned()], None) {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;

    g.bind("<Super>q", move || {
        let focused = g
            .clients()
            .and_then(|all| all.to_list())
            .ok()
            .and_then(|all| all.into_iter().find(|w| w.focused().unwrap_or(false)));
        if let Some(window) = focused {
            let _ = window.close();
        }
    })?;

    g.log("minimal core shell is running");
    Ok(())
}
