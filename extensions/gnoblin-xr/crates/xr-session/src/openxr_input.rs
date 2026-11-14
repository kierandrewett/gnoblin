//! The OpenXR actions the controllers are read through: where each hand aims, and its trigger, grip, thumbstick and
//! buttons.
//!
//! One action set, `gnoblin`, holds seven actions, each with the two hands as subaction paths. [`PROFILES`] is the
//! table that binds them on each interaction profile the extension suggests bindings for. The runtime picks the
//! profile that matches the controllers in the user's hands, and remaps by the user's own bindings if they have set
//! any.
//!
//! | Action             | Type    | Input                                                                      |
//! |--------------------|---------|----------------------------------------------------------------------------|
//! | `aim`              | pose    | `aim/pose`                                                                 |
//! | `trigger`          | float   | the trigger (`trigger/value`), or `select/click` on the simple controller  |
//! | `grip`             | float   | the grip (`squeeze/value`, or `squeeze/click` on a Vive wand)              |
//! | `thumbstick`       | vector2 | the thumbstick, or the Vive trackpad                                       |
//! | `thumbstick_click` | boolean | the stick or pad pressed in                                                |
//! | `primary`          | boolean | A on the right hand, X on the left                                         |
//! | `secondary`        | boolean | B on the right hand, Y on the left, or `menu/click` where there are none   |
//!
//! `trigger` and `grip` are floats, not booleans, so the pointer sees the analog pull of a trigger and applies its own
//! press and release thresholds (`xr_input::PRESS_THRESHOLD`). A runtime reports a click input bound to a float action
//! as `0` or `1`. Where one action has two inputs on a profile, the runtime reports the larger.
//!
//! This module holds no `unsafe`: the `openxr` crate wraps every call here safely.

use openxr as xr;
use xr_input::{Hand, HandState, Hands};

use crate::openxr_runtime::{err, from_posef};
use crate::runtime::{Result, RuntimeError};

/// The name of the action set.
pub const ACTION_SET: &str = "gnoblin";

/// The user path of `hand`.
#[must_use]
pub const fn hand_path(hand: Hand) -> &'static str {
    match hand {
        Hand::Left => "/user/hand/left",
        Hand::Right => "/user/hand/right",
    }
}

/// The path of `input` on `hand`: `/user/hand/right/input/trigger/value` for `trigger/value`.
#[must_use]
pub fn input_path(hand: Hand, input: &str) -> String {
    format!("{}/input/{input}", hand_path(hand))
}

/// The pose the pointer ray and the controller node follow.
#[must_use]
pub fn aim_path(hand: Hand) -> String {
    input_path(hand, "aim/pose")
}

/// How one interaction profile binds the actions on one hand. Every entry is a path under `/user/hand/<hand>/input/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandBindings {
    /// Bound to the `trigger` action.
    pub trigger: &'static [&'static str],
    /// Bound to the `grip` action. Empty for a controller with no grip.
    pub grip: &'static [&'static str],
    /// Bound to the `thumbstick` action. Empty for a controller with no stick or pad.
    pub thumbstick: &'static [&'static str],
    /// Bound to the `thumbstick_click` action.
    pub thumbstick_click: &'static [&'static str],
    /// Bound to the `primary` action. Empty for a controller with no lower face button.
    pub primary: &'static [&'static str],
    /// Bound to the `secondary` action.
    pub secondary: &'static [&'static str],
}

/// How one interaction profile binds the actions on each hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// The interaction profile path.
    pub path: &'static str,
    pub left: HandBindings,
    pub right: HandBindings,
}

const SIMPLE: HandBindings = HandBindings {
    trigger: &["select/click"],
    grip: &[],
    thumbstick: &[],
    thumbstick_click: &[],
    primary: &[],
    secondary: &["menu/click"],
};

const TOUCH_RIGHT: HandBindings = HandBindings {
    trigger: &["trigger/value"],
    grip: &["squeeze/value"],
    thumbstick: &["thumbstick"],
    thumbstick_click: &["thumbstick/click"],
    primary: &["a/click"],
    secondary: &["b/click"],
};

/// The Touch left hand has X and Y where the right has A and B.
const TOUCH_LEFT: HandBindings = HandBindings {
    primary: &["x/click"],
    secondary: &["y/click"],
    ..TOUCH_RIGHT
};

const VIVE: HandBindings = HandBindings {
    trigger: &["trigger/value"],
    grip: &["squeeze/click"],
    thumbstick: &["trackpad"],
    thumbstick_click: &["trackpad/click"],
    primary: &[],
    secondary: &["menu/click"],
};

