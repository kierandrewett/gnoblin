//! The function table of the std library: `GnoblinStdApi`, behind `gnoblin_std_get_api`.
//!
//! Every function runs inside [`call`], which contains a panic (the table is `extern "C"`, which a panic must never leave), turns a
//! failure into its code and records its text for `gnoblin_std_last_error`. A function takes the core's generation context first and
//! finds the generation's state in `state.rs`; one that changes anything posts a turn (`turn.rs`) and returns without running a
//! shell callback.
//!
//! A struct a caller passes in starts with `size: u32`: the library reads that many bytes and takes the rest as zero, so a shell
//! built against an earlier layout keeps working. A shell's `user` pointer is adopted only when the call succeeds; every check that
//! can fail is made before the library takes a layout's table or a listener.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinDropFn, GnoblinNode, GnoblinStr};
use gnoblin_std_abi::{
    ABI_MAJOR, ABI_MINOR, GnoblinStdApi, GnoblinStdCollection, GnoblinStdEventFn, GnoblinStdLayoutSpec,
    GnoblinStdLayoutVtable, GnoblinStdMembers, GnoblinStdOutputPolicy, GnoblinStdQueryTerm, GnoblinStdReg,
    GnoblinStdShow, GnoblinStdStats, GnoblinStdWorkspace, GnoblinStdWorkspaceInfo, GnoblinStdWorkspacePolicy,
    GnoblinStdWorkspaceSpec,
};

use crate::collection::{Source, Want};
use crate::error::{Fail, Res, record, set_last};
use crate::events;
use crate::floating::{DEFAULT_CASCADE_STEP, PlaceMode};
use crate::layout::{Foreign, LayoutObj};
use crate::state::{self, RegData, State, Std};
use crate::turn::schedule;
use crate::workspace::check_gap;

// ---- the shape of a call ---------------------------------------------------------------------------------------------------

/// Runs one table function: contains a panic, records a failure, returns the code.
pub(crate) fn call(name: &'static str, body: impl FnOnce() -> Res<i32>) -> i32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(code)) => code,
        Ok(Err(fail)) => record(&fail),
        Err(_) => {
            set_last(format!("E_PANIC: a panic was caught in the std function `{name}`"));
            abi::E_PANIC
        }
    }
}

/// [`call`] for a function that returns a registration: zero is the failure.
pub(crate) fn call_reg(name: &'static str, body: impl FnOnce() -> Res<GnoblinStdReg>) -> GnoblinStdReg {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(reg)) => reg,
        Ok(Err(fail)) => {
            record(&fail);
            0
        }
        Err(_) => {
            set_last(format!("E_PANIC: a panic was caught in the std function `{name}`"));
            0
        }
    }
}

/// The generation's state, or why there is none.
pub(crate) fn std_of(ctx: GnoblinCtx) -> Res<Rc<Std>> {
    state::lookup(ctx)
}

/// Runs `f` over the generation's state, then posts a turn: whatever it changed is laid out and announced with the next dispatch.
pub(crate) fn mutate<R>(ctx: GnoblinCtx, f: impl FnOnce(&mut State) -> Res<R>) -> Res<R> {
    let std = std_of(ctx)?;
    let result = f(&mut std.st.borrow_mut());
    schedule(&std);
    result
}

/// Runs `f` over the generation's state without changing it.
pub(crate) fn read<R>(ctx: GnoblinCtx, f: impl FnOnce(&State) -> Res<R>) -> Res<R> {
    let std = std_of(ctx)?;
    f(&std.st.borrow())
}

/// The generation's state for a function that needs no desktop: the model is attached when the generation has none, and the library does
/// not become the desktop (animation, styles and the scene work beside a shell that keeps its own workspaces).
pub(crate) fn quiet_of(ctx: GnoblinCtx) -> Res<Rc<Std>> {
    state::lookup_quiet(ctx)
}

/// [`mutate`] for a function that needs no desktop.
pub(crate) fn mutate_quiet<R>(ctx: GnoblinCtx, f: impl FnOnce(&mut State) -> Res<R>) -> Res<R> {
    let std = quiet_of(ctx)?;
    let result = f(&mut std.st.borrow_mut());
    schedule(&std);
    result
}

/// [`read`] for a function that needs no desktop.
pub(crate) fn read_quiet<R>(ctx: GnoblinCtx, f: impl FnOnce(&State) -> Res<R>) -> Res<R> {
    let std = quiet_of(ctx)?;
    f(&std.st.borrow())
}

// ---- reading what the caller passed ----------------------------------------------------------------------------------------

/// `len` bytes at `ptr` as text.
///
/// # Safety
///
/// `ptr` must be null (with `len` zero) or point at `len` readable bytes that stay valid for the returned borrow.
pub(crate) unsafe fn utf8<'a>(ptr: *const u8, len: usize) -> Res<&'a str> {
    if ptr.is_null() || len == 0 {
        return Ok("");
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes).map_err(|_| Fail::type_error("text must be UTF-8"))
}

