//! The window-manager provider across the ABI: [`export_wm`] on std's side and [`ForeignWm`] on the core's.
//!
//! The trait has references in its reads (`window(&self) -> Option<&Window>`), which no C function can return, so the core's
//! [`ForeignWm`] keeps a copy of the provider's record and refreshes it after every call that can change it. A refresh is two
//! calls into the vtable and at most [`MAX_WINDOWS`] records; the provider is called once per window event, never per frame.
//!
//! A provider that cannot be trusted any more is dropped, not retried. It panics (the call answers `E_PANIC`), answers an
//! error, answers a refusal the contract does not have, sends an effect that does not decode, or shows a record that does not
//! decode. [`ForeignWm`] then says so once through its `report`, hands what it held to a [`NullWm`]
//! (`gnoblin_wm_api::hand_over`) and answers every call from it, the one that failed included.

use core::ffi::c_void;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi::{
    self as abi, GnoblinDropFn, GnoblinWmEffect, GnoblinWmInsets, GnoblinWmOpen, GnoblinWmOutput, GnoblinWmRect,
    GnoblinWmSink, GnoblinWmStanding, GnoblinWmVtable, GnoblinWmWindow,
};
use gnoblin_types::{Insets, OutputId, Point, Rect, Size, WindowId};
use gnoblin_wm_api::{
    HeldRecord, MAX_WINDOWS, NewWindowFocus, NullWm, Open, OutputRecord, Standing, Window, WindowKind, WindowManager,
    WindowState, WmEffect, WmRefusal, hand_over,
};

use crate::{Exported, Report, guarded};

// ---- the types, both ways ----------------------------------------------------------------------------------------------

fn rect_to(rect: Rect) -> GnoblinWmRect {
    GnoblinWmRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn rect_of(rect: &GnoblinWmRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

fn insets_to(insets: Insets) -> GnoblinWmInsets {
    GnoblinWmInsets {
        top: insets.top,
        right: insets.right,
        bottom: insets.bottom,
        left: insets.left,
    }
}

fn insets_of(insets: &GnoblinWmInsets) -> Insets {
    Insets {
        top: insets.top,
        right: insets.right,
        bottom: insets.bottom,
        left: insets.left,
    }
}

fn kind_to(kind: WindowKind) -> u32 {
    match kind {
        WindowKind::Normal => abi::WM_KIND_NORMAL,
        WindowKind::Dialog => abi::WM_KIND_DIALOG,
        WindowKind::Modal => abi::WM_KIND_MODAL,
    }
}

fn kind_of(kind: u32) -> Option<WindowKind> {
    match kind {
        abi::WM_KIND_NORMAL => Some(WindowKind::Normal),
        abi::WM_KIND_DIALOG => Some(WindowKind::Dialog),
        abi::WM_KIND_MODAL => Some(WindowKind::Modal),
        _ => None,
    }
}

fn state_to(state: WindowState) -> u32 {
    let mut flags = 0;
    if state.maximised {
        flags |= abi::WM_WINDOW_MAXIMISED;
    }
    if state.fullscreen {
        flags |= abi::WM_WINDOW_FULLSCREEN;
    }
    if state.minimised {
        flags |= abi::WM_WINDOW_MINIMISED;
    }
    flags
}

fn state_of(flags: u32) -> WindowState {
    WindowState {
        maximised: flags & abi::WM_WINDOW_MAXIMISED != 0,
        fullscreen: flags & abi::WM_WINDOW_FULLSCREEN != 0,
        minimised: flags & abi::WM_WINDOW_MINIMISED != 0,
    }
}

fn window_to(window: &Window) -> GnoblinWmWindow {
    let mut flags = state_to(window.state);
    if window.output.is_some() {
        flags |= abi::WM_WINDOW_HAS_OUTPUT;
    }
    if window.transient_for.is_some() {
        flags |= abi::WM_WINDOW_HAS_PARENT;
    }
    GnoblinWmWindow {
        size: size_of::<GnoblinWmWindow>() as u32,
        kind: kind_to(window.kind),
        flags,
        reserved: 0,
        id: window.id.get(),
        transient_for: window.transient_for.map_or(0, WindowId::get),
        output: window.output.map_or(0, OutputId::get),
        geometry: rect_to(window.geometry),
        restore: rect_to(window.restore),
    }
}

fn window_of(window: &GnoblinWmWindow) -> Option<Window> {
    Some(Window {
        id: WindowId::new(window.id),
        kind: kind_of(window.kind)?,
        transient_for: (window.flags & abi::WM_WINDOW_HAS_PARENT != 0).then(|| WindowId::new(window.transient_for)),
        output: (window.flags & abi::WM_WINDOW_HAS_OUTPUT != 0).then(|| OutputId::new(window.output)),
        geometry: rect_of(&window.geometry),
        restore: rect_of(&window.restore),
        state: state_of(window.flags),
    })
}

fn output_to(output: &OutputRecord) -> GnoblinWmOutput {
    GnoblinWmOutput {
        size: size_of::<GnoblinWmOutput>() as u32,
        reserved: 0,
        id: output.id.get(),
        rect: rect_to(output.rect),
        reserved_insets: insets_to(output.reserved),
    }
}

fn output_of(output: &GnoblinWmOutput) -> OutputRecord {
    OutputRecord {
        id: OutputId::new(output.id),
        rect: rect_of(&output.rect),
        reserved: insets_of(&output.reserved_insets),
    }
}

fn refusal_to(refusal: WmRefusal) -> i32 {
    match refusal {
        WmRefusal::Duplicate => abi::WM_REFUSED_DUPLICATE,
        WmRefusal::TooMany => abi::WM_REFUSED_TOO_MANY,
        WmRefusal::NoOutput => abi::WM_REFUSED_NO_OUTPUT,
        WmRefusal::Gone => abi::WM_REFUSED_GONE,
        WmRefusal::NotFloating => abi::WM_REFUSED_NOT_FLOATING,
        WmRefusal::UnknownOutput => abi::WM_REFUSED_UNKNOWN_OUTPUT,
        WmRefusal::FollowsParent => abi::WM_REFUSED_FOLLOWS_PARENT,
    }
}

fn refusal_of(code: i32) -> Option<WmRefusal> {
    match code {
        abi::WM_REFUSED_DUPLICATE => Some(WmRefusal::Duplicate),
        abi::WM_REFUSED_TOO_MANY => Some(WmRefusal::TooMany),
        abi::WM_REFUSED_NO_OUTPUT => Some(WmRefusal::NoOutput),
        abi::WM_REFUSED_GONE => Some(WmRefusal::Gone),
        abi::WM_REFUSED_NOT_FLOATING => Some(WmRefusal::NotFloating),
        abi::WM_REFUSED_UNKNOWN_OUTPUT => Some(WmRefusal::UnknownOutput),
        abi::WM_REFUSED_FOLLOWS_PARENT => Some(WmRefusal::FollowsParent),
        _ => None,
    }
}

// ---- std's side: a Rust provider as a vtable ---------------------------------------------------------------------------

/// What `user` points at: the provider.
struct WmHost {
    wm: Box<dyn WindowManager>,
}

static VTABLE: GnoblinWmVtable = GnoblinWmVtable {
    size: size_of::<GnoblinWmVtable>() as u32,
    reserved: 0,
    windows: host_windows,
    outputs: host_outputs,
    set_role: host_set_role,
    may_take_focus: host_may_take_focus,
    open: host_open,
    close: host_close,
    activate: host_activate,
    raise: host_raise,
    lower: host_lower,
    set_maximised: host_set_maximised,
    set_fullscreen: host_set_fullscreen,
    set_minimised: host_set_minimised,
    move_resize: host_move_resize,
    place: host_place,
    set_rect: host_set_rect,
    adopt_size: host_adopt_size,
    move_to_output: host_move_to_output,
    output_added: host_output_added,
    output_removed: host_output_removed,
    output_moved: host_output_moved,
    reserve: host_reserve,
};

/// Turns `wm` into a provider the core can register with `provider_register(PROVIDER_WM)`.
#[must_use]
pub fn export_wm(wm: Box<dyn WindowManager>) -> Exported<GnoblinWmVtable> {
    Exported {
        vtable: &VTABLE,
        user: Box::into_raw(Box::new(WmHost { wm })).cast(),
        drop_user: host_drop,
    }
}

/// # Safety
/// `user` is the pointer [`export_wm`] made, not yet dropped, used by one thread at a time.
unsafe fn host<'a>(user: *mut c_void) -> &'a mut WmHost {
    // SAFETY: the caller guarantees `user` points at a live `WmHost`.
    unsafe { &mut *user.cast::<WmHost>() }
}

unsafe extern "C" fn host_drop(user: *mut c_void) {
    // A provider's destructor that panics must not unwind into the core.
    // SAFETY: the core calls this once, with the `user` pointer `export_wm` made.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(user.cast::<WmHost>()) })
    }));
}

