//! The hands as the runtime reports them, and what a shell binding's claim takes from the pointer.
//!
//! [`Hands`] is one frame of both controllers: the aim pose and every input the program publishes on a `controller`
//! node. The pointer reads only the right hand, and only through the [`Arbiter`]: a shell binding that owns an input
//! (`controller.claimed`, [`Claims`]) keeps it from reaching the window under the ray, so a `Grip+Drag` that moves a
//! panel is not also a right click in it. The arbiter has no OpenXR types, no socket and no clock.

use xr_scene::Pose;

use crate::{ControllerState, RELEASE_THRESHOLD, SCROLL_DEADZONE, pointing_ray};

/// Which hand a controller is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Hand {
    Left,
    Right,
}

impl Hand {
    /// Both hands, left first.
    pub const BOTH: [Self; 2] = [Self::Left, Self::Right];

    /// The value of the `hand` property of the node: `left` or `right`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// One controller as the runtime reports it for a frame. The controller is there: the runtime has an active aim
/// action for the hand. Whether the pose is usable is `aim`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HandState {
    /// The aim pose in the reference space the layers are placed in, when the runtime locates it validly. `None` is a
    /// controller that is on and briefly out of sight: the node keeps its last pose and reads `tracked = false`.
    pub aim: Option<Pose>,
    /// The trigger's pull, `0` to `1`. A click input reports `0` or `1`.
    pub trigger: f32,
    /// The grip's squeeze, `0` to `1`.
    pub grip: f32,
    /// The thumbstick (or trackpad), `x` to the right and `y` up, each `-1` to `1`.
    pub thumbstick: (f32, f32),
    pub thumbstick_click: bool,
    /// The lower face button: A on the right hand, X on the left.
    pub primary: bool,
    /// The upper face button: B or Y. Controllers that have only a menu button report it here.
    pub secondary: bool,
}

/// Both hands for a frame. A hand that is `None` has no controller the runtime reports as active.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Hands {
    pub left: Option<HandState>,
    pub right: Option<HandState>,
    /// The pose the pointer aims from when the right hand has no aim pose: the head's, which a runtime with an input
    /// source reports. `None` when the runtime has no input source at all, as a headless session has not.
    pub gaze: Option<Pose>,
}

impl Hands {
    /// The state of `hand`, if the controller is there.
    #[must_use]
    pub const fn get(&self, hand: Hand) -> Option<&HandState> {
        match hand {
            Hand::Left => self.left.as_ref(),
            Hand::Right => self.right.as_ref(),
        }
    }
}

/// The inputs a shell binding has claimed on a controller (`controller.claimed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Claims {
    pub trigger: bool,
    pub grip: bool,
    pub thumbstick: bool,
    pub primary: bool,
    pub secondary: bool,
}

impl Claims {
    /// The claims the names state: `trigger`, `grip`, `thumbstick`, `primary` and `secondary`. A name that is none of
    /// them claims nothing.
    #[must_use]
    pub fn from_names<S: AsRef<str>>(names: &[S]) -> Self {
        let mut claims = Self::default();
        for name in names {
            match name.as_ref() {
                "trigger" => claims.trigger = true,
                "grip" => claims.grip = true,
                "thumbstick" => claims.thumbstick = true,
                "primary" => claims.primary = true,
                "secondary" => claims.secondary = true,
                _ => {}
            }
        }
        claims
    }
}

/// One input's gate: a claimed input is withheld, and a press that began under the claim stays withheld until the
/// input is released, even if the claim goes first.
#[derive(Debug, Clone, Copy, Default)]
struct Gate {
    latched: bool,
}

impl Gate {
    /// Whether the input is withheld from the pointer. `floor` is the value at or under which it counts as released.
    fn blocked(&mut self, raw: f32, claimed: bool, floor: f32) -> bool {
        let held = raw > floor;
        if claimed {
            self.latched = held;
            return true;
        }
        self.latched &= held;
        self.latched
    }

    /// `raw`, or zero while the input is withheld.
    fn pass(&mut self, raw: f32, claimed: bool, floor: f32) -> f32 {
        if self.blocked(raw, claimed, floor) { 0.0 } else { raw }
    }
}

/// Turns the right hand into the pointer's sample, withholding what a shell binding has claimed.
///
/// The select input is the trigger. The secondary input is the grip or the upper face button, whichever is pressed
/// further; a claimed grip withholds the grip and leaves the button, a claimed `secondary` the reverse. The stick
/// scrolls unless it is claimed. A claimed input is zero for the pointer, so a button that was down when the claim
/// arrived is released to the window by the pointer's own edge detection.
#[derive(Debug, Clone, Copy, Default)]
pub struct Arbiter {
    trigger: Gate,
    grip: Gate,
    secondary: Gate,
    stick: Gate,
}

impl Arbiter {
    /// The pointer's sample for `hands` this frame under `claims`.
    pub fn sample(&mut self, hands: &Hands, claims: Claims) -> ControllerState {
        let right = hands.right.unwrap_or_default();
        let trigger = self.trigger.pass(right.trigger, claims.trigger, RELEASE_THRESHOLD);
        let grip = self.grip.pass(right.grip, claims.grip, RELEASE_THRESHOLD);
        let button = self.secondary.pass(
            if right.secondary { 1.0 } else { 0.0 },
            claims.secondary,
            RELEASE_THRESHOLD,
        );
        // The deflection's length decides whether a claimed stick was held, so a stick that rests is not.
        let (x, y) = right.thumbstick;
        let scroll = if self.stick.blocked(x.hypot(y), claims.thumbstick, SCROLL_DEADZONE) {
            (0.0, 0.0)
        } else {
            (x, y)
        };
        ControllerState {
            ray: pointing_ray(right.aim.as_ref(), hands.gaze.as_ref()),
            select: trigger,
            secondary: grip.max(button),
            scroll,
        }
    }
}
