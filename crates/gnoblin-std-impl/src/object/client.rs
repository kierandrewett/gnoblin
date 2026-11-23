//! A client the shell passes to a class of `gnoblin.std`: a handle of the core's `Client` (`object:gnoblin.model.client`).
//!
//! The object protocol hands such a value to this library as it is: the wire carries no namespace, so the value says only "an object
//! number and the class the sender thinks it is". The library does not trust it. It trusts the type the catalog declares for the
//! parameter, and asks the core, which issued the handle, what the handle is before it uses it.
//!
//! What a call with a client handle answers:
//!
//! | the handle is                                   | the call fails with                                                       |
//! | ----------------------------------------------- | ------------------------------------------------------------------------- |
//! | not an object (text, a number)                  | `E_TYPE`                                                                  |
//! | one the core released                           | `E_STALE`, as a call on any released handle                               |
//! | an object of another class of the core          | `E_TYPE`                                                                  |
//! | a client the compositor no longer manages       | `E_NO_SUCH_NODE`: `Client#<id>.<member>: the client is gone`, as the core |
//! |                                                 | answers a write or a call on that client                                  |
//! | a live client                                   | none: the call proceeds on the window's node                              |
//!
//! A destroyed client that the shell still holds is not stale: the core keeps it as a frozen object so its last values stay readable.
//! It is gone all the same, so a window state or a menu cannot be made for it.

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::{Fail, Res, Val};

use super::core_api;

/// The node of the window the client `value` is, for a call of the member `member` (named in the message of a failure).
///
/// # Errors
///
/// The value is not a handle of the core's `Client`, the handle was released, or the client is gone: see the table of this module.
pub(crate) fn client_node(ctx: GnoblinCtx, member: &str, value: &Val) -> Res<u64> {
    let Val::Object { obj, .. } = value else {
        return Err(Fail::type_error(format!(
            "`{member}` takes a Client, got {}",
            value.kind_name()
        )));
    };
    let core = core_api(ctx)?;
    // The class the core says the handle is, not the one the sender named: a released handle fails here.
    let class = core.class_of(*obj)?;
    let client = core.class("client")?;
    if class != client {
        return Err(Fail::type_error(format!(
            "`{member}` takes a Client, got an object of another class"
        )));
    }
    let read = |name: &str| -> Res<Val> { core.get(*obj, core.member(client, abi::MEMBER_PROPERTY, name)?) };
    let id = read("id")?.text()?.to_owned();
    if !read("alive")?.bool()? {
        return Err(Fail::new(
            abi::E_NO_SUCH_NODE,
            format!("Client#{id}.{member}: the client is gone"),
        ));
    }
    id.parse::<u64>()
        .map_err(|_| Fail::err(format!("the core named a client `{id}`, which is not a node number")))
}

/// [`client_node`] for a client the call may leave out: none is `None`.
///
/// # Errors
///
/// As [`client_node`], for a client that is given.
pub(crate) fn optional_client_node(ctx: GnoblinCtx, member: &str, value: Option<&Val>) -> Res<Option<u64>> {
    match value {
        None | Some(Val::None) => Ok(None),
        Some(value) => client_node(ctx, member, value).map(Some),
    }
}
