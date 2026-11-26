//! The room: a node named `room` with the head camera and a panel (a `view`) for each window, placed on the ring of its layout.
//!
//! A panel follows its window: it is made when the window maps, takes the window's size, is hidden while the window is minimised and goes
//! when the window closes or turns secure. The room never changes a window. A panel that a drag moved is pinned, and the ring leaves it
//! where it was put until `unpin`.
//!
//! A dialog is a window of its own (its `parent_window` names the window it belongs to) and gets a panel of its own. The panel is a child of
//! its parent's panel, in the parent's pixels: it sits where the dialog sits on the parent window, a little in front of it, and it moves,
//! turns and scales with the parent's panel. A dialog whose parent has no panel is placed on the ring like any window.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use gnoblin_core::NodeId;
use gnoblin_object::server::Cx;
use gnoblin_object::{Closure, Fail, Obj, Res, Val};

use super::XrWorld;
use super::layout::{LayoutData, Spacing};
use super::ring::{self, RingParams};
use crate::animation::staging::Prop;
use crate::scene::Kind;

/// How far in front of its parent's panel a dialog's panel sits, in the parent's pixels: enough to draw over it, and to be picked first.
const DIALOG_LIFT: f64 = 2.0;

/// The metres of a room unit when the head camera says none: the schema's default of a camera's `metres`.
pub(crate) const DEFAULT_METRES: f64 = 0.001;

/// A panel: the window it shows and the view that shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Panel {
    pub(crate) client: u64,
    pub(crate) view: u64,
    pub(crate) obj: Obj,
}

/// What the library keeps of one room.
pub(crate) struct RoomData {
    pub(crate) obj: Obj,
    pub(crate) node: u64,
    /// The camera the program writes the head pose onto: its node and its object.
    pub(crate) head: Cell<(u64, Obj)>,
    /// The layout, held, or none to leave the panels where they are.
    pub(crate) layout: RefCell<Option<(Obj, Rc<LayoutData>)>>,
    pub(crate) accept: RefCell<Option<Closure>>,
    /// The ring's panels, in the order their windows gained them.
    pub(crate) panels: RefCell<Vec<Panel>>,
    /// The dialogs' panels, each with its parent window.
    pub(crate) dialogs: RefCell<Vec<(Panel, u64)>>,
    /// The panels the ring leaves alone, by view.
    pub(crate) pinned: RefCell<BTreeSet<u64>>,
    /// The panels fixed to the head.
    pub(crate) head_panels: RefCell<Vec<Panel>>,
    /// The headset draws the panels through the head camera (`projection` composition).
    pub(crate) projection: Cell<bool>,
}

impl RoomData {
    /// The ring of the room's layout, or none.
    pub(crate) fn ring(&self) -> Option<RingParams> {
        self.layout
            .borrow()
            .as_ref()
            .map(|(_, layout)| layout.spacing.get().ring())
    }

    pub(crate) fn is_pinned(&self, view: u64) -> bool {
        self.pinned.borrow().contains(&view)
    }

    pub(crate) fn pin(&self, view: u64) {
        self.pinned.borrow_mut().insert(view);
    }

    pub(crate) fn unpin(&self, view: u64) {
        self.pinned.borrow_mut().remove(&view);
    }

    /// The panel of `client`: its ring panel, or its dialog panel.
    pub(crate) fn panel_of(&self, client: u64) -> Option<Panel> {
        self.panels
            .borrow()
            .iter()
            .copied()
            .find(|panel| panel.client == client)
            .or_else(|| {
                self.dialogs
                    .borrow()
                    .iter()
                    .map(|(panel, _)| *panel)
                    .find(|panel| panel.client == client)
            })
    }
}

