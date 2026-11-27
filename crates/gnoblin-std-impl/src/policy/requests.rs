//! `window_state`: the answers to the requests windows make of the shell, and the drags that `move` and `resize` start.
//!
//! The compositor waits for the shell to answer each request of a window (`maximize`, `unmaximize`, `fullscreen`, `unfullscreen`, `minimize`,
//! `unminimize`, `close`, `raise`, `lower`, `activate`, `move`, `resize` and `menu`). A listener (`request_listener`) hears each one first, a
//! window's own listeners before the listeners of every window, in the order they were added; the first that calls `handle` answers it. If none
//! does, the stock policy runs, so two answers never meet. A listener that throws is reported and skipped. `menu` is answered by the window
//! menu (`window_menus`) when no listener handled it.
//!
//! - `maximize` fills the work area (the area the bars leave) and keeps the floating rectangle. A snapped window is unsnapped first. Refused
//!   when the window is maximised or fullscreen already.
//! - `fullscreen` covers the whole output, keeps the windowed rectangle and puts the window last among its siblings. Refused when it is
//!   fullscreen already.
//! - `minimize` hides the window and, if it held the keyboard, gives it to the topmost sibling that shows, or to nobody. `unminimize` shows it and
//!   gives it the keyboard only when nothing else has it.
//! - `activate` presents a window of the shown workspace: it comes back if minimised, and is focused and raised. A window of another workspace is
//!   left alone.
//! - `move` and `resize` start the pointer drags of the move and resize bindings. A tiled, fullscreen or maximised window is not dragged. The
//!   pointer chooses the side or the corner a resize takes.

use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_object::server::{Builder, ShellImpl};
use gnoblin_object::{Closure, ErrorCode, Fail, Obj, Res, Val};
use gnoblin_std_abi::{STD_CONSTRAINT_MINIMUM_STRIP, STD_CONSTRAINT_WORK_AREA};

use super::drags::work_area;
use super::stack::{self, alive, rect_in, rect_val};
use crate::bindings::steps::{DragRule, GripChoice};
use crate::bindings::{DragCtx, DragHandler, RuleDrag};
use crate::builtin::Pt;
use crate::floating::shrink;
use crate::nodes;
use crate::object::client::{client_node, optional_client_node};
use crate::object::record::{edit, enumeration, look, record};
use crate::object::with_lib;
use crate::state::lookup_quiet;
use crate::ui::{Ui, ui_of};

/// How near two sides of a window a resize request's pointer must be for the corner between them to move, in pixels.
const RESIZE_CORNER_ZONE: f64 = 16.0;

/// A drag a request started ends after this long even when no button edge arrives, so it cannot follow the pointer indefinitely.
const REQUEST_DRAG_LIMIT_MS: f64 = 30_000.0;

/// Every request the compositor makes of a window, in the order of `client_request_kind`.
const KINDS: [&str; 13] = [
    "maximize",
    "unmaximize",
    "fullscreen",
    "unfullscreen",
    "minimize",
    "unminimize",
    "close",
    "raise",
    "lower",
    "activate",
    "move",
    "resize",
    "menu",
];

const MAXIMIZE: u32 = 1;
const UNMAXIMIZE: u32 = 2;
const FULLSCREEN: u32 = 3;
const UNFULLSCREEN: u32 = 4;
const MINIMIZE: u32 = 5;
const UNMINIMIZE: u32 = 6;
const CLOSE: u32 = 7;
const RAISE: u32 = 8;
const LOWER: u32 = 9;
const ACTIVATE: u32 = 10;
const MOVE: u32 = 11;
const RESIZE: u32 = 12;

/// The place of a request's name in `client_request_kind`, counted from 1.
fn kind_of(name: &str) -> Option<u32> {
    KINDS
        .iter()
        .position(|known| *known == name)
        .and_then(|at| u32::try_from(at + 1).ok())
}

/// A listener a shell added.
#[derive(Clone)]
struct HeldListener {
    obj: Obj,
    shell: Rc<ShellImpl>,
}