/// The Index controller has A and B on both hands.
const INDEX: HandBindings = HandBindings {
    trigger: &["trigger/value"],
    grip: &["squeeze/value"],
    thumbstick: &["thumbstick"],
    thumbstick_click: &["thumbstick/click"],
    primary: &["a/click"],
    secondary: &["b/click"],
};

/// The interaction profiles the extension suggests bindings for.
pub const PROFILES: &[Profile] = &[
    Profile {
        path: "/interaction_profiles/khr/simple_controller",
        left: SIMPLE,
        right: SIMPLE,
    },
    Profile {
        path: "/interaction_profiles/oculus/touch_controller",
        left: TOUCH_LEFT,
        right: TOUCH_RIGHT,
    },
    Profile {
        path: "/interaction_profiles/htc/vive_controller",
        left: VIVE,
        right: VIVE,
    },
    Profile {
        path: "/interaction_profiles/valve/index_controller",
        left: INDEX,
        right: INDEX,
    },
    Profile {
        path: "/interaction_profiles/meta/touch_controller_plus",
        left: TOUCH_LEFT,
        right: TOUCH_RIGHT,
    },
];

/// One hand: its user path and the space its aim pose is read in.
struct Side {
    hand: Hand,
    path: xr::Path,
    aim_space: xr::Space,
}

/// The attached action set and the aim space of each hand.
pub struct Input {
    set: xr::ActionSet,
    aim: xr::Action<xr::Posef>,
    trigger: xr::Action<f32>,
    grip: xr::Action<f32>,
    thumbstick: xr::Action<xr::Vector2f>,
    thumbstick_click: xr::Action<bool>,
    primary: xr::Action<bool>,
    secondary: xr::Action<bool>,
    sides: [Side; 2],
    /// The interaction profiles the runtime accepted suggested bindings for.
    bound: Vec<&'static str>,
}

impl Input {
    /// Creates the action set and its actions, suggests bindings for each profile in [`PROFILES`], attaches the set to
    /// `session` and makes the aim space of each hand.
    ///
    /// A runtime may refuse a profile it does not support (an extension that is off, a controller it does not know):
    /// that profile is skipped. The call fails only when no profile was accepted, so there is nothing to bind.
    ///
    /// # Errors
    ///
    /// The runtime refuses to create the actions, to attach the set, or to bind any profile.
    pub fn create<G>(instance: &xr::Instance, session: &xr::Session<G>) -> Result<Self> {
        let path = |text: &str| instance.string_to_path(text).map_err(|e| err(text, e));
        let users = [path(hand_path(Hand::Left))?, path(hand_path(Hand::Right))?];

        let set = instance
            .create_action_set(ACTION_SET, "Gnoblin controllers", 0)
            .map_err(|e| err("creating the action set", e))?;
        let aim = set
            .create_action::<xr::Posef>("aim", "Aim", &users)
            .map_err(|e| err("creating the aim action", e))?;
        let trigger = set
            .create_action::<f32>("trigger", "Trigger", &users)
            .map_err(|e| err("creating the trigger action", e))?;
        let grip = set
            .create_action::<f32>("grip", "Grip", &users)
            .map_err(|e| err("creating the grip action", e))?;
        let thumbstick = set
            .create_action::<xr::Vector2f>("thumbstick", "Thumbstick", &users)
            .map_err(|e| err("creating the thumbstick action", e))?;
        let thumbstick_click = set
            .create_action::<bool>("thumbstick_click", "Thumbstick click", &users)
            .map_err(|e| err("creating the thumbstick click action", e))?;
        let primary = set
            .create_action::<bool>("primary", "Primary button", &users)
            .map_err(|e| err("creating the primary action", e))?;
        let secondary = set
            .create_action::<bool>("secondary", "Secondary button", &users)
            .map_err(|e| err("creating the secondary action", e))?;

        let mut bound = Vec::new();
        let mut refused = Vec::new();
        for profile in PROFILES {
            let mut bindings = Vec::new();
            for (hand, hand_bindings) in [(Hand::Left, &profile.left), (Hand::Right, &profile.right)] {
                bindings.push(xr::Binding::new(&aim, path(&aim_path(hand))?));
                for (inputs, bind) in [(hand_bindings.trigger, &trigger), (hand_bindings.grip, &grip)] {
                    for input in inputs {
                        bindings.push(xr::Binding::new(bind, path(&input_path(hand, input))?));
                    }
                }
                for input in hand_bindings.thumbstick {
                    bindings.push(xr::Binding::new(&thumbstick, path(&input_path(hand, input))?));
                }
                for (inputs, bind) in [
                    (hand_bindings.thumbstick_click, &thumbstick_click),
                    (hand_bindings.primary, &primary),
                    (hand_bindings.secondary, &secondary),
                ] {
                    for input in inputs {
                        bindings.push(xr::Binding::new(bind, path(&input_path(hand, input))?));
                    }
                }
            }
            match instance.suggest_interaction_profile_bindings(path(profile.path)?, &bindings) {
                Ok(()) => bound.push(profile.path),
                Err(e) => refused.push(format!("{}: {e:?}", profile.path)),
            }
        }
        if bound.is_empty() {
            return Err(RuntimeError(format!(
                "the runtime accepted no controller bindings ({})",
                refused.join("; ")
            )));
        }

        session
            .attach_action_sets(&[&set])
            .map_err(|e| err("attaching the action set", e))?;
        let side = |hand: Hand, user: xr::Path| -> Result<Side> {
            let aim_space = aim
                .create_space(session, user, xr::Posef::IDENTITY)
                .map_err(|e| err("creating the aim space", e))?;
            Ok(Side {
                hand,
                path: user,
                aim_space,
            })
        };
        let sides = [side(Hand::Left, users[0])?, side(Hand::Right, users[1])?];
        Ok(Self {
            set,
            aim,
            trigger,
            grip,
            thumbstick,
            thumbstick_click,
            primary,
            secondary,
            sides,
            bound,
        })
    }

