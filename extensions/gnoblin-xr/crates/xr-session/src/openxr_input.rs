//! The OpenXR actions the pointer reads: where the right controller aims, and its select, secondary and scroll inputs.
//!
//! One action set, `gnoblin`, holds four actions. [`PROFILES`] is the table that binds them on each interaction
//! profile the extension suggests bindings for. The runtime picks the profile that matches the controller in the
//! user's hand, and remaps by the user's own bindings if they have set any.
//!
//! | Action      | Type    | Input                                                                          |
//! |-------------|---------|--------------------------------------------------------------------------------|
//! | `aim`       | pose    | `/user/hand/right/input/aim/pose`                                              |
//! | `select`    | float   | the trigger (`trigger/value`), or `select/click` on the simple controller      |
//! | `secondary` | float   | the grip (`squeeze`), `b/click` where the controller has one, `menu/click` otherwise |
//! | `scroll`    | vector2 | the thumbstick, or the Vive trackpad                                           |
//!
//! `select` and `secondary` are floats, not booleans, so the pointer sees the analog pull of a trigger and applies its
//! own press and release thresholds (`xr_input::PRESS_THRESHOLD`). A runtime reports a click input bound to a float
//! action as `0` or `1`. Where one action has two inputs on a profile, the runtime reports the larger.
//!
//! This module holds no `unsafe`: the `openxr` crate wraps every call here safely.

use openxr as xr;
use xr_scene::Pose;

use crate::openxr_runtime::{err, from_posef};
use crate::runtime::{Result, RuntimeError};

/// The name of the action set.
pub const ACTION_SET: &str = "gnoblin";

/// The pose the pointer ray follows.
pub const AIM_PATH: &str = "/user/hand/right/input/aim/pose";

/// How one interaction profile binds the pointer's actions. Every path is under `/user/hand/right/input/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// The interaction profile path.
    pub path: &'static str,
    /// Bound to the `select` action.
    pub select: &'static [&'static str],
    /// Bound to the `secondary` action.
    pub secondary: &'static [&'static str],
    /// Bound to the `scroll` action. Empty for a controller with no stick or pad.
    pub scroll: &'static [&'static str],
}

/// The interaction profiles the extension suggests bindings for.
pub const PROFILES: &[Profile] = &[
    Profile {
        path: "/interaction_profiles/khr/simple_controller",
        select: &["/user/hand/right/input/select/click"],
        secondary: &["/user/hand/right/input/menu/click"],
        scroll: &[],
    },
    Profile {
        path: "/interaction_profiles/oculus/touch_controller",
        select: &["/user/hand/right/input/trigger/value"],
        secondary: &["/user/hand/right/input/squeeze/value", "/user/hand/right/input/b/click"],
        scroll: &["/user/hand/right/input/thumbstick"],
    },
    Profile {
        path: "/interaction_profiles/htc/vive_controller",
        select: &["/user/hand/right/input/trigger/value"],
        secondary: &[
            "/user/hand/right/input/squeeze/click",
            "/user/hand/right/input/menu/click",
        ],
        scroll: &["/user/hand/right/input/trackpad"],
    },
    Profile {
        path: "/interaction_profiles/valve/index_controller",
        select: &["/user/hand/right/input/trigger/value"],
        secondary: &["/user/hand/right/input/squeeze/value", "/user/hand/right/input/b/click"],
        scroll: &["/user/hand/right/input/thumbstick"],
    },
    Profile {
        path: "/interaction_profiles/meta/touch_controller_plus",
        select: &["/user/hand/right/input/trigger/value"],
        secondary: &["/user/hand/right/input/squeeze/value", "/user/hand/right/input/b/click"],
        scroll: &["/user/hand/right/input/thumbstick"],
    },
];

/// What one `xrSyncActions` and the reads after it returned.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sample {
    /// The aim pose in the reference space, when the controller is tracked and the runtime reports it valid.
    pub aim: Option<Pose>,
    /// `select`, `0` when the action is inactive.
    pub select: f32,
    /// `secondary`, `0` when the action is inactive.
    pub secondary: f32,
    /// `scroll` as `(x, y)`, `(0, 0)` when the action is inactive.
    pub scroll: (f32, f32),
}

/// The attached action set and the aim space.
pub struct Input {
    set: xr::ActionSet,
    aim: xr::Action<xr::Posef>,
    select: xr::Action<f32>,
    secondary: xr::Action<f32>,
    scroll: xr::Action<xr::Vector2f>,
    aim_space: xr::Space,
    /// The interaction profiles the runtime accepted suggested bindings for.
    bound: Vec<&'static str>,
}

