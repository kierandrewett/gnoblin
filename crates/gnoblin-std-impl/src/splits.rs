//! The dividers of a workspace's layout: what a split-resize drag moves.
//!
//! A layout with dividers (a dwindle's splits) answers `find_split` with the divider that a drag which pressed a client at a point
//! would move, as numbers (`GnoblinStdSplit`), and moves it again when told a new ratio. The library only forwards: it finds the
//! layout of the client's workspace, asks it, and makes the workspace compute again after a move. Which divider is nearest, and how
//! a ratio is held to its range, are the layout's.
#![allow(unsafe_code)]

use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinNode};
use gnoblin_std_abi::{GnoblinStdSplit, GnoblinStdWorkspace};

use crate::error::{Fail, Res};
use crate::layout::LayoutObj;
use crate::table::{call, fill_out, mutate, read};

/// The layout of `workspace`.
fn layout_of(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> Res<Rc<LayoutObj>> {
    read(ctx, |st| st.workspace(workspace).map(|ws| Rc::clone(&ws.layout)))
}

/// Tells the workspace that its layout changed what it computes.
fn compute_again(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> Res<i32> {
    mutate(ctx, |st| {
        st.workspace_mut(workspace)?.dirty = true;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn workspace_find_split(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    client: GnoblinNode,
    x: f64,
    y: f64,
    out: *mut GnoblinStdSplit,
) -> i32 {
    call("workspace_find_split", || {
        if !x.is_finite() || !y.is_finite() {
            return Err(Fail::type_error("a drag's point must be two finite numbers"));
        }
        // The layout answers outside the library's borrow, so it may read the SDK while it does.
        let layout = layout_of(ctx, workspace)?;
        match layout.find_split(client, x, y) {
            Some(split) => {
                // SAFETY: the caller guarantees `out` is null or a `GnoblinStdSplit` whose first field is its size.
                unsafe { fill_out(out, &split, "split") }?;
                Ok(1)
            }
            None => Ok(0),
        }
    })
}

pub(crate) unsafe extern "C" fn workspace_set_split(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    token: u64,
    ratio: f64,
    cross_ratio: f64,
) -> i32 {
    call("workspace_set_split", || {
        if !ratio.is_finite() || cross_ratio.is_infinite() {
            return Err(Fail::type_error("a divider's ratio must be a finite number"));
        }
        layout_of(ctx, workspace)?.set_split(token, ratio, cross_ratio);
        compute_again(ctx, workspace)
    })
}

pub(crate) unsafe extern "C" fn workspace_reset_split(
    ctx: GnoblinCtx,
    workspace: GnoblinStdWorkspace,
    token: u64,
) -> i32 {
    call("workspace_reset_split", || {
        layout_of(ctx, workspace)?.reset_split(token);
        compute_again(ctx, workspace)
    })
}
