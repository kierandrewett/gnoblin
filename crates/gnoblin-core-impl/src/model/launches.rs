//! `launches`: launch feedback, the time between starting an application and its window appearing.
//!
//! A shell records a pending launch with `begin`. A window that joins `gnoblin.clients`, takes focus or names its application starts the
//! oldest pending launch that matches, and a launch that no window answers times out at its deadline (a timer of the session's clock).
//! The service listens to the client collection from the turn it is made, so the windows that are there already answer none.
//!
//! A launch leaves `pending` once. Its signals are told in the order the TypeScript library queued its handlers: `changed`, then
//! `started` or `timed_out`, then `closed`, which is always last.

use std::time::{SystemTime, UNIX_EPOCH};

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::glue::{self, PerGeneration};
use super::listener::{self, Registered};
use super::with_world;

/// The most launches the registry keeps. The oldest finished one is dropped for a new one; with all pending, `begin` ends the new one.
const MAX_LAUNCHES: usize = 64;
/// The deadline of a launch that gave none, in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 3000;
/// The shortest and the longest deadline a launch may have, in milliseconds.
const MIN_TIMEOUT_MS: i64 = 100;
const MAX_TIMEOUT_MS: i64 = 10_000;
/// The longest token and application a launch takes.
const TOKEN_MAX: usize = 128;
const APPLICATION_MAX: usize = 512;

/// Where a launch is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LaunchPhase {
    Pending,
    Started,
    TimedOut,
    Ended,
}

impl LaunchPhase {
    fn name(self) -> &'static str {
        match self {
            LaunchPhase::Pending => "pending",
            LaunchPhase::Started => "started",
            LaunchPhase::TimedOut => "timed_out",
            LaunchPhase::Ended => "ended",
        }
    }
}

/// One launch of the registry.
struct LaunchRecord {
    obj: Obj,
    token: String,
    application: String,
    timeout_ms: u64,
    started_at: f64,
    phase: LaunchPhase,
    revision: u64,
    /// The timer that ends it at its deadline, while it is pending.
    timer: u64,
    listeners: Registered,
}

/// What the service keeps for a generation.
#[derive(Default)]
struct LaunchTable {
    service: Obj,
    records: Vec<LaunchRecord>,
}

thread_local! {
    static LAUNCHES: PerGeneration<LaunchTable> = PerGeneration::new();
}

/// The state of every object of the class: a launch is told by its object.
struct LaunchState {
    /// What the launch was when it left the registry: a launch stays readable after it is gone.
    frozen: Option<LaunchRecord>,
}

/// The form two application ids are compared in: lowercase, no `.desktop`.
fn normalise(name: &str) -> String {
    let lower = name.to_lowercase();
    lower.strip_suffix(".desktop").map_or(lower.clone(), str::to_owned)
}

/// Leaves `pending` for `phase`, and queues what is told of it. False for a launch that is not pending.
fn settle(cx: &Cx<'_>, record: &mut LaunchRecord, phase: LaunchPhase, reason: &str) -> bool {
    if record.phase != LaunchPhase::Pending {
        return false;
    }
    record.phase = phase;
    record.revision += 1;
    glue::cancel(std::mem::take(&mut record.timer));
    let launch = cx.object_value(record.obj);
    let fields = ["state", "alive", "revision"]
        .iter()
        .map(|name| Val::Text((*name).to_owned()))
        .collect();
    cx.emit_on(record.obj, "changed", vec![launch.clone(), Val::List(fields)]);
    match phase {
        LaunchPhase::Started => cx.emit_on(record.obj, "started", vec![launch.clone()]),
        LaunchPhase::TimedOut => cx.emit_on(record.obj, "timed_out", vec![launch.clone()]),
        LaunchPhase::Pending | LaunchPhase::Ended => {}
    }
    cx.emit_on(record.obj, "closed", vec![launch, Val::Text(reason.to_owned())]);
    true
}

/// A window of `app_id` appeared or took focus: it starts the oldest pending launch that matches.
fn answered(cx: &Cx<'_>, app_id: &str) {
    if app_id.is_empty() {
        return;
    }
    let wanted = normalise(app_id);
    LAUNCHES.with(|launches| {
        launches.with(cx.ctx(), |table| {
            if let Some(record) = table
                .records
                .iter_mut()
                .find(|record| record.phase == LaunchPhase::Pending && normalise(&record.application) == wanted)
            {
                settle(cx, record, LaunchPhase::Started, "started");
            }
        });
    });
}

/// A launch's deadline came.
fn expired(cx: &Cx<'_>, launch: Obj) {
    LAUNCHES.with(|launches| {
        launches.with(cx.ctx(), |table| {
            if let Some(record) = table.records.iter_mut().find(|record| record.obj == launch) {
                settle(cx, record, LaunchPhase::TimedOut, "timed_out");
            }
        });
    });
}