/// Sets a room up on the node `node` held by `obj`: names it `room`, puts the floor an eye height below the viewer and makes the head
/// camera under it. A room given no layout gets the default one.
///
/// # Errors
///
/// A node could not be made or written.
pub(crate) fn setup(
    world: &XrWorld,
    cx: &mut Cx<'_>,
    node: u64,
    obj: Obj,
    layout: Option<(Obj, Rc<LayoutData>)>,
    accept: Option<Closure>,
) -> Res<Rc<RoomData>> {
    let layout = match layout {
        Some(given) => given,
        None => super::classes::new_layout(world, cx, Spacing::default(), super::layout::Triggers::default())?,
    };
    cx.hold(layout.0)?;
    let eye = layout.1.spacing.get().ring().eye;
    world.put(node, "name", Prop::Text("room".to_owned()))?;
    world.put(node, "y", Prop::Number(-eye))?;
    let (head, head_obj) = world.spawn_node(cx, "xr_camera3d", Kind::Camera3D)?;
    super::classes::camera_setup(world, head, DEFAULT_METRES, None, None)?;
    world.scene.insert(head, node, None)?;
    cx.set_parent(head_obj, obj)?;
    let data = Rc::new(RoomData {
        obj,
        node,
        head: Cell::new((head, head_obj)),
        layout: RefCell::new(Some(layout)),
        accept: RefCell::new(accept),
        panels: RefCell::new(Vec::new()),
        dialogs: RefCell::new(Vec::new()),
        pinned: RefCell::new(BTreeSet::new()),
        head_panels: RefCell::new(Vec::new()),
        projection: Cell::new(false),
    });
    world.rooms.borrow_mut().insert(node, Rc::clone(&data));
    Ok(data)
}

/// The stock room: a room object the library made, with `layout` (held already) or the default one, and `accept`.
///
/// # Errors
///
/// A node could not be made or written.
pub(crate) fn build(
    world: &XrWorld,
    cx: &mut Cx<'_>,
    layout: Option<(Obj, Rc<LayoutData>)>,
    accept: Option<Closure>,
) -> Res<Rc<RoomData>> {
    let (node, obj) = world.spawn_node(cx, "xr_room", Kind::Node)?;
    match setup(world, cx, node, obj, layout, accept) {
        Ok(data) => Ok(data),
        Err(fail) => {
            world.destroy(obj);
            Err(fail)
        }
    }
}

/// What the room reads of one window.
struct RoomWindow {
    node: u64,
    secure: bool,
    minimised: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app_id: String,
    title: String,
    /// The window it belongs to, when it is a dialog whose parent is open.
    parent: Option<u64>,
}

impl RoomWindow {
    /// The `xr_client` record `accept` is told.
    fn record(&self) -> Val {
        Val::Record {
            class: 0,
            fields: vec![
                Val::Text(self.node.to_string()),
                Val::Text(self.app_id.clone()),
                Val::Text(self.title.clone()),
                Val::Double(self.width),
                Val::Double(self.height),
                Val::Bool(self.minimised),
                Val::Bool(self.parent.is_some()),
            ],
        }
    }
}

/// The open windows, in the order they came into the tree.
fn windows(world: &XrWorld) -> Vec<RoomWindow> {
    let nodes: Vec<u64> = world
        .g
        .windows()
        .map(|list| list.iter().map(|window| window.id().0).collect())
        .unwrap_or_default();
    let read = |node: u64| -> RoomWindow {
        let g = &world.g;
        let handle = g.node(NodeId(node));
        let parent = handle
            .get_node("parent_window")
            .ok()
            .flatten()
            .map(|parent| parent.id().0)
            .filter(|parent| *parent != node && nodes.contains(parent));
        RoomWindow {
            node,
            secure: crate::nodes::flag(g, node, "secure"),
            minimised: crate::nodes::flag(g, node, "minimised"),
            x: crate::nodes::number(g, node, "x"),
            y: crate::nodes::number(g, node, "y"),
            width: crate::nodes::number(g, node, "width"),
            height: crate::nodes::number(g, node, "height"),
            app_id: handle.get_text("app_id").unwrap_or_default(),
            title: handle.get_text("title").unwrap_or_default(),
            parent,
        }
    };
    nodes
        .iter()
        .copied()
        .filter(|node| world.g.node(NodeId(*node)).is_alive())
        .map(read)
        .collect()
}

/// Makes a panel of `window` under `parent` (the room, a parent's panel or the head camera), held by the object `owner`.
fn add_panel(world: &XrWorld, cx: &mut Cx<'_>, window: &RoomWindow, parent: u64, owner: Obj) -> Res<Panel> {
    let (view, obj) = world.spawn_node(cx, "view", Kind::View)?;
    let made = (|| -> Res<()> {
        world.put(view, "source", Prop::Node(window.node))?;
        world.put(view, "width", Prop::Number(window.width))?;
        world.put(view, "height", Prop::Number(window.height))?;
        world.put(view, "visible", Prop::Bool(!window.minimised))?;
        world.put(view, "input", Prop::Text("forward".to_owned()))?;
        world.scene.insert(view, parent, None)?;
        cx.set_parent(obj, owner)
    })();
    if let Err(fail) = made {
        world.destroy(obj);
        return Err(fail);
    }
    Ok(Panel {
        client: window.node,
        view,
        obj,
    })
}