/// A `GnoblinStr` as owned text.
///
/// # Safety
///
/// As for [`utf8`].
pub(crate) unsafe fn owned(text: &GnoblinStr) -> Res<String> {
    // SAFETY: forwarded from the caller.
    unsafe { utf8(text.ptr, text.len) }.map(str::to_owned)
}

/// Reads a struct the caller built: the `size` it says it has, the rest zero.
///
/// # Safety
///
/// `ptr` must be null or point at a struct of type `T` whose first field is the `u32` size, readable for that many bytes, and every
/// field of `T` must be valid when all-zero (integers, floats, raw pointers and `Option` of a function pointer).
pub(crate) unsafe fn read_ext<T: Copy>(ptr: *const T, what: &str) -> Res<T> {
    if ptr.is_null() {
        return Err(Fail::err(format!("{what}: a null pointer")));
    }
    // SAFETY: the first field of every extensible struct is its `u32` size, which the caller wrote.
    let size = unsafe { ptr.cast::<u32>().read_unaligned() } as usize;
    if size < size_of::<u32>() {
        return Err(Fail::new(abi::E_ABI, format!("{what}: a malformed size")));
    }
    let known = size_of::<T>();
    let mut zeroed = core::mem::MaybeUninit::<T>::zeroed();
    // SAFETY: `size.min(known)` bytes of the caller's struct are readable by its own `size`, and `zeroed` holds `known` bytes; the
    // caller guarantees all-zero is valid for the rest of the fields.
    unsafe {
        core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), zeroed.as_mut_ptr().cast::<u8>(), size.min(known));
        Ok(zeroed.assume_init())
    }
}

/// Writes `value` into the struct a caller will read, as far as the `size` it set.
///
/// # Safety
///
/// `out` must be null or point at a writable struct of type `T` whose first field is the `u32` size the caller set.
pub(crate) unsafe fn fill_out<T: Copy>(out: *mut T, value: &T, what: &str) -> Res<()> {
    if out.is_null() {
        return Err(Fail::err(format!("{what}: a null output pointer")));
    }
    // SAFETY: the first field is the caller's `u32` size.
    let size = unsafe { out.cast::<u32>().read_unaligned() };
    let count = (size as usize).min(size_of::<T>());
    // SAFETY: `count` bytes of the caller's struct are writable by its own `size`; `value` is a live local of `size_of::<T>()` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(std::ptr::from_ref(value).cast::<u8>(), out.cast::<u8>(), count);
        // The caller's size stays what it was.
        out.cast::<u32>().write_unaligned(size);
    }
    Ok(())
}

/// Writes one value through an output pointer.
///
/// # Safety
///
/// `out` must be null or point at a writable `T`.
pub(crate) unsafe fn put<T>(out: *mut T, value: T) -> Res<()> {
    // SAFETY: the caller guarantees `out` is null or writable.
    match unsafe { out.as_mut() } {
        Some(out) => {
            *out = value;
            Ok(())
        }
        None => Err(Fail::err("a null output pointer")),
    }
}

thread_local! {
    /// The members `collection_members` lent, valid until the next call on this thread.
    static LENT_MEMBERS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// The id and the name `workspace_info` lent.
    static LENT_TEXT: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
}

pub(crate) fn lent(text: &str) -> GnoblinStr {
    GnoblinStr {
        ptr: if text.is_empty() {
            core::ptr::null()
        } else {
            text.as_ptr()
        },
        len: text.len(),
    }
}

// ---- layouts ---------------------------------------------------------------------------------------------------------------

/// What a layout spec asks for, checked, before anything is adopted.
enum Layout {
    Floating {
        mode: PlaceMode,
        step: f64,
    },
    Foreign {
        vtable: GnoblinStdLayoutVtable,
        user: *mut c_void,
        drop_user: Option<GnoblinDropFn>,
    },
}

/// Checks a layout spec and reads what it names. Nothing is adopted: `user` is the caller's until [`Layout::build`].
///
/// # Safety
///
/// `spec` must be null or point at a `GnoblinStdLayoutSpec` and, for a foreign layout, its `vtable` must point at a
/// `GnoblinStdLayoutVtable`.
unsafe fn layout_spec(spec: *const GnoblinStdLayoutSpec) -> Res<Layout> {
    // SAFETY: forwarded from the caller; every field of the spec is an integer, a float, a pointer or an `Option` of a function.
    let spec = unsafe { read_ext(spec, "layout spec") }?;
    match spec.kind {
        gnoblin_std_abi::STD_LAYOUT_FLOATING => {
            let mode = PlaceMode::from_abi(spec.placement)
                .ok_or_else(|| Fail::type_error("the floating layout's placement must be smart, center or cascade"))?;
            let step = if spec.cascade_step == 0.0 {
                DEFAULT_CASCADE_STEP
            } else {
                spec.cascade_step
            };
            if !step.is_finite() || step < 1.0 {
                return Err(Fail::type_error(
                    "FloatingLayout cascadeStep must be a finite number of at least 1",
                ));
            }
            Ok(Layout::Floating { mode, step })
        }
        gnoblin_std_abi::STD_LAYOUT_FOREIGN => {
            if spec.vtable.is_null() {
                return Err(Fail::type_error("a foreign layout needs a vtable"));
            }
            // SAFETY: the vtable's `compute` is a function pointer that must be non-null, so the table is read only when it holds
            // every field up to the optional dividers (the layout's first minor); a shorter tail is read as absent functions, and a
            // longer one, from a later minor, as far as this library knows it.
            let size = unsafe { spec.vtable.cast::<u32>().read_unaligned() } as usize;
            if size < core::mem::offset_of!(GnoblinStdLayoutVtable, find_split) {
                return Err(Fail::new(
                    abi::E_ABI,
                    "the layout's vtable is shorter than this library's: it was built for an earlier layout",
                ));
            }
            // SAFETY: the table is readable for `size` bytes, which covers the fields that are not optional; the fields past `size`
            // are read as zero, which is `None` for each.
            let vtable = unsafe { read_ext(spec.vtable, "layout vtable") }?;
            Ok(Layout::Foreign {
                vtable,
                user: spec.user,
                drop_user: spec.drop_user,
            })
        }
        other => Err(Fail::type_error(format!("unknown layout kind {other}"))),
    }
}

