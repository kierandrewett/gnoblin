//! The table functions for built-in layouts: choosing a kind, changing its options, running its actions, and the pure ring.
//!
//! Every value is checked against `gnoblin_std_abi::layouts::LAYOUT_KINDS` before a layout sees it (`builtin/wire.rs`). A function
//! that changes the layout marks the workspace dirty, so the next turn computes it again.
#![allow(unsafe_code)]

use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_object::ErrorCode;
use gnoblin_std_abi::layouts::{LAYOUT_KINDS, layout_kind};
use gnoblin_std_abi::{
    GnoblinStdOption, GnoblinStdRect, GnoblinStdRingSlot, GnoblinStdWorkspace, STD_BUILTIN_CAROUSEL,
};

use crate::builtin::{
    ActionEnv, Builtin, RingShape, Val, encode_value, read_args, read_one, read_options, read_text, ring_slots,
};
use crate::error::{Fail, Res};
use crate::floating::grow;
use crate::layout::LayoutObj;
use crate::nodes;
use crate::table::{call, fill_out, mutate, put, read, replace_layout};

/// The built-in layout of `workspace`, or why it has none.
fn builtin_of(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> Res<Rc<LayoutObj>> {
    let layout = read(ctx, |st| st.workspace(workspace).map(|ws| Rc::clone(&ws.layout)))?;
    if layout.as_builtin().is_some() {
        Ok(layout)
    } else {
        Err(Fail::refused(
            ErrorCode::Unsupported,
            "the workspace's layout is not a built-in one: it has no options and no actions",
        ))
    }
}

fn builtin(layout: &LayoutObj) -> &Builtin {
    layout
        .as_builtin()
        .unwrap_or_else(|| unreachable!("`builtin_of` checked the kind"))
}

pub(crate) unsafe extern "C" fn workspace_set_layout_kind(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    kind: u32,
    options: *const GnoblinStdOption,
    n_options: usize,
) -> i32 {
    call("workspace_set_layout_kind", || {
        read(ctx, |st| st.workspace(workspace).map(|_| ()))?;
        let desc = layout_kind(kind).ok_or_else(|| {
            Fail::type_error(format!(
                "unknown built-in layout kind {kind}: the kinds are 1 to {}",
                LAYOUT_KINDS.len()
            ))
        })?;
        // SAFETY: the caller guarantees `options` points at `n_options` records.
        let options = unsafe { read_options(desc, options, n_options) }?;
        replace_layout(ctx, workspace, LayoutObj::builtin(Builtin::new(desc, options)))
    })
}

pub(crate) unsafe extern "C" fn workspace_layout_set(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    option: *const GnoblinStdOption,
) -> i32 {
    call("workspace_layout_set", || {
        let layout = builtin_of(ctx, workspace)?;
        let builtin = builtin(&layout);
        // SAFETY: the caller guarantees `option` is a record.
        let (desc, value) = unsafe { read_one(builtin.kind, option) }?;
        builtin.set_option(desc.name, value);
        mutate(ctx, |st| {
            st.workspace_mut(workspace)?.dirty = true;
            Ok(abi::E_OK)
        })
    })
}

pub(crate) unsafe extern "C" fn workspace_layout_get(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    name: *const u8,
    len: usize,
    out: *mut GnoblinStdOption,
) -> i32 {
    call("workspace_layout_get", || {
        let layout = builtin_of(ctx, workspace)?;
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

/// What an action needs to know about `workspace`.
fn action_env(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> Res<ActionEnv> {
    read(ctx, |st| {
        let clients = st.workspace(workspace)?.clients.clone();
        let focused = clients
            .iter()
            .copied()
            .find(|client| nodes::flag(&st.g, *client, "focused"));
        let focused_size = focused.and_then(|client| {
            let own = nodes::rect_of(&st.g, client);
            if !(own.width > 0.0 && own.height > 0.0) {
                return None;
            }
            let outer = grow(&Rect::new(0.0, 0.0, own.width, own.height), &st.inset_of(client));
            Some((outer.width, outer.height))
        });
        Ok(ActionEnv {
            clients,
            focused,
            focused_size,
        })
    })
}

pub(crate) unsafe extern "C" fn workspace_layout_act(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    action: *const u8,
    len: usize,
    args: *const GnoblinStdOption,
    n_args: usize,
    out: *mut GnoblinStdOption,
) -> i32 {
    call("workspace_layout_act", || {
        let layout = builtin_of(ctx, workspace)?;
        let builtin = builtin(&layout);
        let text = abi::GnoblinStr { ptr: action, len };
        // SAFETY: the caller guarantees `len` readable bytes at `action`.
        let name = unsafe { read_text(&text) }?;
        let desc = builtin
            .kind
            .action(name)
            .ok_or_else(|| Fail::no_such(format!("the {} layout has no action `{name}`", builtin.kind.name)))?;
        // SAFETY: the caller guarantees `args` points at `n_args` records.
        let args: Vec<Val> = unsafe { read_args(builtin.kind, desc, args, n_args) }?;
        let env = action_env(ctx, workspace)?;
        // The layout runs outside the library's borrow of its state.
        let outcome = builtin.act(name, &args, &env)?;
        mutate(ctx, |st| {
            if outcome.changed {
                st.workspace_mut(workspace)?.dirty = true;
            }
            if let Some(client) = outcome.focus {
                let _ = st.g.window_handle(gnoblin_core::NodeId(client)).focus();
            }
            Ok(())
        })?;
        let record = encode_value(&outcome.value, desc.returns.as_ref());
        // SAFETY: the caller guarantees `out` is a writable record whose first field is its size.
        unsafe { fill_out(out, &record, "action answer") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn workspace_layout_kind(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    out: *mut u32,
) -> i32 {
    call("workspace_layout_kind", || {
        let layout = read(ctx, |st| st.workspace(workspace).map(|ws| Rc::clone(&ws.layout)))?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, layout.as_builtin().map_or(0, |builtin| builtin.kind.id)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn layout_ring(
    _ctx: GnoblinCtx,
    count: usize,
    focus: usize,
    area: *const GnoblinStdRect,
    options: *const GnoblinStdOption,
    n_options: usize,
    out: *mut GnoblinStdRingSlot,
) -> i32 {
    call("layout_ring", || {
        // SAFETY: the caller guarantees `area` is null or a rectangle.
        let area = unsafe { area.as_ref() }.ok_or_else(|| Fail::err("layout_ring: a null area"))?;
        let kind = layout_kind(STD_BUILTIN_CAROUSEL).unwrap_or_else(|| unreachable!("the carousel is declared"));
        // SAFETY: the caller guarantees `options` points at `n_options` records.
        let options = unsafe { read_options(kind, options, n_options) }?;
        let slots = ring_slots(
            count,
            focus,
            Rect::new(area.x, area.y, area.width, area.height),
            RingShape::read(&options),
        )?;
        if !slots.is_empty() && out.is_null() {
            return Err(Fail::err("layout_ring: a null output pointer"));
        }
        for (index, slot) in slots.iter().enumerate() {
            let raw = GnoblinStdRingSlot {
                rect: GnoblinStdRect {
                    x: slot.rect.x,
                    y: slot.rect.y,
                    width: slot.rect.width,
                    height: slot.rect.height,
                },
                opacity: slot.opacity,
                rotation: slot.rotation,
                z: slot.z,
            };
            // SAFETY: `index < count`, and the caller guarantees room for `count` slots at `out`.
            unsafe { out.add(index).write(raw) };
        }
        Ok(abi::E_OK)
    })
}
