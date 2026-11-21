//! `binding` and `binding_collection`: what a shell binds, and the rules a collection keeps. A collection owns its bindings by id, refuses
//! two with one trigger at one priority, and tells the contexts that follow it when a binding is added, removed or moved to another
//! trigger. A refusal leaves the collection, and every context that follows it, as it was.

use gnoblin_core_abi as abi;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Closure, Fail, Obj, Res, Val};

use super::super::values::fields_of;
use super::trigger::ParsedTrigger;
use super::{
    BindingAction, BindingRec, BindingSet, BindingTier, ContextFollow, InputTable, parse_trigger, trigger_value, with,
};

/// What `bind` takes beside the trigger and the action.
#[derive(Default)]
struct BindOptions {
    id: Option<String>,
    priority: Option<f64>,
    enabled: Option<bool>,
    only: Option<bool>,
}

pub(super) fn wants_only(cx: &Cx<'_>, options: Option<&Val>) -> Res<bool> {
    Ok(options_of(cx, options)?.only == Some(true))
}

fn options_of(cx: &Cx<'_>, options: Option<&Val>) -> Res<BindOptions> {
    let mut read = BindOptions::default();
    let Some(options) = options.filter(|value| !value.is_none()) else {
        return Ok(read);
    };
    for (name, value) in fields_of(cx, "binding_options", options)? {
        if value.is_none() {
            continue;
        }
        match name {
            "id" => read.id = Some(value.text()?.to_owned()),
            "priority" => read.priority = Some(value.double()?),
            "enabled" => read.enabled = Some(value.bool()?),
            "only" => read.only = Some(value.bool()?),
            _ => {}
        }
    }
    Ok(read)
}

/// The state of a `binding` object: what the binding was when it left its collection, which it keeps reading as.
#[derive(Default)]
pub(super) struct BindingState {
    frozen: Option<BindingRec>,
}

impl BindingState {
    /// Reads the binding: from the table while it is in a collection, as it was when it left after.
    fn read<R>(&self, cx: &Cx<'_>, work: impl FnOnce(&BindingRec) -> R) -> Res<R> {
        let me = cx.me();
        let mut work = Some(work);
        let held = with(cx.ctx(), |table| {
            table
                .bindings
                .get(&me)
                .and_then(|rec| work.take().map(|work| work(rec)))
        });
        match (held, work, self.frozen.as_ref()) {
            (Some(value), _, _) => Ok(value),
            (None, Some(work), Some(rec)) => Ok(work(rec)),
            _ => Err(Fail::stale("the binding is gone")),
        }
    }

    /// Edits the binding where it is.
    fn edit<R>(&mut self, cx: &Cx<'_>, work: impl FnOnce(&mut BindingRec) -> R) -> Res<R> {
        let me = cx.me();
        let mut work = Some(work);
        let done = with(cx.ctx(), |table| {
            table
                .bindings
                .get_mut(&me)
                .and_then(|rec| work.take().map(|work| work(rec)))
        });
        match (done, work, self.frozen.as_mut()) {
            (Some(value), _, _) => Ok(value),
            (None, Some(work), Some(rec)) => Ok(work(rec)),
            _ => Err(Fail::stale("the binding is gone")),
        }
    }
}

impl InputTable {
    /// Refuses `trigger` at `priority` for `binding` when another member of `collection` already matches it.
    fn assert_free(&self, collection: Obj, binding: Obj, text: &str, priority: f64, id: &str) -> Res<()> {
        let Some(held) = self.collections.get(&collection) else {
            return Ok(());
        };
        for other in &held.bindings {
            if *other == binding {
                continue;
            }
            if let Some(rec) = self.bindings.get(other)
                && rec.parsed.text == text
                && rec.priority == priority
            {
                return Err(Fail::refused(format!(
                    "binding \"{id}\" conflicts with \"{}\": both match {text} at priority {priority}; give one a different priority or trigger",
                    rec.id
                )));
            }
        }
        Ok(())
    }
}

/// A context that follows a collection, as a binding is registered in it: what `register` needs and the registration the binding has there.
struct FollowPlan {
    follow: u64,
    context: Obj,
    tier: BindingTier,
    owns: Option<Closure>,
    device: String,
    layout: Option<Obj>,
    /// The registration a binding has already, for a binding that moves.
    old: Option<u64>,
}

