//! Pointer bindings: move and resize a window by dragging it, and the trait for a drag of your own.
//!
//! A binding is claimed on a pointer chord (`Super+Button1`: the modifiers and the button) through the core's claim, so the compositor
//! withholds the press from the window under it. The SDK turns the claim's press, motion and release into a drag the way
//! `data/js/gnoblin-core/input/pointer.ts` does, which is the reference: a press records where it was and which window it landed on, the
//! pointer must travel past the binding's [threshold](DragBinding::threshold) before the binding is asked to
//! [start](DragBinding::on_start), the start event says where the press was (so a binding that measures from the position measures from
//! the press), and every later motion is an [update](DragBinding::on_update) until the release ends the drag or the compositor cancels
//! it. A binding that declines to start leaves the press a press: nothing more is asked of it until the next one.
//!
//! ```ignore
//! let std = g.std()?;
//! std.bind_drag(ClientMoveBinding::new())?;                  // Super+Button1 moves the window under the pointer
//! std.bind_drag(ClientResizeBinding::new().trigger("Alt+Button3"))?;
//! ```
//!
//! What the TypeScript bindings do that these do not, and why: `ClientMoveBinding`'s edge snapping into tiled zones needs the library's
//! snap zones (the std library has none yet), the window's frame inset is not in the std ABI (a window's frame is kept inside the work
//! area by its content rectangle here), a window's own minimum and maximum size are not read (the resize keeps the TypeScript's 100 by
//! 80 floor), the dragged window's floating bounds are not remembered, and the work area is the output's rectangle. They are listed in
//! `design/modular-framework.md` section 22.
//!
//! Trigger spelling is the core's: `Super+Button1`, where the TypeScript says `Super+MouseLeft+Drag`.

use gnoblin_core::{DragPhase, PointerDrag, Rect, Subscription};

use crate::api::Std;
use crate::client::Client;
use crate::error::Result;
use crate::workspace::Workspace;

/// How far a pointer travels before a press becomes a drag, in logical pixels.
pub const DRAG_THRESHOLD: f64 = 4.0;

/// How much of a window stays on screen under [`DragConstraint::MinimumStrip`].
pub const DRAG_STRIP: f64 = 75.0;

/// A point in the compositor's global coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal place.
    pub x: f64,
    /// Vertical place.
    pub y: f64,
}

/// The seat's rule for how far a dragged window may leave the work area (`drag_constraint` on the seat).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DragConstraint {
    /// The window stays inside the work area (the default).
    #[default]
    WorkArea,
    /// The window may leave it, but a strip of it stays reachable.
    MinimumStrip,
}

/// What a drag binding is told at each step.
#[derive(Clone, Debug)]
pub struct DragEvent {
    /// The window the press landed on, if one was under the pointer.
    pub client: Option<Client>,
    /// Where the pointer is. On the start, where the press was.
    pub position: Point,
    /// Where the press was.
    pub origin: Point,
    /// `position` in the window's own coordinates, when a window is under the press.
    pub local_position: Option<Point>,
    /// The chord's button number (1 left, 2 middle, 3 right).
    pub button: i64,
    /// The rectangle windows are kept inside: the output the press was on.
    pub work_area: Option<Rect>,
    /// The seat's drag constraint.
    pub constraint: DragConstraint,
}

/// A pointer drag of the shell's own, claimed with [`Std::bind_drag`]. Only [`trigger`](DragBinding::trigger) and
/// [`on_update`](DragBinding::on_update) are required.
///
/// The binding is one value that lives as long as its claim, so it keeps the state of a drag in `self` between `on_start` and
/// `on_end`. The SDK runs one drag of a binding at a time.
pub trait DragBinding: 'static {
    /// The pointer chord that starts the binding: modifiers and a button, `Super+Button1`.
    fn trigger(&self) -> String;

    /// How far the pointer must travel before the binding is asked to start.
    fn threshold(&self) -> f64 {
        DRAG_THRESHOLD
    }

    /// The pointer moved past the threshold. Return `false` to decline (nothing is under the pointer, the window may not be dragged):
    /// the press then stays a press. The event's position is where the press was.
    fn on_start(&mut self, _event: &DragEvent) -> bool {
        true
    }

    /// The pointer moved.
    fn on_update(&mut self, event: &DragEvent);

    /// The button was released.
    fn on_end(&mut self, _event: &DragEvent) {}

    /// The compositor ended the drag (the session locked, the claim went), or the window closed under it. The binding undoes what it
    /// did.
    fn on_cancel(&mut self, _event: &DragEvent) {}
}

