//! The decorations: a frame for each window, filled by a rule, by the shell's `frame` callback or by the stock titlebar, and the room each
//! frame reserves told to the library so its layouts leave that much free round the window.
//!
//! The decorations run from the toolkit's turn. A window that came, went or changed who draws its frame, or a property a frame follows
//! that changed, asks for a turn; the turn makes and drops frames to match the windows (`reconcile`), and after the frames are updated
//! (`send_insets`) tells the library how much room each reserves. What a window was last given is remembered even when it ended with no
//! frame, so a rule that leaves a window bare, or a fill that threw, is not run again until the window's kind of frame changes.

use std::rc::Rc;

use gnoblin_core::host::{Host, Value};
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::{Closure, Fail, Obj, Res, Val};

use super::client::Win;
use super::window_frame::{ActionsHandle, FrameHandle, destroy, resize_handles};
use super::{FrameId, FrameKind, FrameRec, FrameRule, titlebar};
use crate::floating::Inset;
use crate::nodes;
use crate::object::record::record;
use crate::object::with_lib;
use crate::ui::tree::{decorations_started_besides, find};
use crate::ui::widget::Ui;

/// The state of the `decorations` object: the generation's toolkit.
pub(crate) struct DecoHandle {
    pub(crate) ui: Rc<Ui>,
}

/// The window properties a frame follows: a change of one asks for a turn.
const FOLLOWED: [&str; 11] = [
    "title",
    "app_id",
    "focused",
    "fullscreen",
    "maximised",
    "minimised",
    "x",
    "y",
    "width",
    "height",
    "decoration",
];

/// How a window is to be filled.
enum Fill {
    /// A rule leaves the window without a frame.
    Bare,
    /// The stock titlebar.
    Stock,
    /// The shell's callback.
    Closure(Closure),
}

/// The first rule that takes the window decides how it is filled; with none, the `frame` callback does, and without one the stock titlebar.
fn fill_for(ui: &Rc<Ui>, node: u64) -> Result<Fill, String> {
    let win = Win::of(ui, node);
    let (app_id, title) = (win.app_id(), win.title());
    let (rules, fill): (Vec<FrameRule>, Option<Closure>) = {
        let world = ui.frames.borrow();
        (world.deco.rules.clone(), world.deco.fill.clone())
    };
    for rule in rules {
        let mut taken = rule.app_id.as_ref().is_none_or(|wanted| *wanted == app_id)
            && rule.title.as_ref().is_none_or(|wanted| *wanted == title);
        if taken && let Some(test) = &rule.test {
            taken = ui.ask_window(test, node)?;
        }
        if taken {
            return Ok(rule.fill.map_or(Fill::Bare, Fill::Closure));
        }
    }
    Ok(fill.map_or(Fill::Stock, Fill::Closure))
}

/// Writes the seat's `decorations_available`: whether this shell draws window decorations, so a client that asks for server-side is
/// told so.
fn declare(g: &Host, available: bool) {
    if let Some(seat) = nodes::seat(g) {
        // A seat that is going away, or a tree that refuses, is not an error of the decorations.
        let _ = seat.set("decorations_available", Value::Bool(available));
    }
}

/// Takes back the declaration of the generation `ctx`, unless another generation's decorations are started: a reload commits the new
/// generation before the old one retires, so a new generation that started its decorations has declared already and the old one's end
/// must not undo that, and a generation that was staged and failed must not undo the running one's.
fn withdraw(g: &Host, ctx: GnoblinCtx) {
    if !decorations_started_besides(ctx) {
        declare(g, false);
    }
}

/// Asks for a turn of the toolkit.
fn wake(ctx: gnoblin_core_abi::GnoblinCtx) {
    if let Some(ui) = find(ctx) {
        ui.schedule();
    }
}

