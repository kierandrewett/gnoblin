//! The parent's seat, converted into the events this backend delivers.
//!
//! Everything here is arithmetic and a queue: a Wayland event arrives, becomes an
//! [`InputEvent`], and waits for the next [`crate::Backend::dispatch`]. Keeping the
//! conversion out of the `Dispatch` implementations is what makes the awkward cases
//! testable without a socket — an axis event that arrives with and without its
//! discrete twin, a key code that is not a key, a touch that moves after the parent
//! took its surface away.
//!
//! # Failure modes
//!
//! 1. **Input is attributed to an output that does not exist yet.** The pointer only
//!    has an output once the parent has said which surface it entered and that
//!    surface has a size, so a motion before the first configure is counted and
//!    dropped rather than delivered against a made-up output:
//!    `a_motion_without_an_output_is_dropped`.
//! 2. **A scroll arrives twice.** `wl_pointer.axis` and `wl_pointer.axis_value120`
//!    describe the same movement, so a frame that carries both is delivered once,
//!    from the exact value: `a_frame_with_both_axis_events_scrolls_once`.
//! 3. **A Wayland key code is used as a scan code.** Wayland states evdev codes
//!    offset by eight, so a key the parent sends is delivered as the evdev code the
//!    seat translates: `a_wayland_key_is_delivered_as_an_evdev_code`.
//! 4. **A touch that lifts is forgotten before its position is known.** The slot
//!    remembers the output and the last position, so `Up` carries both:
//!    `a_touch_up_keeps_its_output_and_position`.

use std::collections::VecDeque;

use gnoblin_types::{OutputId, Point};

use crate::{InputEvent, InputSource, TouchPhase};

/// A whole scroll step in the 1/120 units [`InputEvent::PointerAxis`] states.
///
/// The unit is `REL_WHEEL_HI_RES`'s: one wheel notch is 120, so a high-resolution
/// touchpad scroll is the same unit at a fraction of a step.
pub const AXIS_STEP: i32 = 120;

/// The offset between a Wayland key code and an evdev scan code.
///
/// Wayland's `wl_keyboard.key` is the Linux input event code plus eight, because
/// the protocol reserved the low eight codes for buttons it never uses. The seat
/// below the backend translates evdev scan codes, so the offset is removed here and
/// nowhere else.
const KEY_OFFSET: u32 = 8;

/// The surface units a wheel notch counts as on the wire.
///
/// `wl_pointer.axis` states smooth scrolling in surface-local units and leaves the
/// size of a notch to convention; ten is the value every compositor that does not
/// send `axis_value120` uses, so a fixed-value axis is converted at that rate. A
/// parent that sends the discrete event never takes this path.
const UNITS_PER_STEP: f64 = 10.0;

/// The parent's seat: where the pointer is and what is waiting to be delivered.
#[derive(Debug, Default)]
pub struct Seat {
    /// The output the pointer is over, which is [`None`] until the parent says the
    /// pointer entered a surface this backend draws on.
    pointer: Option<OutputId>,
    /// The touch points that are down: their identity, the output they started on
    /// and the position they were last at.
    touches: Vec<(u32, OutputId, Point)>,
    /// Events waiting for the next dispatch.
    queue: VecDeque<InputEvent>,
    /// Events that could not be attributed to an output.
    dropped: u64,
    /// Scroll accumulated for the current frame, in 1/120 steps.
    axis: (i32, i32),
    /// The exact scroll the parent sent for this frame, if it sent one.
    discrete: Option<(i32, i32)>,
}

impl Seat {
    /// A seat nothing has happened on.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The pointer entered `output` at `position`.
    pub fn enter(&mut self, output: OutputId, position: Point) {
        self.pointer = Some(output);
        self.queue.push_back(InputEvent::PointerMotion { output, position });
    }

    /// The pointer left every output.
    pub fn leave(&mut self) {
        if self.pointer.take().is_some() {
            self.queue.push_back(InputEvent::PointerLeave);
        }
    }

    /// The pointer moved to `position` on the output it is over.
    ///
    /// A motion with no output is dropped: before the parent has said which surface
    /// the pointer is on there is no output-local position to state.
    pub fn motion(&mut self, position: Point) {
        let Some(output) = self.pointer else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        self.queue.push_back(InputEvent::PointerMotion { output, position });
    }

    /// A pointer button changed state.
    pub fn button(&mut self, button: u32, pressed: bool) {
        self.queue.push_back(InputEvent::PointerButton { button, pressed });
    }

    /// A key changed state; `key` is the code the parent sent.
    pub fn key(&mut self, key: u32, pressed: bool) {
        self.queue.push_back(InputEvent::Key {
            keycode: scan_code(key),
            pressed,
        });
    }

    /// A smooth scroll for the current frame, in surface units.
    ///
    /// Superseded by an exact value for the same axis in the same frame: see
    /// [`Seat::axis_discrete`].
    pub fn axis(&mut self, horizontal: bool, value: f64) {
        let steps = steps_from_units(value);
        let slot = if horizontal { &mut self.axis.0 } else { &mut self.axis.1 };
        *slot = slot.saturating_add(steps);
    }

