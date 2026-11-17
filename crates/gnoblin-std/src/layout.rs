//! Layouts: what decides where a workspace's windows go.
//!
//! A workspace calls its layout when the workspace changes: a window joins or leaves, the gaps or the area change, the layout's own
//! options change, the keyboard moves. The library does the calling (it is the one place the dirty-driven pass lives, so every
//! language's layouts behave alike); a layout answers with where each tiled window goes.
//!
//! Two kinds exist. [`FloatingLayout`] is the library's own: a window is placed once when it joins and is left where the user puts it.
//! Anything that implements [`Layout`] is a layout of the shell's: the SDK hands the library a table of C functions that call the
//! trait object (the same table a scripting runtime fills with closures), so the library's pass cannot tell them apart.
//!
//! ```ignore
//! struct Columns;
//! impl Layout for Columns {
//!     fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
//!         let area = cx.inset_area();
//!         let n = cx.children.len().max(1) as f64;
//!         cx.children.iter().enumerate()
//!             .map(|(i, client)| Placement::tile(client, area.x + area.width / n * i as f64, area.y, area.width / n, area.height))
//!             .collect()
//!     }
//! }
//! workspace.set_layout(Columns)?;
//! ```
//!
//! What a layout may do inside [`Layout::compute`] is read the context and answer. It runs outside any lock of the library, so it may
//! call back into the SDK (to ask for another pass with [`Workspace::invalidate`], say).
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core::{NodeId, Rect};
use gnoblin_core_abi as core_abi;
use gnoblin_std_abi::{
    self as abi, GnoblinStdLayoutContext, GnoblinStdLayoutSpec, GnoblinStdLayoutVtable, GnoblinStdPlacement,
    GnoblinStdRect,
};

use crate::api::Std;
use crate::client::Client;
use crate::workspace::Workspace;

/// How a floating layout places a window when it joins the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FloatPlacement {
    /// The first spot in the area that overlaps nothing, else a cascade.
    #[default]
    Smart,
    /// Centred, cascading only when that exact spot is taken.
    Center,
    /// A cascade from the area's top left.
    Cascade,
}

/// The library's floating layout: a window is placed once when it joins the workspace, and the user then moves and resizes it. Pass it
/// to [`Workspace::set_layout`] or the workspace builder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingLayout {
    placement: FloatPlacement,
    cascade_step: f64,
}

impl Default for FloatingLayout {
    fn default() -> FloatingLayout {
        FloatingLayout {
            placement: FloatPlacement::Smart,
            cascade_step: 32.0,
        }
    }
}

impl FloatingLayout {
    /// A floating layout with the default options: smart placement and a cascade step of 32 pixels.
    pub fn new() -> FloatingLayout {
        FloatingLayout::default()
    }

    /// How a new window is placed.
    #[must_use]
    pub fn placement(mut self, placement: FloatPlacement) -> FloatingLayout {
        self.placement = placement;
        self
    }

    /// The distance between cascade positions, in logical pixels (at least 1).
    #[must_use]
    pub fn cascade_step(mut self, step: f64) -> FloatingLayout {
        self.cascade_step = step;
        self
    }

    pub(crate) fn raw_placement(&self) -> u32 {
        match self.placement {
            FloatPlacement::Smart => abi::STD_PLACE_SMART,
            FloatPlacement::Center => abi::STD_PLACE_CENTER,
            FloatPlacement::Cascade => abi::STD_PLACE_CASCADE,
        }
    }

    pub(crate) fn raw_step(&self) -> f64 {
        self.cascade_step
    }
}

/// What a layout is told: a copy, so a layout can keep it and nothing it does reaches the workspace.
#[derive(Clone, Debug)]
pub struct LayoutContext {
    /// The workspace that owns the layout.
    pub workspace: Workspace,
    /// The clients to place, in the workspace's order: the tiled ones (a floating, fullscreen or maximised client, and a dialog, keep
    /// their own place).
    pub children: Vec<Client>,
    /// The workspace's rectangle. A tiling layout insets it by `gap_outer` itself (see [`inset_area`](LayoutContext::inset_area)).
    pub area: Rect,
    /// The workspace's gap between tiles.
    pub gap_inner: f64,
    /// The workspace's gap between the tiles and the edge of the area.
    pub gap_outer: f64,
    /// The child that has the keyboard, if one does.
    pub focused: Option<Client>,
}

