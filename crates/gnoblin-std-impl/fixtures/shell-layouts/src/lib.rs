//! A std shell for the layout tests: one workspace with gaps, and a chord for each of the library's tiling layouts.
//!
//! `Super+1` is a default master and stack, `Super+2` a wider master area of two windows on the right with its own gap, `Super+3` an
//! automatic grid, `Super+4` a grid of three columns filled by column, `Super+5` a monocle and `Super+6` a master and stack with the
//! master on top.

use gnoblin_std::prelude::*;

/// What a chord does to the workspace.
type Switch = Box<dyn Fn() -> gnoblin_std::Result<()>>;

/// `Super+Button1` swaps a tiled window with the tile it is dropped on.
fn std_swap(g: &Gnoblin) -> gnoblin_std::Result<()> {
    g.std()?.bind_drag(gnoblin_std::ClientSwapBinding::new())?;
    Ok(())
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let main = Workspace::builder("main")?
        .layout(MasterStackLayout::new())
        .gaps(6.0, 12.0)
        .add()?;
    let chords: Vec<(&str, Switch)> = vec![
        ("Super+1", {
            let main = main.clone();
            Box::new(move || main.set_layout(MasterStackLayout::new()))
        }),
        ("Super+2", {
            let main = main.clone();
            Box::new(move || {
                main.set_layout(
                    MasterStackLayout::new()
                        .ratio(0.6)
                        .master_count(2)
                        .gap(4.0)
                        .orientation(Orientation::Right),
                )
            })
        }),
        ("Super+3", {
            let main = main.clone();
            Box::new(move || main.set_layout(GridLayout::new()))
        }),
        ("Super+4", {
            let main = main.clone();
            Box::new(move || main.set_layout(GridLayout::new().columns(Columns::Fixed(3)).fill(GridFill::Column)))
        }),
        ("Super+5", {
            let main = main.clone();
            Box::new(move || main.set_layout(MonocleLayout::new()))
        }),
        ("Super+6", {
            let main = main.clone();
            Box::new(move || main.set_layout(MasterStackLayout::new().orientation(Orientation::Top)))
        }),
    ];
    for (chord, switch) in chords {
        g.bind(chord, move || {
            if let Err(error) = switch() {
                g.log(&format!("cannot switch the layout: {error}"));
            }
        })?;
    }
    std_swap(g)?;
    g.log("layouts shell is running");
    Ok(())
}
