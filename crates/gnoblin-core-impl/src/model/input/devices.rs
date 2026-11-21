//! `device`: an input device of a seat, and the roster a seat context keeps of the devices the seat reports. A device belongs to one context
//! at a time and carries the bindings that match only its own events; they run before a layout's and the context's.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::binding::{bind_closure, bind_handler, follow, new_collection, unfollow, wants_only};
use super::context::{check_device_name, make_binding_only, release_binding_only};
use super::{BindingTier, InputDevice, Life, seat, with};

/// The devices a context holds. A seat context holds the devices of the seat that no other context took; one a shell made holds the
/// devices it was given.
pub(super) fn held_by(cx: &Cx<'_>, context: Obj) -> Res<Val> {
    sync_devices(cx, context);
    let held: Vec<Obj> = with(cx.ctx(), |table| {
        table
            .contexts
            .get(&context)
            .map(|held| {
                if held.scripted {
                    held.owned.iter().map(|(device, _)| *device).collect()
                } else {
                    held.roster
                        .iter()
                        .map(|(_, device)| *device)
                        .filter(|device| held.owned.iter().any(|(owned, _)| owned == device))
                        .collect()
                }
            })
            .unwrap_or_default()
    });
    held.into_iter()
        .map(|device| cx.handout(device))
        .collect::<Res<Vec<Val>>>()
        .map(Val::List)
}

/// Makes the context match the events of `device`: its bindings, and the context's own when the context is one a shell made.
pub(super) fn own(cx: &Cx<'_>, context: Obj, device: Obj) -> Res<()> {
    let (name, bindings, own_bindings, scripted) = with(cx.ctx(), |table| -> Res<(String, Obj, Obj, bool)> {
        let (name, bindings) = table
            .devices
            .get(&device)
            .map(|device| (device.name.clone(), device.bindings))
            .ok_or_else(|| Fail::stale("the device is gone"))?;
        let held = table.context(context)?;
        Ok((name, bindings, held.own, held.scripted))
    })?;
    let mut follows = vec![follow(cx, context, bindings, BindingTier::Device, None, &name)?];
    if scripted {
        match follow(cx, context, own_bindings, BindingTier::ContextRec, None, &name) {
            Ok(id) => follows.push(id),
            Err(fail) => {
                follows.into_iter().for_each(|id| unfollow(cx, context, id));
                return Err(fail);
            }
        }
    }
    with(cx.ctx(), |table| {
        if let Some(held) = table.contexts.get_mut(&context) {
            held.owned.push((device, follows));
        }
        if let Some(held) = table.devices.get_mut(&device) {
            held.context = context;
        }
    });
    Ok(())
}

/// Stops the context matching the events of `device`: an input one of its bindings runs ends, so no button stays held.
pub(super) fn disown(cx: &Cx<'_>, context: Obj, device: Obj) {
    let follows = with(cx.ctx(), |table| {
        let held = table.contexts.get_mut(&context)?;
        let at = held.owned.iter().position(|(owned, _)| *owned == device)?;
        Some(held.owned.remove(at).1)
    });
    for id in follows.unwrap_or_default() {
        unfollow(cx, context, id);
    }
}

/// Brings the roster up to date with the seat's device list: a device that appeared joins this context, a device that went is retired and
/// leaves whichever context held it. Does nothing for a context that is not a seat.
pub(super) fn sync_devices(cx: &Cx<'_>, context: Obj) {
    let Some((seat_node, manager)) = with(cx.ctx(), |table| {
        let held = table.contexts.get(&context)?;
        (held.life == Life::Live && !held.scripted).then_some((held.seat, table.manager))
    }) else {
        return;
    };
    let mut seen: Vec<String> = Vec::new();
    for item in seat::devices(seat_node) {
        let Some((id, name, capabilities)) = seat::split_device(&item) else {
            continue;
        };
        seen.push(id.clone());
        let known = with(cx.ctx(), |table| {
            table
                .contexts
                .get(&context)
                .is_some_and(|held| held.roster.iter().any(|(known, _)| *known == id))
        });
        if known {
            continue;
        }
        let Ok(device) = make_device(cx, context, &id, &name, capabilities) else {
            continue;
        };
        with(cx.ctx(), |table| {
            if let Some(held) = table.contexts.get_mut(&context) {
                held.roster.push((id.clone(), device));
            }
        });
        if let Err(fail) = own(cx, context, device) {
            cx.log(
                gnoblin_core_abi::LOG_WARN,
                &format!("InputContext.sync_devices: {}", fail.message),
            );
        }
        cx.emit_on(manager, "device_added", vec![cx.object_value(device)]);
    }
    let gone: Vec<(String, Obj)> = with(cx.ctx(), |table| {
        table
            .contexts
            .get_mut(&context)
            .map(|held| {
                let mut gone = Vec::new();
                held.roster.retain(|(id, device)| {
                    if seen.contains(id) {
                        true
                    } else {
                        gone.push((id.clone(), *device));
                        false
                    }
                });
                gone
            })
            .unwrap_or_default()
    });
    for (_, device) in gone {
        let holder = with(cx.ctx(), |table| table.devices.get(&device).map(|held| held.context));
        if let Some(holder) = holder {
            disown(cx, holder, device);
        }
        with(cx.ctx(), |table| {
            if let Some(held) = table.devices.get_mut(&device) {
                held.alive = false;
            }
        });
        cx.emit_on(manager, "device_removed", vec![cx.object_value(device)]);
        cx.later_on(device, move |cx| cx.unhold(device));
    }
}

