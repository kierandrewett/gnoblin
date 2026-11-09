//! Touchpad gestures: the configured `touchpad-gestures` bindings, and the routing of
//! libinput's gesture samples through them.
//!
//! A sample is turned into the router's gesture events by [`gnoblin_input::touchpad`],
//! the router decides whether a binding claims it, and a fired binding runs its command
//! or names its built-in action. Nothing here reads a device: the caller passes the tick,
//! the lock state and the sample. Whether a configured binding can ever receive a sample
//! on real hardware is not proven by these tests; they cover the chain without a device.

use crate::log;
use gnoblin_backend::{RawTouchpadKind, RawTouchpadPhase, RawTouchpadSample};
use gnoblin_config::{Gesture, GestureAction as ConfigAction, GestureDirection, GestureKind as ConfigKind};
use gnoblin_input::GestureAction;
use gnoblin_input::GestureError;
use gnoblin_input::GestureKind;
use gnoblin_input::GestureLock;
use gnoblin_input::GestureRoute;
use gnoblin_input::GestureRouter;
use gnoblin_input::GestureSpec;
use gnoblin_input::PinchDirection;
use gnoblin_input::touchpad::{
    TouchpadKind as InputKind, TouchpadPhase as InputPhase, TouchpadSample as InputSample, TouchpadStream,
};
use gnoblin_types::Tick;

/// A binding whose movement matched, and what it runs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fire {
    /// The binding's declared name, for logs.
    pub(crate) name: String,
    /// The command or built-in action the binding declared.
    pub(crate) action: GestureAction,
}

/// The validated touchpad bindings and the one gesture in flight.
#[derive(Debug, Default)]
pub(crate) struct TouchpadGestures {
    /// `None` until the document declares bindings: a touchpad with none routes nothing.
    router: Option<GestureRouter>,
    stream: TouchpadStream,
}

impl TouchpadGestures {
    /// Builds the bindings from the document's `touchpad-gestures`, or says which entry
    /// is refused. An empty list yields no router, so every gesture is dropped.
    pub(crate) fn from_config(gestures: &[Gesture]) -> Result<Self, String> {
        if gestures.is_empty() {
            return Ok(Self::default());
        }
        let mut bindings = Vec::with_capacity(gestures.len());
        for gesture in gestures {
            let binding = spec_of(gesture)
                .validate()
                .map_err(|error: GestureError| format!("{}: {error}", gesture.name.as_deref().unwrap_or("?")))?;
            bindings.push(binding);
        }
        let router = GestureRouter::new(bindings).map_err(|error| error.to_string())?;
        Ok(Self {
            router: Some(router),
            stream: TouchpadStream::default(),
        })
    }

    /// Routes one libinput sample. Returns the binding it fired, if any; every other
    /// outcome is logged here and dropped.
    pub(crate) fn route(&mut self, tick: Tick, lock: GestureLock, sample: RawTouchpadSample) -> Option<Fire> {
        let Some(router) = self.router.as_mut() else {
            log::debug!(
                area = "touchpad",
                "a touchpad gesture was dropped: no bindings are configured"
            );
            return None;
        };
        let Some(event) = self.stream.feed(tick, input_sample(sample)) else {
            log::debug!(
                area = "touchpad",
                kind = ?sample.kind,
                phase = ?sample.phase,
                "a touchpad sample was dropped: it has no binding kind, or no gesture was begun"
            );
            return None;
        };
        match router.observe(&event, lock) {
            GestureRoute::Fired { binding } => {
                let bound = router.binding(binding)?;
                Some(Fire {
                    name: bound.name().as_str().to_owned(),
                    action: bound.action().clone(),
                })
            }
            GestureRoute::Reserved { .. } => None,
            GestureRoute::MatchedNothing { .. } => {
                log::debug!(
                    area = "touchpad",
                    "a reserved gesture matched no binding's movement; nothing ran"
                );
                None
            }
            GestureRoute::Cancelled { .. } => {
                log::debug!(area = "touchpad", "a gesture was cancelled; nothing ran");
                None
            }
            GestureRoute::Unclaimed => {
                log::debug!(
                    area = "touchpad",
                    kind = ?sample.kind,
                    fingers = sample.fingers,
                    "no touchpad binding claims this gesture; it was dropped"
                );
                None
            }
        }
    }
}

