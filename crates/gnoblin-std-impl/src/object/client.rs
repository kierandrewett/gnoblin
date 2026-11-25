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

use std::cell::RefCell;
use std::collections::HashMap;

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::core_api;

/// The namespace the core serves its classes in.
const CORE: &str = "gnoblin.model";

/// How many turns look for a window's `Client` before the library stops: the core lists a window from its next batch.
const ARRIVAL_TRIES: u8 = 3;

/// The core's `Client` of each window the library tracks, for one generation.
#[derive(Default)]
struct HeldClients {
    /// Each window's `Client`, by node, held by one reference of the library's: a value that names it carries no reference, and each
    /// receiver takes its own from the core when the value is handed out (a queued signal holds its own until it is delivered).
    clients: HashMap<u64, Obj>,
    /// The node of each `Client` in `clients`, to read a client the shell passes back without asking the core.
    nodes: HashMap<Obj, u64>,
    /// The core's collection of every client, which a window's `Client` is looked up in once, held by one reference.
    every: Option<Obj>,
    /// Windows that closed since the library's last turn ended: still held, so what the turn tells of them (the signals of their
    /// collections, the layout's `client_removed`) names the core's object, frozen with its last values.
    closing: Vec<u64>,
    /// Windows the library tracks and has not looked up yet, with the turns that looked for them. The turn's end looks each one up, so a
    /// window that closes later is told of as the core's object even when no shell read it: once the window closed, the core no longer
    /// lists it, and a lookup then finds nothing.
    arriving: Vec<(u64, u8)>,
}

thread_local! {
    static HELD: RefCell<HashMap<GnoblinCtx, HeldClients>> = RefCell::new(HashMap::new());
}

/// The core's `Client` of the window `node`, as the value that hands it to a shell (`VALUE_FOREIGN_OBJECT`): what every value of the library
/// that names a window is. The value holds no reference: a signal gives one to each listener, and an answer or the arguments of a method
/// the shell implements must go through `Cx::hand_out` (or `Library::hand_out`) before they are handed over.
///
/// It never reads the library's state, so it may be called while the state is borrowed.
///
/// # Errors
///
/// The core has no client of that node, or its object protocol refused.
pub(crate) fn client_value(ctx: GnoblinCtx, node: u64) -> Res<Val> {
    let held = HELD.with(|held| {
        held.borrow()
            .get(&ctx)
            .and_then(|held| held.clients.get(&node).copied())
    });
    let obj = match held {
        Some(obj) => obj,
        None => look_up(ctx, node)?,
    };
    Ok(Val::Foreign {
        namespace: CORE.to_owned(),
        obj,
    })
}

/// Asks the core for the `Client` of `node` and keeps it.
fn look_up(ctx: GnoblinCtx, node: u64) -> Res<Obj> {
    let core = core_api(ctx)?;
    let every = match HELD.with(|held| held.borrow().get(&ctx).and_then(|held| held.every)) {
        Some(every) => every,
        None => {
            let root = core.create(core.class("gnoblin")?, &[])?;
            let clients = core.member(core.class_of(root)?, abi::MEMBER_PROPERTY, "clients");
            let every = clients
                .and_then(|clients| core.get(root, clients))
                .and_then(|every| every.object());
            let _ = core.release(root);
            let every = every?;
            HELD.with(|held| held.borrow_mut().entry(ctx).or_default().every = Some(every));
            every
        }
    };
    let get = core.member(core.class_of(every)?, abi::MEMBER_METHOD, "get")?;
    let obj = match core.call(every, get, &[Val::Text(node.to_string())])? {
        Val::Object { obj, .. } => obj,
        _ => {
            return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("the core has no client {node}")));
        }
    };
    HELD.with(|held| {
        let mut held = held.borrow_mut();
        let held = held.entry(ctx).or_default();
        held.clients.insert(node, obj);
        held.nodes.insert(obj, node);
    });
    Ok(obj)
}

/// The library started tracking the window `node`: its `Client` is looked up when the library's turn ends ([`turn_ended`]).
///
/// It never reads the library's state, so it may be called while the state is borrowed.
pub(crate) fn client_arrived(ctx: GnoblinCtx, node: u64) {
    HELD.with(|held| {
        let mut held = held.borrow_mut();
        let held = held.entry(ctx).or_default();
        if !held.clients.contains_key(&node) && !held.arriving.iter().any(|(one, _)| *one == node) {
            held.arriving.push((node, 0));
        }
    });
}

