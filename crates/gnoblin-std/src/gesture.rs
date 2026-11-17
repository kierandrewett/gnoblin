//! Touchpad gestures: the [`GestureBinding`] trait, and [`Std::bind_gesture`] that runs one.
//!
//! The library claims every swipe or pinch of a finger count for the shell and runs the matcher (`data/js/gnoblin-core/input/gesture.ts`
//! is the reference for its rules); this crate only turns its steps into the trait's calls:
//!
//! - A gesture **begins** when the fingers touch. A binding with a direction (`Swipe3Left`, `Pinch2In`) waits until the fingers have
//!   gone far enough to have one (12 touchpad units, or a scale that has moved 0.1 from 1) and is asked to
//!   [start](GestureBinding::on_start) only if the gesture went its way; a binding with no direction is asked at once. A binding that
//!   declines leaves the gesture to the next of its trigger (the higher [priority](GestureBinding::priority) is asked first), and is not
//!   asked again for this gesture.
//! - Every later step is an [update](GestureBinding::on_update) with how far the fingers have gone and `progress`, 0 to 1, for a
//!   directed gesture (400 units of swipe, a scale change of 0.5 of a pinch, in its own direction only).
//! - The lift ends it ([`on_end`](GestureBinding::on_end)); the compositor cancelling it, or the window under it closing, ends it with
//!   [`on_cancel`](GestureBinding::on_cancel).
//!
//! ```ignore
//! struct Slide;
//! impl GestureBinding for Slide {
//!     fn trigger(&self) -> String { "Swipe3".to_owned() }
//!     fn on_update(&mut self, event: &GestureInfo) { /* follow the fingers */ }
//!     fn on_end(&mut self, event: &GestureInfo) { if event.progress > SWIPE_COMMIT { /* commit */ } }
//! }
//! std.bind_gesture(Slide)?;
//! ```
//!
//! Bindings of one finger count share a claim, so a swipe left and a swipe right on three fingers are two bindings.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core::{GestureKind, NodeId};
use gnoblin_core_abi as core_abi;
use gnoblin_std_abi::{
    self as abi, GnoblinStdBindingSpec, GnoblinStdGestureEvent, STD_GESTURE_BEGIN, STD_GESTURE_CANCEL, STD_GESTURE_END,
    STD_GESTURE_HAS_POSITION, STD_GESTURE_UPDATE,
};

use crate::api::Std;
use crate::bindings::Point;
use crate::client::Client;
use crate::error::Result;
use crate::subscription::Subscription;

/// Minimum progress for a swipe action to run when the gesture ends: 48 touchpad units of the 400 of a whole swipe.
pub const SWIPE_COMMIT: f64 = 48.0 / 400.0;
/// Minimum progress for a pinch action to run when the gesture ends: 0.12 of the 0.5 of scale of a whole pinch.
pub const PINCH_COMMIT: f64 = 0.12 / 0.5;

/// The way a gesture went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureDirection {
    /// A swipe to the left.
    Left,
    /// A swipe to the right.
    Right,
    /// A swipe up.
    Up,
    /// A swipe down.
    Down,
    /// A pinch that closes.
    In,
    /// A pinch that opens.
    Out,
}

impl GestureDirection {
    fn from_way(way: u32) -> Option<GestureDirection> {
        match way {
            abi::STD_WAY_LEFT => Some(GestureDirection::Left),
            abi::STD_WAY_RIGHT => Some(GestureDirection::Right),
            abi::STD_WAY_UP => Some(GestureDirection::Up),
            abi::STD_WAY_DOWN => Some(GestureDirection::Down),
            abi::STD_WAY_IN => Some(GestureDirection::In),
            abi::STD_WAY_OUT => Some(GestureDirection::Out),
            _ => None,
        }
    }
}

/// Where in a gesture an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureStage {
    /// The binding is being asked to start.
    Begin,
    /// The fingers moved.
    Update,
    /// The fingers lifted.
    End,
    /// The compositor ended the gesture, or the window under it closed.
    Cancel,
}

/// What a gesture binding is told at each step.
#[derive(Clone, Debug)]
pub struct GestureInfo {
    /// The window under the pointer when the gesture began, if one was.
    pub client: Option<Client>,
    /// Where in the gesture this is.
    pub phase: GestureStage,
    /// A swipe or a pinch.
    pub kind: GestureKind,
    /// The way the binding's trigger names, or `None` for a trigger with no direction.
    pub trigger_direction: Option<GestureDirection>,
    /// How many fingers.
    pub fingers: u8,
    /// Horizontal travel of the fingers' centre since the begin, in touchpad units.
    pub dx: f64,
    /// Vertical travel.
    pub dy: f64,
    /// A pinch's scale: 1 at the begin.
    pub scale: f64,
    /// A pinch's rotation in degrees.
    pub rotation: f64,
    /// How far a directed gesture has gone, 0 to 1: 400 units of swipe, or a scale change of 0.5, in the trigger's own direction.
    /// For a trigger with no direction, how far the fingers have gone in any way.
    pub progress: f64,
    /// Where the pointer is, when the seat reports it.
    pub position: Option<Point>,
    way: Option<GestureDirection>,
}

impl GestureInfo {
    /// The way the fingers have gone so far (the longer axis of a swipe; a pinch's scale from 1), or `None` while they have gone less
    /// than 12 touchpad units or 0.1 of scale.
    pub fn direction(&self) -> Option<GestureDirection> {
        self.way
    }
}

