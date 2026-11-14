//! The hands as the runtime reports them, and what a shell binding's claim takes from the pointer.
//!
//! [`Hands`] is one frame of both controllers: the aim pose and every input the program publishes on a `controller`
//! node. The pointer reads only the right hand, and only through the [`Arbiter`]: a shell binding that owns an input
//! (`controller.claimed`, [`Claims`]) keeps it from reaching the window under the ray, so a `Grip+Drag` that moves a
//! panel is not also a right click in it. A binding claims after its chord has settled, so the arbiter holds every press
//! for [`CLAIM_HOLD`] and forwards it only if no claim came. The arbiter has no OpenXR types, no socket and no clock of
//! its own: the frame's display time is its clock.

use xr_scene::{Pose, Ray};

use crate::{ControllerState, PRESS_THRESHOLD, RELEASE_THRESHOLD, SCROLL_DEADZONE, pointing_ray};

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

/// How long a press of the trigger, the grip, the secondary button or the stick is held back from the window before
/// it is forwarded, in nanoseconds (150 ms).
///
/// A shell binding claims an input only after its chord has settled (`session.chordSettleMs`, at most 60 ms) and the
/// claim has travelled shell, compositor and program: the real stock shell on a headless compositor made a claim
/// visible 106 to 125 ms after the press was written (`crates/gnoblin/tests/js_shell.rs`, with 25 ms of polling in
/// that). The program holds a press for longer than that, so a press the shell is going to claim is gone before the
/// window sees it. A press that is not claimed in time is forwarded at the deadline, whether the shell is slow, dead or
/// has no binding for it: the program never waits for the shell, and nothing but the pointer's own events is delayed.
pub const CLAIM_HOLD: i64 = 150_000_000;

/// What a gate gives the pointer for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Gated {
    /// Nothing: the input is idle, held back or withheld.
    Zero,
    /// The raw value, because the window has the press (or this is the frame it is let go).
    Raw,
    /// The press, as a full value, at the pointing ray the hand had when the input went down. The press may be
    /// forwarded well after the hand moved, so it lands where it was aimed.
    Press(Option<Ray>),
}

/// Where an input is between the hand and the window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Phase {
    /// Not down, or down in the band that is not yet a press.
    #[default]
    Idle,
    /// Down since `since`, unclaimed so far and not yet forwarded. `released` is set when it came back up inside the
    /// hold: a tap, which the window gets as a press and a release when the hold is over.
    Held {
        since: i64,
        ray: Option<Ray>,
        released: bool,
    },
    /// The press of a tap went out this frame and its release follows on the next. `then` is a second press that came
    /// down while the first still waited, and is held from `since` on.
    Tap { then: Option<(i64, Option<Ray>)> },
    /// The window has the press and gets the raw values until the input is let go or claimed.
    Passing,
    /// A press that began under a claim, or was taken by one: it stays withheld until the input is let go, even if the
    /// claim goes first.
    Withheld,
}

/// One input's gate. It decides from the raw value, the claim and the clock; it has no other input.
#[derive(Debug, Clone, Copy, Default)]
struct Gate {
    phase: Phase,
}

/// The value at which an input counts as pressed and the value at or under which it counts as released.
#[derive(Debug, Clone, Copy)]
struct Thresholds {
    press: f32,
    release: f32,
    /// Whether a press that is released inside the hold is replayed to the window (a button's click). A stick that
    /// comes back inside the hold scrolls nothing.
    replay: bool,
}

const BUTTON: Thresholds = Thresholds {
    press: PRESS_THRESHOLD,
    release: RELEASE_THRESHOLD,
    replay: true,
};

const STICK: Thresholds = Thresholds {
    press: SCROLL_DEADZONE,
    release: SCROLL_DEADZONE,
    replay: false,
};

