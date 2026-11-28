//! Touchpad gestures that run actions: `swipe_action_binding`, `pinch_action_binding` and the five stock actions of `gestures`.
//!
//! A gesture is bound like a chord (`Swipe4`, `Pinch3`: the gesture and its finger count) and the core claims it, so no client sees it. An action
//! runs once, when the fingers lift, for the way the gesture went; a cancelled gesture runs nothing. The five named actions are
//! `workspace.next`, `workspace.previous`, `window.close`, `window.minimise` (also spelt `window.minimize`) and `window.toggle-maximise`, and a
//! shell can give a function instead of a name.

use std::rc::Rc;

use gnoblin_core::host::GestureKind;
use gnoblin_core::{BindingEvent, GestureBindingHandlers};
use gnoblin_object::server::Builder;
use gnoblin_object::{Closure, Fail, Res, Val};
use gnoblin_std_abi::{STD_WAY_DOWN, STD_WAY_IN, STD_WAY_LEFT, STD_WAY_NONE, STD_WAY_OUT, STD_WAY_RIGHT, STD_WAY_UP};

use super::binding::{BindStep, Flavour, PolicyBinding, gesture_handlers, members};
use super::{requests, stack};
use crate::bindings::way_past;
use crate::object::record::{enumeration, record};
use crate::ui::{Ui, ui_of};

/// The shortest swipe an action binding acts on, in touchpad units.
pub(super) const SWIPE_THRESHOLD: f64 = 48.0;
/// How far from 1 a pinch's scale must be for an action binding to act on it.
pub(super) const PINCH_THRESHOLD: f64 = 0.12;
/// Milliseconds a workspace slide that an action starts takes.
pub(super) const SLIDE_DURATION: f64 = 250.0;

/// The names an action may go by.
pub(super) const ACTION_NAMES: [&str; 6] = [
    "workspace.next",
    "workspace.previous",
    "window.close",
    "window.minimise",
    "window.minimize",
    "window.toggle-maximise",
];

/// What a way of a gesture runs.
#[derive(Clone)]
enum Reaction {
    Named(&'static str),
    Function(Closure),
}

/// The name an action goes by, or a refusal.
fn named(text: &str) -> Res<&'static str> {
    ACTION_NAMES
        .iter()
        .copied()
        .find(|name| *name == text)
        .ok_or_else(|| Fail::type_error(format!("an action is one of {}, got {text}", ACTION_NAMES.join(", "))))
}

/// The actions of a gesture by the way it went: the place of the way in its enum, counted from 1.
#[derive(Clone, Default)]
struct Map(Vec<(u32, Reaction)>);

impl Map {
    fn set(&mut self, way: u32, action: Reaction) {
        self.0.retain(|(held, _)| *held != way);
        self.0.push((way, action));
    }

    fn clear(&mut self, way: u32) {
        self.0.retain(|(held, _)| *held != way);
    }

    fn get(&self, way: u32) -> Option<&Reaction> {
        self.0.iter().find(|(held, _)| *held == way).map(|(_, action)| action)
    }
}

/// A binding that acts when a swipe ends.
pub(super) struct Swipe {
    threshold: f64,
    map: Map,
}

/// A binding that acts when a pinch ends.
pub(super) struct Pinch {
    threshold: f64,
    map: Map,
}

impl Swipe {
    /// The place of a swipe's way in `swipe_way`, counted from 1.
    fn place(way: u32) -> Option<u32> {
        match way {
            STD_WAY_LEFT => Some(1),
            STD_WAY_RIGHT => Some(2),
            STD_WAY_UP => Some(3),
            STD_WAY_DOWN => Some(4),
            _ => None,
        }
    }

    pub(super) fn handlers(&self, ui: &Rc<Ui>) -> GestureBindingHandlers {
        let (threshold, map) = (self.threshold, self.map.clone());
        ends(ui, map, move |event| {
            let way = way_past(
                GestureKind::Swipe,
                event.dx.unwrap_or(0.0),
                event.dy.unwrap_or(0.0),
                1.0,
                threshold,
            );
            Swipe::place(way)
        })
    }
}

