//! The operations of the protocol, on a [`Library`]: what each function of `GnoblinObjectApi` does once its arguments are read.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinDropFn, GnoblinInvokeFn};

use super::Library;
use super::cx::Cx;
use super::object::{CtxState, Entry, Listener, Pending, State};
use super::registry::Inits;
use super::shell::ShellImpl;
use crate::catalog::{ClassDef, MemberDef};
use crate::client::Api;
use crate::error::{Fail, Res};
use crate::value::{Callable, ClassId, MemberId, Obj, Timing, Val};
use crate::wire::RawClosure;

/// The most rounds of work one turn runs before it leaves the rest for the next: a listener that keeps asking for more must not hold
/// the shell thread for ever.
const MAX_ROUNDS: usize = 64;

/// What a call is about.
struct Target {
    class: ClassId,
    state: State,
}

impl Library {
    pub(crate) fn class_def(&self, class: ClassId) -> Res<&'static ClassDef> {
        self.catalog
            .class(class)
            .ok_or_else(|| Fail::no_such(format!("the class id {class} is not in the catalog")))
    }

    fn member_def(&self, class: ClassId, member: MemberId, kind: u32) -> Res<&'static MemberDef> {
        let def = self.class_def(class)?;
        let found = def
            .member(member)
            .ok_or_else(|| Fail::no_such(format!("class `{}` has no member with id {member}", def.name)))?;
        if found.kind != kind {
            return Err(Fail::type_error(format!(
                "`{}.{}` is not a {}",
                def.name,
                found.name,
                kind_word(kind)
            )));
        }
        Ok(found)
    }

    fn target(&self, st: &CtxState, obj: Obj) -> Res<Target> {
        let objects = st.objects.borrow();
        let entry = objects
            .get(obj)
            .ok_or_else(|| Fail::stale("the object is gone, or was never made"))?;
        Ok(Target {
            class: entry.class,
            state: Rc::clone(&entry.state),
        })
    }

    /// The id of the class `name`.
    pub fn class_id(&self, name: &str) -> Res<ClassId> {
        self.catalog.class_id(name).ok_or_else(|| {
            Fail::no_such(format!(
                "the catalog of `{}` has no class `{name}`",
                self.catalog.namespace
            ))
        })
    }

    /// The id of the member `name` of `kind` of `class`.
    pub fn member_id(&self, class: ClassId, kind: u32, name: &str) -> Res<MemberId> {
        let def = self.class_def(class)?;
        def.member_id(kind, name)
            .ok_or_else(|| Fail::no_such(format!("class `{}` has no {} `{name}`", def.name, kind_word(kind))))
    }

    /// The class of an object.
    pub fn class_of(&self, ctx: GnoblinCtx, obj: Obj) -> Res<ClassId> {
        let st = self.state(ctx, false)?;
        Ok(self.target(&st, obj)?.class)
    }

    /// Runs `work` as a call about `obj` of generation `ctx`, without the call: the way library code that runs from a turn of its own
    /// (the toolkit's frame, a signal of the compositor) announces a signal or a property change, makes an object to hand out, or reads
    /// the state of another object, as an implementation does inside a call. Nothing is borrowed while `work` runs, so it may call back
    /// into the library.
    ///
    /// # Errors
    ///
    /// The generation holds no objects, or `obj` is gone.
    pub fn with_cx<R>(&self, ctx: GnoblinCtx, obj: Obj, work: impl FnOnce(&mut Cx<'_>) -> R) -> Res<R> {
        let st = self.state(ctx, false)?;
        let class = self.target(&st, obj)?.class;
        let mut cx = Cx::new(self, &st, obj, class);
        Ok(work(&mut cx))
    }

    // ---- making and ending objects ---------------------------------------------------------------------------------------

    /// Makes an object of `class` with `inits` set, and returns one reference to it. A service class gives its one object.
    pub fn create(&self, ctx: GnoblinCtx, class: ClassId, inits: Vec<(MemberId, Val)>) -> Res<Obj> {
        let st = self.state(ctx, true)?;
        let def = self.class_def(class)?;
        match def.kind {
            abi::CLASS_HANDLE | abi::CLASS_SERVICE => {}
            abi::CLASS_INTERFACE => {
                return Err(Fail::type_error(format!(
                    "`{}` is an interface: a shell implements it with `implement`",
                    def.name
                )));
            }
            _ => return Err(Fail::type_error(format!("`{}` is a value, not an object", def.name))),
        }
        let service = def.kind == abi::CLASS_SERVICE;
        if service && let Some(obj) = st.services.borrow().get(&class).copied() {
            self.apply_inits(&st, obj, class, inits)?;
            return self.add_ref(&st, obj).map(|()| obj);
        }
        // The members of a base class that another library serves go to that library, which makes the base object.
        let (foreign_inits, inits): (Vec<_>, Vec<_>) = inits
            .into_iter()
            .partition(|(member, _)| (*member as usize) <= def.foreign_members);
        let imp = &self.classes[class as usize - 1];
        let ctor = imp
            .ctor
            .as_ref()
            .ok_or_else(|| Fail::unsupported(format!("`{}` has no implementation yet", def.name)))?;
        let mut checked = Vec::with_capacity(inits.len());
        let mut adopt = Vec::with_capacity(inits.len());
        for (member, value) in inits {
            let property = self.member_def(class, member, abi::MEMBER_PROPERTY)?;
            let value = self.coerce(&st, property.ty, value, &format!("`{}.{}`", def.name, property.name))?;
            adopt.push(value.clone());
            checked.push((member, property.name, value));
        }
        let base = match def.foreign_base() {
            Some((home, bare)) if def.foreign_members > 0 => {
                let api = (self.hooks.foreign)(ctx, home)?;
                let base_class = api.class(bare)?;
                Some((home, api.create(base_class, &foreign_inits)?))
            }
            _ => None,
        };
        let obj = st.objects.borrow_mut().insert(Entry {
            class,
            state: Rc::new(RefCell::new(Box::new(()))),
            shell: None,
            refs: 0,
            parent: None,
            children: Vec::new(),
            pinned: service || def.flags & abi::CLASS_FLAG_RETAINED != 0,
            base,
        });
        let mut cx = Cx::new(self, &st, obj, class);
        let mut inits = Inits::new(checked);
        let made = guarded("a constructor", || ctor(&mut cx, &mut inits));
        match made {
            Ok(native) => {
                if let Some(state) = st.objects.borrow().get(obj).map(|entry| Rc::clone(&entry.state)) {
                    *state.borrow_mut() = native;
                }
            }
            Err(fail) => {
                self.drop_object(&st, obj);
                return Err(fail);
            }
        }
        for (member, _name, value) in inits.rest() {
            if let Err(fail) = self.set_value(&st, obj, class, member, value) {
                self.drop_object(&st, obj);
                return Err(fail);
            }
        }
        adopt.iter().for_each(Val::adopt_closures);
        foreign_inits.iter().for_each(|(_, value)| value.adopt_closures());
        if let Some(entry) = st.objects.borrow_mut().get_mut(obj) {
            entry.refs = 1;
        }
        if service {
            st.services.borrow_mut().insert(class, obj);
        }
        Ok(obj)
    }

    /// Ends an object and its children: what they hold is dropped, their listeners go and every handle to them is stale.
    pub fn destroy(&self, ctx: GnoblinCtx, obj: Obj) -> Res<()> {
        let st = self.state(ctx, false)?;
        self.target(&st, obj)?;
        self.drop_object(&st, obj);
        Ok(())
    }

    /// Drops one reference to an object. An object nobody else holds, with no parent, that is not retained, ends here.
    pub fn release(&self, ctx: GnoblinCtx, obj: Obj) -> Res<()> {
        let st = self.state(ctx, false)?;
        self.target(&st, obj)?;
        self.release_ref(&st, obj);
        Ok(())
    }

    pub(crate) fn add_ref(&self, st: &CtxState, obj: Obj) -> Res<()> {
        let mut objects = st.objects.borrow_mut();
        let entry = objects.get_mut(obj).ok_or_else(|| Fail::stale("the object is gone"))?;
        entry.refs += 1;
        Ok(())
    }

    pub(crate) fn release_ref(&self, st: &Rc<CtxState>, obj: Obj) {
        let ends = {
            let mut objects = st.objects.borrow_mut();
            let Some(entry) = objects.get_mut(obj) else { return };
            entry.refs = entry.refs.saturating_sub(1);
            entry.refs == 0 && !entry.pinned && entry.parent.is_none()
        };
        if ends {
            self.drop_object(st, obj);
        }
    }

    /// The value that hands `obj` to the shell, with the reference it must release.
    pub(crate) fn handout(&self, st: &CtxState, obj: Obj) -> Res<Val> {
        let mut objects = st.objects.borrow_mut();
        let entry = objects.get_mut(obj).ok_or_else(|| Fail::stale("the object is gone"))?;
        entry.refs += 1;
        Ok(Val::Object {
            class: entry.class,
            obj,
        })
    }

    /// Adds a reference for every object a value hands to a receiver.
    pub(crate) fn hand_out_all(&self, st: &CtxState, value: &Val) {
        match value {
            Val::Object { obj, .. } => {
                let _ = self.add_ref(st, *obj);
            }
            Val::List(items) => items.iter().for_each(|item| self.hand_out_all(st, item)),
            Val::Record { fields, .. } => fields.iter().for_each(|field| self.hand_out_all(st, field)),
            _ => {}
        }
    }

    /// Removes `obj` and its descendants. What they held is dropped after every borrow is released, because dropping a closure runs
    /// a shell's `drop_user`.
    pub(crate) fn drop_object(&self, st: &Rc<CtxState>, obj: Obj) {
        let mut doomed = Vec::new();
        let mut stack = vec![obj];
        {
            let objects = st.objects.borrow();
            while let Some(next) = stack.pop() {
                if let Some(entry) = objects.get(next) {
                    doomed.push(next);
                    stack.extend(entry.children.iter().copied());
                }
            }
        }
        let mut removed = Vec::with_capacity(doomed.len());
        let mut dead_listeners = Vec::new();
        {
            let mut objects = st.objects.borrow_mut();
            // The parent forgets the child first.
            if let Some(parent) = objects.get(obj).and_then(|entry| entry.parent)
                && let Some(parent) = objects.get_mut(parent)
            {
                parent.children.retain(|held| *held != obj);
            }
            for victim in &doomed {
                if let Some(entry) = objects.remove(*victim) {
                    removed.push(entry);
                }
            }
        }
        {
            let mut services = st.services.borrow_mut();
            services.retain(|_, held| !doomed.contains(held));
        }
        {
            let mut listeners = st.listeners.borrow_mut();
            for victim in &doomed {
                for reg in listeners.of_object(*victim) {
                    dead_listeners.extend(listeners.remove(reg));
                }
            }
        }
        let bases: Vec<(&'static str, Obj)> = removed.iter().filter_map(|entry| entry.base).collect();
        drop(dead_listeners);
        drop(removed);
        for (home, base) in bases {
            if let Ok(api) = (self.hooks.foreign)(st.ctx, home) {
                let _ = api.release(base);
            }
        }
    }

    /// The base object of `obj` and the library that serves it, for a member the base class owns.
    fn forward(&self, st: &CtxState, ctx: GnoblinCtx, obj: Obj) -> Res<(Api, Obj)> {
        let (home, base) = st
            .objects
            .borrow()
            .get(obj)
            .and_then(|entry| entry.base)
            .ok_or_else(|| Fail::stale("the object has no base object"))?;
        Ok(((self.hooks.foreign)(ctx, home)?, base))
    }

    // ---- properties ------------------------------------------------------------------------------------------------------

    /// Reads a property.
    pub fn get(&self, ctx: GnoblinCtx, obj: Obj, member: MemberId) -> Res<Val> {
        let st = self.state(ctx, false)?;
        let target = self.target(&st, obj)?;
        let def = self.class_def(target.class)?;
        let property = self.member_def(target.class, member, abi::MEMBER_PROPERTY)?;
        if member as usize <= def.foreign_members {
            let (api, base) = self.forward(&st, ctx, obj)?;
            let value = api.get(base, member)?;
            self.tell_tracker(obj, target.class, member);
            return Ok(value);
        }
        let imp = &self.classes[target.class as usize - 1];
        let read = imp.get[member as usize - 1].as_ref().ok_or_else(|| {
            Fail::unsupported(format!("`{}` is in the catalog and not implemented yet", property.name))
        })?;
        let mut guard = target
            .state
            .try_borrow_mut()
            .map_err(|_| Fail::new(abi::E_BUSY, "the object is in use by a call that is running"))?;
        let mut cx = Cx::new(self, &st, obj, target.class);
        let value = guarded(property.name, || read(&mut **guard, &mut cx))?;
        self.tell_tracker(obj, target.class, member);
        Ok(value)
    }

    /// Sets a property.
    pub fn set(&self, ctx: GnoblinCtx, obj: Obj, member: MemberId, value: Val) -> Res<()> {
        let st = self.state(ctx, false)?;
        let target = self.target(&st, obj)?;
        let adopt = value.clone();
        self.set_value(&st, obj, target.class, member, value)?;
        adopt.adopt_closures();
        Ok(())
    }

    /// Sets properties in order, stopping at the first failure.
    pub fn update(&self, ctx: GnoblinCtx, obj: Obj, inits: Vec<(MemberId, Val)>) -> Res<()> {
        let st = self.state(ctx, false)?;
        let class = self.target(&st, obj)?.class;
        self.apply_inits(&st, obj, class, inits)
    }

    fn apply_inits(&self, st: &Rc<CtxState>, obj: Obj, class: ClassId, inits: Vec<(MemberId, Val)>) -> Res<()> {
        for (member, value) in inits {
            let adopt = value.clone();
            self.set_value(st, obj, class, member, value)?;
            adopt.adopt_closures();
        }
        Ok(())
    }

    fn set_value(&self, st: &Rc<CtxState>, obj: Obj, class: ClassId, member: MemberId, value: Val) -> Res<()> {
        self.set_value_with(st, obj, class, member, value, None)
    }

    /// Sets an animated property as a timed write: the setter reads how it plays from [`Cx::timing`].
    pub fn set_timed(&self, ctx: GnoblinCtx, obj: Obj, member: MemberId, value: Val, timing: &Timing) -> Res<()> {
        let st = self.state(ctx, false)?;
        let class = self.target(&st, obj)?.class;
        let property = self.member_def(class, member, abi::MEMBER_PROPERTY)?;
        if property.flags & abi::MEMBER_ANIMATED == 0 {
            return Err(Fail::type_error(format!("`{}` cannot be animated", property.name)));
        }
        if !(timing.seconds.is_finite() && timing.seconds > 0.0) {
            return Err(Fail::type_error("a timed write needs a duration above zero"));
        }
        let adopt = value.clone();
        self.set_value_with(&st, obj, class, member, value, Some(timing))?;
        adopt.adopt_closures();
        Ok(())
    }

    fn set_value_with(
        &self,
        st: &Rc<CtxState>,
        obj: Obj,
        class: ClassId,
        member: MemberId,
        value: Val,
        timing: Option<&Timing>,
    ) -> Res<()> {
        let def = self.class_def(class)?;
        let property = self.member_def(class, member, abi::MEMBER_PROPERTY)?;
        if member as usize <= def.foreign_members {
            let (api, base) = self.forward(st, st.ctx, obj)?;
            return match timing {
                Some(timing) => api.set_timed(base, member, &value, timing),
                None => api.set(base, member, &value),
            };
        }
        if property.flags & abi::MEMBER_READONLY != 0 {
            return Err(Fail::type_error(format!(
                "`{}.{}` is read-only",
                def.name, property.name
            )));
        }
        let value = self.coerce(st, property.ty, value, &format!("`{}.{}`", def.name, property.name))?;
        let imp = &self.classes[class as usize - 1];
        let write = imp.set[member as usize - 1].as_ref().ok_or_else(|| {
            Fail::unsupported(format!(
                "`{}.{}` is in the catalog and not implemented yet",
                def.name, property.name
            ))
        })?;
        let state = self.target(st, obj)?.state;
        {
            let mut guard = state
                .try_borrow_mut()
                .map_err(|_| Fail::new(abi::E_BUSY, "the object is in use by a call that is running"))?;
            let mut cx = Cx::new(self, st, obj, class);
            if let Some(timing) = timing {
                cx = cx.with_timing(timing);
            }
            guarded(property.name, || write(&mut **guard, &mut cx, value))?;
        }
        self.queue_changed(st, obj, member);
        Ok(())
    }

    // ---- methods ---------------------------------------------------------------------------------------------------------

    /// Calls a method.
    pub fn call(&self, ctx: GnoblinCtx, obj: Obj, member: MemberId, args: Vec<Val>) -> Res<Val> {
        let st = self.state(ctx, false)?;
        let target = self.target(&st, obj)?;
        let method = self.member_def(target.class, member, abi::MEMBER_METHOD)?;
        if method.is_static() {
            return Err(Fail::type_error(format!(
                "`{}` is a function of the class: call it without an object",
                method.name
            )));
        }
        let def = self.class_def(target.class)?;
        if member as usize <= def.foreign_members {
            let (api, base) = self.forward(&st, ctx, obj)?;
            return api.call(base, member, &args);
        }
        let args = self.check_args(&st, def.name, method, args)?;
        let imp = &self.classes[target.class as usize - 1];
        let run = imp.call[member as usize - 1].as_ref().ok_or_else(|| {
            Fail::unsupported(format!(
                "`{}.{}` is in the catalog and not implemented yet",
                def.name, method.name
            ))
        })?;
        let answer = {
            let mut guard = target
                .state
                .try_borrow_mut()
                .map_err(|_| Fail::new(abi::E_BUSY, "the object is in use by a call that is running"))?;
            let mut cx = Cx::new(self, &st, obj, target.class);
            guarded(method.name, || run(&mut **guard, &mut cx, &args))?
        };
        args.iter().for_each(Val::adopt_closures);
        Ok(answer)
    }

    /// Calls a function of a class.
    pub fn call_static(&self, ctx: GnoblinCtx, class: ClassId, member: MemberId, args: Vec<Val>) -> Res<Val> {
        let st = self.state(ctx, true)?;
        let def = self.class_def(class)?;
        let method = self.member_def(class, member, abi::MEMBER_METHOD)?;
        if !method.is_static() {
            return Err(Fail::type_error(format!(
                "`{}.{}` needs an object",
                def.name, method.name
            )));
        }
        let args = self.check_args(&st, def.name, method, args)?;
        let imp = &self.classes[class as usize - 1];
        let run = imp.stat[member as usize - 1].as_ref().ok_or_else(|| {
            Fail::unsupported(format!(
                "`{}.{}` is in the catalog and not implemented yet",
                def.name, method.name
            ))
        })?;
        let mut cx = Cx::new(self, &st, 0, class);
        let answer = guarded(method.name, || run(&mut cx, &args))?;
        args.iter().for_each(Val::adopt_closures);
        Ok(answer)
    }

    // ---- listeners and the shell's implementations -------------------------------------------------------------------------

    /// Registers a listener of a signal or a property of `obj`. The library adopts `user` when this returns a registration.
    pub fn listen(
        &self,
        ctx: GnoblinCtx,
        obj: Obj,
        member: MemberId,
        invoke: GnoblinInvokeFn,
        user: *mut core::ffi::c_void,
        drop_user: Option<GnoblinDropFn>,
    ) -> Res<u64> {
        let st = self.state(ctx, false)?;
        let target = self.target(&st, obj)?;
        let def = self.class_def(target.class)?;
        let found = def
            .member(member)
            .ok_or_else(|| Fail::no_such(format!("class `{}` has no member with id {member}", def.name)))?;
        if !matches!(found.kind, abi::MEMBER_SIGNAL | abi::MEMBER_PROPERTY) {
            return Err(Fail::type_error(format!(
                "`{}.{}` is a {}: only a signal or a property can be listened to",
                def.name,
                found.name,
                kind_word(found.kind)
            )));
        }
        if member as usize <= def.foreign_members
            && let Some((home, _)) = def.foreign_base()
        {
            // A member of the base class: the base object's library hears it, and what it tells goes to the shell's closure.
            let (api, base) = self.forward(&st, ctx, obj)?;
            let closure = Rc::new(RawClosure::pending(invoke, user, drop_user));
            let run = Rc::clone(&closure);
            let foreign_reg = api.listen(base, member, move |member, args| run.call(member, args))?;
            closure.adopt();
            let reg = self.next_reg();
            st.forwarded.borrow_mut().insert(reg, (home, foreign_reg));
            return Ok(reg);
        }
        // A facade's signal listens to another object's: the redirect names it.
        let (obj, member) = self.redirected(&st, obj, target.class, member)?;
        // Nothing can fail from here: the closure takes `user`.
        let closure: Rc<dyn Callable> = Rc::new(RawClosure::owned(invoke, user, drop_user));
        Ok(self.attach(&st, obj, member, closure, true))
    }

    /// Where a listener of `member` of `obj` goes: the object and signal a redirect names, or the ones given.
    fn redirected(&self, st: &Rc<CtxState>, obj: Obj, class: ClassId, member: MemberId) -> Res<(Obj, MemberId)> {
        let imp = &self.classes[class as usize - 1];
        let Some(redirect) = imp.redirect.get(member as usize - 1).and_then(Option::as_ref) else {
            return Ok((obj, member));
        };
        let mut cx = Cx::new(self, st, obj, class);
        let (other, signal) = guarded("a redirected signal", || redirect(&mut cx))?;
        let other_class = self.target(st, other)?.class;
        let other_member = self.member_id(other_class, abi::MEMBER_SIGNAL, signal)?;
        Ok((other, other_member))
    }

    /// Adds a listener whose closure is already the library's. With `replay`, what the signal tells a new listener is taken now and
    /// told from the next turn.
    pub(crate) fn attach(
        &self,
        st: &Rc<CtxState>,
        obj: Obj,
        member: MemberId,
        closure: Rc<dyn Callable>,
        replay: bool,
    ) -> u64 {
        let reg = self.next_reg();
        let class = st.objects.borrow().get(obj).map(|entry| entry.class);
        st.listeners.borrow_mut().add(
            reg,
            Listener {
                reg,
                obj,
                member,
                closure: Rc::clone(&closure),
            },
        );
        let replay = class.filter(|_| replay).and_then(|class| {
            self.classes[class as usize - 1]
                .replay
                .get(member as usize - 1)
                .and_then(Option::as_ref)
                .cloned()
                .map(|replay| (class, replay))
        });
        if let Some((class, replay)) = replay {
            let mut cx = Cx::new(self, st, obj, class);
            let calls = catch_unwind(AssertUnwindSafe(|| replay(&mut cx))).unwrap_or_default();
            if !calls.is_empty() {
                let state = Rc::clone(st);
                self.queue_task(
                    st,
                    obj,
                    Box::new(move |cx| {
                        // A listener removed before the turn is told nothing.
                        if !state.listeners.borrow().by_reg.contains_key(&reg) {
                            return;
                        }
                        for args in calls {
                            args.iter().for_each(|arg| cx.hand_out(arg));
                            if let Err(fail) = closure.call(member, &args) {
                                cx.log(abi::LOG_WARN, &format!("a listener failed: {fail}"));
                            }
                        }
                    }),
                );
            }
        }
        reg
    }

    /// Ends a listener.
    pub fn unlisten(&self, ctx: GnoblinCtx, reg: u64) -> Res<()> {
        let st = self.state(ctx, false)?;
        let forwarded = st.forwarded.borrow_mut().remove(&reg);
        if let Some((home, foreign_reg)) = forwarded {
            return (self.hooks.foreign)(ctx, home)?.unlisten(foreign_reg);
        }
        let removed = st.listeners.borrow_mut().remove(reg);
        if removed.is_none() {
            return Err(Fail::stale("the listener is gone, or never was"));
        }
        drop(removed);
        Ok(())
    }

    /// Makes an object of the interface class `class` that runs in the shell.
    pub fn implement(
        &self,
        ctx: GnoblinCtx,
        class: ClassId,
        implemented: Vec<u64>,
        invoke: GnoblinInvokeFn,
        user: *mut core::ffi::c_void,
        drop_user: Option<GnoblinDropFn>,
    ) -> Res<Obj> {
        let st = self.state(ctx, true)?;
        let def = self.class_def(class)?;
        if def.kind != abi::CLASS_INTERFACE {
            return Err(Fail::type_error(format!(
                "`{}` is not an interface a shell can implement",
                def.name
            )));
        }
        let has = |member: u32| {
            implemented
                .get((member / 64) as usize)
                .is_some_and(|bits| bits & (1_u64 << (member % 64)) != 0)
        };
        let missing: Vec<&str> = def
            .members
            .iter()
            .enumerate()
            .filter(|(index, member)| member.flags & abi::MEMBER_REQUIRED != 0 && !has(*index as u32 + 1))
            .map(|(_, member)| member.name)
            .collect();
        if !missing.is_empty() {
            return Err(Fail::type_error(format!(
                "`{}` needs the shell to implement {}",
                def.name,
                missing.join(", ")
            )));
        }
        // Nothing can fail from here: the object takes `user`.
        let shell = Rc::new(ShellImpl::adopt(invoke, user, drop_user, implemented));
        let obj = st.objects.borrow_mut().insert(Entry {
            class,
            state: Rc::new(RefCell::new(Box::new(()))),
            shell: Some(shell),
            refs: 1,
            parent: None,
            children: Vec::new(),
            pinned: false,
            base: None,
        });
        Ok(obj)
    }

    /// Runs `work` now as a call about `obj` of a generation, for a library that has work of its own to do between calls (the core
    /// telling its model what the compositor sent). It is never run inside a call of the protocol, so nothing it queues is told here.
    ///
    /// # Errors
    ///
    /// The generation holds no objects of this library, or `obj` is gone.
    pub fn run_in<R>(&self, ctx: GnoblinCtx, obj: Obj, work: impl FnOnce(&mut Cx<'_>) -> R) -> Res<R> {
        let st = self.state(ctx, false)?;
        let class = self.target(&st, obj)?.class;
        let mut cx = Cx::new(self, &st, obj, class);
        Ok(work(&mut cx))
    }

    // ---- turns -----------------------------------------------------------------------------------------------------------

    fn schedule(&self, st: &CtxState) {
        if !st.scheduled.replace(true) {
            (self.hooks.schedule)(st.ctx);
        }
    }

    pub(crate) fn queue_signal(&self, st: &CtxState, obj: Obj, member: MemberId, args: Vec<Val>) {
        if !st.listeners.borrow().hears(obj, member) {
            return;
        }
        st.queue.borrow_mut().push(Pending::Signal { obj, member, args });
        self.schedule(st);
    }

    pub(crate) fn queue_changed(&self, st: &CtxState, obj: Obj, member: MemberId) {
        if !st.listeners.borrow().hears(obj, member) {
            return;
        }
        st.queue.borrow_mut().push(Pending::Changed { obj, member });
        self.schedule(st);
    }

    pub(crate) fn queue_task(&self, st: &CtxState, obj: Obj, run: Box<dyn FnOnce(&mut Cx<'_>)>) {
        st.queue.borrow_mut().push(Pending::Task { obj, run });
        self.schedule(st);
    }

    /// Runs what is queued: the signals and property changes go to their listeners, the tasks run. It is called from the turn the
    /// library posted, on the shell thread, outside any call, so a listener may call anything.
    pub fn flush(&self, ctx: GnoblinCtx) {
        let Ok(st) = self.state(ctx, false) else { return };
        for _ in 0..MAX_ROUNDS {
            let batch = std::mem::take(&mut *st.queue.borrow_mut());
            if batch.is_empty() {
                break;
            }
            let mut seen: Vec<(Obj, MemberId)> = Vec::new();
            // A property told once per turn: the last of its changes stands in for them all.
            let batch: Vec<Pending> = batch
                .into_iter()
                .rev()
                .filter(|pending| match pending {
                    Pending::Changed { obj, member } => {
                        if seen.contains(&(*obj, *member)) {
                            false
                        } else {
                            seen.push((*obj, *member));
                            true
                        }
                    }
                    _ => true,
                })
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            for pending in batch {
                self.run_pending(&st, pending);
            }
        }
        let more = !st.queue.borrow().is_empty();
        st.scheduled.set(false);
        if more {
            self.schedule(&st);
        }
    }

    fn run_pending(&self, st: &Rc<CtxState>, pending: Pending) {
        match pending {
            Pending::Signal { obj, member, args } => self.deliver(st, obj, member, args),
            Pending::Changed { obj, member } => {
                if let Ok(value) = self.get(st.ctx, obj, member) {
                    self.deliver(st, obj, member, vec![value.clone()]);
                    // The getter handed its objects out once; each listener took its own references.
                    self.release_all(st, &value);
                }
            }
            Pending::Task { obj, run } => {
                let Ok(class) = self.target(st, obj).map(|target| target.class) else {
                    return;
                };
                let mut cx = Cx::new(self, st, obj, class);
                if catch_unwind(AssertUnwindSafe(|| run(&mut cx))).is_err() {
                    (self.hooks.log)(st.ctx, abi::LOG_ERROR, "a task of the object library panicked");
                }
            }
        }
    }

    fn deliver(&self, st: &Rc<CtxState>, obj: Obj, member: MemberId, args: Vec<Val>) {
        let heard = st.listeners.borrow().heard_by(obj, member);
        for listener in heard {
            // A listener an earlier one removed hears nothing.
            if !st.listeners.borrow().by_reg.contains_key(&listener.reg) {
                continue;
            }
            // Each listener holds its own reference to every object it is handed.
            args.iter().for_each(|arg| self.hand_out_all(st, arg));
            if let Err(fail) = listener.closure.call(member, &args) {
                (self.hooks.log)(st.ctx, abi::LOG_WARN, &format!("a listener failed: {fail}"));
            }
        }
    }

    /// Gives back a reference for every object in a value.
    fn release_all(&self, st: &Rc<CtxState>, value: &Val) {
        match value {
            Val::Object { obj, .. } => self.release_ref(st, *obj),
            Val::List(items) => items.iter().for_each(|item| self.release_all(st, item)),
            Val::Record { fields, .. } => fields.iter().for_each(|field| self.release_all(st, field)),
            _ => {}
        }
    }
}

/// Runs `body` and turns a panic into a failure that names `what`.
fn guarded<R>(what: &str, body: impl FnOnce() -> Res<R>) -> Res<R> {
    catch_unwind(AssertUnwindSafe(body))
        .unwrap_or_else(|_| Err(Fail::new(abi::E_PANIC, format!("a panic was caught in `{what}`"))))
}

fn kind_word(kind: u32) -> &'static str {
    match kind {
        abi::MEMBER_PROPERTY => "property",
        abi::MEMBER_METHOD => "method",
        abi::MEMBER_SIGNAL => "signal",
        abi::MEMBER_VALUE => "value",
        _ => "member",
    }
}
