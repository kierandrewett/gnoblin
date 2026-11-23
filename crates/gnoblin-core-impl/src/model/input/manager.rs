//! `input_manager`: the contexts of the desktop (one for each seat, then the shell's own), their devices, and how the seats and devices
//! the compositor reports become contexts and devices.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};

use super::context::{attach, dispose, register_context};
use super::devices::{disown, held_by, own, sync_devices};
use super::{Life, seat, with};

/// Brings the contexts up to date with the seats, and each context's devices with its seat's.
pub(super) fn sync(cx: &Cx<'_>) {
    if !super::super::values::attached(cx) {
        return;
    }
    let seats = seat::seats();
    for node in &seats {
        let known = with(cx.ctx(), |table| table.seats.iter().any(|(seat, _)| seat == node));
        if known {
            continue;
        }
        let name = seat::name(*node);
        let made = cx.spawn("input_context", ()).and_then(|obj| {
            cx.hold(obj)?;
            register_context(cx, obj, &name)?;
            attach(cx, obj, *node, false)?;
            Ok(obj)
        });
        match made {
            Ok(obj) => with(cx.ctx(), |table| table.seats.push((*node, obj))),
            Err(fail) => cx.log(
                gnoblin_core_abi::LOG_WARN,
                &format!("the input module could not follow the seat {node}: {}", fail.message),
            ),
        }
    }
    let gone: Vec<(u64, Obj)> = with(cx.ctx(), |table| {
        let mut gone = Vec::new();
        table.seats.retain(|(node, obj)| {
            if seats.contains(node) {
                true
            } else {
                gone.push((*node, *obj));
                false
            }
        });
        gone
    });
    for (node, context) in gone {
        // The compositor released the seat's claims with it; there is nothing to unclaim on. A context a shell made on that seat has
        // nothing left to hold either.
        let scripted: Vec<Obj> = with(cx.ctx(), |table| {
            let mut ended = Vec::new();
            table.scripted.retain(|obj| {
                if table.contexts.get(obj).is_some_and(|held| held.seat == node) {
                    ended.push(*obj);
                    false
                } else {
                    true
                }
            });
            ended
        });
        for obj in scripted {
            dispose(cx, obj);
            cx.later_on(obj, move |cx| cx.unhold(obj));
        }
        dispose(cx, context);
        cx.later_on(context, move |cx| cx.unhold(context));
    }
    let contexts: Vec<Obj> = with(cx.ctx(), |table| table.seats.iter().map(|(_, obj)| *obj).collect());
    for context in contexts {
        sync_devices(cx, context);
    }
}

fn all(cx: &Cx<'_>) -> Vec<Obj> {
    sync(cx);
    with(cx.ctx(), |table| {
        table
            .seats
            .iter()
            .map(|(_, obj)| *obj)
            .chain(table.scripted.iter().copied())
            .collect()
    })
}

fn primary(cx: &Cx<'_>) -> Res<Obj> {
    sync(cx);
    with(cx.ctx(), |table| table.seats.first().map(|(_, obj)| *obj)).ok_or_else(|| {
        Fail::refused(
            ErrorCode::Unavailable,
            "gnoblin.input.primary: the compositor reports no input seat",
        )
    })
}

/// Puts a context a shell made into the contexts, over the first seat's devices.
fn add_context(cx: &Cx<'_>, context: Obj) -> Res<()> {
    let present = all(cx);
    if present.contains(&context) {
        return Ok(());
    }
    let what = "gnoblin.input.contexts.add";
    let (life, name) = with(cx.ctx(), |table| {
        table.contexts.get(&context).map(|held| (held.life, held.name.clone()))
    })
    .ok_or_else(|| {
        Fail::type_error(format!(
            "{what}: expects an InputContext made with new InputContext({{ name }})"
        ))
    })?;
    if life == Life::Removed {
        return Err(Fail::stale(format!("{what}: the context \"{name}\" was removed")));
    }
    if name.is_empty() || name.contains('\n') {
        return Err(Fail::type_error(format!(
            "{what}: a context name is one non-empty line, got {name:?}"
        )));
    }
    let taken = with(cx.ctx(), |table| {
        present
            .iter()
            .any(|other| table.contexts.get(other).is_some_and(|held| held.name == name))
    });
    if taken {
        return Err(Fail::refused(
            ErrorCode::AlreadyExists,
            format!("{what}: a context named \"{name}\" is already there"),
        ));
    }
    let Some(seat_node) = with(cx.ctx(), |table| table.seats.first().map(|(node, _)| *node)) else {
        return Err(Fail::refused(
            ErrorCode::Unavailable,
            format!("{what}: the compositor reports no input seat to hold the context"),
        ));
    };
    attach(cx, context, seat_node, true)?;
    cx.hold(context)?;
    with(cx.ctx(), |table| table.scripted.push(context));
    Ok(())
}