impl LayoutContext {
    /// The area shrunk by the outer gap on every side, never below zero: the rectangle a tiling layout fills.
    pub fn inset_area(&self) -> Rect {
        inset_rect(&self.area, self.gap_outer)
    }
}

/// `area` shrunk by `outer` on every side, never below zero.
pub fn inset_rect(area: &Rect, outer: f64) -> Rect {
    Rect::new(
        area.x + outer,
        area.y + outer,
        (area.width - 2.0 * outer).max(0.0),
        (area.height - 2.0 * outer).max(0.0),
    )
}

/// Where one client goes. A client the layout gives no placement is placed with the defaults: not tiled, shown, no depth, no
/// rotation, unit scale.
#[derive(Clone, Debug)]
pub struct Placement {
    /// The client this placement is for: one of the context's children.
    pub client: Client,
    /// The client's rectangle in logical pixels; `None` leaves it where it is.
    pub bounds: Option<Rect>,
    /// `false` hides the client (a monocle's other windows).
    pub visible: bool,
    /// Marks the client as in a tiled layout, so it drops the border and rounding it draws itself.
    pub tiled: bool,
    /// Depth along the viewing axis; negative is in front of the camera.
    pub z: f64,
    /// Rotation about each axis in radians.
    pub rotation: [f64; 3],
    /// Scale along each axis.
    pub scale: [f64; 3],
    /// How far the rectangle moves; without `bounds`, from the client's own size centred in the area.
    pub offset: Option<(f64, f64)>,
}

/// The error a floor of float arithmetic is allowed: the right edge of 619.2 + 404.8 is 1023.9999999999999, and flooring it as it is
/// would lose a pixel the layout meant to give.
const EDGE_EPSILON: f64 = 1e-6;

impl Placement {
    /// A placement that leaves `client` where it is, shown, with the defaults.
    pub fn new(client: &Client) -> Placement {
        Placement {
            client: client.clone(),
            bounds: None,
            visible: true,
            tiled: false,
            z: 0.0,
            rotation: [0.0; 3],
            scale: [1.0; 3],
            offset: None,
        }
    }

    /// A tiled placement at whole pixels. Each of the four edges is floored on its own, and the size is the distance between two
    /// floored edges: a master of 607.2 and a stack of 404.8 that starts at 619.2 are 607 and 404 wide when each is floored apart,
    /// and the stack still ends at 1023 in an area that ends at 1024.
    pub fn tile(client: &Client, x: f64, y: f64, width: f64, height: f64) -> Placement {
        let left = (x + EDGE_EPSILON).floor();
        let top = (y + EDGE_EPSILON).floor();
        Placement {
            tiled: true,
            bounds: Some(Rect::new(
                left,
                top,
                (x + width + EDGE_EPSILON).floor() - left,
                (y + height + EDGE_EPSILON).floor() - top,
            )),
            ..Placement::new(client)
        }
    }

    /// The client's rectangle.
    #[must_use]
    pub fn bounds(mut self, rect: Rect) -> Placement {
        self.bounds = Some(rect);
        self
    }

    /// Hides the client.
    #[must_use]
    pub fn hidden(mut self) -> Placement {
        self.visible = false;
        self
    }

    /// Whether the client counts as tiled.
    #[must_use]
    pub fn tiled(mut self, tiled: bool) -> Placement {
        self.tiled = tiled;
        self
    }

    /// The client's depth.
    #[must_use]
    pub fn z(mut self, z: f64) -> Placement {
        self.z = z;
        self
    }

    /// The client's rotation about each axis, in radians.
    #[must_use]
    pub fn rotation(mut self, rotation: [f64; 3]) -> Placement {
        self.rotation = rotation;
        self
    }

    /// The client's scale along each axis.
    #[must_use]
    pub fn scale(mut self, scale: [f64; 3]) -> Placement {
        self.scale = scale;
        self
    }

    /// Moves the client's rectangle by `(x, y)`.
    #[must_use]
    pub fn offset(mut self, x: f64, y: f64) -> Placement {
        self.offset = Some((x, y));
        self
    }
}