/// Whether a panel's view still shows its window.
fn holds(world: &XrWorld, panel: &Panel) -> bool {
    world.scene.alive(panel.view)
        && matches!(world.read(panel.view, "source"), Some(Prop::Node(source)) if source == panel.client)
}

/// Asks the room's `accept` about each window. A throw is reported once and counts as no.
fn accepted(world: &XrWorld, room: &RoomData, all: &[RoomWindow]) -> Vec<bool> {
    let accept = room.accept.borrow().clone();
    let Some(accept) = accept else {
        return vec![true; all.len()];
    };
    let mut reported = false;
    all.iter()
        .map(
            |window| match accept.call(0, &[window.record()]).and_then(|answer| answer.bool()) {
                Ok(yes) => yes,
                Err(fail) => {
                    if !reported {
                        world.warn(&format!("XRRoom.accept threw: {}", fail.message));
                        reported = true;
                    }
                    false
                }
            },
        )
        .collect()
}

/// Brings the room's panels up to date with the windows and the focus, and places them. Writes only what changed.
pub(crate) fn reconcile(world: &XrWorld, cx: &mut Cx<'_>, room: &RoomData) {
    if room.layout.borrow().is_none() {
        return;
    }
    let all = windows(world);
    let accepted = accepted(world, room, &all);
    let ok = |at: usize| !all[at].secure && accepted[at];
    // A window that is not a dialog, or a dialog whose parent has no panel, is on the ring.
    let ringed: Vec<bool> = (0..all.len())
        .map(|at| {
            ok(at)
                && match all[at].parent {
                    None => true,
                    Some(parent) => !all
                        .iter()
                        .position(|window| window.node == parent)
                        .is_some_and(|parent_at| ok(parent_at) && all[parent_at].parent.is_none()),
                }
        })
        .collect();
    let anchored: Vec<Option<u64>> = (0..all.len())
        .map(|at| all[at].parent.filter(|_| ok(at) && !ringed[at]))
        .collect();
    let on_ring = |client: u64| {
        all.iter()
            .position(|window| window.node == client)
            .is_some_and(|at| ringed[at])
    };

    // Panels whose window went, turned secure, was refused, or whose view was taken.
    let gone: Vec<Panel> = {
        let mut panels = room.panels.borrow_mut();
        let (keep, gone): (Vec<Panel>, Vec<Panel>) = panels
            .drain(..)
            .partition(|panel| on_ring(panel.client) && holds(world, panel));
        *panels = keep;
        gone
    };
    for panel in gone {
        room.unpin(panel.view);
        world.destroy(panel.obj);
    }
    let gone: Vec<Panel> = {
        let ring: Vec<u64> = room.panels.borrow().iter().map(|panel| panel.client).collect();
        let mut dialogs = room.dialogs.borrow_mut();
        let (keep, gone): (Vec<_>, Vec<_>) = dialogs.drain(..).partition(|(panel, parent)| {
            let still = all
                .iter()
                .position(|window| window.node == panel.client)
                .is_some_and(|at| anchored[at] == Some(*parent));
            still && ring.contains(parent) && holds(world, panel)
        });
        *dialogs = keep;
        gone.into_iter().map(|(panel, _)| panel).collect()
    };
    for panel in gone {
        world.destroy(panel.obj);
    }

    // New panels: the ring's in the order the windows opened, then the dialogs'.
    for (at, window) in all.iter().enumerate() {
        if ringed[at] && room.panels.borrow().iter().all(|panel| panel.client != window.node) {
            match add_panel(world, cx, window, room.node, room.obj) {
                Ok(panel) => room.panels.borrow_mut().push(panel),
                Err(fail) => world.warn(&format!("XRRoom: a panel could not be made: {}", fail.message)),
            }
        }
    }
    for (at, window) in all.iter().enumerate() {
        let Some(parent) = anchored[at] else {
            continue;
        };
        if room
            .dialogs
            .borrow()
            .iter()
            .any(|(panel, _)| panel.client == window.node)
        {
            continue;
        }
        let Some(host) = room
            .panels
            .borrow()
            .iter()
            .copied()
            .find(|panel| panel.client == parent)
        else {
            continue;
        };
        match add_panel(world, cx, window, host.view, host.obj) {
            Ok(panel) => room.dialogs.borrow_mut().push((panel, parent)),
            Err(fail) => world.warn(&format!("XRRoom: a dialog's panel could not be made: {}", fail.message)),
        }
    }

    let camera = if room.projection.get() { room.head.get().0 } else { 0 };
    let window_of = |client: u64| all.iter().find(|window| window.node == client);
    let mut result = Ok(());
    let mut note = |done: Res<()>| {
        if result.is_ok() {
            result = done;
        }
    };

    // The ring.
    let panels: Vec<Panel> = room.panels.borrow().clone();
    let mut hidden = Vec::new();
    let mut ring_panels: Vec<(Panel, f64)> = Vec::new();
    let mut pinned = Vec::new();
    for panel in &panels {
        let Some(window) = window_of(panel.client) else {
            continue;
        };
        note(world.put(panel.view, "width", Prop::Number(window.width)));
        note(world.put(panel.view, "height", Prop::Number(window.height)));
        note(world.put(panel.view, "visible", Prop::Bool(!window.minimised)));
        note(world.put(panel.view, "camera", Prop::Node(camera)));
        if window.minimised {
            hidden.push(panel.view);
        } else if room.is_pinned(panel.view) {
            pinned.push(panel.view);
        } else {
            ring_panels.push((*panel, window.width));
        }
    }
    let focus = crate::nodes::seat(&world.g)
        .and_then(|seat| seat.get_node("focus").ok().flatten())
        .map(|node| node.id().0);
    let focus_at = focus.and_then(|focus| ring_panels.iter().position(|(panel, _)| panel.client == focus));
    let params = room.ring().unwrap_or_default();
    let widths: Vec<f64> = ring_panels.iter().map(|(_, width)| *width).collect();
    let placed = ring::arrange(&widths, focus_at, &params);
    for ((panel, _), pose) in ring_panels.iter().zip(&placed.poses) {
        note(world.put(panel.view, "x", Prop::Number(pose.x)));
        note(world.put(panel.view, "y", Prop::Number(pose.y)));
        note(world.put(panel.view, "z", Prop::Number(pose.z)));
        note(world.put(panel.view, "rotation", Prop::Triple([0.0, pose.yaw, 0.0])));
        note(world.put(panel.view, "scale", Prop::Triple([pose.scale; 3])));
    }

    // Back to front: the program submits the views in tree order and nothing depth-tests, so the nearest is last. A pinned panel goes
    // where its distance puts it among the ring's.
    let mut back: Vec<u64> = placed.order.iter().map(|at| ring_panels[*at].0.view).collect();
    let distance = |view: u64| world.world_pose(view).position.length();
    for view in pinned {
        let own = distance(view);
        let at = back
            .iter()
            .position(|other| distance(*other) < own)
            .unwrap_or(back.len());
        back.insert(at, view);
    }
    let want: Vec<u64> = hidden.into_iter().chain(back).collect();
    let current: Vec<u64> = world
        .scene
        .children_of(room.node)
        .into_iter()
        .map(|(node, _)| node)
        .filter(|node| want.contains(node))
        .collect();
    if current != want {
        for view in &want {
            note(world.scene.insert(*view, room.node, None));
        }
    }

    // The dialogs, where they sit on their parent windows.
    let dialogs: Vec<(Panel, u64)> = room.dialogs.borrow().clone();
    for (panel, parent) in dialogs {
        let (Some(dialog), Some(host)) = (window_of(panel.client), window_of(parent)) else {
            continue;
        };
        let dx = (dialog.x + dialog.width / 2.0) - (host.x + host.width / 2.0);
        let dy = (dialog.y + dialog.height / 2.0) - (host.y + host.height / 2.0);
        note(world.put(panel.view, "width", Prop::Number(dialog.width)));
        note(world.put(panel.view, "height", Prop::Number(dialog.height)));
        note(world.put(panel.view, "visible", Prop::Bool(!dialog.minimised)));
        note(world.put(panel.view, "camera", Prop::Node(camera)));
        note(world.put(panel.view, "x", Prop::Number(dx)));
        // The tree's y grows down on a window and up in the room.
        note(world.put(panel.view, "y", Prop::Number(-dy)));
        note(world.put(panel.view, "z", Prop::Number(DIALOG_LIFT)));
    }

    // The head's panels.
    let head_panels: Vec<Panel> = room.head_panels.borrow().clone();
    let mut kept = Vec::new();
    for panel in head_panels {
        match window_of(panel.client) {
            Some(window) if !window.secure && holds(world, &panel) => {
                note(world.put(panel.view, "width", Prop::Number(window.width)));
                note(world.put(panel.view, "height", Prop::Number(window.height)));
                note(world.put(panel.view, "visible", Prop::Bool(!window.minimised)));
                kept.push(panel);
            }
            _ => world.destroy(panel.obj),
        }
    }
    *room.head_panels.borrow_mut() = kept;
    if let Err(fail) = result {
        world.warn(&format!("XRRoom.reconcile: {}", fail.message));
    }
}

