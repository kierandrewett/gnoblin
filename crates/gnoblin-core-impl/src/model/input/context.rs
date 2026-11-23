//! `input_context`: the bindings of a group of devices and the claims they make on the seat. The compositor reports one for each seat; a
//! shell makes more with a name and adds them to the manager, and they hold the devices of the first seat. Context bindings apply to all the
//! devices held; device bindings only to their device; a layout's to the windows its scope accepts. Pointer and keyboard focus stay the
//! seat's.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Closure, ErrorCode, Fail, Obj, Res, Val};

use super::super::values::record;
use super::binding::{bind_closure, bind_drag, bind_gesture, bind_handler, follow, new_collection, unfollow};
use super::keys::{bounds_of, focused_client};
use super::{BindingTier, ClaimEntry, ContextRec, InputTable, Life, claims, devices, matchers, seat, with};

/// Registers a context that has no seat yet: it collects bindings, and holds no device and makes no claim until it is added.
pub(super) fn register_context(cx: &Cx<'_>, obj: Obj, name: &str) -> Res<()> {
    let own = new_collection(cx, BindingTier::ContextRec, name)?;
    with(cx.ctx(), |table| {
        table.contexts.insert(
            obj,
            ContextRec {
                name: name.to_owned(),
                seat: 0,
                scripted: false,
                life: Life::Detached,
                own,
                claims: Vec::new(),
                registrations: Vec::new(),
                next_registration: 0,
                follows: Vec::new(),
                layouts: Vec::new(),
                roster: Vec::new(),
                owned: Vec::new(),
                only: Vec::new(),
                disposed: false,
            },
        );
    });
    Ok(())
}

/// Puts the context on `seat`: the seat's own context (`scripted` false) makes claims for its bindings at once; one a shell made holds
/// devices and registers its bindings per device.
pub(super) fn attach(cx: &Cx<'_>, context: Obj, seat: u64, scripted: bool) -> Res<()> {
    let own = with(cx.ctx(), |table| {
        table.context(context).map(|held| {
            held.seat = seat;
            held.scripted = scripted;
            held.life = Life::Live;
            held.own
        })
    })?;
    if !scripted {
        follow(cx, context, own, BindingTier::ContextRec, None, "")?;
    }
    Ok(())
}

/// Ends the context: everything in flight ends, every claim is given up, and the context is removed. A claim on a seat that is gone is
/// given up with a call the session ignores, because the seat took its claims with it.
pub(super) fn dispose(cx: &Cx<'_>, context: Obj) {
    let proceed = with(cx.ctx(), |table| {
        table.contexts.get_mut(&context).is_some_and(|held| {
            let first = !held.disposed;
            held.disposed = true;
            first
        })
    });
    if !proceed {
        return;
    }
    matchers(cx.ctx(), |matcher| matcher.cancel_all(cx, context));
    let (follows, layouts, only) = with(cx.ctx(), |table| {
        table.contexts.get_mut(&context).map_or_else(
            || (Vec::new(), Vec::new(), Vec::new()),
            |held| {
                held.owned.clear();
                (
                    held.follows.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
                    std::mem::take(&mut held.layouts),
                    std::mem::take(&mut held.only),
                )
            },
        )
    });
    for id in follows {
        unfollow(cx, context, id);
    }
    for layout in layouts {
        cx.unhold(layout);
    }
    with(cx.ctx(), |table| {
        if let Some(held) = table.contexts.get_mut(&context) {
            held.roster.clear();
            held.life = Life::Removed;
        }
        // Every registration's claim went with it; this is the net for a claim nothing registered against.
        table.drop_claims(context);
    });
    for (_, _, reg) in only {
        crate::registry::remove_id(reg);
    }
}

/// Throws unless the context is in the manager's contexts.
fn live(cx: &Cx<'_>, context: Obj, what: &str) -> Res<(String, u64)> {
    with(cx.ctx(), |table| {
        let held = table.context(context)?;
        match held.life {
            Life::Live => Ok((held.name.clone(), held.seat)),
            Life::Removed => Err(Fail::stale(format!(
                "InputContext \"{}\".{what}: the context was removed from the input manager",
                held.name
            ))),
            Life::Detached => Err(Fail::refused(
                ErrorCode::InvalidState,
                format!(
                    "InputContext \"{}\".{what}: add the context to the input manager first",
                    held.name
                ),
            )),
        }
    })
}

