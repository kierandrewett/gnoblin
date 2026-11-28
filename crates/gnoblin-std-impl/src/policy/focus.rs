//! `focus_policy`: what the desktop does with focus. Click, sloppy and follow-mouse focus, whether a new window takes focus, raise on click
//! and auto raise.
//!
//! The decisions are pure functions of the settings and one event (`focus_decision` and its predicates), so they need no desktop. The
//! wiring turns what the compositor says of a window (it opened, the pointer entered, moved over, left or pressed it) into those events,
//! does what the decision says, and keeps the two timers: a pointer that rests, and a delayed raise. The settings are the seat's
//! (`focus_mode`, `focus_new_windows`, `raise_on_click`, `auto_raise`, `auto_raise_delay`, `focus_change_on_pointer_rest`) and are read on
//! every event; the seat's words for the modes translate: `click` is click, `hover` is sloppy and `hover-strict` is follow-mouse.
//!
//! Nothing here runs before `focus_policy.start`. The library starts it when it becomes a generation's desktop (`events::open`), because a shell
//! that holds the tree is the one that decides focus and a desktop with no focused window cannot receive a key; a shell with a focus policy of its
//! own calls `focus_policy.stop`. The ids of an enum's values
//! are their places in the catalog, counted from 1.

use std::rc::Rc;

use gnoblin_core::host::Subscription;
use gnoblin_object::server::Builder;
use gnoblin_object::{Closure, Fail, Res, Val};

use super::stack;
use crate::nodes;
use crate::object::record::{enumeration, look, record};
use crate::ui::{Ui, ui_of};

/// How long a pointer must rest over a window before `change_on_pointer_rest` focuses it, in milliseconds.
const REST_DELAY: f64 = 200.0;

const MODE_CLICK: u32 = 1;
const MODE_SLOPPY: u32 = 2;
const MODE_FOLLOW_MOUSE: u32 = 3;
const NEW_WINDOWS_ALLOW: u32 = 1;
const NEW_WINDOWS_PREVENT: u32 = 2;
const EVENT_ADDED: u32 = 1;
const EVENT_ENTER: u32 = 2;
const EVENT_MOTION: u32 = 3;
const EVENT_LEAVE: u32 = 4;
const EVENT_PRESS: u32 = 5;

/// The focus settings in force.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    mode: u32,
    new_windows: u32,
    raise_on_click: bool,
    auto_raise: bool,
    auto_raise_delay: f64,
    change_on_pointer_rest: bool,
}

impl Default for Settings {
    /// The documented defaults (`docs/config/configure/window_management.md`).
    fn default() -> Settings {
        Settings {
            mode: MODE_CLICK,
            new_windows: NEW_WINDOWS_PREVENT,
            raise_on_click: true,
            auto_raise: false,
            auto_raise_delay: 500.0,
            change_on_pointer_rest: false,
        }
    }
}

fn flag(fields: &[Val], at: usize, default: bool) -> Res<bool> {
    match fields.get(at) {
        None | Some(Val::None) => Ok(default),
        Some(value) => value.bool(),
    }
}

impl Settings {
    /// The settings a shell gave as a record; a field it left out is the documented default.
    fn read(value: &Val) -> Res<Settings> {
        let fields = value.record()?;
        let defaults = Settings::default();
        let word = |at: usize, default: u32| match fields.get(at) {
            None | Some(Val::None) => Ok(default),
            Some(value) => value.enum_value(),
        };
        Ok(Settings {
            mode: word(0, defaults.mode)?,
            new_windows: word(1, defaults.new_windows)?,
            raise_on_click: flag(fields, 2, defaults.raise_on_click)?,
            auto_raise: flag(fields, 3, defaults.auto_raise)?,
            auto_raise_delay: match fields.get(4) {
                None | Some(Val::None) => defaults.auto_raise_delay,
                Some(value) => value.double()?,
            },
            change_on_pointer_rest: flag(fields, 5, defaults.change_on_pointer_rest)?,
        })
    }

