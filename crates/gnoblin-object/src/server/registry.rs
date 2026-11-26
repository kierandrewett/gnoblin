//! How a library implements the classes of its catalog.

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::rc::Rc;

use gnoblin_core_abi as abi;

use super::cx::Cx;
use super::{Hooks, Library};
use crate::catalog::{CatalogDef, ClassDef};
use crate::error::{Fail, Res};
use crate::value::{MemberId, Obj, Val};

/// What a call about an object holds of its native state: a shared borrow for a getter and a member registered `_ref`, a unique one for
/// the rest. Shared borrows nest, so a `_ref` member may run a closure of the shell that reads a getter or calls another `_ref` member of
/// the same object.
pub(crate) enum Native<'a> {
    Shared(&'a dyn Any),
    Exclusive(&'a mut dyn Any),
}

impl Native<'_> {
    /// The state as a shared reference, which either borrow gives.
    pub(crate) fn shared(&self) -> &dyn Any {
        match self {
            Native::Shared(state) => *state,
            Native::Exclusive(state) => &**state,
        }
    }
}

/// Reads a property: the object's native state and the context.
pub(crate) type GetFn = Rc<dyn Fn(&dyn Any, &mut Cx<'_>) -> Res<Val>>;
type SetShared = Rc<dyn Fn(&dyn Any, &mut Cx<'_>, Val) -> Res<()>>;
type SetExclusive = Rc<dyn Fn(&mut dyn Any, &mut Cx<'_>, Val) -> Res<()>>;
type MethodShared = Rc<dyn Fn(&dyn Any, &mut Cx<'_>, &[Val]) -> Res<Val>>;
type MethodExclusive = Rc<dyn Fn(&mut dyn Any, &mut Cx<'_>, &[Val]) -> Res<Val>>;

/// Writes a property: with a shared borrow of the state (`set_ref`) or a unique one (`set`).
#[derive(Clone)]
pub(crate) enum SetFn {
    Shared(SetShared),
    Exclusive(SetExclusive),
}

impl SetFn {
    /// The call needs a unique borrow of the state.
    pub(crate) fn exclusive(&self) -> bool {
        matches!(self, SetFn::Exclusive(_))
    }

    pub(crate) fn run(&self, native: Native<'_>, cx: &mut Cx<'_>, value: Val) -> Res<()> {
        match (self, native) {
            (SetFn::Shared(write), native) => write(native.shared(), cx, value),
            (SetFn::Exclusive(write), Native::Exclusive(state)) => write(state, cx, value),
            (SetFn::Exclusive(_), Native::Shared(_)) => {
                Err(Fail::err("a setter that changes its object was given a shared borrow"))
            }
        }
    }
}

/// Runs a method with its checked arguments: with a shared borrow of the state (`method_ref`) or a unique one (`method`).
#[derive(Clone)]
pub(crate) enum MethodFn {
    Shared(MethodShared),
    Exclusive(MethodExclusive),
}

impl MethodFn {
    /// The call needs a unique borrow of the state.
    pub(crate) fn exclusive(&self) -> bool {
        matches!(self, MethodFn::Exclusive(_))
    }

    pub(crate) fn run(&self, native: Native<'_>, cx: &mut Cx<'_>, args: &[Val]) -> Res<Val> {
        match (self, native) {
            (MethodFn::Shared(run), native) => run(native.shared(), cx, args),
            (MethodFn::Exclusive(run), Native::Exclusive(state)) => run(state, cx, args),
            (MethodFn::Exclusive(_), Native::Shared(_)) => {
                Err(Fail::err("a method that changes its object was given a shared borrow"))
            }
        }
    }
}

/// Runs a function of a class.
pub(crate) type StaticFn = Rc<dyn Fn(&mut Cx<'_>, &[Val]) -> Res<Val>>;
/// Makes an object's native state from the properties it was created with.
pub(crate) type CtorFn = Rc<dyn Fn(&mut Cx<'_>, &mut Inits) -> Res<Box<dyn Any>>>;
/// Where a listener of a signal really listens: another object, and its signal (a facade that forwards to a collection).
pub(crate) type RedirectFn = Rc<dyn Fn(&mut Cx<'_>) -> Res<(Obj, &'static str)>>;
/// What a new listener of a signal is told at once: the payloads of the calls it missed, taken when it was added and delivered from the
/// next turn.
pub(crate) type ReplayFn = Rc<dyn Fn(&mut Cx<'_>) -> Vec<Vec<Val>>>;

/// The implementation of one class: a table per kind of member, indexed by member id minus one.
pub(crate) struct ClassImpl {
    /// The class holds `()` for state ([`Builder::class`]). There is nothing a call about one of its objects could leave half changed,
    /// so the server takes no borrow of the object for the length of a call, and what runs in the call may call the same object again
    /// (`Library::with_native`). A class with a state is borrowed for the length of a call: shared for a getter and a member registered
    /// `_ref` (`method_ref`, `set_ref`), unique for the rest. A second call about the object that needs a unique borrow, or that runs
    /// while a unique one is held, is refused with `E_BUSY`.
    pub(crate) stateless: bool,
    pub(crate) ctor: Option<CtorFn>,
    pub(crate) get: Vec<Option<GetFn>>,
    pub(crate) set: Vec<Option<SetFn>>,
    pub(crate) call: Vec<Option<MethodFn>>,
    pub(crate) stat: Vec<Option<StaticFn>>,
    pub(crate) redirect: Vec<Option<RedirectFn>>,
    pub(crate) replay: Vec<Option<ReplayFn>>,
}

impl ClassImpl {
    fn sized(members: usize) -> ClassImpl {
        ClassImpl {
            stateless: false,
            ctor: None,
            get: vec![None; members],
            set: vec![None; members],
            call: vec![None; members],
            stat: vec![None; members],
            redirect: vec![None; members],
            replay: vec![None; members],
        }
    }
}

/// The properties an object is created with, by name: the constructor takes what it needs and the rest are set through the setters.
pub struct Inits {
    items: Vec<(MemberId, &'static str, Val)>,
}

impl Inits {
    pub(crate) fn new(items: Vec<(MemberId, &'static str, Val)>) -> Inits {
        Inits { items }
    }

    /// Takes the property `name`, if the caller gave it.
    pub fn take(&mut self, name: &str) -> Option<Val> {
        let position = self.items.iter().position(|(_, held, _)| *held == name)?;
        Some(self.items.remove(position).2)
    }

    /// What the constructor left: set after it, through the setters.
    pub(crate) fn rest(self) -> Vec<(MemberId, &'static str, Val)> {
        self.items
    }
}

/// Builds a [`Library`]: one class at a time, each member by name. A name the catalog lacks is recorded as a problem that
/// [`Library::problems`] reports and does not stop the library, so a mistake is a failing test and not a crash at start.
pub struct Builder {
    catalog: &'static CatalogDef,
    classes: Vec<ClassImpl>,
    problems: Vec<String>,
}

impl Builder {
    /// A builder for `catalog`.
    pub fn new(catalog: &'static CatalogDef) -> Builder {
        Builder {
            catalog,
            classes: catalog
                .classes
                .iter()
                .map(|class| ClassImpl::sized(class.members.len()))
                .collect(),
            problems: Vec::new(),
        }
    }

    /// Implements the class `name`, whose objects hold a `T`.
    ///
    /// A `T` of `()` says the class has no state: a call about one of its objects takes no borrow, so an implementation may run a closure
    /// of the shell that reads or writes the same object (the std library's `request_transition` runs the shell's callback inside its
    /// call, and the callback reads `gnoblin.workspaces`, another member of the same object). Any other `T` is borrowed for the length of
    /// a call about its object, by what the member does with it: a getter and a member registered with `method_ref` or `set_ref` (which
    /// take `&T`) hold a shared borrow, and any number of those may run inside one another, so a closure of the shell that a `_ref`
    /// member runs can read the object's properties and call its other `_ref` members. A member registered with `method` or `set` (which
    /// take `&mut T`) holds a unique borrow, and a call about the same object from inside it, or a call to a `method` or `set` member
    /// from inside a `_ref` member, is refused with `E_BUSY`: it would be a second reference that aliases a `&mut`. A member that does
    /// not change its `T` belongs on the `_ref` form. `Cx::later` runs a closure from the next turn, when nothing is borrowed.
    pub fn class<T: Any>(&mut self, name: &str) -> ClassBuilder<'_, T> {
        let catalog: &'static CatalogDef = self.catalog;
        let index = catalog.classes.iter().position(|class| class.name == name);
        if index.is_none() {
            self.problems
                .push(format!("the catalog has no class `{name}` to implement"));
        }
        if let Some(index) = index {
            self.classes[index].stateless = TypeId::of::<T>() == TypeId::of::<()>();
        }
        ClassBuilder {
            def: index.map(|index| &catalog.classes[index]),
            slot: index.map(|index| &mut self.classes[index]),
            problems: &mut self.problems,
            _state: PhantomData,
        }
    }

    /// The library that serves the classes built so far.
    pub fn finish(self, hooks: Hooks) -> Library {
        Library::new(self.catalog, self.classes, hooks, self.problems)
    }
}

/// Implements one class whose objects hold a `T`. Every method takes `self` and gives it back, so a class reads as one expression.
pub struct ClassBuilder<'a, T> {
    def: Option<&'static ClassDef>,
    slot: Option<&'a mut ClassImpl>,
    problems: &'a mut Vec<String>,
    _state: PhantomData<T>,
}

impl<T: Any> ClassBuilder<'_, T> {
    /// The index of the member `name` of `kind`, or a recorded problem.
    fn find(&mut self, kind: u32, name: &str, what: &str) -> Option<usize> {
        let def = self.def?;
        match def.member_id(kind, name) {
            Some(id) => Some(id as usize - 1),
            None => {
                self.problems
                    .push(format!("class `{}` has no {what} `{name}` to implement", def.name));
                None
            }
        }
    }

    /// How an object's state is made. It takes the properties it needs from `inits`.
    #[must_use]
    pub fn ctor(mut self, make: impl Fn(&mut Cx<'_>, &mut Inits) -> Res<T> + 'static) -> Self {
        if let Some(slot) = self.slot.as_deref_mut() {
            slot.ctor = Some(Rc::new(move |cx, inits| {
                make(cx, inits).map(|state| Box::new(state) as Box<dyn Any>)
            }));
        }
        self
    }

    /// Reads the property `name`. A getter holds a shared borrow of the state.
    #[must_use]
    pub fn get(mut self, name: &str, read: impl Fn(&T, &mut Cx<'_>) -> Res<Val> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_PROPERTY, name, "property") {
            let class = self.def.map_or("", |def| def.name);
            if let Some(slot) = self.slot.as_deref_mut() {
                slot.get[index] = Some(Rc::new(move |state, cx| {
                    let state = state
                        .downcast_ref::<T>()
                        .ok_or_else(|| Fail::err(format!("an object of `{class}` holds the wrong state")))?;
                    read(state, cx)
                }));
            }
        }
        self
    }

    /// Writes the property `name`, with a unique borrow of the state: a call about the same object from inside `write` is refused with
    /// `E_BUSY`. A setter that does not change its `T` is [`ClassBuilder::set_ref`].
    #[must_use]
    pub fn set(mut self, name: &str, write: impl Fn(&mut T, &mut Cx<'_>, Val) -> Res<()> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_PROPERTY, name, "property") {
            let class = self.def.map_or("", |def| def.name);
            if let Some(slot) = self.slot.as_deref_mut() {
                slot.set[index] = Some(SetFn::Exclusive(Rc::new(move |state, cx, value| {
                    let state = state
                        .downcast_mut::<T>()
                        .ok_or_else(|| Fail::err(format!("an object of `{class}` holds the wrong state")))?;
                    write(state, cx, value)
                })));
            }
        }
        self
    }

    /// Writes the property `name` without changing the state: the borrow is a shared one, so `write` may run a closure of the shell
    /// that reads the object or calls its other `_ref` members.
    #[must_use]
    pub fn set_ref(mut self, name: &str, write: impl Fn(&T, &mut Cx<'_>, Val) -> Res<()> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_PROPERTY, name, "property") {
            let class = self.def.map_or("", |def| def.name);
            if let Some(slot) = self.slot.as_deref_mut() {
                slot.set[index] = Some(SetFn::Shared(Rc::new(move |state, cx, value| {
                    let state = state
                        .downcast_ref::<T>()
                        .ok_or_else(|| Fail::err(format!("an object of `{class}` holds the wrong state")))?;
                    write(state, cx, value)
                })));
            }
        }
        self
    }

    /// Runs the method `name`, with a unique borrow of the state: a call about the same object from inside `run` is refused with
    /// `E_BUSY`. Its arguments are checked against the catalog, and an optional one left out reads as [`Val::None`]. A method that does
    /// not change its `T` is [`ClassBuilder::method_ref`].
    #[must_use]
    pub fn method(mut self, name: &str, run: impl Fn(&mut T, &mut Cx<'_>, &[Val]) -> Res<Val> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_METHOD, name, "method") {
            let class = self.def.map_or("", |def| def.name);
            if let Some(slot) = self.slot.as_deref_mut() {
                slot.call[index] = Some(MethodFn::Exclusive(Rc::new(move |state, cx, args| {
                    let state = state
                        .downcast_mut::<T>()
                        .ok_or_else(|| Fail::err(format!("an object of `{class}` holds the wrong state")))?;
                    run(state, cx, args)
                })));
            }
        }
        self
    }

    /// Runs the method `name` without changing the state: the borrow is a shared one, so `run` may run a closure of the shell that
    /// reads the object's properties or calls its other `_ref` members (a call to a `method` or `set` member is refused with `E_BUSY`).
    /// Arguments are checked as for [`ClassBuilder::method`].
    #[must_use]
    pub fn method_ref(mut self, name: &str, run: impl Fn(&T, &mut Cx<'_>, &[Val]) -> Res<Val> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_METHOD, name, "method") {
            let class = self.def.map_or("", |def| def.name);
            if let Some(slot) = self.slot.as_deref_mut() {
                slot.call[index] = Some(MethodFn::Shared(Rc::new(move |state, cx, args| {
                    let state = state
                        .downcast_ref::<T>()
                        .ok_or_else(|| Fail::err(format!("an object of `{class}` holds the wrong state")))?;
                    run(state, cx, args)
                })));
            }
        }
        self
    }

    /// Runs the function `name` of the class (a method marked static).
    #[must_use]
    pub fn function(mut self, name: &str, run: impl Fn(&mut Cx<'_>, &[Val]) -> Res<Val> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_METHOD, name, "function")
            && let Some(slot) = self.slot.as_deref_mut()
        {
            slot.stat[index] = Some(Rc::new(run));
        }
        self
    }

    /// Makes a listener of the signal `name` listen to `target` instead: the signal of another object, found when the listener is
    /// added. A facade that forwards to a collection uses it, so its listeners stand in the collection's own list, in the order they
    /// were added.
    #[must_use]
    pub fn redirect(mut self, name: &str, target: impl Fn(&mut Cx<'_>) -> Res<(Obj, &'static str)> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_SIGNAL, name, "signal")
            && let Some(slot) = self.slot.as_deref_mut()
        {
            slot.redirect[index] = Some(Rc::new(target));
        }
        self
    }

    /// Tells a new listener of the signal `name` what is already there: `run` answers the payloads of the calls it missed, when the
    /// listener is added, and the listener is called with each from the next turn.
    #[must_use]
    pub fn replay(mut self, name: &str, run: impl Fn(&mut Cx<'_>) -> Vec<Vec<Val>> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_SIGNAL, name, "signal")
            && let Some(slot) = self.slot.as_deref_mut()
        {
            slot.replay[index] = Some(Rc::new(run));
        }
        self
    }
}
