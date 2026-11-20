//! A window's frame: its outlets, made when first read and stacked in the window's children, and the frame's end.

use std::rc::Rc;

use gnoblin_object::server::Cx;
use gnoblin_object::{Fail, Obj, Res};

use super::outlet::{self, KIND};
use super::{EdgeShape, FrameId, FrameKind, FramePart, FrameSlot, OutletPart, PartRole, Shape, en};
use crate::object::with_lib;
use crate::ui::widget::{Cid, Special, Ui, en as ui_en};

/// The state of a `window_frame` object: the generation's toolkit and the frame.
pub(crate) struct FrameHandle {
    pub(crate) ui: Rc<Ui>,
    pub(crate) id: FrameId,
}

/// The state of a `window_actions` object: the frame whose window it acts on.
pub(crate) struct ActionsHandle {
    pub(crate) ui: Rc<Ui>,
    pub(crate) id: FrameId,
}

/// The name of an edge's side as a shell spells it.
pub(crate) fn side_name(side: u32) -> &'static str {
    match side {
        en::SIDE_TOP => "top",
        en::SIDE_RIGHT => "right",
        en::SIDE_BOTTOM => "bottom",
        _ => "left",
    }
}

/// The name of a corner as a shell spells it.
pub(crate) fn corner_name(corner: u32) -> &'static str {
    match corner {
        en::CORNER_TOP_LEFT => "top-left",
        en::CORNER_TOP_RIGHT => "top-right",
        en::CORNER_BOTTOM_LEFT => "bottom-left",
        _ => "bottom-right",
    }
}

/// The side of an edge slot, or the corner of a corner slot.
fn shape_of(slot: FrameSlot) -> Shape {
    let edge = |side| {
        Shape::Edge(EdgeShape {
            side,
            size: None,
            measured: 0.0,
            zones: None,
            bar: None,
        })
    };
    match slot {
        FrameSlot::Top => edge(en::SIDE_TOP),
        FrameSlot::Right => edge(en::SIDE_RIGHT),
        FrameSlot::Bottom => edge(en::SIDE_BOTTOM),
        FrameSlot::Left => edge(en::SIDE_LEFT),
        FrameSlot::TopLeft => Shape::Corner {
            corner: en::CORNER_TOP_LEFT,
            size: None,
        },
        FrameSlot::TopRight => Shape::Corner {
            corner: en::CORNER_TOP_RIGHT,
            size: None,
        },
        FrameSlot::BottomLeft => Shape::Corner {
            corner: en::CORNER_BOTTOM_LEFT,
            size: None,
        },
        FrameSlot::BottomRight => Shape::Corner {
            corner: en::CORNER_BOTTOM_RIGHT,
            size: None,
        },
        FrameSlot::Overlay => Shape::Layer { over: true, bleed: 0.0 },
        FrameSlot::Underlay => Shape::Layer {
            over: false,
            bleed: 0.0,
        },
    }
}

/// What a shell reads an outlet as in a message: `the top outlet`, `the top-left corner outlet`.
fn outlet_words(shape: &Shape) -> String {
    match shape {
        Shape::Edge(edge) => format!("the {} outlet", side_name(edge.side)),
        Shape::Corner { corner, .. } => format!("the {} corner outlet", corner_name(*corner)),
        Shape::Layer { over, .. } => format!("the {} outlet", if *over { "overlay" } else { "underlay" }),
        Shape::Zone { .. } => "the zone outlet".to_owned(),
    }
}

/// Puts the canvas `child` (the object `child_obj`) last under `parent` (the object `parent_obj`): the toolkit's tree, the compositor's and
/// the objects', as `canvas.add` does, and tells the outlet it holds one more.
pub(crate) fn link_under(ui: &Rc<Ui>, cx: &Cx<'_>, parent: (Obj, Cid), child: (Obj, Cid)) -> Res<()> {
    ui.link(parent.1, child.1)?;
    cx.set_parent(child.0, parent.0)?;
    super::added(ui, parent.1, child.1);
    ui.schedule();
    Ok(())
}