/// A layout the shell implements. Only [`compute`](Layout::compute) is required.
///
/// A layout is a value the workspace owns: it keeps whatever it needs between calls (`&mut self`). It is asked to compute when the
/// workspace changed in a way that matters to it; options that change without the library seeing it are announced with
/// [`revision`](Layout::revision) or [`Workspace::invalidate`].
pub trait Layout: 'static {
    /// Where `cx.children` go. Pure: the same context and the same options give the same result. A client left out of the answer is
    /// placed with the defaults (and a window that leaves a tiled layout for a floating one stops being tiled).
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement>;

    /// A client joined the workspace.
    fn client_added(&mut self, _client: &Client, _cx: &LayoutContext) {}

    /// A client left the workspace. A layout that overrides this also hands the keyboard on when a window closes (see
    /// [`hands_focus_on_removal`](Layout::hands_focus_on_removal)).
    fn client_removed(&mut self, _client: &Client, _cx: &LayoutContext) {}

    /// The workspace's area changed.
    fn area_changed(&mut self, _old: Rect, _new: Rect) {}

    /// Two clients traded places in the workspace's order (`Collection::swap`). A layout that places by order has nothing to do: its
    /// next `compute` reads the new order.
    fn clients_swapped(&mut self, _a: &Client, _b: &Client) {}

    /// A number that changes whenever one of the layout's own options does, so the workspace computes again. The library reads it
    /// before every pass. The default never changes: the shell calls [`Workspace::invalidate`] after it changes an option.
    fn revision(&self) -> u64 {
        0
    }

    /// Whether the layout keeps its own record of who sits where (a strip's columns are not in window order) and so hands the
    /// keyboard to the right neighbour itself when a window closes. When it does not, the library hands it to the neighbour in the
    /// workspace's order.
    fn hands_focus_on_removal(&self) -> bool {
        false
    }
}

/// Anything a workspace takes as its layout: [`FloatingLayout`], or any [`Layout`].
pub trait IntoLayout {
    /// The layout as a value the SDK passes to the library.
    #[doc(hidden)]
    fn into_layout(self) -> LayoutSpec;
}

/// A layout, as the SDK hands it to the library.
#[doc(hidden)]
pub enum LayoutSpec {
    /// The library's floating layout.
    Floating(FloatingLayout),
    /// A layout of the shell's.
    Foreign {
        /// The layout.
        layout: Box<dyn Layout>,
        /// Whether it hands the keyboard on itself.
        removal: bool,
    },
}

impl IntoLayout for FloatingLayout {
    fn into_layout(self) -> LayoutSpec {
        LayoutSpec::Floating(self)
    }
}

impl<L: Layout> IntoLayout for L {
    fn into_layout(self) -> LayoutSpec {
        let removal = self.hands_focus_on_removal();
        LayoutSpec::Foreign {
            layout: Box::new(self),
            removal,
        }
    }
}

// ---- the table the library calls ---------------------------------------------------------------------------------------------

/// A layout of the shell's behind the library's `user` pointer.
struct Boxed {
    std: Std,
    layout: RefCell<Box<dyn Layout>>,
}

fn rect_of(raw: &GnoblinStdRect) -> Rect {
    Rect::new(raw.x, raw.y, raw.width, raw.height)
}

/// The context the library passed, as the shell's type.
///
/// # Safety
///
/// `cx` must be a valid pointer to the context of a running `compute` or hook, whose `children` are valid for it.
unsafe fn context(std: &Std, cx: &GnoblinStdLayoutContext) -> LayoutContext {
    let children = if cx.children.is_null() || cx.n_children == 0 {
        &[]
    } else {
        // SAFETY: the library lends `n_children` ids at `children` until the call returns.
        unsafe { std::slice::from_raw_parts(cx.children, cx.n_children) }
    };
    let client = |id: u64| Client::new(std, std.gnoblin().window_handle(NodeId(id)));
    LayoutContext {
        workspace: Workspace::from_handle(std, cx.workspace),
        children: children.iter().map(|id| client(*id)).collect(),
        area: rect_of(&cx.area),
        gap_inner: cx.gap_inner,
        gap_outer: cx.gap_outer,
        focused: (cx.focused != 0).then(|| client(cx.focused)),
    }
}

