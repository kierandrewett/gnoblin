//! The lock screen: a clock and a password entry over the screen while the session is locked.
//!
//! It reads the lock node's `state` and sends passwords to the compositor with the core's `session_lock`. Each turn it takes the compositor's
//! answer to a password check, puts the screen up or takes it down by the lock's state, and keeps the keyboard on its entry; after the toolkit
//! has drawn the frame it renews the compositor's lease, but only when the screen is whole.
//!
//! The screen is modal: while it is up the keyboard is on its entry. Something else may take it (a window that arrived under the lock and was
//! given the focus by the policy, or a script), and the keys then go nowhere the screen can read. The screen takes the keyboard back, and a
//! window that took it is the one that has it when the screen comes down, as it would have had it without the lock.

use std::rc::Rc;

use gnoblin_core::Subscription;
use gnoblin_core::objects::{LockAnswer, SessionLock};
use gnoblin_object::server::Cx;
use gnoblin_object::typed::Listener;
use gnoblin_object::{Closure, ErrorCode, Fail, Obj, Res, Val};

use super::entry;
use super::style::LockStyle;
use crate::object::with_lib;
use crate::ui::clock;
use crate::ui::tree::{core_fail, find};
use crate::ui::widget::{Cid, ClockPart, Special, Ui, WidgetKind, en};

/// How often the screen asks for a turn while the session is locked, in milliseconds: the compositor's lease on it lasts two seconds and the
/// core sends a renewal at most twice a second.
const TICK_MS: u64 = 100;

/// What the screen made while it is up.
pub(crate) struct ScreenParts {
    scrim: Cid,
    clock: Cid,
    entry: Cid,
    message: Cid,
    /// The node that had the keyboard before the screen took it, given back when the screen comes down.
    before: Option<u64>,
}

/// What the screen keeps for one generation.
#[derive(Default)]
pub(crate) struct LockWorld {
    /// The `lock_screen` object.
    pub(crate) obj: Obj,
    pub(crate) style: LockStyle,
    pub(crate) now: Option<Closure>,
    pub(crate) started: bool,
    pub(crate) shown: Option<ScreenParts>,
    /// The token of the check in flight. A second Return while it runs is not a second attempt.
    pending: Option<u64>,
    /// A password was accepted: nothing is drawn or heard until the state itself says `not-locked`.
    accepted: bool,
    /// The screen is down for good, until `show`.
    pub(crate) released: bool,
    /// How many steps of the update pass of the screen's canvases had failed when the lease was last considered.
    failures_seen: u32,
    /// The answers to password checks that arrived and were not looked at yet.
    answers: Vec<LockAnswer>,
    service: Option<Rc<SessionLock>>,
    heard: Option<Listener>,
    listening: Vec<Subscription>,
}

/// Puts the screen to work for the generation: it follows the lock's state from the next turn.
///
/// # Errors
///
/// A screen is already at work, the core serves no session lock, or a subscription is refused.
pub(crate) fn begin(ui: &Rc<Ui>, style: LockStyle, obj: Obj) -> Res<()> {
    if ui.lock.borrow().started {
        return Err(Fail::refused(
            ErrorCode::Busy,
            "there is one lock screen for each generation",
        ));
    }
    let objects = ui.g.objects().map_err(core_fail)?;
    let service = Rc::new(SessionLock::get(&objects)?);
    let ctx = ui.ctx;
    let heard = service.on_answered(move |answer| {
        if let Some(ui) = find(ctx) {
            ui.lock.borrow_mut().answers.push(answer);
            ui.schedule();
        }
    })?;
    let wake = move || {
        if let Some(ui) = find(ctx) {
            ui.schedule();
        }
    };
    let listening = vec![
        ui.g.on_class_property_changed("lock", "state", move |_, _| wake())
            .map_err(core_fail)?,
        ui.g.on_class_property_changed("seat", "focus", move |_, _| wake())
            .map_err(core_fail)?,
        ui.g.on_node_created("lock", move |_| wake()).map_err(core_fail)?,
    ];
    {
        let mut world = ui.lock.borrow_mut();
        world.obj = obj;
        world.style = style;
        world.started = true;
        world.released = false;
        world.service = Some(service);
        world.heard = Some(heard);
        world.listening = listening;
    }
    ui.schedule();
    Ok(())
}

