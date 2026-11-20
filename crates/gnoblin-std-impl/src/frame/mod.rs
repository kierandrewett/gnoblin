//! The window frame as native classes of `gnoblin.std`: the frame round one window's content, its outlets, the stock titlebar and its parts,
//! the window's actions and drag, and the decorations that give every window a frame.
//!
//! Its behaviour is held to the recorded fixtures `reference/frame_*.json` (`design/native-std-port-tests.md`). The frame is built over the UI toolkit (`crate::ui`): an outlet and every part is a canvas of the
//! toolkit that carries a [`FramePart`], so it is updated, painted and written to the compositor by the toolkit's turn, and a shell adds its own
//! canvases to an outlet as it adds them to any box. The pieces:
//!
//! - `window_frame`: one frame, its outlets (made when first read, stacked in the window's children), its inset and its geometry;
//! - `outlet`: where an outlet is placed (edge, corner, layer, zone), when it shows and what room it takes;
//! - `decorations`: the service that makes a frame for each window, fills it, and tells the library how much room each frame reserves;
//! - `actions`: close with its fade, minimise, maximise and the seat's click actions;
//! - `drag`: the window behaviour of a drag area;
//! - `titlebar`: the stock titlebar and its style;
//! - `parts`: the app icon, backdrop, border, glow, resize handles, title text and window buttons;
//! - `client`: what a frame reads of its window, and the `frame_client` a closure is given;
//! - `classes`: the catalog's classes (`frame.toml`) implemented over the above.
//!
//! A window is named by its node number. Nothing here runs a shell's closure inside a call of the object protocol: the fill of a frame, a
//! rule's test, a hook and a part's questions run from the toolkit's turn.

mod actions;
mod classes;
mod client;
mod decorations;
mod drag;
mod outlet;
mod parts;
mod titlebar;
mod window_frame;

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use gnoblin_core::Subscription;
use gnoblin_object::{Closure, Obj};
use gnoblin_paint::PaintOp;

use crate::floating::Inset;
use crate::ui::widget::{Cid, Ui};

pub(crate) use classes::register;
pub(crate) use titlebar::TitlebarStyle;

/// The toolkit's own name for a frame: never reused.
pub(crate) type FrameId = u64;

/// The values of the enums of the catalog (`frame.toml`), which are their places in it counted from one.
pub(crate) mod en {
    pub(crate) const KIND_SERVER: u32 = 1;
    pub(crate) const KIND_LAYERS: u32 = 2;
    pub(crate) const SIDE_TOP: u32 = 1;
    pub(crate) const SIDE_RIGHT: u32 = 2;
    pub(crate) const SIDE_BOTTOM: u32 = 3;
    pub(crate) const SIDE_LEFT: u32 = 4;
    pub(crate) const CORNER_TOP_LEFT: u32 = 1;
    pub(crate) const CORNER_TOP_RIGHT: u32 = 2;
    pub(crate) const CORNER_BOTTOM_LEFT: u32 = 3;
    pub(crate) const CORNER_BOTTOM_RIGHT: u32 = 4;
    pub(crate) const BUTTON_MINIMISE: u32 = 1;
    pub(crate) const BUTTON_MAXIMISE: u32 = 2;
    pub(crate) const BUTTON_CLOSE: u32 = 3;
    pub(crate) const RESIZE_TOP_LEFT: u32 = 1;
    pub(crate) const RESIZE_TOP_RIGHT: u32 = 2;
    pub(crate) const RESIZE_BOTTOM_LEFT: u32 = 3;
    pub(crate) const RESIZE_BOTTOM_RIGHT: u32 = 4;
    pub(crate) const RESIZE_LEFT: u32 = 5;
    pub(crate) const RESIZE_RIGHT: u32 = 6;
    pub(crate) const RESIZE_TOP: u32 = 7;
    pub(crate) const RESIZE_BOTTOM: u32 = 8;
    pub(crate) const GESTURE_DOUBLE_CLICK: u32 = 1;
    pub(crate) const GESTURE_MIDDLE_CLICK: u32 = 2;
    pub(crate) const GESTURE_RIGHT_CLICK: u32 = 3;
}