/// Takes a context a shell added out of the contexts: its devices go back to their seat's context and its bindings stop.
fn remove_context(cx: &Cx<'_>, context: Obj) -> Res<bool> {
    sync(cx);
    let (is_seat, is_scripted, name) = with(cx.ctx(), |table| {
        (
            table.seats.iter().any(|(_, obj)| *obj == context),
            table.scripted.contains(&context),
            table
                .contexts
                .get(&context)
                .map(|held| held.name.clone())
                .unwrap_or_default(),
        )
    });
    if is_seat {
        return Err(Fail::refused(
            ErrorCode::Denied,
            format!(
                "gnoblin.input.contexts.remove: \"{name}\" is a seat the compositor reports; only contexts a script added can be removed"
            ),
        ));
    }
    if !is_scripted {
        return Ok(false);
    }
    let (home, held) = with(cx.ctx(), |table| {
        let seat_node = table.contexts.get(&context).map_or(0, |held| held.seat);
        let home = table
            .seats
            .iter()
            .find(|(node, _)| *node == seat_node)
            .map(|(_, obj)| *obj);
        let held: Vec<Obj> = table
            .contexts
            .get(&context)
            .map(|held| held.owned.iter().map(|(device, _)| *device).collect())
            .unwrap_or_default();
        (home, held)
    });
    if let Some(home) = home {
        for device in held {
            disown(cx, context, device);
            if let Err(fail) = own(cx, home, device) {
                cx.log(gnoblin_core_abi::LOG_WARN, &format!("input: {}", fail.message));
            }
        }
    }
    with(cx.ctx(), |table| table.scripted.retain(|other| *other != context));
    dispose(cx, context);
    cx.later_on(context, move |cx| cx.unhold(context));
    Ok(true)
}

fn list(cx: &Cx<'_>, objects: Vec<Obj>) -> Res<Val> {
    objects
        .into_iter()
        .map(|obj| cx.handout(obj))
        .collect::<Res<Vec<Val>>>()
        .map(Val::List)
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("input_manager")
        .ctor(|cx, _| {
            let me = cx.me();
            with(cx.ctx(), |table| table.manager = me);
            Ok(())
        })
        .get("primary", |_, cx| cx.handout(primary(cx)?))
        .get("contexts", |_, cx| list(cx, all(cx)))
        .get("devices", |_, cx| {
            let mut devices = Vec::new();
            for context in all(cx) {
                if let Val::List(items) = held_by(cx, context)? {
                    devices.extend(items);
                }
            }
            Ok(Val::List(devices))
        })
        .get("focused_client", |_, cx| {
            let Ok(context) = primary(cx) else {
                return Ok(Val::None);
            };
            match super::keys::focused_client(cx, context) {
                Some(client) => cx.handout(client),
                None => Ok(Val::None),
            }
        })
        .get("document", |_, cx| {
            Ok(Val::Text(with(cx.ctx(), |table| table.settings.document())))
        })
        .get("document_revision", |_, cx| {
            Ok(Val::Uint(with(cx.ctx(), |table| table.settings.revision())))
        })
        .method("find_device", |_, cx, args| {
            let id = args[0].text()?.to_owned();
            sync(cx);
            let found = with(cx.ctx(), |table| {
                table
                    .devices
                    .iter()
                    .find(|(_, held)| held.alive && held.id == id)
                    .map(|(obj, _)| *obj)
            });
            match found {
                Some(device) => cx.handout(device),
                None => Ok(Val::None),
            }
        })
        .method("find_context", |_, cx, args| {
            let name = args[0].text()?.to_owned();
            let found = all(cx).into_iter().find(|context| {
                with(cx.ctx(), |table| {
                    table.contexts.get(context).is_some_and(|held| held.name == name)
                })
            });
            match found {
                Some(context) => cx.handout(context),
                None => Ok(Val::None),
            }
        })
        .method("add_context", |_, cx, args| {
            add_context(cx, args[0].object()?).map(|()| Val::None)
        })
        .method("remove_context", |_, cx, args| {
            remove_context(cx, args[0].object()?).map(Val::Bool)
        });
}