/// Gives every window a frame, once. The library becomes the desktop (the layouts that leave room for the frames are its), the
/// decorations start listening and the seat is told this shell draws them.
///
/// # Errors
///
/// The library cannot start the desktop, or a subscription is refused.
pub(crate) fn start(ui: &Rc<Ui>) -> Res<()> {
    if ui.frames.borrow().deco.started {
        return Ok(());
    }
    crate::events::open(ui.ctx).map_err(Fail::from)?;
    let ctx = ui.ctx;
    let g = &ui.g;
    let refuse = |error: gnoblin_core::host::CoreError| Fail::new(error.kind().code(), error.to_string());
    let mut listening = Vec::new();
    listening.push(g.on_node_created("window", move |_| wake(ctx)).map_err(refuse)?);
    listening.push(g.on_node_destroyed("window", move |_| wake(ctx)).map_err(refuse)?);
    for property in FOLLOWED {
        listening.push(
            g.on_class_property_changed("window", property, move |_, _| wake(ctx))
                .map_err(refuse)?,
        );
    }
    // The window the compositor takes a frame for stops being framed when this shell goes: the declaration is taken back. The toolkit
    // of the generation is gone by the time this runs (its own hook is registered first), so the hook holds the handle itself.
    let farewell = g.clone();
    listening.push(g.on_shutdown(move || withdraw(&farewell, ctx)).map_err(refuse)?);
    {
        let mut world = ui.frames.borrow_mut();
        world.deco.listening = listening;
        world.deco.started = true;
    }
    declare(&ui.g, true);
    ui.schedule();
    Ok(())
}

/// Takes every frame away, tells the library the room is free again and declares that this shell no longer draws decorations.
pub(crate) fn stop(ui: &Rc<Ui>) {
    let (listening, frames) = {
        let mut world = ui.frames.borrow_mut();
        if !world.deco.started {
            return;
        }
        world.deco.started = false;
        world.decided.clear();
        (
            std::mem::take(&mut world.deco.listening),
            world.frames.keys().copied().collect::<Vec<FrameId>>(),
        )
    };
    for subscription in listening {
        // A subscription of a generation that retired is already gone.
        let _ = subscription.unsubscribe();
    }
    for frame in frames {
        destroy(ui, frame);
    }
    send(ui);
    withdraw(&ui.g, ui.ctx);
}

/// Makes the frame of the window `node` for the kind of frame it takes: the object, its actions, and what fills it.
///
/// A fill that threw, or answered `false`, leaves no frame behind.
fn build(ui: &Rc<Ui>, node: u64, kind: FrameKind) {
    let fill = match fill_for(ui, node) {
        Ok(Fill::Bare) => return,
        Ok(fill) => fill,
        Err(message) => {
            ui.warn(&format!("Decorations.onAdded threw: {message}"));
            return;
        }
    };
    let deco = ui.frames.borrow().deco.obj;
    let ctx = ui.ctx;
    let made = with_lib(|lib| {
        lib.with_cx(ctx, deco, |cx| -> Res<(Obj, FrameId)> {
            let id = ui.frames.borrow_mut().mint_frame();
            let obj = cx.spawn("window_frame", FrameHandle { ui: Rc::clone(ui), id })?;
            cx.set_parent(obj, deco)?;
            let actions = cx.spawn("window_actions", ActionsHandle { ui: Rc::clone(ui), id })?;
            cx.set_parent(actions, obj)?;
            let mut world = ui.frames.borrow_mut();
            world.frames.insert(
                id,
                FrameRec {
                    node,
                    kind,
                    obj,
                    actions,
                    alive: true,
                    outlets: std::collections::HashMap::new(),
                    closing: None,
                    now: None,
                    told: Inset::default(),
                },
            );
            world.by_node.insert(node, id);
            Ok((obj, id))
        })
    });
    let (obj, id) = match made.and_then(|made| made) {
        Ok(made) => made,
        Err(fail) => {
            ui.warn(&format!("a window frame could not be made: {}", fail.message));
            return;
        }
    };
    let answer: Result<bool, String> = match fill {
        Fill::Bare => return,
        Fill::Stock => with_lib(|lib| lib.with_cx(ctx, obj, |cx| titlebar::fill(ui, cx, id, None)))
            .and_then(|done| done)
            .map_err(|fail| fail.message),
        Fill::Closure(closure) => with_lib(|lib| lib.with_cx(ctx, obj, |cx| cx.handout(obj)))
            .and_then(|frame| frame)
            .map_err(|fail| fail.message)
            .and_then(|frame| {
                closure
                    .call(0, &[Win::of(ui, node).record(), frame])
                    .map(|answer| !matches!(answer, Val::Bool(false)))
                    .map_err(|fail| fail.message)
            }),
    };
    match answer {
        Ok(true) => {}
        Ok(false) => {
            destroy(ui, id);
            return;
        }
        Err(message) => {
            ui.warn(&format!("Decorations.onAdded threw: {message}"));
            destroy(ui, id);
            return;
        }
    }
    let handles = ui.frames.borrow().deco.resize_handles;
    if kind == FrameKind::Server && handles > 0.0 {
        let made =
            with_lib(|lib| lib.with_cx(ctx, obj, |cx| resize_handles(ui, cx, id, handles))).and_then(|made| made);
        if let Err(fail) = made {
            ui.warn(&format!(
                "the resize handles of a window frame could not be made: {}",
                fail.message
            ));
        }
    }
}