/// The rectangles a window goes back to.
#[derive(Clone, Copy, Default)]
struct Rests {
    floating: Option<Rect>,
    windowed: Option<Rect>,
}

/// A drag a request started.
struct RequestDrag {
    client: u64,
    rule: RuleDrag,
    origin: Pt,
    started: f64,
    last: Pt,
}

/// What the answers to window requests keep for one generation.
#[derive(Default)]
pub(crate) struct WindowRequests {
    started: bool,
    /// The `window_state` object, which owns what a request makes.
    owner: Option<Obj>,
    now: Option<Closure>,
    /// The listeners of every window, in the order they were added.
    all: Vec<HeldListener>,
    /// The listeners of one window.
    own: Vec<(u64, HeldListener)>,
    kept: HashMap<u64, Rests>,
    drag: Option<RequestDrag>,
}

/// The state of a `client_request`.
struct ClientAsk {
    kind: u32,
    client: u64,
    x: f64,
    y: f64,
    handled: bool,
}

/// The clock the drag's limit runs by, in milliseconds: the shell's, or the core's (see `Ui::core_now`).
fn clock(ui: &Ui) -> f64 {
    let source = ui.policy.borrow().requests.now.clone();
    ui.clock_ms(source.as_ref(), "the clock of the window requests")
}

/// Whether the answers run.
pub(crate) fn started(ui: &Ui) -> bool {
    ui.policy.borrow().requests.started
}

/// The id of `request_listener.on_request`.
fn listener_member() -> Option<u32> {
    with_lib(|lib| {
        let class = lib.class_id("request_listener")?;
        lib.member_id(class, gnoblin_core_abi::MEMBER_METHOD, "on_request")
    })
    .ok()
}

// ---- the compositor's requests --------------------------------------------------------------------------------------------------------

/// A request a window made. Answers whether a listener handled it (the stock answer ran when none did).
pub(crate) fn on_request(ui: &Rc<Ui>, node: u64, request: &str, x: f64, y: f64) -> bool {
    let owner = {
        let policy = ui.policy.borrow();
        if !policy.requests.started {
            return false;
        }
        policy.requests.owner
    };
    let (Some(kind), Some(owner)) = (kind_of(request), owner) else {
        return false;
    };
    if !alive(ui, node) {
        return false;
    }
    let listeners: Vec<HeldListener> = {
        let policy = ui.policy.borrow();
        policy
            .requests
            .own
            .iter()
            .filter(|(client, _)| *client == node)
            .map(|(_, listener)| listener.clone())
            .chain(policy.requests.all.iter().cloned())
            .collect()
    };
    let handled = ui
        .with_cx(owner, |cx| -> Res<bool> {
            let made = cx.spawn(
                "client_request",
                ClientAsk {
                    kind,
                    client: node,
                    x,
                    y,
                    handled: false,
                },
            )?;
            // The library keeps a reference to read what the listeners answered; each listener is handed one of its own.
            cx.hold(made)?;
            let member = listener_member();
            let mut handled = false;
            for listener in listeners {
                let Some(member) = member.filter(|member| listener.shell.has(*member)) else {
                    continue;
                };
                let value = cx.handout(made)?;
                if let Err(fail) = listener.shell.call(member, std::slice::from_ref(&value)) {
                    ui.warn(&format!("a request listener threw: {fail}"));
                }
                if cx
                    .with_state(made, |held: &mut ClientAsk| held.handled)
                    .unwrap_or(false)
                {
                    handled = true;
                    break;
                }
            }
            cx.unhold(made);
            Ok(handled)
        })
        .and_then(Result::ok)
        .unwrap_or(false);
    if !handled {
        answer(ui, node, kind);
    }
    handled
}

