//! Key chords: a press of a claimed chord runs the highest-precedence binding that does not decline.

use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{Closure, Fail, Obj, Res, Val};

use super::super::values::record;
use super::super::world::ClientState;
use super::trigger::SeatClaim;
use super::{BindingAction, BindingRegistration, ClaimEntry, Matcher, seat, with};
use crate::events::ClaimNotice;

/// Matches the presses of the key chords a context claims. A press keeps nothing in flight, so there is nothing for it to end.
#[derive(Default)]
pub(super) struct KeyMatcher;

impl Matcher for KeyMatcher {
    fn claim_event(&mut self, cx: &Cx<'_>, context: Obj, claim: &ClaimEntry, notice: &ClaimNotice) {
        // A pointer chord's press and release are the pointer messages' start and end: the compositor sends no chord for them.
        if notice.kind == abi::CLAIM_EVENT_CHORD
            && matches!(claim.spec, SeatClaim::Chord { .. })
            && !claim.pointer
            && notice.state == "press"
        {
            press(cx, context, claim);
        }
    }

    fn retire(&mut self, _cx: &Cx<'_>, _context: Obj, _registration: &BindingRegistration) {}

    fn cancel_on(&mut self, _cx: &Cx<'_>, _client: Obj) {}

    fn cancel_all(&mut self, _cx: &Cx<'_>, _context: Obj) {}
}

/// The window with the keyboard focus of the context's seat.
pub(super) fn focused_client(cx: &Cx<'_>, context: Obj) -> Option<Obj> {
    let seat = with(cx.ctx(), |table| {
        table
            .contexts
            .get(&context)
            .filter(|held| held.life == super::Life::Live)
            .map(|held| held.seat)
    })?;
    seat::client_of(cx, seat::focus(seat))
}

/// A press of `claim`: the first binding that has an `on_press` and does not decline handles it. The window with the focus is the one at
/// the press; the candidates are looked at from the turn that follows.
fn press(cx: &Cx<'_>, context: Obj, claim: &ClaimEntry) {
    let client = focused_client(cx, context);
    let (spec_key, device) = (claim.spec_key.clone(), claim.device.clone());
    cx.later(move |cx| run(cx, context, &spec_key, &device, client));
}

/// Calls a closure with `args`, each object in them handed out as a listener's is.
fn call_closure(cx: &Cx<'_>, closure: &Closure, args: Vec<Val>) -> Res<Val> {
    args.iter().for_each(|arg| cx.hand_out(arg));
    closure.call(0, &args)
}

/// Whether the registration and its binding apply to the window `client`.
fn applies(cx: &Cx<'_>, registration: &BindingRegistration, scope: Option<&Closure>, client: Option<Obj>) -> bool {
    let layout = registration.tier == super::BindingTier::Layout;
    if layout && client.is_none() {
        return false;
    }
    if let (true, Some(owns), Some(client)) = (layout, registration.owns.as_ref(), client)
        && !asks(cx, owns, client)
    {
        return false;
    }
    match (scope, client) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(scope), Some(client)) => asks(cx, scope, client),
    }
}

/// What a scope answers for a window: a scope that throws accepts nothing.
fn asks(cx: &Cx<'_>, scope: &Closure, client: Obj) -> bool {
    match call_closure(cx, scope, vec![cx.object_value(client)]) {
        Ok(answer) => answer.bool().unwrap_or(false),
        Err(fail) => {
            cx.log(abi::LOG_WARN, &format!("a binding's scope threw: {}", fail.message));
            false
        }
    }
}

/// Whether the window is alive and not minimised, with its bounds.
pub(super) fn bounds_of(cx: &Cx<'_>, client: Obj) -> Option<(f64, f64, f64, f64, bool)> {
    cx.with_state::<ClientState, _>(client, |state| {
        (
            state.read("x").number(),
            state.read("y").number(),
            state.read("width").number(),
            state.read("height").number(),
            state.alive && !state.read("minimised").flag(),
        )
    })
    .ok()
}

/// The `binding_event` for a binding of `registration` that handles an input at `at`.
fn event_of(cx: &Cx<'_>, context: Obj, registration: &BindingRegistration, client: Option<Obj>) -> Res<Val> {
    let (text, modifiers, device, seat_node) = with(cx.ctx(), |table| {
        let rec = table.bindings.get(&registration.binding);
        let held = table.contexts.get(&context);
        let device = (!registration.device.is_empty())
            .then(|| {
                held.and_then(|held| {
                    held.owned.iter().map(|(owned, _)| *owned).find(|owned| {
                        table
                            .devices
                            .get(owned)
                            .is_some_and(|device| device.name == registration.device)
                    })
                })
            })
            .flatten();
        (
            rec.map(|rec| rec.parsed.text.clone()).unwrap_or_default(),
            rec.map(|rec| rec.parsed.modifiers.clone()).unwrap_or_default(),
            device,
            held.map_or(0, |held| held.seat),
        )
    });
    let (x, y) = seat::pointer(seat_node);
    let point = |x: f64, y: f64| record(cx, "input_point", vec![("x", Val::Double(x)), ("y", Val::Double(y))]);
    let now = crate::session::with(|session| session.now_ms());
    let mut fields = vec![
        ("context", cx.object_value(context)),
        ("device", device.map_or(Val::None, |device| cx.object_value(device))),
        ("trigger", Val::Text(text)),
        ("timestamp", Val::Double(now)),
        ("delta_time", Val::Double(0.0)),
        (
            "modifiers",
            Val::List(modifiers.into_iter().map(|name| Val::Text(name.to_owned())).collect()),
        ),
        ("position", point(x, y)?),
    ];
    if let Some(client) = client {
        fields.push(("client", cx.object_value(client)));
        if let Some((cx0, cy0, ..)) = bounds_of(cx, client) {
            fields.push(("local_position", point(x - cx0, y - cy0)?));
        }
    }
    if let Some(layout) = registration.layout {
        fields.push(("layout", cx.object_value(layout)));
    }
    record(cx, "binding_event", fields)
}

/// Runs the candidates for a press in order until one handles it.
fn run(cx: &Cx<'_>, context: Obj, spec_key: &str, device: &str, client: Option<Obj>) {
    let ctx = cx.ctx();
    let candidates = with(ctx, |table| {
        table
            .candidates(context, spec_key, device)
            .into_iter()
            .filter_map(|registration| {
                let rec = table.bindings.get(&registration.binding)?;
                Some((registration, rec.scope.clone()))
            })
            .collect::<Vec<_>>()
    });
    let eligible: Vec<BindingRegistration> = candidates
        .into_iter()
        .filter(|(registration, scope)| applies(cx, registration, scope.as_ref(), client))
        .map(|(registration, _)| registration)
        .collect();
    for registration in eligible {
        // A binding removed while an earlier one ran is not asked.
        let held = with(ctx, |table| {
            let present = table
                .contexts
                .get(&context)
                .is_some_and(|held| held.registrations.iter().any(|item| item.id == registration.id));
            let rec = table.bindings.get(&registration.binding)?;
            present.then(|| (rec.action.clone(), rec.id.clone()))
        });
        let Some((action, id)) = held else {
            continue;
        };
        let outcome = match action {
            BindingAction::Swallow => return,
            BindingAction::Closure(closure) => match event_of(cx, context, &registration, client) {
                Ok(event) => call_closure(cx, &closure, vec![event]),
                Err(fail) => Err(fail),
            },
            BindingAction::Handler(handler) => call_handler(cx, context, &registration, handler, client),
        };
        // A throw is that binding's answer: it handled the press, badly. Only a `false` declines.
        match outcome {
            Ok(Val::Bool(false)) => {}
            Ok(_) => return,
            Err(fail) => {
                cx.log(
                    abi::LOG_WARN,
                    &format!("binding \"{id}\".on_press threw: {}", fail.message),
                );
                return;
            }
        }
    }
}

/// Calls the `on_press` of an object the shell implements.
fn call_handler(
    cx: &Cx<'_>,
    context: Obj,
    registration: &BindingRegistration,
    handler: Obj,
    client: Option<Obj>,
) -> Res<Val> {
    let shell = cx.shell(handler)?;
    let class = cx.class_of(handler)?;
    let method = cx.member_of(class, abi::MEMBER_METHOD, "on_press")?;
    if !shell.has(method) {
        return Err(Fail::type_error("the handler does not implement `on_press`"));
    }
    let event = event_of(cx, context, registration, client)?;
    cx.hand_out(&event);
    shell.call(method, &[event])
}