/// The contexts that follow `held` (the collection `collection`), as plans for registering `binding` in each.
fn plans_of(held: &BindingSet, collection: Obj, binding: Obj) -> Vec<FollowPlan> {
    let layout = (held.tier == BindingTier::Layout).then_some(collection);
    held.follows
        .iter()
        .map(|follow| FollowPlan {
            follow: follow.id,
            context: follow.context,
            tier: follow.tier,
            owns: follow.owns.clone(),
            device: follow.device.clone(),
            layout,
            old: follow
                .held
                .iter()
                .find(|(other, _)| *other == binding)
                .map(|(_, reg)| *reg),
        })
        .collect()
}

/// Makes a collection. A context's own and a device's are held by their owner; one a shell made is held by the library until it ends.
pub(super) fn new_collection(cx: &Cx<'_>, tier: BindingTier, owner: &str) -> Res<Obj> {
    let obj = cx.spawn("binding_collection", ())?;
    cx.hold(obj)?;
    with(cx.ctx(), |table| {
        table.collections.insert(
            obj,
            BindingSet {
                tier,
                owner: owner.to_owned(),
                bindings: Vec::new(),
                follows: Vec::new(),
            },
        );
    });
    Ok(obj)
}

/// What a bind call needs of the holder it binds in, for the messages.
pub(super) fn holder_of(cx: &Cx<'_>, collection: Obj) -> String {
    with(cx.ctx(), |table| {
        table
            .collections
            .get(&collection)
            .map(|held| match (held.tier, held.owner.as_str()) {
                (BindingTier::Device, name) => format!("InputDevice \"{name}\""),
                (BindingTier::ContextRec, name) => format!("InputContext \"{name}\""),
                _ => "BindingCollection".to_owned(),
            })
    })
    .unwrap_or_else(|| "BindingCollection".to_owned())
}

/// Binds `action` to `trigger` in `collection`: the binding is registered in every context that follows the collection, or in none.
pub(super) fn bind_in(
    cx: &Cx<'_>,
    collection: Obj,
    trigger: &str,
    action: BindingAction,
    options: Option<&Val>,
) -> Res<Obj> {
    let what = holder_of(cx, collection);
    let options = options_of(cx, options)?;
    let tier = with(cx.ctx(), |table| {
        table.collections.get(&collection).map(|held| held.tier)
    })
    .ok_or_else(|| Fail::stale("the binding collection is gone"))?;
    if options.only == Some(false) {
        return Err(Fail::type_error(format!(
            "{what}.bind: only must be true to make the device binding-only, got false"
        )));
    }
    if options.only.is_some() && tier != BindingTier::Device {
        return Err(Fail::type_error(format!(
            "{what}.bind: only is for the bind of a device"
        )));
    }
    let parsed = parse_trigger(trigger, &format!("{what}.bind"))?;
    if parsed.kind == super::trigger::TriggerClass::Pointer && parsed.action == Some("drag") {
        return Err(Fail::type_error(format!(
            "{what}.bind: \"{}\" is a drag; a drag needs the hooks of a drag binding, not a function",
            parsed.text
        )));
    }
    let id = match options.id {
        Some(id) => id,
        None => with(cx.ctx(), |table| {
            table.serial += 1;
            format!("bind-{}:{}", table.serial, parsed.text)
        }),
    };
    if id.is_empty() {
        return Err(Fail::type_error("Binding id must be a non-empty string"));
    }
    let priority = options.priority.unwrap_or(0.0);
    if !priority.is_finite() {
        return Err(Fail::type_error(format!(
            "Binding \"{id}\" priority must be a finite number"
        )));
    }
    let rec = BindingRec {
        id,
        written: trigger.to_owned(),
        parsed,
        priority,
        enabled: options.enabled.unwrap_or(true),
        scope: None,
        action,
        collection: Some(collection),
    };
    add(cx, collection, rec)
}