/// Who draws what a frame holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameKind {
    /// A window the shell frames: it has edges and an inset.
    Server,
    /// A window that draws its own frame or has none: an underlay and an overlay, no edge and no inset.
    Layers,
}

impl FrameKind {
    /// The kind the compositor's `decoration` of a window asks for.
    pub(crate) fn of_decoration(decoration: &str) -> FrameKind {
        if decoration == "server" {
            FrameKind::Server
        } else {
            FrameKind::Layers
        }
    }

    /// The value of the catalog's `frame_kind`.
    pub(crate) fn value(self) -> u32 {
        match self {
            FrameKind::Server => en::KIND_SERVER,
            FrameKind::Layers => en::KIND_LAYERS,
        }
    }
}

/// The places of a frame that hold an outlet, in the order they are stacked in the window's children, back to front, then the underlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FrameSlot {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Overlay,
    Underlay,
}

impl FrameSlot {
    /// The slots stacked above the underlay, back to front.
    pub(crate) const STACKING: [FrameSlot; 9] = [
        FrameSlot::Left,
        FrameSlot::Right,
        FrameSlot::Top,
        FrameSlot::Bottom,
        FrameSlot::TopLeft,
        FrameSlot::TopRight,
        FrameSlot::BottomLeft,
        FrameSlot::BottomRight,
        FrameSlot::Overlay,
    ];

    /// The slots that hold an edge, with the side each follows.
    pub(crate) const EDGES: [(FrameSlot, u32); 4] = [
        (FrameSlot::Top, en::SIDE_TOP),
        (FrameSlot::Right, en::SIDE_RIGHT),
        (FrameSlot::Bottom, en::SIDE_BOTTOM),
        (FrameSlot::Left, en::SIDE_LEFT),
    ];
}

/// What an outlet of a frame is.
#[derive(Clone, Debug)]
pub(crate) enum Shape {
    /// One zone of an edge: its place is its parent's row or column.
    Zone { edge: Cid },
    /// A strip along one side of the frame.
    Edge(EdgeShape),
    /// A square at one corner.
    Corner { corner: u32, size: Option<f64> },
    /// The underlay or the overlay.
    Layer { over: bool, bleed: f64 },
}

/// What an edge outlet keeps.
#[derive(Clone, Debug)]
pub(crate) struct EdgeShape {
    pub(crate) side: u32,
    /// The thickness the shell gave; `None` measures what was added.
    pub(crate) size: Option<f64>,
    /// How thick the thickest canvas added was when it was added.
    pub(crate) measured: f64,
    /// The start, centre and end zones, once one was read.
    pub(crate) zones: Option<[Cid; 3]>,
    /// The stock titlebar the edge's height follows, when the stock fill put one in it.
    pub(crate) bar: Option<Cid>,
}

/// What an outlet adds to a box.
#[derive(Clone)]
pub(crate) struct OutletPart {
    pub(crate) name: String,
    pub(crate) hidden: bool,
    pub(crate) hide_when: Option<Closure>,
    pub(crate) on_refresh: Option<Closure>,
    /// The nodes of the compositor, not canvases, that were mounted under the outlet.
    pub(crate) nodes: Vec<u64>,
    pub(crate) shape: Shape,
}

/// What a titlebar adds to a drag area.
#[derive(Clone)]
pub(crate) struct BarPart {
    /// The bar's own style; `None` follows the decorations'.
    pub(crate) style: Option<TitlebarStyle>,
    pub(crate) area: Cid,
    pub(crate) title: Cid,
    pub(crate) buttons: Vec<(u32, Cid)>,
}

/// What a window button adds to an icon.
#[derive(Clone)]
pub(crate) struct ButtonPart {
    pub(crate) name: u32,
    pub(crate) restore_icon: Option<String>,
    /// The icon the button had when it was made: what the maximise button shows while the window is not maximised.
    pub(crate) base_icon: String,
}

/// What a row of window buttons adds to a box.
#[derive(Clone)]
pub(crate) struct ButtonsPart {
    pub(crate) names: Vec<u32>,
    pub(crate) icons: Vec<(u32, String)>,
    pub(crate) size: f64,
    pub(crate) color: String,
    pub(crate) restore_icon: Option<String>,
}

/// What an application's tile adds to a box.
#[derive(Clone)]
pub(crate) struct AppIconPart {
    pub(crate) size: f64,
    pub(crate) color: String,
    pub(crate) app_id: String,
}