/// Runs a fired binding: a command starts without a shell, a built-in action has no
/// executor yet and so is named in the log and does nothing.
pub(crate) fn run(fire: &Fire) {
    match &fire.action {
        GestureAction::Command(argv) => crate::keyboard::spawn_shortcut_command(&fire.name, argv),
        GestureAction::Action(action) => log::warn!(
            area = "touchpad",
            gesture = fire.name.as_str(),
            action = action.as_str(),
            "the built-in gesture action has no executor yet; nothing ran"
        ),
    }
}

/// The configuration's entry as the router's loose spec, before validation.
fn spec_of(gesture: &Gesture) -> GestureSpec {
    let action = match (&gesture.action, &gesture.command) {
        (Some(action), _) => Some(GestureAction::Action(action_name(*action).to_owned())),
        (None, Some(command)) => Some(GestureAction::Command(command.clone())),
        (None, None) => None,
    };
    GestureSpec {
        name: gesture.name.clone().unwrap_or_default(),
        gesture: gesture.gesture.map(|kind| match kind {
            ConfigKind::Swipe => GestureKind::Swipe,
            ConfigKind::Pinch => GestureKind::Pinch,
        }),
        fingers: gesture.fingers.and_then(|fingers| u8::try_from(fingers).ok()),
        path: gesture.path.as_ref().map(|points| {
            points
                .iter()
                .map(|point| (point.x.unwrap_or(0.0), point.y.unwrap_or(0.0)))
                .collect()
        }),
        direction: gesture.direction.map(|direction| match direction {
            GestureDirection::In => PinchDirection::In,
            GestureDirection::Out => PinchDirection::Out,
        }),
        tolerance: gesture.tolerance,
        threshold: gesture.threshold,
        action,
    }
}

/// The name a built-in action is documented under.
fn action_name(action: ConfigAction) -> &'static str {
    match action {
        ConfigAction::WorkspaceNext => "workspace.next",
        ConfigAction::WorkspacePrevious => "workspace.previous",
        ConfigAction::WindowClose => "window.close",
        ConfigAction::WindowMinimize => "window.minimize",
        ConfigAction::WindowToggleMaximize => "window.toggle-maximize",
    }
}

