//! `gnoblin.session.status()`: the session's own state, read from the mirror.
//!
//! The session object publishes its lock state as three properties. A read answers from
//! the mirror, so it is as of the last pump. `lock_state` is reported only once the
//! session has published one, so a script never reads a stand-in value as a real state.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_client::Transport;
use gnoblin_model::Value;
use mlua::{Error, Lua, Table};

use crate::host::Shared;

/// Install `status` on the `session` class table.
pub(super) fn install_methods<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    class: &Table,
) -> mlua::Result<()> {
    let shared = Rc::clone(shared);
    class.set(
        "status",
        lua.create_function(move |lua, ()| status(lua, &shared.borrow()))?,
    )?;
    Ok(())
}

/// The status table: `state` is always `running`; `lock_available`, `revision` and, when
/// available, `lock_state` come from the session object in the mirror.
fn status<T: Transport>(lua: &Lua, shared: &Shared<T>) -> mlua::Result<Table> {
    let session = shared
        .mirror
        .values()
        .find(|view| view.ty().namespace() == "gnoblin" && view.ty().name() == "session");
    let available = matches!(
        session.and_then(|view| view.property("lock_available")),
        Some(Value::Bool(true))
    );
    let revision = match session.and_then(|view| view.property("lock_revision")) {
        Some(Value::Uint(revision)) => *revision,
        _ => 0,
    };
    let revision = i64::try_from(revision)
        .map_err(|_| Error::runtime("the session's lock revision does not fit a Lua integer"))?;

    let table = lua.create_table()?;
    table.set("state", "running")?;
    table.set("lock_available", available)?;
    table.set("revision", revision)?;
    if let Some(Value::Text(state)) = session
        .filter(|_| available)
        .and_then(|view| view.property("lock_state"))
    {
        table.set("lock_state", state.as_str())?;
    }
    Ok(table)
}