/// Answers `kind` for `client` as the stock policy does.
fn answer(ui: &Rc<Ui>, client: u64, kind: u32) {
    match kind {
        MAXIMIZE => drop(maximise(ui, client)),
        UNMAXIMIZE => drop(unmaximise(ui, client)),
        FULLSCREEN => drop(enter_fullscreen(ui, client)),
        UNFULLSCREEN => drop(leave_fullscreen(ui, client)),
        MINIMIZE => drop(minimise(ui, client)),
        UNMINIMIZE => drop(unminimise(ui, client)),
        CLOSE => drop(ui.node(client).call("close", &[])),
        RAISE => stack::raise(ui, client),
        LOWER => stack::lower(ui, client),
        ACTIVATE => drop(present(ui, client)),
        MOVE => drop(begin_move(ui, client)),
        RESIZE => drop(begin_resize(ui, client)),
        _ => {}
    }
}

// ---- the answers ----------------------------------------------------------------------------------------------------------------------

fn remember(ui: &Ui, client: u64, change: impl FnOnce(&mut Rests)) {
    change(ui.policy.borrow_mut().requests.kept.entry(client).or_default());
}

fn forget(ui: &Ui, client: u64, take: impl FnOnce(&mut Rests) -> Option<Rect>) -> Option<Rect> {
    take(ui.policy.borrow_mut().requests.kept.entry(client).or_default())
}

/// A window's frame took another room: the FRAME keeps its place, so the rectangle the window returns to from maximise moves in by what
/// the top and left gained and stays inside the work area. The library does the same for what it keeps itself (`follow_frame`).
pub(super) fn follow_frame(ui: &Ui, client: u64, was: &crate::floating::Inset, now: &crate::floating::Inset) {
    let Some(area) = look(ui.ctx, |st| Ok(st.work_area_of_client(client))).ok().flatten() else {
        return;
    };
    let mut policy = ui.policy.borrow_mut();
    let Some(floating) = policy
        .requests
        .kept
        .get_mut(&client)
        .and_then(|kept| kept.floating.as_mut())
    else {
        return;
    };
    let moved = Rect::new(
        floating.x + now.left - was.left,
        floating.y + now.top - was.top,
        floating.width,
        floating.height,
    );
    *floating = crate::client_state::fit_content(&moved, now, &area);
}

/// Fills the work area and keeps the floating rectangle. Refused (false) when the window is maximised or fullscreen already.
pub(super) fn maximise(ui: &Rc<Ui>, client: u64) -> bool {
    if nodes::flag(&ui.g, client, "maximised") || nodes::flag(&ui.g, client, "fullscreen") {
        return false;
    }
    let before = edit(ui.ctx, |st| {
        if st.rec.get(&client).is_some_and(|rec| rec.snap.is_some()) {
            st.unsnap_client(client, false);
        }
        let before = nodes::rect_of(&st.g, client);
        let inset = st.inset_of(client);
        if let Some(rec) = st.rec.get_mut(&client) {
            rec.float_rect = Some(before);
        }
        nodes::write_flag(&st.g, client, "maximised", true);
        if let Some(area) = st.work_area_of_client(client) {
            nodes::write_rect(&st.g, client, &shrink(&area, &inset));
        }
        Ok(before)
    });
    match before {
        Ok(before) => {
            remember(ui, client, |kept| kept.floating = Some(before));
            true
        }
        Err(fail) => {
            ui.warn(&format!("the window could not be maximised: {fail}"));
            false
        }
    }
}

/// Leaves maximised and returns the floating rectangle. Refused (false) when the window is not maximised.
pub(super) fn unmaximise(ui: &Rc<Ui>, client: u64) -> bool {
    if !nodes::flag(&ui.g, client, "maximised") {
        return false;
    }
    nodes::write_flag(&ui.g, client, "maximised", false);
    if let Some(before) = forget(ui, client, |kept| kept.floating.take()) {
        nodes::write_rect(&ui.g, client, &before);
    }
    true
}

