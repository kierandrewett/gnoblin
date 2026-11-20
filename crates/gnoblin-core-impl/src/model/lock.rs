//! `session_lock`: what a shell's lock screen reads of the compositor's session lock, and what it asks of it.
//!
//! The compositor's tree has one `lock` node. Its `state` is `not-locked`, `pending` or `locked`; the shell records that its lock screen is
//! up in `engaged`, and asks the compositor to check a password with the node's `authenticate` call. The compositor answers later, under the
//! call's token, and unlocks the session itself on an accepted check: nothing here can unlock.
//!
//! The password is the call's secret text. It goes straight into the transaction, which wipes it once it is sent, and is never kept,
//! logged or put in a retry. The screen is alive only while the compositor says so: it renews a lease with the node's `heartbeat` call
//! (at most twice a second), and a check the compositor does not answer within 30 seconds is answered `failed` here, so a lock screen that
//! lost a check takes another attempt instead of waiting for ever.

use std::collections::BTreeMap;

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxAnimation};

use super::glue::{self, PerGeneration};
use super::values::{nodes_of, published, size, tree_class};
use crate::session;

/// The least time between two `heartbeat` calls, in milliseconds. The compositor's lease on the lock screen lasts two seconds, so twice a
/// second is a margin of three lost beats and costs one transaction every half second while the session is locked.
const HEARTBEAT_INTERVAL_MS: f64 = 500.0;

/// How long a password check may go unanswered before the screen is told it failed, in milliseconds. The compositor answers every check it
/// receives, so an answer that has not come by now never will. A PAM check that takes longer than this still unlocks the session when it
/// ends: only the screen's wait is over.
const CHECK_DEADLINE_MS: u32 = 30_000;

/// What a check that went unanswered is answered.
const UNANSWERED: &str = "failed: the compositor did not answer the password check";

/// What the service keeps for a generation.
#[derive(Default)]
struct Table {
    service: Obj,
    /// The checks asked and not answered, by token: the registration that hears the answer and the timer that ends the wait.
    checks: BTreeMap<u64, (u64, u64)>,
    /// When the last `heartbeat` was queued, on the session's clock.
    last_beat: Option<f64>,
}

thread_local! {
    static LOCK: PerGeneration<Table> = PerGeneration::new();
}

/// The tree's lock node, if it has one.
fn node() -> Option<u64> {
    nodes_of("lock").into_iter().next()
}

/// The session lock's state as the compositor last published it.
fn state_name() -> &'static str {
    let Some(node) = node() else {
        return "not-locked";
    };
    match published(node, tree_class("lock"), "state").string().as_str() {
        "pending" => "pending",
        "locked" => "locked",
        _ => "not-locked",
    }
}

/// The size of the screen the lock screen covers: the primary one, else the first.
fn screen_size() -> Option<(f64, f64)> {
    let screens = nodes_of("screen");
    let class = tree_class("screen");
    let screen = screens
        .iter()
        .find(|node| published(**node, class, "primary").flag())
        .or_else(|| screens.first())?;
    Some((
        published(*screen, class, "width").number(),
        published(*screen, class, "height").number(),
    ))
}

/// The answer to a check, told from the turn after it came.
fn answered(cx: &Cx<'_>, token: u64, result: &str, error: &str) {
    let ctx = cx.ctx();
    let waited = LOCK.with(|lock| lock.with(ctx, |table| table.checks.remove(&token)));
    // An answer to another call is not the check's, and one that came after the deadline answered it is not told twice.
    let Some((answer, timer)) = waited else {
        return;
    };
    glue::cancel(answer);
    glue::cancel(timer);
    let answer = glue::record(
        cx,
        "lock_answer",
        vec![
            Val::Uint(token),
            Val::Text(result.to_owned()),
            Val::Text(error.to_owned()),
        ],
    );
    match answer {
        Ok(answer) => cx.emit("answered", vec![answer]),
        Err(fail) => cx.log(
            gnoblin_core_abi::LOG_WARN,
            &format!("a password answer could not be told: {fail}"),
        ),
    }
}

