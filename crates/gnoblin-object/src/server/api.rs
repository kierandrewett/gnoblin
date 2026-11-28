//! The extern "C" functions of `GnoblinObjectApi`, written once for every library.
//!
//! A library names itself with a [`Provider`] (where its [`Library`] lives) and builds its table with [`api_table`]; each function
//! here reads the raw arguments, calls the operation of [`Library`], and turns the result into the ABI's integer codes. Every body
//! runs inside a panic guard, so a panic is `E_PANIC` and a last-error text, never an abort.
#![allow(unsafe_code)]

use core::cell::Cell;
use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi::{
    self as abi, GnoblinCatalogEntry, GnoblinCatalogFn, GnoblinClassId, GnoblinCtx, GnoblinDropFn, GnoblinImpl,
    GnoblinInit, GnoblinInvokeFn, GnoblinMemberId, GnoblinObj, GnoblinObjectApi, GnoblinReg, GnoblinStr, GnoblinTiming,
    GnoblinValue,
};

use super::Library;
use crate::catalog::{ClassDef, MemberDef};
use crate::error::{ErrorCode, Fail, Res};
use crate::value::{MemberId, Timing, Val};
use crate::wire::{from_raw, lend, read_args};

/// Where a library keeps its [`Library`]: one per thread, in a `thread_local!` the library owns.
pub trait Provider: 'static {
    /// Runs `work` with the library, or answers `None` when the library is gone: the thread's slot is being destroyed and so is the
    /// library in it. That is a legitimate state, not a bug in the caller. Dropping the library drops every closure and listener it
    /// still holds, which runs the shell's `drop_user` of each, and what a shell drops there may be an object it holds (`release`) or a
    /// listener (`unlisten`): calls into the table of the very library that is being dropped, from inside its own destructor. A
    /// `thread_local!` answers none of them, so an implementation uses `LocalKey::try_with` and gives `None` for the error.
    fn with<R>(work: impl FnOnce(&Library) -> R) -> Option<R>;
}

/// The table of a library: every function of `GnoblinObjectApi`, bound to `P`. `hash` is the catalog's.
pub const fn api_table<P: Provider>(hash: u64) -> GnoblinObjectApi {
    GnoblinObjectApi {
        size: size_of::<GnoblinObjectApi>() as u32,
        reserved: 0,
        catalog_hash: hash,
        class_id: class_id::<P>,
        member_id: member_id::<P>,
        catalog_each: catalog_each::<P>,
        create: create::<P>,
        destroy: destroy::<P>,
        release: release::<P>,
        get: get::<P>,
        set: set::<P>,
        update: update::<P>,
        set_timed: set_timed::<P>,
        call: call::<P>,
        call_static: call_static::<P>,
        listen: listen::<P>,
        unlisten: unlisten::<P>,
        implement: implement::<P>,
        class_of: class_of::<P>,
        track: track::<P>,
        last_error_code,
        retain: retain::<P>,
    }
}

thread_local! {
    /// The script API's code of the last object function that failed on this thread, or none when it succeeded or failed with a bad
    /// argument. `last_error_code` of the table reads it.
    static LAST_CODE: Cell<Option<ErrorCode>> = const { Cell::new(None) };
}

/// Runs one function: contains a panic, records a failure and its code, returns the integer code. A library that is gone
/// ([`Provider::with`] gives none) answers `E_RETIRED`, the answer of an object of a generation that retired: every object it served
/// went with it, so there is nothing for the call to act on.
fn run<P: Provider>(what: &'static str, body: impl FnOnce(&Library) -> Res<()>) -> i32 {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        P::with(|lib| match body(lib) {
            Ok(()) => {
                LAST_CODE.with(|last| last.set(None));
                abi::E_OK
            }
            Err(fail) => {
                (lib.hooks.set_last)(&fail);
                LAST_CODE.with(|last| last.set(fail.reason()));
                fail.code
            }
        })
    }));
    match outcome {
        Ok(Some(code)) => code,
        Ok(None) => {
            LAST_CODE.with(|last| last.set(ErrorCode::of_abi(abi::E_RETIRED)));
            abi::E_RETIRED
        }
        Err(_) => {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                P::with(|lib| {
                    let fail = Fail::new(
                        abi::E_PANIC,
                        format!("a panic was caught in the object function `{what}`"),
                    );
                    (lib.hooks.set_last)(&fail);
                    LAST_CODE.with(|last| last.set(fail.reason()));
                });
            }));
            abi::E_PANIC
        }
    }
}

