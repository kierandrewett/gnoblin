//! Example 26: workspaces and layouts of `libgnoblin-std`.
//!
//! This shell makes a workspace with the library's master and stack layout, and adds it to the list. When the third window joins, the
//! workspace gets a layout the shell implements itself (equal columns); when the fourth joins it goes back to the library's layout and
//! the shell moves the divider between the master and the stack as a drag would: it asks the workspace for the divider (`find_split`)
//! and sets its ratio (`set_split`). Every line it logs starts with `objects:`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_std::objects::{
    Desks, Gaps, Layout, LayoutHandlers, LayoutPlacement, LayoutRect, MasterStackLayout, Workspace, Workspaces,
};
use gnoblin_std::prelude::*;

/// Equal columns: the layout the shell implements.
fn columns() -> LayoutHandlers {
    LayoutHandlers {
        compute: Box::new(|context| {
            let area = context.area.clone();
            let gap = context.outer_gap;
            let count = context.children.len().max(1) as f64;
            let width = ((area.width - 2.0 * gap) / count).floor();
            Ok(context
                .children
                .into_iter()
                .enumerate()
                .map(|(at, node)| {
                    let mut placement = LayoutPlacement::new(node);
                    placement.bounds = Some(LayoutRect {
                        x: area.x + gap + width * at as f64,
                        y: area.y + gap,
                        width,
                        height: area.height - 2.0 * gap,
                    });
                    placement.tiled = true;
                    placement
                })
                .collect())
        }),
        client_added: None,
        client_removed: None,
        area_changed: None,
        clients_swapped: None,
        revision: None,
        find_split: None,
        move_split: None,
        reset_split: None,
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let main = Workspace::new(&objects, "main")?;
    main.set_gaps(&Gaps {
        inner: 6.0,
        outer: 12.0,
    })?;
    let stock = MasterStackLayout::new(&objects)?;
    main.set_layout(&stock.as_layout()?)?;
    Workspaces::get(&objects)?.add(&main)?;
    g.log(&format!(
        "objects: the workspace {} is in the list at index {}",
        main.id()?,
        main.index()?
    ));

    let phase = Rc::new(Cell::new(0_u8));
    let (Some(clients), Some(counted)) = (main.clients()?, main.clients()?) else {
        g.log("objects: the workspace has no clients");
        return Ok(());
    };
    let desks = Desks::get(&objects)?;
    let held: Rc<RefCell<Option<Layout>>> = Rc::new(RefCell::new(None));
    let (workspace, shell) = (main, g.clone());
    clients
        .on_added(move |client| {
            let count = counted.length().unwrap_or(0);
            let id = client.id().unwrap_or_default();
            shell.log(&format!("objects: client {id} joined main, {count} windows"));
            if count >= 3 && phase.get() == 0 {
                phase.set(1);
                match columns().implement(&objects) {
                    Ok(layout) => {
                        let done = workspace.set_layout(&layout).is_ok();
                        *held.borrow_mut() = Some(layout);
                        shell.log(&format!(
                            "objects: a layout of the shell's own tiles {count} windows: {done}"
                        ));
                    }
                    Err(error) => shell.log(&format!("objects: cannot implement the layout: {error}")),
                }
            } else if count >= 4 && phase.get() == 1 {
                phase.set(2);
                // The master and stack object stays alive while the workspace takes it: the workspace holds it from then on.
                let back = MasterStackLayout::new(&objects).and_then(|stock| {
                    let layout = stock.as_layout()?;
                    workspace.set_layout(&layout)
                });
                shell.log(&format!(
                    "objects: back to the library's layout with {count} windows: {}",
                    back.is_ok()
                ));
            } else if count >= 5 && phase.get() == 2 {
                // The library has laid the four windows out by now, so the divider is there to be found.
                phase.set(3);
                // The divider is asked of a window: the first member, the core's `Client` of it.
                let area = desks.areas(None).ok().and_then(|areas| areas.area);
                let first = counted.members().ok().and_then(|members| members.into_iter().next());
                let (Some(area), Some(first)) = (area.clone(), first.as_ref()) else {
                    shell.log(&format!("objects: cannot find the divider: area {area:?}"));
                    return;
                };
                // The divider lies where the master ends: look along the middle of the area for the press that finds it.
                let mut x = area.x;
                while x < area.x + area.width {
                    if let Ok(Some(split)) = workspace.find_split(first, x, area.y + area.height / 2.0) {
                        let to = 0.7_f64.clamp(split.min, split.max);
                        let moved = workspace.set_split(split.token, to, None).is_ok();
                        shell.log(&format!(
                            "objects: the divider is at {} and moves to {to}: {moved}",
                            split.ratio
                        ));
                        return;
                    }
                    x += 25.0;
                }
                shell.log("objects: no divider found");
            }
        })?
        .keep();
    g.log("objects: running");
    Ok(())
}
