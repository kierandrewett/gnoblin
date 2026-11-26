//! `idle`: run code when the session has had no input for a set time, and hold the session awake.
//!
//! The compositor publishes how long the session has been idle (`idle_ms`) and how many holds keep it awake (`inhibitors`) as one object
//! of its `gnoblin` namespace, and `inhibit` and `uninhibit` are its methods. The owner connection ([`crate::verbs`]) keeps the object's
//! properties and says when one changed. A watch is a threshold over that reading and nothing else, so it behaves like the compositor's
//! own idle notifications: an inhibitor delays `idle` as it delays a screen blank, and one that appears while the session is idle resumes
//! it. Nothing is told inside a call: a watch's signals come from the turn after the reading that moved it.
//!
//! The compositor records a hold by name and the connection that made it, so a generation that ends gives every hold back, and a
//! connection that is lost has lost them already: the holds are no holds and the watches are active again.

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_model::{Fields, Value};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};

use super::glue::{self, PerGeneration};
use super::listener;
use crate::events::{OwnerNotice, ShellEvent};

/// The longest idle time a watch may ask for: a day.
const MAX_IDLE_MS: f64 = 86_400_000.0;
/// How many live watches one generation may have.
const MAX_WATCHES: usize = 256;

/// One watch.
struct IdleWatchState {
    id: String,
    ms: f64,
    idle: bool,
    alive: bool,
}

/// One hold on the session's idle state.
struct Hold {
    holder: String,
    alive: bool,
}

/// What the service keeps for a generation.
#[derive(Default)]
struct IdleTable {
    service: Obj,
    hooked: bool,
    /// The last reading: milliseconds idle, and how many holds.
    reading: Option<(u64, u64)>,
    /// The connection ended and nothing has been read since.
    lost: bool,
    /// The live watches, oldest first.
    watches: Vec<Obj>,
    /// The live holds: the object and the name each is held under.
    holds: Vec<(Obj, String)>,
    count: u64,
    /// Whether the missing or lost connection was said already.
    reported: bool,
}

thread_local! {
    static IDLE: PerGeneration<IdleTable> = PerGeneration::new();
}

/// The last reading the table holds, or what the owner connection says now.
fn reading_now(ctx: GnoblinCtx) -> Option<(u64, u64)> {
    let (held, lost) = IDLE.with(|idle| idle.with(ctx, |table| (table.reading, table.lost)));
    if lost { None } else { held.or_else(reading_of_link) }
}

/// What the compositor said, from the owner connection.
fn reading_of_link() -> Option<(u64, u64)> {
    let uint = |name: &str| match glue::owner_property("idle", name)? {
        Value::Uint(value) => Some(value),
        Value::Int(value) => u64::try_from(value).ok(),
        _ => None,
    };
    Some((uint("idle_ms")?, uint("inhibitors")?))
}

/// Whether an event is one the service listens for: the idle object's readings, and the end of the connection.
fn is_idle_event(event: &ShellEvent) -> bool {
    match event {
        ShellEvent::Owner(OwnerNotice::Lost(_)) => true,
        ShellEvent::Owner(OwnerNotice::Property { class, .. }) => class == "idle",
        _ => false,
    }
}

/// Opens the owner connection and listens for what it says, once for the generation. Says once when there is none.
fn connect(cx: &Cx<'_>) -> bool {
    let ctx = cx.ctx();
    let service = cx.me();
    let opened = crate::session::with(|session| session.verbs().map(|_| ()));
    if let Err((_, why)) = opened {
        let first = IDLE.with(|idle| {
            idle.with(ctx, |table| {
                let first = !table.reported;
                table.reported = true;
                first
            })
        });
        if first {
            cx.log(
                abi::LOG_WARN,
                &format!("gnoblin.idle: the shell has no idle connection, so watches never fire and holds are not recorded: {why}"),
            );
        }
        return false;
    }
    let fresh = IDLE.with(|idle| {
        idle.with(ctx, |table| {
            let fresh = !table.hooked;
            table.hooked = true;
            fresh
        })
    });
    if fresh {
        let heard = glue::hear(ctx, "idle readings", is_idle_event, move |event| {
            let event = event.clone();
            let _ = crate::object::run_in(ctx, service, |cx| match event {
                ShellEvent::Owner(OwnerNotice::Lost(why)) => lost(cx, &why),
                _ => deliver(cx),
            });
        });
        if let Err(fail) = heard {
            cx.log(abi::LOG_WARN, &format!("gnoblin.idle: {fail}"));
            return false;
        }
    }
    true
}

