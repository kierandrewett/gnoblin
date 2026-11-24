//! What the pointer and gesture matchers share: the candidates of an input, the events their hooks are given, and how a hook is called.
//!
//! A [`InputHook`] is a candidate as it was when the input began: the binding's action and trigger are copied, so a binding that is removed while its
//! drag runs can still be told that the drag ended, and nothing here reads the binding again.

use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{Obj, Res, Val};

use super::super::values::record;
use super::keys::{applies, bounds_of, call_closure};
use super::trigger::ParsedTrigger;
use super::{BindingAction, seat, with};

/// The most `delta_time` says, in seconds, so a simulation that integrates it is not thrown by a stall.
const MAX_DELTA_TIME: f64 = 0.1;

/// A candidate for an input: one registration of the context that can take it.
#[derive(Clone)]
pub(super) struct InputHook {
    /// The registration of the context.
    pub registration: u64,
    /// The binding's id, for the log.
    pub id: String,
    pub action: BindingAction,
    pub trigger: ParsedTrigger,
    /// The device the binding is for, or empty for every device.
    pub device: String,
    /// The collection of the layout that supplied it.
    pub layout: Option<Obj>,
    pub threshold: f64,
}

impl InputHook {
    /// Whether the trigger is a pointer button's drag.
    pub(super) fn is_drag(&self) -> bool {
        self.trigger.kind == super::trigger::TriggerClass::Pointer && self.trigger.action == Some("drag")
    }

    /// Whether the trigger is a pointer button's click.
    pub(super) fn is_click(&self) -> bool {
        self.trigger.kind == super::trigger::TriggerClass::Pointer && self.trigger.action == Some("click")
    }
}

/// The registrations of `context` that can take an input on the claim `spec_key` of `device` when `client` is the window under it, best
/// first: the tiers, then the priorities, then the older registration. A scope or a layout that does not accept the window leaves one out.
pub(super) fn candidates(
    cx: &Cx<'_>,
    context: Obj,
    spec_key: &str,
    device: &str,
    client: Option<Obj>,
) -> Vec<InputHook> {
    let found = with(cx.ctx(), |table| {
        table
            .candidates(context, spec_key, device)
            .into_iter()
            .filter_map(|registration| {
                let rec = table.bindings.get(&registration.binding)?;
                Some((
                    registration,
                    rec.scope.clone(),
                    rec.id.clone(),
                    rec.action.clone(),
                    rec.parsed.clone(),
                    rec.threshold,
                ))
            })
            .collect::<Vec<_>>()
    });
    found
        .into_iter()
        .filter(|(registration, scope, ..)| applies(cx, registration, scope.as_ref(), client))
        .map(|(registration, _, id, action, trigger, threshold)| InputHook {
            registration: registration.id,
            id,
            action,
            trigger,
            device: registration.device,
            layout: registration.layout,
            threshold,
        })
        .collect()
}

/// Whether the registration of `hook` is still in `context`: a binding removed while an input runs is not asked again.
pub(super) fn registered(cx: &Cx<'_>, context: Obj, hook: &InputHook) -> bool {
    with(cx.ctx(), |table| {
        table
            .contexts
            .get(&context)
            .is_some_and(|held| held.registrations.iter().any(|item| item.id == hook.registration))
    })
}

/// The device of `context` that reports `name`, when the hook is for one.
fn device_named(cx: &Cx<'_>, context: Obj, name: &str) -> Option<Obj> {
    if name.is_empty() {
        return None;
    }
    with(cx.ctx(), |table| {
        table.contexts.get(&context).and_then(|held| {
            held.owned
                .iter()
                .map(|(owned, _)| *owned)
                .find(|owned| table.devices.get(owned).is_some_and(|device| device.name == name))
        })
    })
}

/// What an event is told of the input besides what its hook is.
pub(super) struct Basis<'a> {
    pub context: Obj,
    pub hook: &'a InputHook,
    /// The window under the input.
    pub client: Option<Obj>,
    /// The pointer.
    pub position: (f64, f64),
    /// When the last hook of this input ran, on the clock of the session.
    pub last_time: f64,
}

