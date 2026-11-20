//! `dialogs` and `authentication`: policy over the compositor's own consent, passphrase and authentication dialogs.
//!
//! The compositor draws these dialogs itself; a shell never does. A shell that claims the `dialogs.observe` slot is shown each request for
//! a short window before the dialog opens (250 ms on the compositor's side) and may refuse it by policy. The library holds the slot while
//! any listener could use it, and gives each request back as one of two fixed verdicts: `deny` when a listener declined it and `pass` for
//! everything else, so a request never waits on a shell. A listener that did nothing, one that failed, one that ran out its budget and a
//! shell that is not there all leave the request to the built-in dialog. There is no way to allow a request and none to read or give a
//! secret: the two byte strings `deny` and `pass` are all the library ever sends.
//!
//! A request is offered on the slot connection ([`crate::slots`]), which publishes it as an event. Each request is an object that lives
//! for the observation window: the listeners are told from the turn after it was offered, each in its own call so a listener that fails
//! leaves the others theirs, and the verdict is sent by the call queued after them.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::glue::{self, PerGeneration};
use super::listener::{self, Registered};
use crate::events::{ShellEvent, SlotNotice};
use crate::session;

/// The slot a shell claims to observe the built-in dialogs.
const SLOT: &str = "dialogs.observe";

/// What a request is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Consent,
    Secret,
    Auth,
}

impl RequestKind {
    fn name(self) -> &'static str {
        match self {
            RequestKind::Consent => "consent",
            RequestKind::Secret => "secret",
            RequestKind::Auth => "auth",
        }
    }
}

/// The part of a polkit authentication the compositor shows an observer.
#[derive(Default)]
struct Authentication {
    action_id: String,
    message: String,
    identities: Vec<String>,
}

/// A request the listeners are shown.
pub(super) struct OfferedRequest {
    kind: RequestKind,
    app_id: String,
    subject: String,
    deadline: f64,
    /// The observation window is open: a `decline` is heard while it is.
    open: bool,
    declined: bool,
    reason: String,
    authentication: Authentication,
}

/// What the services keep for a generation.
#[derive(Default)]
struct DialogTable {
    dialogs: Obj,
    authentication: Obj,
    hooked: bool,
    dialog_listeners: Registered,
    authentication_listeners: Registered,
    /// Whether this generation holds the slot.
    claimed: bool,
    /// A claim was refused or there is no connection: it is not tried again, and not said again, until no listener is left.
    gave_up: bool,
    /// The requests offered and not settled yet: the compositor's id and the object the listeners were shown.
    held: BTreeMap<u64, Obj>,
}

thread_local! {
    static DIALOGS: PerGeneration<DialogTable> = PerGeneration::new();
    /// The generations that hold the slot now. The claim belongs to the shell instance and outlives a generation: it is given back when
    /// an observer wants none, and kept through the gap between one generation and the next.
    static OBSERVERS: RefCell<BTreeSet<GnoblinCtx>> = const { RefCell::new(BTreeSet::new()) };
}

/// The text of a JSON string that starts at `at` (the opening quote), and where it ends.
fn json_string(text: &[u8], at: usize) -> Option<(String, usize)> {
    let mut out: Vec<u8> = Vec::new();
    let mut index = at + 1;
    while index < text.len() {
        match text[index] {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), index + 1)),
            b'\\' => {
                index += 1;
                let escaped = *text.get(index)?;
                match escaped {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'u' => {
                        let hex = std::str::from_utf8(text.get(index + 1..index + 5)?).ok()?;
                        let code = u32::from_str_radix(hex, 16).ok()?;
                        let mut buffer = [0_u8; 4];
                        out.extend_from_slice(
                            char::from_u32(code)
                                .unwrap_or('\u{fffd}')
                                .encode_utf8(&mut buffer)
                                .as_bytes(),
                        );
                        index += 4;
                    }
                    other => out.push(other),
                }
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    None
}

