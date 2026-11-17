//! Pointer bindings: move and resize a window by dragging it, swap tiles, move a divider, and the trait for a drag of your own.
//!
//! A binding is claimed on a pointer chord (`Super+Button1`: the modifiers and the button) through the library, which claims it from
//! the core once however many bindings share it and runs the whole matcher: the press records where it was and which window it landed
//! on, the pointer must travel past the binding's [threshold](DragBinding::threshold) before the binding is asked to
//! [start](DragBinding::on_start), a binding that declines leaves the drag to the next of its chord (the higher
//! [priority](DragBinding::priority) is asked first), and every later motion is an [update](DragBinding::on_update) until the release
//! ends the drag or the compositor, the window it pressed closing, or the binding going away cancels it. This crate only turns the
//! library's steps into the trait's calls (`data/js/gnoblin-core/input/pointer.ts` is the reference for the rules).
//!
//! ```ignore
//! let std = g.std()?;
//! std.bind_drag(ClientMoveBinding::new())?;                  // Super+Button1 moves the window under the pointer
//! std.bind_drag(ClientResizeBinding::new().trigger("Alt+Button3"))?;
//! ```
//!
//! [`ClientMoveBinding`], [`ClientResizeBinding`], [`ClientSwapBinding`] and [`SplitResizeBinding`] are the library's own drags: they
//! name a built-in binding kind (`gnoblin_std_abi::layouts::BINDING_KINDS` declares the options) and the library runs them, so every
//! language's shell gets the same behaviour. What they do beyond a plain drag, all of it the TypeScript bindings' (`data/js/gnoblin-std/
//! bindings/*.ts`): a moved window keeps its whole frame (the content and the [frame inset](crate::Client::frame_inset) round it)
//! inside the work area; a drop within a few pixels of a screen edge snaps the window to a half or all of the work area when the
//! seat's `edge_tiling` is on; a resize stops at the window's own [minimum and maximum size](crate::Client::min_size) and a move or
//! resize remembers the rectangle the window floats at ([`Client::floating_bounds`]); and the split resize drags the divider of a
//! tiling layout that has one. The work area is the rectangle of the output the press was on.
//!
//! Trigger spelling is the core's: `Super+Button1`, where the TypeScript says `Super+MouseLeft+Drag`.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core::{NodeId, Rect};
use gnoblin_core_abi as core_abi;
use gnoblin_std_abi::{
    self as abi, GnoblinStdBindingSpec, GnoblinStdDragEvent, GnoblinStdOption, STD_DRAG_CANCEL, STD_DRAG_EDGE_TILING,
    STD_DRAG_END, STD_DRAG_HAS_LOCAL, STD_DRAG_HAS_WORK_AREA, STD_DRAG_START, STD_DRAG_UPDATE,
};

use crate::api::Std;
use crate::builtin::LayoutValue;
use crate::client::Client;
use crate::error::Result;
use crate::subscription::Subscription;

/// How far a pointer travels before a press becomes a drag, in logical pixels. A binding may ask for another distance.
pub const DRAG_THRESHOLD: f64 = 4.0;

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
    /// The seat's `edge_tiling`: whether a drop on a screen edge tiles the window.
    pub edge_tiling: bool,
}

/// A pointer drag of the shell's own, claimed with [`Std::bind_drag`]. Only [`trigger`](DragBinding::trigger) and
/// [`on_update`](DragBinding::on_update) are required.
///
/// The binding is one value that lives as long as its claim, so it keeps the state of a drag in `self` between `on_start` and
/// `on_end`. The library runs one drag of a binding at a time.
pub trait DragBinding: 'static {
    /// The pointer chord that starts the binding: modifiers and a button, `Super+Button1`.
    fn trigger(&self) -> String;

    /// How far the pointer must travel before the binding is asked to start.
    fn threshold(&self) -> f64 {
        DRAG_THRESHOLD
    }

    /// Among the bindings of one chord the higher priority is asked first; one that declines leaves the drag to the next. Two bindings
    /// of one chord and one priority are refused.
    fn priority(&self) -> f64 {
        0.0
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

/// Anything [`Std::bind_drag`] takes: a [`DragBinding`] of the shell's, or one of the library's own drags
/// ([`ClientMoveBinding`], [`ClientResizeBinding`], [`ClientSwapBinding`], [`SplitResizeBinding`]).
pub trait IntoDragBinding {
    /// Claims the binding.
    #[doc(hidden)]
    fn bind(self, std: &Std) -> Result<Subscription>;
}

impl<B: DragBinding> IntoDragBinding for B {
    fn bind(self, std: &Std) -> Result<Subscription> {
        std.bind_custom_drag(self)
    }
}

// ---- the library's own drags -------------------------------------------------------------------------------------------------

/// A pointer binding the library runs, with its options: a thin handle on a built-in binding kind.
fn builtin(std: &Std, kind: u32, options: &[(&str, LayoutValue)]) -> Result<Subscription> {
    std.bind_builtin(kind, options)
}

/// Drags the window under the pointer with it. The window's frame is held inside the work area as the seat's constraint says, and a
/// drop on a screen edge snaps the window there when the seat's `edge_tiling` is on.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientMoveBinding {
    trigger: String,
    priority: f64,
    snap: Option<f64>,
}