/// `pos` held inside `[start, start + span]` for a window of `size`: a window larger than the span goes to the start.
pub fn clamp_axis(pos: f64, size: f64, start: f64, span: f64, mode: DragConstraint) -> f64 {
    match mode {
        DragConstraint::WorkArea => {
            if size >= span {
                start
            } else {
                pos.max(start).min(start + span - size)
            }
        }
        DragConstraint::MinimumStrip => {
            let visible = DRAG_STRIP.min(size);
            let low = start - (size - visible);
            let high = start + span - visible;
            pos.max(low).min(high)
        }
    }
}

/// `rect` moved to where `mode` allows inside `area`; the size is kept.
pub fn constrain_rect(rect: Rect, area: Rect, mode: DragConstraint) -> Rect {
    Rect::new(
        clamp_axis(rect.x, rect.width, area.x, area.width, mode),
        clamp_axis(rect.y, rect.height, area.y, area.height, mode),
        rect.width,
        rect.height,
    )
}

/// Whether a window may be dragged at all: not fullscreen and not maximised.
fn draggable(client: &Client) -> bool {
    !client.window().is_fullscreen().unwrap_or(false) && !client.window().is_maximised().unwrap_or(false)
}

/// Drags the window under the pointer with it. Held inside the work area as the seat's constraint says.
#[derive(Debug)]
pub struct ClientMoveBinding {
    trigger: String,
    drag: Option<(Client, Point, Rect)>,
}

impl Default for ClientMoveBinding {
    fn default() -> ClientMoveBinding {
        ClientMoveBinding {
            trigger: "Super+Button1".to_owned(),
            drag: None,
        }
    }
}

impl ClientMoveBinding {
    /// A move binding on `Super+Button1`.
    pub fn new() -> ClientMoveBinding {
        ClientMoveBinding::default()
    }

    /// The pointer chord that starts the move.
    #[must_use]
    pub fn trigger(mut self, trigger: impl Into<String>) -> ClientMoveBinding {
        self.trigger = trigger.into();
        self
    }
}

impl DragBinding for ClientMoveBinding {
    fn trigger(&self) -> String {
        self.trigger.clone()
    }

    fn on_start(&mut self, event: &DragEvent) -> bool {
        let Some(client) = event.client.clone().filter(draggable) else {
            return false;
        };
        let Ok(bounds) = client.bounds() else {
            return false;
        };
        self.drag = Some((client, event.origin, bounds));
        true
    }

    fn on_update(&mut self, event: &DragEvent) {
        let Some((client, origin, start)) = &self.drag else {
            return;
        };
        let target = Rect::new(
            start.x + (event.position.x - origin.x),
            start.y + (event.position.y - origin.y),
            start.width,
            start.height,
        );
        let bounds = event
            .work_area
            .map_or(target, |area| constrain_rect(target, area, event.constraint));
        let _ = client.set_bounds(bounds);
    }

    fn on_end(&mut self, _event: &DragEvent) {
        self.drag = None;
    }

    fn on_cancel(&mut self, _event: &DragEvent) {
        if let Some((client, _, start)) = self.drag.take() {
            let _ = client.set_bounds(start);
        }
    }
}

/// A corner of a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeCorner {
    /// The top left corner.
    TopLeft,
    /// The top right corner.
    TopRight,
    /// The bottom left corner.
    BottomLeft,
    /// The bottom right corner.
    BottomRight,
}

/// An edge of a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
}

/// The sides of a window a resize moves: a corner moves two, an edge one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grip {
    /// Moves the left side.
    pub left: bool,
    /// Moves the right side.
    pub right: bool,
    /// Moves the top side.
    pub top: bool,
    /// Moves the bottom side.
    pub bottom: bool,
}

impl From<ResizeCorner> for Grip {
    fn from(corner: ResizeCorner) -> Grip {
        Grip {
            left: matches!(corner, ResizeCorner::TopLeft | ResizeCorner::BottomLeft),
            right: matches!(corner, ResizeCorner::TopRight | ResizeCorner::BottomRight),
            top: matches!(corner, ResizeCorner::TopLeft | ResizeCorner::TopRight),
            bottom: matches!(corner, ResizeCorner::BottomLeft | ResizeCorner::BottomRight),
        }
    }
}