/// The paint order and focus the core passed, as the trait's own type: `stack` owns the ids `Standing` borrows.
///
/// # Safety
/// `standing` is null or points at a `GnoblinWmStanding` whose `stack` is readable for `n_stack` ids.
unsafe fn stack_of(standing: *const GnoblinWmStanding) -> (Vec<WindowId>, Option<WindowId>) {
    // SAFETY: the caller guarantees the pointer is null or valid.
    let Some(standing) = (unsafe { standing.as_ref() }) else {
        return (Vec::new(), None);
    };
    let ids = if standing.stack.is_null() || standing.n_stack == 0 {
        &[][..]
    } else {
        // SAFETY: the caller guarantees `stack` is readable for `n_stack` ids.
        unsafe { std::slice::from_raw_parts(standing.stack, standing.n_stack) }
    };
    let focus = (standing.flags & abi::WM_STANDING_HAS_FOCUS != 0).then(|| WindowId::new(standing.focus));
    (ids.iter().copied().map(WindowId::new).collect(), focus)
}

/// Pushes `effects` to the core's sink.
///
/// # Safety
/// `sink` is null or points at a `GnoblinWmSink` whose `push` may be called during this call.
unsafe fn push_all(sink: *const GnoblinWmSink, effects: &[WmEffect]) -> i32 {
    // SAFETY: the caller guarantees the pointer is null or valid.
    let Some(sink) = (unsafe { sink.as_ref() }) else {
        return abi::E_ERR;
    };
    for effect in effects {
        let mut ids: Vec<u64> = Vec::new();
        let mut wire = GnoblinWmEffect {
            size: size_of::<GnoblinWmEffect>() as u32,
            kind: 0,
            flags: 0,
            reserved: 0,
            window: 0,
            geometry: rect_to(Rect::new(0, 0, 0, 0)),
            ids: core::ptr::null(),
            n_ids: 0,
        };
        match effect {
            WmEffect::Configure {
                window,
                geometry,
                state,
            } => {
                wire.kind = abi::WM_EFFECT_CONFIGURE;
                wire.window = window.get();
                wire.geometry = rect_to(*geometry);
                wire.flags = state_to(*state);
            }
            WmEffect::Visible { window, visible } => {
                wire.kind = abi::WM_EFFECT_VISIBLE;
                wire.window = window.get();
                wire.flags = u32::from(*visible);
            }
            WmEffect::Stack(order) => {
                wire.kind = abi::WM_EFFECT_STACK;
                ids = order.iter().map(|id| id.get()).collect();
                wire.ids = ids.as_ptr();
                wire.n_ids = ids.len();
            }
            WmEffect::Focus(window) => {
                wire.kind = abi::WM_EFFECT_FOCUS;
                wire.window = window.map_or(0, WindowId::get);
                wire.flags = u32::from(window.is_some());
            }
        }
        // SAFETY: `push` and `sink.sink` came from the core together; `wire` and `ids` outlive the call.
        let rc = unsafe { (sink.push)(sink.sink, &raw const wire) };
        drop(ids);
        if rc != abi::E_OK {
            return rc;
        }
    }
    abi::E_OK
}

/// The answer of a call that returns effects or a refusal.
///
/// # Safety
/// As [`push_all`].
unsafe fn reply(result: Result<Vec<WmEffect>, WmRefusal>, sink: *const GnoblinWmSink) -> i32 {
    match result {
        // SAFETY: forwarded.
        Ok(effects) => unsafe { push_all(sink, &effects) },
        Err(refusal) => refusal_to(refusal),
    }
}