impl Default for ClientMoveBinding {
    fn default() -> ClientMoveBinding {
        ClientMoveBinding {
            trigger: "Super+Button1".to_owned(),
            priority: 0.0,
            snap: Some(4.0),
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

    /// Among the bindings of one chord the higher priority is asked first.
    #[must_use]
    pub fn priority(mut self, priority: f64) -> ClientMoveBinding {
        self.priority = priority;
        self
    }

    /// How near a screen edge, in pixels, the pointer must be for a drop to snap the window (left and right take half the work area,
    /// top all of it). Default 4. Only when the seat's `edge_tiling` is on; off, a drop is just a move.
    #[must_use]
    pub fn snap_margin(mut self, margin: f64) -> ClientMoveBinding {
        self.snap = Some(margin.max(0.0));
        self
    }

    /// Never snaps: a drop is just a move.
    #[must_use]
    pub fn no_snap(mut self) -> ClientMoveBinding {
        self.snap = None;
        self
    }
}

impl IntoDragBinding for ClientMoveBinding {
    fn bind(self, std: &Std) -> Result<Subscription> {
        let mut options = vec![
            ("trigger", LayoutValue::Text(self.trigger)),
            ("priority", LayoutValue::Number(self.priority)),
            ("snap", LayoutValue::Bool(self.snap.is_some())),
        ];
        if let Some(margin) = self.snap {
            options.push(("snap_margin", LayoutValue::Number(margin)));
        }
        builtin(std, abi::STD_BINDING_MOVE, &options)
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

impl ResizeGrip {
    fn member(self) -> u32 {
        match self {
            ResizeGrip::Quadrant => 0,
            ResizeGrip::Nearest => 1,
            ResizeGrip::Corner(ResizeCorner::TopLeft) => 2,
            ResizeGrip::Corner(ResizeCorner::TopRight) => 3,
            ResizeGrip::Corner(ResizeCorner::BottomLeft) => 4,
            ResizeGrip::Corner(ResizeCorner::BottomRight) => 5,
        }
    }
}

/// Resizes the window under the pointer by the side or corner nearest the press.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientResizeBinding {
    trigger: String,
    priority: f64,
    grip: ResizeGrip,
}

impl Default for ClientResizeBinding {
    fn default() -> ClientResizeBinding {
        ClientResizeBinding {
            trigger: "Super+Button3".to_owned(),
            priority: 0.0,
            grip: ResizeGrip::Quadrant,
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

    /// Among the bindings of one chord the higher priority is asked first.
    #[must_use]
    pub fn priority(mut self, priority: f64) -> ClientResizeBinding {
        self.priority = priority;
        self
    }

    /// Which side or corner the resize takes hold of.
    #[must_use]
    pub fn grip(mut self, grip: ResizeGrip) -> ClientResizeBinding {
        self.grip = grip;
        self
    }
}

impl IntoDragBinding for ClientResizeBinding {
    fn bind(self, std: &Std) -> Result<Subscription> {
        let options = [
            ("trigger", LayoutValue::Text(self.trigger)),
            ("priority", LayoutValue::Number(self.priority)),
            ("grip", LayoutValue::Enum(self.grip.member())),
        ];
        builtin(std, abi::STD_BINDING_RESIZE, &options)
    }
}

/// Swaps a tiled window with the tile it is dropped on: the pointer takes hold of a tile and the release decides, not the last motion.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientSwapBinding {
    trigger: String,
    priority: f64,
}

impl Default for ClientSwapBinding {
    fn default() -> ClientSwapBinding {
        ClientSwapBinding {
            trigger: "Super+Button1".to_owned(),
            priority: 0.0,
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

    /// Among the bindings of one chord the higher priority is asked first.
    #[must_use]
    pub fn priority(mut self, priority: f64) -> ClientSwapBinding {
        self.priority = priority;
        self
    }
}

impl IntoDragBinding for ClientSwapBinding {
    fn bind(self, std: &Std) -> Result<Subscription> {
        let options = [
            ("trigger", LayoutValue::Text(self.trigger)),
            ("priority", LayoutValue::Number(self.priority)),
        ];
        builtin(std, abi::STD_BINDING_SWAP, &options)
    }
}

/// Moves the divider of a tiling layout nearest the press by as far as the pointer travels, so the tiles on both sides follow it while
/// the pointer is down. The layout decides which divider that is and how its ratio is held; a layout with no divider (a grid, a
/// monocle) has none, and this declines. The ratio is live, so a release keeps it and a cancel puts it back. A window that is not
/// tiled, or is fullscreen or maximised, declines. When the layout hands back a second divider on the other axis, the same drag moves
/// both, each by the pointer's travel along its own axis.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitResizeBinding {
    trigger: String,
    priority: f64,
}

impl Default for SplitResizeBinding {
    fn default() -> SplitResizeBinding {
        SplitResizeBinding {
            trigger: "Super+Button3".to_owned(),
            priority: 0.0,
        }
    }
}

impl SplitResizeBinding {
    /// A split-resize binding on `Super+Button3`.
    pub fn new() -> SplitResizeBinding {
        SplitResizeBinding::default()
    }

    /// The pointer chord that starts the resize.
    #[must_use]
    pub fn trigger(mut self, trigger: impl Into<String>) -> SplitResizeBinding {
        self.trigger = trigger.into();
        self
    }

    /// Among the bindings of one chord the higher priority is asked first.
    #[must_use]
    pub fn priority(mut self, priority: f64) -> SplitResizeBinding {
        self.priority = priority;
        self
    }
}

impl IntoDragBinding for SplitResizeBinding {
    fn bind(self, std: &Std) -> Result<Subscription> {
        let options = [
            ("trigger", LayoutValue::Text(self.trigger)),
            ("priority", LayoutValue::Number(self.priority)),
        ];
        builtin(std, abi::STD_BINDING_SPLIT_RESIZE, &options)
    }
}

// ---- the shell's own drags ---------------------------------------------------------------------------------------------------

/// A binding of the shell's behind the library's `user` pointer.
struct BoxedDrag<B> {
    std: Std,
    binding: RefCell<B>,
}

fn event_of<B>(boxed: &BoxedDrag<B>, raw: &GnoblinStdDragEvent) -> DragEvent {
    let std = &boxed.std;
    DragEvent {
        client: (raw.client != 0).then(|| Client::new(std, std.gnoblin().window_handle(NodeId(raw.client)))),
        position: Point { x: raw.x, y: raw.y },
        origin: Point {
            x: raw.origin_x,
            y: raw.origin_y,
        },
        local_position: (raw.flags & STD_DRAG_HAS_LOCAL != 0).then_some(Point {
            x: raw.local_x,
            y: raw.local_y,
        }),
        button: raw.button,
        work_area: (raw.flags & STD_DRAG_HAS_WORK_AREA != 0).then(|| {
            Rect::new(
                raw.work_area.x,
                raw.work_area.y,
                raw.work_area.width,
                raw.work_area.height,
            )
        }),
        constraint: if raw.constraint == abi::STD_CONSTRAINT_MINIMUM_STRIP {
            DragConstraint::MinimumStrip
        } else {
            DragConstraint::WorkArea
        },
        edge_tiling: raw.flags & STD_DRAG_EDGE_TILING != 0,
    }
}

unsafe extern "C" fn drag_tramp<B: DragBinding>(user: *mut c_void, event: *const GnoblinStdDragEvent) -> i32 {
    // SAFETY: `user` is the `BoxedDrag` leaked by `bind_custom_drag`; the library passes it back unchanged on the shell thread and frees
    // it only through `drop_boxed_drag`, after the binding is removed or the generation retires. `event` is the library's, valid for
    // the call.
    let (Some(boxed), Some(raw)) = (unsafe { user.cast::<BoxedDrag<B>>().as_ref() }, unsafe {
        event.as_ref()
    }) else {
        return core_abi::E_ERR;
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let event = event_of(boxed, raw);
        let mut binding = boxed.binding.borrow_mut();
        match raw.phase {
            STD_DRAG_START => i32::from(binding.on_start(&event)),
            STD_DRAG_UPDATE => {
                binding.on_update(&event);
                core_abi::E_OK
            }
            STD_DRAG_END => {
                binding.on_end(&event);
                core_abi::E_OK
            }
            STD_DRAG_CANCEL => {
                binding.on_cancel(&event);
                core_abi::E_OK
            }
            _ => core_abi::E_OK,
        }
    }));
    outcome.unwrap_or(core_abi::E_PANIC)
}

/// Frees the binding behind a `user` pointer. The library calls it exactly once, when the binding is removed or the generation retires.
unsafe extern "C" fn drop_boxed_drag<B>(user: *mut c_void) {
    // SAFETY: `user` is the `Box<BoxedDrag<B>>` leaked by `bind_custom_drag` and the library calls this exactly once.
    drop(unsafe { Box::from_raw(user.cast::<BoxedDrag<B>>()) });
}

impl Std {
    /// Starts a binding the library runs itself (`kind` is `STD_BINDING_*`, the options those `BINDING_KINDS` declares for it). The
    /// typed bindings ([`ClientMoveBinding`] and the rest) are this call with their options.
    ///
    /// # Errors
    ///
    /// The kind or an option is unknown, a value has the wrong type or is out of range, the trigger and priority are taken, the
    /// compositor refused the claim, or the generation retired.
    pub fn bind_builtin(&self, kind: u32, options: &[(&str, LayoutValue)]) -> Result<Subscription> {
        let records: Vec<GnoblinStdOption> = options.iter().map(|(name, value)| value.record(name)).collect();
        // SAFETY: the records and the names and text they borrow are live for the call; the library copies what it keeps.
        let reg = unsafe { (self.api().binding_builtin)(self.ctx(), kind, records.as_ptr(), records.len()) };
        self.reg_or_failure(reg)
    }

    fn bind_custom_drag<B: DragBinding>(&self, binding: B) -> Result<Subscription> {
        let trigger = binding.trigger();
        let spec = GnoblinStdBindingSpec {
            size: size_of::<GnoblinStdBindingSpec>() as u32,
            reserved: 0,
            trigger: core_abi::GnoblinStr::borrowed(&trigger),
            priority: binding.priority(),
            threshold: binding.threshold(),
        };
        let boxed = Box::into_raw(Box::new(BoxedDrag {
            std: self.clone(),
            binding: RefCell::new(binding),
        }));
        // SAFETY: `spec` and the trigger it borrows are live for the call. On success the library adopts `boxed` and frees it through
        // `drop_boxed_drag`; on failure it has not, and it is reclaimed below.
        let reg = unsafe {
            (self.api().binding_drag)(
                self.ctx(),
                &spec,
                drag_tramp::<B>,
                boxed.cast(),
                Some(drop_boxed_drag::<B>),
            )
        };
        let outcome = self.reg_or_failure(reg);
        if outcome.is_err() {
            // SAFETY: the library did not adopt the binding, so nothing else owns the box.
            drop(unsafe { Box::from_raw(boxed) });
        }
        outcome
    }

    /// The subscription for a registration the library made, or the failure that says why it did not (zero).
    pub(crate) fn reg_or_failure(&self, reg: abi::GnoblinStdReg) -> Result<Subscription> {
        if reg == 0 {
            Err(self.failure())
        } else {
            Ok(Subscription::new(self.clone(), reg))
        }
    }

    /// Claims the binding's chord and runs its drags. The claim lasts for the life of the generation, or until the returned
    /// subscription is removed; a drag in flight when it is removed is cancelled.
    ///
    /// # Errors
    ///
    /// The compositor refused the claim (another shell holds the chord), the chord and the priority are taken by another binding of
    /// this shell, an option is out of range, or the generation retired.
    pub fn bind_drag(&self, binding: impl IntoDragBinding) -> Result<Subscription> {
        binding.bind(self)
    }
}
