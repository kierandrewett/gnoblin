//! `Val` to and from the C ABI's `GnoblinValue`, and closures across the boundary.
//!
//! A `GnoblinValue` borrows: text and lists point at memory somebody else owns. [`to_raw`] builds one that points into the `Val` it is
//! given (text, bytes) and into an [`Arena`] (the arrays of a list or a record, the `GnoblinClosure` of a closure), so the caller keeps
//! both alive for the call. [`from_raw`] copies everything out at once, so what it returns owns its data.
//!
//! A closure that crosses to the other side carries a `user` pointer the receiver adopts when the call succeeds. The sender's
//! [`Arena`] therefore frees the closures it made unless [`Arena::finish`] says they were adopted, and a [`RawClosure`] (a closure the
//! other side handed over) frees nothing unless it was marked adopted.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinClosure, GnoblinDropFn, GnoblinInvokeFn, GnoblinStr, GnoblinValue};

use crate::error::{Fail, Res};
use crate::value::{Callable, ClassId, MemberId, Val};

/// A `GnoblinValue` with the given parts.
fn raw(kind: u32, bits: u64, ptr: *const c_void, len: usize) -> GnoblinValue {
    GnoblinValue {
        size: size_of::<GnoblinValue>() as u32,
        kind,
        flags: 0,
        reserved: 0,
        bits,
        ptr,
        len,
    }
}

/// A value that is none: what a caller sets `out` to before a call writes it.
pub fn none_value() -> GnoblinValue {
    raw(abi::VALUE_NONE, 0, core::ptr::null(), 0)
}

/// What an [`Arena`] made for a call: arrays that stay put until the call is over, and the `user` of every Rust closure it handed out.
#[derive(Default)]
pub struct Arena {
    lists: RefCell<Vec<Box<[GnoblinValue]>>>,
    // Boxed on purpose: the raw value points at the payload, which must not move when the vector grows.
    #[allow(clippy::vec_box)]
    closures: RefCell<Vec<Box<GnoblinClosure>>>,
    users: RefCell<Vec<*mut c_void>>,
    adopted: Cell<bool>,
}

impl Arena {
    /// An empty arena.
    pub fn new() -> Arena {
        Arena::default()
    }

    fn list(&self, values: Vec<GnoblinValue>) -> *const GnoblinValue {
        let boxed = values.into_boxed_slice();
        let ptr = boxed.as_ptr();
        self.lists.borrow_mut().push(boxed);
        ptr
    }

    /// Ends the call. When the library `adopted` the closures this arena handed out, it owns their `user` now; otherwise the arena
    /// frees them.
    pub fn finish(self, adopted: bool) {
        self.adopted.set(adopted);
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        if self.adopted.get() {
            return;
        }
        for user in self.users.borrow_mut().drain(..) {
            // SAFETY: `user` came from `Box::into_raw` in `closure_value`, and nobody adopted it, so this is its one free.
            unsafe { drop_rust_closure(user) };
        }
    }
}

/// The user pointer of a Rust closure handed across: a box holding the shared closure.
type Boxed = Rc<dyn Callable>;

unsafe extern "C" fn drop_rust_closure(user: *mut c_void) {
    if !user.is_null() {
        // SAFETY: `user` is a `Box<Boxed>` made by `closure_value`; the library calls this once.
        drop(unsafe { Box::from_raw(user.cast::<Boxed>()) });
    }
}

/// A shared closure as the three parts the ABI takes: the `invoke` that runs it, a `user` that owns one reference to it, and the
/// `drop_user` that frees that. The caller hands them over together, and frees `user` itself when the call that carried them fails.
pub(crate) fn export_closure(closure: &Rc<dyn Callable>) -> (GnoblinInvokeFn, *mut c_void, GnoblinDropFn) {
    let user = Box::into_raw(Box::new(Rc::clone(closure))).cast::<c_void>();
    (invoke_rust_closure, user, drop_rust_closure)
}

/// Frees a `user` made by [`export_closure`] that no library adopted.
pub(crate) fn free_closure_user(user: *mut c_void) {
    // SAFETY: `user` came from `export_closure` and nobody adopted it, so this is its one free.
    unsafe { drop_rust_closure(user) };
}

thread_local! {
    /// What the last answer lent: the value, kept alive with its arena until the next answer replaces it.
    static LENT: RefCell<Option<(Val, Arena)>> = const { RefCell::new(None) };
}

