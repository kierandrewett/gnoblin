//! Touchpad gestures: the gestures a shell has claimed, the configured `touchpad-gestures`
//! bindings, and the routing of libinput's gesture samples through them.
//!
//! A sample is turned into the router's gesture events by [`gnoblin_input::touchpad`]. A gesture
//! a shell claimed goes to that shell from its begin to its end, and to nothing else; every other
//! gesture goes to the bindings, and a fired binding runs its command or names its built-in
//! action. Nothing here reads a device: the caller passes the tick, the lock state and the
//! sample. Whether a configured binding can ever receive a sample on real hardware is not proven
//! by these tests; they cover the chain without a device.

use crate::log;
use crate::state::State;
use gnoblin_backend::{RawTouchpadKind, RawTouchpadPhase, RawTouchpadSample};
use gnoblin_config::{Gesture, GestureAction as ConfigAction, GestureDirection, GestureKind as ConfigKind};
use gnoblin_input::ClaimRoute;
use gnoblin_input::GestureAction;
use gnoblin_input::GestureClaims;
use gnoblin_input::GestureDelivery;
use gnoblin_input::GestureError;
use gnoblin_input::GestureEvent;
use gnoblin_input::GestureKind;
use gnoblin_input::GestureLock;
use gnoblin_input::GestureRoute;
use gnoblin_input::GestureRouter;
use gnoblin_input::GestureSpec;
use gnoblin_input::PinchDirection;
use gnoblin_input::touchpad::{
    TouchpadKind as InputKind, TouchpadPhase as InputPhase, TouchpadSample as InputSample, TouchpadStream,
};
use gnoblin_render::transaction::TreeCall;
use gnoblin_types::Tick;

/// A binding whose movement matched, and what it runs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fire {
    /// The binding's declared name, for logs.
    pub(crate) name: String,
    /// The command or built-in action the binding declared.
    pub(crate) action: GestureAction,
}

/// What the touchpad router did with one sample.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Routed {
    /// A binding whose movement matched: run it.
    Fire(Fire),
    /// A claimed gesture advanced: tell its holder.
    Claimed(GestureDelivery),
    /// The sample went nowhere: logged and dropped here.
    Dropped,
}

