//! The Lua surface of the shell's slot link. The link itself, and the claims and answers it carries, live in
//! `gnoblin_tree_client::slot_link`; this module only puts them on the `core` table.
//!
//! An answer may be a passphrase. It is copied out of Lua into bytes that are wiped after the send, and an
//! error built here never carries it.

use std::rc::Rc;

use gnoblin_secret::SecretText;
use gnoblin_tree_client::slot_link::SharedSlots;
use gnoblin_tree_client::tree_mirror::LinkError;
use mlua::{Lua, Table};

use crate::requests::parse_reason;

fn no_link() -> mlua::Error {
    mlua::Error::RuntimeError("no-slot-connection: the shell has no connection to the compositor for slots".to_owned())
}

fn lua_error(error: LinkError) -> mlua::Error {
    mlua::Error::RuntimeError(error.0)
}

/// Puts `slot_claim`, `slot_release`, `request_claim`, `request_answer` and `request_decline` on the `core` table.
/// Each one is a call into the shared link, and fails with the compositor's code when it refuses.
pub(crate) fn install(lua: &Lua, core: &Table, slots: &SharedSlots) -> mlua::Result<()> {
    fn request_id(id: i64) -> mlua::Result<u64> {
        u64::try_from(id).map_err(|_| mlua::Error::RuntimeError("a request id is not negative".to_owned()))
    }

    let shared = Rc::clone(slots);
    core.set(
        "slot_claim",
        lua.create_function(move |_, name: String| {
            let mut link = shared.borrow_mut();
            link.as_mut().ok_or_else(no_link)?.claim_slot(&name).map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "slot_release",
        lua.create_function(move |_, name: String| {
            let mut link = shared.borrow_mut();
            link.as_mut()
                .ok_or_else(no_link)?
                .release_slot(&name)
                .map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "request_claim",
        lua.create_function(move |_, id: i64| {
            let id = request_id(id)?;
            let mut link = shared.borrow_mut();
            link.as_mut().ok_or_else(no_link)?.claim_request(id).map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "request_answer",
        lua.create_function(move |_, (id, answer): (i64, mlua::LuaString)| {
            let id = request_id(id)?;
            // The text is copied out of Lua for the send and wiped when `secret` drops. Lua's own copy lives
            // until its collector frees it, as the lock screen's password does.
            let text = answer
                .to_str()
                .map_err(|_| mlua::Error::RuntimeError("an answer is UTF-8 text".to_owned()))?;
            let secret = SecretText::new(text.to_string());
            let mut link = shared.borrow_mut();
            link.as_mut()
                .ok_or_else(no_link)?
                .answer(id, secret.expose().as_bytes())
                .map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "request_decline",
        lua.create_function(move |_, (id, reason): (i64, Option<String>)| {
            let id = request_id(id)?;
            let reason = reason.as_deref().map(parse_reason).transpose()?;
            let mut link = shared.borrow_mut();
            link.as_mut()
                .ok_or_else(no_link)?
                .decline(id, reason)
                .map_err(lua_error)
        })?,
    )?;
    Ok(())
}