/// The `binding_event` for a hook of `basis`, with `extra` fields. Answers it and the time it was made.
pub(super) fn event(cx: &Cx<'_>, basis: &Basis<'_>, extra: Vec<(&'static str, Val)>) -> Res<(Val, f64)> {
    let point = |x: f64, y: f64| record(cx, "input_point", vec![("x", Val::Double(x)), ("y", Val::Double(y))]);
    let now = crate::session::with(|session| session.now_ms());
    let device = device_named(cx, basis.context, &basis.hook.device);
    let delta = ((now - basis.last_time) / 1000.0).clamp(0.0, MAX_DELTA_TIME);
    let (x, y) = basis.position;
    let mut fields = vec![
        ("context", cx.object_value(basis.context)),
        ("device", device.map_or(Val::None, |device| cx.object_value(device))),
        ("trigger", Val::Text(basis.hook.trigger.text.clone())),
        ("timestamp", Val::Double(now)),
        ("delta_time", Val::Double(delta)),
        (
            "modifiers",
            Val::List(
                basis
                    .hook
                    .trigger
                    .modifiers
                    .iter()
                    .map(|name| Val::Text((*name).to_owned()))
                    .collect(),
            ),
        ),
        ("position", point(x, y)?),
    ];
    if let Some(client) = basis.client {
        fields.push(("client", cx.object_value(client)));
        if let Some((left, top, ..)) = bounds_of(cx, client) {
            fields.push(("local_position", point(x - left, y - top)?));
        }
    }
    if let Some(layout) = basis.hook.layout {
        fields.push(("layout", cx.object_value(layout)));
    }
    fields.extend(extra);
    Ok((record(cx, "binding_event", fields)?, now))
}

/// The seat's `drag_constraint` and `edge_tiling`, which a drag's events say.
pub(super) fn drag_policy(seat_node: u64) -> (String, bool) {
    (seat::drag_constraint(seat_node), seat::edge_tiling(seat_node))
}

/// What a call of a hook came to.
pub(super) enum HookAnswer {
    /// The hook answered.
    Said(Val),
    /// The binding has no such hook: a function that takes the input at its start has no more to say, and an object may leave a hook out.
    Absent,
    /// The hook threw. It has been logged.
    Threw,
}

/// Calls the hook `member` of the binding with `event`: a function is called whatever the step, an object only when it implements the method.
pub(super) fn call(cx: &Cx<'_>, hook: &InputHook, member: &str, event: Val) -> HookAnswer {
    let outcome = match &hook.action {
        BindingAction::Closure(closure) => call_closure(cx, closure, vec![event]).map(Some),
        BindingAction::Handler(object) | BindingAction::Drag(object) | BindingAction::Gesture(object) => {
            call_method(cx, *object, member, event)
        }
        BindingAction::Swallow => Ok(None),
    };
    match outcome {
        Ok(Some(value)) => HookAnswer::Said(value),
        Ok(None) => HookAnswer::Absent,
        Err(fail) => {
            cx.log(
                abi::LOG_WARN,
                &format!("binding \"{}\".{member} threw: {}", hook.id, fail.message),
            );
            HookAnswer::Threw
        }
    }
}

/// Calls `member` of an object the shell implements. None when it does not implement it.
fn call_method(cx: &Cx<'_>, object: Obj, member: &str, event: Val) -> Res<Option<Val>> {
    let shell = cx.shell(object)?;
    let class = cx.class_of(object)?;
    let method = cx.member_of(class, abi::MEMBER_METHOD, member)?;
    if !shell.has(method) {
        return Ok(None);
    }
    cx.hand_out(&event)?;
    shell.call(method, &[event]).map(Some)
}

/// Whether the hook takes the input: a function or an object that answers `false`, or throws, declines it, and a binding that only swallows
/// the trigger takes it and calls nothing.
pub(super) fn takes(cx: &Cx<'_>, hook: &InputHook, member: &str, event: Val) -> bool {
    match call(cx, hook, member, event) {
        HookAnswer::Said(Val::Bool(false)) | HookAnswer::Threw => false,
        HookAnswer::Said(_) => true,
        HookAnswer::Absent => matches!(hook.action, BindingAction::Swallow),
    }
}
