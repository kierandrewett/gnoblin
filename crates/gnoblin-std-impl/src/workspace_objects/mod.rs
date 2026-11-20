//! The classes of `workspace.toml`: workspaces and the list of them, the desks and their policies, the clients a workspace holds and
//! what the library remembers about each, the `layout` interface a shell implements, the floating layout and the built-in ones.
//!
//! `data/js/gnoblin-std/host/{workspace,workspaces,membership,desks,client_state,show}.ts` and `workspace.ts` are the reference. The
//! object model behind them was ported before the object protocol existed (`workspace.rs`, `desks.rs`, `collection.rs`,
//! `client_state.rs`, `layout.rs`, `builtin/`), so the std table's `workspace_*`, `collection_*`, `client_*` and `policy_*` functions
//! (which the TypeScript library still calls) and these classes see one set of workspaces, one list of clients and one layout pass.
//! What this module adds is the objects: a name for each of them that a shell holds, and the signals the library tells them.
//!
//! A workspace object is a name for a workspace the library keeps; the library keeps the workspaces of the list whether a shell holds
//! them or not. Asking again for the same workspace gives the same object. A layout is an object of its own that a workspace holds:
//! a layout of the library's (the floating one, or a built-in kind) is an object of a class that extends the interface `layout`, and
//! one a shell implements is an object of the interface itself, which the library calls from its layout pass (`shell_layout.rs`).
//!
//! The library starts the desktop when one of these classes is used (`events::open`): a workspace is where its windows are, so the
//! default workspace, the adoption of windows and the window policy come with the first call, as they come with `open` of the table.

pub(crate) mod clients;
pub(crate) mod layouts;
pub(crate) mod shell_layout;
mod stock;
mod values;
mod workspaces;

use std::cell::RefCell;
use std::collections::HashMap;

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::Builder;
use gnoblin_object::{Obj, Res};

use crate::error::Res as StdRes;
use crate::object::with_lib;
use crate::state::{Id, Notice, State, Std};

pub(crate) use values::{NOTICE_HIDDEN, NOTICE_SHOWN};

thread_local! {
    /// The object of each workspace a shell was given: one for each, so asking for a workspace again gives the one object.
    static WORKSPACE_OBJECTS: RefCell<HashMap<(GnoblinCtx, Id), Obj>> = RefCell::new(HashMap::new());
    /// The object of each collection of clients a shell was given, to tell it who joined and left.
    static COLLECTION_OBJECTS: RefCell<HashMap<(GnoblinCtx, Id), Obj>> = RefCell::new(HashMap::new());
    /// The object of the list of workspaces, once a shell has one.
    static LIST_OBJECTS: RefCell<HashMap<GnoblinCtx, Obj>> = RefCell::new(HashMap::new());
}

/// Runs `work` over the library's state as its desktop, for a change.
pub(super) fn edit<R>(ctx: GnoblinCtx, work: impl FnOnce(&mut State) -> StdRes<R>) -> Res<R> {
    crate::events::open(ctx).map_err(gnoblin_object::Fail::from)?;
    crate::table::mutate(ctx, work).map_err(gnoblin_object::Fail::from)
}

/// Runs `work` over the library's state as its desktop, to read.
pub(super) fn look<R>(ctx: GnoblinCtx, work: impl FnOnce(&State) -> StdRes<R>) -> Res<R> {
    crate::events::open(ctx).map_err(gnoblin_object::Fail::from)?;
    crate::table::read(ctx, work).map_err(gnoblin_object::Fail::from)
}

/// Tells the objects of the library what a notice says happened, from the next turn: a client joined or left a collection or a workspace,
/// a workspace joined or left the list, a workspace was shown on an output or stopped being shown. It runs outside any borrow of the state,
/// as the delivery of the notice to the table's listeners does.
pub(crate) fn announce(std: &Std, notice: &Notice) {
    let (ctx, told) = {
        let st = std.st.borrow();
        (st.ctx, values::told_by(&st, notice))
    };
    for (home, signal, fields) in told {
        let obj = match home {
            values::SignalHome::Workspace(id) => WORKSPACE_OBJECTS.with(|held| held.borrow().get(&(ctx, id)).copied()),
            values::SignalHome::Collection(id) => {
                COLLECTION_OBJECTS.with(|held| held.borrow().get(&(ctx, id)).copied())
            }
            values::SignalHome::List => LIST_OBJECTS.with(|held| held.borrow().get(&ctx).copied()),
        };
        let Some(obj) = obj else { continue };
        let _ = with_lib(|lib| {
            lib.with_cx(ctx, obj, |cx| {
                let fields = fields
                    .into_iter()
                    .map(|field| match field {
                        values::SignalField::Plain(value) => value,
                        values::SignalField::Workspace(id) => workspaces::plain_workspace(cx, id),
                    })
                    .collect();
                cx.emit(signal, fields);
            })
        });
    }
}

/// The generation ended: the objects it held are gone.
pub(crate) fn retired(ctx: GnoblinCtx) {
    WORKSPACE_OBJECTS.with(|held| held.borrow_mut().retain(|(owner, _), _| *owner != ctx));
    COLLECTION_OBJECTS.with(|held| held.borrow_mut().retain(|(owner, _), _| *owner != ctx));
    LIST_OBJECTS.with(|held| held.borrow_mut().remove(&ctx));
    layouts::retired(ctx);
}

/// Implements the classes of the workspace module.
pub(crate) fn register(builder: &mut Builder) {
    workspaces::register(builder);
    clients::register(builder);
    layouts::register(builder);
    stock::register(builder);
}
