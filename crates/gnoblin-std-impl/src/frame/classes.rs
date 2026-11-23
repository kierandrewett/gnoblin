//! The classes of `frame.toml` as the object protocol serves them.
//!
//! A canvas of a frame (an outlet, a part) is a canvas of the toolkit: its class registers the toolkit's members of its base classes and
//! its own, which read and write the [`FramePart`] the canvas carries. The frame, its actions, a drag behaviour and the decorations have
//! state of their own.

use std::borrow::Borrow;
use std::rc::Rc;

use gnoblin_object::server::{Builder, Cx, Inits};
use gnoblin_object::{Closure, Fail, Res, Val};

use super::client::Win;
use super::decorations::{self, DecoHandle, inset_val};
use super::drag;
use super::outlet as outlets;
use super::titlebar::{self, TitlebarStyle, icon_default, icons_of, icons_val, style_of, style_val};
use super::window_frame::{ActionsHandle, FrameHandle, zones};
use super::{
    AppIconPart, BorderPart, ButtonPart, ButtonsPart, FrameId, FramePart, FrameRule, FrameSlot, GlowPart, HandlePart,
    HandlesPart, OutletPart, PartRole, Shape, actions, en, parts,
};
use crate::object::client::client_node;
use crate::object::record::record;
use crate::ui::classes::{Class, canvas_of, class_with, enumeration, icon_members, text_members};
use crate::ui::tree::ui_of;
use crate::ui::widget::{CanvasHandle, Cid, Special, Ui, WidgetKind};
// ---- reading the part a canvas carries -----------------------------------------------------------------------------------------

/// Defines how the members of a class read (and, with a second name, change) the part of one role of the canvas they are about.
macro_rules! role_access {
    ($read:ident, $variant:ident, $part:ty, $what:expr) => {
        fn $read<R>(h: &CanvasHandle, read: impl FnOnce(&$part) -> R) -> Res<R> {
            h.read(|widget| match &widget.sp {
                Special::Frame(FramePart {
                    role: PartRole::$variant(part),
                    ..
                }) => Some(read(part)),
                _ => None,
            })?
            .ok_or_else(|| Fail::type_error(concat!("the canvas is not ", $what)))
        }
    };
    ($read:ident, $change:ident, $variant:ident, $part:ty, $what:expr) => {
        role_access!($read, $variant, $part, $what);

        fn $change(h: &CanvasHandle, change: impl FnOnce(&mut $part)) -> Res<()> {
            h.edit(|widget| {
                if let Special::Frame(FramePart {
                    role: PartRole::$variant(part),
                    ..
                }) = &mut widget.sp
                {
                    change(part);
                }
            })
        }
    };
}

role_access!(outlet_get, outlet_set, Outlet, OutletPart, "an outlet");
role_access!(button_get, button_set, Button, ButtonPart, "a window button");
role_access!(buttons_get, Buttons, ButtonsPart, "a row of window buttons");
role_access!(icon_get, icon_set, AppIcon, AppIconPart, "an application icon");
role_access!(border_get, border_set, Border, BorderPart, "a frame border");
role_access!(glow_get, glow_set, Glow, GlowPart, "a frame glow");
role_access!(handle_get, handle_set, Handle, HandlePart, "a resize handle");
role_access!(
    handles_get,
    handles_set,
    Handles,
    HandlesPart,
    "a set of resize handles"
);

/// The frame a canvas of a frame belongs to.
fn frame_of(h: &CanvasHandle) -> Res<FrameId> {
    h.read(|widget| match &widget.sp {
        Special::Frame(part) => Some(part.frame),
        _ => None,
    })?
    .ok_or_else(|| Fail::type_error("the canvas is not a part of a window frame"))
}

/// The `window_frame` object of a frame.
fn frame_object(ui: &Ui, frame: FrameId, cx: &Cx<'_>) -> Res<Val> {
    let obj = ui
        .frames
        .borrow()
        .frames
        .get(&frame)
        .map(|rec| rec.obj)
        .ok_or_else(|| Fail::stale("the window frame is gone"))?;
    cx.handout(obj)
}

/// The frame a `window_frame` value names.
fn frame_arg(cx: &Cx<'_>, value: &Val) -> Res<FrameId> {
    cx.with_state(value.object()?, |handle: &mut FrameHandle| handle.id)
}

/// A closure a shell gave, or none: a value that is none takes the closure away.
fn closure_of(value: impl Borrow<Val>) -> Res<Option<Closure>> {
    let value = value.borrow();
    if value.is_none() {
        Ok(None)
    } else {
        Ok(Some(Rc::clone(value.closure()?)))
    }
}

fn closure_val(closure: Option<Closure>) -> Val {
    closure.map_or(Val::None, Val::Closure)
}

/// The frame a part constructor was given.
fn frame_init(cx: &Cx<'_>, inits: &mut Inits) -> Res<FrameId> {
    let value = inits
        .take("frame")
        .ok_or_else(|| Fail::type_error("a part of a window frame is made for a frame"))?;
    frame_arg(cx, &value)
}