    /// The settings as a record.
    fn to_val(self) -> Val {
        record(vec![
            enumeration(self.mode),
            enumeration(self.new_windows),
            Val::Bool(self.raise_on_click),
            Val::Bool(self.auto_raise),
            Val::Double(self.auto_raise_delay),
            Val::Bool(self.change_on_pointer_rest),
        ])
    }

    /// The settings the seat publishes. A tree with no seat, and a key the seat does not publish, have the documented defaults.
    ///
    /// # Errors
    ///
    /// The seat's `focus_mode` is a word the library does not know.
    pub(crate) fn of_seat(ui: &Ui) -> Result<Settings, String> {
        let defaults = Settings::default();
        let Some(seat) = nodes::seat(&ui.g) else {
            return Ok(defaults);
        };
        let mode = match seat.get_text("focus_mode").as_deref() {
            Err(_) | Ok("click") => MODE_CLICK,
            Ok("hover") => MODE_SLOPPY,
            Ok("hover-strict") => MODE_FOLLOW_MOUSE,
            Ok(other) => {
                return Err(format!(
                    "the seat's focus_mode must be click, hover or hover-strict, got {other}"
                ));
            }
        };
        let new_windows = match seat.get_text("focus_new_windows").as_deref() {
            Ok("allow") => NEW_WINDOWS_ALLOW,
            _ => defaults.new_windows,
        };
        Ok(Settings {
            mode,
            new_windows,
            raise_on_click: seat.get_bool("raise_on_click").unwrap_or(defaults.raise_on_click),
            auto_raise: seat.get_bool("auto_raise").unwrap_or(defaults.auto_raise),
            auto_raise_delay: seat.get_f64("auto_raise_delay").unwrap_or(defaults.auto_raise_delay),
            change_on_pointer_rest: seat
                .get_bool("focus_change_on_pointer_rest")
                .unwrap_or(defaults.change_on_pointer_rest),
        })
    }

    fn opens_focused(&self, from_launcher: bool) -> bool {
        !from_launcher || self.new_windows == NEW_WINDOWS_ALLOW
    }

    fn hover_focus(&self, hovering: bool, entered: bool) -> bool {
        if self.mode == MODE_CLICK || !hovering {
            return false;
        }
        if self.mode == MODE_FOLLOW_MOUSE {
            return entered;
        }
        true
    }

    fn rest_focus(&self) -> bool {
        self.change_on_pointer_rest && self.mode != MODE_CLICK
    }

    fn leave_clears(&self) -> bool {
        self.mode == MODE_FOLLOW_MOUSE
    }
}

/// One thing that happened to a window.
#[derive(Clone, Copy, Debug)]
enum Occasion {
    Added { from_launcher: bool },
    Enter { held: bool },
    Motion { held: bool },
    Leave { focused: bool, held_here: bool },
    Press,
}

/// What to do about an event.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FocusVerdict {
    focus: bool,
    clear: bool,
    raise: bool,
    focus_after: Option<f64>,
    raise_after: Option<f64>,
}

impl FocusVerdict {
    fn to_val(self) -> Val {
        record(vec![
            Val::Bool(self.focus),
            Val::Bool(self.clear),
            Val::Bool(self.raise),
            self.focus_after.map_or(Val::None, Val::Double),
            self.raise_after.map_or(Val::None, Val::Double),
        ])
    }
}