/// A gesture of the shell's own, claimed with [`Std::bind_gesture`]. Only [`trigger`](GestureBinding::trigger) is required.
///
/// The binding is one value that lives as long as its claim, so it keeps the state of a gesture in `self` between `on_start` and
/// `on_end`. The library runs one gesture of a trigger at a time.
pub trait GestureBinding: 'static {
    /// What the binding listens for: `Swipe3`, `Swipe4Left`, `Pinch2In`.
    fn trigger(&self) -> String;

    /// Among the bindings of one trigger the higher priority is asked first. Two of one trigger and one priority are refused.
    fn priority(&self) -> f64 {
        0.0
    }

    /// The gesture began (or, for a directed binding, went the right way). Return `false` to decline: the binding is not asked again
    /// for this gesture.
    fn on_start(&mut self, _event: &GestureInfo) -> bool {
        true
    }

    /// The fingers moved.
    fn on_update(&mut self, _event: &GestureInfo) {}

    /// The fingers lifted.
    fn on_end(&mut self, _event: &GestureInfo) {}

    /// The compositor ended the gesture, or the window under it closed. The binding undoes what it did.
    fn on_cancel(&mut self, _event: &GestureInfo) {}
}

/// A binding of the shell's behind the library's `user` pointer.
struct BoxedGesture<B> {
    std: Std,
    binding: RefCell<B>,
}

fn info_of<B>(boxed: &BoxedGesture<B>, raw: &GnoblinStdGestureEvent) -> GestureInfo {
    let std = &boxed.std;
    GestureInfo {
        client: (raw.client != 0).then(|| Client::new(std, std.gnoblin().window_handle(NodeId(raw.client)))),
        phase: match raw.phase {
            STD_GESTURE_BEGIN => GestureStage::Begin,
            STD_GESTURE_END => GestureStage::End,
            STD_GESTURE_CANCEL => GestureStage::Cancel,
            _ => GestureStage::Update,
        },
        kind: if raw.kind == abi::STD_GESTURE_PINCH {
            GestureKind::Pinch
        } else {
            GestureKind::Swipe
        },
        trigger_direction: GestureDirection::from_way(raw.direction),
        fingers: raw.fingers as u8,
        dx: raw.dx,
        dy: raw.dy,
        scale: raw.scale,
        rotation: raw.rotation,
        progress: raw.progress,
        position: (raw.flags & STD_GESTURE_HAS_POSITION != 0).then_some(Point { x: raw.x, y: raw.y }),
        way: GestureDirection::from_way(raw.way),
    }
}

unsafe extern "C" fn gesture_tramp<B: GestureBinding>(user: *mut c_void, event: *const GnoblinStdGestureEvent) -> i32 {
    // SAFETY: `user` is the `BoxedGesture` leaked by `bind_gesture`; the library passes it back unchanged on the shell thread and frees
    // it only through `drop_boxed_gesture`, after the binding is removed or the generation retires. `event` is the library's, valid
    // for the call.
    let (Some(boxed), Some(raw)) = (unsafe { user.cast::<BoxedGesture<B>>().as_ref() }, unsafe {
        event.as_ref()
    }) else {
        return core_abi::E_ERR;
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let info = info_of(boxed, raw);
        let mut binding = boxed.binding.borrow_mut();
        match raw.phase {
            STD_GESTURE_BEGIN => i32::from(binding.on_start(&info)),
            STD_GESTURE_UPDATE => {
                binding.on_update(&info);
                core_abi::E_OK
            }
            STD_GESTURE_END => {
                binding.on_end(&info);
                core_abi::E_OK
            }
            STD_GESTURE_CANCEL => {
                binding.on_cancel(&info);
                core_abi::E_OK
            }
            _ => core_abi::E_OK,
        }
    }));
    outcome.unwrap_or(core_abi::E_PANIC)
}

/// Frees the binding behind a `user` pointer. The library calls it exactly once.
unsafe extern "C" fn drop_boxed_gesture<B>(user: *mut c_void) {
    // SAFETY: `user` is the `Box<BoxedGesture<B>>` leaked by `bind_gesture` and the library calls this exactly once.
    drop(unsafe { Box::from_raw(user.cast::<BoxedGesture<B>>()) });
}

impl Std {
    /// Claims the touchpad gesture the binding names and runs its gestures, as the module documentation describes. The claim lasts for
    /// the life of the generation, or until the returned subscription is removed; a gesture in flight when it is removed is cancelled.
    ///
    /// # Errors
    ///
    /// The binding's trigger is not a gesture, the compositor refused the claim (another shell holds it), the trigger and the priority
    /// are taken by another binding of this shell, or the generation retired.
    pub fn bind_gesture<B: GestureBinding>(&self, binding: B) -> Result<Subscription> {
        let trigger = binding.trigger();
        let spec = GnoblinStdBindingSpec {
            size: size_of::<GnoblinStdBindingSpec>() as u32,
            reserved: 0,
            trigger: core_abi::GnoblinStr::borrowed(&trigger),
            priority: binding.priority(),
            threshold: 0.0,
        };
        let boxed = Box::into_raw(Box::new(BoxedGesture {
            std: self.clone(),
            binding: RefCell::new(binding),
        }));
        // SAFETY: `spec` and the trigger it borrows are live for the call. On success the library adopts `boxed` and frees it through
        // `drop_boxed_gesture`; on failure it has not, and it is reclaimed below.
        let reg = unsafe {
            (self.api().binding_gesture)(
                self.ctx(),
                &spec,
                gesture_tramp::<B>,
                boxed.cast(),
                Some(drop_boxed_gesture::<B>),
            )
        };
        let outcome = self.reg_or_failure(reg);
        if outcome.is_err() {
            // SAFETY: the library did not adopt the binding, so nothing else owns the box.
            drop(unsafe { Box::from_raw(boxed) });
        }
        outcome
    }
}
