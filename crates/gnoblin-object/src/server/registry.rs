//! How a library implements the classes of its catalog.

use std::any::Any;
use std::marker::PhantomData;
use std::rc::Rc;

use gnoblin_core_abi as abi;

use super::cx::Cx;
use super::{Hooks, Library};
use crate::catalog::{CatalogDef, ClassDef};
use crate::error::{Fail, Res};
use crate::value::{MemberId, Val};

/// Reads a property: the object's native state and the context.
pub(crate) type GetFn = Rc<dyn Fn(&mut dyn Any, &mut Cx<'_>) -> Res<Val>>;
/// Writes a property.
pub(crate) type SetFn = Rc<dyn Fn(&mut dyn Any, &mut Cx<'_>, Val) -> Res<()>>;
/// Runs a method with its checked arguments.
pub(crate) type MethodFn = Rc<dyn Fn(&mut dyn Any, &mut Cx<'_>, &[Val]) -> Res<Val>>;
/// Runs a function of a class.
pub(crate) type StaticFn = Rc<dyn Fn(&mut Cx<'_>, &[Val]) -> Res<Val>>;
/// Makes an object's native state from the properties it was created with.
pub(crate) type CtorFn = Rc<dyn Fn(&mut Cx<'_>, &mut Inits) -> Res<Box<dyn Any>>>;

/// The implementation of one class: a table per kind of member, indexed by member id minus one.
pub(crate) struct ClassImpl {
    pub(crate) ctor: Option<CtorFn>,
    pub(crate) get: Vec<Option<GetFn>>,
    pub(crate) set: Vec<Option<SetFn>>,
    pub(crate) call: Vec<Option<MethodFn>>,
    pub(crate) stat: Vec<Option<StaticFn>>,
}

impl ClassImpl {
    fn sized(members: usize) -> ClassImpl {
        ClassImpl {
            ctor: None,
            get: vec![None; members],
            set: vec![None; members],
            call: vec![None; members],
            stat: vec![None; members],
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
    pub fn class<T: Any>(&mut self, name: &str) -> ClassBuilder<'_, T> {
        let catalog: &'static CatalogDef = self.catalog;
        let index = catalog.classes.iter().position(|class| class.name == name);
        if index.is_none() {
            self.problems
                .push(format!("the catalog has no class `{name}` to implement"));
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

    /// Reads the property `name`.
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

    /// Writes the property `name`.
    #[must_use]
    pub fn set(mut self, name: &str, write: impl Fn(&mut T, &mut Cx<'_>, Val) -> Res<()> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_PROPERTY, name, "property") {
            let class = self.def.map_or("", |def| def.name);
            if let Some(slot) = self.slot.as_deref_mut() {
                slot.set[index] = Some(Rc::new(move |state, cx, value| {
                    let state = state
                        .downcast_mut::<T>()
                        .ok_or_else(|| Fail::err(format!("an object of `{class}` holds the wrong state")))?;
                    write(state, cx, value)
                }));
            }
        }
        self
    }

    /// Runs the method `name`. Its arguments are checked against the catalog, and an optional one left out reads as [`Val::None`].
    #[must_use]
    pub fn method(mut self, name: &str, run: impl Fn(&mut T, &mut Cx<'_>, &[Val]) -> Res<Val> + 'static) -> Self {
        if let Some(index) = self.find(abi::MEMBER_METHOD, name, "method") {
            let class = self.def.map_or("", |def| def.name);
            if let Some(slot) = self.slot.as_deref_mut() {
                slot.call[index] = Some(Rc::new(move |state, cx, args| {
                    let state = state
                        .downcast_mut::<T>()
                        .ok_or_else(|| Fail::err(format!("an object of `{class}` holds the wrong state")))?;
                    run(state, cx, args)
                }));
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
}