    /// The exact scroll for the current frame, in 1/120 steps.
    pub fn axis_discrete(&mut self, horizontal: bool, steps: i32) {
        let (horizontal_steps, vertical_steps) = self.discrete.unwrap_or((0, 0));
        self.discrete = Some(if horizontal {
            (horizontal_steps.saturating_add(steps), vertical_steps)
        } else {
            (horizontal_steps, vertical_steps.saturating_add(steps))
        });
    }

    /// Ends the frame: what the parent sent for this frame becomes one event.
    pub fn frame(&mut self) {
        let (horizontal, vertical) = self
            .discrete
            .take()
            .or_else(|| (self.axis != (0, 0)).then_some(self.axis))
            .unwrap_or((0, 0));
        self.axis = (0, 0);
        if horizontal != 0 || vertical != 0 {
            self.queue.push_back(InputEvent::PointerAxis { horizontal, vertical });
        }
    }

    /// A touch point went down on `output`.
    ///
    /// A `down` for a surface that is not a configured output of this backend is
    /// counted and dropped, for the same reason a motion with no output is.
    pub fn touch_down(&mut self, slot: u32, output: Option<OutputId>, position: Point) {
        let Some(output) = output else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        self.touches.retain(|(existing, _, _)| *existing != slot);
        self.touches.push((slot, output, position));
        self.queue.push_back(InputEvent::Touch {
            output,
            slot,
            position,
            phase: TouchPhase::Down,
        });
    }

    /// A touch point moved.
    ///
    /// A motion for a slot that is not down is dropped: the parent states the
    /// surface only on `down`, so there is nothing to attribute it to.
    pub fn touch_motion(&mut self, slot: u32, position: Point) {
        let Some(entry) = self.touches.iter_mut().find(|(existing, _, _)| *existing == slot) else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        entry.2 = position;
        let output = entry.1;
        self.queue.push_back(InputEvent::Touch {
            output,
            slot,
            position,
            phase: TouchPhase::Motion,
        });
    }

    /// A touch point lifted, at the position it was last at.
    pub fn touch_up(&mut self, slot: u32) {
        let Some(index) = self.touches.iter().position(|(existing, _, _)| *existing == slot) else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        let (_, output, position) = self.touches.remove(index);
        self.queue.push_back(InputEvent::Touch {
            output,
            slot,
            position,
            phase: TouchPhase::Up,
        });
    }

    /// The parent or the device cancelled every touch point.
    pub fn touch_cancel(&mut self) {
        for (slot, output, position) in std::mem::take(&mut self.touches) {
            self.queue.push_back(InputEvent::Touch {
                output,
                slot,
                position,
                phase: TouchPhase::Cancel,
            });
        }
    }