impl Input {
    /// Creates the action set and its actions, suggests bindings for each profile in [`PROFILES`], attaches the set to
    /// `session` and makes the aim space.
    ///
    /// A runtime may refuse a profile it does not support (an extension that is off, a controller it does not know):
    /// that profile is skipped. The call fails only when no profile was accepted, so there is nothing to bind.
    ///
    /// # Errors
    ///
    /// The runtime refuses to create the actions, to attach the set, or to bind any profile.
    pub fn create<G>(instance: &xr::Instance, session: &xr::Session<G>) -> Result<Self> {
        let set = instance
            .create_action_set(ACTION_SET, "Gnoblin pointer", 0)
            .map_err(|e| err("creating the action set", e))?;
        let aim = set
            .create_action::<xr::Posef>("aim", "Aim", &[])
            .map_err(|e| err("creating the aim action", e))?;
        let select = set
            .create_action::<f32>("select", "Select", &[])
            .map_err(|e| err("creating the select action", e))?;
        let secondary = set
            .create_action::<f32>("secondary", "Secondary", &[])
            .map_err(|e| err("creating the secondary action", e))?;
        let scroll = set
            .create_action::<xr::Vector2f>("scroll", "Scroll", &[])
            .map_err(|e| err("creating the scroll action", e))?;

        let path = |text: &str| instance.string_to_path(text).map_err(|e| err(text, e));
        let aim_path = path(AIM_PATH)?;
        let mut bound = Vec::new();
        let mut refused = Vec::new();
        for profile in PROFILES {
            let mut bindings = vec![xr::Binding::new(&aim, aim_path)];
            for text in profile.select {
                bindings.push(xr::Binding::new(&select, path(text)?));
            }
            for text in profile.secondary {
                bindings.push(xr::Binding::new(&secondary, path(text)?));
            }
            for text in profile.scroll {
                bindings.push(xr::Binding::new(&scroll, path(text)?));
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
        let aim_space = aim
            .create_space(session, xr::Path::NULL, xr::Posef::IDENTITY)
            .map_err(|e| err("creating the aim space", e))?;
        Ok(Self {
            set,
            aim,
            select,
            secondary,
            scroll,
            aim_space,
            bound,
        })
    }

    /// The interaction profiles the runtime accepted bindings for.
    pub fn bound_profiles(&self) -> &[&'static str] {
        &self.bound
    }

    /// `xrSyncActions` for the set, then the state of each action, and the aim pose relative to `base` at `time`.
    ///
    /// A session that is not focused syncs successfully with every action inactive, so the sample is the empty one.
    ///
    /// # Errors
    ///
    /// The runtime fails the sync, a state read or the locate.
    pub fn poll<G>(&self, session: &xr::Session<G>, base: &xr::Space, time: xr::Time) -> Result<Sample> {
        session
            .sync_actions(&[xr::ActiveActionSet::new(&self.set)])
            .map_err(|e| err("xrSyncActions", e))?;

        let aim = if self
            .aim
            .is_active(session, xr::Path::NULL)
            .map_err(|e| err("reading the aim state", e))?
        {
            let location = self
                .aim_space
                .locate(base, time)
                .map_err(|e| err("locating the aim pose", e))?;
            let valid = xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID;
            location
                .location_flags
                .contains(valid)
                .then(|| from_posef(&location.pose))
        } else {
            None
        };

        let state = self
            .select
            .state(session, xr::Path::NULL)
            .map_err(|e| err("reading select", e))?;
        let select = if state.is_active { state.current_state } else { 0.0 };
        let state = self
            .secondary
            .state(session, xr::Path::NULL)
            .map_err(|e| err("reading secondary", e))?;
        let secondary = if state.is_active { state.current_state } else { 0.0 };
        let state = self
            .scroll
            .state(session, xr::Path::NULL)
            .map_err(|e| err("reading scroll", e))?;
        let scroll = if state.is_active {
            (state.current_state.x, state.current_state.y)
        } else {
            (0.0, 0.0)
        };
        Ok(Sample {
            aim,
            select,
            secondary,
            scroll,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn every_profile_binds_select_and_secondary_on_the_right_hand() {
        for profile in PROFILES {
            assert!(!profile.select.is_empty(), "{} binds no select", profile.path);
            assert!(!profile.secondary.is_empty(), "{} binds no secondary", profile.path);
            for path in profile.select.iter().chain(profile.secondary).chain(profile.scroll) {
                assert!(
                    path.starts_with("/user/hand/right/input/"),
                    "{}: {path} is not a right-hand input",
                    profile.path
                );
            }
        }
        assert_eq!(AIM_PATH, "/user/hand/right/input/aim/pose");
    }

    #[test]
    fn only_the_simple_controller_has_no_scroll() {
        for profile in PROFILES {
            assert_eq!(
                profile.scroll.is_empty(),
                profile.path.ends_with("simple_controller"),
                "{}",
                profile.path
            );
        }
    }

    #[test]
    fn a_trigger_is_bound_by_its_analog_value_not_a_click() {
        // The touch, Vive, Index and touch-plus triggers have no `click` on every runtime; the pointer wants the pull.
        for profile in PROFILES
            .iter()
            .filter(|profile| !profile.path.ends_with("simple_controller"))
        {
            assert_eq!(
                profile.select,
                ["/user/hand/right/input/trigger/value"],
                "{}",
                profile.path
            );
        }
    }
}
