//! Layouts: what decides where a workspace's windows go.
//!
//! A workspace has one layout and calls it when the workspace changes (`turn.rs` is the driver, after the TypeScript
//! `layout_driver.ts`). Two kinds exist today:
//!
//! - **Floating**, the library's own: a window is placed once when it joins the workspace and is left where the user puts it.
//!   `compute` places nothing; `client_added` does the work (`floating.rs`).
//! - **Foreign**, a layout the shell implements: a table of C functions and a `user` pointer (`GnoblinStdLayoutVtable`). This is the
//!   path every language's custom layout takes. The Rust SDK adapts its `Layout` trait to it, and the scripting runtimes will do
//!   the same with a closure; the library cannot tell them apart.
//!
//! The library calls a foreign layout only from the layout pass, outside any borrow of its state, so a layout may call back into
//! the library (to ask for another pass, say) without a conflict.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core::{Gnoblin, Rect};
use gnoblin_core_abi::{self as abi, GnoblinDropFn};
use gnoblin_std_abi::{
    GnoblinStdLayoutContext, GnoblinStdLayoutVtable, GnoblinStdPlacement, GnoblinStdRect, STD_PLACEMENT_BOUNDS,
    STD_PLACEMENT_HIDDEN, STD_PLACEMENT_OFFSET, STD_PLACEMENT_TILED,
};

use crate::error::{Fail, Res};
use crate::floating::{DEFAULT_HEIGHT, DEFAULT_WIDTH, Inset, PlaceMode, apart_from, grow, place_rect, shrink};
use crate::nodes;
use crate::state::{Id, NodeId};

/// Where one client goes, as a layout said it and the library checked it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Placement {
    pub(crate) node: NodeId,
    /// `None` leaves the client where it is.
    pub(crate) bounds: Option<Rect>,
    pub(crate) visible: bool,
    pub(crate) tiled: bool,
    pub(crate) z: f64,
    pub(crate) rotation: [f64; 3],
    pub(crate) scale: [f64; 3],
    /// How far the rectangle moves (or, without bounds, the client's own size centred in the area).
    pub(crate) offset: Option<(f64, f64)>,
}

/// What a layout is told: a copy, so a layout can keep it and nothing it does reaches the workspace.
#[derive(Clone, Debug)]
pub(crate) struct LayoutContext {
    pub(crate) workspace: Id,
    pub(crate) children: Vec<NodeId>,
    pub(crate) area: Rect,
    pub(crate) gap_inner: f64,
    pub(crate) gap_outer: f64,
    pub(crate) focused: Option<NodeId>,
}

/// A layout the shell implements: the table of its functions and the closure behind them, freed exactly once.
pub(crate) struct Foreign {
    vtable: GnoblinStdLayoutVtable,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
}

/// What kind of layout a workspace has.
pub(crate) enum Kind {
    Floating {
        mode: PlaceMode,
        step: f64,
        /// The clients this layout placed already.
        placed: RefCell<HashSet<NodeId>>,
    },
    Foreign(Foreign),
}

/// A workspace's layout.
pub(crate) struct LayoutObj {
    pub(crate) kind: Kind,
}

impl Foreign {
    /// A foreign layout over a table the shell passed and the library has copied. The layout owns `user` from here on.
    pub(crate) fn new(vtable: GnoblinStdLayoutVtable, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> Foreign {
        Foreign {
            vtable,
            user,
            drop_user,
        }
    }

    fn raw_context(cx: &LayoutContext, sink: *mut c_void) -> (GnoblinStdLayoutContext, &[NodeId]) {
        let raw = GnoblinStdLayoutContext {
            size: size_of::<GnoblinStdLayoutContext>() as u32,
            reserved: 0,
            workspace: cx.workspace,
            children: cx.children.as_ptr(),
            n_children: cx.children.len(),
            area: rect_out(&cx.area),
            gap_inner: cx.gap_inner,
            gap_outer: cx.gap_outer,
            focused: cx.focused.unwrap_or(0),
            sink,
            emit,
        };
        (raw, &cx.children)
    }

    fn compute(&self, cx: &LayoutContext) -> Res<Vec<Placement>> {
        let mut sink = Sink {
            children: cx.children.iter().copied().collect(),
            placements: Vec::new(),
            error: None,
        };
        let (raw, _keep) = Foreign::raw_context(cx, (&raw mut sink).cast());
        let compute = self.vtable.compute;
        // SAFETY: `compute` and `user` are what the shell gave `workspace_set_layout`; `user` is live until `Drop`. `raw` and the
        // children it points at outlive the call, and `sink` is the one `emit` writes through, live and unaliased for the call.
        let outcome = catch_unwind(AssertUnwindSafe(|| unsafe { compute(self.user, &raw) }));
        match outcome {
            Err(_) => Err(Fail::new(abi::E_PANIC, "the layout's compute panicked")),
            Ok(code) if code != abi::E_OK => Err(Fail::new(
                code,
                sink.error
                    .unwrap_or_else(|| format!("the layout's compute failed with code {code}")),
            )),
            Ok(_) => match sink.error {
                Some(error) => Err(Fail::type_error(error)),
                None => Ok(sink.placements),
            },
        }
    }

    fn hook_client(&self, which: Option<gnoblin_std_abi::GnoblinStdClientHookFn>, cx: &LayoutContext, client: NodeId) {
        let Some(hook) = which else {
            return;
        };
        let (raw, _keep) = Foreign::raw_context(cx, core::ptr::null_mut());
        // SAFETY: as for `compute`; the context's `sink` is null, which a hook never uses (`emit` is for `compute` only).
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe { hook(self.user, &raw, client) }));
    }
}