unsafe extern "C" fn compute_tramp(user: *mut c_void, cx: *const GnoblinStdLayoutContext) -> i32 {
    // SAFETY: `user` is the `Boxed` leaked by `RawLayout`; the library passes it back unchanged on the shell thread and frees it only
    // through `drop_boxed`, after the layout is replaced or the generation retires.
    let boxed = unsafe { &*user.cast::<Boxed>() };
    // SAFETY: the library passes a context valid until this call returns, or null.
    let Some(cx) = (unsafe { cx.as_ref() }) else {
        return core_abi::E_ERR;
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `cx` is the library's context for this call.
        let context = unsafe { context(&boxed.std, cx) };
        let placements = boxed.layout.borrow_mut().compute(&context);
        for placement in &placements {
            let mut flags = 0;
            if placement.bounds.is_some() {
                flags |= abi::STD_PLACEMENT_BOUNDS;
            }
            if !placement.visible {
                flags |= abi::STD_PLACEMENT_HIDDEN;
            }
            if placement.tiled {
                flags |= abi::STD_PLACEMENT_TILED;
            }
            if placement.offset.is_some() {
                flags |= abi::STD_PLACEMENT_OFFSET;
            }
            let bounds = placement.bounds.unwrap_or_default();
            let (offset_x, offset_y) = placement.offset.unwrap_or_default();
            let raw = GnoblinStdPlacement {
                size: size_of::<GnoblinStdPlacement>() as u32,
                flags,
                node: placement.client.window().id().0,
                bounds: GnoblinStdRect {
                    x: bounds.x,
                    y: bounds.y,
                    width: bounds.width,
                    height: bounds.height,
                },
                z: placement.z,
                rotation_x: placement.rotation[0],
                rotation_y: placement.rotation[1],
                rotation_z: placement.rotation[2],
                scale_x: placement.scale[0],
                scale_y: placement.scale[1],
                scale_z: placement.scale[2],
                offset_x,
                offset_y,
            };
            // SAFETY: `emit` and `sink` are the library's for this `compute` call; `raw` is a live local.
            let code = unsafe { (cx.emit)(cx.sink, &raw) };
            if code != core_abi::E_OK {
                return code;
            }
        }
        core_abi::E_OK
    }));
    outcome.unwrap_or(core_abi::E_PANIC)
}

unsafe extern "C" fn client_added_tramp(user: *mut c_void, cx: *const GnoblinStdLayoutContext, client: u64) {
    // SAFETY: as for `compute_tramp`.
    let (Some(boxed), Some(cx)) = (unsafe { user.cast::<Boxed>().as_ref() }, unsafe { cx.as_ref() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `cx` is the library's context for this call.
        let context = unsafe { context(&boxed.std, cx) };
        let client = Client::new(&boxed.std, boxed.std.gnoblin().window_handle(NodeId(client)));
        boxed.layout.borrow_mut().client_added(&client, &context);
    }));
}

unsafe extern "C" fn client_removed_tramp(user: *mut c_void, cx: *const GnoblinStdLayoutContext, client: u64) {
    // SAFETY: as for `compute_tramp`.
    let (Some(boxed), Some(cx)) = (unsafe { user.cast::<Boxed>().as_ref() }, unsafe { cx.as_ref() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `cx` is the library's context for this call.
        let context = unsafe { context(&boxed.std, cx) };
        let client = Client::new(&boxed.std, boxed.std.gnoblin().window_handle(NodeId(client)));
        boxed.layout.borrow_mut().client_removed(&client, &context);
    }));
}

unsafe extern "C" fn area_changed_tramp(user: *mut c_void, old: *const GnoblinStdRect, new: *const GnoblinStdRect) {
    // SAFETY: as for `compute_tramp`; the rectangles are the library's locals for this call.
    let (Some(boxed), Some(old), Some(new)) = (
        unsafe { user.cast::<Boxed>().as_ref() },
        unsafe { old.as_ref() },
        unsafe { new.as_ref() },
    ) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        boxed.layout.borrow_mut().area_changed(rect_of(old), rect_of(new));
    }));
}

