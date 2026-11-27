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
use gnoblin_std::objects::{ArcLayout, Carousel3dLayout, DwindleLayout, GridLayout, MonocleLayout, StripLayout};
use gnoblin_std::objects::{
    Gaps, LayoutHandlers, LayoutPlacement, LayoutRect, LayoutSplit, MasterStackLayout, SplitAxis, Workspace, Workspaces,
};
use gnoblin_std::prelude::*;

/// Equal columns: the layout this shell implements.
fn columns() -> LayoutHandlers {
    LayoutHandlers::new(|context| {
        let area = context.area.clone();
        let count = context.children.len().max(1) as f64;
        let width = (area.width / count).floor();
        Ok(context
            .children
            .into_iter()
            .enumerate()
            .map(|(at, node)| {
                let mut placement = LayoutPlacement::new(node);
                placement.bounds = Some(LayoutRect {
                    x: area.x + width * at as f64,
                    y: area.y,
                    width,
                    height: area.height,
                });
                placement.tiled = true;
                placement
            })
            .collect())
    })
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

/// Gives `workspace` the layout the number `which` names. The layout objects the workspace takes stay alive in `keep`.
fn give_layout(
    objects: &gnoblin_std::objects::Objects,
    workspace: &Workspace,
    which: usize,
    keep: &RefCell<Vec<gnoblin_std::objects::Layout>>,
) -> gnoblin_std::Result<()> {
    let layout = match which {
        0 => MasterStackLayout::new(objects)?.as_layout()?,
        1 => GridLayout::new(objects)?.as_layout()?,
        2 => MonocleLayout::new(objects)?.as_layout()?,
        3 => DwindleLayout::new(objects)?.as_layout()?,
        4 => StripLayout::new(objects)?.as_layout()?,
        5 => ArcLayout::new(objects)?.as_layout()?,
        6 => Carousel3dLayout::new(objects)?.as_layout()?,
        _ => columns().implement(objects)?,
    };
    workspace.set_layout(&layout)?;
    keep.borrow_mut().push(layout);
    Ok(())
}

/// The ratio the pointer's travel from `from` to `now` carries a divider to, from `start`, held to the layout's `min..=max`: the divider's
/// `span` pixels of travel along its axis change the ratio by one, in the `direction` the layout says.
fn follow(
    axis: SplitAxis,
    direction: i64,
    span: f64,
    (min, max): (f64, f64),
    start: f64,
    from: (f64, f64),
    now: (f64, f64),
) -> f64 {
    let travel = match axis {
        SplitAxis::Horizontal => now.0 - from.0,
        SplitAxis::Vertical => now.1 - from.1,
    };
    (start + direction.signum() as f64 * travel / span).clamp(min, max)
}

/// A divider drag that is going: the divider, where the pointer pressed and the ratios the dividers had then.
struct Held {
    split: LayoutSplit,
    from: (f64, f64),
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let make = |id: &str, name: &str, gaps: Option<Gaps>| -> gnoblin_std::Result<Workspace> {
        let workspace = Workspace::new(&objects, id)?;
        workspace.set_name(name)?;
        if let Some(gaps) = gaps {
            workspace.set_gaps(&gaps)?;
        }
        workspace.set_layout(&MasterStackLayout::new(&objects)?.as_layout()?)?;
        Workspaces::get(&objects)?.add(&workspace)?;
        Ok(workspace)
    };
    let main = make(
        "main",
        "Main",
        Some(Gaps {
            inner: 6.0,
            outer: 12.0,
        }),
    )?;
    let side = make("side", "Side", None)?;
    let list = Workspaces::get(&objects)?;
    g.log(&format!(
        "workspaces: the list holds {} workspaces: {} at {}, {} at {}",
        list.length()?,
        main.id()?,
        main.index()?,
        side.id()?,
        side.index()?
    ));
    let held_layouts = Rc::new(RefCell::new(Vec::new()));
    if let Some(clients) = main.clients()? {
        let (main, g) = (main.clone(), g.clone());
        let counted = main.clients()?;
        clients
            .on_added(move |client| {
                let count = counted.as_ref().and_then(|clients| clients.length().ok()).unwrap_or(0);
                g.log(&format!(
                    "workspaces: client {} joined {}, {count} windows",
                    client.id().unwrap_or_default(),
                    main.id().unwrap_or_default()
                ));
            })?
            .keep();
    }

    let core = g.objects()?;
    let Some(context) = ModelRoot::get(&core)?.input()?.primary().ok() else {
        g.log("workspaces: the compositor reports no seat");
        return Ok(());
    };

    for (at, name) in NAMES.iter().enumerate() {
        let (workspace, g, objects, keep) = (main.clone(), g.clone(), objects.clone(), held_layouts.clone());
        context.bind(
            &format!("Super+{}", at + 1),
            Some(move |_| {
                let done = give_layout(&objects, &workspace, at, &keep).is_ok();
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
                let shown = workspace.show(None).is_ok();
                g.log(&format!(
                    "workspaces: show {}: {shown}, active {:?}",
                    workspace.id().unwrap_or_default(),
                    workspace.active().ok()
                ));
                Ok(true)
            }),
            None,
        )?;
    }

    // One function follows the drag from the press to the end. The press picks the window under the pointer.
    let held: Rc<RefCell<Option<Held>>> = Rc::new(RefCell::new(None));
    let Some(seat) = ModelRoot::get(&core)?.input()?.primary().ok() else {
        return Ok(());
    };
    let (workspace, shell) = (main.clone(), g.clone());
    context.bind(
        "Super+MouseLeft+Drag",
        Some(move |event: BindingEvent| {
            let at = (event.position.x, event.position.y);
            match event.phase {
                Some(BindingPhase::Start) => {
                    let hit = seat.hit_test(&InputPoint { x: at.0, y: at.1 })?;
                    // The window under the press; a layout whose divider does not depend on the point is asked about the first window.
                    let client = hit.map(|hit| hit.client).or_else(|| {
                        workspace
                            .clients()
                            .ok()
                            .flatten()
                            .and_then(|clients| clients.members().ok())
                            .and_then(|members| members.into_iter().next())
                    });
                    let Some(split) =
                        client.and_then(|client| workspace.find_split(&client, at.0, at.1).ok().flatten())
                    else {
                        shell.log("workspaces: the press is on no divider");
                        return Ok(false);
                    };
                    shell.log(&format!("workspaces: divider {} at {}", split.token, split.ratio));
                    *held.borrow_mut() = Some(Held { split, from: at });
                }
                Some(BindingPhase::Update) => {
                    if let Some(drag) = held.borrow().as_ref() {
                        // The pointer is where the press was, moved by the drag's distance.
                        let now = (
                            drag.from.0 + event.dx.unwrap_or(0.0),
                            drag.from.1 + event.dy.unwrap_or(0.0),
                        );
                        let split = &drag.split;
                        let ratio = follow(
                            split.axis,
                            split.direction,
                            split.span,
                            (split.min, split.max),
                            split.ratio,
                            drag.from,
                            now,
                        );
                        let cross = split.cross_axis.map(|axis| {
                            follow(
                                axis,
                                split.cross_direction,
                                split.cross_span,
                                (split.cross_min, split.cross_max),
                                split.cross_ratio,
                                drag.from,
                                now,
                            )
                        });
                        let _ = workspace.set_split(split.token, ratio, cross);
                    }
                }
                Some(BindingPhase::End) => {
                    if let Some(drag) = held.borrow_mut().take() {
                        shell.log(&format!("workspaces: divider {} dropped", drag.split.token));
                    }
                }
                Some(BindingPhase::Cancel) => {
                    if let Some(drag) = held.borrow_mut().take() {
                        let _ = workspace.reset_split(drag.split.token);
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