/// Moves a watch to what the reading says, and tells it when that changed its state.
///
/// The holds this generation made count even when the reading does not have them yet: a hold is sent to the compositor and is in the
/// next reading, and a watch made after it in the same script must not hear the session go idle in between (and resume a turn later).
/// A hold the compositor has counted is in the reading too, so the larger of the two is the number of holds.
fn evaluate(cx: &Cx<'_>, watch: Obj, reading: Option<(u64, u64)>) {
    let Some((idle_ms, counted)) = reading else {
        return;
    };
    let own = IDLE.with(|idle| idle.with(cx.ctx(), |table| table.holds.len())) as u64;
    let inhibitors = counted.max(own);
    let moved = cx
        .with_state::<IdleWatchState, _>(watch, |state| {
            let idle = inhibitors == 0 && idle_ms as f64 >= state.ms;
            (state.alive && idle != state.idle).then(|| {
                state.idle = idle;
                idle
            })
        })
        .ok()
        .flatten();
    if let Some(idle) = moved {
        let me = cx.object_value(watch);
        cx.emit_on(watch, if idle { "idle" } else { "resume" }, vec![me]);
    }
}

/// What the compositor said arrived: every watch looks at it.
fn deliver(cx: &Cx<'_>) {
    let ctx = cx.ctx();
    let Some(reading) = reading_of_link() else {
        return;
    };
    let watches = IDLE.with(|idle| {
        idle.with(ctx, |table| {
            table.reading = Some(reading);
            table.lost = false;
            table.watches.clone()
        })
    });
    for watch in watches {
        evaluate(cx, watch, Some(reading));
    }
    cx.changed("reading");
    cx.changed("idle_ms");
    cx.changed("inhibitors");
}

/// The connection ended: nothing is idle any more, and the holds went with it.
fn lost(cx: &Cx<'_>, why: &str) {
    let (watches, holds, first) = IDLE.with(|idle| {
        idle.with(cx.ctx(), |table| {
            table.reading = None;
            table.lost = true;
            let first = !table.reported;
            table.reported = true;
            (table.watches.clone(), std::mem::take(&mut table.holds), first)
        })
    });
    for (hold, _) in holds {
        let _ = cx.with_state::<Hold, _>(hold, |state| state.alive = false);
        cx.later_on(hold, move |cx| cx.unhold(hold));
    }
    for watch in watches {
        let resumed = cx
            .with_state::<IdleWatchState, _>(watch, |state| std::mem::replace(&mut state.idle, false))
            .unwrap_or(false);
        if resumed {
            let me = cx.object_value(watch);
            cx.emit_on(watch, "resume", vec![me]);
        }
    }
    if first {
        cx.log(
            abi::LOG_WARN,
            &format!("gnoblin.idle: the connection to the compositor ended ({why}); the session's holds went with it"),
        );
    }
}

