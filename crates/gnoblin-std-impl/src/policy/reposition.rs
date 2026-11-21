//! `repositioning`: keyboard move and resize. The window menu's Move and Resize start them. The arrow keys move the window by `key_step`
//! pixels (or grow and shrink it by that much when resizing).
//!
//! Return, Enter or Space keeps the new place; Escape puts the window back where it was. The mode holds the keyboard (a `reposition_grab`
//! canvas has it) and gives it back to the window when it ends. It also ends when the window closes (the window goes back) or the session locks
//! (the window keeps its place). Only floating windows on the shown workspace can move this way: a maximised, fullscreen or tiled window is
//! refused, and a snapped one is unsnapped first. Only one window can be repositioned at a time.

use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_std_abi::{STD_CONSTRAINT_MINIMUM_STRIP, STD_CONSTRAINT_WORK_AREA};

use super::drags::work_area;
use super::stack::{self, alive, client_of, rect_val};
use crate::bindings::steps::clamp_axis;
use crate::client_state::Snap;
use crate::floating::{grow, shrink};
use crate::nodes;
use crate::object::record::{edit, enumeration, look, record};
use crate::ui::{CanvasEvent, CanvasHandle, Cid, Special, Ui, WidgetKind, box_members, canvas_members, en, ui_of};

/// How far an arrow key moves or resizes the window, in pixels.
const KEY_STEP: f64 = 16.0;
/// The least width a resize keeps, in pixels.
const MIN_WIDTH: f64 = 120.0;
/// The least height a resize keeps, in pixels.
const MIN_HEIGHT: f64 = 80.0;

const MODE_MOVE: u32 = 1;
const MODE_RESIZE: u32 = 2;

/// What a keyboard mode keeps of its window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GrabPart {
    client: u64,
    mode: u32,
    /// Where the window was when the mode began: `finish(false)` puts it back.
    from: Rect,
    /// The snap it had, which `finish(false)` gives back.
    snapped: Option<Snap>,
}

/// What the keyboard mode keeps for one generation.
#[derive(Default)]
pub(crate) struct KeyMode {
    started: bool,
    /// The `repositioning` object.
    owner: Option<Obj>,
    /// The canvas that holds the keyboard while a mode runs.
    current: Option<Cid>,
}

/// Whether the mode can be begun.
pub(crate) fn started(ui: &Ui) -> bool {
    ui.policy.borrow().reposition.started
}

/// The part of a canvas that holds a mode.
fn part_of(ui: &Ui, cid: Cid) -> Option<GrabPart> {
    ui.read(cid, |widget| match &widget.sp {
        Special::Reposition(part) => Some(**part),
        _ => None,
    })
    .flatten()
}

/// The mode that runs, with its canvas.
fn current(ui: &Ui) -> Option<(Cid, GrabPart)> {
    let cid = ui.policy.borrow().reposition.current?;
    part_of(ui, cid).map(|part| (cid, part))
}

/// The seat's `drag_constraint` as the rule's constraint.
fn constraint(ui: &Ui) -> u32 {
    let rule = nodes::seat(&ui.g).and_then(|seat| seat.get_text("drag_constraint").ok());
    if rule.as_deref() == Some("minimum_strip") {
        STD_CONSTRAINT_MINIMUM_STRIP
    } else {
        STD_CONSTRAINT_WORK_AREA
    }
}

/// Starts the mode for `client`. The mode that ran ends first, keeping where its window is. False for a window that is fullscreen,
/// maximised, tiled, gone or not showing, and before `start`.
pub(crate) fn begin(ui: &Rc<Ui>, cx: &mut Cx<'_>, client: u64, mode: u32) -> Res<bool> {
    let owner = {
        let policy = ui.policy.borrow();
        if !policy.reposition.started {
            return Ok(false);
        }
        policy.reposition.owner
    };
    let Some(owner) = owner else {
        return Ok(false);
    };
    finish(ui, true);
    let floating = alive(ui, client)
        && !["fullscreen", "maximised", "tiled"]
            .iter()
            .any(|flag| nodes::flag(&ui.g, client, flag));
    if !floating || !stack::on_active_workspace(ui, client) {
        return Ok(false);
    }
    let from = nodes::rect_of(&ui.g, client);
    let snapped = look(ui.ctx, |st| Ok(st.rec.get(&client).and_then(|rec| rec.snap)))?;
    if snapped.is_some() {
        edit(ui.ctx, |st| Ok(st.unsnap_client(client, false)))?;
    }
    let (obj, cid) = ui.spawn_named(cx, "reposition_grab", WidgetKind::Box, owner)?;
    let secure = nodes::flag(&ui.g, client, "secure");
    let (_, text) = ui.spawn(cx, WidgetKind::Text, obj)?;
    ui.tweak(text, |widget| {
        widget.t.text = format!(
            "{}: arrow keys, Return to keep it, Escape to cancel",
            if mode == MODE_MOVE { "Move" } else { "Resize" }
        );
        widget.t.size = 14.0;
        widget.b.grow = 1.0;
    });
    ui.tweak(cid, |widget| {
        widget.x = 20.0;
        widget.y = 20.0;
        widget.width = Some(520.0);
        widget.height = Some(36.0);
        widget.b.radius = 8.0;
        widget.b.fill = "#1e1e2e".to_owned();
        widget.b.padding = 8.0;
        widget.b.direction = en::DIRECTION_ROW;
        widget.secure = secure;
        widget.sp = Special::Reposition(Box::new(GrabPart {
            client,
            mode,
            from,
            snapped,
        }));
    });
    ui.link(cid, text)?;
    ui.mount_top(cid)?;
    if let Some(node) = ui.read(cid, |widget| widget.node) {
        ui.focus_node(node)?;
    }
    ui.policy.borrow_mut().reposition.current = Some(cid);
    ui.announce(owner, |cx| {
        cx.changed("current");
        cx.changed("grab");
    });
    Ok(true)
}