/// Adds a binding to a collection and registers it in the contexts that follow it.
fn add(cx: &Cx<'_>, collection: Obj, rec: BindingRec) -> Res<Obj> {
    let obj = cx.spawn("binding", BindingState::default())?;
    cx.hold(obj)?;
    if let BindingAction::Handler(handler) = &rec.action
        && let Err(fail) = cx.hold(*handler)
    {
        cx.unhold(obj);
        return Err(fail);
    }
    let handler = match &rec.action {
        BindingAction::Handler(handler) => Some(*handler),
        _ => None,
    };
    let (id, text, priority) = (rec.id.clone(), rec.parsed.text.clone(), rec.priority);
    let placed = with(cx.ctx(), |table| -> Res<Vec<FollowPlan>> {
        if table.collections.get(&collection).is_some_and(|held| {
            held.bindings
                .iter()
                .any(|other| table.bindings.get(other).is_some_and(|b| b.id == id))
        }) {
            return Err(Fail::refused(format!(
                "a binding with id \"{id}\" is already in this collection"
            )));
        }
        table.assert_free(collection, obj, &text, priority, &id)?;
        table.bindings.insert(obj, rec);
        let held = table
            .collections
            .get_mut(&collection)
            .ok_or_else(|| Fail::stale("the binding collection is gone"))?;
        held.bindings.push(obj);
        Ok(plans_of(held, collection, obj))
    });
    let follows = match placed {
        Ok(follows) => follows,
        Err(fail) => {
            cx.unhold(obj);
            if let Some(handler) = handler {
                cx.unhold(handler);
            }
            return Err(fail);
        }
    };
    let mut made: Vec<(u64, Obj, u64)> = Vec::new();
    let mut failure = None;
    for plan in follows {
        let (follow, context) = (plan.follow, plan.context);
        let registered = with(cx.ctx(), |table| {
            let reg = table.register(cx, context, obj, plan.tier, plan.owns, plan.layout, &plan.device)?;
            if let Some(held) = table
                .collections
                .get_mut(&collection)
                .and_then(|held| held.follows.iter_mut().find(|item| item.id == follow))
            {
                held.held.push((obj, reg));
            }
            Ok::<u64, Fail>(reg)
        });
        match registered {
            Ok(reg) => made.push((follow, context, reg)),
            Err(fail) => {
                failure = Some(fail);
                break;
            }
        }
    }
    if let Some(fail) = failure {
        // A context that cannot take the binding (the compositor refused its claim) leaves nothing of it behind: all or none.
        with(cx.ctx(), |table| {
            for (follow, context, reg) in &made {
                table.unregister(cx, *context, *reg);
                if let Some(held) = table
                    .collections
                    .get_mut(&collection)
                    .and_then(|held| held.follows.iter_mut().find(|item| item.id == *follow))
                {
                    held.held.retain(|(binding, _)| *binding != obj);
                }
            }
            if let Some(held) = table.collections.get_mut(&collection) {
                held.bindings.retain(|binding| *binding != obj);
            }
            table.bindings.remove(&obj);
        });
        cx.unhold(obj);
        if let Some(handler) = handler {
            cx.unhold(handler);
        }
        return Err(fail);
    }
    cx.emit_on(collection, "added", vec![cx.object_value(obj)]);
    Ok(obj)
}

/// Takes a binding out of its collection and out of every context that follows it. Answers what it was.
pub(super) fn remove_binding(cx: &Cx<'_>, binding: Obj) -> Option<BindingRec> {
    let (rec, collection) = with(cx.ctx(), |table| {
        let collection = table.bindings.get(&binding)?.collection?;
        let mut rec = table.bindings.remove(&binding)?;
        rec.collection = None;
        let mut registrations: Vec<(Obj, u64)> = Vec::new();
        if let Some(held) = table.collections.get_mut(&collection) {
            held.bindings.retain(|other| *other != binding);
            for follow in &mut held.follows {
                follow.held.retain(|(other, reg)| {
                    if *other == binding {
                        registrations.push((follow.context, *reg));
                        false
                    } else {
                        true
                    }
                });
            }
        }
        for (context, reg) in registrations {
            table.unregister(cx, context, reg);
        }
        Some((rec, collection))
    })?;
    cx.emit_on(collection, "removed", vec![cx.object_value(binding)]);
    if let BindingAction::Handler(handler) = &rec.action {
        let handler = *handler;
        cx.later_on(handler, move |cx| cx.unhold(handler));
    }
    cx.later_on(binding, move |cx| cx.unhold(binding));
    Some(rec)
}

