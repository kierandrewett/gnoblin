//! What the services of the model share: the events they hear, the timers they set, closures of their own, and the service objects.
//!
//! A service is one object a generation makes on first use (`kind = "service"`), and its state is kept per generation here beside it, in a
//! table of the service's own. What happens outside a call of the shell (a program ended, the compositor answered, a request was offered)
//! arrives as an event of the host's mailbox, on the shell thread, in `dispatch`. A service hears the ones it wants with [`hear`] and does
//! its work as a call about its own object ([`crate::object::run_in`]), so it may queue signals and make objects exactly as a method
//! does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::Cx;
use gnoblin_object::{Callable, ErrorCode, Fail, MemberId, Obj, Res, Val};
use gnoblin_shell_host::generation::{CallbackOutcome, Release};

use crate::events::ShellEvent;
use crate::registry::{self, Subscription};
use crate::session;

/// A closure of the library's own, for a listener it adds to an object (`Cx::listen`).
pub(super) struct Native<F>(pub(super) F);

impl<F: Fn(&[Val]) -> Res<Val>> Callable for Native<F> {
    fn call(&self, _member: MemberId, args: &[Val]) -> Res<Val> {
        (self.0)(args)
    }
}

/// `run` as a closure a listener can be given.
pub(super) fn closure(run: impl Fn(&[Val]) -> Res<Val> + 'static) -> Rc<dyn Callable> {
    Rc::new(Native(run))
}

/// Listens for the events of the host's mailbox that `matches` picks, on the shell thread of the generation `ctx`. `handle` runs for each,
/// in `dispatch`, under the callback budget. The registration is the generation's: it ends with it, or with [`registry::remove_id`].
///
/// # Errors
///
/// The generation cannot hold registrations any more.
pub(super) fn hear(
    ctx: GnoblinCtx,
    what: &'static str,
    matches: impl Fn(&ShellEvent) -> bool + Send + Sync + 'static,
    handle: impl Fn(&ShellEvent) + Send + Sync + 'static,
) -> Res<u64> {
    let subscription = Subscription {
        matches: Box::new(matches),
        call: Arc::new(move |event: &ShellEvent| {
            handle(event);
            CallbackOutcome::Handled
        }),
        cleanup: None,
    };
    registry::subscribe(ctx, what, subscription, std::ptr::null_mut(), None)
        .map_err(|code| Fail::new(code, format!("{what}: the generation cannot hold a registration")))
}

/// Sets a timer that runs `run` once, `ms` milliseconds from now on the session's clock. The number it returns ends the timer with
/// [`cancel`], and a timer that ran is spent.
///
/// # Errors
///
/// The generation cannot hold registrations any more.
pub(super) fn timer(ctx: GnoblinCtx, what: &'static str, ms: u32, run: impl Fn() + Send + Sync + 'static) -> Res<u64> {
    let owner =
        registry::owner_of(ctx, what).map_err(|code| Fail::new(code, format!("{what}: the generation is gone")))?;
    let timer = session::with(|session| session.add_timer(owner, ms, false));
    let reg = Arc::new(AtomicU64::new(0));
    let own = Arc::clone(&reg);
    let cleanup: Release = Box::new(move || session::with(|session| session.remove_timer(timer)));
    let subscription = Subscription {
        matches: Box::new(move |event: &ShellEvent| matches!(event, ShellEvent::Timer(fired) if *fired == timer)),
        call: Arc::new(move |_event: &ShellEvent| {
            // A one-shot is spent: its registration goes once this callback returns.
            registry::remove_id(own.load(Ordering::Acquire));
            run();
            CallbackOutcome::Handled
        }),
        cleanup: Some(cleanup),
    };
    match registry::subscribe(ctx, what, subscription, std::ptr::null_mut(), None) {
        Ok(id) => {
            reg.store(id, Ordering::Release);
            registry::sync_wake();
            Ok(id)
        }
        Err(code) => {
            session::with(|session| session.remove_timer(timer));
            Err(Fail::new(
                code,
                format!("{what}: the generation cannot hold a registration"),
            ))
        }
    }
}