impl Layout {
    /// Makes the layout. A foreign layout owns `user` from here on.
    fn build(self) -> Rc<LayoutObj> {
        match self {
            Layout::Floating { mode, step } => LayoutObj::floating(mode, step),
            Layout::Foreign {
                vtable,
                user,
                drop_user,
            } => LayoutObj::foreign(Foreign::new(vtable, user, drop_user)),
        }
    }
}

// ---- the functions -----------------------------------------------------------------------------------------------------------

unsafe extern "C" fn open(ctx: GnoblinCtx) -> i32 {
    call("open", || events::open(ctx).map(|()| abi::E_OK))
}

unsafe extern "C" fn stats(ctx: GnoblinCtx, out: *mut GnoblinStdStats) -> i32 {
    call("stats", || {
        let value = read(ctx, |st| {
            Ok(GnoblinStdStats {
                size: size_of::<GnoblinStdStats>() as u32,
                reserved: 0,
                workspaces: st.ws.len() as u64,
                clients: st.clients.len() as u64,
                collections: st.colls.len() as u64,
                registrations: st.regs.len() as u64,
                layout_passes: st.counters.layout_passes,
                computes: st.counters.computes,
            })
        })?;
        // SAFETY: the caller guarantees `out` is a writable `GnoblinStdStats` whose first field is its size.
        unsafe { fill_out(out, &value, "stats") }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn remove(reg: GnoblinStdReg) -> i32 {
    call("remove", || {
        let std = state::owner_of(reg)
            .ok_or_else(|| Fail::stale(format!("registration {reg} was removed or never existed")))?;
        if std.router.borrow().has(reg) {
            // A binding: its drag in flight is cancelled and its `drop_user` runs now.
            crate::bindings::remove(&std, reg);
            std.drain();
            return Ok(abi::E_OK);
        }
        let data = {
            let mut st = std.st.borrow_mut();
            let data = st.regs.remove(&reg);
            if let Some(data) = &data {
                st.untrack_if_idle(data.coll);
            }
            data
        };
        // The listener's `drop_user` runs now, outside the borrow, or when the delivery that holds the last other reference ends.
        drop(data);
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn collection_root(ctx: GnoblinCtx, kind: u32, out: *mut GnoblinStdCollection) -> i32 {
    call("collection_root", || {
        if !(gnoblin_std_abi::STD_COLLECTION_CLIENTS..=gnoblin_std_abi::STD_COLLECTION_WORKSPACES).contains(&kind) {
            return Err(Fail::type_error(format!("unknown collection kind {kind}")));
        }
        let root = read(ctx, |st| Ok(st.roots[(kind - 1) as usize]))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, root) }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn collection_where(
    ctx: GnoblinCtx,
    parent: GnoblinStdCollection,
    terms: *const GnoblinStdQueryTerm,
    n_terms: usize,
    out: *mut GnoblinStdCollection,
) -> i32 {
    call("collection_where", || {
        let mut parsed: Vec<(String, Want)> = Vec::with_capacity(n_terms);
        for at in 0..n_terms {
            // SAFETY: the caller guarantees `terms` points at `n_terms` terms.
            let term = unsafe { read_ext(terms.add(at), "query term") }?;
            // SAFETY: the term's strings are the caller's, valid for the call.
            let key = unsafe { owned(&term.key) }?;
            let want = match term.kind {
                gnoblin_std_abi::STD_TERM_TEXT => Want::Text(unsafe { owned(&term.text) }?),
                gnoblin_std_abi::STD_TERM_NUMBER => Want::Number(term.number),
                gnoblin_std_abi::STD_TERM_BOOL => Want::Bool(term.flag != 0),
                other => {
                    return Err(Fail::type_error(format!(
                        "where: `{key}` has an unknown value kind {other}"
                    )));
                }
            };
            parsed.push((key, want));
        }
        let view = mutate(ctx, |st| st.make_view(parent, parsed))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, view) }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn collection_kind(ctx: GnoblinCtx, collection: GnoblinStdCollection, out: *mut u32) -> i32 {
    call("collection_kind", || {
        let kind = read(ctx, |st| {
            st.colls
                .get(&collection)
                .map(|coll| coll.kind)
                .ok_or_else(|| Fail::stale(format!("collection {collection} does not exist")))
        })?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, kind) }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn collection_len(ctx: GnoblinCtx, collection: GnoblinStdCollection, out: *mut usize) -> i32 {
    call("collection_len", || {
        let len = read(ctx, |st| {
            if st.colls.contains_key(&collection) {
                Ok(st.items(collection).len())
            } else {
                Err(Fail::stale(format!("collection {collection} does not exist")))
            }
        })?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, len) }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn collection_members(
    ctx: GnoblinCtx,
    collection: GnoblinStdCollection,
    out: *mut GnoblinStdMembers,
) -> i32 {
    call("collection_members", || {
        let items = read(ctx, |st| {
            if st.colls.contains_key(&collection) {
                Ok(st.items(collection))
            } else {
                Err(Fail::stale(format!("collection {collection} does not exist")))
            }
        })?;
        let (ptr, len) = LENT_MEMBERS.with(|lent| {
            *lent.borrow_mut() = items;
            let lent = lent.borrow();
            (lent.as_ptr(), lent.len())
        });
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe {
            put(
                out,
                GnoblinStdMembers {
                    ptr: if len == 0 { core::ptr::null() } else { ptr },
                    len,
                },
            )
        }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn collection_contains(ctx: GnoblinCtx, collection: GnoblinStdCollection, member: u64) -> i32 {
    call("collection_contains", || {
        read(ctx, |st| {
            if st.colls.contains_key(&collection) {
                Ok(i32::from(st.contains(collection, member)))
            } else {
                Err(Fail::stale(format!("collection {collection} does not exist")))
            }
        })
    })
}

/// The events a collection of `kind` can raise.
fn event_fits(kind: u32, event: u32) -> bool {
    use gnoblin_std_abi as a;
    match kind {
        a::STD_COLLECTION_CLIENTS => {
            (a::STD_EVENT_ADDED..=a::STD_EVENT_READY).contains(&event) || event == a::STD_EVENT_DECORATION_CHANGED
        }
        a::STD_COLLECTION_OUTPUTS => matches!(
            event,
            a::STD_EVENT_ADDED
                | a::STD_EVENT_REMOVED
                | a::STD_EVENT_GEOMETRY_CHANGED
                | a::STD_EVENT_MODE_CHANGED
                | a::STD_EVENT_DISPLAYS
                | a::STD_EVENT_APPLIED
                | a::STD_EVENT_NOTE
        ),
        a::STD_COLLECTION_WORKSPACES => matches!(event, a::STD_EVENT_ADDED | a::STD_EVENT_REMOVED),
        _ => false,
    }
}

unsafe extern "C" fn collection_listen(
    ctx: GnoblinCtx,
    collection: GnoblinStdCollection,
    event: u32,
    flags: u32,
    cb: GnoblinStdEventFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinStdReg {
    call_reg("collection_listen", || {
        let id = state::next_id();
        mutate(ctx, |st| {
            let kind = st
                .colls
                .get(&collection)
                .map(|coll| coll.kind)
                .ok_or_else(|| Fail::stale(format!("collection {collection} does not exist")))?;
            if !event_fits(kind, event) {
                return Err(Fail::type_error(format!(
                    "event {event} is not one a collection of kind {kind} raises"
                )));
            }
            // Every check has passed: from here the library owns `user`.
            st.regs
                .insert(id, Rc::new(RegData::new(collection, event, cb, user, drop_user)));
            st.track(collection);
            if flags & gnoblin_std_abi::STD_LISTEN_REPLAY != 0 && event == gnoblin_std_abi::STD_EVENT_ADDED {
                for member in st.items(collection) {
                    let mut notice = state::Notice::new(event, member, vec![collection]);
                    notice.only = Some(id);
                    st.notices.push(notice);
                }
            }
            Ok(())
        })?;
        Ok(id)
    })
}

unsafe extern "C" fn collection_add(ctx: GnoblinCtx, collection: GnoblinStdCollection, member: u64) -> i32 {
    call("collection_add", || {
        mutate(ctx, |st| match source_of(st, collection)? {
            Source::WorkspaceClients(workspace) => st.transfer_client(workspace, member).map(|()| abi::E_OK),
            Source::Workspaces => st.add_workspace(member).map(|()| abi::E_OK),
            _ => Err(Fail::refused("this collection's membership cannot be changed")),
        })
    })
}

unsafe extern "C" fn collection_remove(ctx: GnoblinCtx, collection: GnoblinStdCollection, member: u64) -> i32 {
    call("collection_remove", || {
        mutate(ctx, |st| match source_of(st, collection)? {
            Source::WorkspaceClients(workspace) => Ok(i32::from(st.detach_client(workspace, member))),
            Source::Workspaces => {
                // The last workspace stays: a shell with nowhere to put a window is unusable.
                if st.workspaces.len() < 2 || !st.workspaces.contains(&member) {
                    return Ok(0);
                }
                st.remove_workspace(member);
                Ok(1)
            }
            _ => Err(Fail::refused("this collection's membership cannot be changed")),
        })
    })
}

unsafe extern "C" fn collection_clear(ctx: GnoblinCtx, collection: GnoblinStdCollection) -> i32 {
    call("collection_clear", || {
        mutate(ctx, |st| match source_of(st, collection)? {
            Source::WorkspaceClients(workspace) => {
                st.clear_clients(workspace);
                Ok(abi::E_OK)
            }
            Source::Workspaces => {
                for workspace in st.workspaces.clone() {
                    st.remove_workspace(workspace);
                }
                Ok(abi::E_OK)
            }
            _ => Err(Fail::refused("this collection's membership cannot be changed")),
        })
    })
}

unsafe extern "C" fn collection_swap(ctx: GnoblinCtx, collection: GnoblinStdCollection, a: u64, b: u64) -> i32 {
    call("collection_swap", || {
        mutate(ctx, |st| match source_of(st, collection)? {
            Source::WorkspaceClients(workspace) => st.swap_clients(workspace, a, b).map(|()| abi::E_OK),
            _ => Err(Fail::refused("this collection's order cannot be changed")),
        })
    })
}

/// Where a mutable root collection's members come from.
fn source_of(st: &State, collection: GnoblinStdCollection) -> Res<Source> {
    let coll = st
        .colls
        .get(&collection)
        .ok_or_else(|| Fail::stale(format!("collection {collection} does not exist")))?;
    coll.source
        .ok_or_else(|| Fail::refused("this collection is a selection: its membership follows its parent's"))
}

unsafe extern "C" fn client_workspace(ctx: GnoblinCtx, client: GnoblinNode, out: *mut GnoblinStdWorkspace) -> i32 {
    call("client_workspace", || {
        let workspace = read(ctx, |st| {
            st.rec
                .get(&client)
                .map(|rec| rec.workspace.unwrap_or(0))
                .ok_or_else(|| Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")))
        })?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, workspace) }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn client_move_to_workspace(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    workspace: GnoblinStdWorkspace,
) -> i32 {
    call("client_move_to_workspace", || {
        mutate(ctx, |st| st.move_client_to(client, workspace).map(i32::from))
    })
}

unsafe extern "C" fn client_set_floating(ctx: GnoblinCtx, client: GnoblinNode, floating: u32) -> i32 {
    call("client_set_floating", || {
        mutate(ctx, |st| {
            let rec = st
                .rec
                .get_mut(&client)
                .ok_or_else(|| Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")))?;
            rec.floating = floating != 0;
            if let Some(workspace) = rec.workspace.and_then(|id| st.ws.get_mut(&id)) {
                workspace.dirty = true;
            }
            Ok(abi::E_OK)
        })
    })
}

unsafe extern "C" fn client_is_floating(ctx: GnoblinCtx, client: GnoblinNode) -> i32 {
    call("client_is_floating", || {
        read(ctx, |st| {
            st.rec
                .get(&client)
                .map(|rec| i32::from(rec.floating))
                .ok_or_else(|| Fail::new(abi::E_NO_SUCH_NODE, format!("node {client} is not a client")))
        })
    })
}

unsafe extern "C" fn workspace_new(
    ctx: GnoblinCtx,
    spec: *const GnoblinStdWorkspaceSpec,
    out: *mut GnoblinStdWorkspace,
) -> i32 {
    call("workspace_new", || {
        // SAFETY: the caller guarantees `spec` is a `GnoblinStdWorkspaceSpec` and its strings are valid for the call.
        let spec_value = unsafe { read_ext(spec, "workspace spec") }?;
        // SAFETY: as above.
        let (id, name) = unsafe { (owned(&spec_value.id)?, owned(&spec_value.name)?) };
        if id.is_empty() {
            return Err(Fail::type_error("Workspace options.id must be a non-empty string"));
        }
        let inner = check_gap(spec_value.gap_inner, "Workspace options.gaps.inner")?;
        let outer = check_gap(spec_value.gap_outer, "Workspace options.gaps.outer")?;
        // The layout is the last thing that can fail: only a layout that is adopted is freed by the library.
        let layout = if spec_value.layout.is_null() {
            Layout::Floating {
                mode: PlaceMode::FreeSpace,
                step: DEFAULT_CASCADE_STEP,
            }
        } else {
            // SAFETY: the caller guarantees the layout spec it points at is valid, with its vtable.
            unsafe { layout_spec(spec_value.layout) }?
        };
        mutate(ctx, |st| {
            let workspace = st.new_workspace(id, name, (inner, outer), layout.build(), false);
            // SAFETY: the caller guarantees `out` is null or writable.
            unsafe { put(out, workspace) }?;
            Ok(abi::E_OK)
        })
    })
}

unsafe extern "C" fn workspace_by_id(ctx: GnoblinCtx, id: *const u8, len: usize, out: *mut GnoblinStdWorkspace) -> i32 {
    call("workspace_by_id", || {
        // SAFETY: the caller guarantees `len` readable bytes at `id`.
        let id = unsafe { utf8(id, len) }?;
        let found = read(ctx, |st| {
            st.workspaces
                .iter()
                .copied()
                .find(|workspace| st.ws.get(workspace).is_some_and(|ws| ws.id == id))
                .ok_or_else(|| Fail::no_such(format!("there is no workspace \"{id}\"")))
        })?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, found) }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn workspace_info(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    out: *mut GnoblinStdWorkspaceInfo,
) -> i32 {
    call("workspace_info", || {
        let (id, name, mut info) = read(ctx, |st| {
            let ws = st.workspace(workspace)?;
            let index = st.workspaces.iter().position(|one| *one == workspace);
            Ok((
                ws.id.clone(),
                ws.name.clone(),
                GnoblinStdWorkspaceInfo {
                    size: size_of::<GnoblinStdWorkspaceInfo>() as u32,
                    attached: u32::from(ws.attached),
                    id: GnoblinStr::EMPTY,
                    name: GnoblinStr::EMPTY,
                    index: index.map_or(-1, |index| index as i64),
                    active: u32::from(st.is_shown(workspace)),
                    reserved: 0,
                    output: st.output_showing(workspace).unwrap_or(0),
                    gap_inner: ws.gap_inner,
                    gap_outer: ws.gap_outer,
                },
            ))
        })?;
        LENT_TEXT.with(|text| {
            *text.borrow_mut() = (id, name);
            let text = text.borrow();
            info.id = lent(&text.0);
            info.name = lent(&text.1);
        });
        // SAFETY: the caller guarantees `out` is a writable `GnoblinStdWorkspaceInfo` whose first field is its size.
        unsafe { fill_out(out, &info, "workspace_info") }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn workspace_add(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> i32 {
    call("workspace_add", || {
        mutate(ctx, |st| st.add_workspace(workspace).map(|()| abi::E_OK))
    })
}

unsafe extern "C" fn workspace_remove(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> i32 {
    call("workspace_remove", || {
        mutate(ctx, |st| {
            let attached = st.workspace(workspace)?.attached;
            // The last workspace stays: a shell with nowhere to put a window is unusable.
            if !attached || st.workspaces.len() < 2 {
                return Ok(0);
            }
            st.remove_workspace(workspace);
            Ok(1)
        })
    })
}

unsafe extern "C" fn workspace_set_name(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    name: *const u8,
    len: usize,
) -> i32 {
    call("workspace_set_name", || {
        // SAFETY: the caller guarantees `len` readable bytes at `name`.
        let name = unsafe { utf8(name, len) }?.to_owned();
        mutate(ctx, |st| {
            st.workspace_mut(workspace)?.name = name;
            Ok(abi::E_OK)
        })
    })
}

unsafe extern "C" fn workspace_set_gaps(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    inner: f64,
    outer: f64,
) -> i32 {
    call("workspace_set_gaps", || {
        let inner = check_gap(inner, "gaps.inner")?;
        let outer = check_gap(outer, "gaps.outer")?;
        mutate(ctx, |st| {
            let ws = st.workspace_mut(workspace)?;
            ws.gap_inner = inner;
            ws.gap_outer = outer;
            ws.dirty = true;
            Ok(abi::E_OK)
        })
    })
}

/// Gives the workspace `layout`. The old layout is dropped at the next turn, outside every borrow: dropping a foreign one runs the
/// shell's `drop_user`.
pub(crate) fn replace_layout(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, layout: Rc<LayoutObj>) -> Res<i32> {
    mutate(ctx, |st| {
        let ws = st.workspace_mut(workspace)?;
        let old = std::mem::replace(&mut ws.layout, layout);
        ws.revision = None;
        ws.dirty = true;
        st.graveyard.push(old);
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn workspace_set_layout(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    spec: *const GnoblinStdLayoutSpec,
) -> i32 {
    call("workspace_set_layout", || {
        // The workspace has to exist before the layout is adopted: on a failure the library has not taken `user`.
        read(ctx, |st| st.workspace(workspace).map(|_| ()))?;
        // SAFETY: the caller guarantees `spec` and its vtable are valid.
        let layout = unsafe { layout_spec(spec) }?;
        replace_layout(ctx, workspace, layout.build())
    })
}

unsafe extern "C" fn workspace_invalidate(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> i32 {
    call("workspace_invalidate", || {
        mutate(ctx, |st| {
            st.workspace_mut(workspace)?.dirty = true;
            Ok(abi::E_OK)
        })
    })
}

unsafe extern "C" fn workspace_clients(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    out: *mut GnoblinStdCollection,
) -> i32 {
    call("workspace_clients", || {
        let coll = read(ctx, |st| st.workspace(workspace).map(|ws| ws.coll))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, coll) }?;
        Ok(abi::E_OK)
    })
}

unsafe extern "C" fn workspace_show(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    show: *const GnoblinStdShow,
) -> i32 {
    call("workspace_show", || {
        // SAFETY: the caller guarantees `show` is null or a `GnoblinStdShow`.
        let show = if show.is_null() {
            GnoblinStdShow {
                size: size_of::<GnoblinStdShow>() as u32,
                reserved: 0,
                duration_ms: 0.0,
                progress: 0.0,
                output: 0,
            }
        } else {
            unsafe { read_ext(show, "show options") }?
        };
        mutate(ctx, |st| {
            let output = (show.output != 0).then_some(show.output);
            st.show_workspace(workspace, output, show.duration_ms, show.progress)
                .map(|()| abi::E_OK)
        })
    })
}

unsafe extern "C" fn workspace_walk(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, step: i64) -> i32 {
    call("workspace_walk", || {
        mutate(ctx, |st| st.walk(workspace, step).map(|()| abi::E_OK))
    })
}

unsafe extern "C" fn policy_outputs(ctx: GnoblinCtx, policy: *const GnoblinStdOutputPolicy) -> i32 {
    call("policy_outputs", || {
        // SAFETY: the caller guarantees `policy` is a `GnoblinStdOutputPolicy`.
        let policy = unsafe { read_ext(policy, "output policy") }?;
        if policy.mode > gnoblin_std_abi::STD_OUTPUTS_EXTEND {
            return Err(Fail::type_error(format!("unknown output policy {}", policy.mode)));
        }
        mutate(ctx, |st| {
            st.policy.outputs = policy.mode;
            st.policy.lead = policy.lead;
            st.apply_policies();
            Ok(abi::E_OK)
        })
    })
}

unsafe extern "C" fn policy_workspaces(ctx: GnoblinCtx, policy: *const GnoblinStdWorkspacePolicy) -> i32 {
    call("policy_workspaces", || {
        // SAFETY: the caller guarantees `policy` is a `GnoblinStdWorkspacePolicy`, its prefix valid for the call.
        let policy = unsafe { read_ext(policy, "workspace policy") }?;
        if policy.mode > gnoblin_std_abi::STD_WORKSPACES_PER_OUTPUT {
            return Err(Fail::type_error(format!("unknown workspace policy {}", policy.mode)));
        }
        // SAFETY: as above.
        let prefix = unsafe { owned(&policy.name_prefix) }?;
        let inner = check_gap(policy.gap_inner, "gaps.inner")?;
        let outer = check_gap(policy.gap_outer, "gaps.outer")?;
        let (mode, step) = if policy.layout.is_null() {
            (PlaceMode::FreeSpace, DEFAULT_CASCADE_STEP)
        } else {
            // SAFETY: the caller guarantees the layout spec is valid; it is read, not adopted.
            match unsafe { layout_spec(policy.layout) }? {
                Layout::Floating { mode, step } => (mode, step),
                Layout::Foreign { .. } => {
                    return Err(Fail::refused(
                        "the per-output policy makes a workspace for each output and a foreign layout has one `user`: set the layout of each workspace instead",
                    ));
                }
            }
        };
        mutate(ctx, |st| {
            st.policy.workspaces = policy.mode;
            st.policy.prefix = if prefix.is_empty() {
                "workspace-".to_owned()
            } else {
                prefix
            };
            st.policy.gaps = (inner, outer);
            st.policy.placement = mode;
            st.policy.step = step;
            st.apply_policies();
            Ok(abi::E_OK)
        })
    })
}

unsafe extern "C" fn output_workspace(ctx: GnoblinCtx, output: GnoblinNode, out: *mut GnoblinStdWorkspace) -> i32 {
    call("output_workspace", || {
        let workspace = read(ctx, |st| {
            let shown = st
                .desk_of_output(output)
                .map(|desk| st.desk(desk).and_then(|desk| desk.spanned_by).unwrap_or(desk))
                .and_then(|desk| st.desk(desk))
                .and_then(|desk| desk.shown);
            Ok(shown.unwrap_or(0))
        })?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, workspace) }?;
        Ok(abi::E_OK)
    })
}

/// The table `gnoblin_std_get_api` returns.
pub(crate) static API: GnoblinStdApi = GnoblinStdApi {
    size: size_of::<GnoblinStdApi>() as u32,
    major: ABI_MAJOR,
    minor: ABI_MINOR,
    reserved: 0,
    open,
    stats,
    remove,
    collection_root,
    collection_where,
    collection_kind,
    collection_len,
    collection_members,
    collection_contains,
    collection_listen,
    collection_add,
    collection_remove,
    collection_clear,
    collection_swap,
    client_workspace,
    client_move_to_workspace,
    client_set_floating,
    client_is_floating,
    workspace_new,
    workspace_by_id,
    workspace_info,
    workspace_add,
    workspace_remove,
    workspace_set_name,
    workspace_set_gaps,
    workspace_set_layout,
    workspace_invalidate,
    workspace_clients,
    workspace_show,
    workspace_walk,
    policy_outputs,
    policy_workspaces,
    output_workspace,
    client_state: crate::client_state::client_state,
    client_set_state: crate::client_state::client_set_state,
    client_snap: crate::client_state::client_snap,
    client_unsnap: crate::client_state::client_unsnap,
    workspace_find_split: crate::splits::workspace_find_split,
    workspace_set_split: crate::splits::workspace_set_split,
    workspace_reset_split: crate::splits::workspace_reset_split,
    easing_sample: crate::animation::table::easing_sample,
    transition_begin: crate::animation::table::transition_begin,
    transition_set: crate::animation::table::transition_set,
    transition_get: crate::animation::table::transition_get,
    transition_commit: crate::animation::table::transition_commit,
    transition_abort: crate::animation::table::transition_abort,
    transition_animate: crate::animation::table::transition_animate,
    transition_control: crate::animation::table::transition_control,
    transition_info: crate::animation::table::transition_info,
    transition_finished: crate::animation::table::transition_finished,
    style_scope: crate::style::table::style_scope,
    style_set: crate::style::table::style_set,
    style_set_list: crate::style::table::style_set_list,
    style_unset: crate::style::table::style_unset,
    style_get: crate::style::table::style_get,
    style_list: crate::style::table::style_list,
    style_computed: crate::style::table::style_computed,
    style_computed_list: crate::style::table::style_computed_list,
    style_origin: crate::style::table::style_origin,
    style_notes: crate::style::table::style_notes,
    scene_new: crate::scene::table::scene_new,
    scene_destroy: crate::scene::table::scene_destroy,
    scene_insert: crate::scene::table::scene_insert,
    scene_root: crate::scene::table::scene_root,
    scene_set: crate::scene::table::scene_set,
    scene_get: crate::scene::table::scene_get,
    workspace_set_layout_kind: crate::builtin_api::workspace_set_layout_kind,
    workspace_layout_set: crate::builtin_api::workspace_layout_set,
    workspace_layout_get: crate::builtin_api::workspace_layout_get,
    workspace_layout_act: crate::builtin_api::workspace_layout_act,
    workspace_layout_kind: crate::builtin_api::workspace_layout_kind,
    layout_ring: crate::builtin_api::layout_ring,
    binding_drag: crate::bindings::binding_drag,
    binding_gesture: crate::bindings::binding_gesture,
    binding_builtin: crate::bindings::binding_builtin,
    builtin_new: crate::standalone::builtin_new,
    builtin_free: crate::standalone::builtin_free,
    builtin_set: crate::standalone::builtin_set,
    builtin_get: crate::standalone::builtin_get,
    builtin_compute: crate::standalone::builtin_compute,
    builtin_act: crate::standalone::builtin_act,
    builtin_client_removed: crate::standalone::builtin_client_removed,
    builtin_clients_swapped: crate::standalone::builtin_clients_swapped,
    builtin_find_split: crate::standalone::builtin_find_split,
    builtin_set_split: crate::standalone::builtin_set_split,
    builtin_reset_split: crate::standalone::builtin_reset_split,
    drag_step: crate::bindings::drag_step,
    drag_clamp: crate::bindings::drag_clamp,
    drag_resize: crate::bindings::drag_resize,
    drag_zone: crate::bindings::drag_zone,
    drag_tile: crate::bindings::drag_tile,
    gesture_way: crate::bindings::gesture_way,
    transition_release: crate::animation::table::transition_release,
    output_info: crate::objects::output_info,
    output_set_insets: crate::objects::output_set_insets,
    output_span: crate::objects::output_span,
    client_areas: crate::objects::client_areas,
    workspace_restore_client: crate::objects::workspace_restore_client,
    client_restore_snap: crate::objects::client_restore_snap,
    floating_place: crate::floating_api::floating_place,
    style_rank: crate::style::table::style_rank,
    scene_children: crate::scene::table::scene_children,
    scene_parent: crate::scene::table::scene_parent,
    profile_new: crate::display_api::profile_new,
    profile_set: crate::display_api::profile_set,
    profile_add: crate::display_api::profile_add,
    profile_remove: crate::display_api::profile_remove,
    profile_list: crate::display_api::profile_list,
    display_new: crate::display_api::display_new,
    display_set_mode: crate::display_api::display_set_mode,
    display_add: crate::display_api::display_add,
    display_remove: crate::display_api::display_remove,
    display_replace: crate::display_api::display_replace,
    display_clear: crate::display_api::display_clear,
    display_list: crate::display_api::display_list,
    display_info: crate::display_api::display_info,
    display_outputs: crate::display_api::display_outputs,
    display_at: crate::display_api::display_at,
    display_assign: crate::display_api::display_assign,
    display_snapshot: crate::display_api::display_snapshot,
    display_restore: crate::display_api::display_restore,
    display_reflow: crate::display_api::display_reflow,
    style_reset: crate::style::table::style_reset,
    object_api: crate::object::object_api,
    #[cfg(feature = "next-minor")]
    uptime_ms: api_uptime_ms,
};

/// The one function a `next-minor` library appends (ABI 1.1, test builds only): milliseconds since the table was first read.
#[cfg(feature = "next-minor")]
unsafe extern "C" fn api_uptime_ms() -> u64 {
    static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    catch_unwind(|| u64::try_from(START.elapsed().as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

#[cfg(test)]
mod split_table_tests {
    use super::*;

    #[test]
    fn the_optional_dividers_end_the_required_part_of_the_layout_vtable() {
        // A vtable built before the dividers existed ends at `revision`; this library must still take it.
        assert_eq!(
            core::mem::offset_of!(GnoblinStdLayoutVtable, find_split),
            core::mem::offset_of!(GnoblinStdLayoutVtable, revision) + size_of::<usize>()
        );
    }
}
