//! The parts a frame is made of: the application's tile, the backdrop, the border, the glow, the resize handles, the title text and the
//! window buttons. What each does once a frame (`refresh`), how it paints, how big it is by itself, and what it does with the pointer.

mod app_icon;
mod buttons;
mod handles;
mod rings;

use std::rc::Rc;

use gnoblin_paint::PaintOp;

pub(crate) use app_icon::tile_of;
pub(crate) use buttons::{init_button, init_title, new_button, new_buttons, new_title, set_button_icons};
pub(crate) use handles::{build_handles, init_handle};

use super::client::Win;
use super::{FramePart, PartRole, Shape, outlet, titlebar};
use crate::ui::widget::{Cid, Ui, en as ui_en};

/// How a part names itself in a message: the class name a shell's log shows.
fn title_of(role: &PartRole) -> &'static str {
    match role {
        PartRole::Titlebar(_) => "Titlebar",
        PartRole::Button(_) => "WindowButton",
        PartRole::Handle(_) => "ResizeHandle",
        PartRole::Outlet(_) => "Outlet",
        _ => "DragArea",
    }
}

/// The bleed of the layer a canvas sits in: how far the layer reaches past the frame. Nothing for a canvas in another parent.
fn layer_bleed(ui: &Ui, parent: Cid) -> f64 {
    match outlet::outlet_of(ui, parent) {
        Some((_, part)) => match part.shape {
            Shape::Layer { bleed, .. } => bleed,
            _ => 0.0,
        },
        None => 0.0,
    }
}

/// Fits the canvas to its parent, less the bleed of the layer it is in when it is a ring or a backdrop (`with_bleed`): the frame's own edge.
fn fit_parent(ui: &Ui, cid: Cid, with_bleed: bool) {
    let Some(parent) = ui.read(cid, |widget| widget.parent).flatten() else {
        return;
    };
    let bleed = if with_bleed { layer_bleed(ui, parent) } else { 0.0 };
    let (width, height) = (ui.width_of(parent), ui.height_of(parent));
    ui.tweak(cid, |widget| {
        widget.x = bleed;
        widget.y = bleed;
        widget.width = Some(width - 2.0 * bleed);
        widget.height = Some(height - 2.0 * bleed);
    });
}

/// Updates a canvas of a frame from the window, once a frame.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid, part: &FramePart) -> Result<(), String> {
    let frame = part.frame;
    let Some(node) = ui.frame_node(frame) else {
        // A part that belongs to no frame (a glow, a backdrop) still fits its parent.
        match &part.role {
            PartRole::Glow(_) => fit_parent(ui, cid, false),
            PartRole::Backdrop | PartRole::Border(_) => fit_parent(ui, cid, true),
            _ => {}
        }
        return Ok(());
    };
    let win = Win::of(ui, node);
    match &part.role {
        PartRole::Outlet(outlet) => return outlet::refresh(ui, cid, frame, outlet),
        PartRole::Titlebar(bar) => return titlebar::refresh(ui, cid, frame, bar),
        PartRole::TitleText => {
            let title = win.title();
            ui.tweak(cid, |widget| widget.t.text = title);
        }
        PartRole::Button(button) => buttons::refresh(ui, cid, &win, button),
        PartRole::AppIcon(_) => app_icon::refresh(ui, cid, &win),
        PartRole::Backdrop | PartRole::Border(_) => fit_parent(ui, cid, true),
        PartRole::Glow(_) => fit_parent(ui, cid, false),
        PartRole::Handle(handle) => handles::refresh_handle(ui, cid, &win, handle.fit_parent),
        PartRole::Handles(_) => handles::refresh_handles(ui, cid),
        PartRole::DragArea | PartRole::Buttons(_) => {}
    }
    Ok(())
}

/// Places the children of a canvas that places them itself: the eight handles round a frame. `false` for any other canvas.
pub(crate) fn arrange(ui: &Ui, cid: Cid, part: &FramePart) -> bool {
    match &part.role {
        PartRole::Handles(handles) => {
            handles::arrange(ui, cid, handles);
            true
        }
        _ => false,
    }
}

/// The natural width (or height) of a canvas of a frame when it is not the toolkit's own.
pub(crate) fn natural(ui: &Ui, cid: Cid, part: &FramePart, vertical: bool) -> Option<f64> {
    match &part.role {
        PartRole::Outlet(_) => Some(outlet::natural(ui, cid, vertical)),
        PartRole::AppIcon(icon) => Some(icon.size),
        PartRole::Buttons(_) => Some(buttons::natural(ui, cid, vertical)),
        _ => None,
    }
}

/// What a canvas of a frame paints by itself when it is not the toolkit's own: the tile's letter, the ring, the glow. The handles paint
/// nothing.
pub(crate) fn paint(ui: &Ui, cid: Cid, part: &FramePart) -> Option<Result<Vec<PaintOp>, String>> {
    match &part.role {
        PartRole::AppIcon(icon) => Some(app_icon::paint(ui, cid, icon)),
        PartRole::Border(border) => Some(rings::border(ui, cid, part.frame, border)),
        PartRole::Glow(glow) => Some(rings::glow(ui, cid, glow)),
        PartRole::Handle(_) | PartRole::Handles(_) => Some(Ok(Vec::new())),
        _ => None,
    }
}

/// What a canvas of a frame does with a pointer event by itself, and whether it stops the event.
pub(crate) fn pointer(ui: &Rc<Ui>, cid: Cid, part: &FramePart, kind: u32, x: f64, y: f64, button: u64) -> bool {
    let title = title_of(&part.role);
    if let Some(drag) = part.drag {
        return match kind {
            ui_en::POINTER_PRESS => {
                if let Err(message) = super::drag::press(ui, drag, x, y, button) {
                    ui.warn(&format!("{title}.onPress threw: {message}"));
                }
                true
            }
            ui_en::POINTER_MOTION => super::drag::motion(ui, drag, x, y),
            ui_en::POINTER_RELEASE => match super::drag::release(ui, drag, x, y, button) {
                Ok(stopped) => stopped,
                Err(message) => {
                    ui.warn(&format!("{title}.onRelease threw: {message}"));
                    false
                }
            },
            _ => false,
        };
    }
    match &part.role {
        PartRole::Button(held) if kind == ui_en::POINTER_PRESS => {
            buttons::press(ui, part.frame, held.name, button, title)
        }
        PartRole::Handle(_) => handles::pointer(ui, cid, part.frame, (kind, x, y, button), title),
        _ => false,
    }
}