    /// The interaction profiles the runtime accepted bindings for.
    pub fn bound_profiles(&self) -> &[&'static str] {
        &self.bound
    }

    /// `xrSyncActions` for the set, then the state of each action on each hand, and each aim pose relative to `base`
    /// at `time`. The result has no gaze: the runtime adds the head's when the right hand has no aim pose.
    ///
    /// A hand whose aim action is not active has no controller the runtime reports, so it is `None`. A session that is
    /// not focused syncs successfully with every action inactive, so both hands are.
    ///
    /// # Errors
    ///
    /// The runtime fails the sync, a state read or the locate.
    pub fn poll<G>(&self, session: &xr::Session<G>, base: &xr::Space, time: xr::Time) -> Result<Hands> {
        session
            .sync_actions(&[xr::ActiveActionSet::new(&self.set)])
            .map_err(|e| err("xrSyncActions", e))?;

        let mut hands = Hands::default();
        for side in &self.sides {
            if !self
                .aim
                .is_active(session, side.path)
                .map_err(|e| err("reading the aim state", e))?
            {
                continue;
            }
            let location = side
                .aim_space
                .locate(base, time)
                .map_err(|e| err("locating the aim pose", e))?;
            let valid = xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID;
            let aim = location
                .location_flags
                .contains(valid)
                .then(|| from_posef(&location.pose));

            let stick = self
                .thumbstick
                .state(session, side.path)
                .map_err(|e| err("reading the thumbstick", e))?;
            let state = HandState {
                aim,
                trigger: analog(&self.trigger, session, side.path, "the trigger")?,
                grip: analog(&self.grip, session, side.path, "the grip")?,
                thumbstick: if stick.is_active {
                    (stick.current_state.x, stick.current_state.y)
                } else {
                    (0.0, 0.0)
                },
                thumbstick_click: down(&self.thumbstick_click, session, side.path, "the thumbstick click")?,
                primary: down(&self.primary, session, side.path, "the primary button")?,
                secondary: down(&self.secondary, session, side.path, "the secondary button")?,
            };
            match side.hand {
                Hand::Left => hands.left = Some(state),
                Hand::Right => hands.right = Some(state),
            }
        }
        Ok(hands)
    }
}

/// An analog action's value on `path`, `0` when the action is inactive.
fn analog<G>(action: &xr::Action<f32>, session: &xr::Session<G>, path: xr::Path, what: &str) -> Result<f32> {
    let state = action
        .state(session, path)
        .map_err(|e| err(&format!("reading {what}"), e))?;
    Ok(if state.is_active { state.current_state } else { 0.0 })
}