/// `last_error_code(buf, cap)` of the table: copies the code of the last failed call on this thread into `buf`, NUL terminated and
/// truncated to `cap` bytes, and returns the code's full length (zero, with `buf` untouched, when there is none).
///
/// # Safety
///
/// `buf` must be null or point at `cap` writable bytes.
unsafe extern "C" fn last_error_code(buf: *mut u8, cap: usize) -> usize {
    let code = LAST_CODE.with(Cell::get).map_or("", ErrorCode::as_str);
    if !code.is_empty() && !buf.is_null() && cap > 0 {
        let kept = code.len().min(cap - 1);
        // SAFETY: the caller guarantees `buf` is writable for `cap` bytes; `kept + 1 <= cap`, and `code` is static text that cannot
        // overlap the caller's buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(code.as_ptr(), buf, kept);
            buf.add(kept).write(0);
        }
    }
    code.len()
}

/// [`run`] for a function that returns a registration: zero is the failure.
fn run_reg<P: Provider>(what: &'static str, body: impl FnOnce(&Library) -> Res<u64>) -> GnoblinReg {
    let mut reg = 0;
    let code = run::<P>(what, |lib| {
        reg = body(lib)?;
        Ok(())
    });
    if code == abi::E_OK { reg } else { 0 }
}

/// The text a caller lent.
///
/// # Safety
///
/// `text` must be null with length zero, or point at `len` readable bytes.
unsafe fn text_of<'a>(text: GnoblinStr) -> Res<&'a str> {
    if text.len == 0 {
        return Ok("");
    }
    if text.ptr.is_null() {
        return Err(Fail::err("a null string with a nonzero length"));
    }
    // SAFETY: the caller guarantees `len` readable bytes.
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(text.ptr, text.len) })
        .map_err(|_| Fail::type_error("text must be UTF-8"))
}

/// Writes through an output pointer, or refuses a null one.
///
/// # Safety
///
/// `out` must be null or writable.
unsafe fn put<T>(out: *mut T, value: T) -> Res<()> {
    // SAFETY: the caller guarantees `out` is null or writable.
    match unsafe { out.as_mut() } {
        Some(out) => {
            *out = value;
            Ok(())
        }
        None => Err(Fail::err("a null output pointer")),
    }
}

/// Lends `value` through `out`.
///
/// # Safety
///
/// `out` must be null or a writable `GnoblinValue`.
unsafe fn answer(out: *mut GnoblinValue, value: Val) -> Res<()> {
    // SAFETY: the caller guarantees `out` is null or writable.
    match unsafe { out.as_mut() } {
        Some(out) => {
            lend(value, out);
            Ok(())
        }
        None => Err(Fail::err("a null output pointer")),
    }
}

/// The properties of a `GnoblinInit` array, read.
///
/// # Safety
///
/// `inits` must be null with `n` zero, or point at `n` readable inits.
unsafe fn read_inits(inits: *const GnoblinInit, n: usize) -> Res<Vec<(MemberId, Val)>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    if inits.is_null() {
        return Err(Fail::err("a null init array with a nonzero count"));
    }
    // SAFETY: the caller guarantees `n` readable inits.
    let slice = unsafe { std::slice::from_raw_parts(inits, n) };
    slice
        .iter()
        // SAFETY: each init's value is one the caller built for this call.
        .map(|init| unsafe { from_raw(&init.value, true) }.map(|value| (init.member, value)))
        .collect()
}

