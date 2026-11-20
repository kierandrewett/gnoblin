//! The client half of the object protocol: how an SDK or a script runtime calls a library's `GnoblinObjectApi`.
//!
//! [`Api`] is the table of one library for one generation, with the conversions to and from [`Val`] done once. A generated typed
//! class holds an [`Object`] (a reference that is released when it is dropped) and calls through it; a script runtime holds the same
//! and converts its own values to and from `Val`. Nothing here knows a class: a name is looked up once and its id is kept.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core_abi::{
    self as abi, GnoblinCatalogEntry, GnoblinCtx, GnoblinImpl, GnoblinInit, GnoblinObjectApi, GnoblinReg, GnoblinStr,
    GnoblinTiming, GnoblinValue,
};

use crate::error::{Fail, Res};
use crate::value::{Callable, ClassId, Closure, MemberId, Obj, Timing, Val};
use crate::wire::{Arena, export_closure, free_closure_user, from_raw, none_value, to_raw};

/// A closure made from a Rust function.
/// The Rust function a closure value runs.
type RunFn = Box<dyn FnMut(MemberId, &[Val]) -> Res<Val>>;

struct FnClosure {
    run: RefCell<RunFn>,
}

impl Callable for FnClosure {
    fn call(&self, member: MemberId, args: &[Val]) -> Res<Val> {
        let mut run = self
            .run
            .try_borrow_mut()
            .map_err(|_| Fail::new(abi::E_BUSY, "the closure was called again while it was running"))?;
        run(member, args)
    }
}

/// A closure value that runs `run`. Give it to a library as an argument (`Val::Closure`) or to [`Api::listen`].
pub fn closure(run: impl FnMut(MemberId, &[Val]) -> Res<Val> + 'static) -> Closure {
    Rc::new(FnClosure {
        run: RefCell::new(Box::new(run)),
    })
}

/// A registration of a listener.
pub type Reg = GnoblinReg;

/// One entry of a library's catalog, copied out of `catalog_each`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogItem {
    /// `CATALOG_CLASS` or `CATALOG_MEMBER`.
    pub entry: u32,
    /// The class, for either.
    pub class: ClassId,
    /// The member id, for a member.
    pub member: MemberId,
    /// A class's `CLASS_*` kind or a member's `MEMBER_*` kind.
    pub kind: u32,
    /// The flags.
    pub flags: u32,
    /// A class's base class.
    pub base: ClassId,
    /// A property's or a method's value kind.
    pub value_kind: u32,
    /// The class that value kind names.
    pub value_class: ClassId,
    /// The argument value kinds.
    pub arg_kinds: Vec<u32>,
    /// The argument names.
    pub arg_names: Vec<String>,
    /// The name.
    pub name: String,
}

/// One library's object protocol, for one generation.
#[derive(Clone, Copy)]
pub struct Api {
    raw: &'static GnoblinObjectApi,
    ctx: GnoblinCtx,
    last_error: fn() -> String,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Api(ctx {}, catalog {:#018x})", self.ctx, self.raw.catalog_hash)
    }
}

impl Api {
    /// The protocol of the table `raw` (from a library's `object_api`) for the generation `ctx`. `last_error` reads the library's
    /// last-error text, which is what a failed call's [`Fail`] carries.
    pub fn new(raw: &'static GnoblinObjectApi, ctx: GnoblinCtx, last_error: fn() -> String) -> Api {
        Api { raw, ctx, last_error }
    }

    /// The generation.
    pub fn ctx(&self) -> GnoblinCtx {
        self.ctx
    }

    /// The hash of the catalog the library was built with.
    pub fn catalog_hash(&self) -> u64 {
        self.raw.catalog_hash
    }

    fn check(&self, code: i32) -> Res<()> {
        if code == abi::E_OK {
            return Ok(());
        }
        let text = (self.last_error)();
        Err(Fail::new(
            code,
            if text.is_empty() {
                format!("the call failed with code {code}")
            } else {
                text
            },
        ))
    }

    /// The id of the class `name`.
    ///
    /// # Errors
    ///
    /// The catalog has no such class.
    pub fn class(&self, name: &str) -> Res<ClassId> {
        let mut id = 0;
        // SAFETY: `name` lives for the call and `id` is a live local.
        let code = unsafe { (self.raw.class_id)(self.ctx, GnoblinStr::borrowed(name), &raw mut id) };
        self.check(code)?;
        Ok(id)
    }

