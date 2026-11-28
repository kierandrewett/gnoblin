//! A std shell for the layout tests: one workspace with gaps, and a chord for each of the library's tiling layouts.
//!
//! `Super+1` is a default master and stack, `Super+2` a wider master area of two windows on the right with its own gap, `Super+3` an
//! automatic grid, `Super+4` a grid of three columns filled by column, `Super+5` a monocle and `Super+6` a master and stack with the
//! master on top.

use gnoblin_core::Gnoblin;
use gnoblin_std::prelude::*;
use gnoblin_std::{
    Gaps, GridFill, GridLayout, GridLayoutOptions, Layout, MasterStackLayout, MasterStackLayoutOptions,
    MasterStackOrientation, MonocleLayout, Objects, SwapBinding, Workspace, WorkspaceOptions, Workspaces,
};

/// What a chord does to the workspace: the layout it gives it.
type Make = Box<dyn Fn(&Objects) -> gnoblin_std::Result<Layout>>;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.objects()?.std()?;
    let main = Workspace::new_with(
        &objects,
        "main",
        WorkspaceOptions {
            layout: Some(MasterStackLayout::new(&objects)?.as_layout()?),
            gaps: Some(Gaps {
                inner: 6.0,
                outer: 12.0,
            }),
            ..WorkspaceOptions::default()
        },
    )?;
    Workspaces::get(&objects)?.add(&main)?;
    let chords: Vec<(&str, Make)> = vec![
        ("Super+1", Box::new(|o| Ok(MasterStackLayout::new(o)?.as_layout()?))),
        (
            "Super+2",
            Box::new(|o| {
                Ok(MasterStackLayout::new_with(
                    o,
                    MasterStackLayoutOptions {
                        ratio: Some(0.6),
                        master_count: Some(2),
                        gap: Some(4.0),
                        orientation: Some(MasterStackOrientation::Right),
                    },
                )?
                .as_layout()?)
            }),
        ),
        ("Super+3", Box::new(|o| Ok(GridLayout::new(o)?.as_layout()?))),
        (
            "Super+4",
            Box::new(|o| {
                Ok(GridLayout::new_with(
                    o,
                    GridLayoutOptions {
                        columns: Some(3),
                        fill: Some(GridFill::Column),
                        ..GridLayoutOptions::default()
                    },
                )?
                .as_layout()?)
            }),
        ),
        ("Super+5", Box::new(|o| Ok(MonocleLayout::new(o)?.as_layout()?))),
        (
            "Super+6",
            Box::new(|o| {
                Ok(MasterStackLayout::new_with(
                    o,
                    MasterStackLayoutOptions {
                        orientation: Some(MasterStackOrientation::Top),
                        ..MasterStackLayoutOptions::default()
                    },
                )?
                .as_layout()?)
            }),
        ),
    ];
    let context = Gnoblin::get(&g.objects()?)?.input()?.primary()?;
    for (chord, make) in chords {
        let (main, objects, shell) = (main.clone(), objects.clone(), g.clone());
        context.bind(
            chord,
            Some(move |_| {
                if let Err(error) = make(&objects).and_then(|layout| Ok(main.set_layout(&layout)?)) {
                    shell.log(&format!("cannot switch the layout: {error}"));
                }
                Ok(true)
            }),
            None,
        )?;
    }
    // `Super+Button1` swaps a tiled window with the tile it is dropped on; the service holds the binding from here on.
    let swap = SwapBinding::new(&objects)?;
    swap.install(None)?;
    std::mem::forget(swap);
    g.log("layouts shell is running");
    Ok(())
}