impl Drop for Foreign {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: the shell gave `user` and `drop_user` together; this is the one call, made when the layout goes away.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop_user(self.user) }));
        }
    }
}

fn rect_out(rect: &Rect) -> GnoblinStdRect {
    GnoblinStdRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn rect_in(rect: &GnoblinStdRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

/// What `emit` fills during one `compute`.
struct Sink {
    children: HashSet<NodeId>,
    placements: Vec<Placement>,
    error: Option<String>,
}

/// The sink a layout's `compute` hands its placements to.
unsafe extern "C" fn emit(sink: *mut c_void, placement: *const GnoblinStdPlacement) -> i32 {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `sink` is the `Sink` `compute` made for this call and `placement` is the shell's own struct, valid for the call;
        // both are null-checked by `as_mut` and `as_ref`.
        let (Some(sink), Some(placement)) = (unsafe { sink.cast::<Sink>().as_mut() }, unsafe { placement.as_ref() })
        else {
            return abi::E_ERR;
        };
        // A struct the caller built against an older layout is shorter: the rest reads as its defaults.
        let known = size_of::<GnoblinStdPlacement>();
        let given = placement.size as usize;
        let value: GnoblinStdPlacement = if given >= known {
            *placement
        } else {
            let mut zeroed = core::mem::MaybeUninit::<GnoblinStdPlacement>::zeroed();
            // SAFETY: `given` bytes of the caller's struct are readable (its own `size`), `zeroed` has room for `known > given`
            // bytes, and every field of the struct is an integer or a float, for which all-zero is valid.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    std::ptr::from_ref(placement).cast::<u8>(),
                    zeroed.as_mut_ptr().cast::<u8>(),
                    given,
                );
                zeroed.assume_init()
            }
        };
        match checked(sink, &value) {
            Ok(placed) => {
                sink.placements.push(placed);
                abi::E_OK
            }
            Err(message) => {
                sink.error.get_or_insert(message);
                abi::E_TYPE
            }
        }
    }));
    outcome.unwrap_or(abi::E_PANIC)
}

fn checked(sink: &Sink, raw: &GnoblinStdPlacement) -> Result<Placement, String> {
    if !sink.children.contains(&raw.node) {
        return Err(format!(
            "placement for node {}, which is not one of the workspace's clients",
            raw.node
        ));
    }
    let numbers = [
        raw.z,
        raw.rotation_x,
        raw.rotation_y,
        raw.rotation_z,
        raw.scale_x,
        raw.scale_y,
        raw.scale_z,
        raw.offset_x,
        raw.offset_y,
    ];
    if numbers.iter().any(|n| !n.is_finite()) {
        return Err(format!(
            "placement for node {}: z, rotation, scale and offset must be finite numbers",
            raw.node
        ));
    }
    let bounds = if raw.flags & STD_PLACEMENT_BOUNDS != 0 {
        let b = &raw.bounds;
        if !(b.x.is_finite() && b.y.is_finite() && b.width.is_finite() && b.height.is_finite()) {
            return Err(format!(
                "placement for node {}: bounds must be finite numbers",
                raw.node
            ));
        }
        if b.width < 0.0 || b.height < 0.0 {
            return Err(format!(
                "placement for node {}: bounds must have a width and a height of at least 0",
                raw.node
            ));
        }
        Some(rect_in(b))
    } else {
        None
    };
    // A scale of zero in the struct reads as the default 1.
    let scale = |value: f64| if value == 0.0 { 1.0 } else { value };
    Ok(Placement {
        node: raw.node,
        bounds,
        visible: raw.flags & STD_PLACEMENT_HIDDEN == 0,
        tiled: raw.flags & STD_PLACEMENT_TILED != 0,
        z: raw.z,
        rotation: [raw.rotation_x, raw.rotation_y, raw.rotation_z],
        scale: [scale(raw.scale_x), scale(raw.scale_y), scale(raw.scale_z)],
        offset: (raw.flags & STD_PLACEMENT_OFFSET != 0).then_some((raw.offset_x, raw.offset_y)),
    })
}