/// The lock screen's object ended: the screen is down for good and nothing is heard. Its canvases go with the object. The lock is no longer
/// engaged, so the compositor's own card takes the password.
pub(crate) fn end(ui: &Ui) {
    let (service, was_shown) = {
        let mut world = ui.lock.borrow_mut();
        let was_shown = world.shown.take().is_some();
        world.started = false;
        world.released = true;
        world.pending = None;
        world.answers.clear();
        world.heard = None;
        world.listening.clear();
        (world.service.take(), was_shown)
    };
    if was_shown && let Some(service) = service {
        // A generation that is ending has no compositor to tell.
        let _ = service.engage(false);
    }
}

/// Takes the screen down at once and keeps it down.
pub(crate) fn release(ui: &Rc<Ui>) {
    ui.lock.borrow_mut().released = true;
    hide(ui);
}

/// Lets the screen come up again.
pub(crate) fn resume(ui: &Rc<Ui>) {
    ui.lock.borrow_mut().released = false;
    ui.schedule();
}

fn service(ui: &Ui) -> Option<Rc<SessionLock>> {
    ui.lock.borrow().service.clone()
}

/// The screen's clock, in milliseconds since the Unix epoch: the shell's, or the system's.
fn now(ui: &Ui) -> Option<Closure> {
    ui.lock.borrow().now.clone()
}

/// One turn of the screen, at the start of the toolkit's frame: it takes the compositor's answer to a password check, puts the screen up or
/// takes it down by the lock's state, and ignores every key once the session is unlocked.
pub(crate) fn turn(ui: &Rc<Ui>) {
    {
        let world = ui.lock.borrow();
        if !world.started || world.released {
            return;
        }
    }
    let answers = std::mem::take(&mut ui.lock.borrow_mut().answers);
    for answer in answers {
        let waiting = ui.lock.borrow().pending;
        if waiting == Some(answer.token) {
            settle(ui, &answer);
        }
    }
    if ui.lock_state() == "not-locked" {
        ui.lock.borrow_mut().accepted = false;
        hide(ui);
    } else if ui.lock.borrow().shown.is_none() {
        if !ui.lock.borrow().accepted {
            show(ui);
        }
    } else {
        hold_keyboard(ui);
        follow_clock(ui);
    }
}

/// Renews the compositor's lease on the screen. It runs after the toolkit has drawn the frame. A turn is healthy, and renews, when the screen
/// is up, every canvas of it is still in the tree, and no refresh, arrange or draw of one of them failed in the turn (a paint that throws, a
/// write the compositor refused). A screen that cannot draw is not one a password can be typed into, so a turn that failed renews nothing and
/// the compositor's own card takes the password when the lease lapses; the first healthy turn after it renews again.
pub(crate) fn after_update(ui: &Rc<Ui>) {
    let (scrim, seen) = {
        let world = ui.lock.borrow();
        match &world.shown {
            Some(shown) if world.started && !world.released => (shown.scrim, world.failures_seen),
            _ => return,
        }
    };
    let failures = failures_of(ui, scrim);
    let drew = ui.alive(scrim) && failures <= seen;
    ui.lock.borrow_mut().failures_seen = failures;
    if drew && let Some(service) = service(ui) {
        // A refusal is made up for by the next beat.
        let _ = service.heartbeat();
    }
}

/// How many steps of the update pass of the canvases under `cid` have failed.
fn failures_of(ui: &Ui, cid: Cid) -> u32 {
    let w = ui.w.borrow();
    w.subtree(cid)
        .into_iter()
        .filter_map(|one| w.get(one).map(|widget| widget.failures))
        .sum()
}

/// How long until the screen next needs a turn: while the session is locked it ticks, to renew the lease and read the clock.
pub(crate) fn wake_in(ui: &Rc<Ui>) -> Option<u64> {
    let (started, released, shown) = {
        let world = ui.lock.borrow();
        (world.started, world.released, world.shown.is_some())
    };
    if !started || released {
        return None;
    }
    (shown || ui.lock_state() != "not-locked").then_some(TICK_MS)
}

/// The size of the screen the lock covers and the lock node, once the tree has both.
fn surface(ui: &Ui) -> Option<((f64, f64), u64)> {
    Some((ui.primary_screen()?, ui.lock_node()?))
}

