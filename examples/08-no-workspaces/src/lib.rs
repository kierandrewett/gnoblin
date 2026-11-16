//! Example 8: a shell with no workspaces and no scene graph.
//!
//! The windows of the tree are the whole model. The shell tiles them master-and-stack on the primary screen by writing each
//! window's rectangle, in the order the compositor stacks them (the first window of the root's children is the master), and
//! it makes no node of its own: no canvas, no camera, no view. There is no `Workspace` object because there is nothing a
//! workspace would hold that the tree does not.
//!
//! - one window fills the screen; with more, the master takes the left half and the others split the right half;
//! - a window opening or closing tiles the rest again, in the same transaction;
//! - `Super+m` makes the window that has the keyboard the master;
//! - `Super+Return` starts a terminal.

use gnoblin_core as gnoblin;
use gnoblin_core::{Gnoblin, Rect, Window};

/// The space between windows and around them.
const GAP: f64 = 10.0;

/// The rectangles of `count` windows in `area`: the first is the master.
fn tiles(area: Rect, count: usize) -> Vec<Rect> {
    let inner = Rect::new(
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
            let mut tiles = vec![Rect::new(inner.x, inner.y, half, inner.height)];
            for index in 0..others {
                tiles.push(Rect::new(
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

/// The primary screen's rectangle: the first screen of the tree, or a default when the compositor has none yet.
fn work_area(g: &Gnoblin) -> Rect {
    let area = || -> gnoblin::Result<Option<Rect>> {
        let screen = g.class("screen")?;
        for node in g.root()?.children()? {
            if node.class()? == screen {
                return Ok(Some(Rect::new(
                    node.get_f64("screen_x")?,
                    node.get_f64("screen_y")?,
                    node.get_f64("screen_width")?,
                    node.get_f64("screen_height")?,
                )));
            }
        }
        Ok(None)
    };
    area().ok().flatten().unwrap_or(Rect::new(0.0, 0.0, 1920.0, 1080.0))
}

/// Tiles every window again.
fn retile(g: &Gnoblin) {
    let result = (|| -> gnoblin::Result<()> {
        let windows = g.windows()?;
        for (window, tile) in windows.iter().zip(tiles(work_area(g), windows.len())) {
            window.set_geometry(tile)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        g.log(&format!("cannot tile the windows: {error}"));
    }
}

/// The window that has the keyboard.
fn focused(g: &Gnoblin) -> Option<Window> {
    g.windows()
        .ok()?
        .into_iter()
        .find(|window| window.is_focused().unwrap_or(false))
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    retile(g);
    g.on_client_created(move |_window| retile(g))?;
    g.on_client_destroyed(move |_window| retile(g))?;

    g.bind("Super+m", move || {
        if let Some(window) = focused(g) {
            // The master is the first of the root's children: lowering a window to the bottom of the stack makes it the first.
            if window.lower().is_ok() {
                retile(g);
            }
        }
    })?;
    g.bind("Super+Return", move || {
        if let Err(error) = g.spawn("foot") {
            g.log(&format!("cannot start a terminal: {error}"));
        }
    })?;
    Ok(())
}