/// `watch`.
fn watch(cx: &Cx<'_>, args: &[Val]) -> Res<Val> {
    let ms = args[0].double()?;
    if !ms.is_finite() || ms <= 0.0 || ms > MAX_IDLE_MS {
        return Err(Fail::type_error(format!(
            "gnoblin.idle.watch: ms must be a number above 0 and at most {MAX_IDLE_MS}"
        )));
    }
    let ctx = cx.ctx();
    let full = IDLE.with(|idle| idle.with(ctx, |table| table.watches.len() >= MAX_WATCHES));
    if full {
        return Err(Fail::type_error(format!(
            "gnoblin.idle.watch: a generation has at most {MAX_WATCHES} live watches"
        )));
    }
    connect(cx);
    let id = IDLE.with(|idle| {
        idle.with(ctx, |table| {
            table.count += 1;
            format!("idle-{}", table.count)
        })
    });
    let obj = cx.spawn(
        "idle_watch",
        IdleWatchState {
            id,
            ms,
            idle: false,
            alive: true,
        },
    )?;
    cx.hold(obj)?;
    if let Some(given) = args.get(1).filter(|value| !value.is_none()) {
        let mut regs = Vec::new();
        listener::add(cx, obj, &mut regs, given)?;
    }
    // A reading the library already has decides the first state at once, so a watch made while the session is idle does not wait for
    // the next change of the counter.
    let reading = reading_now(ctx);
    IDLE.with(|idle| {
        idle.with(ctx, |table| {
            table.watches.push(obj);
            if reading.is_some() {
                table.reading = reading;
            }
        });
    });
    evaluate(cx, obj, reading);
    cx.handout(obj)
}

/// Ends a watch: its `closed` is told once.
fn cancel(cx: &Cx<'_>, watch: &mut IdleWatchState) {
    if !watch.alive {
        return;
    }
    watch.alive = false;
    watch.idle = false;
    let obj = cx.me();
    IDLE.with(|idle| idle.with(cx.ctx(), |table| table.watches.retain(|held| *held != obj)));
    let me = cx.object_value(obj);
    cx.emit("closed", vec![me, Val::Text("cancelled".to_owned())]);
    // What is queued for the watch is told first, and then the library lets go of it.
    cx.later_on(obj, move |cx| cx.unhold(obj));
}

/// `inhibit`.
fn inhibit(cx: &Cx<'_>, holder: &str) -> Res<Val> {
    if holder.is_empty() {
        return Err(Fail::type_error(
            "gnoblin.idle.inhibit: the holder must be a non-empty string",
        ));
    }
    let ctx = cx.ctx();
    let held = IDLE.with(|idle| {
        idle.with(ctx, |table| {
            table.holds.iter().find(|(_, name)| name == holder).map(|(obj, _)| *obj)
        })
    });
    if let Some(obj) = held {
        return cx.handout(obj);
    }
    let connected = connect(cx);
    let obj = cx.spawn(
        "idle_inhibitor",
        Hold {
            holder: holder.to_owned(),
            alive: connected,
        },
    )?;
    cx.hold(obj)?;
    let service = cx.me();
    if connected {
        let mut args = Fields::new();
        args.insert("holder".to_owned(), Value::Text(holder.to_owned()));
        let sent = glue::owner_call(ctx, "idle.inhibit", "idle", "inhibit", args, move |outcome| {
            if let Err(code) = outcome {
                let _ = crate::object::run_in(ctx, service, |cx| refused(cx, obj, &code));
            }
        });
        match sent {
            Ok(()) => IDLE.with(|idle| idle.with(ctx, |table| table.holds.push((obj, holder.to_owned())))),
            Err(fail) => {
                cx.log(
                    abi::LOG_WARN,
                    &format!("gnoblin.idle: the hold {holder:?} was not sent: {fail}"),
                );
                let _ = cx.with_state::<Hold, _>(obj, |state| state.alive = false);
                cx.later_on(obj, move |cx| cx.unhold(obj));
            }
        }
    } else {
        cx.later_on(obj, move |cx| cx.unhold(obj));
    }
    cx.handout(obj)
}