/// Shows `client` in a panel under the head camera, `x`, `y` and `z` metres from the eyes, at `scale` millimetres a pixel (the ring's when
/// none is given).
///
/// # Errors
///
/// The window is gone, or a node could not be made.
pub(crate) fn pin_to_head(
    world: &XrWorld,
    cx: &mut Cx<'_>,
    room: &RoomData,
    client: u64,
    offset: [f64; 3],
    scale: Option<f64>,
) -> Res<Panel> {
    let window = windows(world)
        .into_iter()
        .find(|window| window.node == client)
        .ok_or_else(|| Fail::no_such(format!("XRRoom.pinToHead: the window {client} is gone")))?;
    let metres = unit(world, room);
    let scale = scale.unwrap_or_else(|| room.ring().unwrap_or_default().mm_per_px);
    let (head, head_obj) = room.head.get();
    let panel = add_panel(world, cx, &window, head, head_obj)?;
    let placed = (|| -> Res<()> {
        world.put(panel.view, "x", Prop::Number(offset[0] / metres))?;
        world.put(panel.view, "y", Prop::Number(offset[1] / metres))?;
        world.put(panel.view, "z", Prop::Number(offset[2] / metres))?;
        world.put(panel.view, "scale", Prop::Triple([scale; 3]))
    })();
    if let Err(fail) = placed {
        world.destroy(panel.obj);
        return Err(fail);
    }
    room.head_panels.borrow_mut().push(panel);
    Ok(panel)
}

