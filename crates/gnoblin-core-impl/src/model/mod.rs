//! The object model of `gnoblin-core`: windows, outputs, collections and their listeners, as the classes of `gnoblin.model`.
//!
//! `data/js/gnoblin-core/host/*` is the behaviour it ports:
//!
//! - [`world`]: the model of one generation, and how a batch of what the compositor sent becomes events;
//! - [`collection`]: `client_collection` and `output_collection`, with `where`, listeners and the lists a shell keeps;
//! - [`client`] and [`output`]: a window and an output as handles over tree nodes, readable after they are gone;
//! - [`gnoblin`]: the service that holds the roots, `find`, the timers and the focus history;
//! - [`operation`]: the request an output's `apply` makes, and the timer;
//! - [`properties`]: bindings that follow the properties a function reads;
//! - [`listener`]: what `add_listener` does with a listener the shell implements;
//! - [`input`]: the input matcher, its contexts, devices, bindings and claims;
//! - the services of the session, each one object a generation makes on first use and a module of its own: [`session`] (the session, the
//!   configuration reload and the build identity), [`process`], [`idle`], [`launches`], [`dialogs`] (and the polkit view of them),
//!   [`lock`] (what a lock screen reads of the session lock), [`virtual_output`] and [`inset`]; [`glue`] is what they share.
//!
//! The model of a generation is made on first use and told what the compositor sent by [`pumped`], which the dispatch loop calls once
//! for each pump of the link. Nothing a listener does happens inside the call or the pump that caused it: the library queues every
//! event, and the turn that follows tells the listeners in the order they were queued.

use std::cell::RefCell;
use std::collections::HashMap;

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Obj, Res};

use crate::events::ShellEvent;

mod client;
mod collection;
mod dialogs;
mod glue;
mod gnoblin;
mod idle;
mod input;
mod inset;
mod launches;
mod listener;
mod lock;
mod operation;
mod output;
mod process;
mod properties;
mod session;
mod values;
mod virtual_output;
mod world;

use world::{PumpBatch, World};

thread_local! {
    /// The model of each generation that uses it: one per generation, on the shell thread.
    static WORLDS: RefCell<HashMap<GnoblinCtx, World>> = RefCell::new(HashMap::new());
}

/// Implements the classes of the catalog.
pub(crate) fn register(builder: &mut Builder) {
    collection::register(builder);
    client::register(builder);
    output::register(builder);
    gnoblin::register(builder);
    operation::register(builder);
    properties::register(builder);
    session::register(builder);
    process::register(builder);
    idle::register(builder);
    launches::register(builder);
    input::register(builder);
    dialogs::register(builder);
    lock::register(builder);
    virtual_output::register(builder);
    inset::register(builder);
}

/// Makes the model of the generation of `cx` when it has none.
fn ensure(cx: &Cx<'_>) -> Res<()> {
    let ctx = cx.ctx();
    if WORLDS.with(|worlds| worlds.borrow().contains_key(&ctx)) {
        return Ok(());
    }
    let world = World::new(cx)?;
    WORLDS.with(|worlds| worlds.borrow_mut().insert(ctx, world));
    Ok(())
}

/// Runs `work` on the model of the generation of `cx`, making it first. `work` must not run anything that reaches the model again:
/// it holds it.
fn with_world<R>(cx: &Cx<'_>, work: impl FnOnce(&mut World) -> R) -> Res<R> {
    ensure(cx)?;
    let ctx = cx.ctx();
    let mut world = WORLDS
        .with(|worlds| worlds.borrow_mut().remove(&ctx))
        .ok_or_else(|| gnoblin_object::Fail::err("the model of the generation is in use"))?;
    let outcome = work(&mut world);
    WORLDS.with(|worlds| worlds.borrow_mut().insert(ctx, world));
    world_booted(cx, ctx);
    Ok(outcome)
}

/// Boots the model once the tree is there, so a call that reads it finds the windows the compositor already had.
fn world_booted(cx: &Cx<'_>, ctx: GnoblinCtx) {
    let booted = WORLDS.with(|worlds| worlds.borrow().get(&ctx).is_none_or(|world| world.booted));
    if booted || !values::attached(cx) {
        return;
    }
    let Some(mut world) = WORLDS.with(|worlds| worlds.borrow_mut().remove(&ctx)) else {
        return;
    };
    let _ = world.boot(cx);
    WORLDS.with(|worlds| worlds.borrow_mut().insert(ctx, world));
}

/// What a pump of the tree link brought: the model reads it as one batch and queues the events it implies.
pub(crate) fn pumped(ctx: GnoblinCtx, events: &[ShellEvent]) {
    pumped_world(ctx, events);
    input::pumped(ctx);
}

fn pumped_world(ctx: GnoblinCtx, events: &[ShellEvent]) {
    let Some(root) = WORLDS.with(|worlds| worlds.borrow().get(&ctx).map(|world| world.client_root)) else {
        return;
    };
    let batch = PumpBatch::of(events);
    let ran = crate::object::run_in(ctx, root, |cx| {
        let Some(mut world) = WORLDS.with(|worlds| worlds.borrow_mut().remove(&ctx)) else {
            return;
        };
        if let Err(fail) = world.turn(cx, batch) {
            cx.log(
                gnoblin_core_abi::LOG_WARN,
                &format!("the object model could not read a batch: {fail}"),
            );
        }
        WORLDS.with(|worlds| worlds.borrow_mut().insert(ctx, world));
        properties::turn(cx);
    });
    if ran.is_err() {
        // The generation ended: its model goes with it.
        forget(ctx);
    }
}

/// The generation is ending, and the shell's code is still mapped: what its bindings began and did not finish ends, and the hooks hear it.
pub(crate) fn end_inflight(ctx: GnoblinCtx) {
    input::end_inflight(ctx);
}

/// The revision of what the shell of the generation `ctx` wrote to its settings, zero while it wrote nothing: the cheap question, asked
/// every turn, before the text is built ([`written_settings`]).
pub(crate) fn written_revision(ctx: GnoblinCtx) -> u64 {
    input::written_revision(ctx)
}

/// What the shell of the generation `ctx` wrote to the settings of the devices and of the compositor: the revision of what it wrote and the
/// sections of the configuration document they make. None while it wrote nothing. The shell host commits it ([`crate::document`]).
pub(crate) fn written_settings(ctx: GnoblinCtx) -> Option<(u64, String)> {
    input::written(ctx)
}

/// The generation ended: its model is dropped.
pub(crate) fn forget(ctx: GnoblinCtx) {
    WORLDS.with(|worlds| worlds.borrow_mut().remove(&ctx));
    properties::retire(ctx);
    session::forget(ctx);
    process::forget(ctx);
    idle::forget(ctx);
    launches::forget(ctx);
    input::forget(ctx);
    dialogs::forget(ctx);
    lock::forget(ctx);
}

/// The object of `obj` as a value for the shell, with a reference it must release.
fn handout(cx: &Cx<'_>, obj: Obj) -> Res<gnoblin_object::Val> {
    cx.handout(obj)
}