/// A struct the caller built, read by the `size` it says it has; the fields past it are zero.
///
/// # Safety
///
/// `ptr` must be null or point at a `T` whose first field is its `u32` size, readable for that many bytes, and every field of `T`
/// must be valid when all-zero.
unsafe fn read_ext<T: Copy>(ptr: *const T, what: &str) -> Res<T> {
    if ptr.is_null() {
        return Err(Fail::err(format!("{what}: a null pointer")));
    }
    // SAFETY: the first field of every extensible struct is its `u32` size, which the caller wrote.
    let caller = unsafe { ptr.cast::<u32>().read_unaligned() } as usize;
    let size = caller.min(size_of::<T>());
    if size < size_of::<u32>() {
        return Err(Fail::new(
            abi::E_ABI,
            format!("{what}: the caller's struct is too small to carry its size"),
        ));
    }
    let mut value = core::mem::MaybeUninit::<T>::zeroed();
    // SAFETY: `size` bytes are readable at `ptr` and fit in `value`; zeroed `T` is valid by the caller's guarantee.
    unsafe {
        core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), value.as_mut_ptr().cast::<u8>(), size);
        Ok(value.assume_init())
    }
}

// ---- the functions ------------------------------------------------------------------------------------------------------

unsafe extern "C" fn class_id<P: Provider>(_ctx: GnoblinCtx, name: GnoblinStr, out: *mut GnoblinClassId) -> i32 {
    run::<P>("class_id", |lib| {
        // SAFETY: the table's contract passes `name` as readable text and `out` as writable.
        unsafe {
            let id = lib.class_id(text_of(name)?)?;
            put(out, id)
        }
    })
}

unsafe extern "C" fn member_id<P: Provider>(
    _ctx: GnoblinCtx,
    class: GnoblinClassId,
    kind: u32,
    name: GnoblinStr,
    out: *mut GnoblinMemberId,
) -> i32 {
    run::<P>("member_id", |lib| {
        // SAFETY: as for `class_id`.
        unsafe {
            let id = lib.member_id(class, kind, text_of(name)?)?;
            put(out, id)
        }
    })
}

fn class_entry(lib: &Library, id: u32, class: &'static ClassDef) -> GnoblinCatalogEntry {
    GnoblinCatalogEntry {
        size: size_of::<GnoblinCatalogEntry>() as u32,
        entry: abi::CATALOG_CLASS,
        class: id,
        member: 0,
        kind: class.kind,
        flags: class.flags,
        base: lib.catalog.class_id(class.base).unwrap_or(0),
        value_kind: 0,
        value_class: 0,
        arg_count: 0,
        arg_kinds: core::ptr::null(),
        arg_names: GnoblinStr::EMPTY,
        name: GnoblinStr::borrowed(class.name),
    }
}

fn member_entry(lib: &Library, id: u32, member_id: u32, member: &'static MemberDef) -> GnoblinCatalogEntry {
    let (value_kind, value_class) = match member.kind {
        abi::MEMBER_PROPERTY | abi::MEMBER_METHOD => {
            (member.ty.kind, lib.catalog.class_id(member.ty.class).unwrap_or(0))
        }
        _ => (0, 0),
    };
    GnoblinCatalogEntry {
        size: size_of::<GnoblinCatalogEntry>() as u32,
        entry: abi::CATALOG_MEMBER,
        class: id,
        member: member_id,
        kind: member.kind,
        flags: member.flags,
        base: 0,
        value_kind,
        value_class,
        arg_count: member.args.len() as u32,
        arg_kinds: member.arg_kinds.as_ptr(),
        arg_names: GnoblinStr::borrowed(member.arg_names),
        name: GnoblinStr::borrowed(member.name),
    }
}

