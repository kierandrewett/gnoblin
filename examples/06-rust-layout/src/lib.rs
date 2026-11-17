//! Example 6: a tiling layout written in Rust.
//!
//! `Columns` implements [`Layout`]: given the clients of a workspace and its area, it answers with where each goes, one column each,
//! the newest on the right. The library calls it whenever the workspace changes (a window joins or leaves, two swap places, the area
//! changes) and writes what it answers, so the shell never positions a window itself.
//!
//! - every window that opens joins the one workspace and takes a column;
//! - a window that closes leaves its column and the rest share the room;
//! - `<Super>Tab` swaps the window that has the keyboard with the first, which moves both columns without the layout being told how.
//!
//! The layout is an ordinary value: it may keep state between calls (`&mut self`), and it runs outside any lock of the library, so it
//! may call back into the SDK. This one keeps nothing.
//!
//! Build it with `cargo build -p gnoblin-example-rust-layout` and run it with `gnoblin ./target/debug/librust_layout.so`.

use gnoblin_std::prelude::*;

/// Equal-width columns across the workspace, separated by the workspace's inner gap.
struct Columns;

impl Layout for Columns {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let count = cx.children.len();
        if count == 0 {
            return Vec::new();
        }
        let area = cx.inset_area();
        let width = ((area.width - cx.gap_inner * (count - 1) as f64) / count as f64).max(0.0);
        cx.children
            .iter()
            .enumerate()
            .map(|(index, client)| {
                Placement::tile(
                    client,
                    area.x + index as f64 * (width + cx.gap_inner),
                    area.y,
                    width,
                    area.height,
                )
            })
            .collect()
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let main = Workspace::builder("main")?
        .name("Main")
        .layout(Columns)
        .gaps(10.0, 20.0)
        .add()?;

    g.bind("<Super>Tab", move || {
        let swapped = (|| -> gnoblin_std::Result<()> {
            let clients = main.clients()?;
            if let (Some(focused), Some(first)) = (clients.focused()?, clients.at(0)?) {
                main.swap(&focused, &first)?;
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