fn decision(settings: &Settings, event: Occasion) -> FocusVerdict {
    match event {
        Occasion::Added { from_launcher } => FocusVerdict {
            focus: settings.opens_focused(from_launcher),
            ..FocusVerdict::default()
        },
        Occasion::Press => FocusVerdict {
            focus: true,
            raise: settings.raise_on_click,
            ..FocusVerdict::default()
        },
        Occasion::Enter { held } | Occasion::Motion { held } => {
            if held {
                return FocusVerdict::default();
            }
            let entered = matches!(event, Occasion::Enter { .. });
            let rests = settings.rest_focus();
            FocusVerdict {
                focus: !rests && settings.hover_focus(true, entered),
                focus_after: rests.then_some(REST_DELAY),
                // Auto raise starts on entry; motion inside the window leaves a pending raise alone.
                raise_after: (entered && settings.auto_raise).then_some(settings.auto_raise_delay),
                ..FocusVerdict::default()
            }
        }
        Occasion::Leave { focused, held_here } => FocusVerdict {
            clear: !held_here && focused && settings.leave_clears(),
            ..FocusVerdict::default()
        },
    }
}

/// The event a shell gave as a record.
fn event_of(value: &Val) -> Res<Occasion> {
    let fields = value.record()?;
    let kind = fields
        .first()
        .ok_or_else(|| Fail::type_error("a focus event needs its kind"))?
        .enum_value()?;
    Ok(match kind {
        EVENT_ADDED => Occasion::Added {
            from_launcher: flag(fields, 1, false)?,
        },
        EVENT_PRESS => Occasion::Press,
        EVENT_ENTER => Occasion::Enter {
            held: flag(fields, 2, false)?,
        },
        EVENT_MOTION => Occasion::Motion {
            held: flag(fields, 2, false)?,
        },
        EVENT_LEAVE => Occasion::Leave {
            held_here: flag(fields, 3, false)?,
            focused: flag(fields, 4, false)?,
        },
        other => return Err(Fail::type_error(format!("unknown focus event kind {other}"))),
    })
}

// ---- the wiring ---------------------------------------------------------------------------------------------------------------------

/// What a window waits for: a rest that focuses it, and a delayed raise. Each is the time it comes due, on the policy's clock.
#[derive(Clone, Copy, Debug)]
struct FocusWaiting {
    focus_at: Option<f64>,
    raise_at: Option<f64>,
}

/// What the focus policy keeps for one generation.
#[derive(Default)]
pub(crate) struct Focus {
    started: bool,
    now: Option<Closure>,
    /// The windows that wait, in the order they began to.
    pending: Vec<(u64, FocusWaiting)>,
    /// The window whose button is held, so focus and rest wait for the release.
    pressed: Option<u64>,
    listening: Vec<Subscription>,
}

/// The policy's clock, in milliseconds: the shell's, or the core's (see `Ui::core_now`).
fn clock(ui: &Ui) -> f64 {
    let source = ui.policy.borrow().focus.now.clone();
    ui.clock_ms(source.as_ref(), "the focus policy's clock")
}

/// The settings now, or none when the seat's are not ones the library knows (that is reported).
fn settings(ui: &Ui) -> Option<Settings> {
    match Settings::of_seat(ui) {
        Ok(settings) => Some(settings),
        Err(message) => {
            ui.warn(&message);
            None
        }
    }
}

/// Starts the policy, once.
pub(super) fn start(ui: &Rc<Ui>) -> Res<()> {
    if ui.policy.borrow().focus.started {
        return Ok(());
    }
    // The library holds the windows that were there before the shell started, so a window that comes later is told from them.
    look(ui.ctx, |_| Ok(()))?;
    let weak = Rc::downgrade(ui);
    let created =
        ui.g.on_client_created({
            let weak = weak.clone();
            move |window| {
                if let Some(ui) = weak.upgrade() {
                    added(&ui, window.id().0);
                }
            }
        })
        .map_err(crate::ui::tree::core_fail)?;
    let gone =
        ui.g.on_client_destroyed(move |window| {
            if let Some(ui) = weak.upgrade() {
                closed(&ui, window.id().0);
            }
        })
        .map_err(crate::ui::tree::core_fail)?;
    let mut policy = ui.policy.borrow_mut();
    policy.focus.started = true;
    policy.focus.listening = vec![created, gone];
    drop(policy);
    ui.schedule();
    Ok(())
}

