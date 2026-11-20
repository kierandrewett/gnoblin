//! `processes` and `process`: the programs a shell starts, and how each one ended.
//!
//! The children are the session's (`gnoblin-shell-host`'s `Processes`: one waiter thread per child, a process group of its own, a timeout
//! and bounded capture), so a program a generation's `init` starts waits until that generation takes over, and one a reload starts again with
//! the same arguments is adopted. This module is the object a shell holds for each of them: the end of a program arrives as an event of the
//! host's mailbox, and is told from the turn after it was seen, to the process's own `exited` signal and to `processes`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_shell_host::process::{ProcessCode, ProcessError, SpawnOptions, StreamMode};

use super::glue::{self, PerGeneration};
use super::listener::{self, Registered};
use super::operation;
use super::values::{enum_name, fields_of};
use crate::events::{ExitNotice, ShellEvent};
use crate::session;

/// One started program.
pub(super) struct ProcessState {
    id: u64,
    argv: Vec<String>,
    ended: Option<ExitNotice>,
    /// The operation `wait` returned: the same one every time.
    waiting: Option<Obj>,
}

/// What the service keeps for a generation.
#[derive(Default)]
struct ProcessTable {
    service: Obj,
    hooked: bool,
    /// The processes that have not ended, oldest first: their ids and objects.
    running: Vec<(u64, Obj)>,
    listeners: Registered,
}

thread_local! {
    static PROCESSES: PerGeneration<ProcessTable> = PerGeneration::new();
}

/// The words a failed spawn or kill carries, in the code the ABI has for them.
fn failure(error: &ProcessError, what: &str) -> Fail {
    let code = match error.code {
        ProcessCode::InvalidArgument => abi::E_TYPE,
        ProcessCode::NotFound => abi::E_NO_SUCH_NODE,
        ProcessCode::Busy => abi::E_BUSY,
        ProcessCode::Internal => abi::E_ERR,
    };
    Fail::new(code, format!("{what}: {}", error.message))
}

/// The stream mode an enum value names.
fn stream(cx: &Cx<'_>, value: &Val) -> Res<StreamMode> {
    match enum_name(cx, value)?.as_str() {
        "inherit" => Ok(StreamMode::Log),
        "discard" => Ok(StreamMode::Drop),
        "capture" => Ok(StreamMode::Keep),
        other => Err(Fail::type_error(format!(
            "gnoblin.spawn: `{other}` is not a stream mode"
        ))),
    }
}

/// The options of a spawn, from the record a shell gave.
fn options_of(cx: &Cx<'_>, options: Option<&Val>) -> Res<SpawnOptions> {
    let mut out = SpawnOptions::default();
    let Some(options) = options.filter(|value| !value.is_none()) else {
        return Ok(out);
    };
    for (name, value) in fields_of(cx, "spawn_options", options)? {
        if value.is_none() {
            continue;
        }
        match name {
            "cwd" => out.cwd = Some(PathBuf::from(value.text()?)),
            "env" => {
                let mut env = BTreeMap::new();
                for pair in value.list()? {
                    let pair = pair.text()?;
                    let (key, rest) = pair.split_once('=').ok_or_else(|| {
                        Fail::type_error(format!("gnoblin.spawn: env entries are NAME=value, not `{pair}`"))
                    })?;
                    if key.is_empty() {
                        return Err(Fail::type_error(format!(
                            "gnoblin.spawn: `{pair}` has no variable name"
                        )));
                    }
                    env.insert(key.to_owned(), rest.to_owned());
                }
                out.env = env;
            }
            "stdin" => out.stdin = Some(value.text()?.to_owned()),
            "stdout" => out.stdout = stream(cx, value)?,
            "stderr" => out.stderr = stream(cx, value)?,
            "output_limit" => {
                let limit = value.int()?;
                out.output_limit = Some(
                    u64::try_from(limit)
                        .map_err(|_| Fail::type_error("gnoblin.spawn: output_limit must be from 0 to 4194304"))?,
                );
            }
            "timeout" => out.timeout = Some(value.double()?),
            _ => {}
        }
    }
    Ok(out)
}

/// The record `process_exit` of an ending.
fn exit_value(cx: &Cx<'_>, ended: &ExitNotice) -> Res<Val> {
    let some_text = |text: &Option<String>| text.clone().map_or(Val::None, Val::Text);
    glue::record(
        cx,
        "process_exit",
        vec![
            ended.code.map_or(Val::None, |code| Val::Int(i64::from(code))),
            some_text(&ended.signal),
            Val::Bool(ended.timed_out),
            some_text(&ended.stdout),
            some_text(&ended.stderr),
            Val::Bool(ended.truncated & 1 != 0),
            Val::Bool(ended.truncated & 2 != 0),
        ],
    )
}