impl Pinch {
    /// The place of a pinch's way in `pinch_way`, counted from 1.
    fn place(way: u32) -> Option<u32> {
        match way {
            STD_WAY_IN => Some(1),
            STD_WAY_OUT => Some(2),
            _ => None,
        }
    }

    pub(super) fn handlers(&self, ui: &Rc<Ui>) -> GestureBindingHandlers {
        let (threshold, map) = (self.threshold, self.map.clone());
        ends(ui, map, move |event| {
            let way = way_past(GestureKind::Pinch, 0.0, 0.0, event.scale.unwrap_or(1.0), threshold);
            Pinch::place(way)
        })
    }
}

/// The handlers of a binding that takes every gesture of its trigger, follows none of it, and acts when it ends: `way` says which way it
/// went, if it went far enough.
fn ends(ui: &Rc<Ui>, map: Map, way: impl Fn(&BindingEvent) -> Option<u32> + 'static) -> GestureBindingHandlers {
    let weak = Rc::downgrade(ui);
    gesture_handlers(
        |_| Ok(true),
        move |phase, event| {
            if phase != BindStep::End {
                return;
            }
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let Some(action) = way(event).and_then(|place| map.get(place)) else {
                return;
            };
            match action {
                Reaction::Named(name) => {
                    if let Err(fail) = run_named(&ui, name) {
                        ui.warn(&format!("the gesture action {name} failed: {fail}"));
                    }
                }
                Reaction::Function(run) => {
                    if let Err(fail) = run.call(0, &[event_val(event)]) {
                        ui.warn(&format!("a gesture function threw: {fail}"));
                    }
                }
            }
        },
    )
}

/// The last event of a gesture, as the function of an action is given it.
fn event_val(event: &BindingEvent) -> Val {
    record(vec![
        Val::Text(event.trigger.clone()),
        Val::Uint(event.fingers.unwrap_or(0)),
        Val::Double(event.dx.unwrap_or(0.0)),
        Val::Double(event.dy.unwrap_or(0.0)),
        Val::Double(event.scale.unwrap_or(1.0)),
        Val::Double(event.rotation.unwrap_or(0.0)),
        Val::Double(event.progress.unwrap_or(0.0)),
        Val::Double(event.position.x),
        Val::Double(event.position.y),
    ])
}

// ---- the stock actions --------------------------------------------------------------------------------------------------------------

/// Shows the workspace `step` places from the one the output under the pointer shows; nothing past either end.
fn step_workspace(ui: &Ui, step: i64) -> Res<()> {
    crate::object::record::edit(ui.ctx, |st| {
        let shown = st
            .current_desk()
            .and_then(|desk| st.desk(desk))
            .and_then(|desk| desk.shown);
        let Some(at) = shown.and_then(|shown| st.workspaces.iter().position(|id| *id == shown)) else {
            return Ok(());
        };
        let target = i64::try_from(at)
            .ok()
            .and_then(|at| usize::try_from(at + step).ok())
            .and_then(|index| st.workspaces.get(index).copied());
        match target {
            Some(target) => st.show_workspace(target, None, SLIDE_DURATION, 0.0),
            None => Ok(()),
        }
    })
}

/// Runs the named stock action.
///
/// # Errors
///
/// The name is not one of `ACTION_NAMES`, or the action cannot be done.
pub(super) fn run_named(ui: &Rc<Ui>, name: &str) -> Res<()> {
    match named(name)? {
        "workspace.next" => step_workspace(ui, 1),
        "workspace.previous" => step_workspace(ui, -1),
        "window.close" => {
            if let Some(client) = stack::focused(ui) {
                let _ = ui.node(client).call("close", &[]);
            }
            Ok(())
        }
        "window.minimise" | "window.minimize" => {
            if let Some(client) = stack::focused(ui) {
                requests::minimise(ui, client);
            }
            Ok(())
        }
        _ => {
            if let Some(client) = stack::focused(ui) {
                if crate::nodes::flag(&ui.g, client, "maximised") {
                    requests::unmaximise(ui, client);
                } else {
                    requests::maximise(ui, client);
                }
            }
            Ok(())
        }
    }
}

// ---- the classes --------------------------------------------------------------------------------------------------------------------