/// What a frame's border adds to a canvas.
#[derive(Clone)]
pub(crate) struct BorderPart {
    pub(crate) thickness: f64,
    pub(crate) color: String,
    pub(crate) color_of: Option<Closure>,
    pub(crate) radius: f64,
}

/// What a frame's glow adds to a canvas.
#[derive(Clone)]
pub(crate) struct GlowPart {
    pub(crate) color: String,
    pub(crate) strength: f64,
    pub(crate) radius: f64,
    pub(crate) steps: u64,
}

/// What a resize handle adds to a box.
#[derive(Clone)]
pub(crate) struct HandlePart {
    pub(crate) side: u32,
    pub(crate) fit_parent: bool,
    /// Where the pointer pressed, in the handle's own coordinates, while a resize is going.
    pub(crate) from: Option<(f64, f64)>,
}

/// What the eight resize handles' container adds to a box.
#[derive(Clone)]
pub(crate) struct HandlesPart {
    pub(crate) thickness: f64,
    pub(crate) inside: f64,
    pub(crate) corner: f64,
}

/// What a canvas is to its frame.
#[derive(Clone)]
pub(crate) enum PartRole {
    Outlet(OutletPart),
    DragArea,
    Titlebar(BarPart),
    TitleText,
    Button(ButtonPart),
    Buttons(ButtonsPart),
    AppIcon(AppIconPart),
    Backdrop,
    Border(BorderPart),
    Glow(GlowPart),
    Handle(HandlePart),
    Handles(HandlesPart),
}

/// What a canvas of a window frame adds to a canvas of the toolkit.
#[derive(Clone)]
pub(crate) struct FramePart {
    /// The frame the canvas belongs to; zero for a part that belongs to none (a glow, a backdrop).
    pub(crate) frame: FrameId,
    pub(crate) role: PartRole,
    /// The drag behaviour the canvas is a handle for, if it is one: a key of [`FrameWorld::drags`].
    pub(crate) drag: Option<u64>,
}

/// The state of a window's drag: which frame it moves, when the last press was and where this one began.
#[derive(Clone, Debug)]
pub(crate) struct DragState {
    pub(crate) frame: FrameId,
    pub(crate) last_press: Option<f64>,
    pub(crate) from: Option<(f64, f64)>,
}

/// How the window's close fades: when to close it, and when to bring it back if it stayed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CloseFade {
    pub(crate) close_at: Option<f64>,
    pub(crate) restore_at: Option<f64>,
}

/// One window's frame.
pub(crate) struct FrameRec {
    pub(crate) node: u64,
    pub(crate) kind: FrameKind,
    /// The `window_frame` object, and the `window_actions` object beneath it.
    pub(crate) obj: Obj,
    pub(crate) actions: Obj,
    pub(crate) alive: bool,
    pub(crate) outlets: HashMap<FrameSlot, Cid>,
    pub(crate) closing: Option<CloseFade>,
    /// The clock of this frame's actions, when the shell gave one.
    pub(crate) now: Option<Closure>,
    /// The inset last told to `room_changed`.
    pub(crate) told: Inset,
}

/// The shell operations a click on a frame asks for.
#[derive(Default, Clone)]
pub(crate) struct FrameHooks {
    pub(crate) menu: Option<Closure>,
    pub(crate) lower: Option<Closure>,
    pub(crate) maximise: Option<Closure>,
    pub(crate) minimise: Option<Closure>,
    pub(crate) press: Option<Closure>,
    pub(crate) release: Option<Closure>,
}

/// A rule of the decorations: which windows it takes, and how it fills them.
#[derive(Clone)]
pub(crate) struct FrameRule {
    pub(crate) app_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) test: Option<Closure>,
    /// The fill; `None` leaves the window without a frame.
    pub(crate) fill: Option<Closure>,
}

/// What the decorations keep.
#[derive(Default)]
pub(crate) struct Deco {
    pub(crate) started: bool,
    /// The `decorations` object, which the frames are children of.
    pub(crate) obj: Obj,
    pub(crate) fill: Option<Closure>,
    pub(crate) rules: Vec<FrameRule>,
    pub(crate) style: TitlebarStyle,
    pub(crate) resize_handles: f64,
    pub(crate) now: Option<Closure>,
    pub(crate) hooks: FrameHooks,
    /// What the decorations listen to: a change of a window the frame follows asks for a turn.
    pub(crate) listening: Vec<Subscription>,
}