unsafe extern "C" fn catalog_each<P: Provider>(_ctx: GnoblinCtx, cb: GnoblinCatalogFn, user: *mut c_void) -> i32 {
    let mut stopped = abi::E_OK;
    let code = run::<P>("catalog_each", |lib| {
        'walk: for (index, class) in lib.catalog.classes.iter().enumerate() {
            let id = index as u32 + 1;
            let entry = class_entry(lib, id, class);
            // SAFETY: `cb` and `user` are the caller's; `entry` is valid for the call.
            stopped = unsafe { cb(user, &entry) };
            if stopped != abi::E_OK {
                break 'walk;
            }
            for (position, member) in class.members.iter().enumerate() {
                let entry = member_entry(lib, id, position as u32 + 1, member);
                // SAFETY: as above.
                stopped = unsafe { cb(user, &entry) };
                if stopped != abi::E_OK {
                    break 'walk;
                }
            }
        }
        Ok(())
    });
    if code == abi::E_OK { stopped } else { code }
}

unsafe extern "C" fn create<P: Provider>(
    ctx: GnoblinCtx,
    class: GnoblinClassId,
    inits: *const GnoblinInit,
    n: usize,
    out: *mut GnoblinObj,
) -> i32 {
    run::<P>("create", |lib| {
        // SAFETY: the table's contract passes `inits` as `n` readable inits and `out` as writable.
        unsafe {
            let inits = read_inits(inits, n)?;
            let obj = lib.create(ctx, class, inits)?;
            if let Err(fail) = put(out, obj) {
                let _ = lib.destroy(ctx, obj);
                return Err(fail);
            }
            Ok(())
        }
    })
}

unsafe extern "C" fn destroy<P: Provider>(ctx: GnoblinCtx, obj: GnoblinObj) -> i32 {
    run::<P>("destroy", |lib| lib.destroy(ctx, obj))
}

unsafe extern "C" fn release<P: Provider>(ctx: GnoblinCtx, obj: GnoblinObj) -> i32 {
    run::<P>("release", |lib| lib.release(ctx, obj))
}

unsafe extern "C" fn retain<P: Provider>(ctx: GnoblinCtx, obj: GnoblinObj) -> i32 {
    run::<P>("retain", |lib| lib.retain(ctx, obj))
}

unsafe extern "C" fn get<P: Provider>(
    ctx: GnoblinCtx,
    obj: GnoblinObj,
    member: GnoblinMemberId,
    out: *mut GnoblinValue,
) -> i32 {
    run::<P>("get", |lib| {
        let value = lib.get(ctx, obj, member)?;
        // SAFETY: the table's contract passes `out` as a writable value.
        unsafe { answer(out, value) }
    })
}

unsafe extern "C" fn set<P: Provider>(
    ctx: GnoblinCtx,
    obj: GnoblinObj,
    member: GnoblinMemberId,
    value: *const GnoblinValue,
) -> i32 {
    run::<P>("set", |lib| {
        // SAFETY: the table's contract passes `value` as a readable value.
        let value = unsafe { value.as_ref() }.ok_or_else(|| Fail::err("a null value"))?;
        // SAFETY: the value is one the caller built for this call.
        let value = unsafe { from_raw(value, true) }?;
        lib.set(ctx, obj, member, value)
    })
}

unsafe extern "C" fn update<P: Provider>(ctx: GnoblinCtx, obj: GnoblinObj, inits: *const GnoblinInit, n: usize) -> i32 {
    run::<P>("update", |lib| {
        // SAFETY: the table's contract passes `inits` as `n` readable inits.
        let inits = unsafe { read_inits(inits, n) }?;
        lib.update(ctx, obj, inits)
    })
}

unsafe extern "C" fn set_timed<P: Provider>(
    ctx: GnoblinCtx,
    obj: GnoblinObj,
    member: GnoblinMemberId,
    value: *const GnoblinValue,
    timing: *const GnoblinTiming,
) -> i32 {
    run::<P>("set_timed", |lib| {
        // SAFETY: the table's contract passes `timing` as a readable struct and `value` as a readable value.
        let (timing, value) = unsafe {
            let timing = read_ext(timing, "set_timed: the timing")?;
            let samples = if timing.samples.is_null() || timing.n_samples == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(timing.samples, timing.n_samples).to_vec()
            };
            let value = value.as_ref().ok_or_else(|| Fail::err("a null value"))?;
            (
                Timing {
                    seconds: timing.seconds,
                    delay: timing.delay,
                    tween: timing.tween,
                    samples,
                },
                from_raw(value, true)?,
            )
        };
        lib.set_timed(ctx, obj, member, value, &timing)
    })
}