    /// The id of the member `name` of `kind` of `class`.
    ///
    /// # Errors
    ///
    /// The class has no such member.
    pub fn member(&self, class: ClassId, kind: u32, name: &str) -> Res<MemberId> {
        let mut id = 0;
        // SAFETY: as for `class`.
        let code = unsafe { (self.raw.member_id)(self.ctx, class, kind, GnoblinStr::borrowed(name), &raw mut id) };
        self.check(code)?;
        Ok(id)
    }

    /// The whole catalog, copied.
    ///
    /// # Errors
    ///
    /// The library refuses.
    pub fn catalog(&self) -> Res<Vec<CatalogItem>> {
        struct Walk(Vec<CatalogItem>);
        unsafe extern "C" fn collect(user: *mut c_void, entry: *const GnoblinCatalogEntry) -> i32 {
            // SAFETY: `user` is the `Walk` of `catalog`, and `entry` is valid for the callback; its strings are the library's statics.
            let (walk, entry) = unsafe { (&mut *user.cast::<Walk>(), &*entry) };
            let text = |text: GnoblinStr| -> String {
                if text.ptr.is_null() || text.len == 0 {
                    String::new()
                } else {
                    // SAFETY: the library lends `len` readable bytes at `ptr`.
                    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text.ptr, text.len) }).into_owned()
                }
            };
            let kinds = if entry.arg_kinds.is_null() || entry.arg_count == 0 {
                Vec::new()
            } else {
                // SAFETY: the library lends `arg_count` kinds at `arg_kinds`.
                unsafe { std::slice::from_raw_parts(entry.arg_kinds, entry.arg_count as usize) }.to_vec()
            };
            let names = text(entry.arg_names);
            walk.0.push(CatalogItem {
                entry: entry.entry,
                class: entry.class,
                member: entry.member,
                kind: entry.kind,
                flags: entry.flags,
                base: entry.base,
                value_kind: entry.value_kind,
                value_class: entry.value_class,
                arg_kinds: kinds,
                arg_names: names.split_whitespace().map(str::to_owned).collect(),
                name: text(entry.name),
            });
            abi::E_OK
        }
        let mut walk = Walk(Vec::new());
        // SAFETY: `collect` reads `user` as the `Walk` it is given.
        let code = unsafe { (self.raw.catalog_each)(self.ctx, collect, (&raw mut walk).cast()) };
        self.check(code)?;
        Ok(walk.0)
    }

    /// Makes an object of `class` with `inits` set. The handle carries one reference: wrap it in an [`Object`] to release it.
    ///
    /// # Errors
    ///
    /// The library refuses: a value of the wrong type, a class that cannot be made.
    pub fn create(&self, class: ClassId, inits: &[(MemberId, Val)]) -> Res<Obj> {
        let arena = Arena::new();
        let raws: Vec<GnoblinInit> = inits
            .iter()
            .map(|(member, value)| GnoblinInit {
                member: *member,
                reserved: 0,
                value: to_raw(value, &arena),
            })
            .collect();
        let mut obj = 0;
        // SAFETY: `raws`, the values they point at and the arena outlive the call; `obj` is a live local.
        let code = unsafe { (self.raw.create)(self.ctx, class, raws.as_ptr(), raws.len(), &raw mut obj) };
        arena.finish(code == abi::E_OK);
        self.check(code)?;
        Ok(obj)
    }

    /// Ends an object and its children.
    ///
    /// # Errors
    ///
    /// The object is gone.
    pub fn destroy(&self, obj: Obj) -> Res<()> {
        // SAFETY: the call takes plain values.
        self.check(unsafe { (self.raw.destroy)(self.ctx, obj) })
    }

    /// Drops one reference to an object.
    ///
    /// # Errors
    ///
    /// The object is gone.
    pub fn release(&self, obj: Obj) -> Res<()> {
        // SAFETY: the call takes plain values.
        self.check(unsafe { (self.raw.release)(self.ctx, obj) })
    }

    /// Reads a property.
    ///
    /// # Errors
    ///
    /// The object is gone, or the library refuses.
    pub fn get(&self, obj: Obj, member: MemberId) -> Res<Val> {
        let mut out = none_value();
        // SAFETY: `out` is a live local the library writes.
        let code = unsafe { (self.raw.get)(self.ctx, obj, member, &raw mut out) };
        self.check(code)?;
        // SAFETY: the library lent `out` until our next call into it, which is after this copy.
        unsafe { from_raw(&out, false) }
    }

    /// Sets a property.
    ///
    /// # Errors
    ///
    /// The value is of the wrong type, the property is read-only, or the object is gone.
    pub fn set(&self, obj: Obj, member: MemberId, value: &Val) -> Res<()> {
        let arena = Arena::new();
        let wire = to_raw(value, &arena);
        // SAFETY: `raw` and the arena outlive the call.
        let code = unsafe { (self.raw.set)(self.ctx, obj, member, &raw const wire) };
        arena.finish(code == abi::E_OK);
        self.check(code)
    }

    /// Sets several properties, in order, stopping at the first that fails.
    ///
    /// # Errors
    ///
    /// As for [`Api::set`].
    pub fn update(&self, obj: Obj, inits: &[(MemberId, Val)]) -> Res<()> {
        let arena = Arena::new();
        let raws: Vec<GnoblinInit> = inits
            .iter()
            .map(|(member, value)| GnoblinInit {
                member: *member,
                reserved: 0,
                value: to_raw(value, &arena),
            })
            .collect();
        // SAFETY: `raws` and the arena outlive the call.
        let code = unsafe { (self.raw.update)(self.ctx, obj, raws.as_ptr(), raws.len()) };
        arena.finish(code == abi::E_OK);
        self.check(code)
    }

    /// Sets an animated property as a timed write.
    ///
    /// # Errors
    ///
    /// As for [`Api::set`], and the property must be animated.
    pub fn set_timed(&self, obj: Obj, member: MemberId, value: &Val, timing: &Timing) -> Res<()> {
        let arena = Arena::new();
        let wire = to_raw(value, &arena);
        let curve = GnoblinTiming {
            size: size_of::<GnoblinTiming>() as u32,
            reserved: 0,
            seconds: timing.seconds,
            delay: timing.delay,
            tween: timing.tween,
            samples: timing.samples.as_ptr(),
            n_samples: timing.samples.len(),
        };
        // SAFETY: `raw`, `curve`, the samples and the arena outlive the call.
        let code = unsafe { (self.raw.set_timed)(self.ctx, obj, member, &raw const wire, &raw const curve) };
        arena.finish(code == abi::E_OK);
        self.check(code)
    }

    /// Calls a method.
    ///
    /// # Errors
    ///
    /// The arguments do not fit the member, or the method fails.
    pub fn call(&self, obj: Obj, member: MemberId, args: &[Val]) -> Res<Val> {
        let arena = Arena::new();
        let raws: Vec<GnoblinValue> = args.iter().map(|arg| to_raw(arg, &arena)).collect();
        let mut out = none_value();
        // SAFETY: `raws` and the arena outlive the call; `out` is a live local.
        let code = unsafe { (self.raw.call)(self.ctx, obj, member, raws.as_ptr(), raws.len(), &raw mut out) };
        arena.finish(code == abi::E_OK);
        self.check(code)?;
        // SAFETY: the library lent `out` until our next call into it.
        unsafe { from_raw(&out, false) }
    }

    /// Calls a function of a class.
    ///
    /// # Errors
    ///
    /// As for [`Api::call`].
    pub fn call_static(&self, class: ClassId, member: MemberId, args: &[Val]) -> Res<Val> {
        let arena = Arena::new();
        let raws: Vec<GnoblinValue> = args.iter().map(|arg| to_raw(arg, &arena)).collect();
        let mut out = none_value();
        // SAFETY: as for `call`.
        let code = unsafe { (self.raw.call_static)(self.ctx, class, member, raws.as_ptr(), raws.len(), &raw mut out) };
        arena.finish(code == abi::E_OK);
        self.check(code)?;
        // SAFETY: the library lent `out` until our next call into it.
        unsafe { from_raw(&out, false) }
    }

    /// Listens to a signal or a property of `obj`. `run` is called from the library's next turn with the member and its payload; an
    /// object in the payload is a reference it must release. The listener lasts until [`Api::unlisten`] or the generation's end.
    ///
    /// # Errors
    ///
    /// The object is gone, or the member is neither a signal nor a property.
    pub fn listen(
        &self,
        obj: Obj,
        member: MemberId,
        run: impl FnMut(MemberId, &[Val]) -> Res<Val> + 'static,
    ) -> Res<Reg> {
        let (invoke, user, drop_user) = export_closure(&closure(run));
        // SAFETY: `invoke`, `user` and `drop_user` go together; the library adopts them when it returns a registration.
        let reg = unsafe { (self.raw.listen)(self.ctx, obj, member, invoke, user, Some(drop_user)) };
        if reg == 0 {
            free_closure_user(user);
            return Err(Fail::new(abi::E_ERR, (self.last_error)()));
        }
        Ok(reg)
    }

    /// Ends a listener.
    ///
    /// # Errors
    ///
    /// The registration is gone.
    pub fn unlisten(&self, reg: Reg) -> Res<()> {
        // SAFETY: the call takes plain values.
        self.check(unsafe { (self.raw.unlisten)(self.ctx, reg) })
    }

    /// Makes an object of the interface `class` whose members `implemented` run `run`. The library calls `run` from its turn.
    ///
    /// # Errors
    ///
    /// The class is not an interface, or a required member is missing from `implemented`.
    pub fn implement(
        &self,
        class: ClassId,
        implemented: &[MemberId],
        run: impl FnMut(MemberId, &[Val]) -> Res<Val> + 'static,
    ) -> Res<Obj> {
        let words = implemented
            .iter()
            .map(|member| *member as usize / 64 + 1)
            .max()
            .unwrap_or(0);
        let mut mask = vec![0_u64; words];
        for member in implemented {
            mask[*member as usize / 64] |= 1_u64 << (member % 64);
        }
        let (invoke, user, drop_user) = export_closure(&closure(run));
        let imp = GnoblinImpl {
            size: size_of::<GnoblinImpl>() as u32,
            reserved: 0,
            implemented: mask.as_ptr(),
            n_words: mask.len(),
            invoke: Some(invoke),
            user,
            drop_user: Some(drop_user),
        };
        let mut obj = 0;
        // SAFETY: `imp` and the mask outlive the call; the library adopts `user` when it succeeds.
        let code = unsafe { (self.raw.implement)(self.ctx, class, &raw const imp, &raw mut obj) };
        if code != abi::E_OK {
            free_closure_user(user);
        }
        self.check(code)?;
        Ok(obj)
    }

    /// The most derived class of an object.
    ///
    /// # Errors
    ///
    /// The object is gone.
    pub fn class_of(&self, obj: Obj) -> Res<ClassId> {
        let mut class = 0;
        // SAFETY: `class` is a live local.
        let code = unsafe { (self.raw.class_of)(self.ctx, obj, &raw mut class) };
        self.check(code)?;
        Ok(class)
    }
}