/// The application id a window has now.
fn app_id_of(ctx: GnoblinCtx, client: Obj) -> String {
    crate::object::with_lib(|lib| {
        let class = lib.class_of(ctx, client)?;
        let member = lib.member_id(class, abi::MEMBER_PROPERTY, "app_id")?;
        lib.get(ctx, client, member)
    })
    .ok()
    .and_then(|value| value.text().ok().map(str::to_owned))
    .unwrap_or_default()
}

/// Makes the service tell `launches` of the windows that appear, take focus or name their application.
fn hook_clients(cx: &Cx<'_>) -> Res<()> {
    let (ctx, service) = (cx.ctx(), cx.me());
    let root = with_world(cx, |world| world.client_root)?;
    cx.listen(
        root,
        "added",
        glue::closure(move |args| {
            tell_of(ctx, service, app_id_of(ctx, args[0].object()?));
            Ok(Val::None)
        }),
    )?;
    cx.listen(
        root,
        "focus_changed",
        glue::closure(move |args| {
            if args[1].bool()? {
                tell_of(ctx, service, app_id_of(ctx, args[0].object()?));
            }
            Ok(Val::None)
        }),
    )?;
    cx.listen(
        root,
        "app_id_changed",
        glue::closure(move |args| {
            tell_of(ctx, service, args[1].text()?.to_owned());
            Ok(Val::None)
        }),
    )?;
    Ok(())
}

fn tell_of(ctx: GnoblinCtx, service: Obj, app_id: String) {
    let _ = crate::object::run_in(ctx, service, |cx| answered(cx, &app_id));
}

/// The options of `begin`: the deadline and the listener.
fn options_of(cx: &Cx<'_>, options: Option<&Val>) -> Res<(u64, Option<Val>)> {
    let Some(options) = options.filter(|value| !value.is_none()) else {
        return Ok((DEFAULT_TIMEOUT_MS, None));
    };
    let mut timeout = DEFAULT_TIMEOUT_MS;
    let mut listener = None;
    for (name, value) in super::values::fields_of(cx, "launch_options", options)? {
        if value.is_none() {
            continue;
        }
        match name {
            "timeout_ms" => {
                let given = value.int()?;
                if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&given) {
                    return Err(Fail::type_error(format!(
                        "gnoblin.launches.begin: timeout_ms must be an integer from {MIN_TIMEOUT_MS} to {MAX_TIMEOUT_MS}"
                    )));
                }
                timeout = u64::try_from(given).unwrap_or(DEFAULT_TIMEOUT_MS);
            }
            "listener" => listener = Some(value.clone()),
            _ => {}
        }
    }
    Ok((timeout, listener))
}

/// `begin`.
fn begin(cx: &Cx<'_>, args: &[Val]) -> Res<Val> {
    let token = args[0].text()?.to_owned();
    let application = args[1].text()?.to_owned();
    if token.is_empty() || token.chars().count() > TOKEN_MAX {
        return Err(Fail::type_error(format!(
            "gnoblin.launches.begin: the token must be a non-empty string of at most {TOKEN_MAX} characters"
        )));
    }
    if application.is_empty() || application.chars().count() > APPLICATION_MAX {
        return Err(Fail::type_error(format!(
            "gnoblin.launches.begin: the application must be a non-empty string of at most {APPLICATION_MAX} characters"
        )));
    }
    let (timeout_ms, listener_val) = options_of(cx, args.get(2))?;
    let ctx = cx.ctx();
    let service = cx.me();
    let obj = cx.spawn("launch", LaunchState { frozen: None })?;
    let mut record = LaunchRecord {
        obj,
        token: token.clone(),
        application,
        timeout_ms,
        started_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |since| since.as_secs_f64() * 1000.0),
        phase: LaunchPhase::Pending,
        revision: 0,
        timer: 0,
        listeners: Vec::new(),
    };
    if let Some(listener_val) = &listener_val {
        listener::add(cx, obj, &mut record.listeners, listener_val)?;
    }
    cx.hold(obj)?;
    // A token that is in use ends the earlier launch: it is replaced.
    let replaced = LAUNCHES.with(|launches| {
        launches.with(ctx, |table| {
            table.records.iter().position(|held| held.token == token).map(|at| {
                let mut earlier = table.records.remove(at);
                settle(cx, &mut earlier, LaunchPhase::Ended, "replaced");
                earlier
            })
        })
    });
    if let Some(earlier) = replaced {
        leave(cx, earlier);
    }
    let full = LAUNCHES.with(|launches| launches.with(ctx, |table| table.records.len() >= MAX_LAUNCHES));
    if full {
        let dropped = LAUNCHES.with(|launches| {
            launches.with(ctx, |table| {
                table
                    .records
                    .iter()
                    .position(|held| held.phase != LaunchPhase::Pending)
                    .map(|at| table.records.remove(at))
            })
        });
        match dropped {
            Some(old) => leave(cx, old),
            None => {
                // All pending: the new launch ends at once, and is not kept.
                settle(
                    cx,
                    &mut record,
                    LaunchPhase::Ended,
                    &format!("the registry holds {MAX_LAUNCHES} pending launches"),
                );
                leave(cx, record);
                return cx.handout(obj);
            }
        }
    }
    record.timer = glue::timer(
        ctx,
        "launch deadline",
        u32::try_from(timeout_ms).unwrap_or(u32::MAX),
        move || {
            let _ = crate::object::run_in(ctx, service, |cx| expired(cx, obj));
        },
    )?;
    LAUNCHES.with(|launches| launches.with(ctx, |table| table.records.push(record)));
    cx.handout(obj)
}