/// The index after the value that starts at `at`: a string, a number, a literal, or an object or array balanced to its end.
fn skip_value(text: &[u8], at: usize) -> Option<usize> {
    match *text.get(at)? {
        b'"' => json_string(text, at).map(|(_, end)| end),
        b'{' | b'[' => {
            let mut depth = 0_usize;
            let mut index = at;
            while index < text.len() {
                match text[index] {
                    b'"' => index = json_string(text, index)?.1 - 1,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(index + 1);
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            None
        }
        _ => {
            let mut index = at;
            while index < text.len() && !matches!(text[index], b',' | b'}' | b']') {
                index += 1;
            }
            Some(index)
        }
    }
}

/// The part of an authentication the compositor shows an observer: `{ "actionId", "requester", "identities" }` as JSON. A payload that
/// is not JSON leaves every field empty: the request is still observed, and still refusable.
fn authentication_details(payload: &[u8]) -> (String, String, Vec<String>) {
    let mut action_id = String::new();
    let mut requester = String::new();
    let mut identities = Vec::new();
    let skip_space = |mut at: usize| {
        while payload.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        at
    };
    let mut at = skip_space(0);
    if payload.get(at) != Some(&b'{') {
        return (action_id, requester, identities);
    }
    at += 1;
    loop {
        at = skip_space(at);
        if payload.get(at) != Some(&b'"') {
            break;
        }
        let Some((key, after)) = json_string(payload, at) else {
            break;
        };
        at = skip_space(after);
        if payload.get(at) != Some(&b':') {
            break;
        }
        at = skip_space(at + 1);
        let Some(end) = skip_value(payload, at) else {
            break;
        };
        match key.as_str() {
            "actionId" => action_id = json_string(payload, at).map(|(text, _)| text).unwrap_or_default(),
            "requester" => requester = json_string(payload, at).map(|(text, _)| text).unwrap_or_default(),
            "identities" if payload.get(at) == Some(&b'[') => {
                let mut item = skip_space(at + 1);
                while payload.get(item) == Some(&b'"') {
                    let Some((text, after)) = json_string(payload, item) else {
                        break;
                    };
                    identities.push(text);
                    item = skip_space(after);
                    if payload.get(item) == Some(&b',') {
                        item = skip_space(item + 1);
                    }
                }
            }
            _ => {}
        }
        at = skip_space(end);
        if payload.get(at) == Some(&b',') {
            at += 1;
        } else {
            break;
        }
    }
    (action_id, requester, identities)
}

/// Whether any listener could use the slot.
fn wanted(table: &DialogTable) -> bool {
    !table.dialog_listeners.is_empty() || !table.authentication_listeners.is_empty()
}

/// Claims the slot while a listener could use it and gives it back when none can.
fn sync(cx: &Cx<'_>) {
    let ctx = cx.ctx();
    let (want, claimed, gave_up) =
        DIALOGS.with(|dialogs| dialogs.with(ctx, |table| (wanted(table), table.claimed, table.gave_up)));
    if !want {
        DIALOGS.with(|dialogs| dialogs.with(ctx, |table| table.gave_up = false));
    }
    if want == claimed || (want && gave_up) {
        return;
    }
    if want {
        match session::with(|session| session.slots().map(|slots| slots.claim(SLOT))) {
            Ok(()) => {
                OBSERVERS.with(|observers| observers.borrow_mut().insert(ctx));
                DIALOGS.with(|dialogs| dialogs.with(ctx, |table| table.claimed = true));
            }
            Err(why) => {
                DIALOGS.with(|dialogs| dialogs.with(ctx, |table| table.gave_up = true));
                cx.log(
                    abi::LOG_WARN,
                    &format!("{SLOT} was not claimed, so dialog listeners never run; the built-in dialogs serve every request: {why}"),
                );
            }
        }
    } else {
        DIALOGS.with(|dialogs| dialogs.with(ctx, |table| table.claimed = false));
        // The claim is the shell instance's: another generation that observes the slot keeps it.
        let others = OBSERVERS.with(|observers| {
            let mut observers = observers.borrow_mut();
            observers.remove(&ctx);
            !observers.is_empty()
        });
        if !others {
            session::with(|session| {
                if let Some(slots) = session.slots_if_open() {
                    slots.release(SLOT);
                }
            });
        }
    }
}

/// Listens for what the slot connection offers, once for the generation.
fn hook(cx: &Cx<'_>) -> Res<()> {
    let (ctx, service) = (cx.ctx(), cx.me());
    let fresh = DIALOGS.with(|dialogs| {
        dialogs.with(ctx, |table| {
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
        "dialog requests",
        |event| matches!(event, ShellEvent::Slot(_)),
        move |event| {
            if let ShellEvent::Slot(notice) = event {
                let notice = notice.clone();
                let _ = crate::object::run_in(ctx, service, |cx| slot_event(cx, notice));
            }
        },
    )?;
    Ok(())
}

/// The slot connection said something.
fn slot_event(cx: &Cx<'_>, notice: SlotNotice) {
    let ctx = cx.ctx();
    match notice {
        SlotNotice::Offered {
            request,
            deadline,
            slot,
            title,
            message,
            subject,
        } => {
            // A request of a slot this shell never claimed is not its to answer.
            if slot == SLOT {
                offered(cx, request, deadline, &title, &message, &subject);
            }
        }
        SlotNotice::Finished { request, .. } => {
            // The window ended (answered, refused, timed out): a late `decline` has nothing to refuse.
            let obj = DIALOGS.with(|dialogs| dialogs.with(ctx, |table| table.held.get(&request).copied()));
            if let Some(obj) = obj {
                let _ = cx.with_state::<OfferedRequest, _>(obj, |state| state.open = false);
            }
        }
        SlotNotice::Lost(why) => {
            let held = DIALOGS.with(|dialogs| {
                dialogs.with(ctx, |table| {
                    table.claimed = false;
                    table.gave_up = true;
                    std::mem::take(&mut table.held)
                })
            });
            for obj in held.into_values() {
                let _ = cx.with_state::<OfferedRequest, _>(obj, |state| state.open = false);
                cx.later_on(obj, move |cx| cx.unhold(obj));
            }
            cx.log(
                abi::LOG_WARN,
                &format!("the slot connection is lost ({why}); the built-in dialogs serve every request again"),
            );
        }
        SlotNotice::Refused(why) => {
            DIALOGS.with(|dialogs| {
                dialogs.with(ctx, |table| {
                    table.claimed = false;
                    table.gave_up = true;
                });
            });
            cx.log(
                abi::LOG_WARN,
                &format!("{why}, so dialog listeners never run; the built-in dialogs serve every request"),
            );
        }
    }
}

/// A request was offered: the listeners are shown it, and the verdict goes after them.
fn offered(cx: &Cx<'_>, id: u64, deadline: u64, title: &str, message: &str, subject: &[u8]) {
    let ctx = cx.ctx();
    let mut request = OfferedRequest {
        kind: RequestKind::Consent,
        app_id: String::new(),
        subject: String::new(),
        deadline: deadline as f64,
        open: true,
        declined: false,
        reason: String::new(),
        authentication: Authentication::default(),
    };
    if let Some(interface) = title.strip_prefix("consent:") {
        request.kind = RequestKind::Consent;
        request.subject = interface.to_owned();
        request.app_id = String::from_utf8_lossy(subject).into_owned();
    } else if title == "secret" {
        request.kind = RequestKind::Secret;
        request.subject = message.to_owned();
        request.app_id = String::from_utf8_lossy(subject).into_owned();
    } else if title == "auth" {
        let (action_id, requester, identities) = authentication_details(subject);
        request.kind = RequestKind::Auth;
        // polkit names a process and never an application, so the verified application id is always empty.
        request.subject = requester;
        request.authentication = Authentication {
            action_id,
            message: message.to_owned(),
            identities,
        };
    } else {
        // A request this library does not know how to describe is not refused: it passes.
        answer(id, false);
        return;
    }
    let kind = request.kind;
    let class = if kind == RequestKind::Auth {
        "authentication_request"
    } else {
        "dialog_request"
    };
    let Ok(obj) = cx.spawn(class, request) else {
        answer(id, false);
        return;
    };
    if cx.hold(obj).is_err() {
        answer(id, false);
        return;
    }
    let (dialogs, authentication) = DIALOGS.with(|dialogs| {
        dialogs.with(ctx, |table| {
            table.held.insert(id, obj);
            (table.dialogs, table.authentication)
        })
    });
    let shown = cx.object_value(obj);
    match kind {
        RequestKind::Consent => cx.emit_on(dialogs, "consent_request", vec![shown]),
        RequestKind::Secret => cx.emit_on(dialogs, "secret_request", vec![shown]),
        RequestKind::Auth => cx.emit_on(authentication, "authentication_request", vec![shown]),
    }
    // The settling call is queued last, so it runs last: the queue runs in order.
    cx.later_on(obj, move |cx| settle(cx, id, obj));
}

/// The listeners have been told: the verdict goes back, unless the window ended first.
fn settle(cx: &Cx<'_>, id: u64, obj: Obj) {
    let ctx = cx.ctx();
    let held = DIALOGS.with(|dialogs| dialogs.with(ctx, |table| table.held.remove(&id).is_some()));
    let outcome = cx
        .with_state::<OfferedRequest, _>(obj, |state| {
            let was_open = state.open;
            state.open = false;
            (was_open, state.declined, state.reason.clone())
        })
        .ok();
    cx.later_on(obj, move |cx| cx.unhold(obj));
    if !held {
        return;
    }
    if let Some((true, deny, reason)) = outcome {
        if deny {
            cx.log(
                abi::LOG_INFO,
                &format!("a dialog listener declined request {id} by policy: {reason}"),
            );
        }
        answer(id, deny);
    }
}

/// Sends the verdict: `deny` or `pass`. A request that ended already is not an error.
fn answer(request: u64, deny: bool) {
    session::with(|session| {
        if let Some(slots) = session.slots_if_open() {
            slots.answer(request, deny);
        }
    });
}

/// Adds or removes a listener of a service and brings the slot in step.
fn listen(cx: &Cx<'_>, authentication: bool, listener_val: &Val, add: bool) -> Res<Val> {
    let ctx = cx.ctx();
    let me = cx.me();
    let mut regs = DIALOGS.with(|dialogs| {
        dialogs.with(ctx, |table| {
            std::mem::take(if authentication {
                &mut table.authentication_listeners
            } else {
                &mut table.dialog_listeners
            })
        })
    });
    let done = if add {
        listener::add(cx, me, &mut regs, listener_val).map(|_| ())
    } else {
        listener::remove(cx, &mut regs, listener_val)
    };
    DIALOGS.with(|dialogs| {
        dialogs.with(ctx, |table| {
            if authentication {
                table.authentication_listeners = regs;
            } else {
                table.dialog_listeners = regs;
            }
        });
    });
    done?;
    if add {
        hook(cx)?;
    }
    sync(cx);
    Ok(Val::None)
}

/// The members every request class has.
fn request_members<'a>(class: ClassBuilder<'a, OfferedRequest>) -> ClassBuilder<'a, OfferedRequest> {
    class
        .ctor(|_, _| Err(Fail::refused("a request is offered by the compositor")))
        .get("kind", |state, cx| cx.enum_val("dialog_kind", state.kind.name()))
        .get("app_id", |state, _| Ok(Val::Text(state.app_id.clone())))
        .get("subject", |state, _| Ok(Val::Text(state.subject.clone())))
        .get("deadline", |state, _| Ok(Val::Double(state.deadline)))
        .get("open", |state, _| Ok(Val::Bool(state.open)))
        .method("decline", |state, _, args| {
            // The first call wins. A call after the window has closed does nothing.
            if state.open && !state.declined {
                state.declined = true;
                state.reason = args
                    .first()
                    .filter(|value| !value.is_none())
                    .map_or(Ok("policy"), |reason| reason.text())?
                    .to_owned();
            }
            Ok(Val::None)
        })
}

pub(super) fn register(builder: &mut Builder) {
    let _ = request_members(builder.class::<OfferedRequest>("dialog_request"));
    let _ = request_members(builder.class::<OfferedRequest>("authentication_request"))
        .get("action_id", |state, _| {
            Ok(Val::Text(state.authentication.action_id.clone()))
        })
        .get("message", |state, _| {
            Ok(Val::Text(state.authentication.message.clone()))
        })
        .get("identities", |state, _| {
            Ok(Val::List(
                state.authentication.identities.iter().cloned().map(Val::Text).collect(),
            ))
        });
    let _ = builder
        .class::<()>("dialogs")
        .ctor(|cx, _| {
            let me = cx.me();
            DIALOGS.with(|dialogs| dialogs.with(cx.ctx(), |table| table.dialogs = me));
            Ok(())
        })
        .method("add_listener", |_, cx, args| listen(cx, false, &args[0], true))
        .method("remove_listener", |_, cx, args| listen(cx, false, &args[0], false));
    let _ = builder
        .class::<()>("authentication")
        .ctor(|cx, _| {
            let me = cx.me();
            DIALOGS.with(|dialogs| dialogs.with(cx.ctx(), |table| table.authentication = me));
            Ok(())
        })
        .method("add_listener", |_, cx, args| listen(cx, true, &args[0], true))
        .method("remove_listener", |_, cx, args| listen(cx, true, &args[0], false));
}

/// The generation ended. A request it was shown and has not answered is refused, never passed: the verdict a listener would have given is
/// not known. The claim is kept, so that a generation that takes over finds it held, and the slot connection refuses what nobody answers;
/// a claim that nobody observes for the grace is given back (`SlotTimings::grace`), because the shell that held the policy is gone.
/// A generation that never held the slot (one whose load failed) leaves the claim as it was.
pub(super) fn forget(ctx: GnoblinCtx) {
    let Some(table) = DIALOGS.with(|dialogs| dialogs.take(ctx)) else {
        return;
    };
    let none_left = OBSERVERS.with(|observers| {
        let mut observers = observers.borrow_mut();
        observers.remove(&ctx);
        observers.is_empty()
    });
    session::with(|session| {
        if let Some(slots) = session.slots_if_open() {
            for request in table.held.keys() {
                slots.answer(*request, true);
            }
            if table.claimed && none_left {
                slots.gap(true);
            }
        }
    });
}