fn make_device(cx: &Cx<'_>, context: Obj, id: &str, name: &str, capabilities: Vec<String>) -> Res<Obj> {
    let device = cx.spawn("device", ())?;
    cx.hold(device)?;
    let bindings = new_collection(cx, BindingTier::Device, name)?;
    with(cx.ctx(), |table| {
        table.devices.insert(
            device,
            InputDevice {
                id: id.to_owned(),
                name: name.to_owned(),
                capabilities,
                alive: true,
                context,
                bindings,
            },
        );
    });
    Ok(device)
}

/// Moves `device` from `from` to `to`: its bindings, and the bindings `to` made for all its devices, start matching the device's events,
/// and `from`'s stop. Its binding-only claim goes with it.
fn move_device(cx: &Cx<'_>, from: Obj, device: Obj, to: Obj) -> Res<()> {
    let path = with(cx.ctx(), |table| {
        table
            .devices
            .get(&device)
            .map(|held| format!("Device#{}.context", held.id))
    })
    .unwrap_or_default();
    let (to_life, to_seat, from_seat, to_name, name) = with(cx.ctx(), |table| {
        let target = table.contexts.get(&to);
        (
            target.map(|held| held.life),
            target.map_or(0, |held| held.seat),
            table.contexts.get(&from).map_or(0, |held| held.seat),
            target.map(|held| held.name.clone()).unwrap_or_default(),
            table
                .devices
                .get(&device)
                .map(|held| held.name.clone())
                .unwrap_or_default(),
        )
    });
    if to_life != Some(Life::Live) {
        return Err(Fail::type_error(format!(
            "{path}: assign a context that is in the input manager's contexts"
        )));
    }
    if to_seat != from_seat {
        return Err(Fail::unsupported(format!(
            "{path}: the compositor attaches \"{name}\" to seat {from_seat}, so it cannot move to context \"{to_name}\""
        )));
    }
    let was_only = release_binding_only(cx, from, &name);
    disown(cx, from, device);
    let moved = own(cx, to, device).and_then(|()| {
        if was_only {
            make_binding_only(cx, to, &name)
        } else {
            Ok(())
        }
    });
    if let Err(fail) = moved {
        disown(cx, to, device);
        let _ = own(cx, from, device);
        if was_only {
            let _ = make_binding_only(cx, from, &name);
        }
        return Err(fail);
    }
    Ok(())
}

/// The context that holds the device and the device's name.
fn holder(cx: &Cx<'_>, device: Obj) -> Res<(Obj, String)> {
    with(cx.ctx(), |table| {
        table
            .devices
            .get(&device)
            .map(|held| (held.context, held.name.clone()))
            .ok_or_else(|| Fail::stale("the device is gone"))
    })
}

fn read<R>(cx: &Cx<'_>, work: impl FnOnce(&InputDevice) -> R) -> Res<R> {
    let me = cx.me();
    with(cx.ctx(), |table| table.devices.get(&me).map(work)).ok_or_else(|| Fail::stale("the device is gone"))
}

/// The `bind` of a device: `only` first makes it binding-only.
fn bind_device(cx: &Cx<'_>, args: &[Val], handler: bool) -> Res<Val> {
    let me = cx.me();
    let (context, name) = holder(cx, me)?;
    let bindings = read(cx, |device| device.bindings)?;
    if wants_only(cx, args.get(2))? {
        make_binding_only(cx, context, &name)?;
    }
    if handler {
        bind_handler(cx, bindings, args)
    } else {
        bind_closure(cx, bindings, args)
    }
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("device")
        .ctor(|_, _| Err(Fail::refused("a device is reported by the seat")))
        .get("id", |_, cx| read(cx, |device| Val::Text(device.id.clone())))
        .get("name", |_, cx| read(cx, |device| Val::Text(device.name.clone())))
        .get("capabilities", |_, cx| {
            read(cx, |device| {
                Val::List(device.capabilities.iter().map(|cap| Val::Text(cap.clone())).collect())
            })
        })
        .get("alive", |_, cx| read(cx, |device| Val::Bool(device.alive)))
        .get("enabled", |_, _| Ok(Val::Bool(true)))
        .set("enabled", |_, cx, value| {
            if value.bool()? {
                return Ok(());
            }
            let id = read(cx, |device| device.id.clone())?;
            Err(Fail::unsupported(format!(
                "Device#{id}.enabled: the compositor cannot disable one device; make it binding-only to keep its keys from clients"
            )))
        })
        .get("context", |_, cx| cx.handout(read(cx, |device| device.context)?))
        .set("context", |_, cx, value| {
            let me = cx.me();
            let (alive, id) = read(cx, |device| (device.alive, device.id.clone()))?;
            if !alive {
                return Err(Fail::stale(format!("Device#{id}.context: the device was unplugged")));
            }
            let to = value.object()?;
            let (from, _) = holder(cx, me)?;
            if from == to {
                return Ok(());
            }
            move_device(cx, from, me, to)
        })
        .get("bindings", |_, cx| cx.handout(read(cx, |device| device.bindings)?))
        .get("binding_only", |_, cx| {
            let me = cx.me();
            let (context, name) = holder(cx, me)?;
            Ok(Val::Bool(with(cx.ctx(), |table| table.is_binding_only(context, &name))))
        })
        .set("binding_only", |_, cx, value| {
            let (context, name) = holder(cx, cx.me())?;
            check_device_name(&name, "Device.binding_only")?;
            if value.bool()? {
                make_binding_only(cx, context, &name)
            } else {
                release_binding_only(cx, context, &name);
                Ok(())
            }
        })
        .method("bind", |_, cx, args| bind_device(cx, args, false))
        .method("bind_binding", |_, cx, args| bind_device(cx, args, true));
}