impl Gate {
    /// Moves the gate to `now` with the input at `raw`, `claimed` or not, and says what the window gets. `ray` is the
    /// pointing ray of this frame.
    fn step(&mut self, raw: f32, claimed: bool, at: Thresholds, now: i64, ray: Option<Ray>) -> Gated {
        let down = raw > at.release;
        let pressing = raw >= at.press && down;
        match self.phase {
            Phase::Idle => {
                if claimed {
                    if pressing {
                        self.phase = Phase::Withheld;
                    }
                } else if pressing {
                    self.phase = Phase::Held {
                        since: now,
                        ray,
                        released: false,
                    };
                }
                Gated::Zero
            }
            Phase::Withheld => {
                if !down {
                    self.phase = Phase::Idle;
                }
                Gated::Zero
            }
            Phase::Passing => {
                if claimed {
                    // A press the window already has is let go, so that no button stays down in it.
                    self.phase = if down { Phase::Withheld } else { Phase::Idle };
                    Gated::Zero
                } else {
                    if !down {
                        self.phase = Phase::Idle;
                    }
                    Gated::Raw
                }
            }
            Phase::Tap { then } => {
                self.phase = then.map_or(Phase::Idle, |(since, ray)| Phase::Held {
                    since,
                    ray,
                    released: false,
                });
                Gated::Zero
            }
            Phase::Held {
                since,
                ray: pinned,
                released,
            } => {
                if claimed {
                    // The shell took it inside the hold: the window never has it.
                    self.phase = if down { Phase::Withheld } else { Phase::Idle };
                    return Gated::Zero;
                }
                let due = now.saturating_sub(since) >= CLAIM_HOLD;
                if released && pressing {
                    // Pressed again before the first press was let through: the first goes out now and in order.
                    self.phase = Phase::Tap { then: Some((now, ray)) };
                    return Gated::Press(pinned);
                }
                if !released && !down {
                    if !at.replay {
                        self.phase = Phase::Idle;
                        return Gated::Zero;
                    }
                    self.phase = Phase::Held {
                        since,
                        ray: pinned,
                        released: true,
                    };
                }
                if !due {
                    return Gated::Zero;
                }
                if released || !down {
                    self.phase = Phase::Tap { then: None };
                } else {
                    self.phase = Phase::Passing;
                }
                Gated::Press(pinned)
            }
        }
    }

    /// The input's source is gone (the hand was lost): nothing is held, and nothing that was held is clicked.
    fn lose(&mut self) {
        self.phase = Phase::Idle;
    }
}

/// Turns the right hand into the pointer's sample, withholding what a shell binding has claimed.
///
/// The select input is the trigger. The secondary input is the grip or the upper face button, whichever is pressed
/// further; a claimed grip withholds the grip and leaves the button, a claimed `secondary` the reverse. The stick
/// scrolls unless it is claimed.
///
/// Every press waits [`CLAIM_HOLD`] before the window gets it, and a claim that arrives in that time takes it: the
/// window never sees an input the shell claims. A press that is let go inside the hold reaches the window as a press
/// and a release one frame apart when the hold is over, at the ray the hand had when it went down. A press that the
/// window already has when a claim arrives (the claim was later than the hold) is let go by zeroing the input, so the
/// pointer's own edge detection releases it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Arbiter {
    trigger: Gate,
    grip: Gate,
    secondary: Gate,
    stick: Gate,
}

impl Arbiter {
    /// The pointer's sample for `hands` this frame under `claims`, at display time `now` in nanoseconds.
    pub fn sample(&mut self, hands: &Hands, claims: Claims, now: i64) -> ControllerState {
        let ray = pointing_ray(hands.right.and_then(|right| right.aim).as_ref(), hands.gaze.as_ref());
        let Some(right) = hands.right else {
            for gate in [&mut self.trigger, &mut self.grip, &mut self.secondary, &mut self.stick] {
                gate.lose();
            }
            return ControllerState {
                ray,
                ..ControllerState::default()
            };
        };
        let trigger = self.trigger.step(right.trigger, claims.trigger, BUTTON, now, ray);
        let grip = self.grip.step(right.grip, claims.grip, BUTTON, now, ray);
        let face = if right.secondary { 1.0 } else { 0.0 };
        let button = self.secondary.step(face, claims.secondary, BUTTON, now, ray);
        // The deflection's length decides whether a stick was held, so a stick that rests is not.
        let (x, y) = right.thumbstick;
        let stick = self.stick.step(x.hypot(y), claims.thumbstick, STICK, now, ray);
        let analog = |gated: Gated, raw: f32| match gated {
            Gated::Zero => 0.0,
            Gated::Raw => raw,
            Gated::Press(_) => 1.0,
        };
        // A press that was let through late is aimed where the hand was when it went down.
        let aimed = [trigger, grip, button]
            .into_iter()
            .find_map(|gated| match gated {
                Gated::Press(Some(pinned)) => Some(pinned),
                _ => None,
            })
            .or(ray);
        ControllerState {
            ray: aimed,
            select: analog(trigger, right.trigger),
            secondary: analog(grip, right.grip).max(analog(button, face)),
            scroll: if stick == Gated::Zero { (0.0, 0.0) } else { (x, y) },
        }
    }
}
