//! Outlets: the named places of a frame for canvases. Where an outlet goes, when it shows, what room it takes, how thick the canvases in
//! an edge make it, and what it does when something is added or cleared.

use std::rc::Rc;

use gnoblin_core::Rect;

use super::client::Win;
use super::{FrameId, FrameKind, FramePart, FrameSlot, OutletPart, PartRole, Shape, en, part_of};
use crate::floating::Inset;
use crate::ui::widget::{Cid, Special, Ui, WidgetKind, en as ui_en};

/// The window's content size and the room its frame takes round it: what an outlet is placed from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) inset: Inset,
}

/// The part of an outlet canvas, with the frame it belongs to.
pub(crate) fn outlet_of(ui: &Ui, cid: Cid) -> Option<(FrameId, OutletPart)> {
    match part_of(ui, cid)? {
        FramePart {
            frame,
            role: PartRole::Outlet(outlet),
            ..
        } => Some((frame, outlet)),
        _ => None,
    }
}

impl Ui {
    /// Changes the outlet part of a canvas, without asking for a turn. Does nothing for a canvas that is not an outlet.
    pub(crate) fn tweak_outlet(&self, cid: Cid, change: impl FnOnce(&mut OutletPart)) {
        self.tweak(cid, |widget| {
            if let Special::Frame(FramePart {
                role: PartRole::Outlet(outlet),
                ..
            }) = &mut widget.sp
            {
                change(outlet);
            }
        });
    }

    /// Whether the outlet would show and take its room if the window were not fullscreen.
    pub(crate) fn outlet_enabled(&self, node: u64, part: &OutletPart) -> Result<bool, String> {
        if part.hidden {
            return Ok(false);
        }
        match &part.hide_when {
            Some(question) => Ok(!self.ask_window(question, node)?),
            None => Ok(true),
        }
    }

    /// Whether the outlet shows and takes its room now.
    pub(crate) fn outlet_shown(&self, node: u64, part: &OutletPart) -> Result<bool, String> {
        Ok(!Win::of(self, node).fullscreen() && self.outlet_enabled(node, part)?)
    }

    /// The room an edge keeps for itself while the window is not fullscreen: nothing while it is hidden.
    pub(crate) fn edge_reserved(&self, node: u64, part: &OutletPart) -> Result<f64, String> {
        let Shape::Edge(edge) = &part.shape else {
            return Ok(0.0);
        };
        Ok(if self.outlet_enabled(node, part)? {
            edge.size.unwrap_or(edge.measured)
        } else {
            0.0
        })
    }

    /// The room an edge takes now: nothing while the window is fullscreen.
    pub(crate) fn edge_thickness(&self, node: u64, part: &OutletPart) -> Result<f64, String> {
        if self.outlet_shown(node, part)? {
            self.edge_reserved(node, part)
        } else {
            Ok(0.0)
        }
    }

    /// The room a frame takes round its window's content: what the edges take now, or (`reserved`) what they keep even while fullscreen
    /// hides them. Nothing for a frame the client draws.
    pub(crate) fn frame_inset(&self, frame: FrameId, reserved: bool) -> Result<Inset, String> {
        let (node, kind, edges) = {
            let world = self.frames.borrow();
            let Some(rec) = world.frames.get(&frame) else {
                return Ok(Inset::default());
            };
            let edges: Vec<(u32, Cid)> = FrameSlot::EDGES
                .iter()
                .filter_map(|(slot, side)| rec.outlets.get(slot).map(|cid| (*side, *cid)))
                .collect();
            (rec.node, rec.kind, edges)
        };
        let mut inset = Inset::default();
        if kind != FrameKind::Server {
            return Ok(inset);
        }
        for (side, cid) in edges {
            let Some((_, part)) = outlet_of(self, cid) else {
                continue;
            };
            let room = if reserved {
                self.edge_reserved(node, &part)?
            } else {
                self.edge_thickness(node, &part)?
            };
            match side {
                en::SIDE_TOP => inset.top = room,
                en::SIDE_RIGHT => inset.right = room,
                en::SIDE_BOTTOM => inset.bottom = room,
                _ => inset.left = room,
            }
        }
        Ok(inset)
    }