/// The `frame` member every part has.
fn frame_member(class: Class<'_>) -> Class<'_> {
    class.get("frame", |h, cx| frame_object(&h.ui, frame_of(h)?, cx))
}

// ---- the frame ------------------------------------------------------------------------------------------------------------------

fn frame_members(builder: &mut Builder) {
    let _ = builder
        .class::<FrameHandle>("window_frame")
        .ctor(|_cx, _inits| Err(Fail::refused("a window frame is made by the decorations")))
        .get("client", |h, _| {
            Ok(Val::Text(
                h.ui.frame_node(h.id).map_or_else(String::new, |node| node.to_string()),
            ))
        })
        .get("kind", |h, _| {
            let kind =
                h.ui.frame_kind(h.id)
                    .ok_or_else(|| Fail::stale("the window frame is gone"))?;
            Ok(enumeration(kind.value()))
        })
        .get("server", |h, _| {
            Ok(Val::Bool(h.ui.frame_kind(h.id) == Some(super::FrameKind::Server)))
        })
        .get("alive", |h, _| {
            let alive = h.ui.frames.borrow().frames.get(&h.id).is_some_and(|rec| rec.alive);
            Ok(Val::Bool(
                alive && h.ui.frame_node(h.id).is_some_and(|node| Win::of(&h.ui, node).alive()),
            ))
        })
        .get("inset", |h, _| {
            Ok(inset_val(&h.ui.frame_inset(h.id, false).map_err(Fail::err)?))
        })
        .get("reserved", |h, _| {
            Ok(inset_val(&h.ui.frame_inset(h.id, true).map_err(Fail::err)?))
        })
        .get("content", |h, _| {
            let geometry = h.ui.frame_geometry(h.id).map_err(Fail::err)?;
            Ok(record(vec![
                Val::Double(0.0),
                Val::Double(0.0),
                Val::Double(geometry.width),
                Val::Double(geometry.height),
                inset_val(&geometry.inset),
            ]))
        })
        .get("top", |h, cx| slot_val(h, cx, FrameSlot::Top))
        .get("bottom", |h, cx| slot_val(h, cx, FrameSlot::Bottom))
        .get("left", |h, cx| slot_val(h, cx, FrameSlot::Left))
        .get("right", |h, cx| slot_val(h, cx, FrameSlot::Right))
        .get("top_left", |h, cx| slot_val(h, cx, FrameSlot::TopLeft))
        .get("top_right", |h, cx| slot_val(h, cx, FrameSlot::TopRight))
        .get("bottom_left", |h, cx| slot_val(h, cx, FrameSlot::BottomLeft))
        .get("bottom_right", |h, cx| slot_val(h, cx, FrameSlot::BottomRight))
        .get("underlay", |h, cx| slot_val(h, cx, FrameSlot::Underlay))
        .get("overlay", |h, cx| slot_val(h, cx, FrameSlot::Overlay))
        .get("start", |h, cx| zone_val(h, cx, 0))
        .get("center", |h, cx| zone_val(h, cx, 1))
        .get("end", |h, cx| zone_val(h, cx, 2))
        .get("actions", |h, cx| {
            let actions =
                h.ui.frames
                    .borrow()
                    .frames
                    .get(&h.id)
                    .map(|rec| rec.actions)
                    .ok_or_else(|| Fail::stale("the window frame is gone"))?;
            cx.handout(actions)
        })
        .method("resize_handles", |h, cx, args| {
            let thickness = args
                .first()
                .filter(|value| !value.is_none())
                .map_or(Ok(6.0), Val::double)?;
            let handles = super::window_frame::resize_handles(&h.ui, cx, h.id, thickness)?;
            cx.handout(handles)
        });
}

/// The outlet of `slot`, made when first read.
fn slot_val(h: &FrameHandle, cx: &mut Cx<'_>, slot: FrameSlot) -> Res<Val> {
    let (obj, _) = super::window_frame::outlet(&h.ui, cx, h.id, slot)?;
    cx.handout(obj)
}

/// One zone of the top edge: `start` (0), `center` (1) or `end` (2). Reading one makes all three.
fn zone_val(h: &FrameHandle, cx: &mut Cx<'_>, which: usize) -> Res<Val> {
    let (_, edge) = super::window_frame::outlet(&h.ui, cx, h.id, FrameSlot::Top)?;
    let made = zones(&h.ui, cx, h.id, edge)?;
    let obj =
        h.ui.read(made[which], |widget| widget.obj)
            .ok_or_else(|| Fail::stale("the zone is gone"))?;
    cx.handout(obj)
}

// ---- outlets -------------------------------------------------------------------------------------------------------------------

/// The members every outlet has, on top of the box's.
fn outlet_members(class: Class<'_>) -> Class<'_> {
    class
        .get("name", |h, _| Ok(Val::Text(outlet_get(h, |part| part.name.clone())?)))
        .get("owner", |h, cx| frame_object(&h.ui, frame_of(h)?, cx))
        .get("hidden", |h, _| Ok(Val::Bool(outlet_get(h, |part| part.hidden)?)))
        .set("hidden", |h, _, v| {
            let hidden = v.bool()?;
            outlet_set(h, |part| part.hidden = hidden)
        })
        .get("hide_when", |h, _| {
            Ok(closure_val(outlet_get(h, |part| part.hide_when.clone())?))
        })
        .set("hide_when", |h, _, v| {
            let question = closure_of(v)?;
            outlet_set(h, |part| part.hide_when = question)
        })
        .get("on_refresh", |h, _| {
            Ok(closure_val(outlet_get(h, |part| part.on_refresh.clone())?))
        })
        .set("on_refresh", |h, _, v| {
            let hook = closure_of(v)?;
            outlet_set(h, |part| part.on_refresh = hook)
        })
        .get("shown", |h, _| {
            let part = outlet_get(h, Clone::clone)?;
            let node =
                h.ui.frame_node(frame_of(h)?)
                    .ok_or_else(|| Fail::stale("the window frame is gone"))?;
            Ok(Val::Bool(h.ui.outlet_shown(node, &part).map_err(Fail::err)?))
        })
        .get("enabled", |h, _| {
            let part = outlet_get(h, Clone::clone)?;
            let node =
                h.ui.frame_node(frame_of(h)?)
                    .ok_or_else(|| Fail::stale("the window frame is gone"))?;
            Ok(Val::Bool(h.ui.outlet_enabled(node, &part).map_err(Fail::err)?))
        })
        .method("add_node", |h, _, args| {
            let node = args[0].uint()?;
            let outlet = h.read(|widget| widget.node)?;
            h.ui.node(node)
                .insert_into(&h.ui.node(outlet), None)
                .map_err(|error| Fail::refused(format!("Outlet.addNode: the host refused ({error})")))?;
            outlet_set(h, |part| part.nodes.push(node))?;
            h.ui.schedule();
            Ok(Val::None)
        })
        .method("clear", |h, _, _| {
            outlets::clear(&h.ui, h.cid);
            Ok(Val::None)
        })
}

/// Registers an outlet class: a box nobody makes but a frame.
fn outlet_class<'a>(builder: &'a mut Builder, name: &str) -> Class<'a> {
    outlet_members(class_with(builder, name, WidgetKind::Box, |_ui, _cx, _inits, _cid| {
        Err(Fail::refused(
            "an outlet is made when a frame's outlet is first read (`frame.top`, `frame.overlay`)",
        ))
    }))
}