/// The backend's sample in the adapter's own units.
fn input_sample(sample: RawTouchpadSample) -> InputSample {
    InputSample {
        fingers: sample.fingers,
        phase: match sample.phase {
            RawTouchpadPhase::Begin => InputPhase::Begin,
            RawTouchpadPhase::Update => InputPhase::Update,
            RawTouchpadPhase::End => InputPhase::End,
            RawTouchpadPhase::Cancel => InputPhase::Cancel,
        },
        kind: match sample.kind {
            RawTouchpadKind::Swipe => InputKind::Swipe,
            RawTouchpadKind::Pinch => InputKind::Pinch,
            RawTouchpadKind::Hold => InputKind::Hold,
        },
        dx: sample.dx,
        dy: sample.dy,
        scale: sample.scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::GesturePoint;

    fn gesture(name: &str, kind: ConfigKind, fingers: i64, action: ConfigAction) -> Gesture {
        Gesture {
            name: Some(name.to_owned()),
            gesture: Some(kind),
            fingers: Some(fingers),
            direction: None,
            path: None,
            action: Some(action),
            command: None,
            threshold: None,
            tolerance: None,
        }
    }

    fn swipe_path(name: &str, fingers: i64) -> Gesture {
        Gesture {
            path: Some(vec![
                GesturePoint {
                    x: Some(0.0),
                    y: Some(0.0),
                },
                GesturePoint {
                    x: Some(1.0),
                    y: Some(0.0),
                },
            ]),
            tolerance: Some(0.5),
            ..gesture(name, ConfigKind::Swipe, fingers, ConfigAction::WorkspaceNext)
        }
    }

    fn sample(fingers: u8, phase: RawTouchpadPhase, kind: RawTouchpadKind, dx: f64, scale: f64) -> RawTouchpadSample {
        RawTouchpadSample {
            device: 1,
            phase,
            kind,
            fingers,
            dx,
            dy: 0.0,
            scale,
        }
    }

    /// Feeds a swipe of `fingers` fingers moving right, then its end.
    fn swipe(routes: &mut TouchpadGestures, fingers: u8, lock: GestureLock, cancel: bool) -> Option<Fire> {
        let mut fired = routes.route(
            Tick::new(1),
            lock,
            sample(fingers, RawTouchpadPhase::Begin, RawTouchpadKind::Swipe, 0.0, 1.0),
        );
        for tick in 2..6 {
            fired = fired.or(routes.route(
                Tick::new(tick),
                lock,
                sample(fingers, RawTouchpadPhase::Update, RawTouchpadKind::Swipe, 120.0, 1.0),
            ));
        }
        let phase = if cancel {
            RawTouchpadPhase::Cancel
        } else {
            RawTouchpadPhase::End
        };
        fired.or(routes.route(
            Tick::new(9),
            lock,
            sample(fingers, phase, RawTouchpadKind::Swipe, 0.0, 1.0),
        ))
    }

    fn routes(gestures: &[Gesture]) -> TouchpadGestures {
        TouchpadGestures::from_config(gestures).expect("the test bindings validate")
    }

    #[test]
    fn a_three_finger_swipe_fires_the_configured_binding() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        let fire = swipe(&mut routes, 3, GestureLock::Unlocked, false).expect("the swipe fires");
        assert_eq!(fire.name, "three");
        assert_eq!(fire.action, GestureAction::Action("workspace.next".to_owned()));
    }

    #[test]
    fn a_four_finger_swipe_reaches_its_own_binding_not_the_three_finger_one() {
        let mut routes = routes(&[swipe_path("three", 3), swipe_path("four", 4)]);
        let fire = swipe(&mut routes, 4, GestureLock::Unlocked, false).expect("the swipe fires");
        assert_eq!(fire.name, "four");
    }

    #[test]
    fn a_cancelled_swipe_fires_nothing() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        assert_eq!(swipe(&mut routes, 3, GestureLock::Unlocked, true), None);
    }

    #[test]
    fn a_gesture_while_locked_fires_nothing() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        assert_eq!(swipe(&mut routes, 3, GestureLock::Locked, false), None);
    }

    #[test]
    fn a_gesture_with_no_matching_binding_fires_nothing() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        assert_eq!(swipe(&mut routes, 2, GestureLock::Unlocked, false), None);
    }

    #[test]
    fn a_pinch_fires_only_when_a_pinch_binding_exists() {
        let pinch = Gesture {
            gesture: Some(ConfigKind::Pinch),
            direction: Some(GestureDirection::Out),
            path: None,
            ..gesture("spread", ConfigKind::Pinch, 4, ConfigAction::WindowClose)
        };
        let mut bound = routes(&[pinch]);
        let pinched = |routes: &mut TouchpadGestures| {
            let begin = sample(4, RawTouchpadPhase::Begin, RawTouchpadKind::Pinch, 0.0, 1.0);
            let end = sample(4, RawTouchpadPhase::End, RawTouchpadKind::Pinch, 0.0, 1.5);
            routes.route(Tick::new(1), GestureLock::Unlocked, begin);
            let spread = sample(4, RawTouchpadPhase::Update, RawTouchpadKind::Pinch, 0.0, 1.5);
            routes.route(Tick::new(2), GestureLock::Unlocked, spread);
            routes.route(Tick::new(3), GestureLock::Unlocked, end)
        };
        assert_eq!(pinched(&mut bound).map(|fire| fire.name), Some("spread".to_owned()));

        let mut unbound = routes(&[swipe_path("three", 3)]);
        assert_eq!(pinched(&mut unbound), None);
    }

    #[test]
    fn a_hold_is_dropped_without_reaching_a_binding() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        let hold = sample(3, RawTouchpadPhase::Begin, RawTouchpadKind::Hold, 0.0, 1.0);
        assert_eq!(routes.route(Tick::new(1), GestureLock::Unlocked, hold), None);
    }

    #[test]
    fn no_configured_bindings_drop_every_gesture() {
        let mut routes = routes(&[]);
        assert_eq!(swipe(&mut routes, 3, GestureLock::Unlocked, false), None);
    }

    #[test]
    fn a_refused_binding_names_itself_and_is_not_routed() {
        let bad = Gesture {
            fingers: Some(9),
            ..gesture("nine", ConfigKind::Swipe, 3, ConfigAction::WorkspaceNext)
        };
        let error = TouchpadGestures::from_config(&[bad]).expect_err("nine fingers are refused");
        assert!(error.starts_with("nine:"), "{error}");
    }
}