    /// The window's content size and the inset now: what an outlet is placed from.
    pub(crate) fn frame_geometry(&self, frame: FrameId) -> Result<Geometry, String> {
        let Some(node) = self.frame_node(frame) else {
            return Ok(Geometry {
                width: 0.0,
                height: 0.0,
                inset: Inset::default(),
            });
        };
        let win = Win::of(self, node);
        Ok(Geometry {
            width: win.width(),
            height: win.height(),
            inset: self.frame_inset(frame, false)?,
        })
    }
}

/// The rectangle of an edge strip in the window's coordinates, from the content's size and the inset.
fn edge_rect(side: u32, geometry: &Geometry) -> Rect {
    let Geometry { width, height, inset } = *geometry;
    match side {
        en::SIDE_TOP => Rect::new(-inset.left, -inset.top, inset.left + width + inset.right, inset.top),
        en::SIDE_BOTTOM => Rect::new(-inset.left, height, inset.left + width + inset.right, inset.bottom),
        en::SIDE_LEFT => Rect::new(-inset.left, 0.0, inset.left, height),
        _ => Rect::new(width, 0.0, inset.right, height),
    }
}

/// The square at one corner of the frame, in the window's coordinates.
fn corner_rect(geometry: &Geometry, corner: u32, size: Option<f64>) -> Rect {
    let Geometry { width, height, inset } = *geometry;
    let top = matches!(corner, en::CORNER_TOP_LEFT | en::CORNER_TOP_RIGHT);
    let left = matches!(corner, en::CORNER_TOP_LEFT | en::CORNER_BOTTOM_LEFT);
    let side = size.unwrap_or_else(|| {
        (if top { inset.top } else { inset.bottom }).max(if left { inset.left } else { inset.right })
    });
    let (frame_left, frame_top) = (-inset.left, -inset.top);
    let (frame_right, frame_bottom) = (width + inset.right, height + inset.bottom);
    Rect::new(
        if left { frame_left } else { frame_right - side },
        if top { frame_top } else { frame_bottom - side },
        side,
        side,
    )
}

/// Where the outlet goes in the window's coordinates, or `None` when its parent's layout places it.
fn place(shape: &Shape, geometry: &Geometry) -> Option<Rect> {
    match shape {
        Shape::Zone { .. } => None,
        Shape::Edge(edge) => Some(edge_rect(edge.side, geometry)),
        Shape::Corner { corner, size } => Some(corner_rect(geometry, *corner, *size)),
        Shape::Layer { bleed, .. } => {
            let Geometry { width, height, inset } = *geometry;
            Some(Rect::new(
                -inset.left - bleed,
                -inset.top - bleed,
                inset.left + width + inset.right + 2.0 * bleed,
                inset.top + height + inset.bottom + 2.0 * bleed,
            ))
        }
    }
}

/// Updates the outlet's placement, visibility and refresh callback.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid, frame: FrameId, part: &OutletPart) -> Result<(), String> {
    let Some(node) = ui.frame_node(frame) else {
        return Ok(());
    };
    let geometry = ui.frame_geometry(frame)?;
    if let Some(rect) = place(&part.shape, &geometry) {
        ui.tweak(cid, |widget| {
            widget.x = rect.x;
            widget.y = rect.y;
            widget.width = Some(rect.width);
            widget.height = Some(rect.height);
        });
    }
    let shown = ui.outlet_shown(node, part)?;
    ui.tweak(cid, |widget| widget.visible = shown);
    // The stock titlebar's edge is as high as the bar's style says: the bar's height is read once a frame, after the edge was placed.
    if let Shape::Edge(edge) = &part.shape
        && let Some(bar) = edge.bar
    {
        let height = super::titlebar::bar_style(ui, bar).height;
        ui.tweak_outlet(cid, |held| {
            if let Shape::Edge(held) = &mut held.shape {
                held.size = Some(height);
            }
        });
    }
    if let Some(hook) = &part.on_refresh {
        hook.call(0, &[]).map_err(|fail| fail.message)?;
    }
    Ok(())
}