/// Ends a timer made with [`timer`] (or any registration made with [`hear`]). One that ended is ignored.
pub(super) fn cancel(registration: u64) {
    if registration != 0 {
        registry::remove_id(registration);
    }
}

/// The service `class` of the generation `cx` is in, made on first use: the object and the reference `create` took for the caller, which
/// is the shell's when it is handed out and is given back with [`Cx::unhold`] when it is not.
///
/// # Errors
///
/// The class is not a service of the catalog, or its constructor refused.
pub(super) fn service(ctx: GnoblinCtx, class: &str) -> Res<(gnoblin_object::ClassId, Obj)> {
    crate::object::with_lib(|lib| {
        let id = lib.class_id(class)?;
        lib.create(ctx, id, Vec::new()).map(|obj| (id, obj))
    })
}

/// The service `class` as a value for the shell to keep.
///
/// # Errors
///
/// As [`service`].
pub(super) fn service_value(ctx: GnoblinCtx, class: &str) -> Res<Val> {
    service(ctx, class).map(|(class, obj)| Val::Object { class, obj })
}

/// The state a service keeps for each generation. `T` is made with `Default` on first use.
pub(super) struct PerGeneration<T: Default + 'static> {
    table: RefCell<HashMap<GnoblinCtx, T>>,
}

impl<T: Default + 'static> PerGeneration<T> {
    pub(super) fn new() -> Self {
        Self {
            table: RefCell::new(HashMap::new()),
        }
    }

    /// Runs `work` on the generation's state. `work` must not call into the shell or reach the same table again.
    pub(super) fn with<R>(&self, ctx: GnoblinCtx, work: impl FnOnce(&mut T) -> R) -> R {
        work(self.table.borrow_mut().entry(ctx).or_default())
    }

    /// Runs `work` on the generation's state if it has any, without making it: a generation the service was never used in has none.
    /// `work` must not call into the shell or reach the same table again.
    pub(super) fn peek<R>(&self, ctx: GnoblinCtx, work: impl FnOnce(&T) -> R) -> Option<R> {
        self.table.borrow().get(&ctx).map(work)
    }

    /// Takes the generation's state away: the generation ended.
    pub(super) fn take(&self, ctx: GnoblinCtx) -> Option<T> {
        self.table.borrow_mut().remove(&ctx)
    }
}

/// Whether the generation is the shell's running one: a generation that is only staged is still loading its configuration.
pub(super) fn loading(ctx: GnoblinCtx) -> bool {
    !matches!(
        registry::state_of(ctx),
        Some(gnoblin_shell_host::generation::GenerationState::Active)
    )
}

/// A value of the catalog's record `class` from its fields in the catalog's order.
pub(super) fn record(cx: &Cx<'_>, class: &str, fields: Vec<Val>) -> Res<Val> {
    let names = cx.record_fields(class)?;
    if names.len() != fields.len() {
        return Err(Fail::err(format!(
            "the record `{class}` has {} fields, not {}",
            names.len(),
            fields.len()
        )));
    }
    Ok(Val::Record {
        class: cx.class_named(class)?,
        fields,
    })
}

// ---- the compositor's owner connection --------------------------------------------------------------------------------------

