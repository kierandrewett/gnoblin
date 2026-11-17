//! Built-in layouts that belong to no workspace: `builtin_new` and the functions that act on its handle.
//!
//! A workspace's layout is computed by the library when the workspace is dirty, from the clients the library knows. A shell that
//! keeps its own workspaces (the TypeScript library's, which still drive their windows in the script) has no use for that pass and
//! needs the algorithms: it makes the layout, hands it the clients it has and the area, and gets the placements back. The layout is
//! the same object the workspace's is (`builtin/`), so the rectangles, the options and their checks, the actions and the state a
//! kind keeps (a dwindle's tree) are the library's once, for every language.
//!
//! The clients of a query are ids the caller chooses. The library reads nothing of them from the tree: a layout is pure over ids and
//! rectangles. A call never touches a workspace, and posts no turn.
#![allow(unsafe_code)]

use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinNode};
use gnoblin_std_abi::layouts::{LAYOUT_KINDS, layout_kind};
use gnoblin_std_abi::{
    GnoblinStdBuiltin, GnoblinStdLayoutQuery, GnoblinStdOption, GnoblinStdPlacement, GnoblinStdPlacements,
    GnoblinStdSplit, STD_PLACEMENT_BOUNDS, STD_PLACEMENT_HIDDEN, STD_PLACEMENT_OFFSET, STD_PLACEMENT_TILED,
    STD_QUERY_FOCUSED_SIZE, STD_QUERY_POINTER,
};

use crate::builtin::{ActionEnv, Builtin, encode_value, read_args, read_one, read_options, read_text};
use crate::error::{Fail, Res};
use crate::layout::{LayoutContext, LayoutObj, Placement};
use crate::state::next_id;
use crate::table::{call, fill_out, put, read, read_ext, std_of};