/// Lends `value` to the caller through `out`, valid until the next time this thread lends one: the rule of every answer of the
/// protocol. The previous answer's memory is freed now.
pub fn lend(value: Val, out: &mut GnoblinValue) {
    // The answer this one replaces is dropped after the slot is free again: dropping it may run a shell's `drop_user`.
    let previous = LENT.with(|slot| slot.borrow_mut().replace((value, Arena::new())));
    LENT.with(|slot| {
        if let Some((value, arena)) = slot.borrow().as_ref() {
            *out = to_raw(value, arena);
        }
    });
    drop(previous);
}

/// The raw form of `value`, pointing into `value` and `arena`: keep both alive for as long as the raw value is read.
pub fn to_raw(value: &Val, arena: &Arena) -> GnoblinValue {
    match value {
        Val::None => none_value(),
        Val::Bool(flag) => raw(abi::VALUE_BOOL, u64::from(*flag), core::ptr::null(), 0),
        Val::Int(number) => raw(abi::VALUE_INT, *number as u64, core::ptr::null(), 0),
        Val::Uint(number) => raw(abi::VALUE_UINT, *number, core::ptr::null(), 0),
        Val::Double(number) => raw(abi::VALUE_DOUBLE, number.to_bits(), core::ptr::null(), 0),
        Val::Text(text) => raw(abi::VALUE_TEXT, 0, text.as_ptr().cast(), text.len()),
        Val::Secret(text) => raw(abi::VALUE_SECRET, 0, text.as_ptr().cast(), text.len()),
        Val::Bytes(bytes) => raw(abi::VALUE_BYTES, 0, bytes.as_ptr().cast(), bytes.len()),
        Val::Node(id) => raw(abi::VALUE_NODE, *id, core::ptr::null(), 0),
        Val::List(items) => {
            let raws: Vec<GnoblinValue> = items.iter().map(|item| to_raw(item, arena)).collect();
            let len = raws.len();
            raw(abi::VALUE_LIST, 0, arena.list(raws).cast(), len)
        }
        Val::Record { class, fields } => {
            let raws: Vec<GnoblinValue> = fields.iter().map(|field| to_raw(field, arena)).collect();
            let len = raws.len();
            raw(abi::VALUE_RECORD, u64::from(*class), arena.list(raws).cast(), len)
        }
        Val::Object { class, obj } => raw(abi::VALUE_OBJECT, *obj, core::ptr::null(), *class as usize),
        Val::Foreign { namespace, obj } => raw(
            abi::VALUE_FOREIGN_OBJECT,
            *obj,
            namespace.as_ptr().cast(),
            namespace.len(),
        ),
        Val::Enum { class, value } => raw(abi::VALUE_ENUM, u64::from(*value), core::ptr::null(), *class as usize),
        Val::Closure(closure) => closure_value(closure, arena),
    }
}

/// A closure as a value: its `user` is a box holding the shared closure, freed by `drop_user` when the library lets go.
fn closure_value(closure: &Rc<dyn Callable>, arena: &Arena) -> GnoblinValue {
    let user = Box::into_raw(Box::new(Rc::clone(closure))).cast::<c_void>();
    arena.users.borrow_mut().push(user);
    let payload = Box::new(GnoblinClosure {
        size: size_of::<GnoblinClosure>() as u32,
        reserved: 0,
        invoke: Some(invoke_rust_closure),
        user,
        drop_user: Some(drop_rust_closure),
    });
    let ptr: *const GnoblinClosure = &*payload;
    arena.closures.borrow_mut().push(payload);
    raw(abi::VALUE_CLOSURE, 0, ptr.cast(), 0)
}