/// Registers `done` for the answer that comes under `token`, once: the registration ends when the answer has been told.
pub(super) fn answer_once(
    ctx: GnoblinCtx,
    what: &'static str,
    token: u64,
    done: impl Fn(&crate::events::AnswerNotice) + Send + Sync + 'static,
) -> Res<u64> {
    let reg = Arc::new(AtomicU64::new(0));
    let own = Arc::clone(&reg);
    let subscription = Subscription {
        matches: Box::new(
            move |event: &ShellEvent| matches!(event, ShellEvent::Answer(answer) if answer.token == token),
        ),
        call: Arc::new(move |event: &ShellEvent| {
            registry::remove_id(own.load(Ordering::Acquire));
            if let ShellEvent::Answer(answer) = event {
                done(answer);
            }
            CallbackOutcome::Handled
        }),
        cleanup: None,
    };
    let id = registry::subscribe(ctx, what, subscription, std::ptr::null_mut(), None)
        .map_err(|code| Fail::new(code, format!("{what}: the generation cannot hold a registration")))?;
    reg.store(id, Ordering::Release);
    Ok(id)
}

/// Why a call could not go out: the compositor cannot be reached.
fn unreachable_owner(what: &str, text: impl std::fmt::Display) -> Fail {
    Fail::refused(ErrorCode::Unavailable, format!("{what}: {text}"))
}

/// Calls `method` of the compositor's one `class` object (`session`, `idle`) and tells `done` how it ended: `Ok` when the compositor
/// accepted, or its code (`denied`, `unsupported`, `too-many-inhibitors`, `unavailable`).
///
/// # Errors
///
/// The call could not go out: the host gave the core no owner connection, or it is gone. `done` is not called.
pub(super) fn owner_call(
    ctx: GnoblinCtx,
    what: &'static str,
    class: &str,
    method: &str,
    args: gnoblin_model::Fields,
    done: impl Fn(Result<(), String>) + Send + Sync + 'static,
) -> Res<()> {
    let token = crate::verbs::next_token();
    let reg = answer_once(ctx, what, token, move |answer| {
        done(if answer.error.is_empty() {
            Ok(())
        } else {
            Err(answer.error.clone())
        });
    })?;
    let call = crate::verbs::VerbCall {
        token,
        object: 0,
        class: class.to_owned(),
        method: method.to_owned(),
        args,
    };
    let sent = session::with(|session| {
        session
            .verbs()
            .map_err(|(_, text)| text)
            .and_then(|link| link.call(call))
    });
    sent.map_err(|text| {
        cancel(reg);
        unreachable_owner(what, text)
    })
}

/// Calls a method and does not wait for its answer.
pub(super) fn owner_fire(what: &'static str, class: &str, method: &str, args: gnoblin_model::Fields) -> Res<()> {
    let call = crate::verbs::VerbCall {
        token: crate::verbs::next_token(),
        object: 0,
        class: class.to_owned(),
        method: method.to_owned(),
        args,
    };
    session::with(|session| {
        session
            .verbs()
            .map_err(|(_, text)| text)
            .and_then(|link| link.call(call))
    })
    .map_err(|text| unreachable_owner(what, text))
}

/// Starts an [`crate::verbs::OwnerRequest`] and tells `done` its text, or its code and words.
///
/// # Errors
///
/// The request could not go out. `done` is not called.
pub(super) fn owner_request(
    ctx: GnoblinCtx,
    what: &'static str,
    request: crate::verbs::OwnerRequest,
    done: impl Fn(Result<String, (String, String)>) + Send + Sync + 'static,
) -> Res<()> {
    let token = crate::verbs::next_token();
    let reg = answer_once(ctx, what, token, move |answer| {
        let text = String::from_utf8_lossy(&answer.result).into_owned();
        done(if answer.error.is_empty() {
            Ok(text)
        } else {
            Err((answer.error.clone(), text))
        });
    })?;
    let sent = session::with(|session| {
        session
            .verbs()
            .map_err(|(_, text)| text)
            .and_then(|link| link.request(token, request))
    });
    sent.map_err(|text| {
        cancel(reg);
        unreachable_owner(what, text)
    })
}

/// The last value the compositor published for `name` of its one `class` object, when the owner connection is open.
pub(super) fn owner_property(class: &str, name: &str) -> Option<gnoblin_model::Value> {
    session::with(|session| session.verbs_if_open().and_then(|link| link.property(class, name)))
}
