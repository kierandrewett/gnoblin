//! Example 8: a shell with no workspaces and no scene graph.
//!
//! The windows of the compositor are the whole model. The shell tiles them master-and-stack on the primary screen by writing each
//! window's rectangle, in the order the windows opened in (the oldest is the master until the user chooses another), and it
//! makes no node of its own: no canvas, no camera, no view. There is no `Workspace` object because there is nothing a
//! workspace would hold that the tree does not.
//!
//! - one window fills the screen; with more, the master takes the left half and the others split the right half;
//! - a window opening or closing tiles the rest again, in the same transaction;
//! - `<Super>m` makes the window that has the keyboard the master;
//! - `<Super>Return` starts a terminal.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core as gnoblin;
use gnoblin_core::{Client, Gnoblin, Rect};

/// The space between windows and around them.
const GAP: f64 = 10.0;

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect { x, y, width, height }
}

/// The rectangles of `count` windows in `area`: the first is the master.
fn tiles(area: &Rect, count: usize) -> Vec<Rect> {
    let inner = rect(
        area.x + GAP,
        area.y + GAP,
        area.width - 2.0 * GAP,
        area.height - 2.0 * GAP,
    );
    match count {
        0 => Vec::new(),
        1 => vec![inner],
        _ => {
            let half = (inner.width - GAP) / 2.0;
            let others = count - 1;
            let each = (inner.height - GAP * (others as f64 - 1.0)) / others as f64;
            let mut tiles = vec![rect(inner.x, inner.y, half, inner.height)];
            for index in 0..others {
                tiles.push(rect(
                    inner.x + half + GAP,
                    inner.y + index as f64 * (each + GAP),
                    half,
                    each,
                ));
            }
            tiles
        }
    }
}

/// The primary screen's rectangle: the first output, or a default when the compositor has none yet.
fn work_area(g: &Gnoblin) -> Rect {
    let area = || -> gnoblin::Res<Option<Rect>> {
        let Some(output) = g.outputs()?.to_list()?.into_iter().next() else {
            return Ok(None);
        };
        Ok(output.bounds()?)
    };
    area()
        .ok()
        .flatten()
        .unwrap_or(rect(0.0, 0.0, 1920.0, 1080.0))
}

/// Tiles every window again: the master first, then the others in the order they opened.
fn retile(g: &Gnoblin, master: &RefCell<Option<String>>) {
    let result = (|| -> gnoblin::Res<()> {
        let mut windows = g.clients()?.to_list()?;
        let chosen = master.borrow().clone();
        if let Some(index) = windows
            .iter()
            .position(|window| window.id().ok() == chosen)
            && chosen.is_some()
        {
            let chosen = windows.remove(index);
            windows.insert(0, chosen);
        }
        for (window, tile) in windows.iter().zip(tiles(&work_area(g), windows.len())) {
            window.set_bounds(&tile)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        g.log(&format!("cannot tile the windows: {error}"));
    }
}

/// The window that has the keyboard.
fn focused(g: &Gnoblin) -> Option<Client> {
    g.clients()
        .ok()?
        .to_list()
        .ok()?
        .into_iter()
        .find(|window| window.focused().unwrap_or(false))
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    // The window the user chose as the master, if one; a closed one is simply not found and the oldest leads again.
    let master: Rc<RefCell<Option<String>>> = Rc::default();
    retile(g, &master);
    {
        let master = Rc::clone(&master);
        g.on_client_created(move |_window, _| retile(g, &master))?.keep();
    }
    {
        let master = Rc::clone(&master);
        g.on_client_closed(move |_window| retile(g, &master))?.keep();
    }

    g.bind("<Super>m", move || {
        if let Some(window) = focused(g) {
            *master.borrow_mut() = window.id().ok();
            retile(g, &master);
        }
    })?;
    g.bind("<Super>Return", move || {
        if let Err(error) = g.spawn(&["foot".to_owned()], None) {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;
    Ok(())
}
