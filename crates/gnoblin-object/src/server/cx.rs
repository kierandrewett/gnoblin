//! What an implementation may do while it runs: queue a signal, make a child object, hold an object, ask for work in the next turn.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinCtx};

use super::Library;
use super::object::{CtxState, Entry};
use super::shell::ShellImpl;
use crate::catalog::ClassDef;
use crate::error::{Fail, Res};
use crate::value::{ClassId, MemberId, Obj, Timing, Val};

/// The context of one call into an implementation: the generation, the object it is about, and what it may ask of the library.
///
/// A signal or a property change an implementation announces is queued, and its listeners hear it from the next turn, so a listener
/// never runs inside the call that caused the change.
pub struct Cx<'a> {
    lib: &'a Library,
    st: &'a Rc<CtxState>,
    me: Obj,
    class: ClassId,
    timing: Option<&'a Timing>,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(lib: &'a Library, st: &'a Rc<CtxState>, me: Obj, class: ClassId) -> Cx<'a> {
        Cx {
            lib,
            st,
            me,
            class,
            timing: None,
        }
    }

    /// The same context for a timed write: [`Cx::timing`] answers while the setter runs.
    pub(crate) fn with_timing(mut self, timing: &'a Timing) -> Cx<'a> {
        self.timing = Some(timing);
        self
    }

    /// How the property being set is to play, when the write is a timed one (`set_timed`).
    pub fn timing(&self) -> Option<&Timing> {
        self.timing
    }

    /// The class of the object the call is about; zero for a function of a class that has no object.
    pub fn class(&self) -> ClassId {
        self.class
    }

    /// The generation the call is for.
    pub fn ctx(&self) -> GnoblinCtx {
        self.st.ctx
    }

    /// The object the call is about; zero for a function of a class.
    pub fn me(&self) -> Obj {
        self.me
    }

    /// The library's hook to the generation's log.
    pub fn log(&self, level: u32, message: &str) {
        (self.lib.hooks.log)(self.st.ctx, level, message);
    }

    fn class_def(&self, class: ClassId) -> Res<&'static ClassDef> {
        self.lib
            .catalog
            .class(class)
            .ok_or_else(|| Fail::err(format!("the class id {class} is not in the catalog")))
    }

    /// The id of the member `name` of `kind` of `class`.
    ///
    /// # Errors
    ///
    /// The class has no such member.
    pub fn member_of(&self, class: ClassId, kind: u32, name: &str) -> Res<MemberId> {
        let def = self.class_def(class)?;
        def.member_id(kind, name)
            .ok_or_else(|| Fail::no_such(format!("class `{}` has no member `{name}`", def.name)))
    }

    /// The id of a class of the catalog.
    ///
    /// # Errors
    ///
    /// The catalog has no such class.
    pub fn class_named(&self, name: &str) -> Res<ClassId> {
        self.lib
            .catalog
            .class_id(name)
            .ok_or_else(|| Fail::no_such(format!("the catalog has no class `{name}`")))
    }

    /// The class of an object of this generation.
    ///
    /// # Errors
    ///
    /// The handle names no object.
    pub fn class_of(&self, obj: Obj) -> Res<ClassId> {
        self.st
            .objects
            .borrow()
            .get(obj)
            .map(|entry| entry.class)
            .ok_or_else(|| Fail::stale("the object is gone"))
    }

    /// An object as a value that holds no reference: for the fields of a signal, where the library takes one reference for each
    /// listener. A value for the shell to keep is [`Cx::handout`].
    pub fn object_value(&self, obj: Obj) -> Val {
        Val::Object {
            class: self.class_of(obj).unwrap_or(0),
            obj,
        }
    }

    /// Queues the signal `name` of this object with `fields` as its payload. It is told to the listeners from the next turn, and only
    /// when there are any. An object in the fields is a plain handle ([`Cx::object_value`]): each listener is given a reference of
    /// its own.
    pub fn emit(&self, name: &str, fields: Vec<Val>) {
        self.emit_on(self.me, name, fields);
    }

    /// Queues a signal of another object of this generation.
    pub fn emit_on(&self, obj: Obj, name: &str, fields: Vec<Val>) {
        let Ok(class) = self.class_of(obj) else { return };
        let Ok(member) = self.member_of(class, abi::MEMBER_SIGNAL, name) else {
            self.log(abi::LOG_ERROR, &format!("the class has no signal `{name}`"));
            return;
        };
        self.lib.queue_signal(self.st, obj, member, fields);
    }

    /// Announces that the property `name` of this object changed. Listeners hear its value as of the turn, once however many changes
    /// there were.
    pub fn changed(&self, name: &str) {
        self.changed_on(self.me, name);
    }

    /// Announces a change of a property of another object of this generation.
    pub fn changed_on(&self, obj: Obj, name: &str) {
        let Ok(class) = self.class_of(obj) else { return };
        let Ok(member) = self.member_of(class, abi::MEMBER_PROPERTY, name) else {
            self.log(abi::LOG_ERROR, &format!("the class has no property `{name}`"));
            return;
        };
        self.lib.queue_changed(self.st, obj, member);
    }

    /// Runs `work` from the next turn, as a call about this object: the place to call a closure or an implemented method the shell
    /// gave, which the library never does inside a call.
    pub fn later(&self, work: impl FnOnce(&mut Cx<'_>) + 'static) {
        self.lib.queue_task(self.st, self.me, Box::new(work));
    }

    /// Makes an object of `class` that holds `state`, with no reference held by anyone yet: hand it to the shell with
    /// [`Cx::handout`], or give it a parent with [`Cx::set_parent`], or it is freed with the generation.
    ///
    /// # Errors
    ///
    /// The class is not in the catalog.
    pub fn spawn<T: Any>(&self, class: &str, state: T) -> Res<Obj> {
        let class = self.class_named(class)?;
        let def = self.class_def(class)?;
        let entry = Entry {
            class,
            state: Rc::new(RefCell::new(Box::new(state))),
            shell: None,
            refs: 0,
            parent: None,
            children: Vec::new(),
            pinned: def.flags & abi::CLASS_FLAG_RETAINED != 0,
            base: None,
        };
        Ok(self.st.objects.borrow_mut().insert(entry))
    }

    /// The value that hands `obj` to the shell: one more reference it must `release`. Return it from a getter or a method, or put
    /// it in a signal's fields.
    ///
    /// # Errors
    ///
    /// The object is gone.
    pub fn handout(&self, obj: Obj) -> Res<Val> {
        self.lib.handout(self.st, obj)
    }

    /// Makes `parent` own `child`: the child lives until `destroy` of it or of the parent, whatever the shell releases.
    ///
    /// # Errors
    ///
    /// Either object is gone.
    pub fn set_parent(&self, child: Obj, parent: Obj) -> Res<()> {
        let mut objects = self.st.objects.borrow_mut();
        let previous = objects
            .get(child)
            .ok_or_else(|| Fail::stale("the child is gone"))?
            .parent;
        if objects.get(parent).is_none() {
            return Err(Fail::stale("the parent is gone"));
        }
        if let Some(previous) = previous
            && let Some(old) = objects.get_mut(previous)
        {
            old.children.retain(|held| *held != child);
        }
        if let Some(entry) = objects.get_mut(child) {
            entry.parent = Some(parent);
        }
        if let Some(entry) = objects.get_mut(parent) {
            entry.children.push(child);
        }
        Ok(())
    }

    /// The children of `obj`, in the order they were added.
    pub fn children(&self, obj: Obj) -> Vec<Obj> {
        self.st
            .objects
            .borrow()
            .get(obj)
            .map(|entry| entry.children.clone())
            .unwrap_or_default()
    }

    /// The parent of `obj`.
    pub fn parent(&self, obj: Obj) -> Option<Obj> {
        self.st.objects.borrow().get(obj).and_then(|entry| entry.parent)
    }

    /// Takes a reference to `obj` for native state that keeps it (a layout a workspace uses): it is not freed while held.
    ///
    /// # Errors
    ///
    /// The object is gone.
    pub fn hold(&self, obj: Obj) -> Res<()> {
        let mut objects = self.st.objects.borrow_mut();
        let entry = objects.get_mut(obj).ok_or_else(|| Fail::stale("the object is gone"))?;
        entry.refs += 1;
        Ok(())
    }

    /// Gives back a reference taken with [`Cx::hold`]. An object nobody else holds ends here.
    pub fn unhold(&self, obj: Obj) {
        self.lib.release_ref(self.st, obj);
    }

    /// The shell's methods of an object made by `implement`.
    ///
    /// # Errors
    ///
    /// The object is gone, or the shell did not implement it.
    pub fn shell(&self, obj: Obj) -> Res<Rc<ShellImpl>> {
        self.st
            .objects
            .borrow()
            .get(obj)
            .ok_or_else(|| Fail::stale("the object is gone"))?
            .shell
            .clone()
            .ok_or_else(|| Fail::type_error("the object is not one the shell implements"))
    }

    /// The native state of another object, for an implementation that works on two (a workspace and its client). Gives `work` the
    /// state when it is a `T`.
    ///
    /// # Errors
    ///
    /// The object is gone, is held by a call that is running, or holds another type.
    pub fn with_state<T: Any, R>(&self, obj: Obj, work: impl FnOnce(&mut T) -> R) -> Res<R> {
        let state = self
            .st
            .objects
            .borrow()
            .get(obj)
            .ok_or_else(|| Fail::stale("the object is gone"))?
            .state
            .clone();
        let mut guard = state
            .try_borrow_mut()
            .map_err(|_| Fail::new(abi::E_BUSY, "the object is in use by a call that is running"))?;
        let state = guard
            .downcast_mut::<T>()
            .ok_or_else(|| Fail::type_error("the object holds another kind of state"))?;
        Ok(work(state))
    }
}
