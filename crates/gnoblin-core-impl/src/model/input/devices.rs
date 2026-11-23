//! `device`: an input device of a seat, and the roster a seat context keeps of the devices the seat reports. A device belongs to one context
//! at a time and carries the bindings that match only its own events; they run before a layout's and the context's.

use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};

use super::binding::{
    bind_closure, bind_drag, bind_gesture, bind_handler, follow, new_collection, unfollow, wants_only,
};
use super::context::{check_device_name, make_binding_only, release_binding_only};
use super::settings::{self, DeviceSetting};
use super::{BindingTier, InputDevice, Life, seat, with};

/// What a device is, from what the seat says it can do. The class it has in the catalog follows: the settings of a kind are the kind's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::model) enum InputDeviceKind {
    /// A device that is none of the others, a lid switch.
    Device,
    Keyboard,
    Mouse,
    Touchpad,
    Touchscreen,
    Tablet,
}

impl InputDeviceKind {
    /// The kind a device with these capabilities (the words of the seat's device list) is: a tablet tool or pad, a touch screen, a
    /// touchpad (it reports gestures), a keyboard (also one with a pointing stick) or a mouse (any other pointer).
    pub(super) fn of(capabilities: &[String]) -> InputDeviceKind {
        let has = |name: &str| capabilities.iter().any(|held| held == name);
        if has("tablet-tool") || has("tablet-pad") {
            InputDeviceKind::Tablet
        } else if has("touch") {
            InputDeviceKind::Touchscreen
        } else if has("gesture") {
            InputDeviceKind::Touchpad
        } else if has("keyboard") {
            InputDeviceKind::Keyboard
        } else if has("pointer") {
            InputDeviceKind::Mouse
        } else {
            InputDeviceKind::Device
        }
    }

    /// The name of its class in the catalog.
    fn class(self) -> &'static str {
        match self {
            InputDeviceKind::Device => "device",
            InputDeviceKind::Keyboard => "keyboard",
            InputDeviceKind::Mouse => "mouse",
            InputDeviceKind::Touchpad => "touchpad",
            InputDeviceKind::Touchscreen => "touchscreen",
            InputDeviceKind::Tablet => "tablet",
        }
    }

    /// The name a refusal gives the class.
    pub(super) fn title(self) -> &'static str {
        match self {
            InputDeviceKind::Device => "Device",
            InputDeviceKind::Keyboard => "Keyboard",
            InputDeviceKind::Mouse => "Mouse",
            InputDeviceKind::Touchpad => "Touchpad",
            InputDeviceKind::Touchscreen => "Touchscreen",
            InputDeviceKind::Tablet => "Tablet",
        }
    }

    /// The group of the document's `input` section that holds the settings of the kind.
    pub(super) fn group(self) -> &'static str {
        match self {
            InputDeviceKind::Keyboard => "keyboard",
            InputDeviceKind::Touchpad => "touchpad",
            _ => "mouse",
        }
    }
}

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
    let kind = InputDeviceKind::of(&capabilities);
    let device = cx.spawn(kind.class(), ())?;
    cx.hold(device)?;
    let bindings = new_collection(cx, BindingTier::Device, name)?;
    with(cx.ctx(), |table| {
        table.devices.insert(
            device,
            InputDevice {
                id: id.to_owned(),
                name: name.to_owned(),
                capabilities,
                kind,
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
            .map(|held| format!("{}#{}.context", held.kind.title(), held.id))
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

/// What a call of `bind` of a device binds.
#[derive(Clone, Copy)]
enum Binds {
    Function,
    Press,
    Drag,
    Gesture,
}

/// The `bind` of a device: `only` first makes it binding-only.
fn bind_device(cx: &Cx<'_>, args: &[Val], binds: Binds) -> Res<Val> {
    let me = cx.me();
    let (context, name) = holder(cx, me)?;
    let bindings = read(cx, |device| device.bindings)?;
    if wants_only(cx, args.get(2))? {
        make_binding_only(cx, context, &name)?;
    }
    match binds {
        Binds::Function => bind_closure(cx, bindings, args),
        Binds::Press => bind_handler(cx, bindings, args),
        Binds::Drag => bind_drag(cx, bindings, args),
        Binds::Gesture => bind_gesture(cx, bindings, args),
    }
}

/// What the device is called in a refusal: its class and id, as `Mouse#2`.
fn title(cx: &Cx<'_>) -> Res<String> {
    read(cx, |device| format!("{}#{}", device.kind.title(), device.id))
}

/// The members every class of a device has.
fn base(class: ClassBuilder<'_, ()>) -> ClassBuilder<'_, ()> {
    class
        .ctor(|_, _| Err(Fail::refused(ErrorCode::Unsupported, "a device is reported by the seat")))
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
            Err(Fail::unsupported(format!(
                "{}.enabled: the compositor cannot disable one device; make it binding-only to keep its keys from clients",
                title(cx)?
            )))
        })
        .get("context", |_, cx| cx.handout(read(cx, |device| device.context)?))
        .set("context", |_, cx, value| {
            let me = cx.me();
            let alive = read(cx, |device| device.alive)?;
            if !alive {
                return Err(Fail::stale(format!("{}.context: the device was unplugged", title(cx)?)));
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
        .method("bind", |_, cx, args| bind_device(cx, args, Binds::Function))
        .method("bind_binding", |_, cx, args| bind_device(cx, args, Binds::Press))
        .method("bind_drag", |_, cx, args| bind_device(cx, args, Binds::Drag))
        .method("bind_gesture", |_, cx, args| bind_device(cx, args, Binds::Gesture))
}

/// The property `name` of a class of device, which is `setting` of the ledger: the setting is the kind's, not the device's.
fn ledger_property<'a>(
    class: ClassBuilder<'a, ()>,
    name: &'static str,
    setting: DeviceSetting,
) -> ClassBuilder<'a, ()> {
    class
        .get(name, move |_, cx| settings::read(cx, setting))
        .set(name, move |_, cx, value| settings::write(cx, cx.me(), setting, &value))
}

/// The `reset` of a class of device: takes the setting `name` back to the system's, for a name the class has.
fn resettable<'a>(
    class: ClassBuilder<'a, ()>,
    names: &'static [(&'static str, DeviceSetting)],
) -> ClassBuilder<'a, ()> {
    class.method("reset", move |_, cx, args| {
        let name = args[0].text()?;
        let Some((_, setting)) = names.iter().find(|(known, _)| *known == name) else {
            return Err(Fail::type_error(format!("{}.reset: no setting `{name}`", title(cx)?)));
        };
        settings::write(cx, cx.me(), *setting, &Val::None).map(|()| Val::None)
    })
}