/// Covers the whole output and goes last among its siblings. Refused (false) when the window is fullscreen already.
fn enter_fullscreen(ui: &Rc<Ui>, client: u64) -> bool {
    if nodes::flag(&ui.g, client, "fullscreen") {
        return false;
    }
    let output = look(ui.ctx, |st| Ok(st.output_rect_of(client))).ok().flatten();
    if output.is_some() {
        let before = nodes::rect_of(&ui.g, client);
        remember(ui, client, |kept| kept.windowed = Some(before));
    }
    nodes::write_flag(&ui.g, client, "fullscreen", true);
    if let Some(output) = output {
        nodes::write_rect(&ui.g, client, &output);
        stack::raise(ui, client);
    }
    true
}

/// Returns the rectangle the window had before it went fullscreen. Refused (false) when it is not fullscreen.
fn leave_fullscreen(ui: &Rc<Ui>, client: u64) -> bool {
    if !nodes::flag(&ui.g, client, "fullscreen") {
        return false;
    }
    nodes::write_flag(&ui.g, client, "fullscreen", false);
    if let Some(before) = forget(ui, client, |kept| kept.windowed.take()) {
        nodes::write_rect(&ui.g, client, &before);
    }
    true
}

/// Hides the window and hands the keyboard on: the topmost sibling still showing, or nobody. A window that did not hold the keyboard leaves
/// it where it is. Refused (false) when it is minimised already.
pub(super) fn minimise(ui: &Rc<Ui>, client: u64) -> bool {
    if nodes::flag(&ui.g, client, "minimised") {
        return false;
    }
    nodes::write_flag(&ui.g, client, "minimised", true);
    if stack::focused(ui) == Some(client) {
        let next = stack::siblings(ui, client)
            .into_iter()
            .rev()
            .find(|other| *other != client && alive(ui, *other) && !nodes::flag(&ui.g, *other, "minimised"));
        stack::focus(ui, next);
    }
    true
}

/// Shows the window again. The keyboard goes to it only when nothing holds it. Refused (false) when it is not minimised.
fn unminimise(ui: &Rc<Ui>, client: u64) -> bool {
    if !nodes::flag(&ui.g, client, "minimised") {
        return false;
    }
    nodes::write_flag(&ui.g, client, "minimised", false);
    if stack::focused(ui).is_none() {
        stack::focus(ui, Some(client));
    }
    true
}

/// An activation: brings a minimised window back, focuses and raises it, when its workspace is the one on show.
fn present(ui: &Rc<Ui>, client: u64) -> bool {
    if !stack::on_active_workspace(ui, client) {
        return false;
    }
    if nodes::flag(&ui.g, client, "minimised") {
        unminimise(ui, client);
    }
    stack::focus(ui, Some(client));
    stack::raise(ui, client);
    true
}

// ---- the drags a move or resize request starts -------------------------------------------------------------------------------------------

/// The seat's rules for a drag: how far a window may leave the work area, and whether the edges tile.
fn seat_rules(ui: &Ui) -> (u32, bool) {
    let seat = nodes::seat(&ui.g);
    let constraint = match seat.as_ref().and_then(|seat| seat.get_text("drag_constraint").ok()) {
        Some(rule) if rule == "minimum_strip" => STD_CONSTRAINT_MINIMUM_STRIP,
        _ => STD_CONSTRAINT_WORK_AREA,
    };
    let edge_tiling = seat
        .as_ref()
        .and_then(|seat| seat.get_bool("edge_tiling").ok())
        .unwrap_or(false);
    (constraint, edge_tiling)
}

/// The step of a drag a request started, with the pointer at `at`.
fn step(ui: &Ui, client: u64, origin: Pt, at: Pt) -> DragCtx {
    let bounds = nodes::rect_of(&ui.g, client);
    let (constraint, edge_tiling) = seat_rules(ui);
    DragCtx {
        client: Some(client),
        position: at,
        origin,
        local: Some(Pt {
            x: at.x - bounds.x,
            y: at.y - bounds.y,
        }),
        button: 1,
        work_area: work_area(ui.ctx, Some(client)),
        constraint,
        edge_tiling,
    }
}

