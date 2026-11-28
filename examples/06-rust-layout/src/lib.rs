//! Example 6: a tiling layout written in Rust.
//!
//! `columns` makes a `LayoutHandlers` whose `compute` is given the children of a workspace and its area, and answers with where each
//! goes: one column each, the newest on the right. The library calls it whenever the workspace changes (a window joins or leaves, two
//! swap places, the area changes) and writes what it answers, so the shell never positions a window itself.
//!
//! - every window that opens joins the one workspace and takes a column;
//! - a window that closes leaves its column and the rest share the room;
//! - `<Super>Tab` swaps the window that has the keyboard with the first, which moves both columns without the layout being told how.
//!
//! The handler is an ordinary closure: it may keep state between calls, and it runs outside any lock of the library, so it may call
//! back into the SDK. This one keeps nothing.
//!
//! Build it with `cargo build -p gnoblin-example-rust-layout` and run it with `gnoblin ./target/debug/librust_layout.so`.

use gnoblin_std::{Gaps, LayoutHandlers, LayoutPlacement, LayoutRect, Workspace, Workspaces};
use gnoblin_std::prelude::*;

/// Equal-width columns across the workspace, separated by the workspace's inner gap.
fn columns() -> LayoutHandlers {
    LayoutHandlers::new(|context| {
        let count = context.children.len();
        if count == 0 {
            return Ok(Vec::new());
        }
        let area = context.area.clone();
        let (outer, gap) = (context.outer_gap, context.inner_gap);
        let left = area.x + outer;
        let top = (area.y + outer).floor();
        let bottom = (area.y + area.height - outer).floor();
        let width = ((area.width - 2.0 * outer - gap * (count - 1) as f64) / count as f64).max(0.0);
        Ok(context
            .children
            .into_iter()
            .enumerate()
            .map(|(index, node)| {
                // Each edge is floored to a whole pixel on its own, so the columns tile with no seam.
                let start = (left + index as f64 * (width + gap)).floor();
                let end = (left + index as f64 * (width + gap) + width).floor();
                let mut placement = LayoutPlacement::new(node);
                placement.bounds = Some(LayoutRect {
                    x: start,
                    y: top,
                    width: end - start,
                    height: bottom - top,
                });
                placement.tiled = true;
                placement
            })
            .collect())
    })
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let main = Workspace::new(&objects, "main")?;
    main.set_name("Main")?;
    main.set_gaps(&Gaps {
        inner: 10.0,
        outer: 20.0,
    })?;
    // The workspace holds the layout from now on; the shell keeps the handle so the layout stays alive.
    let layout = columns().implement(&objects)?;
    main.set_layout(&layout)?;
    Workspaces::get(&objects)?.add(&main)?;

    g.bind("<Super>Tab", move || {
        let _keep = &layout;
        let swapped = (|| -> gnoblin_std::Result<()> {
            let Some(clients) = main.clients()? else {
                return Ok(());
            };
            let members = clients.members()?;
            if let (Some(first), Some(focused)) = (
                members.first(),
                members.iter().find(|client| client.focused().unwrap_or(false)),
            ) {
                clients.swap(focused, first)?;
            }
            Ok(())
        })();
        if let Err(error) = swapped {
            g.log(&format!("cannot swap: {error}"));
        }
    })?;

    g.log("rust layout shell is running");
    Ok(())
}