/// Stops the policy and forgets what it waited for.
pub(crate) fn stop(ui: &Rc<Ui>) {
    let listening = {
        let mut policy = ui.policy.borrow_mut();
        policy.focus.started = false;
        policy.focus.pending.clear();
        policy.focus.pressed = None;
        std::mem::take(&mut policy.focus.listening)
    };
    for subscription in listening {
        let _ = subscription.unsubscribe();
    }
}

/// Whether the policy runs.
pub(crate) fn started(ui: &Ui) -> bool {
    ui.policy.borrow().focus.started
}

/// A new window: it takes focus as the settings say, unless the shell found it already open.
fn added(ui: &Rc<Ui>, client: u64) {
    if !started(ui) {
        return;
    }
    let arrival = look(ui.ctx, |st| Ok(!st.rec.get(&client).is_some_and(|rec| rec.preexisting))).unwrap_or(true);
    if !arrival {
        return;
    }
    let Some(settings) = settings(ui) else {
        return;
    };
    let from_launcher = nodes::flag(&ui.g, client, "from_launcher");
    run(ui, client, decision(&settings, Occasion::Added { from_launcher }));
}

/// A window closed: what it waited for is cancelled.
fn closed(ui: &Rc<Ui>, client: u64) {
    let mut policy = ui.policy.borrow_mut();
    policy.focus.pending.retain(|(held, _)| *held != client);
    if policy.focus.pressed == Some(client) {
        policy.focus.pressed = None;
    }
}

/// The pointer's edges on a window, as the compositor reports them.
pub(crate) fn on_pointer(ui: &Rc<Ui>, client: u64, kind: &str) {
    if !started(ui) || !stack::alive(ui, client) {
        return;
    }
    let pressed = ui.policy.borrow().focus.pressed;
    let event = match kind {
        "enter" => Occasion::Enter {
            held: pressed.is_some(),
        },
        "motion" => Occasion::Motion {
            held: pressed.is_some(),
        },
        "leave" => {
            ui.policy.borrow_mut().focus.pending.retain(|(held, _)| *held != client);
            Occasion::Leave {
                focused: stack::focused(ui) == Some(client),
                held_here: pressed == Some(client),
            }
        }
        "press" => {
            ui.policy.borrow_mut().focus.pressed = Some(client);
            Occasion::Press
        }
        "release" => {
            ui.policy.borrow_mut().focus.pressed = None;
            return;
        }
        _ => return,
    };
    let Some(settings) = settings(ui) else {
        return;
    };
    run(ui, client, decision(&settings, event));
}

/// Does what a decision says about `client`.
fn run(ui: &Rc<Ui>, client: u64, action: FocusVerdict) {
    if !stack::alive(ui, client) {
        return;
    }
    if action.clear {
        stack::focus(ui, None);
    }
    if action.focus {
        stack::focus(ui, Some(client));
    }
    if action.raise {
        stack::raise(ui, client);
    }
    if action.focus_after.is_none() && action.raise_after.is_none() {
        return;
    }
    let now = clock(ui);
    {
        let mut policy = ui.policy.borrow_mut();
        let before = policy
            .focus
            .pending
            .iter()
            .find(|(held, _)| *held == client)
            .map(|(_, pending)| *pending);
        // A rest that starts again moves the focus time; a raise that was waiting keeps its own.
        let next = FocusWaiting {
            focus_at: match action.focus_after {
                Some(after) => Some(now + after),
                None => before.and_then(|held| held.focus_at),
            },
            raise_at: match action.raise_after {
                Some(after) => Some(now + after),
                None => before.and_then(|held| held.raise_at),
            },
        };
        match policy.focus.pending.iter_mut().find(|(held, _)| *held == client) {
            Some(slot) => slot.1 = next,
            None => policy.focus.pending.push((client, next)),
        }
    }
    ui.schedule();
}