/// The compositor refused a hold: it is not a hold, and `alive` says so.
fn refused(cx: &Cx<'_>, obj: Obj, code: &str) {
    let removed = IDLE.with(|idle| {
        idle.with(cx.ctx(), |table| {
            let before = table.holds.len();
            table.holds.retain(|(held, _)| *held != obj);
            table.holds.len() != before
        })
    });
    if !removed {
        return;
    }
    let holder = cx
        .with_state::<Hold, _>(obj, |state| {
            state.alive = false;
            state.holder.clone()
        })
        .unwrap_or_default();
    cx.log(
        abi::LOG_WARN,
        &format!("gnoblin.idle: the compositor refused the hold {holder:?}: {code}"),
    );
    cx.later_on(obj, move |cx| cx.unhold(obj));
}

/// Gives a hold back.
fn release(cx: &Cx<'_>, hold: &mut Hold) {
    if !hold.alive {
        return;
    }
    hold.alive = false;
    let obj = cx.me();
    IDLE.with(|idle| idle.with(cx.ctx(), |table| table.holds.retain(|(held, _)| *held != obj)));
    let mut args = Fields::new();
    args.insert("holder".to_owned(), Value::Text(hold.holder.clone()));
    if let Err(fail) = glue::owner_fire("idle.uninhibit", "idle", "uninhibit", args) {
        cx.log(
            abi::LOG_WARN,
            &format!("gnoblin.idle: a hold was not given back: {fail}"),
        );
    }
    cx.later_on(obj, move |cx| cx.unhold(obj));
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<IdleWatchState>("idle_watch")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "a watch is made with `gnoblin.idle.watch`",
            ))
        })
        .get("id", |state, _| Ok(Val::Text(state.id.clone())))
        .get("ms", |state, _| Ok(Val::Double(state.ms)))
        .get("is_idle", |state, _| Ok(Val::Bool(state.idle)))
        .get("alive", |state, _| Ok(Val::Bool(state.alive)))
        .method("cancel", |state, cx, _| {
            cancel(cx, state);
            Ok(Val::None)
        });
    let _ = builder
        .class::<Hold>("idle_inhibitor")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "a hold is made with `gnoblin.idle.inhibit`",
            ))
        })
        .get("holder", |state, _| Ok(Val::Text(state.holder.clone())))
        .get("alive", |state, _| Ok(Val::Bool(state.alive)))
        .method("release", |state, cx, _| {
            release(cx, state);
            Ok(Val::None)
        });
    let _ = builder
        .class::<()>("idle")
        .ctor(|cx, _| {
            let me = cx.me();
            IDLE.with(|idle| idle.with(cx.ctx(), |table| table.service = me));
            Ok(())
        })
        .get("reading", |_, cx| match reading_now(cx.ctx()) {
            Some((idle_ms, inhibitors)) => {
                glue::record(cx, "idle_reading", vec![Val::Uint(idle_ms), Val::Uint(inhibitors)])
            }
            None => Ok(Val::None),
        })
        .get("idle_ms", |_, cx| {
            Ok(Val::Uint(reading_now(cx.ctx()).map_or(0, |(idle_ms, _)| idle_ms)))
        })
        .get("inhibitors", |_, cx| {
            Ok(Val::Uint(reading_now(cx.ctx()).map_or(0, |(_, inhibitors)| inhibitors)))
        })
        .get("watches", |_, cx| {
            let live: Vec<Obj> = IDLE.with(|idle| idle.with(cx.ctx(), |table| table.watches.clone()));
            live.into_iter()
                .map(|obj| cx.handout(obj))
                .collect::<Res<Vec<Val>>>()
                .map(Val::List)
        })
        .method("watch", |_, cx, args| watch(cx, args))
        .method("inhibit", |_, cx, args| inhibit(cx, args[0].text()?));
}

/// The generation ended: every hold is given back, and its table goes.
pub(super) fn forget(ctx: GnoblinCtx) {
    let Some(table) = IDLE.with(|idle| idle.take(ctx)) else {
        return;
    };
    for (_, holder) in table.holds {
        let mut args = Fields::new();
        args.insert("holder".to_owned(), Value::Text(holder));
        let _ = glue::owner_fire("idle.uninhibit", "idle", "uninhibit", args);
    }
}