/// A boolean action's value on `path`, `false` when the action is inactive.
fn down<G>(action: &xr::Action<bool>, session: &xr::Session<G>, path: xr::Path, what: &str) -> Result<bool> {
    let state = action
        .state(session, path)
        .map_err(|e| err(&format!("reading {what}"), e))?;
    Ok(state.is_active && state.current_state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_binding(profile: &Profile) -> impl Iterator<Item = (Hand, &'static str)> + '_ {
        [(Hand::Left, &profile.left), (Hand::Right, &profile.right)]
            .into_iter()
            .flat_map(|(hand, bindings)| {
                [
                    bindings.trigger,
                    bindings.grip,
                    bindings.thumbstick,
                    bindings.thumbstick_click,
                    bindings.primary,
                    bindings.secondary,
                ]
                .into_iter()
                .flatten()
                .map(move |input| (hand, *input))
            })
    }

    #[test]
    fn the_table_covers_the_five_profiles_each_once() {
        let paths: Vec<_> = PROFILES.iter().map(|profile| profile.path).collect();
        assert_eq!(
            paths,
            [
                "/interaction_profiles/khr/simple_controller",
                "/interaction_profiles/oculus/touch_controller",
                "/interaction_profiles/htc/vive_controller",
                "/interaction_profiles/valve/index_controller",
                "/interaction_profiles/meta/touch_controller_plus",
            ]
        );
    }

    #[test]
    fn the_paths_are_built_under_the_hand() {
        assert_eq!(hand_path(Hand::Left), "/user/hand/left");
        assert_eq!(
            input_path(Hand::Right, "trigger/value"),
            "/user/hand/right/input/trigger/value"
        );
        assert_eq!(aim_path(Hand::Right), "/user/hand/right/input/aim/pose");
        assert_eq!(aim_path(Hand::Left), "/user/hand/left/input/aim/pose");
    }

    #[test]
    fn every_profile_binds_a_trigger_and_a_secondary_input_on_each_hand() {
        for profile in PROFILES {
            for bindings in [&profile.left, &profile.right] {
                assert!(!bindings.trigger.is_empty(), "{} binds no trigger", profile.path);
                assert!(!bindings.secondary.is_empty(), "{} binds no secondary", profile.path);
            }
        }
    }

    #[test]
    fn no_input_is_given_a_path_that_names_a_hand_or_the_input_prefix() {
        for profile in PROFILES {
            for (hand, input) in every_binding(profile) {
                assert!(!input.starts_with('/'), "{}: {input} is not relative", profile.path);
                assert!(
                    input_path(hand, input).starts_with(hand_path(hand)),
                    "{}: {input}",
                    profile.path
                );
            }
        }
    }

    #[test]
    fn only_the_simple_controller_has_no_thumbstick_and_no_grip() {
        for profile in PROFILES {
            let simple = profile.path.ends_with("simple_controller");
            for bindings in [&profile.left, &profile.right] {
                assert_eq!(bindings.thumbstick.is_empty(), simple, "{}", profile.path);
                assert_eq!(bindings.grip.is_empty(), simple, "{}", profile.path);
            }
        }
    }

    #[test]
    fn a_trigger_is_bound_by_its_analog_value_not_a_click() {
        // The touch, Vive, Index and touch-plus triggers have no `click` on every runtime; the pointer wants the pull.
        for profile in PROFILES
            .iter()
            .filter(|profile| !profile.path.ends_with("simple_controller"))
        {
            for bindings in [&profile.left, &profile.right] {
                assert_eq!(bindings.trigger, ["trigger/value"], "{}", profile.path);
            }
        }
    }

    #[test]
    fn the_touch_left_hand_has_x_and_y_where_the_right_has_a_and_b() {
        for profile in PROFILES
            .iter()
            .filter(|profile| profile.path.contains("touch_controller"))
        {
            assert_eq!(profile.right.primary, ["a/click"], "{}", profile.path);
            assert_eq!(profile.right.secondary, ["b/click"], "{}", profile.path);
            assert_eq!(profile.left.primary, ["x/click"], "{}", profile.path);
            assert_eq!(profile.left.secondary, ["y/click"], "{}", profile.path);
        }
        let index = PROFILES
            .iter()
            .find(|profile| profile.path.contains("index"))
            .expect("index");
        assert_eq!(
            index.left, index.right,
            "the Index controller has A and B on both hands"
        );
    }

    #[test]
    fn the_pointer_inputs_of_the_right_hand_are_what_they_were_before_the_left_hand() {
        // The secondary input of the pointer was the grip or the upper face button (or menu); both are still bound.
        let touch = PROFILES
            .iter()
            .find(|profile| profile.path.contains("oculus"))
            .expect("touch");
        assert_eq!(touch.right.grip, ["squeeze/value"]);
        assert_eq!(touch.right.secondary, ["b/click"]);
        let vive = PROFILES
            .iter()
            .find(|profile| profile.path.contains("vive"))
            .expect("vive");
        assert_eq!(vive.right.grip, ["squeeze/click"]);
        assert_eq!(vive.right.secondary, ["menu/click"]);
        assert_eq!(vive.right.thumbstick, ["trackpad"]);
    }
}