/// The timers that are due: a rest that focuses, a delayed raise. Runs once a frame of the toolkit.
pub(crate) fn turn(ui: &Rc<Ui>) {
    if ui.policy.borrow().focus.pending.is_empty() {
        return;
    }
    let now = clock(ui);
    let pressed = ui.policy.borrow().focus.pressed;
    let list = std::mem::take(&mut ui.policy.borrow_mut().focus.pending);
    let mut keep = Vec::new();
    for (client, mut pending) in list {
        if !stack::alive(ui, client) {
            continue;
        }
        if pending.focus_at.is_some_and(|at| now >= at) {
            pending.focus_at = None;
            if pressed != Some(client) {
                stack::focus(ui, Some(client));
            }
        }
        if pending.raise_at.is_some_and(|at| now >= at) {
            pending.raise_at = None;
            if stack::focused(ui) == Some(client) {
                stack::raise(ui, client);
            }
        }
        if pending.focus_at.is_some() || pending.raise_at.is_some() {
            keep.push((client, pending));
        }
    }
    // A window the pass queued while it ran (a decision it made) is kept as well.
    let mut policy = ui.policy.borrow_mut();
    let queued = std::mem::take(&mut policy.focus.pending);
    policy.focus.pending = keep;
    for (client, pending) in queued {
        match policy.focus.pending.iter_mut().find(|(held, _)| *held == client) {
            Some(slot) => slot.1 = pending,
            None => policy.focus.pending.push((client, pending)),
        }
    }
}

/// How long until the next timer is due, in milliseconds.
pub(crate) fn wake_in(ui: &Ui) -> Option<u64> {
    let pending: Vec<FocusWaiting> = ui
        .policy
        .borrow()
        .focus
        .pending
        .iter()
        .map(|(_, pending)| *pending)
        .collect();
    if pending.is_empty() {
        return None;
    }
    let now = clock(ui);
    pending
        .iter()
        .flat_map(|pending| [pending.focus_at, pending.raise_at])
        .flatten()
        .map(|at| (at - now).max(0.0).ceil() as u64)
        .min()
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("focus_policy")
        .ctor(|_cx, _inits| Ok(()))
        .function("focus_decision", |_cx, args| {
            Ok(decision(&Settings::read(&args[0])?, event_of(&args[1])?).to_val())
        })
        .function("opens_focused", |_cx, args| {
            Ok(Val::Bool(Settings::read(&args[0])?.opens_focused(args[1].bool()?)))
        })
        .function("hover_focus", |_cx, args| {
            Ok(Val::Bool(
                Settings::read(&args[0])?.hover_focus(args[1].bool()?, args[2].bool()?),
            ))
        })
        .function("rest_focus", |_cx, args| {
            Ok(Val::Bool(Settings::read(&args[0])?.rest_focus()))
        })
        .function("leave_clears", |_cx, args| {
            Ok(Val::Bool(Settings::read(&args[0])?.leave_clears()))
        })
        .get("started", |_, cx| Ok(Val::Bool(started(&*ui_of(cx)?))))
        .get("settings", |_, cx| {
            let ui = ui_of(cx)?;
            Settings::of_seat(&ui).map(Settings::to_val).map_err(Fail::type_error)
        })
        .get("rest_delay", |_, _| Ok(Val::Double(REST_DELAY)))
        .get("now", |_, cx| {
            Ok(ui_of(cx)?
                .policy
                .borrow()
                .focus
                .now
                .clone()
                .map_or(Val::None, Val::Closure))
        })
        .set("now", |_, cx, value| {
            let ui = ui_of(cx)?;
            ui.policy.borrow_mut().focus.now = if value.is_none() {
                None
            } else {
                Some(Rc::clone(value.closure()?))
            };
            Ok(())
        })
        .method("start", |_, cx, _| {
            start(&ui_of(cx)?)?;
            Ok(Val::None)
        })
        .method("stop", |_, cx, _| {
            stop(&ui_of(cx)?);
            Ok(Val::None)
        });
}
