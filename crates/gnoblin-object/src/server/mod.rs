//! The server half of the object protocol: what gives a library its classes.
//!
//! A library (`libgnoblin-std`, `libgnoblin-core`) declares its classes in a catalog (`CatalogDef`, written by `gnoblin-sdkgen` from
//! the schema files) and implements them in Rust through a [`Builder`]: for each class a constructor, getters and setters of its
//! properties, its methods and its functions. This module does the rest, once, for every class of every library:
//!
//! - the object table of a generation, with handles that cannot name a new object after the old one is gone;
//! - the argument checks the catalog implies, so an implementation reads values of the types it declared;
//! - references, parents and children, and what a `release` or a `destroy` ends;
//! - closures and implemented interfaces the shell gave (`wire::RawClosure`, [`ShellImpl`]), adopted only when a call succeeds;
//! - signals and property changes, queued by the implementation and told to listeners from the next turn, never inside a call;
//! - the extern "C" functions of `GnoblinObjectApi` ([`api_table`]).
//!
//! An implementation reaches all of it through [`Cx`]. A library holds one [`Library`] per thread (the shell thread) and tells the
//! module how it posts a turn and records an error through [`Hooks`].

mod api;
mod check;
mod cx;
mod object;
mod ops;
mod registry;
mod shell;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinCtx};

pub use api::{Provider, api_table};
pub use cx::Cx;
pub use registry::{Builder, ClassBuilder, Inits};
pub use shell::ShellImpl;

use crate::catalog::CatalogDef;
use crate::error::{Fail, Res};
use object::CtxState;
use registry::ClassImpl;

/// What a library tells this module about itself.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// A generation uses the library for the first time: arrange for [`Library::retire`] when it ends. An error refuses the use.
    pub first_use: fn(GnoblinCtx) -> Res<()>,
    /// Post a call of [`Library::flush`] to the generation's thread, to run before the events of its next dispatch.
    pub schedule: fn(GnoblinCtx),
    /// Record a failure as the text the library's `last_error` returns.
    pub set_last: fn(&Fail),
    /// Write a line to the generation's log (`LOG_*` level).
    pub log: fn(GnoblinCtx, u32, &str),
}

/// A library's objects, on the shell thread.
pub struct Library {
    pub(crate) catalog: &'static CatalogDef,
    pub(crate) classes: Vec<ClassImpl>,
    pub(crate) hooks: Hooks,
    problems: Vec<String>,
    ctxs: RefCell<HashMap<GnoblinCtx, Rc<CtxState>>>,
    retired: RefCell<HashSet<GnoblinCtx>>,
    next_reg: Cell<u64>,
}

impl Library {
    pub(crate) fn new(
        catalog: &'static CatalogDef,
        classes: Vec<ClassImpl>,
        hooks: Hooks,
        problems: Vec<String>,
    ) -> Library {
        Library {
            catalog,
            classes,
            hooks,
            problems,
            ctxs: RefCell::new(HashMap::new()),
            retired: RefCell::new(HashSet::new()),
            next_reg: Cell::new(0),
        }
    }

    /// The catalog the library serves.
    pub fn catalog(&self) -> &'static CatalogDef {
        self.catalog
    }

    /// What is wrong with the implementation's registrations and its coverage of the catalog, one sentence each. The two-way
    /// completeness test of a library asserts this is empty: every class of the catalog has an implementation and every implementation
    /// names a member of the catalog.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = self.problems.clone();
        for (index, class) in self.catalog.classes.iter().enumerate() {
            let implementation = &self.classes[index];
            if matches!(class.kind, abi::CLASS_ENUM | abi::CLASS_RECORD) {
                continue;
            }
            if class.kind != abi::CLASS_INTERFACE
                && implementation.ctor.is_none()
                && !class.members.iter().all(is_static_only)
            {
                problems.push(format!("class `{}` has no constructor", class.name));
            }
            if class.kind == abi::CLASS_INTERFACE {
                continue;
            }
            for (member_index, member) in class.members.iter().enumerate() {
                if member.kind == abi::MEMBER_SIGNAL || member.flags & abi::MEMBER_INERT != 0 || !member.abi.is_empty()
                {
                    continue;
                }
                let done = match member.kind {
                    abi::MEMBER_PROPERTY => {
                        implementation.get[member_index].is_some()
                            && (member.flags & abi::MEMBER_READONLY != 0 || implementation.set[member_index].is_some())
                    }
                    abi::MEMBER_METHOD if member.is_static() => implementation.stat[member_index].is_some(),
                    abi::MEMBER_METHOD => implementation.call[member_index].is_some(),
                    _ => true,
                };
                if !done {
                    problems.push(format!(
                        "`{}.{}` is in the catalog and not implemented",
                        class.name, member.name
                    ));
                }
            }
        }
        problems
    }

    /// The generation's objects, made on first use when `create` is set.
    pub(crate) fn state(&self, ctx: GnoblinCtx, create: bool) -> Res<Rc<CtxState>> {
        if let Some(state) = self.ctxs.borrow().get(&ctx) {
            return Ok(Rc::clone(state));
        }
        if self.retired.borrow().contains(&ctx) {
            return Err(Fail::new(
                abi::E_RETIRED,
                "the generation has retired; its objects are gone",
            ));
        }
        if !create {
            return Err(Fail::stale("the generation holds no objects of this library"));
        }
        (self.hooks.first_use)(ctx)?;
        let state = Rc::new(CtxState::new(ctx));
        self.ctxs.borrow_mut().insert(ctx, Rc::clone(&state));
        Ok(state)
    }

    /// The generation ended: every object, listener and closure goes now, while the shell's code is still mapped, so each
    /// `drop_user` runs once.
    pub fn retire(&self, ctx: GnoblinCtx) {
        let state = self.ctxs.borrow_mut().remove(&ctx);
        self.retired.borrow_mut().insert(ctx);
        let Some(state) = state else { return };
        let queue = std::mem::take(&mut *state.queue.borrow_mut());
        let listeners = std::mem::take(&mut *state.listeners.borrow_mut());
        let objects = state.objects.borrow_mut().drain();
        drop(queue);
        drop(listeners);
        drop(objects);
    }

    /// How many objects the generation holds, for tests.
    pub fn live_objects(&self, ctx: GnoblinCtx) -> usize {
        self.ctxs
            .borrow()
            .get(&ctx)
            .map_or(0, |state| state.objects.borrow().len())
    }

    fn next_reg(&self) -> u64 {
        let next = self.next_reg.get() + 1;
        self.next_reg.set(next);
        next
    }
}

fn is_static_only(member: &crate::catalog::MemberDef) -> bool {
    member.kind != abi::MEMBER_PROPERTY && (member.is_static() || member.kind == abi::MEMBER_SIGNAL)
}