/// The `invoke` of a Rust closure crossing to the other side: converts the arguments, runs the closure, lends its answer.
unsafe extern "C" fn invoke_rust_closure(
    user: *mut c_void,
    member: MemberId,
    args: *const GnoblinValue,
    n_args: usize,
    out: *mut GnoblinValue,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user` is the `Box<Boxed>` of `closure_value`, alive until its `drop_user`; `args` is `n_args` readable values.
        let (closure, values) = unsafe {
            let closure = Rc::clone(&*user.cast::<Boxed>());
            (closure, read_args(args, n_args, false))
        };
        let answer = values.and_then(|values| closure.call(member, &values));
        match answer {
            Ok(value) => {
                // SAFETY: `out` is the writable value the caller set to none.
                if let Some(out) = unsafe { out.as_mut() } {
                    lend(value, out);
                }
                abi::E_OK
            }
            Err(fail) => {
                // The callee's own words for the failure, where the caller may read them: the code says only which kind it was.
                // SAFETY: `out` is the writable value the caller set to none.
                if let Some(out) = unsafe { out.as_mut() } {
                    lend(Val::Text(fail.message), out);
                }
                fail.code
            }
        }
    }))
    .unwrap_or(abi::E_PANIC)
}

/// Reads `n` values at `args`.
///
/// # Safety
///
/// `args` must be null (with `n` zero) or point at `n` readable values.
pub unsafe fn read_args(args: *const GnoblinValue, n: usize, adopting: bool) -> Res<Vec<Val>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    if args.is_null() {
        return Err(Fail::err("a null argument array with a nonzero count"));
    }
    // SAFETY: the caller guarantees `n` readable values at `args`.
    let slice = unsafe { std::slice::from_raw_parts(args, n) };
    slice
        .iter()
        // SAFETY: each element is a value the caller built for this call.
        .map(|value| unsafe { from_raw(value, adopting) })
        .collect()
}

/// The text `len` bytes at `ptr`.
///
/// # Safety
///
/// `ptr` must be null (with `len` zero) or point at `len` readable bytes.
unsafe fn text_at(ptr: *const c_void, len: usize) -> Res<String> {
    if len == 0 {
        return Ok(String::new());
    }
    if ptr.is_null() {
        return Err(Fail::err("a null text with a nonzero length"));
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    String::from_utf8(bytes.to_vec()).map_err(|_| Fail::type_error("text must be UTF-8"))
}

/// A value read out of its raw form, copied. With `adopting` the closures in it are the library's to free once it adopts them.
///
/// # Safety
///
/// `value` must be a well-formed `GnoblinValue`: its pointers readable for its lengths, its closure payloads and arrays alive for
/// the call.
pub unsafe fn from_raw(value: &GnoblinValue, adopting: bool) -> Res<Val> {
    match value.kind {
        0 | abi::VALUE_NONE => Ok(Val::None),
        abi::VALUE_BOOL => Ok(Val::Bool(value.bits != 0)),
        abi::VALUE_INT => Ok(Val::Int(value.bits as i64)),
        abi::VALUE_UINT => Ok(Val::Uint(value.bits)),
        abi::VALUE_DOUBLE => Ok(Val::Double(f64::from_bits(value.bits))),
        // SAFETY: forwarded from the caller's guarantee about `ptr` and `len`.
        abi::VALUE_TEXT => unsafe { text_at(value.ptr, value.len) }.map(Val::Text),
        // SAFETY: as above.
        abi::VALUE_SECRET => unsafe { text_at(value.ptr, value.len) }.map(Val::Secret),
        abi::VALUE_NODE => Ok(Val::Node(value.bits)),
        abi::VALUE_BYTES => {
            if value.len == 0 {
                return Ok(Val::Bytes(Vec::new()));
            }
            if value.ptr.is_null() {
                return Err(Fail::err("null bytes with a nonzero length"));
            }
            // SAFETY: `len` readable bytes at `ptr`, by the caller's guarantee.
            Ok(Val::Bytes(
                unsafe { std::slice::from_raw_parts(value.ptr.cast::<u8>(), value.len) }.to_vec(),
            ))
        }
        // SAFETY: `ptr` points at `len` values, by the caller's guarantee.
        abi::VALUE_LIST => unsafe { read_args(value.ptr.cast(), value.len, adopting) }.map(Val::List),
        abi::VALUE_TEXTS => {
            if value.len == 0 {
                return Ok(Val::List(Vec::new()));
            }
            if value.ptr.is_null() {
                return Err(Fail::err("a null list of texts with a nonzero length"));
            }
            // SAFETY: `ptr` points at `len` `GnoblinStr`s, by the caller's guarantee.
            let strs = unsafe { std::slice::from_raw_parts(value.ptr.cast::<GnoblinStr>(), value.len) };
            strs.iter()
                // SAFETY: each string is readable for its length.
                .map(|text| unsafe { text_at(text.ptr.cast(), text.len) }.map(Val::Text))
                .collect::<Res<Vec<_>>>()
                .map(Val::List)
        }
        abi::VALUE_RECORD => {
            // SAFETY: `ptr` points at `len` fields, by the caller's guarantee.
            let fields = unsafe { read_args(value.ptr.cast(), value.len, adopting) }?;
            Ok(Val::Record {
                class: value.bits as ClassId,
                fields,
            })
        }
        abi::VALUE_OBJECT => Ok(Val::Object {
            class: value.len as ClassId,
            obj: value.bits,
        }),
        abi::VALUE_FOREIGN_OBJECT => {
            // SAFETY: `ptr` holds `len` bytes of the namespace, by the caller's guarantee.
            let namespace = unsafe { text_at(value.ptr, value.len) }?;
            if namespace.is_empty() {
                return Err(Fail::type_error("an object of another library that names no namespace"));
            }
            Ok(Val::Foreign {
                namespace,
                obj: value.bits,
            })
        }
        abi::VALUE_ENUM => Ok(Val::Enum {
            class: value.len as ClassId,
            value: value.bits as u32,
        }),
        abi::VALUE_CLOSURE => {
            // SAFETY: `ptr` is a `GnoblinClosure` the sender keeps alive for the call.
            let payload = unsafe { value.ptr.cast::<GnoblinClosure>().as_ref() };
            let Some(payload) = payload else {
                return Err(Fail::type_error("a closure value with no payload"));
            };
            let Some(invoke) = payload.invoke else {
                return Err(Fail::type_error("a closure with no function"));
            };
            Ok(Val::Closure(Rc::new(RawClosure {
                invoke,
                user: payload.user,
                drop_user: payload.drop_user,
                own: adopting,
                adopted: Cell::new(false),
            })))
        }
        other => Err(Fail::unsupported(format!(
            "the value kind {other} is not one this library reads"
        ))),
    }
}

/// A closure the other side handed over: it runs through the `invoke` it gave. It frees `user` only when it was handed over to be
/// owned (`adopting`) and the call that carried it succeeded ([`Callable::adopt`]).
pub struct RawClosure {
    invoke: GnoblinInvokeFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    own: bool,
    adopted: Cell<bool>,
}

impl RawClosure {
    /// A closure the library will own once the call that carries it succeeds: until [`Callable::adopt`], dropping it frees nothing.
    pub(crate) fn pending(invoke: GnoblinInvokeFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> RawClosure {
        RawClosure {
            invoke,
            user,
            drop_user,
            own: true,
            adopted: Cell::new(false),
        }
    }

    /// A listener the shell registered: the library owns `user` from the start and frees it when the closure is dropped.
    pub(crate) fn owned(invoke: GnoblinInvokeFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> RawClosure {
        RawClosure {
            invoke,
            user,
            drop_user,
            own: true,
            adopted: Cell::new(true),
        }
    }
}

impl Callable for RawClosure {
    fn call(&self, member: MemberId, args: &[Val]) -> Res<Val> {
        let arena = Arena::new();
        let raws: Vec<GnoblinValue> = args.iter().map(|arg| to_raw(arg, &arena)).collect();
        let mut out = none_value();
        // SAFETY: `invoke` and `user` are what the other side gave; `raws` and the arena outlive the call.
        let code = unsafe { (self.invoke)(self.user, member, raws.as_ptr(), raws.len(), &raw mut out) };
        arena.finish(false);
        if code != abi::E_OK {
            // A callee that gives its words writes them to `out` as text; one that does not leaves the none the call set.
            // SAFETY: `out` is what the callee wrote, valid until our next call into it, and the text is copied here.
            let words = if out.kind == abi::VALUE_TEXT {
                unsafe { text_at(out.ptr, out.len) }.ok()
            } else {
                None
            };
            return Err(Fail::new(
                code,
                words.unwrap_or_else(|| "the callback failed".to_owned()),
            ));
        }
        // SAFETY: `out` was written by the callee and stays valid until our next call into it, which is after this copy.
        unsafe { from_raw(&out, false) }
    }

    fn adopt(&self) {
        if self.own {
            self.adopted.set(true);
        }
    }
}

impl Drop for RawClosure {
    fn drop(&mut self) {
        if self.adopted.get()
            && let Some(drop_user) = self.drop_user
        {
            // SAFETY: the other side gave `user` with its `drop_user`; the library adopted it and this is its one call.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop_user(self.user) }));
        }
    }
}
