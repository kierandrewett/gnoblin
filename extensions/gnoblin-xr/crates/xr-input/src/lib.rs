//! The controller input model: one sample of the controller per frame in, pointer events out.
//!
//! The runtime reports a [`ControllerState`] each frame: the pointing ray and the raw values of the select and
//! secondary inputs and the scroll stick. [`Controller`] turns the sequence of samples into the pointer events the
//! seat needs: a button press or release when an input crosses a threshold, and scroll amounts over time. It has no
//! OpenXR types, no socket and no clock, so every rule here runs under `cargo test` with no headset.
//!
//! The mapping is fixed (design/xr-extension.md, "Input"):
//!
//! | Input             | Pointer event                                                                  |
//! |-------------------|--------------------------------------------------------------------------------|
//! | select            | `BTN_LEFT` (`0x110`), pressed above [`PRESS_THRESHOLD`], released below [`RELEASE_THRESHOLD`] |
//! | secondary         | `BTN_RIGHT` (`0x111`), with the same thresholds                                |
//! | scroll stick      | axis amounts, [`SCROLL_RATE`] units a second at full deflection past [`SCROLL_DEADZONE`] |
//!
//! [`Hands`] is a frame of both controllers with every input the program publishes on a `controller` node, and the
//! [`Arbiter`] turns the right hand into the [`ControllerState`] above while a shell binding's claim
//! (`controller.claimed`, [`Claims`]) keeps an input from reaching the window under the ray. A press waits
//! [`CLAIM_HOLD`] for a claim before the window gets it.

mod hands;

pub use hands::{Arbiter, CLAIM_HOLD, Claims, Hand, HandState, Hands};
use xr_scene::{Pose, Ray};

/// The Linux evdev code for the left button. The select input produces it.
pub const BTN_LEFT: u32 = 0x110;
/// The Linux evdev code for the right button. The secondary input produces it.
pub const BTN_RIGHT: u32 = 0x111;

/// An analog input at or above this value presses its button.
pub const PRESS_THRESHOLD: f32 = 0.75;
/// An analog input at or below this value releases its button. Between the two thresholds the button keeps its state,
/// so a trigger that rests at half pull neither chatters nor presses.
pub const RELEASE_THRESHOLD: f32 = 0.25;

/// A stick deflection (its length, from `0` at rest to `1` fully pushed) up to this is ignored. Deflection beyond it is
/// rescaled so that scrolling starts from zero speed at the edge of the deadzone.
pub const SCROLL_DEADZONE: f32 = 0.15;
/// The scroll speed at full deflection, in axis units a second. A Wayland wheel click is 15 units, so full
/// deflection is about 27 clicks a second.
pub const SCROLL_RATE: f32 = 400.0;
/// The longest frame time, in seconds, that scrolls. A stall (a paused session, a slow frame) does not turn into one
/// huge jump when it ends.
pub const MAX_STEP: f32 = 0.1;

/// One sample of the controller, as the runtime reports it for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ControllerState {
    /// The pointing ray in the reference space the layers are placed in: the controller's aim pose, or the gaze when
    /// no controller is tracked. `None` when the runtime tracks neither.
    pub ray: Option<Ray>,
    /// The select input, `0` to `1`. A button reports `0` or `1`, a trigger its pull.
    pub select: f32,
    /// The secondary input, `0` to `1`, as for `select`.
    pub secondary: f32,
    /// The scroll stick, `x` to the right and `y` up, each `-1` to `1`.
    pub scroll: (f32, f32),
}

/// What a frame's sample means for the seat's pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerEvent {
    /// A button went down or up. `button` is a Linux evdev code.
    Button { button: u32, pressed: bool },
    /// The pointer scrolled by `(dx, dy)` axis units. Positive `dy` is down the page, as on a wheel.
    Scroll { dx: f64, dy: f64 },
}

/// One frame's events, in the order they happened: select edge, secondary edge, then scroll.
pub type PointerEvents = Vec<PointerEvent>;

/// An analog input with hysteresis: it presses at [`PRESS_THRESHOLD`] and releases at [`RELEASE_THRESHOLD`].
#[derive(Debug, Clone, Copy, Default)]
struct Trigger {
    down: bool,
}

impl Trigger {
    /// The edge the new value causes: `Some(true)` for a press, `Some(false)` for a release. A NaN value changes
    /// nothing.
    fn update(&mut self, value: f32) -> Option<bool> {
        if !self.down && value >= PRESS_THRESHOLD {
            self.down = true;
            Some(true)
        } else if self.down && value <= RELEASE_THRESHOLD {
            self.down = false;
            Some(false)
        } else {
            None
        }
    }
}

/// Turns a sequence of controller samples into pointer events.
#[derive(Debug, Clone, Copy, Default)]
pub struct Controller {
    select: Trigger,
    secondary: Trigger,
}

impl Controller {
    /// The events `state` causes, `dt` seconds after the previous sample.
    ///
    /// The first sample of a session passes `0`. A `dt` that is negative, not finite, or longer than [`MAX_STEP`] is
    /// clamped, so a stall never scrolls further than one maximal step.
    pub fn update(&mut self, state: &ControllerState, dt: f32) -> PointerEvents {
        let mut events = PointerEvents::new();
        if let Some(pressed) = self.select.update(state.select) {
            events.push(PointerEvent::Button {
                button: BTN_LEFT,
                pressed,
            });
        }
        if let Some(pressed) = self.secondary.update(state.secondary) {
            events.push(PointerEvent::Button {
                button: BTN_RIGHT,
                pressed,
            });
        }
        let (dx, dy) = scroll_step(state.scroll, dt);
        if dx != 0.0 || dy != 0.0 {
            events.push(PointerEvent::Scroll {
                dx: f64::from(dx),
                dy: f64::from(dy),
            });
        }
        events
    }

    /// Whether a button is down now, by the events [`update`](Self::update) has returned.
    pub fn any_down(&self) -> bool {
        self.select.down || self.secondary.down
    }
}

/// The axis amounts a stick at `stick` scrolls in `dt` seconds. Up on the stick is up the page, which is a negative
/// vertical axis amount.
fn scroll_step(stick: (f32, f32), dt: f32) -> (f32, f32) {
    let dt = if dt.is_finite() { dt.clamp(0.0, MAX_STEP) } else { 0.0 };
    let (x, y) = stick;
    let length = x.hypot(y);
    // A NaN or infinite stick scrolls nothing, and neither does one inside the deadzone.
    let pushed = length.is_finite() && length > SCROLL_DEADZONE;
    if !pushed {
        return (0.0, 0.0);
    }
    let speed = ((length - SCROLL_DEADZONE) / (1.0 - SCROLL_DEADZONE)).min(1.0) * SCROLL_RATE * dt;
    (x / length * speed, -(y / length) * speed)
}

/// The ray the pointer casts: the controller's aim pose when one is tracked, else the head's gaze.
///
/// Both point down their own `-Z` axis. `None` when the runtime locates neither.
pub fn pointing_ray(aim: Option<&Pose>, head: Option<&Pose>) -> Option<Ray> {
    aim.or(head).map(Ray::from_pose)
}