/// Puts the screen up. The tree has no screen or no lock node yet: it waits.
fn show(ui: &Rc<Ui>) {
    let Some(((width, height), lock)) = surface(ui) else {
        return;
    };
    let (style, owner) = {
        let world = ui.lock.borrow();
        (world.style.clone(), world.obj)
    };
    let clock_now = now(ui);
    let ctx = ui.ctx;
    let built = with_lib(|lib| {
        lib.with_cx(ctx, owner, |cx| {
            build(ui, cx, owner, &style, (width, height), lock, clock_now)
        })
    })
    .and_then(|built| built);
    let (scrim, clock, entry, message) = match built {
        Ok(made) => made,
        Err(fail) => {
            ui.warn(&format!("LockScreen.show threw: {fail}"));
            return;
        }
    };
    let before = ui.seat_focus();
    let failures = failures_of(ui, scrim);
    {
        let mut world = ui.lock.borrow_mut();
        world.shown = Some(ScreenParts {
            scrim,
            clock,
            entry,
            message,
            before,
        });
        world.failures_seen = failures;
    }
    // The compositor refused the focus (the tree has no seat yet): the screen is up all the same.
    let _ = crate::shell::focus_canvas(ui, entry);
    if let Some(service) = service(ui)
        && let Err(fail) = service.engage(true)
    {
        ui.warn(&format!("LockScreen.show threw: {fail}"));
    }
    ui.schedule();
}

/// Makes the canvases of the screen under the lock node: the scrim, the card, the clock, the entry and the message.
fn build(
    ui: &Rc<Ui>,
    cx: &mut Cx<'_>,
    owner: Obj,
    style: &LockStyle,
    (width, height): (f64, f64),
    lock: u64,
    clock_now: Option<Closure>,
) -> Res<(Cid, Cid, Cid, Cid)> {
    let (scrim_obj, scrim) = ui.spawn(cx, WidgetKind::Box, owner)?;
    ui.tweak(scrim, |widget| {
        widget.width = Some(width);
        widget.height = Some(height);
        widget.b.fill = style.scrim_fill.clone();
    });
    ui.mount(scrim, lock)?;
    let (card_obj, card) = ui.spawn(cx, WidgetKind::Box, scrim_obj)?;
    ui.tweak(card, |widget| {
        widget.x = ((width - style.card_width) / 2.0).floor();
        widget.y = (height * style.card_top).floor();
        widget.width = Some(style.card_width);
        widget.height = Some(style.card_height);
        widget.b.radius = style.card_radius;
        widget.b.fill = style.card_fill.clone();
        widget.b.padding = style.card_padding;
        widget.b.gap = style.card_gap;
        widget.b.direction = en::DIRECTION_COLUMN;
    });
    ui.link(scrim, card)?;
    let (_, time) = ui.spawn_named(cx, "clock", WidgetKind::Clock, card_obj)?;
    ui.tweak(time, |widget| {
        widget.t.size = style.clock_size;
        widget.t.color = style.clock_color.clone();
        widget.sp = Special::Clock(ClockPart {
            format: "%H:%M".to_owned(),
            now: clock_now,
        });
    });
    clock::set_text(ui, time).map_err(Fail::err)?;
    ui.link(card, time)?;
    let (_, password) = ui.spawn_named(cx, "password_entry", WidgetKind::Box, card_obj)?;
    entry::init(ui, password, style, style.card_width - 2.0 * style.card_padding, true);
    ui.link(card, password)?;
    let (_, message) = ui.spawn(cx, WidgetKind::Text, card_obj)?;
    ui.tweak(message, |widget| {
        widget.t.text = style.hint.clone();
        widget.t.size = style.message_size;
        widget.t.color = style.hint_color.clone();
    });
    ui.link(card, message)?;
    Ok((scrim, time, password, message))
}

/// The clock of the screen follows the shell's `now`.
fn follow_clock(ui: &Ui) {
    let (clock, source) = {
        let world = ui.lock.borrow();
        match &world.shown {
            Some(shown) => (shown.clock, world.now.clone()),
            None => return,
        }
    };
    ui.tweak(clock, |widget| {
        if let Special::Clock(part) = &mut widget.sp {
            part.now = source;
        }
    });
}

