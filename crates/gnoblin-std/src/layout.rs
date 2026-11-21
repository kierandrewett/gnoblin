//! Layouts: what decides where a workspace's windows go.
//!
//! A workspace calls its layout when the workspace changes: a window joins or leaves, the gaps or the area change, the layout's own
//! options change, the keyboard moves. The library does the calling (it is the one place the dirty-driven pass lives, so every
//! language's layouts behave alike); a layout answers with where each tiled window goes.
//!
//! Two kinds exist. [`FloatingLayout`] is the library's own: a window is placed once when it joins and is left where the user puts it.
//! Anything that implements [`Layout`] is a layout of the shell's: the SDK makes an object of the interface `layout` whose handlers call
//! the trait object (the same interface a scripting runtime fills with closures), so the library's pass cannot tell them apart.
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

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_object as go;

use crate::api::Std;
use crate::bindings::Point;
use crate::builtin::BuiltinLayout;
use crate::client::Client;
use crate::error::Result;
use crate::objects as classes;
use crate::split::Split;
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

    pub(crate) fn place_mode(&self) -> classes::PlaceMode {
        match self.placement {
            FloatPlacement::Smart => classes::PlaceMode::Smart,
            FloatPlacement::Center => classes::PlaceMode::Center,
            FloatPlacement::Cascade => classes::PlaceMode::Cascade,
        }
    }

    pub(crate) fn cascade_step_value(&self) -> f64 {
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

    /// The divider that a drag which pressed `client` at `point` would move, or `None` when the layout has none there. A layout with
    /// dividers answers from the geometry of its last compute; the [`SplitResizeBinding`](crate::SplitResizeBinding) drives what it
    /// returns.
    fn find_split(&mut self, _client: &Client, _point: Point) -> Option<Split> {
        None
    }

    /// Moves the dividers of the drag that [`find_split`](Layout::find_split) named `token`: the first to `ratio` and, when the split
    /// has a second divider, the second to `cross_ratio`. The layout holds each to its range and changes
    /// [`revision`](Layout::revision), so the workspace computes again.
    fn set_split(&mut self, _token: u64, _ratio: f64, _cross_ratio: Option<f64>) {}

    /// Puts the dividers of the drag named `token` back as they were when `find_split` returned them: the drag was cancelled.
    fn reset_split(&mut self, _token: u64) {}
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
    /// One of the library's own tiling layouts, by kind and options.
    Builtin(BuiltinLayout),
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

// ---- the classes ----------------------------------------------------------------------------------------------------------------

fn rect_of(rect: &classes::LayoutRect) -> Rect {
    Rect::new(rect.x, rect.y, rect.width, rect.height)
}

fn rect_out(rect: Rect) -> classes::LayoutRect {
    classes::LayoutRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

/// The client a text id names.
fn client_of(std: &Std, id: &str) -> go::Res<Client> {
    id.parse()
        .ok()
        .and_then(|raw| Client::from_raw_id(std, raw))
        .ok_or_else(|| go::Fail::type_error(format!("`{id}` is not a client's id")))
}

/// The context the library passed, as the shell's type.
fn context(std: &Std, cx: &classes::LayoutContext) -> go::Res<LayoutContext> {
    Ok(LayoutContext {
        workspace: Workspace::of_id(std, &cx.workspace).map_err(|error| go::Fail::err(error.to_string()))?,
        children: cx
            .children
            .iter()
            .map(|id| client_of(std, id))
            .collect::<go::Res<Vec<_>>>()?,
        area: rect_of(&cx.area),
        gap_inner: cx.inner_gap,
        gap_outer: cx.outer_gap,
        focused: cx.focused.as_deref().map(|id| client_of(std, id)).transpose()?,
    })
}

/// A placement as the class's record.
fn placement_out(placement: &Placement) -> classes::LayoutPlacement {
    classes::LayoutPlacement {
        node: placement.client.window().id().0.to_string(),
        bounds: placement.bounds.map(rect_out),
        visible: placement.visible,
        tiled: placement.tiled,
        z: placement.z,
        rotation: Some(placement.rotation.to_vec()),
        scale: Some(placement.scale.to_vec()),
        offset_x: placement.offset.map(|(x, _)| x),
        offset_y: placement.offset.map(|(_, y)| y),
    }
}

/// Runs a hook of the shell's layout. A panic in it is the library's error, not the process's.
fn guarded<R>(work: impl FnOnce() -> R) -> go::Res<R> {
    catch_unwind(AssertUnwindSafe(work)).map_err(|_| go::Fail::err("the layout panicked"))
}

/// The hooks of a layout of the shell's, as the class's handlers. The hooks the trait has a default for are given always, as the
/// library's pass calls them whether or not the shell defined them; `client_removed` only when the layout hands the keyboard on itself.
fn handlers(std: &Std, layout: Box<dyn Layout>, removal: bool) -> classes::LayoutHandlers {
    let shared = Rc::new(RefCell::new(layout));
    let compute = {
        let (layout, std) = (Rc::clone(&shared), std.clone());
        Box::new(move |cx: classes::LayoutContext| {
            let context = context(&std, &cx)?;
            let placements = guarded(|| layout.borrow_mut().compute(&context))?;
            Ok(placements.iter().map(placement_out).collect())
        })
    };
    let client_added = {
        let (layout, std) = (Rc::clone(&shared), std.clone());
        Box::new(move |cx: classes::LayoutContext, id: String| {
            let (context, client) = (context(&std, &cx)?, client_of(&std, &id)?);
            guarded(|| layout.borrow_mut().client_added(&client, &context))
        }) as Box<dyn FnMut(classes::LayoutContext, String) -> go::Res<()>>
    };
    let client_removed = removal.then(|| {
        let (layout, std) = (Rc::clone(&shared), std.clone());
        Box::new(move |cx: classes::LayoutContext, id: String| {
            let (context, client) = (context(&std, &cx)?, client_of(&std, &id)?);
            guarded(|| layout.borrow_mut().client_removed(&client, &context))
        }) as Box<dyn FnMut(classes::LayoutContext, String) -> go::Res<()>>
    });
    let area_changed = {
        let layout = Rc::clone(&shared);
        Box::new(move |old: classes::LayoutRect, new: classes::LayoutRect| {
            guarded(|| layout.borrow_mut().area_changed(rect_of(&old), rect_of(&new)))
        })
    };
    let clients_swapped = {
        let (layout, std) = (Rc::clone(&shared), std.clone());
        Box::new(move |a: String, b: String| {
            let (a, b) = (client_of(&std, &a)?, client_of(&std, &b)?);
            guarded(|| layout.borrow_mut().clients_swapped(&a, &b))
        })
    };
    let revision = {
        let layout = Rc::clone(&shared);
        Box::new(move || guarded(|| layout.borrow().revision()))
    };
    let find_split = {
        let (layout, std) = (Rc::clone(&shared), std.clone());
        Box::new(move |id: String, x: f64, y: f64| {
            let client = client_of(&std, &id)?;
            let found = guarded(|| layout.borrow_mut().find_split(&client, Point { x, y }))?;
            Ok(found.map(|split| split.to_class()).into_iter().collect())
        })
    };
    let move_split = {
        let layout = Rc::clone(&shared);
        Box::new(move |token: u64, ratio: f64, cross: Option<f64>| {
            guarded(|| layout.borrow_mut().set_split(token, ratio, cross))
        })
    };
    let reset_split = {
        let layout = shared;
        Box::new(move |token: u64| guarded(|| layout.borrow_mut().reset_split(token)))
    };
    classes::LayoutHandlers {
        compute,
        client_added: Some(client_added),
        client_removed,
        area_changed: Some(area_changed),
        clients_swapped: Some(clients_swapped),
        revision: Some(revision),
        find_split: Some(find_split),
        move_split: Some(move_split),
        reset_split: Some(reset_split),
    }
}

/// A layout as the object a workspace takes.
pub(crate) fn make(std: &Std, spec: LayoutSpec) -> Result<classes::Layout> {
    let objects = std.objects()?;
    match spec {
        LayoutSpec::Floating(floating) => {
            let layout = classes::FloatingLayout::new(&objects)?;
            layout.set_placement(floating.place_mode())?;
            layout.set_cascade_step(floating.cascade_step_value())?;
            // The layout value takes over the reference the class object holds.
            let taken =
                <classes::Layout as go::typed::FromVal<classes::Objects>>::from_val(&objects, &layout.handle().val())?;
            std::mem::forget(layout);
            Ok(taken)
        }
        LayoutSpec::Builtin(builtin) => builtin.make(&objects),
        LayoutSpec::Foreign { layout, removal } => Ok(handlers(std, layout, removal).implement(&objects)?),
    }
}