/// The library forgot the window `node` (it closed): its `Client` is let go when the library's turn ends ([`turn_ended`]).
pub(crate) fn forget_client(ctx: GnoblinCtx, node: u64) {
    HELD.with(|held| {
        if let Some(held) = held.borrow_mut().get_mut(&ctx) {
            held.arriving.retain(|(one, _)| *one != node);
            if held.clients.contains_key(&node) && !held.closing.contains(&node) {
                held.closing.push(node);
            }
        }
    });
}

/// The library's turn ended: everything it tells of a window that closed has been said or queued (a queued signal holds what it
/// names), so the `Client`s of closed windows are let go. The windows that arrived since the last turn are looked up and kept.
pub(crate) fn turn_ended(ctx: GnoblinCtx) {
    let due: Vec<Obj> = HELD.with(|held| {
        let mut held = held.borrow_mut();
        let Some(held) = held.get_mut(&ctx) else {
            return Vec::new();
        };
        let closing = std::mem::take(&mut held.closing);
        closing
            .into_iter()
            .filter_map(|node| held.clients.remove(&node))
            .inspect(|obj| {
                held.nodes.remove(obj);
            })
            .collect()
    });
    look_up_arrivals(ctx);
    if due.is_empty() {
        return;
    }
    // Released outside the borrow: the core may end the object, which tells nothing of this library.
    if let Ok(core) = core_api(ctx) {
        for obj in due {
            let _ = core.release(obj);
        }
    }
}

/// Looks up and keeps the `Client` of each window that arrived and has none yet: at the end of a turn, and once the library has
/// taken stock of the windows already open (a library that opens runs no turn until something changes). A window the core does not
/// list yet is looked for again at the end of the next turns, a few times; a signal that names it later looks it up then anyway.
///
/// It never reads the library's state, so it may be called while the state is borrowed.
pub(crate) fn look_up_arrivals(ctx: GnoblinCtx) {
    let arriving = HELD.with(|held| {
        held.borrow_mut()
            .get_mut(&ctx)
            .map(|held| std::mem::take(&mut held.arriving))
            .unwrap_or_default()
    });
    // Looked up outside the borrow, as `client_value` does.
    let mut again = Vec::new();
    for (node, tries) in arriving {
        let known = HELD.with(|held| {
            held.borrow()
                .get(&ctx)
                .is_some_and(|held| held.clients.contains_key(&node))
        });
        if !known && look_up(ctx, node).is_err() && tries + 1 < ARRIVAL_TRIES {
            again.push((node, tries + 1));
        }
    }
    if !again.is_empty() {
        HELD.with(|held| {
            if let Some(held) = held.borrow_mut().get_mut(&ctx) {
                held.arriving.extend(again);
            }
        });
    }
}

/// The generation retired: what it held went with it.
pub(crate) fn retired(ctx: GnoblinCtx) {
    let gone = HELD.with(|held| held.borrow_mut().remove(&ctx));
    drop(gone);
}

/// The node of a `Client` the library handed out and whose window is open, without asking the core: `None` for a handle it does not hold
/// or one whose window closed.
fn held_node(ctx: GnoblinCtx, obj: Obj) -> Option<u64> {
    HELD.with(|held| {
        let held = held.borrow();
        let held = held.get(&ctx)?;
        let node = *held.nodes.get(&obj)?;
        (!held.closing.contains(&node)).then_some(node)
    })
}

/// The node of the window the client `value` is, for a call of the member `member` (named in the message of a failure).
///
/// # Errors
///
/// The value is not a handle of the core's `Client`, the handle was released, or the client is gone: see the table of this module.
pub(crate) fn client_node(ctx: GnoblinCtx, member: &str, value: &Val) -> Res<u64> {
    let obj = match value {
        Val::Object { obj, .. } => obj,
        Val::Foreign { namespace, obj } if namespace == CORE => obj,
        _ => {
            return Err(Fail::type_error(format!(
                "`{member}` takes a Client, got {}",
                value.kind_name()
            )));
        }
    };
    // A `Client` this library handed out is a window it knows: the common case on a layout's path. Only whether it is alive is asked of
    // the core, which marks a window dead before it tells anyone it closed; the library hears of the close later, so its own record of
    // the window can still say open while a shell's `closed` listener runs.
    if let Some(node) = held_node(ctx, *obj) {
        let core = core_api(ctx)?;
        let alive = core.member(core.class("client")?, abi::MEMBER_PROPERTY, "alive")?;
        if core.get(*obj, alive)?.bool()? {
            return Ok(node);
        }
        return Err(Fail::new(
            abi::E_NO_SUCH_NODE,
            format!("Client#{node}.{member}: the client is gone"),
        ));
    }
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
