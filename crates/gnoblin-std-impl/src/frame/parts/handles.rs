//! The resize handles: one canvas that resizes its window when it is dragged, and the eight round a frame.

use std::rc::Rc;

use gnoblin_core::host::Value;
use gnoblin_object::server::Cx;
use gnoblin_object::{Obj, Res};
use gnoblin_std_abi as abi;

use super::super::client::{Hook, Win};
use super::super::drag::LEFT;
use super::super::window_frame::link_under;
use super::super::{FrameId, FramePart, HandlePart, HandlesPart, PartRole, en};
use crate::bindings::steps::{MIN_SIZE, Sides, resized_rect};
use crate::builtin::Pt;
use crate::nodes;
use crate::ui::widget::{Cid, Special, Ui, WidgetKind, en as ui_en};

/// The order the eight handles are made and placed in.
pub(crate) const SIDES: [u32; 8] = [
    en::RESIZE_TOP_LEFT,
    en::RESIZE_TOP_RIGHT,
    en::RESIZE_BOTTOM_LEFT,
    en::RESIZE_BOTTOM_RIGHT,
    en::RESIZE_TOP,
    en::RESIZE_BOTTOM,
    en::RESIZE_LEFT,
    en::RESIZE_RIGHT,
];

/// The sides of the window a handle on `side` moves with the pointer.
fn sides_of(side: u32) -> Sides {
    Sides::from_flags(match side {
        en::RESIZE_TOP_LEFT => abi::STD_SIDE_TOP | abi::STD_SIDE_LEFT,
        en::RESIZE_TOP_RIGHT => abi::STD_SIDE_TOP | abi::STD_SIDE_RIGHT,
        en::RESIZE_BOTTOM_LEFT => abi::STD_SIDE_BOTTOM | abi::STD_SIDE_LEFT,
        en::RESIZE_BOTTOM_RIGHT => abi::STD_SIDE_BOTTOM | abi::STD_SIDE_RIGHT,
        en::RESIZE_LEFT => abi::STD_SIDE_LEFT,
        en::RESIZE_RIGHT => abi::STD_SIDE_RIGHT,
        en::RESIZE_TOP => abi::STD_SIDE_TOP,
        _ => abi::STD_SIDE_BOTTOM,
    })
}

/// Gives a canvas a resize handle's behaviour: it holds `side` of the window of `frame`.
pub(crate) fn init_handle(ui: &Ui, cid: Cid, frame: FrameId, side: u32, fit_parent: bool) {
    ui.tweak(cid, |widget| {
        widget.sp = Special::Frame(FramePart {
            frame,
            role: PartRole::Handle(HandlePart {
                side,
                fit_parent,
                from: None,
            }),
            drag: None,
        });
    });
}

/// Fills the canvas `handles` (the object `handles.0`, its canvas `handles.1`) with the eight handles round a frame, which lets the
/// pointer through everywhere but on a handle.
///
/// # Errors
///
/// A handle cannot be made or put under the canvas.
pub(crate) fn build_handles(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    frame: FrameId,
    handles: (Obj, Cid),
    thickness: f64,
) -> Res<()> {
    ui.tweak(handles.1, |widget| {
        widget.input = false;
        widget.sp = Special::Frame(FramePart {
            frame,
            role: PartRole::Handles(HandlesPart {
                thickness,
                inside: 2.0,
                corner: 14.0,
            }),
            drag: None,
        });
    });
    for side in SIDES {
        let handle = ui.spawn_named(cx, "resize_handle", WidgetKind::Box, handles.0)?;
        init_handle(ui, handle.1, frame, side, false);
        link_under(ui, cx, handles, handle)?;
    }
    Ok(())
}

/// Updates the handle's visibility and, when it is configured to and its parent leaves its canvases where they are, fits it to its parent.
pub(super) fn refresh_handle(ui: &Ui, cid: Cid, win: &Win<'_>, fit_parent: bool) {
    let visible = !win.maximised() && !win.fullscreen();
    let parent = ui.read(cid, |widget| widget.parent).flatten();
    let fit = parent.filter(|_| fit_parent).and_then(|parent| {
        ui.read(parent, |widget| {
            (widget.kind.is_box() && widget.b.direction == ui_en::DIRECTION_NONE).then_some(parent)
        })
        .flatten()
    });
    let size = fit.map(|parent| (ui.width_of(parent), ui.height_of(parent)));
    ui.tweak(cid, |widget| {
        widget.visible = visible;
        if let Some((width, height)) = size {
            widget.x = 0.0;
            widget.y = 0.0;
            widget.width = Some(width);
            widget.height = Some(height);
        }
    });
}

/// Sizes the eight handles' canvas to its parent, which is as large as the frame.
pub(super) fn refresh_handles(ui: &Ui, cid: Cid) {
    let Some(parent) = ui.read(cid, |widget| widget.parent).flatten() else {
        return;
    };
    if !ui.read(parent, |widget| widget.kind.is_box()).unwrap_or(false) {
        return;
    }
    let (width, height) = (ui.width_of(parent), ui.height_of(parent));
    ui.tweak(cid, |widget| {
        widget.x = 0.0;
        widget.y = 0.0;
        widget.width = Some(width);
        widget.height = Some(height);
    });
}