/// The object of the service `processes` of this generation: where the end of a program is told from.
fn service_object(cx: &Cx<'_>) -> Res<Obj> {
    let (_, obj) = glue::service(cx.ctx(), "processes")?;
    // The service lives as long as the generation: the reference `create` took is not needed.
    cx.unhold(obj);
    Ok(obj)
}

/// `gnoblin.spawn` of an argument vector: starts the program and makes its object.
pub(super) fn spawn(cx: &Cx<'_>, argv: Vec<String>, options: Option<&Val>) -> Res<Val> {
    if argv.is_empty() {
        return Err(Fail::type_error(
            "gnoblin.spawn: expects a command string or a non-empty array of strings",
        ));
    }
    let options = options_of(cx, options)?;
    let ctx = cx.ctx();
    let service = service_object(cx)?;
    hook(ctx, service)?;
    let started =
        session::with(|session| session.spawn(&argv, &options)).map_err(|error| failure(&error, "gnoblin.spawn"))?;
    let obj = cx.spawn(
        "process",
        ProcessState {
            id: started.id,
            argv,
            ended: None,
            waiting: None,
        },
    )?;
    // The library holds a running process until its end is told, whatever the shell does with the handle.
    cx.hold(obj)?;
    PROCESSES.with(|processes| processes.with(ctx, |table| table.running.push((started.id, obj))));
    cx.handout(obj)
}

/// `gnoblin.spawn` of a command line: `/bin/sh -c`.
pub(super) fn shell(cx: &Cx<'_>, command: &str, options: Option<&Val>) -> Res<Val> {
    if command.is_empty() {
        return Err(Fail::type_error(
            "gnoblin.spawn: expects a command string or a non-empty array of strings",
        ));
    }
    spawn(
        cx,
        vec!["/bin/sh".to_owned(), "-c".to_owned(), command.to_owned()],
        options,
    )
}

/// Listens for the end of programs, once for the generation.
fn hook(ctx: GnoblinCtx, service: Obj) -> Res<()> {
    let fresh = PROCESSES.with(|processes| {
        processes.with(ctx, |table| {
            let fresh = !table.hooked;
            table.hooked = true;
            fresh
        })
    });
    if !fresh {
        return Ok(());
    }
    glue::hear(
        ctx,
        "process exit",
        |event| matches!(event, ShellEvent::Exit(_)),
        move |event| {
            if let ShellEvent::Exit(notice) = event {
                let notice = notice.clone();
                let _ = crate::object::run_in(ctx, service, |cx| exited(cx, &notice));
            }
        },
    )?;
    Ok(())
}

/// A program ended: its object records how, settles `wait`, and tells the listeners.
fn exited(cx: &Cx<'_>, notice: &ExitNotice) {
    let ctx = cx.ctx();
    let found = PROCESSES.with(|processes| {
        processes.with(ctx, |table| {
            let at = table.running.iter().position(|(id, _)| *id == notice.process)?;
            Some(table.running.remove(at).1)
        })
    });
    // A program of another generation, or one the C table started, is none of ours.
    let Some(obj) = found else {
        return;
    };
    let ended = notice.clone();
    let waiting = cx
        .with_state::<ProcessState, _>(obj, |process| {
            process.ended = Some(ended);
            process.waiting
        })
        .ok()
        .flatten();
    if let Some(op) = waiting {
        operation::finish(cx, op, Ok(()));
    }
    let process = cx.object_value(obj);
    cx.emit_on(obj, "exited", vec![process.clone()]);
    cx.emit("exited", vec![process]);
    cx.later_on(obj, move |cx| cx.unhold(obj));
}