impl From<ResizeEdge> for Grip {
    fn from(edge: ResizeEdge) -> Grip {
        Grip {
            left: edge == ResizeEdge::Left,
            right: edge == ResizeEdge::Right,
            top: edge == ResizeEdge::Top,
            bottom: edge == ResizeEdge::Bottom,
        }
    }
}

/// Which side or corner a resize takes hold of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResizeGrip {
    /// The corner of the quadrant the press landed in (the default).
    #[default]
    Quadrant,
    /// The edge nearest the press.
    Nearest,
    /// Always this corner.
    Corner(ResizeCorner),
}

/// The smallest size a resize leaves a window.
pub const MIN_SIZE: (f64, f64) = (100.0, 80.0);

/// `start` resized by `delta` with the sides of `grip` following the pointer, never below `min` or above `max`; a side that is stopped
/// by a limit leaves the opposite side where it was.
pub fn resized_rect(start: Rect, grip: Grip, delta: Point, min: (f64, f64), max: Option<(f64, f64)>) -> Rect {
    let mut r = start;
    if grip.left {
        r.x += delta.x;
        r.width -= delta.x;
    } else if grip.right {
        r.width += delta.x;
    }
    if grip.top {
        r.y += delta.y;
        r.height -= delta.y;
    } else if grip.bottom {
        r.height += delta.y;
    }
    if grip.left || grip.right {
        let limited = r.width.min(max.map_or(f64::INFINITY, |max| max.0)).max(min.0);
        if grip.left {
            r.x += r.width - limited;
        }
        r.width = limited;
    }
    if grip.top || grip.bottom {
        let limited = r.height.min(max.map_or(f64::INFINITY, |max| max.1)).max(min.1);
        if grip.top {
            r.y += r.height - limited;
        }
        r.height = limited;
    }
    r
}

fn nearest_edge(at: Point, width: f64, height: f64) -> ResizeEdge {
    let gaps = [
        (ResizeEdge::Left, at.x),
        (ResizeEdge::Top, at.y),
        (ResizeEdge::Right, width - at.x),
        (ResizeEdge::Bottom, height - at.y),
    ];
    gaps.into_iter()
        .reduce(|best, gap| if gap.1 < best.1 { gap } else { best })
        .map_or(ResizeEdge::Left, |(edge, _)| edge)
}

/// Resizes the window under the pointer by the side or corner nearest the press.
#[derive(Debug)]
pub struct ClientResizeBinding {
    trigger: String,
    grip: ResizeGrip,
    drag: Option<(Client, Rect, Point, Grip)>,
}

impl Default for ClientResizeBinding {
    fn default() -> ClientResizeBinding {
        ClientResizeBinding {
            trigger: "Super+Button3".to_owned(),
            grip: ResizeGrip::Quadrant,
            drag: None,
        }
    }
}

impl ClientResizeBinding {
    /// A resize binding on `Super+Button3` that takes the corner of the quadrant the press landed in.
    pub fn new() -> ClientResizeBinding {
        ClientResizeBinding::default()
    }

    /// The pointer chord that starts the resize.
    #[must_use]
    pub fn trigger(mut self, trigger: impl Into<String>) -> ClientResizeBinding {
        self.trigger = trigger.into();
        self
    }

    /// Which side or corner the resize takes hold of.
    #[must_use]
    pub fn grip(mut self, grip: ResizeGrip) -> ClientResizeBinding {
        self.grip = grip;
        self
    }
}

impl DragBinding for ClientResizeBinding {
    fn trigger(&self) -> String {
        self.trigger.clone()
    }

    fn on_start(&mut self, event: &DragEvent) -> bool {
        let Some(client) = event.client.clone().filter(draggable) else {
            return false;
        };
        let Ok(start) = client.bounds() else {
            return false;
        };
        let at = event.local_position.unwrap_or(Point {
            x: event.position.x - start.x,
            y: event.position.y - start.y,
        });
        let grip = match self.grip {
            ResizeGrip::Quadrant => {
                let corner = match (at.y < start.height / 2.0, at.x < start.width / 2.0) {
                    (true, true) => ResizeCorner::TopLeft,
                    (true, false) => ResizeCorner::TopRight,
                    (false, true) => ResizeCorner::BottomLeft,
                    (false, false) => ResizeCorner::BottomRight,
                };
                Grip::from(corner)
            }
            ResizeGrip::Nearest => Grip::from(nearest_edge(at, start.width, start.height)),
            ResizeGrip::Corner(corner) => Grip::from(corner),
        };
        self.drag = Some((client, start, event.position, grip));
        true
    }