/// An object, held by one reference that is released when the value is dropped.
#[derive(Debug)]
pub struct Object {
    api: Api,
    class: ClassId,
    obj: Obj,
}

impl Object {
    /// Takes over the reference a call returned for `obj`.
    pub fn adopt(api: Api, class: ClassId, obj: Obj) -> Object {
        Object { api, class, obj }
    }

    /// Takes over the reference an object value carries, as a call's result or a callback's argument hands it out.
    ///
    /// # Errors
    ///
    /// The value is not an object.
    pub fn from_val(api: Api, value: &Val) -> Res<Object> {
        match value {
            Val::Object { class, obj } => Ok(Object::adopt(api, *class, *obj)),
            other => Err(Fail::type_error(format!(
                "expected an object, got {}",
                other.kind_name()
            ))),
        }
    }

    /// Makes an object of `class` and holds it.
    ///
    /// # Errors
    ///
    /// As for [`Api::create`].
    pub fn create(api: Api, class: ClassId, inits: &[(MemberId, Val)]) -> Res<Object> {
        let obj = api.create(class, inits)?;
        Ok(Object { api, class, obj })
    }

    /// The protocol the object is served through.
    pub fn api(&self) -> Api {
        self.api
    }

    /// The handle.
    pub fn handle(&self) -> Obj {
        self.obj
    }

    /// The class the object was made or handed out as.
    pub fn class(&self) -> ClassId {
        self.class
    }

    /// The object as a value to pass to a call. It holds no reference of its own: the receiver takes what it keeps.
    pub fn val(&self) -> Val {
        Val::Object {
            class: self.class,
            obj: self.obj,
        }
    }

    /// Keeps the reference past this value: the handle, which the caller now releases.
    pub fn into_handle(self) -> Obj {
        let obj = self.obj;
        std::mem::forget(self);
        obj
    }

    /// Ends the object and its children, whoever holds it.
    ///
    /// # Errors
    ///
    /// The object is gone.
    pub fn destroy(&self) -> Res<()> {
        self.api.destroy(self.obj)
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        let _ = self.api.release(self.obj);
    }
}