/// Gives the binding the trigger `text`: it is refused, and the binding stays, when the text is not a trigger, when another binding of its
/// collection has that trigger at its priority, or when the compositor refuses a claim.
fn retrigger(cx: &Cx<'_>, binding: Obj, text: &str) -> Res<()> {
    let id = with(cx.ctx(), |table| table.bindings.get(&binding).map(|rec| rec.id.clone()));
    let Some(id) = id else {
        return Ok(());
    };
    let parsed = parse_trigger(text, &format!("Binding \"{id}\" trigger"))?;
    let moved = with(
        cx.ctx(),
        |table| -> Res<Option<(String, ParsedTrigger, Vec<FollowPlan>)>> {
            let Some(rec) = table.bindings.get(&binding) else {
                return Ok(None);
            };
            let (collection, priority) = (rec.collection, rec.priority);
            let Some(collection) = collection else {
                return Ok(None);
            };
            table.assert_free(collection, binding, &parsed.text, priority, &id)?;
            let plans = table
                .collections
                .get(&collection)
                .map(|held| plans_of(held, collection, binding))
                .unwrap_or_default();
            let rec = table
                .bindings
                .get_mut(&binding)
                .ok_or_else(|| Fail::stale("the binding is gone"))?;
            let written = std::mem::replace(&mut rec.written, text.to_owned());
            let previous = std::mem::replace(&mut rec.parsed, parsed.clone());
            Ok(Some((written, previous, plans)))
        },
    )?;
    let Some((written, previous, plans)) = moved else {
        return Ok(());
    };
    // The new registrations are made before the old ones are given up, so a claim the two share is kept.
    let mut fresh: Vec<(u64, Obj, u64, Option<u64>)> = Vec::new();
    let mut failure = None;
    for plan in plans {
        let (follow, context, old) = (plan.follow, plan.context, plan.old);
        let made = with(cx.ctx(), |table| {
            table.register(cx, context, binding, plan.tier, plan.owns, plan.layout, &plan.device)
        });
        match made {
            Ok(reg) => fresh.push((follow, context, reg, old)),
            Err(fail) => {
                failure = Some(fail);
                break;
            }
        }
    }
    if let Some(fail) = failure {
        with(cx.ctx(), |table| {
            for (_, context, reg, _) in &fresh {
                table.unregister(cx, *context, *reg);
            }
            if let Some(rec) = table.bindings.get_mut(&binding) {
                rec.written = written;
                rec.parsed = previous;
            }
        });
        return Err(fail);
    }
    with(cx.ctx(), |table| {
        for (follow, context, reg, old) in &fresh {
            if let Some(old) = old {
                table.unregister(cx, *context, *old);
            }
            let collection = table.bindings.get(&binding).and_then(|rec| rec.collection);
            if let Some(held) = collection
                .and_then(|collection| table.collections.get_mut(&collection))
                .and_then(|held| held.follows.iter_mut().find(|item| item.id == *follow))
                && let Some(slot) = held.held.iter_mut().find(|(other, _)| *other == binding)
            {
                slot.1 = *reg;
            }
        }
    });
    Ok(())
}

/// Makes a context follow `source`: every binding in it is registered in the context, and so is each one added later. All or none.
pub(super) fn follow(
    cx: &Cx<'_>,
    context: Obj,
    source: Obj,
    tier: BindingTier,
    owns: Option<Closure>,
    device: &str,
) -> Res<u64> {
    with(cx.ctx(), |table| {
        table.follow_serial += 1;
        let id = table.follow_serial;
        let held = table
            .collections
            .get(&source)
            .ok_or_else(|| Fail::stale("the binding collection is gone"))?;
        let layout = (held.tier == BindingTier::Layout).then_some(source);
        let members = held.bindings.clone();
        let mut mine: Vec<(Obj, u64)> = Vec::new();
        for binding in members {
            match table.register(cx, context, binding, tier, owns.clone(), layout, device) {
                Ok(reg) => mine.push((binding, reg)),
                Err(fail) => {
                    // A binding the seat refuses leaves nothing of this source behind, so the caller sees all or none.
                    for (_, reg) in mine {
                        table.unregister(cx, context, reg);
                    }
                    return Err(fail);
                }
            }
        }
        if let Some(held) = table.collections.get_mut(&source) {
            held.follows.push(ContextFollow {
                id,
                context,
                tier,
                owns,
                device: device.to_owned(),
                held: mine,
            });
        }
        if let Some(held) = table.contexts.get_mut(&context) {
            held.follows.push((source, id));
        }
        Ok(id)
    })
}

