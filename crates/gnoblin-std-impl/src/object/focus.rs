//! `focus_policy`: the focus decisions, pure functions of the settings and one event (`focusDecision` and its predicates in the
//! TypeScript library). The ids of an enum's values are their places in the catalog, counted from 1.

use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Res, Val};

/// How long a pointer must rest over a window before `change_on_pointer_rest` focuses it, in milliseconds.
const REST_DELAY: f64 = 200.0;

const MODE_CLICK: u32 = 1;
const MODE_FOLLOW_MOUSE: u32 = 3;
const NEW_WINDOWS_ALLOW: u32 = 1;
const NEW_WINDOWS_PREVENT: u32 = 2;
const EVENT_ADDED: u32 = 1;
const EVENT_ENTER: u32 = 2;
const EVENT_MOTION: u32 = 3;
const EVENT_LEAVE: u32 = 4;
const EVENT_PRESS: u32 = 5;

/// The settings in force; a field the caller leaves out is the documented default.
struct Settings {
    mode: u32,
    new_windows: u32,
    raise_on_click: bool,
    auto_raise: bool,
    auto_raise_delay: f64,
    change_on_pointer_rest: bool,
}

fn flag(fields: &[Val], at: usize, default: bool) -> Res<bool> {
    match fields.get(at) {
        None | Some(Val::None) => Ok(default),
        Some(value) => value.bool(),
    }
}

impl Settings {
    fn read(value: &Val) -> Res<Settings> {
        let fields = value.record()?;
        let word = |at: usize, default: u32| match fields.get(at) {
            None | Some(Val::None) => Ok(default),
            Some(value) => value.enum_value(),
        };
        Ok(Settings {
            mode: word(0, MODE_CLICK)?,
            new_windows: word(1, NEW_WINDOWS_PREVENT)?,
            raise_on_click: flag(fields, 2, true)?,
            auto_raise: flag(fields, 3, false)?,
            auto_raise_delay: match fields.get(4) {
                None | Some(Val::None) => 500.0,
                Some(value) => value.double()?,
            },
            change_on_pointer_rest: flag(fields, 5, false)?,
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

/// What to do about an event: `focus`, `clear`, `raise`, then the two delays.
fn action(focus: bool, clear: bool, raise: bool, focus_after: Option<f64>, raise_after: Option<f64>) -> Val {
    Val::Record {
        class: 0,
        fields: vec![
            Val::Bool(focus),
            Val::Bool(clear),
            Val::Bool(raise),
            focus_after.map_or(Val::None, Val::Double),
            raise_after.map_or(Val::None, Val::Double),
        ],
    }
}

fn decision(settings: &Settings, event: &Val) -> Res<Val> {
    let fields = event.record()?;
    let kind = fields
        .first()
        .ok_or_else(|| Fail::type_error("a focus event needs its kind"))?
        .enum_value()?;
    Ok(match kind {
        EVENT_ADDED => action(
            settings.opens_focused(flag(fields, 1, false)?),
            false,
            false,
            None,
            None,
        ),
        EVENT_PRESS => action(true, false, settings.raise_on_click, None, None),
        EVENT_ENTER | EVENT_MOTION => {
            if flag(fields, 2, false)? {
                return Ok(action(false, false, false, None, None));
            }
            let entered = kind == EVENT_ENTER;
            let rests = settings.rest_focus();
            action(
                !rests && settings.hover_focus(true, entered),
                false,
                false,
                rests.then_some(REST_DELAY),
                (entered && settings.auto_raise).then_some(settings.auto_raise_delay),
            )
        }
        EVENT_LEAVE => action(
            false,
            !flag(fields, 3, false)? && flag(fields, 4, false)? && settings.leave_clears(),
            false,
            None,
            None,
        ),
        other => return Err(Fail::type_error(format!("unknown focus event kind {other}"))),
    })
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("focus_policy")
        .ctor(|_cx, _inits| Ok(()))
        .function("focus_decision", |_cx, args| {
            decision(&Settings::read(&args[0])?, &args[1])
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
        });
}