/// One arrow key of the running mode: the window moves, or grows or shrinks, by `dx` and `dy`.
fn step(ui: &Ui, grab: &GrabPart, dx: f64, dy: f64) {
    let Some(area) = work_area(ui.ctx, Some(grab.client)) else {
        return;
    };
    let at = nodes::rect_of(&ui.g, grab.client);
    let inset = look(ui.ctx, |st| Ok(st.inset_of(grab.client))).unwrap_or_default();
    if grab.mode == MODE_MOVE {
        // The frame is what stays inside the area: the window's frame lies round the content.
        let rule = constraint(ui);
        let frame = grow(&at, &inset);
        let kept = Rect::new(
            clamp_axis(frame.x + dx, frame.width, area.x, area.width, rule),
            clamp_axis(frame.y + dy, frame.height, area.y, area.height, rule),
            frame.width,
            frame.height,
        );
        let content = shrink(&kept, &inset);
        nodes::write_rect(
            &ui.g,
            grab.client,
            &Rect::new(content.x, content.y, at.width, at.height),
        );
    } else {
        // The frame's far edges are what reach the area's.
        let widest = area.x + area.width - at.x - inset.right;
        let tallest = area.y + area.height - at.y - inset.bottom;
        nodes::write_rect(
            &ui.g,
            grab.client,
            &Rect::new(
                at.x,
                at.y,
                MIN_WIDTH.max((at.width + dx).min(widest)),
                MIN_HEIGHT.max((at.height + dy).min(tallest)),
            ),
        );
    }
}

/// Ends the mode. With `keep` the window stays where it is and that is the rectangle it floats at; without, it goes back to where the mode
/// found it, snapped again if it was. False when no mode was running.
pub(crate) fn finish(ui: &Rc<Ui>, keep: bool) -> bool {
    let Some((cid, grab)) = current(ui) else {
        return false;
    };
    let owner = {
        let mut policy = ui.policy.borrow_mut();
        policy.reposition.current = None;
        policy.reposition.owner
    };
    if alive(ui, grab.client) {
        if keep {
            let bounds = nodes::rect_of(&ui.g, grab.client);
            let _ = edit(ui.ctx, |st| {
                if let Some(rec) = st.rec.get_mut(&grab.client) {
                    rec.float_rect = Some(bounds);
                }
                Ok(())
            });
        } else {
            nodes::write_rect(&ui.g, grab.client, &grab.from);
            if let Some(snap) = grab.snapped {
                let _ = edit(ui.ctx, |st| {
                    if let Some(rec) = st.rec.get_mut(&grab.client) {
                        rec.snap = Some(snap);
                    }
                    Ok(())
                });
            }
        }
    }
    ui.kill(cid);
    if alive(ui, grab.client) {
        stack::focus(ui, Some(grab.client));
    }
    if let Some(owner) = owner {
        ui.announce(owner, |cx| {
            cx.changed("current");
            cx.changed("grab");
            cx.emit("ended", vec![Val::Text(grab.client.to_string()), Val::Bool(keep)]);
        });
    }
    true
}

