//! The classes of the toolkit as the object protocol serves them: `canvas` and what derives from it. Every one holds the same state, a
//! [`CanvasHandle`] on its canvas in the generation's toolkit, and its members read and write that canvas.

use std::rc::Rc;

use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{Fail, Res, Val};

use super::tree::ui_of;
use super::widget::{BarPart, Behaviour, CanvasHandle, Cid, ClockPart, Metrics, Special, Widget, WidgetKind, en};
use super::{clock, entry, paint, toast};
use crate::object::with_lib;

pub(crate) type Class<'a> = ClassBuilder<'a, CanvasHandle>;

impl CanvasHandle {
    pub(crate) fn read<R>(&self, read: impl FnOnce(&Widget) -> R) -> Res<R> {
        self.ui
            .read(self.cid, read)
            .ok_or_else(|| Fail::stale("the canvas is gone"))
    }

    pub(crate) fn edit<R>(&self, change: impl FnOnce(&mut Widget) -> R) -> Res<R> {
        self.ui.edit(self.cid, change)
    }

    pub(crate) fn number(&self, read: impl FnOnce(&Widget) -> f64) -> Res<Val> {
        self.read(|widget| Val::Double(read(widget)))
    }

    pub(crate) fn text(&self, read: impl FnOnce(&Widget) -> &str) -> Res<Val> {
        self.read(|widget| Val::Text(read(widget).to_owned()))
    }

    pub(crate) fn flag(&self, read: impl FnOnce(&Widget) -> bool) -> Res<Val> {
        self.read(|widget| Val::Bool(read(widget)))
    }
}

pub(crate) fn enumeration(value: u32) -> Val {
    Val::Enum { class: 0, value }
}

/// The canvas an object of the toolkit holds.
pub(crate) fn canvas_of(cx: &Cx<'_>, obj: u64) -> Res<Cid> {
    cx.with_state(obj, |handle: &mut CanvasHandle| handle.cid)
}