fn own_of(cx: &Cx<'_>, context: Obj) -> Res<Obj> {
    with(cx.ctx(), |table| table.context(context).map(|held| held.own))
}

/// A point record.
fn point(cx: &Cx<'_>, x: f64, y: f64) -> Res<Val> {
    record(cx, "input_point", vec![("x", Val::Double(x)), ("y", Val::Double(y))])
}

/// The topmost window whose bounds hold the point, and the corner of its bounds.
pub(super) fn hit_client(cx: &Cx<'_>, x: f64, y: f64) -> Option<(Obj, f64, f64)> {
    for client in seat::stack(cx).into_iter().rev() {
        let Some((left, top, width, height, shown)) = bounds_of(cx, client) else {
            continue;
        };
        if shown && x >= left && x < left + width && y >= top && y < top + height {
            return Some((client, left, top));
        }
    }
    None
}

/// The topmost window whose bounds hold the point.
fn hit_test(cx: &Cx<'_>, x: f64, y: f64) -> Res<Val> {
    let Some((client, left, top)) = hit_client(cx, x, y) else {
        return Ok(Val::None);
    };
    record(
        cx,
        "input_hit",
        vec![
            ("client", cx.handout(client)?),
            ("local_position", point(cx, x - left, y - top)?),
        ],
    )
}

/// Each registered binding and its rank among the bindings of the same trigger.
fn describe(cx: &Cx<'_>, context: Obj) -> Res<Val> {
    struct Row {
        context: String,
        id: String,
        trigger: String,
        kind: &'static str,
        owner: BindingTier,
        layout: Option<String>,
        device: Option<String>,
        priority: f64,
        enabled: bool,
        rank: u64,
    }
    let rows = with(cx.ctx(), |table| -> Res<Vec<Row>> {
        let held = table.context(context)?;
        let mut groups: Vec<(String, Vec<super::BindingRegistration>)> = Vec::new();
        for registration in &held.registrations {
            match groups.iter_mut().find(|(key, _)| *key == registration.spec_key) {
                Some((_, group)) => group.push(registration.clone()),
                None => groups.push((registration.spec_key.clone(), vec![registration.clone()])),
            }
        }
        let name = held.name.clone();
        let mut rows = Vec::new();
        for (_, mut group) in groups {
            group.sort_by(|a, b| table.precedence(a, b));
            for (index, registration) in group.iter().enumerate() {
                let Some(rec) = table.bindings.get(&registration.binding) else {
                    continue;
                };
                rows.push(Row {
                    context: name.clone(),
                    id: rec.id.clone(),
                    trigger: rec.written.clone(),
                    kind: rec.parsed.kind.name(),
                    owner: registration.tier,
                    layout: registration.layout.map(|layout| {
                        table
                            .collections
                            .get(&layout)
                            .map(|held| held.owner.clone())
                            .filter(|owner| !owner.is_empty())
                            .unwrap_or_else(|| "layout".to_owned())
                    }),
                    device: (!registration.device.is_empty()).then(|| registration.device.clone()),
                    priority: rec.priority,
                    enabled: rec.enabled,
                    rank: index as u64 + 1,
                });
            }
        }
        Ok(rows)
    })?;
    rows.into_iter()
        .map(|row| {
            record(
                cx,
                "binding_info",
                vec![
                    ("context", Val::Text(row.context)),
                    ("id", Val::Text(row.id)),
                    ("trigger", Val::Text(row.trigger)),
                    ("kind", cx.enum_val("trigger_kind", row.kind)?),
                    ("owner", cx.enum_val("binding_tier", row.owner.name())?),
                    ("layout", row.layout.map_or(Val::None, Val::Text)),
                    ("device", row.device.map_or(Val::None, Val::Text)),
                    ("priority", Val::Double(row.priority)),
                    ("enabled", Val::Bool(row.enabled)),
                    ("rank", Val::Uint(row.rank)),
                ],
            )
        })
        .collect::<Res<Vec<Val>>>()
        .map(Val::List)
}

/// The claims the context holds.
fn claims_of(cx: &Cx<'_>, context: Obj) -> Res<Val> {
    let held: Vec<ClaimEntry> = with(cx.ctx(), |table| table.context(context).map(|held| held.claims.clone()))?;
    held.into_iter()
        .map(|claim| {
            record(
                cx,
                "claim_info",
                vec![
                    ("id", Val::Int(claim.id)),
                    ("trigger", Val::Text(claim.spec_key)),
                    ("device", Val::Text(claim.device)),
                    ("pointer", Val::Bool(claim.pointer)),
                    ("refs", Val::Uint(claim.refs as u64)),
                ],
            )
        })
        .collect::<Res<Vec<Val>>>()
        .map(Val::List)
}