/// A key for the grab: it takes every key, pressed or released, until the mode ends. Answers whether the canvas is a grab.
pub(crate) fn on_key(ui: &Rc<Ui>, cid: Cid, raw: &CanvasEvent) -> bool {
    let Some(grab) = part_of(ui, cid) else {
        return false;
    };
    if !raw.pressed {
        return true;
    }
    let arrow = match raw.key.as_str() {
        "Left" => Some((-1.0, 0.0)),
        "Right" => Some((1.0, 0.0)),
        "Up" => Some((0.0, -1.0)),
        "Down" => Some((0.0, 1.0)),
        _ => None,
    };
    if let Some((dx, dy)) = arrow {
        if ui.policy.borrow().reposition.current == Some(cid) {
            step(ui, &grab, dx * KEY_STEP, dy * KEY_STEP);
        }
    } else if matches!(raw.key.as_str(), "Return" | "KP_Enter" | "space") {
        finish(ui, true);
    } else if raw.key == "Escape" {
        finish(ui, false);
    }
    true
}

/// The window going ends the mode (it goes back), and the session locking ends it (it stays). Runs once a frame of the toolkit.
pub(crate) fn watch(ui: &Rc<Ui>) {
    let Some((_, grab)) = current(ui) else {
        return;
    };
    if !alive(ui, grab.client) {
        finish(ui, false);
    } else if ui.locked() {
        finish(ui, true);
    }
}

/// Whether the toolkit has to run every frame for the mode: the window going is not an event of the tree it can wait for.
pub(crate) fn polls(ui: &Ui) -> bool {
    ui.policy.borrow().reposition.current.is_some()
}

fn read<R>(handle: &CanvasHandle, read: impl FnOnce(&GrabPart) -> R) -> Res<R> {
    part_of(&handle.ui, handle.cid)
        .map(|part| read(&part))
        .ok_or_else(|| Fail::stale("the mode has ended"))
}

pub(super) fn register(builder: &mut Builder) {
    let class = builder
        .class::<CanvasHandle>("reposition_grab")
        .ctor(|_cx, _inits| Err(Fail::refused("the library makes the line when a keyboard mode begins")));
    let _ = box_members(canvas_members(class))
        .get("mode", |h, _| Ok(enumeration(read(h, |part| part.mode)?)))
        .get("client", |h, _| Ok(Val::Text(read(h, |part| part.client)?.to_string())))
        .get("from", |h, _| Ok(rect_val(&read(h, |part| part.from)?)))
        .get("snapped", |h, _| {
            Ok(read(h, |part| part.snapped)?.map_or(Val::None, |snap| {
                record(vec![enumeration(snap.zone), rect_val(&snap.floating)])
            }))
        })
        .method("step", |h, _, args| {
            let grab = read(h, |part| *part)?;
            if h.ui.policy.borrow().reposition.current == Some(h.cid) {
                step(&h.ui, &grab, args[0].double()?, args[1].double()?);
            }
            Ok(Val::None)
        });
    let _ = builder
        .class::<()>("repositioning")
        .ctor(|_cx, _inits| Ok(()))
        .get("started", |_, cx| Ok(Val::Bool(started(&*ui_of(cx)?))))
        .get("current", |_, cx| {
            let ui = ui_of(cx)?;
            Ok(current(&ui).map_or(Val::None, |(_, grab)| Val::Text(grab.client.to_string())))
        })
        .get("grab", |_, cx| {
            let ui = ui_of(cx)?;
            match current(&ui).and_then(|(cid, _)| ui.read(cid, |widget| widget.obj)) {
                Some(obj) => cx.handout(obj),
                None => Ok(Val::None),
            }
        })
        .get("key_step", |_, _| Ok(Val::Double(KEY_STEP)))
        .get("min_width", |_, _| Ok(Val::Double(MIN_WIDTH)))
        .get("min_height", |_, _| Ok(Val::Double(MIN_HEIGHT)))
        .method("start", |_, cx, _| {
            let ui = ui_of(cx)?;
            let mut policy = ui.policy.borrow_mut();
            if !policy.reposition.started {
                policy.reposition.started = true;
                policy.reposition.owner = Some(cx.me());
                drop(policy);
                ui.schedule();
            }
            Ok(Val::None)
        })
        .method("begin", |_, cx, args| {
            let ui = ui_of(cx)?;
            let mode = args[1].enum_value()?;
            Ok(Val::Bool(begin(&ui, cx, client_of(&args[0])?, mode)?))
        })
        .method("finish", |_, cx, args| {
            Ok(Val::Bool(finish(&ui_of(cx)?, args[0].bool()?)))
        });
}

/// Begins the keyboard mode for the window menu: `mode` is `MODE_MOVE` or `MODE_RESIZE`.
pub(crate) fn begin_from_menu(ui: &Rc<Ui>, client: u64, resize: bool) {
    let owner = ui.policy.borrow().reposition.owner;
    let Some(owner) = owner else {
        return;
    };
    let mode = if resize { MODE_RESIZE } else { MODE_MOVE };
    let done = ui.with_cx(owner, |cx| begin(ui, cx, client, mode));
    if let Some(Err(fail)) = done {
        ui.warn(&format!("the keyboard move or resize did not begin: {fail}"));
    }
}