unsafe extern "C" fn host_windows(user: *mut c_void, out: *mut GnoblinWmWindow, cap: usize) -> usize {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user` is ours; `out` is writable for `cap` records, as the vtable says.
        let host = unsafe { host(user) };
        let mut count = 0;
        for window in host.wm.windows() {
            if count < cap && !out.is_null() {
                // SAFETY: `count < cap`.
                unsafe { out.add(count).write(window_to(window)) };
            }
            count += 1;
        }
        count
    }))
    .unwrap_or(usize::MAX)
}

unsafe extern "C" fn host_outputs(user: *mut c_void, out: *mut GnoblinWmOutput, cap: usize) -> usize {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as `host_windows`.
        let host = unsafe { host(user) };
        let mut count = 0;
        for output in host.wm.outputs() {
            if count < cap && !out.is_null() {
                // SAFETY: `count < cap`.
                unsafe { out.add(count).write(output_to(output)) };
            }
            count += 1;
        }
        count
    }))
    .unwrap_or(usize::MAX)
}

unsafe extern "C" fn host_set_role(
    user: *mut c_void,
    id: u64,
    kind: u32,
    flags: u32,
    transient_for: u64,
    standing: *const GnoblinWmStanding,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        let Some(kind) = kind_of(kind) else {
            return abi::E_ERR;
        };
        // SAFETY: the vtable's contract: `user`, `standing` and `sink` are valid for the call.
        unsafe {
            let (stack, focus) = stack_of(standing);
            let parent = (flags & abi::WM_WINDOW_HAS_PARENT != 0).then(|| WindowId::new(transient_for));
            let effects = host(user)
                .wm
                .set_role(WindowId::new(id), kind, parent, Standing { stack: &stack, focus });
            push_all(sink, &effects)
        }
    })
}

unsafe extern "C" fn host_may_take_focus(
    user: *mut c_void,
    id: u64,
    policy: u32,
    standing: *const GnoblinWmStanding,
) -> i32 {
    guarded(|| {
        let policy = match policy {
            abi::WM_FOCUS_ALLOW => NewWindowFocus::Allow,
            abi::WM_FOCUS_PREVENT => NewWindowFocus::Prevent,
            _ => return abi::E_ERR,
        };
        // SAFETY: the vtable's contract.
        unsafe {
            let (stack, focus) = stack_of(standing);
            i32::from(
                host(user)
                    .wm
                    .may_take_focus(WindowId::new(id), policy, Standing { stack: &stack, focus }),
            )
        }
    })
}

unsafe extern "C" fn host_open(
    user: *mut c_void,
    open: *const GnoblinWmOpen,
    standing: *const GnoblinWmStanding,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let Some(open) = open.as_ref() else {
                return abi::E_ERR;
            };
            let Some(kind) = kind_of(open.kind) else {
                return abi::E_ERR;
            };
            let (stack, focus) = stack_of(standing);
            let open = Open {
                id: WindowId::new(open.id),
                kind,
                transient_for: (open.flags & abi::WM_WINDOW_HAS_PARENT != 0).then(|| WindowId::new(open.transient_for)),
                requested: (open.flags & abi::WM_OPEN_HAS_REQUESTED != 0).then(|| rect_of(&open.requested)),
            };
            reply(host(user).wm.open(open, Standing { stack: &stack, focus }), sink)
        }
    })
}

/// Defines a host function that takes a window and the standing, and answers effects.
macro_rules! standing_call {
    ($name:ident, $method:ident) => {
        unsafe extern "C" fn $name(
            user: *mut c_void,
            id: u64,
            standing: *const GnoblinWmStanding,
            sink: *const GnoblinWmSink,
        ) -> i32 {
            guarded(|| {
                // SAFETY: the vtable's contract.
                unsafe {
                    let (stack, focus) = stack_of(standing);
                    let effects = host(user)
                        .wm
                        .$method(WindowId::new(id), Standing { stack: &stack, focus });
                    push_all(sink, &effects)
                }
            })
        }
    };
}

standing_call!(host_close, close);
standing_call!(host_raise, raise);
standing_call!(host_lower, lower);

unsafe extern "C" fn host_activate(
    user: *mut c_void,
    id: u64,
    raise: u32,
    standing: *const GnoblinWmStanding,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let (stack, focus) = stack_of(standing);
            let effects = host(user)
                .wm
                .activate(WindowId::new(id), raise != 0, Standing { stack: &stack, focus });
            push_all(sink, &effects)
        }
    })
}

unsafe extern "C" fn host_set_maximised(user: *mut c_void, id: u64, on: u32, sink: *const GnoblinWmSink) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let effects = host(user).wm.set_maximised(WindowId::new(id), on != 0);
            push_all(sink, &effects)
        }
    })
}

unsafe extern "C" fn host_set_fullscreen(
    user: *mut c_void,
    id: u64,
    on: u32,
    standing: *const GnoblinWmStanding,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let (stack, focus) = stack_of(standing);
            let effects = host(user)
                .wm
                .set_fullscreen(WindowId::new(id), on != 0, Standing { stack: &stack, focus });
            push_all(sink, &effects)
        }
    })
}

unsafe extern "C" fn host_set_minimised(
    user: *mut c_void,
    id: u64,
    on: u32,
    standing: *const GnoblinWmStanding,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let (stack, focus) = stack_of(standing);
            let effects = host(user)
                .wm
                .set_minimised(WindowId::new(id), on != 0, Standing { stack: &stack, focus });
            push_all(sink, &effects)
        }
    })
}

/// Defines a host function that takes a window and a rectangle, and answers effects or a refusal.
macro_rules! rect_call {
    ($name:ident, $method:ident) => {
        unsafe extern "C" fn $name(
            user: *mut c_void,
            id: u64,
            wanted: *const GnoblinWmRect,
            sink: *const GnoblinWmSink,
        ) -> i32 {
            guarded(|| {
                // SAFETY: the vtable's contract.
                unsafe {
                    let Some(wanted) = wanted.as_ref() else {
                        return abi::E_ERR;
                    };
                    reply(host(user).wm.$method(WindowId::new(id), rect_of(wanted)), sink)
                }
            })
        }
    };
}

rect_call!(host_move_resize, move_resize);
rect_call!(host_set_rect, set_rect);

unsafe extern "C" fn host_place(user: *mut c_void, id: u64, x: i32, y: i32, sink: *const GnoblinWmSink) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe { reply(host(user).wm.place(WindowId::new(id), Point::new(x, y)), sink) }
    })
}

unsafe extern "C" fn host_adopt_size(user: *mut c_void, id: u64, width: u32, height: u32) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        let changed = unsafe { host(user) }
            .wm
            .adopt_size(WindowId::new(id), Size::new(width, height));
        i32::from(changed)
    })
}

unsafe extern "C" fn host_move_to_output(user: *mut c_void, id: u64, target: u64, sink: *const GnoblinWmSink) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            reply(
                host(user).wm.move_to_output(WindowId::new(id), OutputId::new(target)),
                sink,
            )
        }
    })
}

unsafe extern "C" fn host_output_added(
    user: *mut c_void,
    id: u64,
    rect: *const GnoblinWmRect,
    standing: *const GnoblinWmStanding,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let Some(rect) = rect.as_ref() else {
                return abi::E_ERR;
            };
            let (stack, focus) = stack_of(standing);
            let effects =
                host(user)
                    .wm
                    .output_added(OutputId::new(id), rect_of(rect), Standing { stack: &stack, focus });
            push_all(sink, &effects)
        }
    })
}

unsafe extern "C" fn host_output_removed(
    user: *mut c_void,
    id: u64,
    standing: *const GnoblinWmStanding,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let (stack, focus) = stack_of(standing);
            let effects = host(user)
                .wm
                .output_removed(OutputId::new(id), Standing { stack: &stack, focus });
            push_all(sink, &effects)
        }
    })
}

unsafe extern "C" fn host_output_moved(
    user: *mut c_void,
    id: u64,
    rect: *const GnoblinWmRect,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let Some(rect) = rect.as_ref() else {
                return abi::E_ERR;
            };
            let effects = host(user).wm.output_moved(OutputId::new(id), rect_of(rect));
            push_all(sink, &effects)
        }
    })
}

unsafe extern "C" fn host_reserve(
    user: *mut c_void,
    id: u64,
    reserved: *const GnoblinWmInsets,
    sink: *const GnoblinWmSink,
) -> i32 {
    guarded(|| {
        // SAFETY: the vtable's contract.
        unsafe {
            let Some(reserved) = reserved.as_ref() else {
                return abi::E_ERR;
            };
            let effects = host(user).wm.reserve(OutputId::new(id), insets_of(reserved));
            push_all(sink, &effects)
        }
    })
}

// ---- the core's side: a vtable as a Rust provider ----------------------------------------------------------------------

/// The effects a provider pushed during one call, decoded.
#[derive(Default)]
struct EffectCollector {
    effects: Vec<WmEffect>,
    /// An effect did not decode, or the window ids of a stack were unreadable.
    invalid: bool,
}

unsafe extern "C" fn collect(sink: *mut c_void, effect: *const GnoblinWmEffect) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `sink` is the `EffectCollector` `WmServed::call` made, alive for the call; `effect` is valid until we return.
        let (collector, effect) = unsafe { (&mut *sink.cast::<EffectCollector>(), effect.as_ref()) };
        let Some(effect) = effect else {
            collector.invalid = true;
            return abi::E_ERR;
        };
        let decoded = match effect.kind {
            abi::WM_EFFECT_CONFIGURE => Some(WmEffect::Configure {
                window: WindowId::new(effect.window),
                geometry: rect_of(&effect.geometry),
                state: state_of(effect.flags),
            }),
            abi::WM_EFFECT_VISIBLE => Some(WmEffect::Visible {
                window: WindowId::new(effect.window),
                visible: effect.flags != 0,
            }),
            abi::WM_EFFECT_STACK => {
                if effect.ids.is_null() && effect.n_ids != 0 {
                    None
                } else if effect.n_ids == 0 {
                    Some(WmEffect::Stack(Vec::new()))
                } else {
                    // SAFETY: the provider promises `ids` is readable for `n_ids` ids during the call.
                    let ids = unsafe { std::slice::from_raw_parts(effect.ids, effect.n_ids) };
                    Some(WmEffect::Stack(ids.iter().copied().map(WindowId::new).collect()))
                }
            }
            abi::WM_EFFECT_FOCUS => Some(WmEffect::Focus(
                (effect.flags != 0).then(|| WindowId::new(effect.window)),
            )),
            _ => None,
        };
        match decoded {
            Some(effect) => {
                collector.effects.push(effect);
                abi::E_OK
            }
            None => {
                collector.invalid = true;
                abi::E_ERR
            }
        }
    }))
    .unwrap_or(abi::E_PANIC)
}

/// What came of one call into the vtable.
enum WmAnswer {
    /// It answered `E_OK` with effects that decode.
    Done,
    /// It refused, as the contract allows: nothing changed.
    Refused(WmRefusal),
    /// It cannot be trusted any more, and why.
    Broken(String),
}

fn classify(rc: i32, collector: &EffectCollector) -> WmAnswer {
    if collector.invalid {
        return WmAnswer::Broken("sent an effect that does not decode".to_owned());
    }
    match rc {
        abi::E_OK => WmAnswer::Done,
        abi::E_PANIC => WmAnswer::Broken("panicked".to_owned()),
        rc if rc < 0 => WmAnswer::Broken(format!("answered error {rc}")),
        rc => refusal_of(rc).map_or_else(
            || WmAnswer::Broken(format!("answered {rc}, which is not a refusal the contract has")),
            WmAnswer::Refused,
        ),
    }
}

/// A registered provider: the vtable, its state and the copy of its record the trait's reads answer from.
struct WmServed {
    vtable: GnoblinWmVtable,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    windows: BTreeMap<WindowId, Window>,
    outputs: Vec<OutputRecord>,
    /// A call through a shared reference failed: the next call that can swap the provider does.
    broken: Cell<bool>,
}

impl Drop for WmServed {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: `drop_user` and `user` came from one registration; this is the one call it is owed.
            unsafe { drop_user(self.user) };
        }
    }
}

impl WmServed {
    /// Calls into the vtable with a sink, and decodes what the provider pushed.
    fn call(
        &self,
        call: impl FnOnce(&GnoblinWmVtable, *mut c_void, *const GnoblinWmSink) -> i32,
    ) -> (WmAnswer, Vec<WmEffect>) {
        let mut collector = EffectCollector::default();
        let sink = GnoblinWmSink {
            size: size_of::<GnoblinWmSink>() as u32,
            reserved: 0,
            sink: (&raw mut collector).cast(),
            push: collect,
        };
        let rc = call(&self.vtable, self.user, &raw const sink);
        (classify(rc, &collector), collector.effects)
    }

    /// Reads the provider's record again. `false` when it does not decode or does not hold still.
    fn refresh(&mut self) -> bool {
        // SAFETY: `user` and the vtable are the registration's; a null `out` with a zero cap asks for the count only.
        let wanted = unsafe { (self.vtable.windows)(self.user, core::ptr::null_mut(), 0) };
        if wanted > MAX_WINDOWS * 4 {
            return false;
        }
        let blank = window_to(&Window {
            id: WindowId::new(0),
            kind: WindowKind::Normal,
            transient_for: None,
            output: None,
            geometry: Rect::new(0, 0, 0, 0),
            restore: Rect::new(0, 0, 0, 0),
            state: WindowState::default(),
        });
        let mut windows = vec![blank; wanted];
        // SAFETY: `windows` is writable for `wanted` records.
        let held = unsafe { (self.vtable.windows)(self.user, windows.as_mut_ptr(), wanted) };
        if held != wanted {
            return false;
        }
        let mut decoded = BTreeMap::new();
        for wire in &windows {
            let Some(window) = window_of(wire) else {
                return false;
            };
            decoded.insert(window.id, window);
        }
        // SAFETY: as above.
        let wanted = unsafe { (self.vtable.outputs)(self.user, core::ptr::null_mut(), 0) };
        if wanted > 1024 {
            return false;
        }
        let mut outputs = vec![
            output_to(&OutputRecord {
                id: OutputId::new(0),
                rect: Rect::new(0, 0, 0, 0),
                reserved: Insets::ZERO,
            });
            wanted
        ];
        // SAFETY: `outputs` is writable for `wanted` records.
        if unsafe { (self.vtable.outputs)(self.user, outputs.as_mut_ptr(), wanted) } != wanted {
            return false;
        }
        self.windows = decoded;
        self.outputs = outputs.iter().map(output_of).collect();
        true
    }
}

enum WmHeld {
    WmServed(Box<WmServed>),
    Null(Box<NullWm>),
}

/// A window manager registered through the ABI, as the compositor's [`WindowManager`].
///
/// It is `Send`: the core builds it on the thread that registered the provider and hands it to the compositor's thread, which
/// is the only one that calls it afterwards.
pub struct ForeignWm {
    inner: WmHeld,
    /// Effects the fall-back owes the clients and has not been able to hand out yet.
    owed: Vec<WmEffect>,
    report: Report,
}

// SAFETY: `ForeignWm` holds a raw `user` pointer and function pointers, which are not `Send` by themselves. The registration
// contract says the provider's functions run on the compositor's thread only, after the registering thread has handed the
// provider over, so no two threads use it at once.
unsafe impl Send for ForeignWm {}

impl std::fmt::Debug for ForeignWm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForeignWm")
            .field("falling_back", &matches!(self.inner, WmHeld::Null(_)))
            .finish_non_exhaustive()
    }
}

impl ForeignWm {
    /// Wraps the provider a registration named. `report` says, once, why the provider was dropped. The wrapper does not own
    /// `user` yet: until [`ForeignWm::adopt`] it is the registrant's, so a registration that fails after this leaves it there.
    ///
    /// # Errors
    /// The vtable is shorter than this build's, or the provider's record does not decode.
    ///
    /// # Safety
    /// `vtable` points at a vtable whose functions may be called with `user` from one thread at a time for as long as the
    /// result lives.
    pub unsafe fn new(vtable: *const GnoblinWmVtable, user: *mut c_void, report: Report) -> Result<Self, String> {
        // SAFETY: the caller guarantees the pointer is null or valid for `size` bytes.
        let Some(table) = (unsafe { vtable.as_ref() }) else {
            return Err("a null window manager vtable".to_owned());
        };
        if (table.size as usize) < size_of::<GnoblinWmVtable>() {
            return Err(format!(
                "the window manager vtable is {} bytes; this core needs {}",
                table.size,
                size_of::<GnoblinWmVtable>()
            ));
        }
        let mut live = WmServed {
            vtable: *table,
            user,
            drop_user: None,
            windows: BTreeMap::new(),
            outputs: Vec::new(),
            broken: Cell::new(false),
        };
        if !live.refresh() {
            return Err("the window manager's record does not decode".to_owned());
        }
        Ok(Self {
            inner: WmHeld::WmServed(Box::new(live)),
            owed: Vec::new(),
            report,
        })
    }

    /// Takes ownership of `user`: `drop_user` runs on it once, when the provider ends. Called when the registration has
    /// succeeded, so one that fails first leaves `user` with the registrant.
    pub fn adopt(&mut self, drop_user: Option<GnoblinDropFn>) {
        if let WmHeld::WmServed(live) = &mut self.inner {
            live.drop_user = drop_user;
        }
    }

    /// Whether the provider has been dropped for the null one.
    #[must_use]
    pub fn fell_back(&self) -> bool {
        matches!(self.inner, WmHeld::Null(_))
    }

    /// Drops the provider for a [`NullWm`] holding what it held, and says why. Returns what the clients are owed.
    fn fall_back(&mut self, seat: Standing<'_>, why: &str) -> Vec<WmEffect> {
        let WmHeld::WmServed(live) = &self.inner else {
            return Vec::new();
        };
        (self.report)(&format!(
            "the window manager provider {why}; the session runs NullWm from now on"
        ));
        let record = HeldRecord {
            windows: live.windows.values().cloned().collect(),
            outputs: live.outputs.clone(),
        };
        let mut null = NullWm::default();
        let effects = hand_over(&record, &mut null, seat);
        // Dropping the old inner frees the provider's `user`.
        self.inner = WmHeld::Null(Box::new(null));
        effects
    }

    /// Runs one call that can change the record: through the vtable while the provider is live, through `null` after.
    fn run(
        &mut self,
        seat: Standing<'_>,
        what: &str,
        live: impl FnOnce(&GnoblinWmVtable, *mut c_void, *const GnoblinWmSink) -> i32,
        null: impl FnOnce(&mut NullWm) -> Result<Vec<WmEffect>, WmRefusal>,
    ) -> Result<Vec<WmEffect>, WmRefusal> {
        if let WmHeld::WmServed(held) = &mut self.inner {
            let broken = if held.broken.get() {
                Some("failed an earlier call".to_owned())
            } else {
                let (outcome, effects) = held.call(live);
                match outcome {
                    WmAnswer::Done if held.refresh() => return Ok(self.settled(effects)),
                    WmAnswer::Done => Some("holds a record that does not decode".to_owned()),
                    WmAnswer::Refused(refusal) => return Err(refusal),
                    WmAnswer::Broken(why) => Some(format!("{what} {why}")),
                }
            };
            if let Some(why) = broken {
                let lead = self.fall_back(seat, &why);
                self.owed.extend(lead);
            }
        }
        let WmHeld::Null(fallback) = &mut self.inner else {
            return Err(WmRefusal::Gone);
        };
        let result = null(fallback);
        result.map(|effects| self.settled(effects))
    }

    /// `effects` with what the fall-back owes in front of them.
    fn settled(&mut self, effects: Vec<WmEffect>) -> Vec<WmEffect> {
        if self.owed.is_empty() {
            return effects;
        }
        let mut all = std::mem::take(&mut self.owed);
        all.extend(effects);
        all
    }

    /// A call through a shared reference: it cannot swap the provider, so a failure only marks it.
    fn ask(&self, call: impl FnOnce(&GnoblinWmVtable, *mut c_void, *const GnoblinWmSink) -> i32) -> Vec<WmEffect> {
        let WmHeld::WmServed(live) = &self.inner else {
            return Vec::new();
        };
        if live.broken.get() {
            return Vec::new();
        }
        let (outcome, effects) = live.call(call);
        match outcome {
            WmAnswer::Done => effects,
            WmAnswer::Refused(_) => Vec::new(),
            WmAnswer::Broken(why) => {
                (self.report)(&format!(
                    "the window manager provider {why}; the session runs NullWm from now on"
                ));
                live.broken.set(true);
                Vec::new()
            }
        }
    }
}

/// Builds the C view of the paint order and focus a call carries.
struct StandingBuf {
    ids: Vec<u64>,
    focus: Option<u64>,
}

impl StandingBuf {
    fn of(seat: Standing<'_>) -> Self {
        Self {
            ids: seat.stack.iter().map(|id| id.get()).collect(),
            focus: seat.focus.map(WindowId::get),
        }
    }

    fn wire(&self) -> GnoblinWmStanding {
        GnoblinWmStanding {
            size: size_of::<GnoblinWmStanding>() as u32,
            flags: if self.focus.is_some() {
                abi::WM_STANDING_HAS_FOCUS
            } else {
                0
            },
            focus: self.focus.unwrap_or(0),
            stack: self.ids.as_ptr(),
            n_stack: self.ids.len(),
        }
    }
}

impl WindowManager for ForeignWm {
    fn window(&self, id: WindowId) -> Option<&Window> {
        match &self.inner {
            WmHeld::WmServed(live) => live.windows.get(&id),
            WmHeld::Null(null) => null.window(id),
        }
    }

    fn windows(&self) -> Box<dyn Iterator<Item = &Window> + '_> {
        match &self.inner {
            WmHeld::WmServed(live) => Box::new(live.windows.values()),
            WmHeld::Null(null) => null.windows(),
        }
    }

    fn output(&self, id: OutputId) -> Option<&OutputRecord> {
        match &self.inner {
            WmHeld::WmServed(live) => live.outputs.iter().find(|output| output.id == id),
            WmHeld::Null(null) => null.output(id),
        }
    }

    fn outputs(&self) -> Box<dyn Iterator<Item = &OutputRecord> + '_> {
        match &self.inner {
            WmHeld::WmServed(live) => Box::new(live.outputs.iter()),
            WmHeld::Null(null) => null.outputs(),
        }
    }

    fn set_role(
        &mut self,
        id: WindowId,
        kind: WindowKind,
        transient_for: Option<WindowId>,
        seat: Standing<'_>,
    ) -> Vec<WmEffect> {
        let standing = StandingBuf::of(seat);
        let flags = if transient_for.is_some() {
            abi::WM_WINDOW_HAS_PARENT
        } else {
            0
        };
        self.run(
            seat,
            "set_role",
            |vt, user, sink| {
                let wire = standing.wire();
                // SAFETY: the registration's contract; `wire` and `sink` live for the call.
                unsafe {
                    (vt.set_role)(
                        user,
                        id.get(),
                        kind_to(kind),
                        flags,
                        transient_for.map_or(0, WindowId::get),
                        &raw const wire,
                        sink,
                    )
                }
            },
            |null| Ok(null.set_role(id, kind, transient_for, seat)),
        )
        .unwrap_or_default()
    }

    fn may_take_focus(&self, id: WindowId, policy: NewWindowFocus, seat: Standing<'_>) -> bool {
        match &self.inner {
            WmHeld::Null(null) => null.may_take_focus(id, policy, seat),
            WmHeld::WmServed(live) => {
                if live.broken.get() {
                    return false;
                }
                let standing = StandingBuf::of(seat);
                let wire = standing.wire();
                let policy = match policy {
                    NewWindowFocus::Allow => abi::WM_FOCUS_ALLOW,
                    NewWindowFocus::Prevent => abi::WM_FOCUS_PREVENT,
                };
                // SAFETY: the registration's contract; `wire` lives for the call.
                let rc = unsafe { (live.vtable.may_take_focus)(live.user, id.get(), policy, &raw const wire) };
                match rc {
                    0 => false,
                    1 => true,
                    rc => {
                        (self.report)(&format!(
                            "the window manager provider answered {rc} to may_take_focus; the session runs NullWm from now on"
                        ));
                        live.broken.set(true);
                        false
                    }
                }
            }
        }
    }

    fn open(&mut self, open: Open, seat: Standing<'_>) -> Result<Vec<WmEffect>, WmRefusal> {
        let standing = StandingBuf::of(seat);
        let mut flags = 0;
        if open.transient_for.is_some() {
            flags |= abi::WM_WINDOW_HAS_PARENT;
        }
        if open.requested.is_some() {
            flags |= abi::WM_OPEN_HAS_REQUESTED;
        }
        let wire_open = GnoblinWmOpen {
            size: size_of::<GnoblinWmOpen>() as u32,
            kind: kind_to(open.kind),
            flags,
            reserved: 0,
            id: open.id.get(),
            transient_for: open.transient_for.map_or(0, WindowId::get),
            requested: rect_to(open.requested.unwrap_or(Rect::new(0, 0, 0, 0))),
        };
        self.run(
            seat,
            "open",
            |vt, user, sink| {
                let wire = standing.wire();
                // SAFETY: the registration's contract; the structs live for the call.
                unsafe { (vt.open)(user, &raw const wire_open, &raw const wire, sink) }
            },
            |null| null.open(open, seat),
        )
    }

    fn close(&mut self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
        let standing = StandingBuf::of(seat);
        self.run(
            seat,
            "close",
            |vt, user, sink| {
                let wire = standing.wire();
                // SAFETY: the registration's contract.
                unsafe { (vt.close)(user, id.get(), &raw const wire, sink) }
            },
            |null| Ok(null.close(id, seat)),
        )
        .unwrap_or_default()
    }

    fn activate(&self, id: WindowId, raise: bool, seat: Standing<'_>) -> Vec<WmEffect> {
        if let WmHeld::Null(null) = &self.inner {
            return null.activate(id, raise, seat);
        }
        let standing = StandingBuf::of(seat);
        self.ask(|vt, user, sink| {
            let wire = standing.wire();
            // SAFETY: the registration's contract.
            unsafe { (vt.activate)(user, id.get(), u32::from(raise), &raw const wire, sink) }
        })
    }

    fn raise(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
        if let WmHeld::Null(null) = &self.inner {
            return null.raise(id, seat);
        }
        let standing = StandingBuf::of(seat);
        self.ask(|vt, user, sink| {
            let wire = standing.wire();
            // SAFETY: the registration's contract.
            unsafe { (vt.raise)(user, id.get(), &raw const wire, sink) }
        })
    }

    fn lower(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
        if let WmHeld::Null(null) = &self.inner {
            return null.lower(id, seat);
        }
        let standing = StandingBuf::of(seat);
        self.ask(|vt, user, sink| {
            let wire = standing.wire();
            // SAFETY: the registration's contract.
            unsafe { (vt.lower)(user, id.get(), &raw const wire, sink) }
        })
    }

    fn set_maximised(&mut self, id: WindowId, on: bool) -> Vec<WmEffect> {
        self.run(
            NO_SEAT,
            "set_maximised",
            // SAFETY: the registration's contract.
            |vt, user, sink| unsafe { (vt.set_maximised)(user, id.get(), u32::from(on), sink) },
            |null| Ok(null.set_maximised(id, on)),
        )
        .unwrap_or_default()
    }

    fn set_fullscreen(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<WmEffect> {
        let standing = StandingBuf::of(seat);
        self.run(
            seat,
            "set_fullscreen",
            |vt, user, sink| {
                let wire = standing.wire();
                // SAFETY: the registration's contract.
                unsafe { (vt.set_fullscreen)(user, id.get(), u32::from(on), &raw const wire, sink) }
            },
            |null| Ok(null.set_fullscreen(id, on, seat)),
        )
        .unwrap_or_default()
    }

    fn set_minimised(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<WmEffect> {
        let standing = StandingBuf::of(seat);
        self.run(
            seat,
            "set_minimised",
            |vt, user, sink| {
                let wire = standing.wire();
                // SAFETY: the registration's contract.
                unsafe { (vt.set_minimised)(user, id.get(), u32::from(on), &raw const wire, sink) }
            },
            |null| Ok(null.set_minimised(id, on, seat)),
        )
        .unwrap_or_default()
    }

    fn move_resize(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal> {
        let rect = rect_to(wanted);
        self.run(
            NO_SEAT,
            "move_resize",
            // SAFETY: the registration's contract; `rect` lives for the call.
            |vt, user, sink| unsafe { (vt.move_resize)(user, id.get(), &raw const rect, sink) },
            |null| null.move_resize(id, wanted),
        )
    }

    fn place(&mut self, id: WindowId, at: Point) -> Result<Vec<WmEffect>, WmRefusal> {
        self.run(
            NO_SEAT,
            "place",
            // SAFETY: the registration's contract.
            |vt, user, sink| unsafe { (vt.place)(user, id.get(), at.x, at.y, sink) },
            |null| null.place(id, at),
        )
    }

    fn set_rect(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal> {
        let rect = rect_to(wanted);
        self.run(
            NO_SEAT,
            "set_rect",
            // SAFETY: the registration's contract; `rect` lives for the call.
            |vt, user, sink| unsafe { (vt.set_rect)(user, id.get(), &raw const rect, sink) },
            |null| null.set_rect(id, wanted),
        )
    }

    fn adopt_size(&mut self, id: WindowId, size: Size) -> bool {
        let answer = match &self.inner {
            WmHeld::Null(_) => None,
            // SAFETY: the registration's contract.
            WmHeld::WmServed(live) => {
                Some(unsafe { (live.vtable.adopt_size)(live.user, id.get(), size.width, size.height) })
            }
        };
        let Some(answer) = answer else {
            return match &mut self.inner {
                WmHeld::Null(null) => null.adopt_size(id, size),
                WmHeld::WmServed(_) => false,
            };
        };
        match answer {
            0 => false,
            1 => {
                let readable = match &mut self.inner {
                    WmHeld::WmServed(live) => live.refresh(),
                    WmHeld::Null(_) => true,
                };
                if !readable {
                    let lead = self.fall_back(NO_SEAT, "holds a record that does not decode");
                    self.owed.extend(lead);
                }
                true
            }
            rc => {
                let lead = self.fall_back(NO_SEAT, &format!("answered {rc} to adopt_size"));
                self.owed.extend(lead);
                false
            }
        }
    }

    fn move_to_output(&mut self, id: WindowId, target: OutputId) -> Result<Vec<WmEffect>, WmRefusal> {
        self.run(
            NO_SEAT,
            "move_to_output",
            // SAFETY: the registration's contract.
            |vt, user, sink| unsafe { (vt.move_to_output)(user, id.get(), target.get(), sink) },
            |null| null.move_to_output(id, target),
        )
    }

    fn output_added(&mut self, id: OutputId, rect: Rect, seat: Standing<'_>) -> Vec<WmEffect> {
        let standing = StandingBuf::of(seat);
        let wire_rect = rect_to(rect);
        self.run(
            seat,
            "output_added",
            |vt, user, sink| {
                let wire = standing.wire();
                // SAFETY: the registration's contract.
                unsafe { (vt.output_added)(user, id.get(), &raw const wire_rect, &raw const wire, sink) }
            },
            |null| Ok(null.output_added(id, rect, seat)),
        )
        .unwrap_or_default()
    }

    fn output_removed(&mut self, id: OutputId, seat: Standing<'_>) -> Vec<WmEffect> {
        let standing = StandingBuf::of(seat);
        self.run(
            seat,
            "output_removed",
            |vt, user, sink| {
                let wire = standing.wire();
                // SAFETY: the registration's contract.
                unsafe { (vt.output_removed)(user, id.get(), &raw const wire, sink) }
            },
            |null| Ok(null.output_removed(id, seat)),
        )
        .unwrap_or_default()
    }

    fn output_moved(&mut self, id: OutputId, rect: Rect) -> Vec<WmEffect> {
        let wire_rect = rect_to(rect);
        self.run(
            NO_SEAT,
            "output_moved",
            // SAFETY: the registration's contract.
            |vt, user, sink| unsafe { (vt.output_moved)(user, id.get(), &raw const wire_rect, sink) },
            |null| Ok(null.output_moved(id, rect)),
        )
        .unwrap_or_default()
    }

    fn reserve(&mut self, id: OutputId, reserved: Insets) -> Vec<WmEffect> {
        let wire_insets = insets_to(reserved);
        self.run(
            NO_SEAT,
            "reserve",
            // SAFETY: the registration's contract.
            |vt, user, sink| unsafe { (vt.reserve)(user, id.get(), &raw const wire_insets, sink) },
            |null| Ok(null.reserve(id, reserved)),
        )
        .unwrap_or_default()
    }
}

/// The paint order a call that is not given one falls back with: none and no focus. A fall-back that starts in such a call
/// replays the record without a stacking, which the compositor re-derives from its own scene.
const NO_SEAT: Standing<'static> = Standing {
    stack: &[],
    focus: None,
};

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    fn id(raw: u64) -> WindowId {
        WindowId::new(raw)
    }

    fn seat() -> Standing<'static> {
        NO_SEAT
    }

    fn reports() -> (Report, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        (
            Box::new(move |line| sink.lock().expect("lock").push(line.to_owned())),
            lines,
        )
    }

    /// A provider made of `inner` behind the vtable, wrapped back as a Rust provider.
    fn through_the_abi(inner: Box<dyn WindowManager>) -> (ForeignWm, Arc<Mutex<Vec<String>>>) {
        let exported = export_wm(inner);
        let (report, lines) = reports();
        // SAFETY: the vtable is the static `export_wm` made, `user` its state, and `drop_user` frees it once.
        let mut foreign =
            unsafe { ForeignWm::new(exported.vtable, exported.user, report) }.expect("the exported provider wraps");
        foreign.adopt(Some(exported.drop_user));
        (foreign, lines)
    }

    fn open_at(wm: &mut dyn WindowManager, window: u64, parent: Option<u64>) -> Result<Vec<WmEffect>, WmRefusal> {
        wm.open(
            Open {
                id: id(window),
                kind: if parent.is_some() {
                    WindowKind::Dialog
                } else {
                    WindowKind::Normal
                },
                transient_for: parent.map(id),
                requested: Some(Rect::new(10, 20, 640, 480)),
            },
            seat(),
        )
    }

    /// One call of the provider a test makes, and the effects it answers.
    type Step = Box<dyn Fn(&mut dyn WindowManager) -> Vec<WmEffect>>;

    #[test]
    fn a_provider_behind_the_vtable_decides_what_it_decides_directly() {
        let (mut through, lines) = through_the_abi(Box::<NullWm>::default());
        let mut direct = NullWm::default();
        let steps: Vec<Step> = vec![
            Box::new(|wm| wm.output_added(OutputId::new(1), MONITOR, seat())),
            Box::new(|wm| open_at(wm, 1, None).expect("opens")),
            Box::new(|wm| open_at(wm, 2, Some(1)).expect("opens")),
            Box::new(|wm| wm.set_maximised(id(1), true)),
            Box::new(|wm| {
                wm.reserve(
                    OutputId::new(1),
                    Insets {
                        top: 30,
                        ..Insets::ZERO
                    },
                )
            }),
            Box::new(|wm| wm.move_resize(id(2), Rect::new(5, 6, 300, 200)).expect("floats")),
            Box::new(|wm| wm.set_minimised(id(2), true, seat())),
            Box::new(|wm| wm.raise(id(1), seat())),
            Box::new(|wm| wm.close(id(1), seat())),
        ];
        for step in &steps {
            assert_eq!(step(&mut through), step(&mut direct));
        }
        assert_eq!(
            through.windows().cloned().collect::<Vec<_>>(),
            direct.windows().cloned().collect::<Vec<_>>()
        );
        assert_eq!(
            through.outputs().copied().collect::<Vec<_>>(),
            direct.outputs().copied().collect::<Vec<_>>()
        );
        assert!(!through.fell_back());
        assert!(
            lines.lock().expect("lock").is_empty(),
            "nothing went wrong, so nothing was said"
        );
    }

    #[test]
    fn a_refusal_crosses_as_the_refusal_it_is() {
        let (mut through, _) = through_the_abi(Box::<NullWm>::default());
        through.output_added(OutputId::new(1), MONITOR, seat());
        open_at(&mut through, 1, None).expect("opens");
        assert_eq!(open_at(&mut through, 1, None), Err(WmRefusal::Duplicate));
        assert_eq!(through.move_resize(id(9), MONITOR), Err(WmRefusal::Gone));
        assert!(!through.fell_back(), "a refusal is an answer, not a failure");
    }

    /// The vtable of a provider that fails every `open` the way `how` says.
    fn failing_open(
        how: unsafe extern "C" fn(
            *mut c_void,
            *const GnoblinWmOpen,
            *const GnoblinWmStanding,
            *const GnoblinWmSink,
        ) -> i32,
    ) -> (ForeignWm, Arc<Mutex<Vec<String>>>) {
        let exported = export_wm(Box::<NullWm>::default());
        // The vtable is a copy with one function replaced; the wrapper copies it again, so a local is enough.
        let mut table = *exported.vtable;
        table.open = how;
        let (report, lines) = reports();
        // SAFETY: the table and the state come from `export_wm`; `drop_user` frees the state once.
        let mut foreign = unsafe { ForeignWm::new(&raw const table, exported.user, report) }.expect("wraps");
        foreign.adopt(Some(exported.drop_user));
        (foreign, lines)
    }

    unsafe extern "C" fn panics(
        _: *mut c_void,
        _: *const GnoblinWmOpen,
        _: *const GnoblinWmStanding,
        _: *const GnoblinWmSink,
    ) -> i32 {
        abi::E_PANIC
    }

    unsafe extern "C" fn pushes_nonsense(
        _: *mut c_void,
        _: *const GnoblinWmOpen,
        _: *const GnoblinWmStanding,
        sink: *const GnoblinWmSink,
    ) -> i32 {
        let effect = GnoblinWmEffect {
            size: size_of::<GnoblinWmEffect>() as u32,
            kind: 99,
            flags: 0,
            reserved: 0,
            window: 1,
            geometry: rect_to(Rect::new(0, 0, 0, 0)),
            ids: core::ptr::null(),
            n_ids: 0,
        };
        // SAFETY: the sink is the core's, valid for the call.
        unsafe { ((*sink).push)((*sink).sink, &raw const effect) };
        abi::E_OK
    }

    unsafe extern "C" fn refuses_what_the_contract_lacks(
        _: *mut c_void,
        _: *const GnoblinWmOpen,
        _: *const GnoblinWmStanding,
        _: *const GnoblinWmSink,
    ) -> i32 {
        77
    }

    #[test]
    fn a_provider_that_fails_is_replaced_by_null_wm_with_its_windows_and_the_failing_call_is_served() {
        for how in [panics, pushes_nonsense, refuses_what_the_contract_lacks] {
            let (mut wm, lines) = failing_open(how);
            wm.output_added(OutputId::new(1), MONITOR, seat());
            // The provider's own `open` is the broken one, so the first window is served by the fall-back.
            let effects = open_at(&mut wm, 1, None).expect("the failing call is answered by NullWm");
            assert!(wm.fell_back());
            assert_eq!(
                wm.window(id(1)).expect("held").geometry,
                Rect::new(0, 0, 640, 480),
                "NullWm puts it at the origin"
            );
            assert!(
                effects
                    .iter()
                    .any(|effect| matches!(effect, WmEffect::Configure { .. }))
            );
            // It keeps working, and says nothing more.
            open_at(&mut wm, 2, None).expect("opens");
            assert_eq!(wm.windows().count(), 2);
            let said = lines.lock().expect("lock").clone();
            assert_eq!(said.len(), 1, "one line, once: {said:?}");
            assert!(said[0].contains("NullWm"), "{said:?}");
        }
    }

    #[test]
    fn what_a_failed_provider_held_is_kept() {
        let (mut wm, _) = failing_open(panics);
        wm.output_added(OutputId::new(1), MONITOR, seat());
        let _ = open_at(&mut wm, 1, None);
        assert!(wm.fell_back());
        wm.move_resize(id(1), Rect::new(100, 100, 300, 300)).expect("floats");
        assert_eq!(wm.window(id(1)).expect("held").geometry, Rect::new(100, 100, 300, 300));
        assert_eq!(wm.outputs().count(), 1);
    }

    #[test]
    fn a_vtable_shorter_than_this_build_is_refused_and_the_state_stays_with_the_registrant() {
        let exported = export_wm(Box::<NullWm>::default());
        let mut table = *exported.vtable;
        table.size = 16;
        let (report, _) = reports();
        // SAFETY: the vtable is a stack copy, valid for the call.
        let wrapped = unsafe { ForeignWm::new(&raw const table, exported.user, report) };
        assert!(wrapped.is_err());
        // The registrant still owns `user`: free it ourselves.
        // SAFETY: `new` did not take it.
        unsafe { (exported.drop_user)(exported.user) };
    }
}
