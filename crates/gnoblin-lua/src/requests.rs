//! The `gnoblin.request` class: the offered, claimed and answered exchange as scripts see it.
//!
//! A request is not a mirror object. It has no properties a script reads, and its live
//! state belongs to the compositor's broker, so its three members are class functions
//! that take the request's identity. Its two signals are built from the event the client
//! delivered, not from the graph.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_client::{ClientError, Transport};
use gnoblin_model::{DeclineReason, RequestCompletion};
use gnoblin_types::{RequestId, Tick};
use mlua::{Error, Lua, Table};

use crate::host::Shared;

/// The table a `gnoblin.request.offered` handler receives.
///
/// `subject` is opaque bytes the offering client chose, so it reaches the handler as
/// a Lua string without interpretation.
pub(super) fn offered_table(
    lua: &Lua,
    request: RequestId,
    deadline: Tick,
    slot: &str,
    title: &str,
    message: &str,
    subject: &[u8],
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("name", "gnoblin.request.offered")?;
    table.set("request", request.get())?;
    table.set("deadline", deadline.get())?;
    table.set("slot", slot)?;
    table.set("title", title)?;
    table.set("message", message)?;
    table.set("subject", lua.create_string(subject)?)?;
    Ok(table)
}

/// The table a `gnoblin.request.finished` handler receives.
///
/// The answer is never in it: only the host that offered the request reads that, and a
/// script learns the outcome from `completion` alone. A decline's reason is empty for
/// every other completion.
pub(super) fn finished_table(lua: &Lua, request: RequestId, completion: &RequestCompletion) -> mlua::Result<Table> {
    let (name, reason) = match completion {
        RequestCompletion::Answered => ("answered", ""),
        RequestCompletion::Ended => ("ended", ""),
        RequestCompletion::Refused(reason) => ("refused", reason_name(*reason)),
        RequestCompletion::TimedOut => ("timed-out", ""),
    };
    let table = lua.create_table()?;
    table.set("name", "gnoblin.request.finished")?;
    table.set("request", request.get())?;
    table.set("completion", name)?;
    table.set("reason", reason)?;
    Ok(table)
}

/// Add `claim`, `answer` and `decline` to the `request` class table.
///
/// Each call goes straight to the compositor, which owns the broker. A refusal, such as
/// another owner holding the request, surfaces as a script error carrying the schema's
/// code, so a script can tell `request-taken` from `request-finished`.
pub(super) fn install_methods<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    class: &Table,
) -> mlua::Result<()> {
    {
        let shared = Rc::clone(shared);
        class.set(
            "claim",
            lua.create_function(move |_, id: i64| {
                let request = request_id(id)?;
                shared.borrow_mut().client.claim(request).map_err(refused)
            })?,
        )?;
    }
    {
        let shared = Rc::clone(shared);
        class.set(
            "answer",
            lua.create_function(move |_, (id, answer): (i64, mlua::LuaString)| {
                let request = request_id(id)?;
                shared
                    .borrow_mut()
                    .client
                    .answer(request, &answer.as_bytes())
                    .map_err(refused)
            })?,
        )?;
    }
    {
        let shared = Rc::clone(shared);
        class.set(
            "decline",
            lua.create_function(move |_, (id, reason): (i64, Option<String>)| {
                let request = request_id(id)?;
                let reason = reason.as_deref().map(parse_reason).transpose()?;
                shared
                    .borrow_mut()
                    .client
                    .decline_with(request, reason)
                    .map_err(refused)
            })?,
        )?;
    }
    Ok(())
}

/// The request identity a script passed, which is never negative.
fn request_id(id: i64) -> mlua::Result<RequestId> {
    u64::try_from(id)
        .map(RequestId::new)
        .map_err(|_| Error::runtime(format!("a request id is never negative, found {id}")))
}

/// The wire name of a decline reason, as the `finished` signal carries it.
fn reason_name(reason: DeclineReason) -> &'static str {
    match reason {
        DeclineReason::Locked => "locked",
        DeclineReason::Denied => "denied",
        DeclineReason::Unavailable => "unavailable",
    }
}

/// Read a decline reason a script named. An unknown name is refused, not treated as no
/// reason, so a typo cannot silently turn a locked refusal into a plain one.
fn parse_reason(name: &str) -> mlua::Result<DeclineReason> {
    match name {
        "locked" => Ok(DeclineReason::Locked),
        "denied" => Ok(DeclineReason::Denied),
        "unavailable" => Ok(DeclineReason::Unavailable),
        other => Err(Error::runtime(format!(
            "a decline reason is `locked`, `denied` or `unavailable`, found `{other}`"
        ))),
    }
}

/// The script-visible error for a refused client call, keeping the compositor's code.
fn refused(error: ClientError) -> Error {
    Error::runtime(error.to_string())
}