fn edge_size_members(class: Class<'_>) -> Class<'_> {
    class
}

fn outlet_classes(builder: &mut Builder) {
    let _ = outlet_class(builder, "outlet");
    let _ = edge_size_members(outlet_class(builder, "edge_outlet"))
        .get("side", |h, _| {
            Ok(enumeration(outlet_get(h, |part| match &part.shape {
                Shape::Edge(edge) => edge.side,
                _ => en::SIDE_TOP,
            })?))
        })
        .get("size", |h, _| {
            Ok(Val::Double(outlet_get(h, |part| match &part.shape {
                Shape::Edge(edge) => edge.size.unwrap_or(0.0),
                _ => 0.0,
            })?))
        })
        .set("size", |h, _, v| {
            let size = v.double()?;
            outlet_set(h, |part| {
                if let Shape::Edge(edge) = &mut part.shape {
                    edge.size = Some(size);
                }
            })
        })
        .get("reserved", |h, _| edge_room(h, true))
        .get("thickness", |h, _| edge_room(h, false))
        .get("start", |h, cx| edge_zone(h, cx, 0))
        .get("center", |h, cx| edge_zone(h, cx, 1))
        .get("end", |h, cx| edge_zone(h, cx, 2))
        .method("clear_size", |h, _, _| {
            outlet_set(h, |part| {
                if let Shape::Edge(edge) = &mut part.shape {
                    edge.size = None;
                }
            })?;
            Ok(Val::None)
        });
    let _ = outlet_class(builder, "corner_outlet")
        .get("corner", |h, _| {
            Ok(enumeration(outlet_get(h, |part| match &part.shape {
                Shape::Corner { corner, .. } => *corner,
                _ => en::CORNER_TOP_LEFT,
            })?))
        })
        .get("size", |h, _| {
            Ok(Val::Double(outlet_get(h, |part| match &part.shape {
                Shape::Corner { size, .. } => size.unwrap_or(0.0),
                _ => 0.0,
            })?))
        })
        .set("size", |h, _, v| {
            let size = v.double()?;
            outlet_set(h, |part| {
                if let Shape::Corner { size: held, .. } = &mut part.shape {
                    *held = Some(size);
                }
            })
        })
        .method("clear_size", |h, _, _| {
            outlet_set(h, |part| {
                if let Shape::Corner { size, .. } = &mut part.shape {
                    *size = None;
                }
            })?;
            Ok(Val::None)
        });
    let _ = outlet_class(builder, "layer_outlet")
        .get("kind", |h, _| {
            Ok(enumeration(outlet_get(h, |part| match &part.shape {
                Shape::Layer { over: true, .. } => 2,
                _ => 1,
            })?))
        })
        .get("bleed", |h, _| {
            Ok(Val::Double(outlet_get(h, |part| match &part.shape {
                Shape::Layer { bleed, .. } => *bleed,
                _ => 0.0,
            })?))
        })
        .set("bleed", |h, _, v| {
            let reach = v.double()?;
            outlet_set(h, |part| {
                if let Shape::Layer { bleed, .. } = &mut part.shape {
                    *bleed = reach;
                }
            })
        });
}

/// The room an edge keeps (`reserved`) or takes now.
fn edge_room(h: &CanvasHandle, reserved: bool) -> Res<Val> {
    let part = outlet_get(h, Clone::clone)?;
    let node =
        h.ui.frame_node(frame_of(h)?)
            .ok_or_else(|| Fail::stale("the window frame is gone"))?;
    let room = if reserved {
        h.ui.edge_reserved(node, &part)
    } else {
        h.ui.edge_thickness(node, &part)
    };
    Ok(Val::Double(room.map_err(Fail::err)?))
}

/// One zone of an edge: `start` (0), `center` (1) or `end` (2). Reading one makes all three.
fn edge_zone(h: &CanvasHandle, cx: &mut Cx<'_>, which: usize) -> Res<Val> {
    let made = zones(&h.ui, cx, frame_of(h)?, h.cid)?;
    let obj =
        h.ui.read(made[which], |widget| widget.obj)
            .ok_or_else(|| Fail::stale("the zone is gone"))?;
    cx.handout(obj)
}

// ---- actions and drag ---------------------------------------------------------------------------------------------------------

fn actions_classes(builder: &mut Builder) {
    let _ = builder
        .class::<ActionsHandle>("window_actions")
        .ctor(|_cx, _inits| Err(Fail::refused("a window's actions are made with its frame")))
        .get("frame", |h, cx| frame_object(&h.ui, h.id, cx))
        .get("closing", |h, _| Ok(Val::Bool(h.ui.frame_closing(h.id))))
        .get("now", |h, _| {
            let now = h.ui.frames.borrow().frames.get(&h.id).and_then(|rec| rec.now.clone());
            Ok(closure_val(now))
        })
        .set("now", |h, _, v| {
            let clock = closure_of(v)?;
            if let Some(rec) = h.ui.frames.borrow_mut().frames.get_mut(&h.id) {
                rec.now = clock;
            }
            Ok(())
        })
        .method("close", |h, _, _| {
            actions::close(&h.ui, h.id).map_err(Fail::err)?;
            Ok(Val::None)
        })
        .method("toggle_maximise", |h, _, _| {
            actions::toggle_maximise(&h.ui, h.id).map_err(Fail::err)?;
            Ok(Val::None)
        })
        .method("minimise", |h, _, _| {
            actions::minimise(&h.ui, h.id).map_err(Fail::err)?;
            Ok(Val::None)
        })
        .method("menu", |h, _, _| {
            actions::menu(&h.ui, h.id).map_err(Fail::err)?;
            Ok(Val::None)
        })
        .method("lower", |h, _, _| {
            actions::lower(&h.ui, h.id).map_err(Fail::err)?;
            Ok(Val::None)
        })
        .method("gesture", |h, _, args| {
            actions::gesture(&h.ui, h.id, args[0].enum_value()?).map_err(Fail::err)?;
            Ok(Val::None)
        });
    let _ = builder
        .class::<DragHandle>("window_drag")
        .ctor(|cx, inits| {
            let ui = ui_of(cx)?;
            let frame = frame_init(cx, inits)?;
            let key = ui.frames.borrow_mut().mint_drag(frame);
            Ok(DragHandle { ui, key })
        })
        .get("frame", |h, cx| {
            let frame =
                h.ui.frames
                    .borrow()
                    .drags
                    .get(&h.key)
                    .map(|state| state.frame)
                    .ok_or_else(|| Fail::stale("the drag is gone"))?;
            frame_object(&h.ui, frame, cx)
        })
        .method("attach", |h, cx, args| {
            let cid = canvas_of(cx, args[0].object()?)?;
            let frame =
                h.ui.frames
                    .borrow()
                    .drags
                    .get(&h.key)
                    .map(|state| state.frame)
                    .ok_or_else(|| Fail::stale("the drag is gone"))?;
            let key = h.key;
            let attached = h.ui.edit(cid, |widget| match &mut widget.sp {
                Special::None => {
                    widget.sp = Special::Frame(FramePart {
                        frame,
                        role: PartRole::DragArea,
                        drag: Some(key),
                    });
                    true
                }
                Special::Frame(part) => {
                    part.drag = Some(key);
                    true
                }
                _ => false,
            })?;
            if attached {
                Ok(Val::None)
            } else {
                Err(Fail::type_error(
                    "only a plain canvas, a box or a part of a frame can be made a drag area",
                ))
            }
        })
        .method("press", |h, _, args| {
            let (x, y, button) = pointer_of(&args[0])?;
            drag::press(&h.ui, h.key, x, y, button).map_err(Fail::err)?;
            Ok(Val::None)
        })
        .method("motion", |h, _, args| {
            let (x, y, _) = pointer_of(&args[0])?;
            drag::motion(&h.ui, h.key, x, y);
            Ok(Val::None)
        })
        .method("release", |h, _, args| {
            let (x, y, button) = pointer_of(&args[0])?;
            drag::release(&h.ui, h.key, x, y, button).map_err(Fail::err)?;
            Ok(Val::None)
        });
}

/// The state of a `window_drag` object: the toolkit and the key of its drag.
pub(crate) struct DragHandle {
    ui: Rc<Ui>,
    key: u64,
}

impl Drop for DragHandle {
    fn drop(&mut self) {
        // A canvas that was made a drag area keeps the key; the state goes with the generation.
        let _ = &self.key;
    }
}

/// The position and button of a `ui_pointer_event` value.
fn pointer_of(event: &Val) -> Res<(f64, f64, u64)> {
    let fields = event.record()?;
    let at = |index: usize| fields.get(index).ok_or_else(|| Fail::type_error("not a pointer event"));
    Ok((at(1)?.double()?, at(2)?.double()?, at(3)?.uint()?))
}

// ---- the parts ------------------------------------------------------------------------------------------------------------------

/// A canvas that was just made for the frame `frame` becomes a part of it with `role`.
fn become_part(ui: &Ui, cid: Cid, frame: FrameId, role: PartRole, drag: Option<u64>) {
    ui.tweak(cid, |widget| {
        widget.sp = Special::Frame(FramePart { frame, role, drag })
    });
}

fn part_classes(builder: &mut Builder) {
    let _ = frame_member(class_with(
        builder,
        "drag_area",
        WidgetKind::Box,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            let drag = ui.frames.borrow_mut().mint_drag(frame);
            become_part(ui, cid, frame, PartRole::DragArea, Some(drag));
            Ok(())
        },
    ));
    let _ = titlebar_members(frame_member(class_with(
        builder,
        "titlebar",
        WidgetKind::Box,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            let obj = cx.me();
            titlebar::build(ui, cx, frame, (obj, cid), None)
        },
    )));
    let _ = frame_member(text_members(class_with(
        builder,
        "title_text",
        WidgetKind::Text,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            parts::init_title(ui, cid, frame, &TitlebarStyle::default());
            Ok(())
        },
    )));
    let _ = window_button_members(frame_member(icon_members(class_with(
        builder,
        "window_button",
        WidgetKind::Icon,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            let name = inits
                .take("name")
                .ok_or_else(|| Fail::type_error("a window button is made for one of the three buttons"))?
                .enum_value()?;
            parts::init_button(ui, cid, frame, name, icon_default(name), 16.0, "#cdd6f4");
            Ok(())
        },
    ))));
    let _ = window_buttons_members(frame_member(class_with(
        builder,
        "window_buttons",
        WidgetKind::Box,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            let names = inits
                .take("buttons")
                .ok_or_else(|| Fail::type_error("a row of window buttons is made for its buttons"))?
                .list()?
                .iter()
                .map(Val::enum_value)
                .collect::<Res<Vec<u32>>>()?;
            let obj = cx.me();
            parts::new_buttons(ui, cx, frame, (obj, cid), names)
        },
    )));
    let _ = app_icon_members(frame_member(class_with(
        builder,
        "app_icon",
        WidgetKind::Box,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            ui.tweak(cid, |widget| widget.b.radius = 4.0);
            become_part(
                ui,
                cid,
                frame,
                PartRole::AppIcon(AppIconPart {
                    size: 16.0,
                    color: "#ffffff".to_owned(),
                    app_id: String::new(),
                }),
                None,
            );
            Ok(())
        },
    )));
    let _ = class_with(builder, "frame_backdrop", WidgetKind::Box, |ui, _cx, _inits, cid| {
        ui.tweak(cid, |widget| widget.b.fill = "#1e1e2e".to_owned());
        become_part(ui, cid, 0, PartRole::Backdrop, None);
        Ok(())
    });
    let _ = border_members(frame_member(class_with(
        builder,
        "frame_border",
        WidgetKind::Canvas,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            become_part(
                ui,
                cid,
                frame,
                PartRole::Border(BorderPart {
                    thickness: 1.0,
                    color: "#89b4fa".to_owned(),
                    color_of: None,
                    radius: 0.0,
                }),
                None,
            );
            Ok(())
        },
    )));
    let _ = glow_members(class_with(
        builder,
        "frame_glow",
        WidgetKind::Canvas,
        |ui, _cx, _inits, cid| {
            become_part(
                ui,
                cid,
                0,
                PartRole::Glow(GlowPart {
                    color: "#89b4fa".to_owned(),
                    strength: 0.5,
                    radius: 0.0,
                    steps: 8,
                }),
                None,
            );
            Ok(())
        },
    ));
    let _ = handle_members(frame_member(class_with(
        builder,
        "resize_handle",
        WidgetKind::Box,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            let side = inits
                .take("side")
                .ok_or_else(|| Fail::type_error("a resize handle holds a side or a corner"))?
                .enum_value()?;
            parts::init_handle(ui, cid, frame, side, true);
            Ok(())
        },
    )));
    let _ = handles_members(frame_member(class_with(
        builder,
        "resize_handles",
        WidgetKind::Box,
        |ui, cx, inits, cid| {
            let frame = frame_init(cx, inits)?;
            let obj = cx.me();
            build_handles(ui, cx, frame, (obj, cid), 6.0)
        },
    )));
}