/// The side of the window a resize request takes hold of: the corner the pointer is near, or the nearest side.
fn resize_grip(bounds: &Rect, at: Pt) -> u32 {
    let (x, y) = (at.x - bounds.x, at.y - bounds.y);
    let near_x = x.min(bounds.width - x);
    let near_y = y.min(bounds.height - y);
    if near_x <= RESIZE_CORNER_ZONE && near_y <= RESIZE_CORNER_ZONE {
        return match (y < bounds.height / 2.0, x < bounds.width / 2.0) {
            (true, true) => 2,
            (true, false) => 3,
            (false, true) => 4,
            (false, false) => 5,
        };
    }
    1
}

/// Starts the drag of `rule` for `client`, following the seat's pointer. False when none starts.
fn begin(ui: &Rc<Ui>, client: u64, make: impl FnOnce(&Rect, Pt) -> DragRule) -> bool {
    if !alive(ui, client)
        || ["tiled", "fullscreen", "maximised"]
            .iter()
            .any(|flag| nodes::flag(&ui.g, client, flag))
    {
        return false;
    }
    let Some((x, y)) = ui.pointer_place() else {
        return false;
    };
    let pointer = Pt { x, y };
    let Ok(std) = lookup_quiet(ui.ctx) else {
        return false;
    };
    if ui.policy.borrow().requests.drag.is_some() {
        end(ui);
    }
    let rule = make(&nodes::rect_of(&ui.g, client), pointer);
    let mut held = RuleDrag::new(rule);
    if !held.start(&std, &step(ui, client, pointer, pointer)) {
        return false;
    }
    let started = clock(ui);
    ui.policy.borrow_mut().requests.drag = Some(RequestDrag {
        client,
        rule: held,
        origin: pointer,
        started,
        last: pointer,
    });
    ui.schedule();
    true
}

/// Starts the stock move drag for `client`, following the seat's pointer. False when none starts.
pub(super) fn begin_move(ui: &Rc<Ui>, client: u64) -> bool {
    begin(ui, client, |_, _| DragRule::Move { snap: Some(4.0) })
}

/// Starts the stock resize drag for `client`. The corner or the side is chosen from where the pointer is.
pub(super) fn begin_resize(ui: &Rc<Ui>, client: u64) -> bool {
    begin(ui, client, |bounds, at| DragRule::Resize {
        grip: GripChoice::from_member(resize_grip(bounds, at)),
    })
}

/// Moves the drag to the pointer's place now. A window that is gone or an expired limit ends the drag.
fn follow(ui: &Rc<Ui>, ending: bool) {
    let Some(mut drag) = ui.policy.borrow_mut().requests.drag.take() else {
        return;
    };
    let Ok(std) = lookup_quiet(ui.ctx) else {
        return;
    };
    if !alive(ui, drag.client) {
        return;
    }
    if !ending && clock(ui) - drag.started >= REQUEST_DRAG_LIMIT_MS {
        ui.policy.borrow_mut().requests.drag = Some(drag);
        end(ui);
        return;
    }
    if let Some((x, y)) = ui.pointer_place() {
        let at = Pt { x, y };
        if at != drag.last {
            drag.last = at;
            let event = step(ui, drag.client, drag.origin, at);
            drag.rule.update(&std, &event);
        }
    }
    ui.policy.borrow_mut().requests.drag = Some(drag);
}

/// Ends the drag after one last step to where the pointer is.
fn end(ui: &Rc<Ui>) {
    if ui.policy.borrow().requests.drag.is_none() {
        return;
    }
    follow(ui, true);
    let Some(mut drag) = ui.policy.borrow_mut().requests.drag.take() else {
        return;
    };
    if !alive(ui, drag.client) {
        return;
    }
    if let Ok(std) = lookup_quiet(ui.ctx) {
        let event = step(ui, drag.client, drag.origin, drag.last);
        drag.rule.end(&std, &event);
    }
}