impl LayoutObj {
    /// The library's floating layout.
    pub(crate) fn floating(mode: PlaceMode, step: f64) -> Rc<LayoutObj> {
        Rc::new(LayoutObj {
            kind: Kind::Floating {
                mode,
                step,
                placed: RefCell::new(HashSet::new()),
            },
        })
    }

    /// A layout the shell implements.
    pub(crate) fn foreign(foreign: Foreign) -> Rc<LayoutObj> {
        Rc::new(LayoutObj {
            kind: Kind::Foreign(foreign),
        })
    }

    /// A number that changes when one of the layout's own options does: the workspace computes again.
    pub(crate) fn revision(&self) -> u64 {
        match &self.kind {
            Kind::Floating { mode, step, .. } => ((*mode as u64) << 56) ^ step.to_bits(),
            Kind::Foreign(foreign) => match foreign.vtable.revision {
                // SAFETY: `revision` and `user` are the shell's, live for the layout's life.
                Some(revision) => catch_unwind(AssertUnwindSafe(|| unsafe { revision(foreign.user) })).unwrap_or(0),
                None => 0,
            },
        }
    }

    /// Whether the layout has a hook for a client leaving, and so hands the keyboard on itself (a strip's columns are not in window
    /// order); without one the library gives the keyboard to the neighbour of a closed window.
    pub(crate) fn handles_removal(&self) -> bool {
        match &self.kind {
            Kind::Floating { .. } => false,
            Kind::Foreign(foreign) => foreign.vtable.client_removed.is_some(),
        }
    }

    /// Where the tiled clients go. A floating layout places nothing here.
    pub(crate) fn compute(&self, cx: &LayoutContext) -> Res<Vec<Placement>> {
        match &self.kind {
            Kind::Floating { .. } => Ok(Vec::new()),
            Kind::Foreign(foreign) => foreign.compute(cx),
        }
    }

    /// A client joined the workspace.
    pub(crate) fn client_added(&self, g: &Gnoblin, cx: &LayoutContext, client: NodeId) {
        match &self.kind {
            Kind::Floating { mode, step, placed } => place_floating(g, cx, client, *mode, *step, placed),
            Kind::Foreign(foreign) => foreign.hook_client(foreign.vtable.client_added, cx, client),
        }
    }

    /// A client left the workspace.
    pub(crate) fn client_removed(&self, cx: &LayoutContext, client: NodeId) {
        if let Kind::Foreign(foreign) = &self.kind {
            foreign.hook_client(foreign.vtable.client_removed, cx, client);
        }
    }

    /// The workspace's rectangle changed.
    pub(crate) fn area_changed(&self, old: &Rect, new: &Rect) {
        if let Kind::Foreign(foreign) = &self.kind
            && let Some(hook) = foreign.vtable.area_changed
        {
            let (old, new) = (rect_out(old), rect_out(new));
            // SAFETY: `hook` and `user` are the shell's; the two rectangles are live locals.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { hook(foreign.user, &old, &new) }));
        }
    }

    /// Two clients traded places in the workspace's order. A layout that places by order has nothing to do: its next compute reads
    /// the new order.
    pub(crate) fn clients_swapped(&self, a: NodeId, b: NodeId) {
        if let Kind::Foreign(foreign) = &self.kind
            && let Some(hook) = foreign.vtable.clients_swapped
        {
            // SAFETY: `hook` and `user` are the shell's.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { hook(foreign.user, a, b) }));
        }
    }
}

/// The floating layout's `onClientAdded`: places a newly added client once, unless this layout placed it already.
fn place_floating(
    g: &Gnoblin,
    cx: &LayoutContext,
    client: NodeId,
    mode: PlaceMode,
    step: f64,
    placed: &RefCell<HashSet<NodeId>>,
) {
    // A window that came from a tiled layout is floating now, placed or not.
    if nodes::flag(g, client, "tiled") {
        nodes::write_flag(g, client, "tiled", false);
    }
    if placed.borrow().contains(&client) {
        return;
    }
    // Until the shell writes the size, the properties hold what the client committed.
    let own = nodes::rect_of(g, client);
    let width = if own.width > 0.0 { own.width } else { DEFAULT_WIDTH };
    let height = if own.height > 0.0 { own.height } else { DEFAULT_HEIGHT };
    let inset = Inset::default();
    let occupied: Vec<Rect> = cx
        .children
        .iter()
        .filter(|other| **other != client && nodes::visible(g, **other))
        .map(|other| grow(&nodes::rect_of(g, *other), &inset))
        .collect();
    let room = grow(&Rect::new(0.0, 0.0, width, height), &inset);
    let spot = place_rect(mode, room.width, room.height, &cx.area, &occupied, step);
    let frame = apart_from(&spot, &occupied, &cx.area, step);
    nodes::write_rect(g, client, &shrink(&frame, &inset));
    placed.borrow_mut().insert(client);
}