/// Stops a context following by the follow `id`: its registrations go, and the claims no one else stands on with them.
pub(super) fn unfollow(cx: &Cx<'_>, context: Obj, id: u64) {
    with(cx.ctx(), |table| {
        let Some(held) = table.contexts.get_mut(&context) else {
            return;
        };
        let Some(at) = held.follows.iter().position(|(_, follow)| *follow == id) else {
            return;
        };
        let (source, _) = held.follows.remove(at);
        let mut registrations = Vec::new();
        if let Some(held) = table.collections.get_mut(&source) {
            held.follows.retain(|follow| {
                if follow.id == id {
                    registrations.extend(follow.held.iter().map(|(_, reg)| *reg));
                    false
                } else {
                    true
                }
            });
        }
        for reg in registrations {
            table.unregister(cx, context, reg);
        }
    });
}

/// The action a call of `bind` gave: its function, or none.
fn closure_action(action: Option<&Val>) -> Res<BindingAction> {
    match action.filter(|value| !value.is_none()) {
        None => Ok(BindingAction::Swallow),
        Some(value) => Ok(BindingAction::Closure(std::rc::Rc::clone(value.closure()?))),
    }
}

/// The `bind` of a collection, a context and a device.
pub(super) fn bind_closure(cx: &Cx<'_>, collection: Obj, args: &[Val]) -> Res<Val> {
    let action = closure_action(args.get(1))?;
    let obj = bind_in(cx, collection, args[0].text()?, action, args.get(2))?;
    cx.handout(obj)
}

/// The `bind_binding` of a collection, a context and a device.
pub(super) fn bind_handler(cx: &Cx<'_>, collection: Obj, args: &[Val]) -> Res<Val> {
    let handler = args[1].object()?;
    let class = cx.class_of(handler)?;
    let method = cx.member_of(class, abi::MEMBER_METHOD, "on_press")?;
    if !cx.shell(handler)?.has(method) {
        return Err(Fail::type_error(
            "bind_binding: the handler does not implement `on_press`",
        ));
    }
    let obj = bind_in(
        cx,
        collection,
        args[0].text()?,
        BindingAction::Handler(handler),
        args.get(2),
    )?;
    cx.handout(obj)
}

/// The binding of `collection` with this id.
fn find(cx: &Cx<'_>, collection: Obj, id: &str) -> Option<Obj> {
    with(cx.ctx(), |table| {
        table
            .collections
            .get(&collection)?
            .bindings
            .iter()
            .copied()
            .find(|binding| table.bindings.get(binding).is_some_and(|rec| rec.id == id))
    })
}