/// The record of the launch `state` is, read by `work`: the registry's, or the one the launch kept when it left it.
fn read<R>(state: &LaunchState, cx: &Cx<'_>, work: impl Fn(&LaunchRecord) -> R) -> Res<R> {
    let me = cx.me();
    let held = LAUNCHES.with(|launches| {
        launches.with(cx.ctx(), |table| {
            table.records.iter().find(|record| record.obj == me).map(&work)
        })
    });
    match held {
        Some(value) => Ok(value),
        None => state
            .frozen
            .as_ref()
            .map(work)
            .ok_or_else(|| Fail::stale("the launch is not in the registry any more")),
    }
}

/// A launch left the registry: it keeps what it was, and the library lets go of it once what is queued has been told.
fn leave(cx: &Cx<'_>, record: LaunchRecord) {
    let obj = record.obj;
    let _ = cx.with_state::<LaunchState, _>(obj, |launch| launch.frozen = Some(record));
    cx.later_on(obj, move |cx| cx.unhold(obj));
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<LaunchState>("launch")
        .ctor(|_, _| Err(Fail::refused("a launch is made by `gnoblin.launches.begin`")))
        .get("token", |state, cx| {
            read(state, cx, |record| Val::Text(record.token.clone()))
        })
        .get("application", |state, cx| {
            read(state, cx, |record| Val::Text(record.application.clone()))
        })
        .get("timeout_ms", |state, cx| {
            read(state, cx, |record| Val::Uint(record.timeout_ms))
        })
        .get("started_at", |state, cx| {
            read(state, cx, |record| Val::Double(record.started_at))
        })
        .get("state", |state, cx| {
            let name = read(state, cx, |record| record.phase.name())?;
            cx.enum_val("launch_state", name)
        })
        .get("alive", |state, cx| {
            read(state, cx, |record| Val::Bool(record.phase == LaunchPhase::Pending))
        })
        .get("revision", |state, cx| {
            read(state, cx, |record| Val::Uint(record.revision))
        })
        .method("finish", |_, cx, _| {
            let me = cx.me();
            LAUNCHES.with(|launches| {
                launches.with(cx.ctx(), |table| {
                    if let Some(record) = table.records.iter_mut().find(|record| record.obj == me) {
                        settle(cx, record, LaunchPhase::Ended, "ended");
                    }
                });
            });
            Ok(Val::None)
        });
    let _ = builder
        .class::<()>("launches")
        .ctor(|cx, _| {
            let me = cx.me();
            let fresh = LAUNCHES.with(|launches| {
                launches.with(cx.ctx(), |table| {
                    let fresh = table.service == 0;
                    table.service = me;
                    fresh
                })
            });
            if fresh {
                hook_clients(cx)?;
            }
            Ok(())
        })
        .get("pending", |_, cx| {
            Ok(Val::Bool(LAUNCHES.with(|launches| {
                launches.with(cx.ctx(), |table| {
                    table.records.iter().any(|record| record.phase == LaunchPhase::Pending)
                })
            })))
        })
        .method("list", |_, cx, _| {
            let objects: Vec<Obj> = LAUNCHES.with(|launches| {
                launches.with(cx.ctx(), |table| {
                    table.records.iter().map(|record| record.obj).collect()
                })
            });
            objects
                .into_iter()
                .map(|obj| cx.handout(obj))
                .collect::<Res<Vec<Val>>>()
                .map(Val::List)
        })
        .method("begin", |_, cx, args| begin(cx, args));
}

/// The generation ended: its launches go with it.
pub(super) fn forget(ctx: GnoblinCtx) {
    LAUNCHES.with(|launches| launches.take(ctx));
}