/// The natural width (or height) of an outlet: the room the canvases under it take along its row or column, with the padding and the
/// gaps.
pub(crate) fn natural(ui: &Ui, cid: Cid, vertical: bool) -> f64 {
    let (direction, gap, padding, children) = ui
        .read(cid, |widget| {
            (
                widget.b.direction,
                widget.b.gap,
                widget.b.padding,
                widget.children.clone(),
            )
        })
        .unwrap_or((ui_en::DIRECTION_NONE, 0.0, 0.0, Vec::new()));
    let along = if vertical {
        direction == ui_en::DIRECTION_COLUMN
    } else {
        direction == ui_en::DIRECTION_ROW
    };
    let sizes: Vec<f64> = children
        .into_iter()
        .map(|child| {
            if vertical {
                ui.height_of(child)
            } else {
                ui.width_of(child)
            }
        })
        .collect();
    let total = if along {
        sizes.iter().sum::<f64>() + gap * (sizes.len().saturating_sub(1) as f64)
    } else {
        sizes.iter().copied().fold(0.0, f64::max)
    };
    total + 2.0 * padding
}

/// Learns how thick a canvas added to the edge `edge` (or to one of its zones) is, when the edge has no size of its own.
fn note_thickness(ui: &Ui, edge: Cid, child: Cid) {
    let Some((_, part)) = outlet_of(ui, edge) else {
        return;
    };
    let Shape::Edge(shape) = &part.shape else {
        return;
    };
    let along = matches!(shape.side, en::SIDE_TOP | en::SIDE_BOTTOM);
    let own = if along { ui.height_of(child) } else { ui.width_of(child) };
    ui.tweak_outlet(edge, |held| {
        if let Shape::Edge(held) = &mut held.shape {
            held.measured = held.measured.max(own);
        }
    });
}

/// A canvas was added under `parent`: an edge, or one of its zones, learns how thick it is; a layer lets the pointer through it.
pub(crate) fn added(ui: &Rc<Ui>, parent: Cid, child: Cid) {
    let Some((_, part)) = outlet_of(ui, parent) else {
        return;
    };
    match part.shape {
        Shape::Edge(_) => note_thickness(ui, parent, child),
        Shape::Zone { edge } => note_thickness(ui, edge, child),
        Shape::Layer { .. } => {
            ui.tweak(child, |widget| widget.input = false);
        }
        Shape::Corner { .. } => {}
    }
}

/// Destroys every canvas and node that was added to the outlet and leaves it empty. An edge forgets how thick its canvases were, and its
/// zones go with them.
pub(crate) fn clear(ui: &Rc<Ui>, cid: Cid) {
    let Some((_, part)) = outlet_of(ui, cid) else {
        return;
    };
    let children = ui.read(cid, |widget| widget.children.clone()).unwrap_or_default();
    for child in children {
        ui.destroy(child);
    }
    for node in &part.nodes {
        // A node the compositor already took is not an error.
        let _ = ui.node(*node).destroy();
    }
    ui.tweak_outlet(cid, |held| {
        held.nodes.clear();
        if let Shape::Edge(edge) = &mut held.shape {
            edge.zones = None;
            edge.measured = 0.0;
        }
    });
    ui.schedule();
}

/// A frame's canvas is an outlet of kind `Box`; its class is named by its shape.
pub(crate) fn class_of(shape: &Shape) -> &'static str {
    match shape {
        Shape::Zone { .. } => "outlet",
        Shape::Edge(_) => "edge_outlet",
        Shape::Corner { .. } => "corner_outlet",
        Shape::Layer { .. } => "layer_outlet",
    }
}

/// The direction an edge's canvases are laid out in: a row along the top and the bottom, a column along the sides.
pub(crate) fn edge_direction(side: u32) -> u32 {
    if matches!(side, en::SIDE_TOP | en::SIDE_BOTTOM) {
        ui_en::DIRECTION_ROW
    } else {
        ui_en::DIRECTION_COLUMN
    }
}

/// The kind of canvas every outlet is.
pub(crate) const KIND: WidgetKind = WidgetKind::Box;
