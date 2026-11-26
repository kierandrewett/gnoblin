//! Example 34: workspaces and what lays their windows out.
//!
//! This shell makes two workspaces, `main` and `side`, and adds both to the list. The library puts every window that opens on the workspace
//! that is shown, so the windows join `main`. The chords change the layout of `main` and show a workspace:
//!
//! - `Super+1` to `Super+7`: the library's own layouts, master and stack, grid, monocle, dwindle, strip, arc and carousel;
//! - `Super+8`: a layout this shell implements itself (equal columns);
//! - `Super+9` and `Super+0`: show `side`, show `main`;
//! - `Super+MouseLeft+Drag`: a drag that moves the divider between two tiles. The press names a window, the workspace names the divider
//!   that window borders at that point (`find_split`), and the pointer's travel sets its ratio (`set_split`).
//!
//! The chords and the drag are bindings on the seat's input context of the core; the workspaces, the layouts and the divider are the std
//! SDK. Every line it logs starts with `workspaces:`. Build it with `cargo build -p gnoblin-example-native-workspaces` and run it with
//! `gnoblin ./target/debug/libnative_workspaces.so`.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::objects::{BindingEvent, BindingPhase, InputPoint, ModelRoot};
use gnoblin_std::prelude::*;
use gnoblin_std::{Point, Split};

/// Equal columns: the layout this shell implements.
struct Columns;

impl Layout for Columns {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let area = cx.inset_area();
        let width = (area.width / cx.children.len().max(1) as f64).floor();
        cx.children
            .iter()
            .enumerate()
            .map(|(at, client)| Placement::tile(client, area.x + width * at as f64, area.y, width, area.height))
            .collect()
    }
}

/// The names of the chords' layouts, in the order of `Super+1` to `Super+8`.
const NAMES: [&str; 8] = [
    "master_stack",
    "grid",
    "monocle",
    "dwindle",
    "strip",
    "arc",
    "carousel",
    "columns",
];

/// Gives `workspace` the layout the number `which` names.
fn give_layout(workspace: &Workspace, which: usize) -> gnoblin_std::Result<()> {
    match which {
        0 => workspace.set_layout(MasterStackLayout::new()),
        1 => workspace.set_layout(GridLayout::new()),
        2 => workspace.set_layout(MonocleLayout::new()),
        3 => workspace.set_layout(DwindleLayout::new()),
        4 => workspace.set_layout(StripLayout::new()),
        5 => workspace.set_layout(ArcLayout::new()),
        6 => workspace.set_layout(Carousel3dLayout::new()),
        _ => workspace.set_layout(Columns),
    }
}

/// A divider drag that is going: the divider, where the pointer pressed and the ratios the dividers had then.
struct Held {
    split: Split,
    from: Point,
    ratio: f64,
    cross_ratio: Option<f64>,
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?;
    let main = Workspace::builder_in(&std, "main")
        .name("Main")
        .gaps(6.0, 12.0)
        .layout(MasterStackLayout::new())
        .add()?;
    let side = Workspace::builder_in(&std, "side")
        .name("Side")
        .layout(MasterStackLayout::new())
        .add()?;
    let at = |workspace: &Workspace| workspace.index().map(|index| index.map_or(-1, |index| index as i64));
    g.log(&format!(
        "workspaces: the list holds {} workspaces: {} at {}, {} at {}",
        std.workspaces()?.len()?,
        main.id()?,
        at(&main)?,
        side.id()?,
        at(&side)?
    ));
    main.clients()?.on_added({
        let (main, g) = (main.clone(), g.clone());
        move |client| {
            let count = main.clients().and_then(|clients| clients.len()).unwrap_or(0);
            g.log(&format!(
                "workspaces: client {} joined {}, {count} windows",
                client.window().id().0,
                main.id().unwrap_or_default()
            ));
        }
    })?;

    let objects = g.objects()?;
    let Some(context) = ModelRoot::get(&objects)?.input()?.primary().ok() else {
        g.log("workspaces: the compositor reports no seat");
        return Ok(());
    };

