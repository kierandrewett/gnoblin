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
//! 3. **A key code is shifted on the way through.** `wl_keyboard.key` is the evdev
//!    scan code itself (a client adds eight to get the XKB keycode, the wire does
//!    not), so a key the parent sends is delivered unchanged. An earlier version
//!    subtracted eight and typed the wrong key for every press on a real host:
//!    `a_wayland_key_is_delivered_as_the_evdev_code_it_is`.
//! 4. **A touch that lifts is forgotten before its position is known.** The slot
//!    remembers the output and the last position, so `Up` carries both:
//!    `a_touch_up_keeps_its_output_and_position`.

use std::collections::VecDeque;

use gnoblin_types::{DeviceId, OutputId, Point};

use crate::{
    DeviceCaps, DeviceInfo, DeviceKind, InputEvent, InputSource, RawTouchpadKind, RawTouchpadPhase, RawTouchpadSample,
    TouchPhase,
};

/// The identity the seat gives the parent's keyboard once the parent says it has one.
const KEYBOARD: DeviceId = DeviceId::new(1);
/// The identity of the parent's pointer.
const POINTER: DeviceId = DeviceId::new(2);
/// The identity of the parent's touch device.
const TOUCH: DeviceId = DeviceId::new(3);

/// A whole scroll step in the 1/120 units [`InputEvent::PointerAxis`] states.
///
/// The unit is `REL_WHEEL_HI_RES`'s: one wheel notch is 120, so a high-resolution
/// touchpad scroll is the same unit at a fraction of a step.
pub const AXIS_STEP: i32 = 120;

/// The surface units a wheel notch counts as on the wire.
///
/// `wl_pointer.axis` states smooth scrolling in surface-local units and leaves the
/// size of a notch to convention; ten is the value every compositor that does not
/// send `axis_value120` uses, so a fixed-value axis is converted at that rate. A
/// parent that sends the discrete event never takes this path.
const UNITS_PER_STEP: f64 = 10.0;

/// A gesture the parent has begun and not ended.
#[derive(Clone, Copy, Debug)]
struct OpenGesture {
    kind: RawTouchpadKind,
    fingers: u8,
    /// The rotation the parent last stated, in degrees since the begin, so a sample can carry the change.
    rotation: f64,
}

/// The parent's seat: where the pointer is and what is waiting to be delivered.
///
/// The devices a parent's seat has are announced as the session's own ([`InputEvent::DeviceAdded`]) when the parent
/// states its capabilities, and every event carries the one that produced it. Until a capability is stated, and
/// after it goes, an event is the virtual device's.
#[derive(Debug)]
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
    /// The keyboard's identity while the parent has one.
    keyboard: DeviceId,
    /// The pointer's identity while the parent has one.
    pointer_device: DeviceId,
    /// The touch device's identity while the parent has one.
    touch_device: DeviceId,
    /// The evdev codes of the keys the session saw pressed and not released.
    keys: Vec<u32>,
    /// The codes of the buttons the session saw pressed and not released.
    buttons: Vec<u32>,
    /// The gesture in progress.
    gesture: Option<OpenGesture>,
    /// Whether the parent holds the pointer locked for the session, so its relative motion is the session's motion.
    locked: bool,
    /// The part of the parent's relative motion not yet delivered, because the session counts whole pixels.
    relative_remainder: (f64, f64),
}

impl Default for Seat {
    fn default() -> Self {
        Self::new()
    }
}