/// Metres in one room unit: the head camera's.
pub(crate) fn unit(world: &XrWorld, room: &RoomData) -> f64 {
    match world.read(room.head.get().0, "metres") {
        Some(Prop::Number(metres)) if metres > 0.0 => metres,
        _ => DEFAULT_METRES,
    }
}

/// The panel the ray strikes, and where: the last of the room's views first, because the room lists them back to front. A panel that is
/// hidden or takes no input is passed over, and a quad is one-sided, so the ray must travel towards its front.
pub(crate) fn pick(
    world: &XrWorld,
    room: &RoomData,
    ray: crate::math::vector::RayLine,
) -> Option<(Panel, crate::math::vector::Point3)> {
    let panels = room.panels.borrow().clone();
    let children: Vec<u64> = world
        .scene
        .children_of(room.node)
        .into_iter()
        .map(|(node, _)| node)
        .collect();
    for view in children.iter().rev() {
        let Some(panel) = panels.iter().copied().find(|panel| panel.view == *view) else {
            continue;
        };
        if matches!(world.read(panel.view, "visible"), Some(Prop::Bool(false))) {
            continue;
        }
        if matches!(world.read(panel.view, "input"), Some(Prop::Bool(false))) {
            continue;
        }
        let pose = world.world_pose(panel.view);
        let plane = pose.plane();
        if plane.normal.dot(ray.direction) >= 0.0 {
            continue;
        }
        let Some((point, _)) = plane.intersect(ray) else {
            continue;
        };
        let local = pose.inverse().transform_point(point);
        if local.x.abs() <= world.number(panel.view, "width") / 2.0
            && local.y.abs() <= world.number(panel.view, "height") / 2.0
        {
            return Some((panel, point));
        }
    }
    None
}