pub(super) fn register(builder: &mut Builder) {
    let _ = base(builder.class::<()>("device"));
    let keyboard = ledger_property(
        base(builder.class::<()>("keyboard")),
        "repeat_rate",
        DeviceSetting::RepeatRate,
    );
    let keyboard = ledger_property(keyboard, "repeat_delay", DeviceSetting::RepeatDelay);
    let _ = resettable(
        keyboard,
        &[
            ("repeat_rate", DeviceSetting::RepeatRate),
            ("repeat_delay", DeviceSetting::RepeatDelay),
        ],
    );
    let mouse = ledger_property(
        base(builder.class::<()>("mouse")),
        "sensitivity",
        DeviceSetting::MouseSensitivity,
    );
    let mouse = ledger_property(mouse, "acceleration", DeviceSetting::MouseAcceleration);
    let mouse = ledger_property(mouse, "natural_scroll", DeviceSetting::MouseNaturalScroll);
    let _ = resettable(
        mouse,
        &[
            ("sensitivity", DeviceSetting::MouseSensitivity),
            ("acceleration", DeviceSetting::MouseAcceleration),
            ("natural_scroll", DeviceSetting::MouseNaturalScroll),
        ],
    );
    let touchpad = ledger_property(
        base(builder.class::<()>("touchpad")),
        "sensitivity",
        DeviceSetting::TouchpadSensitivity,
    );
    let touchpad = ledger_property(touchpad, "acceleration", DeviceSetting::TouchpadAcceleration);
    let touchpad = ledger_property(touchpad, "natural_scroll", DeviceSetting::TouchpadNaturalScroll);
    let touchpad = ledger_property(touchpad, "tap_to_click", DeviceSetting::TapToClick);
    let _ = resettable(
        touchpad,
        &[
            ("sensitivity", DeviceSetting::TouchpadSensitivity),
            ("acceleration", DeviceSetting::TouchpadAcceleration),
            ("natural_scroll", DeviceSetting::TouchpadNaturalScroll),
            ("tap_to_click", DeviceSetting::TapToClick),
        ],
    );
    let _ = base(builder.class::<()>("touchscreen"));
    let _ = base(builder.class::<()>("tablet"));
}