unsafe extern "C" fn call<P: Provider>(
    ctx: GnoblinCtx,
    obj: GnoblinObj,
    member: GnoblinMemberId,
    args: *const GnoblinValue,
    n: usize,
    out: *mut GnoblinValue,
) -> i32 {
    run::<P>("call", |lib| {
        // SAFETY: the table's contract passes `args` as `n` readable values and `out` as writable.
        unsafe {
            let args = read_args(args, n, true)?;
            let value = lib.call(ctx, obj, member, args)?;
            answer(out, value)
        }
    })
}

unsafe extern "C" fn call_static<P: Provider>(
    ctx: GnoblinCtx,
    class: GnoblinClassId,
    member: GnoblinMemberId,
    args: *const GnoblinValue,
    n: usize,
    out: *mut GnoblinValue,
) -> i32 {
    run::<P>("call_static", |lib| {
        // SAFETY: as for `call`.
        unsafe {
            let args = read_args(args, n, true)?;
            let value = lib.call_static(ctx, class, member, args)?;
            answer(out, value)
        }
    })
}

unsafe extern "C" fn listen<P: Provider>(
    ctx: GnoblinCtx,
    obj: GnoblinObj,
    member: GnoblinMemberId,
    cb: GnoblinInvokeFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    run_reg::<P>("listen", |lib| lib.listen(ctx, obj, member, cb, user, drop_user))
}

unsafe extern "C" fn unlisten<P: Provider>(ctx: GnoblinCtx, reg: GnoblinReg) -> i32 {
    run::<P>("unlisten", |lib| lib.unlisten(ctx, reg))
}

unsafe extern "C" fn implement<P: Provider>(
    ctx: GnoblinCtx,
    class: GnoblinClassId,
    imp: *const GnoblinImpl,
    out: *mut GnoblinObj,
) -> i32 {
    run::<P>("implement", |lib| {
        // SAFETY: the table's contract passes `imp` as a readable struct whose mask is `n_words` readable words.
        unsafe {
            let imp = read_ext(imp, "implement")?;
            let Some(invoke) = imp.invoke else {
                return Err(Fail::type_error("implement: the shell gave no function to call"));
            };
            let words = if imp.implemented.is_null() || imp.n_words == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(imp.implemented, imp.n_words).to_vec()
            };
            let obj = lib.implement(ctx, class, words, invoke, imp.user, imp.drop_user)?;
            if let Err(fail) = put(out, obj) {
                // The object took `user`: a failed answer cannot give it back, so the object ends and frees it.
                let _ = lib.destroy(ctx, obj);
                return Err(fail);
            }
            Ok(())
        }
    })
}

unsafe extern "C" fn class_of<P: Provider>(ctx: GnoblinCtx, obj: GnoblinObj, out: *mut GnoblinClassId) -> i32 {
    run::<P>("class_of", |lib| {
        let class = lib.class_of(ctx, obj)?;
        // SAFETY: the table's contract passes `out` as writable.
        unsafe { put(out, class) }
    })
}

unsafe extern "C" fn track<P: Provider>(_ctx: GnoblinCtx, cb: Option<abi::GnoblinTrackFn>, user: *mut c_void) -> i32 {
    run::<P>("track", |lib| {
        let user = user as usize;
        let tracker = cb.map(|cb| -> crate::server::Tracker {
            Box::new(move |obj, class, member| {
                // SAFETY: `cb` and `user` are the tracker's, which its owner keeps alive until it clears the tracker.
                unsafe { cb(user as *mut c_void, obj, class, member) }
            })
        });
        lib.set_tracker(tracker);
        Ok(())
    })
}