fn freeze(cx: &Cx<'_>, binding: Obj, rec: BindingRec) {
    let _ = cx.with_state::<BindingState, _>(binding, |state| state.frozen = Some(rec));
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<BindingState>("binding")
        .ctor(|_, _| Err(Fail::refused("a binding is made by `bind`")))
        .get("id", |state, cx| state.read(cx, |rec| Val::Text(rec.id.clone())))
        .get("trigger", |state, cx| {
            state.read(cx, |rec| Val::Text(rec.written.clone()))
        })
        .set("trigger", |state, cx, value| {
            let text = value.text()?.to_owned();
            if state.frozen.is_some() {
                let id = state.frozen.as_ref().map(|rec| rec.id.clone()).unwrap_or_default();
                let parsed = parse_trigger(&text, &format!("Binding \"{id}\" trigger"))?;
                if let Some(rec) = state.frozen.as_mut() {
                    rec.written = text;
                    rec.parsed = parsed;
                }
                return Ok(());
            }
            retrigger(cx, cx.me(), &text)
        })
        .get("parsed", |state, cx| {
            let parsed = state.read(cx, |rec| rec.parsed.clone())?;
            trigger_value(cx, &parsed)
        })
        .get("priority", |state, cx| state.read(cx, |rec| Val::Double(rec.priority)))
        .set("priority", |state, cx, value| {
            let priority = value.double()?;
            if !priority.is_finite() {
                return Err(Fail::type_error("Binding priority must be a finite number"));
            }
            state.edit(cx, |rec| rec.priority = priority)
        })
        .get("enabled", |state, cx| state.read(cx, |rec| Val::Bool(rec.enabled)))
        .set("enabled", |state, cx, value| {
            let enabled = value.bool()?;
            state.edit(cx, |rec| rec.enabled = enabled)
        })
        .get("tier", |state, cx| {
            let collection = state.read(cx, |rec| rec.collection)?;
            let tier = collection
                .and_then(|collection| {
                    with(cx.ctx(), |table| {
                        table.collections.get(&collection).map(|held| held.tier)
                    })
                })
                .unwrap_or(BindingTier::ContextRec);
            cx.enum_val("binding_tier", tier.name())
        })
        .get("scope", |state, cx| {
            Ok(state.read(cx, |rec| rec.scope.clone())?.map_or(Val::None, Val::Closure))
        })
        .set("scope", |state, cx, value| {
            let scope = if value.is_none() {
                None
            } else {
                Some(std::rc::Rc::clone(value.closure()?))
            };
            state.edit(cx, |rec| rec.scope = scope)
        })
        .get("alive", |state, cx| {
            state.read(cx, |rec| Val::Bool(rec.collection.is_some()))
        })
        .method("unbind", |state, cx, _| {
            if let Some(rec) = remove_binding(cx, cx.me()) {
                state.frozen = Some(rec);
            }
            Ok(Val::None)
        });
    let _ = builder
        .class::<()>("binding_collection")
        .ctor(|cx, _| {
            // A collection a shell makes is a layout's: it applies to the windows a context's `attach_layout` says.
            let me = cx.me();
            with(cx.ctx(), |table| {
                table.collections.insert(
                    me,
                    BindingSet {
                        tier: BindingTier::Layout,
                        owner: String::new(),
                        bindings: Vec::new(),
                        follows: Vec::new(),
                    },
                );
            });
            Ok(())
        })
        .get("length", |_, cx| {
            let me = cx.me();
            Ok(Val::Uint(with(cx.ctx(), |table| {
                table.collections.get(&me).map_or(0, |held| held.bindings.len()) as u64
            })))
        })
        .get("tier", |_, cx| {
            let me = cx.me();
            let tier = with(cx.ctx(), |table| table.collections.get(&me).map(|held| held.tier));
            cx.enum_val("binding_tier", tier.unwrap_or(BindingTier::Layout).name())
        })
        .get("owner", |_, cx| {
            let me = cx.me();
            Ok(Val::Text(
                with(cx.ctx(), |table| {
                    table.collections.get(&me).map(|held| held.owner.clone())
                })
                .unwrap_or_default(),
            ))
        })
        .method("bind", |_, cx, args| bind_closure(cx, cx.me(), args))
        .method("bind_binding", |_, cx, args| bind_handler(cx, cx.me(), args))
        .method("find", |_, cx, args| match find(cx, cx.me(), args[0].text()?) {
            Some(binding) => cx.handout(binding),
            None => Ok(Val::None),
        })
        .method("has", |_, cx, args| {
            Ok(Val::Bool(find(cx, cx.me(), args[0].text()?).is_some()))
        })
        .method("remove", |_, cx, args| {
            let Some(binding) = find(cx, cx.me(), args[0].text()?) else {
                return Ok(Val::Bool(false));
            };
            if let Some(rec) = remove_binding(cx, binding) {
                freeze(cx, binding, rec);
            }
            Ok(Val::Bool(true))
        })
        .method("clear", |_, cx, _| {
            let me = cx.me();
            let members = with(cx.ctx(), |table| {
                table
                    .collections
                    .get(&me)
                    .map(|held| held.bindings.clone())
                    .unwrap_or_default()
            });
            for binding in members {
                if let Some(rec) = remove_binding(cx, binding) {
                    freeze(cx, binding, rec);
                }
            }
            Ok(Val::None)
        })
        .method("list", |_, cx, _| {
            let me = cx.me();
            let members = with(cx.ctx(), |table| {
                table
                    .collections
                    .get(&me)
                    .map(|held| held.bindings.clone())
                    .unwrap_or_default()
            });
            members
                .into_iter()
                .map(|binding| cx.handout(binding))
                .collect::<Res<Vec<Val>>>()
                .map(Val::List)
        });
}