/// What the toolkit keeps of the window frames of one generation.
#[derive(Default)]
pub(crate) struct FrameWorld {
    next: FrameId,
    next_drag: u64,
    pub(crate) frames: BTreeMap<FrameId, FrameRec>,
    pub(crate) by_node: HashMap<u64, FrameId>,
    /// The kind of frame each window was last given, even when that ended with no frame (a rule that leaves it bare, a fill that threw):
    /// the fill runs again only when the kind changes.
    pub(crate) decided: HashMap<u64, FrameKind>,
    /// The room each window's frame reserved when the library was last told.
    pub(crate) sent: HashMap<u64, Inset>,
    pub(crate) drags: HashMap<u64, DragState>,
    pub(crate) deco: Deco,
}

impl FrameWorld {
    pub(crate) fn mint_frame(&mut self) -> FrameId {
        self.next += 1;
        self.next
    }

    pub(crate) fn mint_drag(&mut self, frame: FrameId) -> u64 {
        self.next_drag += 1;
        self.drags.insert(
            self.next_drag,
            DragState {
                frame,
                last_press: None,
                from: None,
            },
        );
        self.next_drag
    }
}

/// The part of a canvas that belongs to a frame.
pub(crate) fn part_of(ui: &Ui, cid: Cid) -> Option<FramePart> {
    ui.read(cid, |widget| match &widget.sp {
        crate::ui::widget::Special::Frame(part) => Some(part.clone()),
        _ => None,
    })
    .flatten()
}

/// One turn of the decorations, at the start of the toolkit's frame: windows that came, went or changed who draws their frame, and the
/// close fades that are due.
pub(crate) fn turn(ui: &Rc<Ui>) {
    if !ui.frames.borrow().deco.started {
        return;
    }
    decorations::reconcile(ui);
    actions::advance_all(ui);
}

/// The end of the toolkit's frame: the frames are placed, so the room each reserves is known and the library is told.
pub(crate) fn after_update(ui: &Rc<Ui>) {
    if ui.frames.borrow().deco.started {
        decorations::send_insets(ui);
    }
}

/// How long until a close fade is next due, in milliseconds, when one is going.
pub(crate) fn wake_in(ui: &Rc<Ui>) -> Option<u64> {
    actions::wake_in(ui)
}

/// Updates a canvas of a frame from the window: the part's own refresh, once a frame, before its children are arranged.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid) -> Result<(), String> {
    match part_of(ui, cid) {
        Some(part) => parts::refresh(ui, cid, &part),
        None => Ok(()),
    }
}

/// What a canvas of a frame does with a pointer event by itself, and whether it stops the event.
pub(crate) fn pointer(ui: &Rc<Ui>, cid: Cid, kind: u32, x: f64, y: f64, button: u64) -> bool {
    match part_of(ui, cid) {
        Some(part) => parts::pointer(ui, cid, &part, kind, x, y, button),
        None => false,
    }
}

/// Places the children of a canvas of a frame when it places them itself (the eight resize handles). `false` for any other canvas.
pub(crate) fn arrange(ui: &Ui, cid: Cid) -> bool {
    part_of(ui, cid).is_some_and(|part| parts::arrange(ui, cid, &part))
}

/// The natural width (or height) of a canvas of a frame, when it is not the toolkit's own.
pub(crate) fn natural(ui: &Ui, cid: Cid, vertical: bool) -> Option<f64> {
    parts::natural(ui, cid, &part_of(ui, cid)?, vertical)
}

/// What a canvas of a frame paints by itself, when it is not the toolkit's own.
pub(crate) fn paint(ui: &Ui, cid: Cid) -> Option<Result<Vec<PaintOp>, String>> {
    parts::paint(ui, cid, &part_of(ui, cid)?)
}

/// A canvas was added under another: an outlet learns how thick what it holds is, and a layer lets the pointer through.
pub(crate) fn added(ui: &Rc<Ui>, parent: Cid, child: Cid) {
    outlet::added(ui, parent, child);
}