/// The members every canvas has.
pub(crate) fn canvas_members(class: Class<'_>) -> Class<'_> {
    class
        .get("node", |h, _| h.read(|w| Val::Uint(w.node)))
        .get("parent", |h, cx| {
            let parent = h.read(|w| w.parent)?.and_then(|p| h.ui.read(p, |w| w.obj));
            Ok(match parent {
                Some(obj) => cx.handout(obj)?,
                None => Val::None,
            })
        })
        .get("children", |h, cx| {
            let kids = h.read(|w| w.children.clone())?;
            let mut list = Vec::with_capacity(kids.len());
            for kid in kids {
                if let Some(obj) = h.ui.read(kid, |w| w.obj) {
                    list.push(cx.handout(obj)?);
                }
            }
            Ok(Val::List(list))
        })
        .set("children", |h, cx, value| {
            for kid in value.list()? {
                add_child(h, cx, kid.object()?)?;
            }
            Ok(())
        })
        .get("x", |h, _| h.number(|w| w.x))
        .set("x", |h, _, v| h.edit(|w| w.x = v.double().unwrap_or(0.0)))
        .get("y", |h, _| h.number(|w| w.y))
        .set("y", |h, _, v| h.edit(|w| w.y = v.double().unwrap_or(0.0)))
        .get("width", |h, _| Ok(Val::Double(h.ui.width_of(h.cid))))
        .set("width", |h, _, v| h.edit(|w| w.width = v.double().ok()))
        .get("height", |h, _| Ok(Val::Double(h.ui.height_of(h.cid))))
        .set("height", |h, _, v| h.edit(|w| w.height = v.double().ok()))
        .get("visible", |h, _| h.flag(|w| w.visible))
        .set("visible", |h, _, v| h.edit(|w| w.visible = v.bool().unwrap_or(true)))
        .get("opacity", |h, _| h.number(|w| w.opacity))
        .set("opacity", |h, _, v| h.edit(|w| w.opacity = v.double().unwrap_or(1.0)))
        .get("secure", |h, _| h.flag(|w| w.secure))
        .set("secure", |h, _, v| h.edit(|w| w.secure = v.bool().unwrap_or(false)))
        .get("input", |h, _| h.flag(|w| w.input))
        .set("input", |h, _, v| h.edit(|w| w.input = v.bool().unwrap_or(true)))
        .get("placement", |h, _| h.read(|w| enumeration(w.placement)))
        .set("placement", |h, _, v| {
            let value = v.enum_value()?;
            h.edit(|w| w.placement = value)
        })
        .get("alive", |h, _| h.flag(|w| w.alive))
        .get("failures", |h, _| {
            let w = h.ui.w.borrow();
            let total: u32 = w
                .subtree(h.cid)
                .into_iter()
                .filter_map(|cid| w.get(cid).map(|widget| widget.failures))
                .sum();
            Ok(Val::Uint(u64::from(total)))
        })
        .get("focused", |h, _| Ok(Val::Bool(h.ui.focused(h.cid))))
        .get("natural_width", |h, _| Ok(Val::Double(h.ui.natural_width(h.cid))))
        .get("natural_height", |h, _| Ok(Val::Double(h.ui.natural_height(h.cid))))
        .get("behaviour", |h, cx| {
            let behaviour = h.read(|w| w.behaviour.clone())?;
            Ok(match behaviour {
                Some(held) if cx.class_of(held.obj).is_ok() => cx.handout(held.obj)?,
                _ => Val::None,
            })
        })
        .set("behaviour", |h, cx, value| {
            let obj = value.object()?;
            let shell = cx.shell(obj)?;
            h.edit(|w| w.behaviour = Some(Behaviour { obj, shell }))
        })
        .get("on_pointer", |h, _| {
            Ok(h.read(|w| w.on_pointer.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("on_pointer", |h, _, value| {
            let closure = Rc::clone(value.closure()?);
            h.edit(|w| w.on_pointer = Some(closure))
        })
        .get("on_key", |h, _| {
            Ok(h.read(|w| w.on_key.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("on_key", |h, _, value| {
            let closure = Rc::clone(value.closure()?);
            h.edit(|w| w.on_key = Some(closure))
        })
        .method("clear_width", |h, _, _| {
            h.edit(|w| w.width = None)?;
            Ok(Val::None)
        })
        .method("clear_height", |h, _, _| {
            h.edit(|w| w.height = None)?;
            Ok(Val::None)
        })
        .method("add", |h, cx, args| {
            add_child(h, cx, args[0].object()?)?;
            Ok(Val::None)
        })
        .method("remove", |h, cx, args| {
            let child = canvas_of(cx, args[0].object()?)?;
            if h.ui.read(child, |w| w.parent) != Some(Some(h.cid)) {
                return Err(Fail::type_error("that canvas is not a child of this one"));
            }
            h.ui.kill(child);
            Ok(Val::None)
        })
        .method("mount", |h, _, args| {
            h.ui.mount(h.cid, args[0].uint()?)?;
            Ok(Val::None)
        })
        .method("mount_top", |h, _, _| {
            h.ui.mount_top(h.cid)?;
            Ok(Val::None)
        })
        .method("focus", |h, _, _| {
            let node = h
                .read(|w| w.alive.then_some(w.node))?
                .ok_or_else(|| Fail::stale("Canvas.focus: the canvas is destroyed"))?;
            h.ui.focus_node(node)?;
            Ok(Val::None)
        })
        .method("contains", |h, cx, args| {
            let other = args[0].object()?;
            if other == cx.me() {
                return Ok(Val::Bool(true));
            }
            let other = canvas_of(cx, other)?;
            Ok(Val::Bool(h.ui.w.borrow().contains(h.cid, other)))
        })
        .method("invalidate", |h, _, _| {
            h.ui.schedule();
            Ok(Val::None)
        })
        .method("paint_default", |h, _, _| {
            let ops = h.ui.paint_default(h.cid).map_err(Fail::err)?;
            Ok(paint::to_val(&ops))
        })
        .method("arrange_default", |h, _, _| {
            h.ui.arrange_default(h.cid);
            Ok(Val::None)
        })
        .method("to_global", |h, _, args| {
            let (mut x, mut y) = (args[0].double()?, args[1].double()?);
            let w = h.ui.w.borrow();
            let mut at = Some(h.cid);
            while let Some(cid) = at {
                let Some(widget) = w.get(cid) else { break };
                x += widget.x;
                y += widget.y;
                at = widget.parent;
            }
            Ok(Val::Record {
                class: 0,
                fields: vec![Val::Double(x), Val::Double(y)],
            })
        })
}

/// Puts the canvas of the object `child` last under this one: the toolkit's tree, the compositor's and the objects'.
pub(crate) fn add_child(h: &CanvasHandle, cx: &mut Cx<'_>, child: u64) -> Res<()> {
    if child == cx.me() {
        return Err(Fail::type_error("a canvas cannot be its own ancestor"));
    }
    let kid = canvas_of(cx, child)?;
    h.ui.link(h.cid, kid)?;
    cx.set_parent(child, cx.me())?;
    // An outlet of a window frame learns what was added to it.
    crate::frame::added(&h.ui, h.cid, kid);
    h.ui.schedule();
    Ok(())
}

/// The members a box adds.
pub(crate) fn box_members(class: Class<'_>) -> Class<'_> {
    class
        .get("direction", |h, _| h.read(|w| enumeration(w.b.direction)))
        .set("direction", |h, _, v| {
            let value = v.enum_value()?;
            h.edit(|w| w.b.direction = value)
        })
        .get("gap", |h, _| h.number(|w| w.b.gap))
        .set("gap", |h, _, v| h.edit(|w| w.b.gap = v.double().unwrap_or(0.0)))
        .get("padding", |h, _| h.number(|w| w.b.padding))
        .set("padding", |h, _, v| h.edit(|w| w.b.padding = v.double().unwrap_or(0.0)))
        .get("grow", |h, _| h.number(|w| w.b.grow))
        .set("grow", |h, _, v| h.edit(|w| w.b.grow = v.double().unwrap_or(0.0)))
        .get("fill", |h, _| h.text(|w| &w.b.fill))
        .set("fill", |h, _, v| {
            let fill = v.text()?.to_owned();
            h.edit(|w| w.b.fill = fill)
        })
        .get("radius", |h, _| h.number(|w| w.b.radius))
        .set("radius", |h, _, v| h.edit(|w| w.b.radius = v.double().unwrap_or(0.0)))
}

/// A `text_metrics` record of `metrics`.
pub(super) fn metrics_val(metrics: Metrics) -> Val {
    Val::Record {
        class: 0,
        fields: vec![
            Val::Double(metrics.width),
            Val::Double(metrics.height),
            Val::Double(metrics.ascent),
        ],
    }
}

/// The members a text adds.
pub(crate) fn text_members(class: Class<'_>) -> Class<'_> {
    class
        .get("text", |h, _| h.text(|w| &w.t.text))
        .set("text", |h, _, v| {
            let text = v.text()?.to_owned();
            h.edit(|w| w.t.text = text)
        })
        .get("size", |h, _| h.number(|w| w.t.size))
        .set("size", |h, _, v| h.edit(|w| w.t.size = v.double().unwrap_or(14.0)))
        .get("color", |h, _| h.text(|w| &w.t.color))
        .set("color", |h, _, v| {
            let color = v.text()?.to_owned();
            h.edit(|w| w.t.color = color)
        })
        .get("font", |h, _| h.text(|w| &w.t.font))
        .set("font", |h, _, v| {
            let font = v.text()?.to_owned();
            h.edit(|w| w.t.font = font)
        })
        .get("ellipsize", |h, _| h.flag(|w| w.t.ellipsize))
        .set("ellipsize", |h, _, v| {
            h.edit(|w| w.t.ellipsize = v.bool().unwrap_or(false))
        })
        .get("metrics", |h, _| Ok(metrics_val(h.ui.metrics(h.cid))))
        .get("ascent", |h, _| Ok(Val::Double(h.ui.metrics(h.cid).ascent)))
        .method("shown", |h, _, _| Ok(Val::Text(h.ui.shown(h.cid))))
}

/// The members an entry adds.
pub(crate) fn entry_members(class: Class<'_>) -> Class<'_> {
    fn part<R>(h: &CanvasHandle, read: impl FnOnce(&super::widget::EntryPart) -> R) -> Res<R> {
        h.read(|w| match &w.sp {
            Special::Entry(part) => Some(read(part)),
            _ => None,
        })?
        .ok_or_else(|| Fail::type_error("the canvas is not an entry"))
    }
    fn change(h: &CanvasHandle, change: impl FnOnce(&mut super::widget::EntryPart)) -> Res<()> {
        h.edit(|w| {
            if let Special::Entry(part) = &mut w.sp {
                change(part);
            }
        })
    }
    class
        .get("model", |h, cx| cx.handout(part(h, |p| p.model)?))
        .get("text", |h, _| {
            Ok(Val::Text(entry::model_text(&h.ui, part(h, |p| p.model)?)))
        })
        .set("text", |h, _, v| {
            let model = part(h, |p| p.model)?;
            let member = h.ui.ids().model.text;
            let ctx = h.ui.ctx;
            with_lib(|lib| lib.set(ctx, model, member, Val::Text(v.text()?.to_owned())))?;
            h.ui.schedule();
            Ok(())
        })
        .get("secret", |h, _| Ok(Val::Bool(part(h, |p| p.secret)?)))
        .set("secret", |h, _, v| {
            let secret = v.bool()?;
            change(h, |p| p.secret = secret)
        })
        .get("size", |h, _| Ok(Val::Double(part(h, |p| p.size)?)))
        .set("size", |h, _, v| {
            let size = v.double()?;
            change(h, |p| p.size = size)
        })
        .get("color", |h, _| Ok(Val::Text(part(h, |p| p.color.clone())?)))
        .set("color", |h, _, v| {
            let color = v.text()?.to_owned();
            change(h, |p| p.color = color)
        })
        .get("font", |h, _| Ok(Val::Text(part(h, |p| p.font.clone())?)))
        .set("font", |h, _, v| {
            let font = v.text()?.to_owned();
            change(h, |p| p.font = font)
        })
        .get("selection_fill", |h, _| {
            Ok(Val::Text(part(h, |p| p.selection_fill.clone())?))
        })
        .set("selection_fill", |h, _, v| {
            let fill = v.text()?.to_owned();
            change(h, |p| p.selection_fill = fill)
        })
        .get("on_submit", |h, _| {
            Ok(part(h, |p| p.on_submit.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("on_submit", |h, _, v| {
            let closure = Rc::clone(v.closure()?);
            change(h, |p| p.on_submit = Some(closure))
        })
        .method("shown", |h, _, args| {
            Ok(Val::Text(entry::shown(part(h, |p| p.secret)?, args[0].text()?)))
        })
}

/// The members a clock adds.
fn clock_members(class: Class<'_>) -> Class<'_> {
    fn part<R>(h: &CanvasHandle, read: impl FnOnce(&ClockPart) -> R) -> Res<R> {
        h.read(|w| match &w.sp {
            Special::Clock(part) => Some(read(part)),
            _ => None,
        })?
        .ok_or_else(|| Fail::type_error("the canvas is not a clock"))
    }
    fn change(h: &CanvasHandle, change: impl FnOnce(&mut ClockPart)) -> Res<()> {
        h.edit(|w| {
            if let Special::Clock(part) = &mut w.sp {
                change(part);
            }
        })?;
        // The text follows the format and the clock at once, as the clock's own constructor sets it.
        clock::set_text(&h.ui, h.cid).map_err(Fail::err)
    }
    class
        .get("format", |h, _| Ok(Val::Text(part(h, |p| p.format.clone())?)))
        .set("format", |h, _, v| {
            let format = v.text()?.to_owned();
            change(h, |p| p.format = format)
        })
        .get("now", |h, _| {
            Ok(part(h, |p| p.now.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("now", |h, _, v| {
            let source = Rc::clone(v.closure()?);
            change(h, |p| p.now = Some(source))
        })
}

/// The members a bar adds, and its functions.
fn bar_members(class: Class<'_>) -> Class<'_> {
    fn part<R>(h: &CanvasHandle, read: impl FnOnce(&BarPart) -> R) -> Res<R> {
        h.read(|w| match &w.sp {
            Special::Bar(part) => Some(read(part)),
            _ => None,
        })?
        .ok_or_else(|| Fail::type_error("the canvas is not a bar"))
    }
    fn change(h: &CanvasHandle, change: impl FnOnce(&mut BarPart)) -> Res<()> {
        h.edit(|w| {
            if let Special::Bar(part) = &mut w.sp {
                change(part);
            }
        })
    }
    class
        .get("edge", |h, _| Ok(enumeration(part(h, |p| p.edge)?)))
        .set("edge", |h, _, v| {
            let edge = v.enum_value()?;
            change(h, |p| p.edge = edge)
        })
        .get("hide_when", |h, _| {
            Ok(part(h, |p| p.hide_when.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("hide_when", |h, _, v| {
            let question = Rc::clone(v.closure()?);
            change(h, |p| p.hide_when = Some(question))
        })
        .get("output", |h, _| Ok(Val::Uint(part(h, |p| p.output)?)))
        .set("output", |h, _, v| {
            let output = v.uint()?;
            change(h, |p| p.output = output)
        })
        .get("shown", |h, _| Ok(Val::Bool(h.ui.bar_shown(h.cid).map_err(Fail::err)?)))
        .function("give_way_to", |cx, args| {
            let ui = ui_of(cx)?;
            ui.w.borrow_mut().bars_cover = args.first().and_then(|v| v.closure().ok()).map(Rc::clone);
            ui.schedule();
            Ok(Val::None)
        })
        .function("on_outputs", |cx, args| {
            let ui = ui_of(cx)?;
            ui.w.borrow_mut().bars_primary = args.first().and_then(|v| v.closure().ok()).map(Rc::clone);
            ui.schedule();
            Ok(Val::None)
        })
        .function("insets", |cx, args| {
            let ui = ui_of(cx)?;
            let output = args.first().and_then(|v| v.uint().ok()).unwrap_or(0);
            let (top, bottom) = ui.bar_insets(output).map_err(Fail::err)?;
            Ok(Val::Record {
                class: 0,
                fields: vec![Val::Double(top), Val::Double(bottom)],
            })
        })
}

/// The members a toast adds, and its function.
fn toast_members(class: Class<'_>) -> Class<'_> {
    fn part<R>(h: &CanvasHandle, read: impl FnOnce(&super::widget::ToastPart) -> R) -> Res<R> {
        h.read(|w| match &w.sp {
            Special::Toast(part) => Some(read(part)),
            _ => None,
        })?
        .ok_or_else(|| Fail::type_error("the canvas is not a toast"))
    }
    fn change(h: &CanvasHandle, change: impl FnOnce(&mut super::widget::ToastPart)) -> Res<()> {
        h.edit(|w| {
            if let Special::Toast(part) = &mut w.sp {
                change(part);
            }
        })
    }
    class
        .get("text", |h, _| {
            let message = part(h, |p| p.message)?;
            Ok(Val::Text(h.ui.read(message, |w| w.t.text.clone()).unwrap_or_default()))
        })
        .set("text", |h, _, v| {
            let message = part(h, |p| p.message)?;
            let text = v.text()?.to_owned();
            h.ui.edit(message, |w| w.t.text = text)
        })
        .get("message", |h, cx| {
            let message = part(h, |p| p.message)?;
            Ok(match h.ui.read(message, |w| w.obj) {
                Some(obj) => cx.handout(obj)?,
                None => Val::None,
            })
        })
        .get("fade_in", |h, _| Ok(Val::Double(part(h, |p| p.fade_in)?)))
        .set("fade_in", |h, _, v| {
            let fade = v.double()?;
            change(h, |p| p.fade_in = fade)
        })
        .get("hold", |h, _| Ok(Val::Double(part(h, |p| p.hold)?)))
        .set("hold", |h, _, v| {
            let hold = v.double()?;
            change(h, |p| p.hold = hold)
        })
        .get("fade_out", |h, _| Ok(Val::Double(part(h, |p| p.fade_out)?)))
        .set("fade_out", |h, _, v| {
            let fade = v.double()?;
            change(h, |p| p.fade_out = fade)
        })
        .get("clock", |h, _| {
            Ok(part(h, |p| p.clock.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("clock", |h, _, v| {
            let source = Rc::clone(v.closure()?);
            change(h, |p| p.clock = Some(source))
        })
        .function("show", |cx, args| {
            let ui = ui_of(cx)?;
            let (obj, cid) = make_object(&ui, cx, WidgetKind::Toast)?;
            let text = args[0].text()?.to_owned();
            let number = |at: usize| args.get(at).filter(|v| !v.is_none()).and_then(|v| v.double().ok());
            let clock = args.get(4).and_then(|v| v.closure().ok()).map(Rc::clone);
            let message = ui.tweak(cid, |widget| {
                let Special::Toast(part) = &mut widget.sp else {
                    return None;
                };
                part.fade_in = number(1).unwrap_or(part.fade_in);
                part.hold = number(2).unwrap_or(part.hold);
                part.fade_out = number(3).unwrap_or(part.fade_out);
                if clock.is_some() {
                    part.clock = clock;
                }
                Some(part.message)
            });
            if let Some(Some(message)) = message {
                ui.tweak(message, |w| w.t.text = text);
            }
            cx.handout(obj)
        })
}

/// Makes an object of `kind` that is not the one the call is about (a function of a class makes one): the canvas and what its class
/// builds around it.
fn make_object(ui: &Rc<super::widget::Ui>, cx: &mut Cx<'_>, kind: WidgetKind) -> Res<(u64, Cid)> {
    let cid = ui.make(kind)?;
    let handle = CanvasHandle { ui: Rc::clone(ui), cid };
    let obj = cx.spawn(kind.class(), handle)?;
    ui.tweak(cid, |widget| widget.obj = obj);
    build_for(ui, cx, obj, cid, kind)?;
    Ok((obj, cid))
}

/// What a class builds around its canvas once it has an object: its hidden parts and its defaults.
fn build_for(ui: &Rc<super::widget::Ui>, cx: &mut Cx<'_>, obj: u64, cid: Cid, kind: WidgetKind) -> Res<()> {
    match kind {
        WidgetKind::Entry => entry::build(ui, cx, obj, cid)?,
        WidgetKind::Clock => {
            ui.tweak(cid, |widget| {
                widget.sp = Special::Clock(ClockPart {
                    format: "%H:%M".to_owned(),
                    now: None,
                });
            });
            clock::set_text(ui, cid).map_err(Fail::err)?;
        }
        WidgetKind::Bar => {
            ui.tweak(cid, |widget| {
                widget.b.direction = en::DIRECTION_ROW;
                widget.height = Some(32.0);
                widget.b.fill = "#1e1e2e".to_owned();
                widget.b.padding = 6.0;
                widget.b.gap = 12.0;
                widget.sp = Special::Bar(BarPart {
                    edge: en::BAR_TOP,
                    hide_when: None,
                    output: 0,
                });
            });
            ui.mount_top(cid)?;
        }
        WidgetKind::Toast => toast::build(ui, cx, obj, cid)?,
        _ => {}
    }
    Ok(())
}

/// Registers one class of the toolkit: its constructor and the members of the classes it derives from.
fn class<'a>(builder: &'a mut Builder, kind: WidgetKind) -> Class<'a> {
    class_with(builder, kind.class(), kind, move |ui, cx, _inits, cid| {
        let obj = cx.me();
        build_for(ui, cx, obj, cid, kind)
    })
}

/// Registers a class named `name` whose canvases are of `kind`: its constructor makes the canvas, then lets `make` build what the class
/// adds (and take the initial values it names), and the members of the classes it derives from are added. A class of the frame module
/// derives from one of these and is registered through here.
pub(crate) fn class_with<'a>(
    builder: &'a mut Builder,
    name: &str,
    kind: WidgetKind,
    make: impl Fn(&Rc<super::widget::Ui>, &mut Cx<'_>, &mut gnoblin_object::server::Inits, Cid) -> Res<()> + 'static,
) -> Class<'a> {
    let made = builder.class::<CanvasHandle>(name).ctor(move |cx, inits| {
        let ui = ui_of(cx)?;
        let obj = cx.me();
        let handle = ui.make_for(obj, kind)?;
        make(&ui, cx, inits, handle.cid)?;
        Ok(handle)
    });
    let made = canvas_members(made);
    if kind.is_box() { box_members(made) } else { made }
}

/// The members an icon adds.
pub(crate) fn icon_members(class: Class<'_>) -> Class<'_> {
    class
        .get("icon", |h, _| h.text(|w| &w.i.icon))
        .set("icon", |h, _, v| {
            let icon = v.text()?.to_owned();
            h.edit(|w| w.i.icon = icon)
        })
        .get("size", |h, _| h.number(|w| w.i.size))
        .set("size", |h, _, v| h.edit(|w| w.i.size = v.double().unwrap_or(16.0)))
        .get("color", |h, _| h.text(|w| &w.i.color))
        .set("color", |h, _, v| {
            let color = v.text()?.to_owned();
            h.edit(|w| w.i.color = color)
        })
}

pub(super) fn register(builder: &mut Builder) {
    let _ = class(builder, WidgetKind::Canvas);
    let _ = class(builder, WidgetKind::Box);
    let _ = class(builder, WidgetKind::Grid);
    let _ = text_members(class(builder, WidgetKind::Text));
    let _ = icon_members(class(builder, WidgetKind::Icon));
    let _ = entry_members(class(builder, WidgetKind::Entry));
    let _ = clock_members(text_members(class(builder, WidgetKind::Clock)));
    let _ = bar_members(class(builder, WidgetKind::Bar));
    let _ = toast_members(class(builder, WidgetKind::Toast));
    let _ = builder
        .class::<()>("time_format")
        .function("format_time", |_cx, args| clock::call(args));
}