/// Places each edge and corner handle from the size of the canvas that holds them.
pub(super) fn arrange(ui: &Ui, cid: Cid, part: &HandlesPart) {
    let HandlesPart {
        thickness: t,
        inside,
        corner,
    } = *part;
    let reach = t + inside;
    let run = corner - t;
    let (width, height) = (ui.width_of(cid), ui.height_of(cid));
    let children = ui.read(cid, |widget| widget.children.clone()).unwrap_or_default();
    for child in children {
        let Some(side) = ui
            .read(child, |widget| match &widget.sp {
                Special::Frame(FramePart {
                    role: PartRole::Handle(handle),
                    ..
                }) => Some(handle.side),
                _ => None,
            })
            .flatten()
        else {
            continue;
        };
        let (x, y, w, h) = match side {
            en::RESIZE_TOP_LEFT => (-t, -t, corner, corner),
            en::RESIZE_TOP_RIGHT => (width - run, -t, corner, corner),
            en::RESIZE_BOTTOM_LEFT => (-t, height - run, corner, corner),
            en::RESIZE_BOTTOM_RIGHT => (width - run, height - run, corner, corner),
            en::RESIZE_TOP => (run, -t, width - 2.0 * run, reach),
            en::RESIZE_BOTTOM => (run, height - inside, width - 2.0 * run, reach),
            en::RESIZE_LEFT => (-t, run, reach, height - 2.0 * run),
            _ => (width - inside, run, reach, height - 2.0 * run),
        };
        ui.tweak(child, |widget| {
            widget.x = x;
            widget.y = y;
            widget.width = Some(w.max(0.0));
            widget.height = Some(h.max(0.0));
        });
    }
}

/// The least and the most the window of `node` may be resized to: what the client states, else the stock least.
fn limits(ui: &Ui, node: u64) -> ((f64, f64), Option<(f64, f64)>) {
    let state = crate::table::read(ui.ctx, |st| Ok(st.client_state_of(node)))
        .ok()
        .flatten();
    match state {
        Some(state) => (
            if state.flags & abi::STD_CLIENT_HAS_MIN != 0 {
                (state.min_width, state.min_height)
            } else {
                MIN_SIZE
            },
            (state.flags & abi::STD_CLIENT_HAS_MAX != 0).then_some((state.max_width, state.max_height)),
        ),
        None => (MIN_SIZE, None),
    }
}

fn set_from(ui: &Ui, cid: Cid, from: Option<(f64, f64)>) {
    ui.tweak(cid, |widget| {
        if let Special::Frame(FramePart {
            role: PartRole::Handle(handle),
            ..
        }) = &mut widget.sp
        {
            handle.from = from;
        }
    });
}

/// What a handle does with a pointer event (`kind`, `x`, `y`, `button`): a left press takes the window and starts a resize, a motion moves
/// the side or corner it holds by how far the pointer moved, a release ends it. Answers whether the event stops here.
pub(super) fn pointer(ui: &Rc<Ui>, cid: Cid, frame: FrameId, event: (u32, f64, f64, u64), title: &str) -> bool {
    let (kind, x, y, button) = event;
    let Some((side, from)) = super::super::part_of(ui, cid).and_then(|part| match part.role {
        PartRole::Handle(handle) => Some((handle.side, handle.from)),
        _ => None,
    }) else {
        return false;
    };
    match kind {
        ui_en::POINTER_PRESS => {
            if button == LEFT {
                if let Err(message) = ui.frame_button(frame, Hook::Press, x, y, button) {
                    ui.warn(&format!("{title}.onPress threw: {message}"));
                }
                set_from(ui, cid, Some((x, y)));
            }
            true
        }
        ui_en::POINTER_MOTION => {
            let (Some((from_x, from_y)), Some(node)) = (from, ui.frame_node_alive(frame)) else {
                return false;
            };
            let delta = Pt {
                x: x - from_x,
                y: y - from_y,
            };
            let (min, max) = limits(ui, node);
            let rect = resized_rect(nodes::rect_of(&ui.g, node), sides_of(side), delta, min, max);
            for (key, value) in [
                ("x", rect.x),
                ("y", rect.y),
                ("width", rect.width),
                ("height", rect.height),
            ] {
                if let Err(error) = ui.node(node).set(key, Value::Double(value)) {
                    ui.warn(&format!("the host refused {key} ({error})"));
                }
            }
            true
        }
        ui_en::POINTER_RELEASE => {
            if let Err(message) = ui.frame_button(frame, Hook::Release, x, y, button) {
                ui.warn(&format!("{title}.onRelease threw: {message}"));
                return false;
            }
            if from.is_some() {
                set_from(ui, cid, None);
                return true;
            }
            false
        }
        _ => false,
    }
}