    fn on_update(&mut self, event: &DragEvent) {
        let Some((client, start, origin, grip)) = &self.drag else {
            return;
        };
        let delta = Point {
            x: event.position.x - origin.x,
            y: event.position.y - origin.y,
        };
        let _ = client.set_bounds(resized_rect(*start, *grip, delta, MIN_SIZE, None));
    }

    fn on_end(&mut self, _event: &DragEvent) {
        self.drag = None;
    }

    fn on_cancel(&mut self, _event: &DragEvent) {
        if let Some((client, start, _, _)) = self.drag.take() {
            let _ = client.set_bounds(start);
        }
    }
}

/// Whether `client` sits in a tile: tiled by its layout, not floating, not a dialog, and draggable.
fn in_tile(client: &Client) -> bool {
    client.is_tiled().unwrap_or(false)
        && !client.is_floating().unwrap_or(false)
        && client.parent_window().ok().flatten().is_none()
        && draggable(client)
}

/// The tile of `workspace` that holds `at`, other than `except`: the first in the workspace's order that is shown.
fn tile_at(workspace: &Workspace, at: Point, except: &Client) -> Option<Client> {
    let clients = workspace.clients().ok()?.to_vec().ok()?;
    clients.into_iter().find(|client| {
        let Ok(rect) = client.bounds() else {
            return false;
        };
        client.window().id() != except.window().id()
            && in_tile(client)
            && !client.window().is_minimised().unwrap_or(false)
            && client.is_visible().unwrap_or(true)
            && at.x >= rect.x
            && at.x < rect.x + rect.width
            && at.y >= rect.y
            && at.y < rect.y + rect.height
    })
}

/// Swaps a tiled window with the tile it is dropped on: the pointer takes hold of a tile and the release decides, not the last motion.
#[derive(Debug)]
pub struct ClientSwapBinding {
    trigger: String,
    drag: Option<(Client, Workspace)>,
}

impl Default for ClientSwapBinding {
    fn default() -> ClientSwapBinding {
        ClientSwapBinding {
            trigger: "Super+Button1".to_owned(),
            drag: None,
        }
    }
}

impl ClientSwapBinding {
    /// A swap binding on `Super+Button1`.
    pub fn new() -> ClientSwapBinding {
        ClientSwapBinding::default()
    }

    /// The pointer chord that starts the swap.
    #[must_use]
    pub fn trigger(mut self, trigger: impl Into<String>) -> ClientSwapBinding {
        self.trigger = trigger.into();
        self
    }
}

impl DragBinding for ClientSwapBinding {
    fn trigger(&self) -> String {
        self.trigger.clone()
    }

    fn on_start(&mut self, event: &DragEvent) -> bool {
        let Some(client) = event.client.clone().filter(in_tile) else {
            return false;
        };
        let Some(workspace) = client.workspace().ok().flatten() else {
            return false;
        };
        self.drag = Some((client, workspace));
        true
    }

    fn on_update(&mut self, _event: &DragEvent) {}

    fn on_end(&mut self, event: &DragEvent) {
        let Some((dragged, workspace)) = self.drag.take() else {
            return;
        };
        // The release decides: the pointer may have moved since the last sample.
        if let Some(over) = tile_at(&workspace, event.position, &dragged)
            && dragged.workspace().ok().flatten().as_ref() == Some(&workspace)
        {
            let _ = workspace.swap(&dragged, &over);
        }
    }

    fn on_cancel(&mut self, _event: &DragEvent) {
        self.drag = None;
    }
}

/// One press and what became of it.
struct Press {
    origin: Point,
    button: i64,
    client: Option<Client>,
    local: Option<Point>,
    work_area: Option<Rect>,
    last: Point,
    state: PressState,
}

#[derive(PartialEq, Eq)]
enum PressState {
    /// Down, and not yet moved far enough.
    Pressed,
    /// The binding took it.
    Dragging,
    /// The binding declined it: the press stays one, and nothing more is asked.
    Declined,
}