/// While the screen is up the keyboard is on its entry. Whoever took it is the one the screen gives it back to.
fn hold_keyboard(ui: &Rc<Ui>) {
    let Some(entry) = ui.lock.borrow().shown.as_ref().map(|shown| shown.entry) else {
        return;
    };
    let held = ui.seat_focus();
    let node = ui.read(entry, |widget| widget.node);
    if held == node {
        return;
    }
    if let Some(held) = held
        && let Some(shown) = ui.lock.borrow_mut().shown.as_mut()
    {
        shown.before = Some(held);
    }
    // The compositor refused the focus (the tree has no seat): the compositor's card takes the keys.
    let _ = crate::shell::focus_canvas(ui, entry);
}

/// The password the user typed goes to the compositor to be checked.
pub(crate) fn submit(ui: &Rc<Ui>, password: &str) {
    {
        let world = ui.lock.borrow();
        if world.shown.is_none() || world.pending.is_some() || world.accepted {
            // A Return pressed while a check runs is not a second attempt.
            return;
        }
    }
    let Some(service) = service(ui) else {
        return;
    };
    let (failed, checking, error_color, hint_color, owner) = {
        let world = ui.lock.borrow();
        (
            world.style.failed.clone(),
            world.style.checking.clone(),
            world.style.error_color.clone(),
            world.style.hint_color.clone(),
            world.obj,
        )
    };
    match service.authenticate(password) {
        Ok(token) => {
            ui.lock.borrow_mut().pending = Some(token);
            say(ui, &checking, &hint_color);
            ui.announce(owner, |cx| cx.emit("submitted", vec![Val::Uint(token)]));
        }
        Err(_) => say(ui, &failed, &error_color),
    }
}

/// The compositor answered the check in flight.
fn settle(ui: &Rc<Ui>, answer: &LockAnswer) {
    ui.lock.borrow_mut().pending = None;
    if answer.result == "ok" && answer.error.is_empty() {
        // The compositor has unlocked the session. The screen comes down now; the state follows.
        ui.lock.borrow_mut().accepted = true;
        hide(ui);
        return;
    }
    let (text, color) = {
        let style = ui.lock.borrow().style.clone();
        (feedback(&style, answer), style.error_color)
    };
    say(ui, &text, &color);
}

/// What the screen says about an answer that did not unlock.
fn feedback(style: &LockStyle, answer: &LockAnswer) -> String {
    match answer.error.as_str() {
        "rate-limited" => {
            let wait = answer.result.parse::<f64>().unwrap_or(0.0);
            let wait = if wait.is_finite() { wait } else { 0.0 };
            style
                .rate_limited
                .replace("{seconds}", &format!("{}", (wait / 1000.0).ceil()))
        }
        "denied" => style.denied.clone(),
        "busy" => style.busy.clone(),
        _ => style.failed.clone(),
    }
}

/// Changes what the message says.
fn say(ui: &Ui, text: &str, color: &str) {
    let Some(message) = ui.lock.borrow().shown.as_ref().map(|shown| shown.message) else {
        return;
    };
    ui.tweak(message, |widget| {
        widget.t.text = text.to_owned();
        widget.t.color = color.to_owned();
    });
}

/// Takes the screen down: its entry forgets what it held and stops taking keys, the keyboard goes back to the window that had it if the
/// screen still held it, and the lock is no longer engaged.
fn hide(ui: &Rc<Ui>) {
    let Some(shown) = ui.lock.borrow_mut().shown.take() else {
        return;
    };
    ui.lock.borrow_mut().pending = None;
    entry::change(ui, shown.entry, |part| {
        part.enabled = false;
        part.on_submit = None;
        part.screen = false;
    });
    entry::clear(ui, shown.entry);
    // The keyboard goes back to the window that had it only if the screen still held it: a menu or another window that took the focus
    // while the screen was going down keeps it.
    let held = ui.read(shown.entry, |widget| widget.node) == ui.seat_focus();
    ui.kill(shown.scrim);
    if held && let Some(before) = shown.before {
        // The window that had the keyboard is gone: nothing to give it back to.
        let _ = ui.focus_node(before);
    }
    if let Some(service) = service(ui) {
        let _ = service.engage(false);
    }
}

/// Whether the screen is up.
pub(crate) fn is_shown(ui: &Ui) -> bool {
    ui.lock.borrow().shown.is_some()
}