/// The pointer's button edges on a window: a press or a release ends the drag a request started.
pub(crate) fn pointer_edge(ui: &Rc<Ui>, kind: &str) {
    if started(ui) && (kind == "press" || kind == "release") {
        end(ui);
    }
}

/// The drag follows the pointer once a frame of the toolkit.
pub(crate) fn turn(ui: &Rc<Ui>) {
    if ui.policy.borrow().requests.drag.is_some() {
        follow(ui, false);
    }
}

/// How long until the drag's limit, in milliseconds; the toolkit also asks for a turn on every frame while a drag follows the pointer.
pub(crate) fn wake_in(ui: &Ui) -> Option<u64> {
    let started = ui.policy.borrow().requests.drag.as_ref().map(|drag| drag.started)?;
    let left = (started + REQUEST_DRAG_LIMIT_MS - clock(ui)).max(0.0);
    Some(left.min(16.0).ceil() as u64)
}

// ---- the classes ----------------------------------------------------------------------------------------------------------------------

/// A listener as a shell gave it.
fn listener_of(cx: &gnoblin_object::server::Cx<'_>, value: &Val) -> Res<HeldListener> {
    let obj = value.object()?;
    let shell = cx.shell(obj)?;
    Ok(HeldListener { obj, shell })
}

fn rects_val(kept: Rests) -> Val {
    record(vec![
        kept.floating.as_ref().map_or(Val::None, rect_val),
        kept.windowed.as_ref().map_or(Val::None, rect_val),
    ])
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<ClientAsk>("client_request")
        .ctor(|_cx, _inits| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "the library makes a request when a window asks",
            ))
        })
        .get("kind", |h, _| Ok(enumeration(h.kind)))
        .get("client", |h, _| Ok(Val::Text(h.client.to_string())))
        .get("x", |h, _| Ok(Val::Double(h.x)))
        .get("y", |h, _| Ok(Val::Double(h.y)))
        .get("handled", |h, _| Ok(Val::Bool(h.handled)))
        .method("handle", |h, _, _| {
            h.handled = true;
            Ok(Val::None)
        });
    let _ = builder
        .class::<()>("window_state")
        .ctor(|_cx, _inits| Ok(()))
        .get("started", |_, cx| Ok(Val::Bool(started(&*ui_of(cx)?))))
        .get("dragging", |_, cx| {
            Ok(Val::Bool(ui_of(cx)?.policy.borrow().requests.drag.is_some()))
        })
        .get("resize_corner_zone", |_, _| Ok(Val::Double(RESIZE_CORNER_ZONE)))
        .get("drag_limit_ms", |_, _| Ok(Val::Double(REQUEST_DRAG_LIMIT_MS)))
        .get("now", |_, cx| {
            Ok(ui_of(cx)?
                .policy
                .borrow()
                .requests
                .now
                .clone()
                .map_or(Val::None, Val::Closure))
        })
        .set("now", |_, cx, value| {
            ui_of(cx)?.policy.borrow_mut().requests.now = if value.is_none() {
                None
            } else {
                Some(Rc::clone(value.closure()?))
            };
            Ok(())
        })
        .method("start", |_, cx, _| {
            let ui = ui_of(cx)?;
            let mut policy = ui.policy.borrow_mut();
            if !policy.requests.started {
                policy.requests.started = true;
                policy.requests.owner = Some(cx.me());
                drop(policy);
                ui.schedule();
            }
            Ok(Val::None)
        })
        .method("add_listener", |_, cx, args| {
            let ui = ui_of(cx)?;
            let listener = listener_of(cx, &args[0])?;
            let client = optional_client_node(cx.ctx(), "add_listener", args.get(1))?;
            cx.hold(listener.obj)?;
            let mut policy = ui.policy.borrow_mut();
            match client {
                Some(client) => policy.requests.own.push((client, listener)),
                None => policy.requests.all.push(listener),
            }
            Ok(Val::None)
        })
        .method("remove_listener", |_, cx, args| {
            let ui = ui_of(cx)?;
            let obj = args[0].object()?;
            let mut gone = 0;
            {
                let mut policy = ui.policy.borrow_mut();
                let before = policy.requests.all.len() + policy.requests.own.len();
                policy.requests.all.retain(|held| held.obj != obj);
                policy.requests.own.retain(|(_, held)| held.obj != obj);
                gone += before - policy.requests.all.len() - policy.requests.own.len();
            }
            for _ in 0..gone {
                cx.unhold(obj);
            }
            Ok(Val::None)
        })
        .method("answer", |_, cx, args| {
            let ui = ui_of(cx)?;
            answer(&ui, client_node(cx.ctx(), "answer", &args[0])?, args[1].enum_value()?);
            Ok(Val::None)
        })
        .method("maximise", |_, cx, args| {
            Ok(Val::Bool(maximise(
                &ui_of(cx)?,
                client_node(cx.ctx(), "maximise", &args[0])?,
            )))
        })
        .method("unmaximise", |_, cx, args| {
            Ok(Val::Bool(unmaximise(
                &ui_of(cx)?,
                client_node(cx.ctx(), "unmaximise", &args[0])?,
            )))
        })
        .method("enter_fullscreen", |_, cx, args| {
            Ok(Val::Bool(enter_fullscreen(
                &ui_of(cx)?,
                client_node(cx.ctx(), "enter_fullscreen", &args[0])?,
            )))
        })
        .method("leave_fullscreen", |_, cx, args| {
            Ok(Val::Bool(leave_fullscreen(
                &ui_of(cx)?,
                client_node(cx.ctx(), "leave_fullscreen", &args[0])?,
            )))
        })
        .method("minimise", |_, cx, args| {
            Ok(Val::Bool(minimise(
                &ui_of(cx)?,
                client_node(cx.ctx(), "minimise", &args[0])?,
            )))
        })
        .method("unminimise", |_, cx, args| {
            Ok(Val::Bool(unminimise(
                &ui_of(cx)?,
                client_node(cx.ctx(), "unminimise", &args[0])?,
            )))
        })
        .method("present", |_, cx, args| {
            Ok(Val::Bool(present(
                &ui_of(cx)?,
                client_node(cx.ctx(), "present", &args[0])?,
            )))
        })
        .method("raise", |_, cx, args| {
            stack::raise(&*ui_of(cx)?, client_node(cx.ctx(), "raise", &args[0])?);
            Ok(Val::None)
        })
        .method("lower", |_, cx, args| {
            stack::lower(&*ui_of(cx)?, client_node(cx.ctx(), "lower", &args[0])?);
            Ok(Val::None)
        })
        .method("begin_move", |_, cx, args| {
            Ok(Val::Bool(begin_move(
                &ui_of(cx)?,
                client_node(cx.ctx(), "begin_move", &args[0])?,
            )))
        })
        .method("begin_resize", |_, cx, args| {
            Ok(Val::Bool(begin_resize(
                &ui_of(cx)?,
                client_node(cx.ctx(), "begin_resize", &args[0])?,
            )))
        })
        .method("kept", |_, cx, args| {
            let ui = ui_of(cx)?;
            let client = client_node(cx.ctx(), "kept", &args[0])?;
            let kept = ui
                .policy
                .borrow()
                .requests
                .kept
                .get(&client)
                .copied()
                .unwrap_or_default();
            Ok(rects_val(kept))
        })
        .method("keep", |_, cx, args| {
            let ui = ui_of(cx)?;
            let fields = args[1].record()?;
            let client = client_node(cx.ctx(), "keep", &args[0])?;
            let take = |at: usize| -> Res<Option<Rect>> {
                fields.get(at).filter(|value| !value.is_none()).map(rect_in).transpose()
            };
            let (floating, windowed) = (take(0)?, take(1)?);
            remember(&ui, client, |kept| {
                if floating.is_some() {
                    kept.floating = floating;
                }
                if windowed.is_some() {
                    kept.windowed = windowed;
                }
            });
            Ok(Val::None)
        });
}