impl Std {
    /// The topmost window whose rectangle holds `at`, not minimised: what a press lands on.
    fn window_at(&self, at: Point) -> Option<(Client, Rect)> {
        let windows = self.gnoblin().windows().ok()?;
        windows.into_iter().rev().find_map(|window| {
            if window.is_minimised().unwrap_or(false) {
                return None;
            }
            let rect = window.geometry().ok()?;
            let inside = at.x >= rect.x && at.x < rect.x + rect.width && at.y >= rect.y && at.y < rect.y + rect.height;
            inside.then(|| (self.client(&window), rect))
        })
    }

    /// The output rectangle that holds `at`, else the first output's.
    fn work_area_at(&self, at: Point) -> Option<Rect> {
        let outputs = self.outputs().ok()?.to_vec().ok()?;
        let rects: Vec<Rect> = outputs.iter().filter_map(|output| output.rect().ok()).collect();
        rects
            .iter()
            .copied()
            .find(|r| at.x >= r.x && at.x < r.x + r.width && at.y >= r.y && at.y < r.y + r.height)
            .or_else(|| rects.first().copied())
    }

    /// The seat's `drag_constraint`.
    fn drag_constraint(&self) -> DragConstraint {
        let read = || -> Result<DragConstraint> {
            let g = self.gnoblin();
            let seat = g.class("seat")?;
            for node in g.root()?.children()? {
                if node.class()? == seat {
                    return Ok(if node.get_text("drag_constraint")? == "minimum_strip" {
                        DragConstraint::MinimumStrip
                    } else {
                        DragConstraint::WorkArea
                    });
                }
            }
            Ok(DragConstraint::WorkArea)
        };
        read().unwrap_or_default()
    }

    /// Claims the binding's chord and runs its drags. The claim lasts for the life of the generation, or until the returned
    /// subscription is removed; a drag in flight when it is removed is not told (the claim is gone, as it is when a session locks).
    ///
    /// # Errors
    ///
    /// The compositor refused the claim (another shell holds the chord), or the generation retired.
    pub fn bind_drag<B: DragBinding>(&self, mut binding: B) -> Result<Subscription> {
        let std = self.clone();
        let mut press: Option<Press> = None;
        let trigger = binding.trigger();
        let threshold = binding.threshold();
        let claim = self.gnoblin().bind_pointer(&trigger, move |drag: &PointerDrag| {
            let here = Point { x: drag.x, y: drag.y };
            match drag.phase {
                DragPhase::Start => {
                    // A start before the end of the last press: the first one's end was lost, so it is finished as cancelled.
                    if let Some(previous) = press.take()
                        && previous.state == PressState::Dragging
                    {
                        binding.on_cancel(&event_of(&std, &previous, previous.last));
                    }
                    let hit = std.window_at(here);
                    press = Some(Press {
                        origin: here,
                        button: drag.button,
                        local: hit.as_ref().map(|(_, rect)| Point {
                            x: here.x - rect.x,
                            y: here.y - rect.y,
                        }),
                        client: hit.map(|(client, _)| client),
                        work_area: std.work_area_at(here),
                        last: here,
                        state: PressState::Pressed,
                    });
                }
                DragPhase::Update => {
                    let Some(held) = press.as_mut() else { return };
                    if held.state == PressState::Pressed {
                        let moved = (here.x - held.origin.x).hypot(here.y - held.origin.y);
                        if moved >= threshold {
                            // The start says where the press was; the update that follows says where the pointer is.
                            let start = event_of(&std, held, held.origin);
                            if binding.on_start(&start) {
                                held.state = PressState::Dragging;
                                held.last = held.origin;
                            } else {
                                held.state = PressState::Declined;
                            }
                        }
                    }
                    if held.state == PressState::Dragging && here != held.last {
                        binding.on_update(&event_of(&std, held, here));
                        held.last = here;
                    }
                }
                DragPhase::End => {
                    if let Some(held) = press.take()
                        && held.state == PressState::Dragging
                    {
                        binding.on_end(&event_of(&std, &held, here));
                    }
                }
                DragPhase::Cancel => {
                    if let Some(held) = press.take()
                        && held.state == PressState::Dragging
                    {
                        binding.on_cancel(&event_of(&std, &held, held.last));
                    }
                }
            }
        })?;
        Ok(claim)
    }
}

fn event_of(std: &Std, press: &Press, position: Point) -> DragEvent {
    DragEvent {
        client: press.client.clone(),
        position,
        origin: press.origin,
        local_position: press.local,
        button: press.button,
        work_area: press.work_area,
        constraint: std.drag_constraint(),
    }
}