/// Makes and drops frames to match the windows: a frame of a window that left goes, and a window whose kind of frame is not the one it
/// was last given has it made again.
pub(crate) fn reconcile(ui: &Rc<Ui>) {
    let Ok(windows) = ui.g.windows() else {
        return;
    };
    let windows: Vec<u64> = windows.iter().map(|window| window.id().0).collect();
    let gone: Vec<FrameId> = ui
        .frames
        .borrow()
        .frames
        .iter()
        .filter(|(_, rec)| !windows.contains(&rec.node))
        .map(|(id, _)| *id)
        .collect();
    for frame in gone {
        destroy(ui, frame);
    }
    ui.frames.borrow_mut().decided.retain(|node, _| windows.contains(node));
    for node in windows {
        let kind = FrameKind::of_decoration(&Win::of(ui, node).decoration());
        let (decided, have) = {
            let world = ui.frames.borrow();
            (world.decided.get(&node).copied(), world.by_node.get(&node).copied())
        };
        if decided == Some(kind) {
            continue;
        }
        if let Some(frame) = have {
            destroy(ui, frame);
        }
        ui.frames.borrow_mut().decided.insert(node, kind);
        build(ui, node, kind);
    }
}

/// Takes the frame of `node` away and has it made again by the next turn.
pub(crate) fn reframe(ui: &Rc<Ui>, node: u64) {
    let have = {
        let mut world = ui.frames.borrow_mut();
        world.decided.remove(&node);
        world.by_node.get(&node).copied()
    };
    if let Some(frame) = have {
        destroy(ui, frame);
    }
    ui.schedule();
}

/// Tells the library the room each frame reserves, for every window whose reservation changed since it was last told.
fn send(ui: &Rc<Ui>) {
    let mut windows: Vec<u64> = {
        let world = ui.frames.borrow();
        world.by_node.keys().chain(world.sent.keys()).copied().collect()
    };
    windows.sort_unstable();
    windows.dedup();
    for node in windows {
        let frame = ui.frames.borrow().by_node.get(&node).copied();
        let wanted = match frame {
            Some(frame) => match ui.frame_inset(frame, true) {
                Ok(inset) => inset,
                Err(message) => {
                    ui.warn(&format!("a window frame's inset threw: {message}"));
                    continue;
                }
            },
            None => Inset::default(),
        };
        let was = ui.frames.borrow().sent.get(&node).copied().unwrap_or_default();
        if wanted == was {
            continue;
        }
        if !nodes::alive(&ui.g, node) {
            ui.frames.borrow_mut().sent.remove(&node);
            continue;
        }
        if write_inset(ui, node, &wanted) {
            ui.frames.borrow_mut().sent.insert(node, wanted);
            if let Some(frame) = frame {
                announce(ui, frame, &wanted);
            }
        }
    }
}

/// The end of the toolkit's frame: the room each frame reserves is told to the library.
pub(crate) fn send_insets(ui: &Rc<Ui>) {
    send(ui);
}

/// Gives the library the frame inset of a window, keeping what else it holds of the client. A window the library does not know yet is
/// asked again next turn.
fn write_inset(ui: &Rc<Ui>, node: u64, inset: &Inset) -> bool {
    crate::table::mutate(ui.ctx, |st| {
        let mut state = st
            .client_state_of(node)
            .ok_or_else(|| crate::error::Fail::stale(format!("node {node} is not a client")))?;
        state.inset_top = inset.top;
        state.inset_right = inset.right;
        state.inset_bottom = inset.bottom;
        state.inset_left = inset.left;
        st.set_client_state(node, &state)
    })
    .is_ok()
}

/// Tells the frame's `room_changed` listeners what the frame reserves now.
fn announce(ui: &Rc<Ui>, frame: FrameId, inset: &Inset) {
    let obj = {
        let mut world = ui.frames.borrow_mut();
        let Some(rec) = world.frames.get_mut(&frame) else {
            return;
        };
        if rec.told == *inset {
            return;
        }
        rec.told = *inset;
        rec.obj
    };
    ui.announce(obj, |cx| cx.emit("room_changed", vec![inset_val(inset)]));
}

/// A `frame_inset` value.
pub(crate) fn inset_val(inset: &Inset) -> Val {
    record(vec![
        Val::Double(inset.top),
        Val::Double(inset.right),
        Val::Double(inset.bottom),
        Val::Double(inset.left),
    ])
}