/// The outlet of `slot`, made on the first read: its object (owned by the frame's) and its canvas. A frame the client draws has the layers
/// only.
///
/// # Errors
///
/// The slot is an edge or a corner of a client-drawn frame, the frame was destroyed, or the compositor refuses the mounting.
pub(crate) fn outlet(ui: &Rc<Ui>, cx: &mut Cx<'_>, frame: FrameId, slot: FrameSlot) -> Res<(Obj, Cid)> {
    let (kind, node, frame_obj, existing, alive) = {
        let world = ui.frames.borrow();
        let rec = world
            .frames
            .get(&frame)
            .ok_or_else(|| Fail::stale("the frame was destroyed"))?;
        (rec.kind, rec.node, rec.obj, rec.outlets.get(&slot).copied(), rec.alive)
    };
    let shape = shape_of(slot);
    if kind != FrameKind::Server && !matches!(shape, Shape::Layer { .. }) {
        return Err(Fail::type_error(format!(
            "{} belongs to a window the shell frames: this window draws its own frame, so it has the underlay and the overlay only",
            outlet_words(&shape)
        )));
    }
    if let Some(cid) = existing
        && ui.alive(cid)
        && let Some(obj) = ui.read(cid, |widget| widget.obj)
    {
        return Ok((obj, cid));
    }
    if !alive {
        return Err(Fail::stale("the frame was destroyed"));
    }
    let name = match &shape {
        Shape::Edge(edge) => side_name(edge.side).to_owned(),
        Shape::Corner { corner, .. } => corner_name(*corner).to_owned(),
        Shape::Layer { over, .. } => (if *over { "overlay" } else { "underlay" }).to_owned(),
        Shape::Zone { .. } => String::new(),
    };
    let (obj, cid) = ui.spawn_named(cx, outlet::class_of(&shape), KIND, frame_obj)?;
    ui.tweak(cid, |widget| {
        match &shape {
            Shape::Edge(edge) => widget.b.direction = outlet::edge_direction(edge.side),
            Shape::Layer { over, .. } => {
                widget.placement = if *over {
                    ui_en::PLACEMENT_OVER
                } else {
                    ui_en::PLACEMENT_UNDER
                };
                widget.input = false;
            }
            _ => {}
        }
        widget.sp = Special::Frame(FramePart {
            frame,
            role: PartRole::Outlet(OutletPart {
                name,
                hidden: false,
                hide_when: None,
                on_refresh: None,
                nodes: Vec::new(),
                shape,
            }),
            drag: None,
        });
    });
    let above: Vec<Cid> = {
        let mut world = ui.frames.borrow_mut();
        let rec = world
            .frames
            .get_mut(&frame)
            .ok_or_else(|| Fail::stale("the frame was destroyed"))?;
        rec.outlets.insert(slot, cid);
        // Outlets made earlier that stack above this one are mounted again, so they stay above it. The underlay is painted behind the
        // window, whatever its place among the children.
        match FrameSlot::STACKING.iter().position(|held| *held == slot) {
            Some(at) => FrameSlot::STACKING[at + 1..]
                .iter()
                .filter_map(|held| rec.outlets.get(held).copied())
                .collect(),
            None => Vec::new(),
        }
    };
    ui.mount(cid, node)?;
    for held in above {
        ui.mount(held, node)?;
    }
    Ok((obj, cid))
}

/// The three zones of an edge, made the first time one is read: the start and the end take the size of their canvases, the centre takes
/// the room that is left.
///
/// # Errors
///
/// The canvases cannot be made or put under the edge.
pub(crate) fn zones(ui: &Rc<Ui>, cx: &mut Cx<'_>, frame: FrameId, edge: Cid) -> Res<[Cid; 3]> {
    let Some((_, part)) = outlet::outlet_of(ui, edge) else {
        return Err(Fail::type_error("the canvas is not an edge outlet"));
    };
    let Shape::Edge(shape) = &part.shape else {
        return Err(Fail::type_error("the canvas is not an edge outlet"));
    };
    if let Some(zones) = shape.zones {
        return Ok(zones);
    }
    let edge_obj = ui
        .read(edge, |widget| widget.obj)
        .ok_or_else(|| Fail::stale("the canvas is gone"))?;
    let direction = outlet::edge_direction(shape.side);
    let side = side_name(shape.side);
    let mut made = [0; 3];
    for (at, (name, grow)) in [("start", 0.0), ("center", 1.0), ("end", 0.0)].into_iter().enumerate() {
        let (obj, cid) = ui.spawn_named(cx, "outlet", KIND, edge_obj)?;
        ui.tweak(cid, |widget| {
            widget.b.direction = direction;
            widget.b.grow = grow;
            widget.sp = Special::Frame(FramePart {
                frame,
                role: PartRole::Outlet(OutletPart {
                    name: format!("{side}.{name}"),
                    hidden: false,
                    hide_when: None,
                    on_refresh: None,
                    nodes: Vec::new(),
                    shape: Shape::Zone { edge },
                }),
                drag: None,
            });
        });
        // A zone is put under its edge as a box is: the edge learns nothing from it.
        ui.link(edge, cid)?;
        cx.set_parent(obj, edge_obj)?;
        made[at] = cid;
    }
    ui.tweak_outlet(edge, |held| {
        if let Shape::Edge(edge) = &mut held.shape {
            edge.zones = Some(made);
        }
    });
    ui.schedule();
    Ok(made)
}

/// Takes the frame away: its outlets and everything in them are destroyed, and it no longer has a window.
pub(crate) fn destroy(ui: &Rc<Ui>, frame: FrameId) {
    let obj = {
        let mut world = ui.frames.borrow_mut();
        let Some(rec) = world.frames.remove(&frame) else {
            return;
        };
        if world.by_node.get(&rec.node) == Some(&frame) {
            world.by_node.remove(&rec.node);
        }
        world.drags.retain(|_, drag| drag.frame != frame);
        rec.obj
    };
    let ctx = ui.ctx;
    // The frame's object owns the outlets, the actions and what a shell put in them: they end with it, and so do their nodes.
    let _ = with_lib(|lib| lib.destroy(ctx, obj));
    ui.schedule();
}

/// Puts the eight resize handles round the frame, `thickness` pixels reaching outside it, in its overlay, and answers them.
///
/// # Errors
///
/// The handles cannot be made or put in the overlay.
pub(crate) fn resize_handles(ui: &Rc<Ui>, cx: &mut Cx<'_>, frame: FrameId, thickness: f64) -> Res<Obj> {
    let frame_obj = ui
        .frames
        .borrow()
        .frames
        .get(&frame)
        .map(|rec| rec.obj)
        .ok_or_else(|| Fail::stale("the frame was destroyed"))?;
    // The handles are made before the overlay they go in: the order of the nodes is the fixtures'.
    let handles = ui.spawn_named(cx, "resize_handles", KIND, frame_obj)?;
    super::parts::build_handles(ui, cx, frame, handles, thickness)?;
    let overlay = outlet(ui, cx, frame, FrameSlot::Overlay)?;
    link_under(ui, cx, overlay, handles)?;
    Ok(handles.0)
}