/// The gestures a shell claimed, the validated touchpad bindings and the one gesture in flight.
#[derive(Debug, Default)]
pub(crate) struct TouchpadGestures {
    /// `None` until the document declares bindings: a touchpad with none routes nothing.
    router: Option<GestureRouter>,
    stream: TouchpadStream,
    claims: GestureClaims,
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
            claims: GestureClaims::new(),
        })
    }

    /// The gestures a shell has claimed, which take precedence over every binding.
    pub(crate) fn claims_mut(&mut self) -> &mut GestureClaims {
        &mut self.claims
    }

    /// Routes one libinput sample. A claimed gesture is delivered to its holder and never reaches
    /// a binding; any other returns the binding it fired, if any, and every other outcome is
    /// logged here and dropped.
    pub(crate) fn route(&mut self, tick: Tick, lock: GestureLock, sample: RawTouchpadSample) -> Routed {
        let Some(event) = self.stream.feed(tick, input_sample(sample)) else {
            log::debug!(
                area = "touchpad",
                kind = ?sample.kind,
                phase = ?sample.phase,
                "a touchpad sample was dropped: it has no binding kind, or no gesture was begun"
            );
            return Routed::Dropped;
        };
        match self.claims.observe(&event, lock) {
            ClaimRoute::Deliver(delivery) => return Routed::Claimed(delivery),
            ClaimRoute::Withheld => {
                log::debug!(
                    area = "touchpad",
                    "a claimed gesture's holder is gone or the session is locked; the sample was withheld"
                );
                return Routed::Dropped;
            }
            ClaimRoute::Unclaimed => {}
        }
        self.route_to_bindings(&event, lock, sample)
    }

    /// Routes a gesture nobody claimed through the configured bindings.
    fn route_to_bindings(&mut self, event: &GestureEvent, lock: GestureLock, sample: RawTouchpadSample) -> Routed {
        let Some(router) = self.router.as_mut() else {
            log::debug!(
                area = "touchpad",
                "a touchpad gesture was dropped: no bindings are configured"
            );
            return Routed::Dropped;
        };
        match router.observe(event, lock) {
            GestureRoute::Fired { binding } => {
                let Some(bound) = router.binding(binding) else {
                    return Routed::Dropped;
                };
                Routed::Fire(Fire {
                    name: bound.name().as_str().to_owned(),
                    action: bound.action().clone(),
                })
            }
            GestureRoute::Reserved { .. } => Routed::Dropped,
            GestureRoute::MatchedNothing { .. } => {
                log::debug!(
                    area = "touchpad",
                    "a reserved gesture matched no binding's movement; nothing ran"
                );
                Routed::Dropped
            }
            GestureRoute::Cancelled { .. } => {
                log::debug!(area = "touchpad", "a gesture was cancelled; nothing ran");
                Routed::Dropped
            }
            GestureRoute::Unclaimed => {
                log::debug!(
                    area = "touchpad",
                    kind = ?sample.kind,
                    fingers = sample.fingers,
                    "no touchpad binding claims this gesture; it was dropped"
                );
                Routed::Dropped
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

impl State {
    /// Applies one seat `claim_gesture` call: the claim's kind is the call's text and its finger count
    /// the second argument. A kind or count the table refuses, or a gesture another claim takes, is
    /// logged and the claim is not made, as a chord that does not parse.
    pub(crate) fn claim_gesture(&mut self, call: &TreeCall, id: i64) {
        let Some(kind) = GestureKind::from_name(call.text.expose()) else {
            log::warn!(
                area = "claim",
                claim = id,
                "a seat gesture claim names a kind that is not swipe or pinch"
            );
            return;
        };
        let fingers = call.args.get(1).copied().unwrap_or(0);
        if let Err(error) = self
            .touchpad_gestures
            .claims_mut()
            .claim(call.node.get(), id, kind, fingers)
        {
            log::warn!(area = "claim", claim = id, "a seat gesture claim was refused: {error}");
        }
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
        angle: sample.angle,
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
            angle: 0.0,
        }
    }

    /// The binding a routed sample fired, if it fired one.
    fn fired_of(routed: Routed) -> Option<Fire> {
        match routed {
            Routed::Fire(fire) => Some(fire),
            Routed::Claimed(_) | Routed::Dropped => None,
        }
    }

    /// Feeds a swipe of `fingers` fingers moving right, then its end, and returns every routing.
    fn swipe_routes(routes: &mut TouchpadGestures, fingers: u8, lock: GestureLock, cancel: bool) -> Vec<Routed> {
        let mut out = vec![routes.route(
            Tick::new(1),
            lock,
            sample(fingers, RawTouchpadPhase::Begin, RawTouchpadKind::Swipe, 0.0, 1.0),
        )];
        for tick in 2..6 {
            out.push(routes.route(
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
        out.push(routes.route(
            Tick::new(9),
            lock,
            sample(fingers, phase, RawTouchpadKind::Swipe, 0.0, 1.0),
        ));
        out
    }

    /// Feeds a swipe and returns the binding it fired, if any.
    fn swipe(routes: &mut TouchpadGestures, fingers: u8, lock: GestureLock, cancel: bool) -> Option<Fire> {
        swipe_routes(routes, fingers, lock, cancel)
            .into_iter()
            .find_map(fired_of)
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
            fired_of(routes.route(Tick::new(3), GestureLock::Unlocked, end))
        };
        assert_eq!(pinched(&mut bound).map(|fire| fire.name), Some("spread".to_owned()));

        let mut unbound = routes(&[swipe_path("three", 3)]);
        assert_eq!(pinched(&mut unbound), None);
    }

    #[test]
    fn a_hold_is_dropped_without_reaching_a_binding() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        let hold = sample(3, RawTouchpadPhase::Begin, RawTouchpadKind::Hold, 0.0, 1.0);
        assert_eq!(routes.route(Tick::new(1), GestureLock::Unlocked, hold), Routed::Dropped);
    }

    #[test]
    fn no_configured_bindings_drop_every_gesture() {
        let mut routes = routes(&[]);
        assert_eq!(swipe(&mut routes, 3, GestureLock::Unlocked, false), None);
    }

    /// The deliveries in a run of routings, as `(phase, dx)`.
    fn deliveries(routed: &[Routed]) -> Vec<(gnoblin_input::GesturePhase, f64)> {
        routed
            .iter()
            .filter_map(|routed| match routed {
                Routed::Claimed(delivery) => Some((delivery.phase, delivery.dx)),
                _ => None,
            })
            .collect()
    }

    fn claim_swipe(routes: &mut TouchpadGestures, id: i64, fingers: i64) {
        routes
            .claims_mut()
            .claim(7, id, GestureKind::Swipe, fingers)
            .expect("the claim is accepted");
    }

    #[test]
    fn a_claimed_swipe_is_delivered_to_its_holder_and_never_fires_a_binding() {
        use gnoblin_input::GesturePhase::{Begin, End, Update};
        let mut routes = routes(&[swipe_path("three", 3)]);
        claim_swipe(&mut routes, 1, 3);
        let routed = swipe_routes(&mut routes, 3, GestureLock::Unlocked, false);
        assert_eq!(
            deliveries(&routed),
            [
                (Begin, 0.0),
                (Update, 120.0),
                (Update, 240.0),
                (Update, 360.0),
                (Update, 480.0),
                (End, 480.0)
            ]
        );
        assert!(
            routed.iter().cloned().all(|routed| fired_of(routed).is_none()),
            "the binding the claim shadows never fires"
        );
    }

    #[test]
    fn a_claim_needs_no_configured_bindings() {
        let mut routes = routes(&[]);
        claim_swipe(&mut routes, 1, 3);
        let routed = swipe_routes(&mut routes, 3, GestureLock::Unlocked, false);
        assert_eq!(deliveries(&routed).len(), 6, "{routed:?}");
    }

    #[test]
    fn a_gesture_another_claim_does_not_take_still_fires_its_binding() {
        let mut routes = routes(&[swipe_path("three", 3), swipe_path("four", 4)]);
        claim_swipe(&mut routes, 1, 4);
        let fire = swipe(&mut routes, 3, GestureLock::Unlocked, false).expect("the unclaimed swipe fires");
        assert_eq!(fire.name, "three");
        assert_eq!(swipe(&mut routes, 4, GestureLock::Unlocked, false), None);
    }

    #[test]
    fn a_claimed_gesture_while_locked_is_delivered_to_nobody() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        claim_swipe(&mut routes, 1, 3);
        let routed = swipe_routes(&mut routes, 3, GestureLock::Locked, false);
        assert!(routed.iter().all(|routed| *routed == Routed::Dropped), "{routed:?}");
    }

    #[test]
    fn a_lock_mid_gesture_cancels_the_holders_gesture_and_fires_no_binding() {
        use gnoblin_input::GesturePhase::Cancel;
        let mut routes = routes(&[swipe_path("three", 3)]);
        claim_swipe(&mut routes, 1, 3);
        routes.route(
            Tick::new(1),
            GestureLock::Unlocked,
            sample(3, RawTouchpadPhase::Begin, RawTouchpadKind::Swipe, 0.0, 1.0),
        );
        let locked = routes.route(
            Tick::new(2),
            GestureLock::Locked,
            sample(3, RawTouchpadPhase::Update, RawTouchpadKind::Swipe, 50.0, 1.0),
        );
        assert!(matches!(locked, Routed::Claimed(delivery) if delivery.phase == Cancel));
        let end = routes.route(
            Tick::new(3),
            GestureLock::Locked,
            sample(3, RawTouchpadPhase::End, RawTouchpadKind::Swipe, 0.0, 1.0),
        );
        assert_eq!(end, Routed::Dropped);
    }

    #[test]
    fn a_holder_that_goes_away_mid_gesture_drops_the_rest_without_firing_a_binding() {
        let mut routes = routes(&[swipe_path("three", 3)]);
        claim_swipe(&mut routes, 1, 3);
        routes.route(
            Tick::new(1),
            GestureLock::Unlocked,
            sample(3, RawTouchpadPhase::Begin, RawTouchpadKind::Swipe, 0.0, 1.0),
        );
        assert_eq!(routes.claims_mut().release(&[7]), 1);
        for (tick, phase) in [(2, RawTouchpadPhase::Update), (3, RawTouchpadPhase::End)] {
            let routed = routes.route(
                Tick::new(tick),
                GestureLock::Unlocked,
                sample(3, phase, RawTouchpadKind::Swipe, 120.0, 1.0),
            );
            assert_eq!(routed, Routed::Dropped);
        }
        // The next gesture is nobody's claim again and reaches the binding.
        let fire = swipe(&mut routes, 3, GestureLock::Unlocked, false).expect("the binding fires again");
        assert_eq!(fire.name, "three");
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