fn threshold(text: &str, value: &Val) -> Res<f64> {
    let by = value.double()?;
    if !by.is_finite() || by < 0.0 {
        return Err(Fail::type_error(format!(
            "a gesture threshold is a finite number of at least 0, got {by} for {text}"
        )));
    }
    Ok(by)
}

fn way_place(value: &Val, ways: u32) -> Res<u32> {
    let place = value.enum_value()?;
    if place == 0 || place > ways {
        return Err(Fail::type_error(format!("{place} is not a way")));
    }
    Ok(place)
}

macro_rules! action_class {
    ($builder:expr, $class:literal, $trigger:literal, $fallback:expr, $ways:literal, $variant:ident, $kind:ident) => {{
        let _ = members($builder.class::<PolicyBinding>($class).ctor(|_cx, inits| {
            Ok(PolicyBinding::new(
                inits
                    .take("trigger")
                    .map_or_else(|| Ok($trigger.to_owned()), |value| value.text().map(str::to_owned))?,
                inits.take("priority").map_or(Ok(0.0), |value| value.double())?,
                Flavour::$variant($kind {
                    threshold: match inits.take("threshold") {
                        Some(value) => threshold("threshold", &value)?,
                        None => $fallback,
                    },
                    map: Map::default(),
                }),
            ))
        }))
        .get("threshold", |h, _| match &h.flavour {
            Flavour::$variant(held) => Ok(Val::Double(held.threshold)),
            _ => Ok(Val::None),
        })
        .set("threshold", |h, _, value| {
            h.open()?;
            if let Flavour::$variant(held) = &mut h.flavour {
                held.threshold = threshold("threshold", &value)?;
            }
            Ok(())
        })
        .method("set_action", |h, _, args| {
            h.open()?;
            let place = way_place(&args[0], $ways)?;
            let action = Reaction::Named(named(args[1].text()?)?);
            if let Flavour::$variant(held) = &mut h.flavour {
                held.map.set(place, action);
            }
            Ok(Val::None)
        })
        .method("set_function", |h, _, args| {
            h.open()?;
            let place = way_place(&args[0], $ways)?;
            let run = Rc::clone(args[1].closure()?);
            if let Flavour::$variant(held) = &mut h.flavour {
                held.map.set(place, Reaction::Function(run));
            }
            Ok(Val::None)
        })
        .method("clear", |h, _, args| {
            h.open()?;
            let place = way_place(&args[0], $ways)?;
            if let Flavour::$variant(held) = &mut h.flavour {
                held.map.clear(place);
            }
            Ok(Val::None)
        });
    }};
}

pub(super) fn register(builder: &mut Builder) {
    action_class!(
        builder,
        "swipe_action_binding",
        "Swipe3",
        SWIPE_THRESHOLD,
        4,
        Swipe,
        Swipe
    );
    action_class!(
        builder,
        "pinch_action_binding",
        "Pinch2",
        PINCH_THRESHOLD,
        2,
        Pinch,
        Pinch
    );
    let _ = builder
        .class::<()>("gestures")
        .ctor(|_cx, _inits| Ok(()))
        .get("swipe_threshold", |_, _| Ok(Val::Double(SWIPE_THRESHOLD)))
        .get("pinch_threshold", |_, _| Ok(Val::Double(PINCH_THRESHOLD)))
        .get("action_names", |_, _| {
            Ok(Val::List(
                ACTION_NAMES.iter().map(|name| Val::Text((*name).to_owned())).collect(),
            ))
        })
        .method("run", |_, cx, args| {
            let ui = ui_of(cx)?;
            run_named(&ui, args[0].text()?)?;
            Ok(Val::None)
        })
        .function("direction", |_cx, args| {
            let by = match args.get(2) {
                None | Some(Val::None) => SWIPE_THRESHOLD,
                Some(value) => threshold("threshold", value)?,
            };
            let way = way_past(GestureKind::Swipe, args[0].double()?, args[1].double()?, 1.0, by);
            Ok(match way {
                STD_WAY_NONE => Val::None,
                other => Swipe::place(other).map_or(Val::None, |place| record(vec![enumeration(place)])),
            })
        });
}