impl InputTable {
    /// Whether the devices that report `name` are binding-only on the context.
    pub(super) fn is_binding_only(&self, context: Obj, name: &str) -> bool {
        self.contexts
            .get(&context)
            .is_some_and(|held| held.only.iter().any(|(only, ..)| only == name))
    }
}

/// Makes the devices that report `name` binding-only on the context. Once is enough.
pub(super) fn make_binding_only(cx: &Cx<'_>, context: Obj, name: &str) -> Res<()> {
    check_device_name(name, "InputContext.make_binding_only")?;
    let (context_name, seat_node) = live(cx, context, "make_binding_only")?;
    if with(cx.ctx(), |table| table.is_binding_only(context, name)) {
        return Ok(());
    }
    let (id, reg) = claims::claim_device(cx, seat_node, name).map_err(|(code, text)| {
        Fail::new(
            code,
            format!("InputContext \"{context_name}\".make_binding_only: the compositor refused to make \"{name}\" binding-only ({text})"),
        )
    })?;
    with(cx.ctx(), |table| {
        if let Some(held) = table.contexts.get_mut(&context) {
            held.only.push((name.to_owned(), id, reg));
        }
    });
    Ok(())
}

/// Gives the binding-only claim on `name` up, so its keys reach clients again. False when there was none.
pub(super) fn release_binding_only(cx: &Cx<'_>, context: Obj, name: &str) -> bool {
    let reg = with(cx.ctx(), |table| {
        let held = table.contexts.get_mut(&context)?;
        let at = held.only.iter().position(|(only, ..)| only == name)?;
        Some(held.only.remove(at).2)
    });
    match reg {
        Some(reg) => {
            crate::registry::remove_id(reg);
            true
        }
        None => false,
    }
}