/// `authenticate`.
fn authenticate(cx: &Cx<'_>, password: &str) -> Res<Val> {
    let ctx = cx.ctx();
    let service = cx.me();
    let node = node().ok_or_else(|| Fail::refused("gnoblin.lock.authenticate: the tree has no lock node"))?;
    let token = session::with(|session| {
        session
            .tree()
            .map_err(|(code, text)| Fail::new(code, text))?
            .call(node, "authenticate", Vec::new(), password.to_owned())
            .map_err(|error| {
                Fail::new(
                    gnoblin_core_abi::E_NO_SUCH_NODE,
                    format!("gnoblin.lock.authenticate: {error}"),
                )
            })
    })?;
    let answer = glue::answer_once(ctx, "password answer", token, move |answer| {
        let result = String::from_utf8_lossy(&answer.result).into_owned();
        let error = answer.error.clone();
        let _ = crate::object::run_in(ctx, service, |cx| answered(cx, token, &result, &error));
    })?;
    let timer = glue::timer(ctx, "password deadline", CHECK_DEADLINE_MS, move || {
        let _ = crate::object::run_in(ctx, service, |cx| answered(cx, token, "", UNANSWERED));
    });
    match timer {
        Ok(timer) => LOCK.with(|lock| lock.with(ctx, |table| table.checks.insert(token, (answer, timer)))),
        Err(fail) => {
            glue::cancel(answer);
            return Err(fail);
        }
    };
    Ok(Val::Uint(token))
}

/// `heartbeat`.
fn heartbeat(cx: &Cx<'_>) -> Res<Val> {
    let Some(node) = node() else {
        return Ok(Val::None);
    };
    let now = session::with(|session| session.now_ms());
    let ctx = cx.ctx();
    let recent = LOCK.with(|lock| {
        lock.with(ctx, |table| {
            table
                .last_beat
                .is_some_and(|at| (now - at).abs() < HEARTBEAT_INTERVAL_MS)
        })
    });
    if recent {
        return Ok(Val::None);
    }
    // A call on the lock node that has no answer and changes no node. It is not a secret and may go again, so a refusal that sends it
    // again does no harm; a lost one is made up for by the next.
    let sent = session::with(|session| {
        session
            .tree()
            .map_err(|(code, text)| Fail::new(code, text))?
            .call(node, "heartbeat", Vec::new(), String::new())
            .map_err(|error| {
                Fail::new(
                    gnoblin_core_abi::E_NO_SUCH_NODE,
                    format!("gnoblin.lock.heartbeat: {error}"),
                )
            })
    });
    if sent.is_ok() {
        LOCK.with(|lock| lock.with(ctx, |table| table.last_beat = Some(now)));
    }
    Ok(Val::None)
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("session_lock")
        .ctor(|cx, _| {
            let me = cx.me();
            LOCK.with(|lock| lock.with(cx.ctx(), |table| table.service = me));
            Ok(())
        })
        .get("state", |_, cx| cx.enum_val("lock_condition", state_name()))
        .get("node", |_, _| Ok(Val::Uint(node().unwrap_or(0))))
        .get("screen", |_, cx| match screen_size() {
            Some((width, height)) => size(cx, width, height),
            None => Ok(Val::None),
        })
        .method("engage", |_, _, args| {
            let engaged = args[0].bool()?;
            let node = node().ok_or_else(|| Fail::refused("gnoblin.lock.engage: the tree has no lock node"))?;
            session::with(|session| {
                session
                    .tree()
                    .map_err(|(code, text)| Fail::new(code, text))?
                    .set(node, "engaged", MirrorValue::Bool(engaged), TxAnimation::default())
                    .map_err(|error| {
                        Fail::new(
                            gnoblin_core_abi::E_NO_SUCH_NODE,
                            format!("gnoblin.lock.engage: {error}"),
                        )
                    })
            })?;
            Ok(Val::None)
        })
        .method("authenticate", |_, cx, args| authenticate(cx, args[0].text()?))
        .method("heartbeat", |_, cx, _| heartbeat(cx));
}

/// The generation ended: the checks it waits on are not waited on any more.
pub(super) fn forget(ctx: GnoblinCtx) {
    if let Some(table) = LOCK.with(|lock| lock.take(ctx)) {
        for (answer, timer) in table.checks.into_values() {
            glue::cancel(answer);
            glue::cancel(timer);
        }
    }
}