use super::parts::build_handles;

/// What a titlebar says of itself.
fn titlebar_members(class: Class<'_>) -> Class<'_> {
    class
        .get("style", |h, _| {
            let own = bar_get(h, |bar| bar.style.clone())?;
            Ok(style_val(
                &own.unwrap_or_else(|| h.ui.frames.borrow().deco.style.clone()),
            ))
        })
        .set("style", |h, _, v| {
            let style = style_of(v)?;
            h.edit(|widget| {
                if let Special::Frame(FramePart {
                    role: PartRole::Titlebar(bar),
                    ..
                }) = &mut widget.sp
                {
                    bar.style = Some(style);
                }
            })
        })
        .get("title_area", |h, cx| {
            let area = bar_get(h, |bar| bar.area)?;
            let obj =
                h.ui.read(area, |widget| widget.obj)
                    .ok_or_else(|| Fail::stale("the title area is gone"))?;
            cx.handout(obj)
        })
        .get("title", |h, cx| {
            let title = bar_get(h, |bar| bar.title)?;
            let obj =
                h.ui.read(title, |widget| widget.obj)
                    .ok_or_else(|| Fail::stale("the title is gone"))?;
            cx.handout(obj)
        })
        .get("buttons", |h, cx| {
            let buttons = bar_get(h, |bar| bar.buttons.clone())?;
            let mut list = Vec::with_capacity(buttons.len());
            for (_, cid) in buttons {
                if let Some(obj) = h.ui.read(cid, |widget| widget.obj) {
                    list.push(cx.handout(obj)?);
                }
            }
            Ok(Val::List(list))
        })
        .function("install", |cx, args| {
            let ui = ui_of(cx)?;
            let frame = frame_arg(cx, &args[0])?;
            let style = args.get(1).filter(|value| !value.is_none()).map(style_of).transpose()?;
            Ok(Val::Bool(titlebar::fill(&ui, cx, frame, style)?))
        })
}

role_access!(bar_get, Titlebar, super::BarPart, "a titlebar");

fn window_button_members(class: Class<'_>) -> Class<'_> {
    class
        .get("name", |h, _| Ok(enumeration(button_get(h, |button| button.name)?)))
        .get("restore_icon", |h, _| {
            Ok(Val::Text(button_get(h, |button| {
                button.restore_icon.clone().unwrap_or_default()
            })?))
        })
        .set("restore_icon", |h, _, v| {
            let icon = v.text()?.to_owned();
            button_set(h, |button| button.restore_icon = (!icon.is_empty()).then_some(icon))
        })
}

fn window_buttons_members(class: Class<'_>) -> Class<'_> {
    class
        .get("buttons", |h, _| {
            Ok(Val::List(
                buttons_get(h, |row| row.names.clone())?
                    .into_iter()
                    .map(enumeration)
                    .collect(),
            ))
        })
        .get("icons", |h, _| {
            let set = buttons_get(h, |row| row.icons.clone())?;
            let mut icons = [
                icon_default(en::BUTTON_MINIMISE).to_owned(),
                icon_default(en::BUTTON_MAXIMISE).to_owned(),
                icon_default(en::BUTTON_CLOSE).to_owned(),
            ];
            for (name, icon) in set {
                icons[(name as usize).saturating_sub(1).min(2)] = icon;
            }
            Ok(icons_val(&icons))
        })
        .set("icons", |h, _, v| {
            let icons = icons_of(v)?;
            parts::set_button_icons(&h.ui, h.cid, |row| {
                row.icons = [en::BUTTON_MINIMISE, en::BUTTON_MAXIMISE, en::BUTTON_CLOSE]
                    .into_iter()
                    .zip(icons)
                    .collect();
            });
            Ok(())
        })
        .get("button_size", |h, _| Ok(Val::Double(buttons_get(h, |row| row.size)?)))
        .set("button_size", |h, _, v| {
            let size = v.double()?;
            parts::set_button_icons(&h.ui, h.cid, |row| row.size = size);
            Ok(())
        })
        .get("button_color", |h, _| {
            Ok(Val::Text(buttons_get(h, |row| row.color.clone())?))
        })
        .set("button_color", |h, _, v| {
            let color = v.text()?.to_owned();
            parts::set_button_icons(&h.ui, h.cid, |row| row.color = color);
            Ok(())
        })
        .get("restore_icon", |h, _| {
            Ok(Val::Text(buttons_get(h, |row| {
                row.restore_icon.clone().unwrap_or_default()
            })?))
        })
        .set("restore_icon", |h, _, v| {
            let icon = v.text()?.to_owned();
            parts::set_button_icons(&h.ui, h.cid, |row| {
                row.restore_icon = (!icon.is_empty()).then_some(icon)
            });
            Ok(())
        })
        .method("button", |h, cx, args| {
            let name = args[0].enum_value()?;
            let children = h.read(|widget| widget.children.clone())?;
            for child in children {
                let found =
                    h.ui.read(child, |widget| match &widget.sp {
                        Special::Frame(FramePart {
                            role: PartRole::Button(button),
                            ..
                        }) if button.name == name => Some(widget.obj),
                        _ => None,
                    })
                    .flatten();
                if let Some(obj) = found {
                    return cx.handout(obj);
                }
            }
            Ok(Val::None)
        })
}

fn app_icon_members(class: Class<'_>) -> Class<'_> {
    class
        .get("size", |h, _| Ok(Val::Double(icon_get(h, |icon| icon.size)?)))
        .set("size", |h, _, v| {
            let size = v.double()?;
            icon_set(h, |icon| icon.size = size)
        })
        .get("color", |h, _| Ok(Val::Text(icon_get(h, |icon| icon.color.clone())?)))
        .set("color", |h, _, v| {
            let color = v.text()?.to_owned();
            icon_set(h, |icon| icon.color = color)
        })
        .function("tile_of", |_cx, args| {
            let (letter, colour) = parts::tile_of(args[0].text()?);
            Ok(record(vec![Val::Text(letter), Val::Text(colour.to_owned())]))
        })
}

fn border_members(class: Class<'_>) -> Class<'_> {
    class
        .get("thickness", |h, _| {
            Ok(Val::Double(border_get(h, |border| border.thickness)?))
        })
        .set("thickness", |h, _, v| {
            let line = v.double()?;
            border_set(h, |border| border.thickness = line)
        })
        .get("color", |h, _| {
            Ok(Val::Text(border_get(h, |border| border.color.clone())?))
        })
        .set("color", |h, _, v| {
            let color = v.text()?.to_owned();
            border_set(h, |border| border.color = color)
        })
        .get("color_of", |h, _| {
            Ok(closure_val(border_get(h, |border| border.color_of.clone())?))
        })
        .set("color_of", |h, _, v| {
            let colour = closure_of(v)?;
            border_set(h, |border| border.color_of = colour)
        })
        .get("radius", |h, _| Ok(Val::Double(border_get(h, |border| border.radius)?)))
        .set("radius", |h, _, v| {
            let radius = v.double()?;
            border_set(h, |border| border.radius = radius)
        })
}

fn glow_members(class: Class<'_>) -> Class<'_> {
    class
        .get("color", |h, _| Ok(Val::Text(glow_get(h, |glow| glow.color.clone())?)))
        .set("color", |h, _, v| {
            let color = v.text()?.to_owned();
            glow_set(h, |glow| glow.color = color)
        })
        .get("strength", |h, _| Ok(Val::Double(glow_get(h, |glow| glow.strength)?)))
        .set("strength", |h, _, v| {
            let strength = v.double()?;
            glow_set(h, |glow| glow.strength = strength)
        })
        .get("radius", |h, _| Ok(Val::Double(glow_get(h, |glow| glow.radius)?)))
        .set("radius", |h, _, v| {
            let radius = v.double()?;
            glow_set(h, |glow| glow.radius = radius)
        })
        .get("steps", |h, _| Ok(Val::Uint(glow_get(h, |glow| glow.steps)?)))
        .set("steps", |h, _, v| {
            let steps = v.uint()?;
            glow_set(h, |glow| glow.steps = steps)
        })
}

fn handle_members(class: Class<'_>) -> Class<'_> {
    class
        .get("side", |h, _| Ok(enumeration(handle_get(h, |handle| handle.side)?)))
        .get("fit_parent", |h, _| {
            Ok(Val::Bool(handle_get(h, |handle| handle.fit_parent)?))
        })
        .set("fit_parent", |h, _, v| {
            let fit = v.bool()?;
            handle_set(h, |handle| handle.fit_parent = fit)
        })
}

fn handles_members(class: Class<'_>) -> Class<'_> {
    class
        .get("thickness", |h, _| {
            Ok(Val::Double(handles_get(h, |handles| handles.thickness)?))
        })
        .set("thickness", |h, _, v| {
            let thickness = v.double()?;
            handles_set(h, |handles| handles.thickness = thickness)
        })
        .get("inside", |h, _| {
            Ok(Val::Double(handles_get(h, |handles| handles.inside)?))
        })
        .set("inside", |h, _, v| {
            let inside = v.double()?;
            handles_set(h, |handles| handles.inside = inside)
        })
        .get("corner", |h, _| {
            Ok(Val::Double(handles_get(h, |handles| handles.corner)?))
        })
        .set("corner", |h, _, v| {
            let corner = v.double()?;
            handles_set(h, |handles| handles.corner = corner)
        })
        .get("handles", |h, cx| {
            let children = h.read(|widget| widget.children.clone())?;
            let mut list = Vec::with_capacity(children.len());
            for child in children {
                if let Some(obj) = h.ui.read(child, |widget| widget.obj) {
                    list.push(cx.handout(obj)?);
                }
            }
            Ok(Val::List(list))
        })
}

// ---- the decorations ----------------------------------------------------------------------------------------------------------

/// A hook of the decorations as a property: its closure, read and written through `world.deco.hooks`.
macro_rules! hook_property {
    ($class:expr, $name:literal, $field:ident) => {
        $class
            .get($name, |h, _| {
                Ok(closure_val(h.ui.frames.borrow().deco.hooks.$field.clone()))
            })
            .set($name, |h, _, v| {
                let hook = closure_of(v)?;
                h.ui.frames.borrow_mut().deco.hooks.$field = hook;
                Ok(())
            })
    };
}

fn decorations_class(builder: &mut Builder) {
    let class = builder
        .class::<DecoHandle>("decorations")
        .ctor(|cx, _inits| {
            let ui = ui_of(cx)?;
            ui.frames.borrow_mut().deco.obj = cx.me();
            Ok(DecoHandle { ui })
        })
        .get("started", |h, _| Ok(Val::Bool(h.ui.frames.borrow().deco.started)))
        .get("frame", |h, _| Ok(closure_val(h.ui.frames.borrow().deco.fill.clone())))
        .set("frame", |h, _, v| {
            let fill = closure_of(v)?;
            h.ui.frames.borrow_mut().deco.fill = fill;
            Ok(())
        })
        .get("style", |h, _| Ok(style_val(&h.ui.frames.borrow().deco.style)))
        .set("style", |h, _, v| {
            let style = style_of(v)?;
            h.ui.frames.borrow_mut().deco.style = style;
            h.ui.schedule();
            Ok(())
        })
        .get("resize_handles", |h, _| {
            Ok(Val::Double(h.ui.frames.borrow().deco.resize_handles))
        })
        .set("resize_handles", |h, _, v| {
            let reach = v.double()?;
            h.ui.frames.borrow_mut().deco.resize_handles = reach.max(0.0);
            Ok(())
        })
        .get("now", |h, _| Ok(closure_val(h.ui.frames.borrow().deco.now.clone())))
        .set("now", |h, _, v| {
            let clock = closure_of(v)?;
            h.ui.frames.borrow_mut().deco.now = clock;
            Ok(())
        });
    let class = hook_property!(class, "menu", menu);
    let class = hook_property!(class, "lower", lower);
    let class = hook_property!(class, "maximise", maximise);
    let class = hook_property!(class, "minimise", minimise);
    let class = hook_property!(class, "press", press);
    let class = hook_property!(class, "release", release);
    let _ = class
        .method("start", |h, _, _| {
            decorations::start(&h.ui)?;
            Ok(Val::None)
        })
        .method("stop", |h, _, _| {
            decorations::stop(&h.ui);
            Ok(Val::None)
        })
        .method("rule", |h, _, args| {
            let text = |at: usize| {
                args.get(at)
                    .filter(|value| !value.is_none())
                    .map(|value| value.text().map(str::to_owned))
                    .transpose()
            };
            let closure = |at: usize| args.get(at).map(closure_of).transpose().map(Option::flatten);
            let rule = FrameRule {
                app_id: text(0)?,
                title: text(1)?,
                test: closure(2)?,
                fill: closure(3)?,
            };
            h.ui.frames.borrow_mut().deco.rules.push(rule);
            Ok(Val::None)
        })
        .method("frame_of", |h, cx, args| {
            let node = client_node(cx.ctx(), "frame_of", &args[0])?;
            let obj = {
                let world = h.ui.frames.borrow();
                world
                    .by_node
                    .get(&node)
                    .and_then(|frame| world.frames.get(frame))
                    .map(|rec| rec.obj)
            };
            match obj {
                Some(obj) => cx.handout(obj),
                None => Ok(Val::None),
            }
        })
        .method("inset_of", |h, cx, args| {
            inset_of(h, client_node(cx.ctx(), "inset_of", &args[0])?, false)
        })
        .method("reserved_of", |h, cx, args| {
            inset_of(h, client_node(cx.ctx(), "reserved_of", &args[0])?, true)
        })
        .method("reframe", |h, cx, args| {
            decorations::reframe(&h.ui, client_node(cx.ctx(), "reframe", &args[0])?);
            Ok(Val::None)
        });
}

/// The room the frame of the window `node` takes round its content: none for a window with no frame.
fn inset_of(h: &DecoHandle, node: u64, reserved: bool) -> Res<Val> {
    let frame = h.ui.frames.borrow().by_node.get(&node).copied();
    let inset = match frame {
        Some(frame) => h.ui.frame_inset(frame, reserved).map_err(Fail::err)?,
        None => crate::floating::Inset::default(),
    };
    Ok(inset_val(&inset))
}

/// Implements the classes of the frame module.
pub(crate) fn register(builder: &mut Builder) {
    frame_members(builder);
    outlet_classes(builder);
    actions_classes(builder);
    part_classes(builder);
    decorations_class(builder);
}