/// The record `process_exit`, or none while the program runs.
fn exit_of(state: &ProcessState, cx: &Cx<'_>) -> Res<Val> {
    match &state.ended {
        Some(ended) => exit_value(cx, ended),
        None => Ok(Val::None),
    }
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<ProcessState>("process")
        .ctor(|_, _| Err(Fail::refused("a process is started with `gnoblin.spawn`")))
        .get("id", |state, _| Ok(Val::Uint(state.id)))
        .get("argv", |state, _| {
            Ok(Val::List(state.argv.iter().cloned().map(Val::Text).collect()))
        })
        .get("pid", |state, _| {
            let pid = if state.ended.is_some() {
                0
            } else {
                session::with(|session| session.pid(state.id)).unwrap_or(0)
            };
            Ok(Val::Uint(u64::from(pid)))
        })
        .get("alive", |state, _| Ok(Val::Bool(state.ended.is_none())))
        .get("status", |state, cx| {
            cx.enum_val(
                "process_status",
                if state.ended.is_none() { "running" } else { "exited" },
            )
        })
        .get("exit_code", |state, _| {
            Ok(state
                .ended
                .as_ref()
                .and_then(|ended| ended.code)
                .map_or(Val::None, |code| Val::Int(i64::from(code))))
        })
        .get("signal", |state, _| {
            Ok(state
                .ended
                .as_ref()
                .and_then(|ended| ended.signal.clone())
                .map_or(Val::None, Val::Text))
        })
        .get("timed_out", |state, _| {
            Ok(Val::Bool(state.ended.as_ref().is_some_and(|ended| ended.timed_out)))
        })
        .get("exit", |state, cx| exit_of(state, cx))
        .method("wait", |state, cx, _| {
            if let Some(op) = state.waiting {
                return cx.handout(op);
            }
            let op = operation::begin(cx)?;
            state.waiting = Some(op);
            if state.ended.is_some() {
                operation::finish(cx, op, Ok(()));
            }
            cx.handout(op)
        })
        .method("kill", |state, cx, args| {
            let name = match args.first().filter(|value| !value.is_none()) {
                Some(signal) => enum_name(cx, signal)?,
                None => "term".to_owned(),
            };
            let target = format!("process.kill#{}", state.id);
            if state.ended.is_some() {
                return operation::failed(cx, "not_found", &format!("{target}: the process has ended"));
            }
            match session::with(|session| session.kill(state.id, &name)) {
                Ok(()) => {
                    let op = operation::begin(cx)?;
                    operation::finish(cx, op, Ok(()));
                    cx.handout(op)
                }
                Err(error) => operation::failed(cx, error.code.as_str(), &error.message),
            }
        });
    let _ = builder
        .class::<()>("processes")
        .ctor(|cx, _| {
            let me = cx.me();
            PROCESSES.with(|processes| processes.with(cx.ctx(), |table| table.service = me));
            Ok(())
        })
        .method("spawn", |_, cx, args| {
            let argv = args[0]
                .list()?
                .iter()
                .map(|part| part.text().map(str::to_owned))
                .collect::<Res<Vec<String>>>()?;
            spawn(cx, argv, args.get(1))
        })
        .method("shell", |_, cx, args| shell(cx, args[0].text()?, args.get(1)))
        .method("list", |_, cx, _| {
            let running: Vec<Obj> = PROCESSES.with(|processes| {
                processes.with(cx.ctx(), |table| table.running.iter().map(|(_, obj)| *obj).collect())
            });
            running
                .into_iter()
                .map(|obj| cx.handout(obj))
                .collect::<Res<Vec<Val>>>()
                .map(Val::List)
        })
        .method("by_id", |_, cx, args| {
            let id = args[0].uint()?;
            let found = PROCESSES.with(|processes| {
                processes.with(cx.ctx(), |table| {
                    table.running.iter().find(|(held, _)| *held == id).map(|(_, obj)| *obj)
                })
            });
            match found {
                Some(obj) => cx.handout(obj),
                None => Ok(Val::None),
            }
        })
        .method("add_listener", |_, cx, args| {
            let me = cx.me();
            let mut listeners =
                PROCESSES.with(|processes| processes.with(cx.ctx(), |table| std::mem::take(&mut table.listeners)));
            let done = listener::add(cx, me, &mut listeners, &args[0]);
            PROCESSES.with(|processes| processes.with(cx.ctx(), |table| table.listeners = listeners));
            done.map(|_| Val::None)
        })
        .method("remove_listener", |_, cx, args| {
            let mut listeners =
                PROCESSES.with(|processes| processes.with(cx.ctx(), |table| std::mem::take(&mut table.listeners)));
            let done = listener::remove(cx, &mut listeners, &args[0]);
            PROCESSES.with(|processes| processes.with(cx.ctx(), |table| table.listeners = listeners));
            done.map(|()| Val::None)
        });
}

/// The generation ended: its table goes with it. A program a handler started belongs to the user and keeps running.
pub(super) fn forget(ctx: GnoblinCtx) {
    PROCESSES.with(|processes| processes.take(ctx));
}