/// A device name is one non-empty line: the compositor matches it exactly against what a device reports.
pub(super) fn check_device_name(name: &str, what: &str) -> Res<()> {
    if name.is_empty() || name.contains('\n') {
        return Err(Fail::type_error(format!(
            "{what}: a device name is a non-empty line of text from devices, got {name:?}"
        )));
    }
    Ok(())
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("input_context")
        .ctor(|cx, inits| {
            let name = match inits.take("name") {
                Some(value) => match &value {
                    Val::Text(name) => name.clone(),
                    other => {
                        return Err(Fail::type_error(format!(
                            "InputContext: the name must be text, got {}",
                            other.kind_name()
                        )));
                    }
                },
                None => return Err(Fail::type_error("InputContext: expects an object with a name")),
            };
            register_context(cx, cx.me(), &name)
        })
        .get("name", |_, cx| {
            let me = cx.me();
            Ok(Val::Text(
                with(cx.ctx(), |table| table.contexts.get(&me).map(|held| held.name.clone())).unwrap_or_default(),
            ))
        })
        .get("id", |_, cx| {
            let me = cx.me();
            Ok(Val::Text(
                with(cx.ctx(), |table| table.contexts.get(&me).map(|held| held.name.clone())).unwrap_or_default(),
            ))
        })
        .get("alive", |_, cx| {
            let me = cx.me();
            Ok(Val::Bool(with(cx.ctx(), |table| {
                table.contexts.get(&me).is_some_and(|held| held.life != Life::Removed)
            })))
        })
        .get("seat", |_, cx| {
            let me = cx.me();
            Ok(Val::Uint(with(cx.ctx(), |table| {
                table.contexts.get(&me).map_or(0, |held| held.seat)
            })))
        })
        .get("scripted", |_, cx| {
            let me = cx.me();
            Ok(Val::Bool(with(cx.ctx(), |table| {
                table.contexts.get(&me).is_some_and(|held| held.scripted)
            })))
        })
        .get("bindings", |_, cx| cx.handout(own_of(cx, cx.me())?))
        .get("devices", |_, cx| devices::held_by(cx, cx.me()))
        .get("pointer", |_, cx| {
            let (_, seat_node) = live(cx, cx.me(), "pointer")?;
            let (x, y) = seat::pointer(seat_node);
            point(cx, x, y)
        })
        .get("focused_client", |_, cx| match focused_client(cx, cx.me()) {
            Some(client) => cx.handout(client),
            None => Ok(Val::None),
        })
        .get("settings", |_, cx| {
            let me = cx.me();
            let seat_node = with(cx.ctx(), |table| table.contexts.get(&me).map_or(0, |held| held.seat));
            seat::settings(cx, seat_node)
        })
        .get("builtin_motion", |_, cx| {
            let me = cx.me();
            let seat_node = with(cx.ctx(), |table| table.contexts.get(&me).map_or(0, |held| held.seat));
            Ok(Val::Bool(seat::builtin_motion(seat_node)))
        })
        .set("builtin_motion", |_, cx, value| {
            let (_, seat_node) = live(cx, cx.me(), "builtin_motion")?;
            seat::set_builtin_motion(seat_node, value.bool()?)
        })
        .method("bind", |_, cx, args| bind_closure(cx, own_of(cx, cx.me())?, args))
        .method("bind_binding", |_, cx, args| {
            bind_handler(cx, own_of(cx, cx.me())?, args)
        })
        .method("bind_drag", |_, cx, args| bind_drag(cx, own_of(cx, cx.me())?, args))
        .method("bind_gesture", |_, cx, args| {
            bind_gesture(cx, own_of(cx, cx.me())?, args)
        })
        .method("is_binding_only", |_, cx, args| {
            let me = cx.me();
            let name = args[0].text()?;
            Ok(Val::Bool(with(cx.ctx(), |table| table.is_binding_only(me, name))))
        })
        .method("make_binding_only", |_, cx, args| {
            make_binding_only(cx, cx.me(), args[0].text()?).map(|()| Val::None)
        })
        .method("release_binding_only", |_, cx, args| {
            Ok(Val::Bool(release_binding_only(cx, cx.me(), args[0].text()?)))
        })
        .method("describe_bindings", |_, cx, _| describe(cx, cx.me()))
        .method("claims", |_, cx, _| claims_of(cx, cx.me()))
        .method("hit_test", |_, cx, args| {
            let fields = super::super::values::fields_of(cx, "input_point", &args[0])?;
            let number = |name: &str| {
                fields
                    .iter()
                    .find(|(field, _)| *field == name)
                    .map_or(Ok(0.0), |(_, value)| value.double())
            };
            hit_test(cx, number("x")?, number("y")?)
        })
        .method("clear_focus", |_, cx, _| {
            let (_, seat_node) = live(cx, cx.me(), "clear_focus")?;
            seat::clear_focus(seat_node).map(|()| Val::None)
        })
        .method("attach_layout", |_, cx, args| {
            let me = cx.me();
            live(cx, me, "attach_layout")?;
            let layout = args[0].object()?;
            let owns: Option<Closure> = match args.get(1).filter(|value| !value.is_none()) {
                Some(value) => Some(std::rc::Rc::clone(value.closure()?)),
                None => None,
            };
            let attached = with(cx.ctx(), |table| {
                table
                    .contexts
                    .get(&me)
                    .is_some_and(|held| held.layouts.contains(&layout))
            });
            if attached {
                return Err(Fail::refused(
                    ErrorCode::InvalidState,
                    "this layout is already attached to the input context",
                ));
            }
            if with(cx.ctx(), |table| table.collections.get(&layout).map(|held| held.tier)) != Some(BindingTier::Layout)
            {
                return Err(Fail::type_error(
                    "attach_layout: the layout must be a binding collection a shell made",
                ));
            }
            follow(cx, me, layout, BindingTier::Layout, owns, "")?;
            cx.hold(layout)?;
            with(cx.ctx(), |table| {
                if let Some(held) = table.contexts.get_mut(&me) {
                    held.layouts.push(layout);
                }
            });
            Ok(Val::None)
        })
        .method("detach_layout", |_, cx, args| {
            let me = cx.me();
            let layout = args[0].object()?;
            let id = with(cx.ctx(), |table| {
                let held = table.contexts.get_mut(&me)?;
                let at = held.layouts.iter().position(|item| *item == layout)?;
                held.layouts.remove(at);
                held.follows
                    .iter()
                    .find(|(source, _)| *source == layout)
                    .map(|(_, id)| *id)
            });
            if let Some(id) = id {
                unfollow(cx, me, id);
                cx.unhold(layout);
            }
            Ok(Val::None)
        });
}