    for (at, name) in NAMES.iter().enumerate() {
        let (workspace, g) = (main.clone(), g.clone());
        context.bind(
            &format!("Super+{}", at + 1),
            Some(move |_| {
                let done = give_layout(&workspace, at).is_ok();
                g.log(&format!(
                    "workspaces: {} has the {name} layout: {done}",
                    workspace.id().unwrap_or_default()
                ));
                Ok(true)
            }),
            None,
        )?;
    }
    for (trigger, workspace) in [("Super+9", side.clone()), ("Super+0", main.clone())] {
        let g = g.clone();
        context.bind(
            trigger,
            Some(move |_| {
                let shown = workspace.show().is_ok();
                g.log(&format!(
                    "workspaces: show {}: {shown}, active {:?}",
                    workspace.id().unwrap_or_default(),
                    workspace.is_active().ok()
                ));
                Ok(true)
            }),
            None,
        )?;
    }

    // One function follows the drag from the press to the end. The press picks the window under the pointer.
    let held: Rc<RefCell<Option<Held>>> = Rc::new(RefCell::new(None));
    let Some(seat) = ModelRoot::get(&objects)?.input()?.primary().ok() else {
        return Ok(());
    };
    let (workspace, shell) = (main.clone(), g.clone());
    context.bind(
        "Super+MouseLeft+Drag",
        Some(move |event: BindingEvent| {
            let at = Point {
                x: event.position.x,
                y: event.position.y,
            };
            match event.phase {
                Some(BindingPhase::Start) => {
                    let hit = seat.hit_test(&InputPoint { x: at.x, y: at.y })?;
                    // The window under the press; a layout whose divider does not depend on the point is asked about the first window.
                    let client = hit
                        .and_then(|hit| hit.client.id().ok())
                        .and_then(|id| id.parse::<u64>().ok())
                        .or_else(|| {
                            let first = workspace.clients().ok()?.at(0).ok()??;
                            Some(first.window().id().0)
                        })
                        .map(|raw| shell.window_handle(gnoblin_core::NodeId(raw)));
                    let Some(split) = client.and_then(|client| workspace.find_split(client, at).ok().flatten()) else {
                        shell.log("workspaces: the press is on no divider");
                        return Ok(false);
                    };
                    shell.log(&format!(
                        "workspaces: divider {} at {}",
                        split.token, split.divider.ratio
                    ));
                    *held.borrow_mut() = Some(Held {
                        split,
                        from: at,
                        ratio: split.divider.ratio,
                        cross_ratio: split.cross.map(|cross| cross.ratio),
                    });
                }
                Some(BindingPhase::Update) => {
                    if let Some(drag) = held.borrow().as_ref() {
                        // The pointer is where the press was, moved by the drag's distance.
                        let now = Point {
                            x: drag.from.x + event.dx.unwrap_or(0.0),
                            y: drag.from.y + event.dy.unwrap_or(0.0),
                        };
                        let ratio = drag.split.divider.follow(drag.ratio, drag.from, now);
                        let cross = drag
                            .split
                            .cross
                            .zip(drag.cross_ratio)
                            .map(|(cross, start)| cross.follow(start, drag.from, now));
                        let _ = workspace.set_split(&drag.split, ratio, cross);
                    }
                }
                Some(BindingPhase::End) => {
                    if let Some(drag) = held.borrow_mut().take() {
                        shell.log(&format!("workspaces: divider {} dropped", drag.split.token));
                    }
                }
                Some(BindingPhase::Cancel) => {
                    if let Some(drag) = held.borrow_mut().take() {
                        let _ = workspace.reset_split(&drag.split);
                        shell.log(&format!("workspaces: divider {} put back", drag.split.token));
                    }
                }
                None => {}
            }
            Ok(true)
        }),
        None,
    )?;
    g.log("workspaces: running");
    Ok(())
}