    /// How many events could not be attributed to an output.
    ///
    /// It is on the contract's side of the wire so a session can tell "the parent
    /// sent nothing" from "the parent sent input this backend refused".
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Whether nothing is waiting.
    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl InputSource for Seat {
    fn next_event(&mut self) -> Option<InputEvent> {
        self.queue.pop_front()
    }
}

/// An evdev scan code for a Wayland key code.
fn scan_code(key: u32) -> u32 {
    key.saturating_sub(KEY_OFFSET)
}

/// A surface-local Wayland coordinate as the integer point the contract states.
///
/// A non-finite coordinate becomes zero rather than saturating to the smallest
/// point: a parent that states no position has stated the origin.
pub(crate) fn position(x: f64, y: f64) -> Point {
    Point::new(axis_coordinate(x), axis_coordinate(y))
}

/// One axis of [`position`].
fn axis_coordinate(value: f64) -> i32 {
    if value.is_finite() {
        value.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    } else {
        0
    }
}

/// Steps in 1/120 units for a `wl_pointer.axis` value.
fn steps_from_units(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let steps = (value / UNITS_PER_STEP) * f64::from(AXIS_STEP);
    steps.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: OutputId = OutputId::new(3);

    #[test]
    fn a_motion_without_an_output_is_dropped() {
        let mut seat = Seat::new();
        seat.motion(Point::new(10, 20));
        assert!(seat.is_empty(), "nothing is delivered before there is an output");
        assert_eq!(seat.dropped(), 1, "the refusal is counted, not silent");
        assert_eq!(seat.next_event(), None);

        seat.enter(OUTPUT, Point::new(1, 2));
        seat.motion(Point::new(10, 20));
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::PointerMotion {
                output: OUTPUT,
                position: Point::new(1, 2),
            }),
            "the enter states the position it happened at"
        );
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::PointerMotion {
                output: OUTPUT,
                position: Point::new(10, 20),
            }),
            "and the motion that follows is on that output"
        );
    }

    #[test]
    fn a_leave_is_sent_once_for_one_pointer() {
        let mut seat = Seat::new();
        seat.leave();
        assert!(
            seat.is_empty(),
            "a pointer that was never on an output cannot leave one"
        );
        seat.enter(OUTPUT, Point::new(0, 0));
        let _ = seat.next_event();
        seat.leave();
        seat.leave();
        assert_eq!(seat.next_event(), Some(InputEvent::PointerLeave));
        assert_eq!(seat.next_event(), None, "the second leave is not a second departure");
    }

    #[test]
    fn a_wayland_key_is_delivered_as_an_evdev_code() {
        let mut seat = Seat::new();
        // `KEY_ENTER` is evdev 28, so the parent sends 36.
        seat.key(36, true);
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Key {
                keycode: 28,
                pressed: true,
            })
        );
        // A code below the offset has no scan code rather than a wrapped one.
        seat.key(3, true);
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Key {
                keycode: 0,
                pressed: true,
            })
        );
    }

    #[test]
    fn a_frame_with_both_axis_events_scrolls_once() {
        let mut seat = Seat::new();
        // A parent that sends both: the fixed value would be 120 as well.
        seat.axis(false, 10.0);
        seat.axis_discrete(false, 240);
        seat.frame();
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::PointerAxis {
                horizontal: 0,
                vertical: 240,
            }),
            "the exact value wins, and only once"
        );
        assert_eq!(seat.next_event(), None);
    }

    #[test]
    fn a_fixed_axis_value_is_a_hundred_and_twenty_per_notch() {
        let mut seat = Seat::new();
        seat.axis(false, 10.0);
        seat.axis(false, 5.0);
        seat.frame();
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::PointerAxis {
                horizontal: 0,
                vertical: 180,
            }),
            "a notch and a half"
        );
        seat.frame();
        assert_eq!(seat.next_event(), None, "an empty frame is not a scroll of zero");
    }

    #[test]
    fn a_scroll_with_no_movement_is_not_delivered() {
        let mut seat = Seat::new();
        seat.axis(true, 0.0);
        seat.axis_discrete(false, 0);
        seat.frame();
        assert!(seat.is_empty());
        seat.axis(true, f64::NAN);
        seat.frame();
        assert!(seat.is_empty(), "a non-finite scroll has no direction");
    }

    #[test]
    fn a_touch_motion_without_a_down_is_dropped() {
        let mut seat = Seat::new();
        seat.touch_motion(1, Point::new(4, 5));
        assert!(seat.is_empty());
        assert_eq!(seat.dropped(), 1);

        seat.touch_down(1, Some(OUTPUT), Point::new(4, 5));
        seat.touch_motion(1, Point::new(6, 7));
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Touch {
                output: OUTPUT,
                slot: 1,
                position: Point::new(4, 5),
                phase: TouchPhase::Down,
            })
        );
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Touch {
                output: OUTPUT,
                slot: 1,
                position: Point::new(6, 7),
                phase: TouchPhase::Motion,
            })
        );
    }

    #[test]
    fn a_touch_on_a_surface_that_is_not_an_output_is_dropped() {
        let mut seat = Seat::new();
        seat.touch_down(1, None, Point::new(4, 5));
        assert!(seat.is_empty(), "no output means no point to state");
        assert_eq!(seat.dropped(), 1);
        seat.touch_up(1);
        assert_eq!(seat.dropped(), 2, "and its lift has nothing to lift");
    }

    #[test]
    fn a_touch_up_keeps_its_output_and_position() {
        let mut seat = Seat::new();
        seat.touch_down(2, Some(OUTPUT), Point::new(4, 5));
        seat.touch_motion(2, Point::new(9, 9));
        let _ = seat.next_event();
        let _ = seat.next_event();
        seat.touch_up(2);
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Touch {
                output: OUTPUT,
                slot: 2,
                position: Point::new(9, 9),
                phase: TouchPhase::Up,
            })
        );
        // A second up for the same slot is not a second point.
        seat.touch_up(2);
        assert_eq!(seat.next_event(), None);
    }

    #[test]
    fn a_cancelled_gesture_reports_every_point() {
        let mut seat = Seat::new();
        seat.touch_down(1, Some(OUTPUT), Point::new(1, 1));
        seat.touch_down(2, Some(OutputId::new(4)), Point::new(2, 2));
        let _ = seat.next_event();
        let _ = seat.next_event();
        seat.touch_cancel();
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Touch {
                output: OUTPUT,
                slot: 1,
                position: Point::new(1, 1),
                phase: TouchPhase::Cancel,
            })
        );
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Touch {
                output: OutputId::new(4),
                slot: 2,
                position: Point::new(2, 2),
                phase: TouchPhase::Cancel,
            })
        );
        assert_eq!(seat.next_event(), None);
    }

    #[test]
    fn a_non_finite_position_is_the_origin() {
        assert_eq!(position(f64::NAN, f64::INFINITY), Point::new(0, 0));
        assert_eq!(position(-0.4, 2.6), Point::new(0, 3));
    }
}