thread_local! {
    /// The placements the last `builtin_compute` lent, valid until the next call on this thread.
    static LENT_PLACEMENTS: std::cell::RefCell<Vec<GnoblinStdPlacement>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The layout behind `handle`, or why there is none.
fn layout_of(ctx: GnoblinCtx, handle: GnoblinStdBuiltin) -> Res<Rc<LayoutObj>> {
    read(ctx, |st| {
        st.builtins.get(&handle).map(Rc::clone).ok_or_else(|| {
            Fail::no_such(format!(
                "there is no built-in layout {handle}: it was freed, or never made"
            ))
        })
    })
}

fn builtin(layout: &LayoutObj) -> &Builtin {
    layout
        .as_builtin()
        .unwrap_or_else(|| unreachable!("only built-in layouts are stored"))
}

/// What a layout is asked about, as the library's own context.
///
/// # Safety
///
/// `query` must be null or a `GnoblinStdLayoutQuery` whose `children` points at `n_children` ids.
unsafe fn read_query(query: *const GnoblinStdLayoutQuery) -> Res<(LayoutContext, ActionEnv)> {
    // SAFETY: forwarded from the caller; every field of the query is an integer, a float or a raw pointer.
    let raw = unsafe { read_ext(query, "layout query") }?;
    let children: Vec<GnoblinNode> = if raw.children.is_null() || raw.n_children == 0 {
        Vec::new()
    } else {
        // SAFETY: the caller guarantees `n_children` readable ids at `children`.
        unsafe { std::slice::from_raw_parts(raw.children, raw.n_children) }.to_vec()
    };
    let numbers = [
        raw.area.x,
        raw.area.y,
        raw.area.width,
        raw.area.height,
        raw.gap_inner,
        raw.gap_outer,
    ];
    if numbers.iter().any(|n| !n.is_finite()) || raw.area.width < 0.0 || raw.area.height < 0.0 {
        return Err(Fail::type_error(
            "a layout's area and gaps must be finite numbers, and the area's size at least 0",
        ));
    }
    let pointer = (raw.flags & STD_QUERY_POINTER != 0).then_some((raw.pointer_x, raw.pointer_y));
    let focused = (raw.focused != 0).then_some(raw.focused);
    let focused_size = (raw.flags & STD_QUERY_FOCUSED_SIZE != 0 && focused.is_some())
        .then_some((raw.focused_width, raw.focused_height));
    let cx = LayoutContext {
        workspace: 0,
        children: children.clone(),
        area: Rect::new(raw.area.x, raw.area.y, raw.area.width, raw.area.height),
        gap_inner: raw.gap_inner,
        gap_outer: raw.gap_outer,
        focused,
        pointer,
    };
    let env = ActionEnv {
        clients: children,
        focused,
        focused_size,
    };
    Ok((cx, env))
}

pub(crate) unsafe extern "C" fn builtin_new(
    ctx: GnoblinCtx,
    kind: u32,
    options: *const GnoblinStdOption,
    n_options: usize,
    out: *mut GnoblinStdBuiltin,
) -> i32 {
    call("builtin_new", || {
        let std = std_of(ctx)?;
        let desc = layout_kind(kind).ok_or_else(|| {
            Fail::type_error(format!(
                "unknown built-in layout kind {kind}: the kinds are 1 to {}",
                LAYOUT_KINDS.len()
            ))
        })?;
        // SAFETY: the caller guarantees `options` points at `n_options` records.
        let options = unsafe { read_options(desc, options, n_options) }?;
        let layout = LayoutObj::builtin(Builtin::new(desc, options));
        let id = next_id();
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, id) }?;
        std.st.borrow_mut().builtins.insert(id, layout);
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_free(ctx: GnoblinCtx, handle: GnoblinStdBuiltin) -> i32 {
    call("builtin_free", || {
        let std = std_of(ctx)?;
        let gone = std.st.borrow_mut().builtins.remove(&handle);
        gone.map(drop)
            .ok_or_else(|| Fail::no_such(format!("there is no built-in layout {handle}")))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_set(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    option: *const GnoblinStdOption,
) -> i32 {
    call("builtin_set", || {
        let layout = layout_of(ctx, handle)?;
        let builtin = builtin(&layout);
        // SAFETY: the caller guarantees `option` is a record.
        let (desc, value) = unsafe { read_one(builtin.kind, option) }?;
        builtin.set_option(desc.name, value);
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_get(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    name: *const u8,
    len: usize,
    out: *mut GnoblinStdOption,
) -> i32 {
    call("builtin_get", || {
        let layout = layout_of(ctx, handle)?;
        let builtin = builtin(&layout);
        let text = abi::GnoblinStr { ptr: name, len };
        // SAFETY: the caller guarantees `len` readable bytes at `name`.
        let name = unsafe { read_text(&text) }?;
        let desc = builtin
            .kind
            .option(name)
            .ok_or_else(|| Fail::no_such(format!("the {} layout has no option `{name}`", builtin.kind.name)))?;
        let record = encode_value(&builtin.option(name), Some(&desc.value));
        // SAFETY: the caller guarantees `out` is a writable record whose first field is its size.
        unsafe { fill_out(out, &record, "option") }?;
        Ok(abi::E_OK)
    })
}

/// A placement as the shell reads it.
fn placement_out(placement: &Placement) -> GnoblinStdPlacement {
    let mut flags = 0;
    let bounds = placement.bounds.as_ref().map_or_else(
        || gnoblin_std_abi::GnoblinStdRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        },
        |rect| {
            flags |= STD_PLACEMENT_BOUNDS;
            gnoblin_std_abi::GnoblinStdRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            }
        },
    );
    if !placement.visible {
        flags |= STD_PLACEMENT_HIDDEN;
    }
    if placement.tiled {
        flags |= STD_PLACEMENT_TILED;
    }
    let offset = placement
        .offset
        .inspect(|_| flags |= STD_PLACEMENT_OFFSET)
        .unwrap_or((0.0, 0.0));
    GnoblinStdPlacement {
        size: size_of::<GnoblinStdPlacement>() as u32,
        flags,
        node: placement.node,
        bounds,
        z: placement.z,
        rotation_x: placement.rotation[0],
        rotation_y: placement.rotation[1],
        rotation_z: placement.rotation[2],
        scale_x: placement.scale[0],
        scale_y: placement.scale[1],
        scale_z: placement.scale[2],
        offset_x: offset.0,
        offset_y: offset.1,
    }
}

pub(crate) unsafe extern "C" fn builtin_compute(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    query: *const GnoblinStdLayoutQuery,
    out: *mut GnoblinStdPlacements,
) -> i32 {
    call("builtin_compute", || {
        let layout = layout_of(ctx, handle)?;
        // SAFETY: the caller guarantees `query` is a query and its children are readable.
        let (cx, _) = unsafe { read_query(query) }?;
        let placements = layout.compute(&cx)?;
        let lent = LENT_PLACEMENTS.with(|slot| {
            let mut slot = slot.borrow_mut();
            *slot = placements.iter().map(placement_out).collect();
            (slot.as_ptr(), slot.len())
        });
        // SAFETY: the caller guarantees `out` is null or a writable `GnoblinStdPlacements`.
        unsafe {
            put(
                out,
                GnoblinStdPlacements {
                    ptr: lent.0,
                    len: lent.1,
                },
            )
        }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_act(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    action: *const u8,
    len: usize,
    values: *const GnoblinStdOption,
    n_values: usize,
    query: *const GnoblinStdLayoutQuery,
    focus: *mut GnoblinNode,
    out: *mut GnoblinStdOption,
) -> i32 {
    call("builtin_act", || {
        let layout = layout_of(ctx, handle)?;
        let builtin = builtin(&layout);
        let text = abi::GnoblinStr { ptr: action, len };
        // SAFETY: the caller guarantees `len` readable bytes at `action`.
        let name = unsafe { read_text(&text) }?;
        let desc = builtin
            .kind
            .action(name)
            .ok_or_else(|| Fail::no_such(format!("the {} layout has no action `{name}`", builtin.kind.name)))?;
        // SAFETY: the caller guarantees `values` points at `n_values` records.
        let args = unsafe { read_args(builtin.kind, desc, values, n_values) }?;
        // SAFETY: the caller guarantees `query` is a query and its children are readable.
        let (_, env) = unsafe { read_query(query) }?;
        let outcome = builtin.act(name, &args, &env)?;
        if !focus.is_null() {
            // SAFETY: `focus` was checked for null and the caller guarantees it is writable.
            unsafe { focus.write(outcome.focus.unwrap_or(0)) };
        }
        let record = encode_value(&outcome.value, desc.returns.as_ref());
        // SAFETY: the caller guarantees `out` is a writable record whose first field is its size.
        unsafe { fill_out(out, &record, "action answer") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_client_removed(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    query: *const GnoblinStdLayoutQuery,
    client: GnoblinNode,
    focus: *mut GnoblinNode,
) -> i32 {
    call("builtin_client_removed", || {
        let layout = layout_of(ctx, handle)?;
        // SAFETY: the caller guarantees `query` is a query and its children are readable.
        let (cx, _) = unsafe { read_query(query) }?;
        let next = layout.client_removed(&cx, client);
        // SAFETY: the caller guarantees `focus` is null or writable.
        if let Some(focus) = unsafe { focus.as_mut() } {
            *focus = next.unwrap_or(0);
        }
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_clients_swapped(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    a: GnoblinNode,
    b: GnoblinNode,
) -> i32 {
    call("builtin_clients_swapped", || {
        layout_of(ctx, handle)?.clients_swapped(a, b);
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_find_split(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    client: GnoblinNode,
    x: f64,
    y: f64,
    out: *mut GnoblinStdSplit,
) -> i32 {
    call("builtin_find_split", || {
        if !x.is_finite() || !y.is_finite() {
            return Err(Fail::type_error("a drag's point must be two finite numbers"));
        }
        match layout_of(ctx, handle)?.find_split(client, x, y) {
            Some(split) => {
                // SAFETY: the caller guarantees `out` is null or a `GnoblinStdSplit` whose first field is its size.
                unsafe { fill_out(out, &split, "split") }?;
                Ok(1)
            }
            None => Ok(0),
        }
    })
}

pub(crate) unsafe extern "C" fn builtin_set_split(
    ctx: GnoblinCtx,
    handle: GnoblinStdBuiltin,
    token: u64,
    ratio: f64,
    cross_ratio: f64,
) -> i32 {
    call("builtin_set_split", || {
        if !ratio.is_finite() || cross_ratio.is_infinite() {
            return Err(Fail::type_error("a divider's ratio must be a finite number"));
        }
        layout_of(ctx, handle)?.set_split(token, ratio, cross_ratio);
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn builtin_reset_split(ctx: GnoblinCtx, handle: GnoblinStdBuiltin, token: u64) -> i32 {
    call("builtin_reset_split", || {
        layout_of(ctx, handle)?.reset_split(token);
        Ok(abi::E_OK)
    })
}