impl Seat {
    /// A seat nothing has happened on.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pointer: None,
            touches: Vec::new(),
            queue: VecDeque::new(),
            dropped: 0,
            axis: (0, 0),
            discrete: None,
            keyboard: DeviceId::VIRTUAL,
            pointer_device: DeviceId::VIRTUAL,
            touch_device: DeviceId::VIRTUAL,
            keys: Vec::new(),
            buttons: Vec::new(),
            gesture: None,
            locked: false,
            relative_remainder: (0.0, 0.0),
        }
    }

    /// The parent's seat now has these capabilities: each one that appeared is a device that joins the session's
    /// seat, and each one that went is a device that leaves it, after everything it held was released.
    ///
    /// `gestures` says the parent also delivers swipe and pinch gestures, which the pointer's capabilities then
    /// include.
    pub fn set_capabilities(&mut self, keyboard: bool, pointer: bool, touch: bool, gestures: bool) {
        if keyboard && self.keyboard == DeviceId::VIRTUAL {
            self.keyboard = KEYBOARD;
            self.queue.push_back(InputEvent::DeviceAdded {
                device: DeviceInfo::new(KEYBOARD, "Parent compositor keyboard", DeviceKind::Keyboard),
            });
        } else if !keyboard && self.keyboard != DeviceId::VIRTUAL {
            self.release_keys();
            self.queue.push_back(InputEvent::DeviceRemoved { id: KEYBOARD });
            self.keyboard = DeviceId::VIRTUAL;
        }
        if pointer && self.pointer_device == DeviceId::VIRTUAL {
            self.pointer_device = POINTER;
            let caps = if gestures {
                DeviceKind::Mouse.default_caps().union(DeviceCaps::GESTURE)
            } else {
                DeviceKind::Mouse.default_caps()
            };
            self.queue.push_back(InputEvent::DeviceAdded {
                device: DeviceInfo::new(POINTER, "Parent compositor pointer", DeviceKind::Mouse).with_caps(caps),
            });
        } else if !pointer && self.pointer_device != DeviceId::VIRTUAL {
            for button in std::mem::take(&mut self.buttons) {
                self.queue.push_back(InputEvent::PointerButton {
                    device: POINTER,
                    button,
                    pressed: false,
                });
            }
            if let Some(gesture) = self.gesture {
                self.gesture_end(gesture.kind, true);
            }
            self.leave();
            self.queue.push_back(InputEvent::DeviceRemoved { id: POINTER });
            self.pointer_device = DeviceId::VIRTUAL;
        }
        if touch && self.touch_device == DeviceId::VIRTUAL {
            self.touch_device = TOUCH;
            self.queue.push_back(InputEvent::DeviceAdded {
                device: DeviceInfo::new(TOUCH, "Parent compositor touch", DeviceKind::Touch),
            });
        } else if !touch && self.touch_device != DeviceId::VIRTUAL {
            self.touch_cancel();
            self.queue.push_back(InputEvent::DeviceRemoved { id: TOUCH });
            self.touch_device = DeviceId::VIRTUAL;
        }
    }

    /// The pointer entered `output` at `position`.
    pub fn enter(&mut self, output: OutputId, position: Point) {
        self.pointer = Some(output);
        self.queue.push_back(InputEvent::PointerMotion {
            device: self.pointer_device,
            output,
            position,
        });
    }

    /// The pointer left every output.
    pub fn leave(&mut self) {
        if self.pointer.take().is_some() {
            self.queue.push_back(InputEvent::PointerLeave);
        }
    }

    /// The output the pointer is over, [`None`] while it is over none of the session's windows.
    #[must_use]
    pub fn pointer_output(&self) -> Option<OutputId> {
        self.pointer
    }

    /// The parent now holds the pointer locked for the session, or no longer does.
    ///
    /// While it does, the parent's relative motion is delivered as [`InputEvent::PointerMotionRelative`]. While it
    /// does not, that motion is the same movement the absolute positions already state, and delivering both would
    /// move the pointer twice.
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        self.relative_remainder = (0.0, 0.0);
    }

    /// The parent's pointer moved by `dx`, `dy` surface-local units. Only a locked pointer delivers it.
    ///
    /// The session counts whole logical pixels, so the fraction a slow hand leaves is kept for the next event and
    /// added to it, the way the hardware backend does: dropping it would make a slow pointer stand still.
    pub fn relative_motion(&mut self, dx: f64, dy: f64) {
        // A non-finite delta would poison the remainder for every later event.
        if !self.locked || !dx.is_finite() || !dy.is_finite() {
            return;
        }
        let total = (self.relative_remainder.0 + dx, self.relative_remainder.1 + dy);
        let (whole_x, whole_y) = (total.0.trunc(), total.1.trunc());
        self.relative_remainder = (total.0 - whole_x, total.1 - whole_y);
        // The conversion saturates, so a hostile or absurd delta cannot wrap.
        let (dx, dy) = (whole_x as i32, whole_y as i32);
        if dx != 0 || dy != 0 {
            self.queue.push_back(InputEvent::PointerMotionRelative {
                device: self.pointer_device,
                dx,
                dy,
            });
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
        self.queue.push_back(InputEvent::PointerMotion {
            device: self.pointer_device,
            output,
            position,
        });
    }

    /// A pointer button changed state.
    pub fn button(&mut self, button: u32, pressed: bool) {
        if pressed {
            if !self.buttons.contains(&button) {
                self.buttons.push(button);
            }
        } else {
            self.buttons.retain(|held| *held != button);
        }
        self.queue.push_back(InputEvent::PointerButton {
            device: self.pointer_device,
            button,
            pressed,
        });
    }

    /// A key changed state; `key` is the code the parent sent.
    pub fn key(&mut self, key: u32, pressed: bool) {
        let keycode = key;
        if pressed {
            if !self.keys.contains(&keycode) {
                self.keys.push(keycode);
            }
        } else {
            self.keys.retain(|held| *held != keycode);
        }
        self.queue.push_back(InputEvent::Key {
            device: self.keyboard,
            keycode,
            pressed,
        });
    }

    /// Releases every key the session saw pressed: the parent took the keyboard away from the window, so the
    /// releases will never arrive.
    pub fn release_keys(&mut self) {
        for keycode in std::mem::take(&mut self.keys) {
            self.queue.push_back(InputEvent::Key {
                device: self.keyboard,
                keycode,
                pressed: false,
            });
        }
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
            self.queue.push_back(InputEvent::PointerAxis {
                device: self.pointer_device,
                horizontal,
                vertical,
            });
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
            device: self.touch_device,
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
            device: self.touch_device,
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
            device: self.touch_device,
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
                device: self.touch_device,
                output,
                slot,
                position,
                phase: TouchPhase::Cancel,
            });
        }
    }

    /// A swipe or pinch gesture began with `fingers` fingers.
    pub fn gesture_begin(&mut self, kind: RawTouchpadKind, fingers: u32) {
        let fingers = u8::try_from(fingers).unwrap_or(u8::MAX);
        self.gesture = Some(OpenGesture {
            kind,
            fingers,
            rotation: 0.0,
        });
        self.touchpad(kind, RawTouchpadPhase::Begin, fingers, (0.0, 0.0), 1.0, 0.0);
    }

    /// The gesture moved by `(dx, dy)`. `scale` is the cumulative scale since the begin and `rotation` the cumulative
    /// rotation in degrees, both as the parent states them; the sample carries the rotation since the previous one.
    /// An update for a gesture that is not in progress is dropped.
    pub fn gesture_update(&mut self, kind: RawTouchpadKind, dx: f64, dy: f64, scale: f64, rotation: f64) {
        let Some(gesture) = self.gesture.as_mut().filter(|gesture| gesture.kind == kind) else {
            return;
        };
        let angle = (rotation - gesture.rotation).to_radians();
        gesture.rotation = rotation;
        let fingers = gesture.fingers;
        self.touchpad(kind, RawTouchpadPhase::Update, fingers, (dx, dy), scale, angle);
    }

    /// The gesture ended, or was cancelled.
    pub fn gesture_end(&mut self, kind: RawTouchpadKind, cancelled: bool) {
        let Some(gesture) = self.gesture.take().filter(|gesture| gesture.kind == kind) else {
            return;
        };
        let phase = if cancelled {
            RawTouchpadPhase::Cancel
        } else {
            RawTouchpadPhase::End
        };
        self.touchpad(kind, phase, gesture.fingers, (0.0, 0.0), 1.0, 0.0);
    }

    /// Queues one gesture sample.
    fn touchpad(
        &mut self,
        kind: RawTouchpadKind,
        phase: RawTouchpadPhase,
        fingers: u8,
        (dx, dy): (f64, f64),
        scale: f64,
        angle: f64,
    ) {
        self.queue.push_back(InputEvent::Touchpad(RawTouchpadSample {
            device: self.pointer_device,
            phase,
            kind,
            fingers,
            dx,
            dy,
            scale,
            angle,
        }));
    }

    /// Queues events another translator produced (the parent's tablets), in the order they arrived.
    pub(super) fn extend(&mut self, events: impl IntoIterator<Item = InputEvent>) {
        self.queue.extend(events);
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
                device: DeviceId::VIRTUAL,
                output: OUTPUT,
                position: Point::new(1, 2),
            }),
            "the enter states the position it happened at"
        );
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::PointerMotion {
                device: DeviceId::VIRTUAL,
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
    fn a_wayland_key_is_delivered_as_the_evdev_code_it_is() {
        let mut seat = Seat::new();
        // `KEY_ENTER` is evdev 28, and `wl_keyboard.key` states 28.
        seat.key(28, true);
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Key {
                device: DeviceId::VIRTUAL,
                keycode: 28,
                pressed: true,
            })
        );
        // Nothing is added or taken away: a low code stays the low code it was sent as.
        seat.key(3, true);
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Key {
                device: DeviceId::VIRTUAL,
                keycode: 3,
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
                device: DeviceId::VIRTUAL,
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
                device: DeviceId::VIRTUAL,
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

    /// Everything the seat has queued.
    fn drained(seat: &mut Seat) -> Vec<InputEvent> {
        std::iter::from_fn(|| seat.next_event()).collect()
    }

    #[test]
    fn the_devices_of_the_parents_seat_join_the_session_with_the_capabilities_it_has() {
        let mut seat = Seat::new();
        seat.set_capabilities(true, true, true, true);
        let events = drained(&mut seat);
        let added: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                InputEvent::DeviceAdded { device } => Some((device.kind, device.caps)),
                _ => None,
            })
            .collect();
        assert_eq!(
            added,
            vec![
                (DeviceKind::Keyboard, DeviceCaps::KEYBOARD),
                (DeviceKind::Mouse, DeviceCaps::POINTER.union(DeviceCaps::GESTURE)),
                (DeviceKind::Touch, DeviceCaps::TOUCH),
            ],
            "one device per capability, the pointer with gestures when the parent delivers them"
        );
        seat.set_capabilities(true, true, true, true);
        assert!(
            seat.is_empty(),
            "a capability the seat already has is not announced again"
        );
        seat.key(30, true);
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Key {
                device: KEYBOARD,
                keycode: 30,
                pressed: true
            }),
            "an event names the device that produced it"
        );
    }

    #[test]
    fn a_parent_with_no_gestures_has_a_pointer_without_the_gesture_capability() {
        let mut seat = Seat::new();
        seat.set_capabilities(false, true, false, false);
        assert!(matches!(
            seat.next_event(),
            Some(InputEvent::DeviceAdded { device }) if device.caps == DeviceCaps::POINTER
        ));
    }

    #[test]
    fn losing_the_keyboard_focus_releases_every_key_the_session_saw_pressed() {
        let mut seat = Seat::new();
        seat.set_capabilities(true, false, false, false);
        drained(&mut seat);
        seat.key(29, true);
        seat.key(30, true);
        seat.key(30, false);
        drained(&mut seat);
        seat.release_keys();
        assert_eq!(
            drained(&mut seat),
            vec![InputEvent::Key {
                device: KEYBOARD,
                keycode: 29,
                pressed: false
            }],
            "only the key still held is released, as the key the parent already released needs nothing"
        );
        seat.release_keys();
        assert!(seat.is_empty(), "a key is released once");
    }

    #[test]
    fn a_device_that_goes_releases_what_it_held_before_it_leaves() {
        let mut seat = Seat::new();
        seat.set_capabilities(true, true, true, false);
        drained(&mut seat);
        seat.key(29, true);
        seat.button(0x110, true);
        seat.touch_down(1, Some(OUTPUT), Point::new(4, 5));
        drained(&mut seat);
        seat.set_capabilities(false, false, false, false);
        let events = drained(&mut seat);
        let position_of = |wanted: &InputEvent| events.iter().position(|event| event == wanted);
        let released_key = position_of(&InputEvent::Key {
            device: KEYBOARD,
            keycode: 29,
            pressed: false,
        })
        .expect("the key is released");
        let gone = position_of(&InputEvent::DeviceRemoved { id: KEYBOARD }).expect("the keyboard leaves");
        assert!(
            released_key < gone,
            "the release comes before the device leaves: {events:?}"
        );
        assert!(
            events.contains(&InputEvent::PointerButton {
                device: POINTER,
                button: 0x110,
                pressed: false
            }),
            "the button is released: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                InputEvent::Touch {
                    phase: TouchPhase::Cancel,
                    ..
                }
            )),
            "the touch point is cancelled: {events:?}"
        );
    }

    #[test]
    fn a_pinch_carries_the_scale_and_the_rotation_since_the_previous_sample() {
        let mut seat = Seat::new();
        seat.set_capabilities(false, true, false, true);
        drained(&mut seat);
        seat.gesture_begin(RawTouchpadKind::Pinch, 2);
        seat.gesture_update(RawTouchpadKind::Pinch, 1.0, 2.0, 1.5, 90.0);
        seat.gesture_update(RawTouchpadKind::Pinch, 0.0, 0.0, 2.0, 180.0);
        seat.gesture_end(RawTouchpadKind::Pinch, false);
        let samples: Vec<RawTouchpadSample> = drained(&mut seat)
            .into_iter()
            .filter_map(|event| match event {
                InputEvent::Touchpad(sample) => Some(sample),
                _ => None,
            })
            .collect();
        let phases: Vec<_> = samples.iter().map(|sample| sample.phase).collect();
        assert_eq!(
            phases,
            vec![
                RawTouchpadPhase::Begin,
                RawTouchpadPhase::Update,
                RawTouchpadPhase::Update,
                RawTouchpadPhase::End
            ]
        );
        assert!(
            samples
                .iter()
                .all(|sample| sample.fingers == 2 && sample.device == POINTER)
        );
        assert_eq!(samples[1].scale, 1.5);
        assert!(
            (samples[1].angle - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "a quarter turn"
        );
        assert!(
            (samples[2].angle - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "the second sample carries the turn since the first, not since the begin"
        );
    }

    #[test]
    fn a_gesture_update_with_no_gesture_in_progress_is_dropped() {
        let mut seat = Seat::new();
        seat.gesture_update(RawTouchpadKind::Swipe, 1.0, 1.0, 1.0, 0.0);
        seat.gesture_end(RawTouchpadKind::Swipe, false);
        assert!(seat.is_empty());
        seat.gesture_begin(RawTouchpadKind::Swipe, 3);
        seat.gesture_update(RawTouchpadKind::Pinch, 1.0, 1.0, 1.0, 0.0);
        seat.gesture_end(RawTouchpadKind::Pinch, true);
        assert_eq!(
            drained(&mut seat).len(),
            1,
            "only the begin: a pinch is not the swipe in progress"
        );
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
                device: DeviceId::VIRTUAL,
                output: OUTPUT,
                slot: 1,
                position: Point::new(4, 5),
                phase: TouchPhase::Down,
            })
        );
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Touch {
                device: DeviceId::VIRTUAL,
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
                device: DeviceId::VIRTUAL,
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
                device: DeviceId::VIRTUAL,
                output: OUTPUT,
                slot: 1,
                position: Point::new(1, 1),
                phase: TouchPhase::Cancel,
            })
        );
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::Touch {
                device: DeviceId::VIRTUAL,
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

    #[test]
    fn relative_motion_is_delivered_only_while_the_parent_holds_the_pointer_locked() {
        let mut seat = Seat::new();
        seat.relative_motion(5.0, 3.0);
        assert!(
            seat.next_event().is_none(),
            "an unlocked pointer's motion is the absolute positions' business"
        );
        seat.set_locked(true);
        seat.relative_motion(5.0, 3.0);
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::PointerMotionRelative {
                device: DeviceId::VIRTUAL,
                dx: 5,
                dy: 3
            })
        );
        seat.set_locked(false);
        seat.relative_motion(5.0, 3.0);
        assert!(seat.next_event().is_none(), "an unlock ends the delivery");
    }

    #[test]
    fn a_slow_locked_pointer_still_moves() {
        let mut seat = Seat::new();
        seat.set_locked(true);
        for _ in 0..3 {
            seat.relative_motion(0.4, -0.4);
        }
        // 0.4 + 0.4 + 0.4 is one whole pixel and 0.2 left over, each way.
        assert_eq!(
            seat.next_event(),
            Some(InputEvent::PointerMotionRelative {
                device: DeviceId::VIRTUAL,
                dx: 1,
                dy: -1
            })
        );
        assert!(seat.next_event().is_none());
    }

    #[test]
    fn an_absurd_delta_saturates_instead_of_wrapping() {
        let mut seat = Seat::new();
        seat.set_locked(true);
        seat.relative_motion(f64::NAN, 1.0);
        assert!(
            seat.next_event().is_none(),
            "a delta that is not a number is dropped, not remembered"
        );
        seat.relative_motion(1.0e300, 0.0);
        match seat.next_event() {
            Some(InputEvent::PointerMotionRelative { dx, dy, .. }) => assert_eq!((dx, dy), (i32::MAX, 0)),
            other => panic!("expected a saturated motion, got {other:?}"),
        }
    }
}