unsafe extern "C" fn clients_swapped_tramp(user: *mut c_void, a: u64, b: u64) {
    // SAFETY: as for `compute_tramp`.
    let Some(boxed) = (unsafe { user.cast::<Boxed>().as_ref() }) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let client = |id: u64| Client::new(&boxed.std, boxed.std.gnoblin().window_handle(NodeId(id)));
        boxed.layout.borrow_mut().clients_swapped(&client(a), &client(b));
    }));
}

unsafe extern "C" fn revision_tramp(user: *mut c_void) -> u64 {
    // SAFETY: as for `compute_tramp`.
    let Some(boxed) = (unsafe { user.cast::<Boxed>().as_ref() }) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| boxed.layout.borrow().revision())).unwrap_or(0)
}

/// Frees the layout behind a `user` pointer. The library calls it exactly once, when the layout is replaced or the generation retires.
unsafe extern "C" fn drop_boxed(user: *mut c_void) {
    // SAFETY: `user` is the `Box<Boxed>` leaked by `RawLayout::new` and the library calls this exactly once.
    drop(unsafe { Box::from_raw(user.cast::<Boxed>()) });
}

/// The functions every layout of the shell's has.
const BASE: GnoblinStdLayoutVtable = GnoblinStdLayoutVtable {
    size: size_of::<GnoblinStdLayoutVtable>() as u32,
    reserved: 0,
    compute: compute_tramp,
    client_added: Some(client_added_tramp),
    client_removed: None,
    area_changed: Some(area_changed_tramp),
    clients_swapped: Some(clients_swapped_tramp),
    revision: Some(revision_tramp),
};

/// The table of a layout that does not hand the keyboard on itself.
static VTABLE: GnoblinStdLayoutVtable = BASE;

/// The table of a layout that overrides `client_removed`: the library leaves the keyboard to it.
static VTABLE_REMOVAL: GnoblinStdLayoutVtable = GnoblinStdLayoutVtable {
    client_removed: Some(client_removed_tramp),
    ..BASE
};

/// A layout as the library's table function takes it, with the box behind `user` reclaimed if the library does not adopt it.
pub(crate) struct RawLayout {
    spec: GnoblinStdLayoutSpec,
    owned: Option<*mut Boxed>,
}

impl RawLayout {
    /// Makes the table argument for `layout`.
    pub(crate) fn new(layout: impl IntoLayout, std: &Std) -> RawLayout {
        let mut spec = GnoblinStdLayoutSpec {
            size: size_of::<GnoblinStdLayoutSpec>() as u32,
            kind: abi::STD_LAYOUT_FLOATING,
            placement: abi::STD_PLACE_SMART,
            reserved: 0,
            cascade_step: 0.0,
            vtable: core::ptr::null(),
            user: core::ptr::null_mut(),
            drop_user: None,
        };
        match layout.into_layout() {
            LayoutSpec::Floating(floating) => {
                spec.placement = floating.raw_placement();
                spec.cascade_step = floating.raw_step();
                RawLayout { spec, owned: None }
            }
            LayoutSpec::Foreign { layout, removal } => {
                let boxed = Box::into_raw(Box::new(Boxed {
                    std: std.clone(),
                    layout: RefCell::new(layout),
                }));
                spec.kind = abi::STD_LAYOUT_FOREIGN;
                spec.vtable = if removal {
                    &raw const VTABLE_REMOVAL
                } else {
                    &raw const VTABLE
                };
                spec.user = boxed.cast();
                spec.drop_user = Some(drop_boxed);
                RawLayout {
                    spec,
                    owned: Some(boxed),
                }
            }
        }
    }

    /// The table argument.
    pub(crate) fn spec(&self) -> *const GnoblinStdLayoutSpec {
        &raw const self.spec
    }

    /// The library took the layout: it owns the box now.
    pub(crate) fn adopted(mut self) {
        self.owned = None;
    }
}

impl Drop for RawLayout {
    fn drop(&mut self) {
        if let Some(boxed) = self.owned.take() {
            // SAFETY: the library did not adopt the layout (a call that fails has not taken `user`), so nothing else owns the box.
            drop(unsafe { Box::from_raw(boxed) });
        }
    }
}
