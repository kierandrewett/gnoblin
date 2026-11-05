//! Which `touchpad_gestures` binding claims a gesture.
//!
//! The published rules are short and strict: at gesture start Gnoblin reserves input
//! when a binding matches the gesture type, the finger count and the active lock
//! context, and at the end it runs the action or command only if the movement also
//! matches the configured path or pinch direction and threshold. A reserved gesture
//! whose movement does not match has no direct action and is not passed to another
//! handler.
//!
//! Two bindings that could reserve the same gesture are therefore a configuration
//! error, not a race: [`GestureRouter::new`] refuses them and names both. That is what
//! makes "no two bindings claim the same movement" a property of the type rather than
//! a rule this module has to remember at runtime.
//!
//! The C router instead collected every candidate that matched the begin and scanned
//! them at the end, so with two same-kind same-finger declarations the loser could
//! still win if the first one's path did not match. The reservation is singular in the
//! documented behaviour, and a declaration that silently loses to another on some
//! gestures is exactly the shadowing this crate exists to remove.

use std::fmt;

use crate::gesture::{GestureEvent, GestureKind, GesturePhase, path_matches};
use crate::name::{Name, NameError};

/// Which way a pinch goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PinchDirection {
    /// The contacts gathered.
    In,
    /// The contacts spread.
    Out,
}

impl PinchDirection {
    /// The name the configuration uses.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

/// The session state a gesture binding applies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum When {
    /// In either state.
    Any,
    /// Only in an unlocked session.
    Unlocked,
    /// Only on the lock screen.
    Locked,
}

impl When {
    /// The documented spellings, including the two legacy names.
    ///
    /// One table for the aliases, because the C tree had `"normal"` and
    /// `"unlock-screen"` handled in three separate files and they disagreed.
    pub fn parse(text: &str) -> Result<Self, WhenError> {
        match text {
            "any" => Ok(Self::Any),
            "unlocked" | "normal" => Ok(Self::Unlocked),
            "locked" | "unlock-screen" => Ok(Self::Locked),
            _ => Err(WhenError { text: text.to_owned() }),
        }
    }

    /// Whether this value applies in `context`.
    pub const fn matches(self, context: Context) -> bool {
        matches!(
            (self, context),
            (Self::Any, _) | (Self::Unlocked, Context::Unlocked) | (Self::Locked, Context::Locked)
        )
    }

    /// Whether two values can both apply in one session state.
    ///
    /// `any` overlaps both single states, which is what makes "any" plus "unlocked" a
    /// conflict rather than an ordering question.
    pub const fn overlaps(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Any, _) | (_, Self::Any) | (Self::Unlocked, Self::Unlocked) | (Self::Locked, Self::Locked)
        )
    }
}

impl fmt::Display for When {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Any => "any",
            Self::Unlocked => "unlocked",
            Self::Locked => "locked",
        })
    }
}

/// A `when` value that is not one of the documented spellings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhenError {
    text: String,
}

impl fmt::Display for WhenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "when \"{}\" is not a session state; use \"unlocked\", \"locked\", or \"any\"",
            self.text
        )
    }
}

impl std::error::Error for WhenError {}

/// The session state a gesture arrives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    /// The session is unlocked.
    Unlocked,
    /// The lock screen is up.
    Locked,
}

/// What a matched gesture does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GestureAction {
    /// Run a built-in action by its documented name, such as `workspace.next`.
    Action(String),
    /// Start a program with these arguments.
    Command(Vec<String>),
}

/// How a swipe's movement is compared with the configured path.
#[derive(Clone, Debug, PartialEq)]
pub enum Matcher {
    /// A path and the tolerance it must match within.
    Path {
        /// The configured shape, starting at `(0.0, 0.0)`.
        points: Vec<(f64, f64)>,
        /// Root-mean-square distance allowed, from 0.05 to 0.5.
        tolerance: f64,
    },
    /// A pinch direction.
    Direction(PinchDirection),
}

/// One validated `touchpad_gestures` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct GestureBinding {
    name: Name,
    kind: GestureKind,
    fingers: u8,
    matcher: Matcher,
    threshold: f64,
    when: When,
    action: GestureAction,
}

impl GestureBinding {
    /// The declared name.
    pub const fn name(&self) -> &Name {
        &self.name
    }

    /// Swipe or pinch.
    pub const fn kind(&self) -> GestureKind {
        self.kind
    }

    /// How many fingers it needs.
    pub const fn fingers(&self) -> u8 {
        self.fingers
    }

    /// The movement test.
    pub const fn matcher(&self) -> &Matcher {
        &self.matcher
    }

    /// The minimum movement before the matcher can match.
    pub const fn threshold(&self) -> f64 {
        self.threshold
    }

    /// The session state it applies in.
    pub const fn when(&self) -> When {
        self.when
    }

    /// What it runs.
    pub const fn action(&self) -> &GestureAction {
        &self.action
    }

    /// Whether this binding reserves a gesture that began as given.
    pub const fn claims(&self, kind: GestureKind, fingers: u8, context: Context) -> bool {
        self.fingers == fingers && self.matches_kind(kind) && self.when.matches(context)
    }

    /// Whether a recognised kind is this binding's kind.
    const fn matches_kind(&self, kind: GestureKind) -> bool {
        matches!(
            (self.kind, kind),
            (GestureKind::Swipe, GestureKind::Swipe) | (GestureKind::Pinch, GestureKind::Pinch)
        )
    }

    /// Whether the finished gesture's movement matches this binding.
    fn matches_end(&self, event: &GestureEvent) -> bool {
        let GestureEvent::End {
            path, travel, scale, ..
        } = event
        else {
            return false;
        };
        match &self.matcher {
            Matcher::Path { points, tolerance } => {
                *travel >= self.threshold && path_matches(points, path.points(), *tolerance)
            }
            Matcher::Direction(direction) => {
                let delta = scale - 1.0;
                if delta.abs() < self.threshold {
                    return false;
                }
                match direction {
                    PinchDirection::In => delta < 0.0,
                    PinchDirection::Out => delta > 0.0,
                }
            }
        }
    }
}

/// The smallest accepted swipe path.
pub const MIN_PATH_POINTS: usize = 2;

/// The largest accepted swipe path.
pub const MAX_PATH_POINTS: usize = 16;

/// The smallest accepted tolerance.
pub const MIN_TOLERANCE: f64 = 0.05;

/// The largest accepted tolerance.
pub const MAX_TOLERANCE: f64 = 0.5;

/// The default tolerance.
pub const DEFAULT_TOLERANCE: f64 = 0.22;

/// The default swipe threshold, in caller units.
pub const DEFAULT_SWIPE_THRESHOLD: f64 = 48.0;

/// The default pinch threshold, as a fraction of the starting spread.
pub const DEFAULT_PINCH_THRESHOLD: f64 = 0.12;

/// One `touchpad_gestures` entry as the configuration layer reads it, before validation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GestureSpec {
    /// The declared name.
    pub name: String,
    /// Swipe or pinch. Swipe is assumed when the field is absent so validation can
    /// report a missing field as what it is rather than as a wrong kind.
    pub gesture: Option<GestureKind>,
    /// How many fingers, 2 to 5.
    pub fingers: Option<u8>,
    /// The configured swipe path.
    pub path: Option<Vec<(f64, f64)>>,
    /// The configured pinch direction.
    pub direction: Option<PinchDirection>,
    /// The tolerance, for a swipe.
    pub tolerance: Option<f64>,
    /// The minimum movement.
    pub threshold: Option<f64>,
    /// The session state. Defaults to `unlocked`, as documented.
    pub when: Option<When>,
    /// The action or command, exactly one of which is required.
    pub action: Option<GestureAction>,
}

impl GestureSpec {
    /// Validate the entry into a binding, or say precisely what is wrong.
    pub fn validate(self) -> Result<GestureBinding, GestureError> {
        let name = Name::new(&self.name).map_err(GestureError::Name)?;
        let Some(kind) = self.gesture else {
            return Err(GestureError::MissingGesture);
        };
        let Some(fingers) = self.fingers else {
            return Err(GestureError::MissingFingers);
        };
        if !(crate::gesture::MIN_FINGERS..=crate::gesture::MAX_FINGERS).contains(&fingers) {
            return Err(GestureError::FingersOutOfRange { fingers });
        }
        let Some(action) = self.action else {
            return Err(GestureError::MissingAction);
        };
        if matches!(&action, GestureAction::Command(arguments) if arguments.is_empty()) {
            return Err(GestureError::EmptyCommand);
        }
        let when = self.when.unwrap_or(When::Unlocked);
        let matcher = match kind {
            GestureKind::Swipe => {
                if self.direction.is_some() {
                    return Err(GestureError::UnexpectedDirection);
                }
                let Some(path) = self.path else {
                    return Err(GestureError::MissingPath);
                };
                validate_path(&path)?;
                let tolerance = self.tolerance.unwrap_or(DEFAULT_TOLERANCE);
                if !(MIN_TOLERANCE..=MAX_TOLERANCE).contains(&tolerance) {
                    return Err(GestureError::ToleranceOutOfRange { tolerance });
                }
                Matcher::Path {
                    points: path,
                    tolerance,
                }
            }
            GestureKind::Pinch => {
                if self.path.is_some() {
                    return Err(GestureError::UnexpectedPath);
                }
                let Some(direction) = self.direction else {
                    return Err(GestureError::MissingDirection);
                };
                Matcher::Direction(direction)
            }
        };
        let threshold = match (kind, self.threshold) {
            (GestureKind::Swipe, None) => DEFAULT_SWIPE_THRESHOLD,
            (GestureKind::Pinch, None) => DEFAULT_PINCH_THRESHOLD,
            (GestureKind::Swipe, Some(threshold)) => {
                if !(16.0..=240.0).contains(&threshold) {
                    return Err(GestureError::ThresholdOutOfRange { threshold });
                }
                threshold
            }
            (GestureKind::Pinch, Some(threshold)) => {
                if !(MIN_TOLERANCE..=MAX_TOLERANCE).contains(&threshold) {
                    return Err(GestureError::ThresholdOutOfRange { threshold });
                }
                threshold
            }
        };
        Ok(GestureBinding {
            name,
            kind,
            fingers,
            matcher,
            threshold,
            when,
            action,
        })
    }
}

/// Check a configured path against the documented shape rules.
fn validate_path(path: &[(f64, f64)]) -> Result<(), GestureError> {
    if path.len() < MIN_PATH_POINTS {
        return Err(GestureError::PathTooShort { points: path.len() });
    }
    if path.len() > MAX_PATH_POINTS {
        return Err(GestureError::PathTooLong { points: path.len() });
    }
    let Some(first) = path.first() else {
        return Err(GestureError::PathTooShort { points: 0 });
    };
    if *first != (0.0, 0.0) {
        return Err(GestureError::PathMustStartAtOrigin { x: first.0, y: first.1 });
    }
    for point in path {
        if !(-1.0..=1.0).contains(&point.0) || !(-1.0..=1.0).contains(&point.1) {
            return Err(GestureError::PathPointOutOfRange { x: point.0, y: point.1 });
        }
    }
    Ok(())
}

/// Why a gesture binding was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum GestureError {
    /// The name is not a valid binding name.
    Name(NameError),
    /// The `gesture` field is absent.
    MissingGesture,
    /// The `fingers` field is absent.
    MissingFingers,
    /// The finger count is not 2 to 5.
    FingersOutOfRange {
        /// The refused count.
        fingers: u8,
    },
    /// A swipe with no path.
    MissingPath,
    /// A pinch with no direction.
    MissingDirection,
    /// A swipe with a direction, which would be ignored.
    UnexpectedDirection,
    /// A pinch with a path, which would be ignored.
    UnexpectedPath,
    /// The path has fewer than two points.
    PathTooShort {
        /// The number of points given.
        points: usize,
    },
    /// The path has more than sixteen points.
    PathTooLong {
        /// The number of points given.
        points: usize,
    },
    /// The path does not start at the origin.
    PathMustStartAtOrigin {
        /// The first point's x.
        x: f64,
        /// The first point's y.
        y: f64,
    },
    /// A path point outside -1 to 1.
    PathPointOutOfRange {
        /// The refused x.
        x: f64,
        /// The refused y.
        y: f64,
    },
    /// The tolerance is outside 0.05 to 0.5.
    ToleranceOutOfRange {
        /// The refused tolerance.
        tolerance: f64,
    },
    /// The threshold is outside its range for this gesture kind.
    ThresholdOutOfRange {
        /// The refused threshold.
        threshold: f64,
    },
    /// Neither an action nor a command was given.
    MissingAction,
    /// The command is empty.
    EmptyCommand,
    /// Two bindings would reserve the same gesture.
    Conflict {
        /// The first declaration.
        first: Name,
        /// The later declaration that also claims it.
        second: Name,
    },
}

impl fmt::Display for GestureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(error) => write!(formatter, "{error}"),
            Self::MissingGesture => formatter.write_str("a touchpad gesture needs gesture = \"swipe\" or \"pinch\""),
            Self::MissingFingers => formatter.write_str("a touchpad gesture needs a finger count from 2 to 5"),
            Self::FingersOutOfRange { fingers } => {
                write!(formatter, "fingers = {fingers} is outside 2 to 5")
            }
            Self::MissingPath => formatter.write_str("a swipe needs a path starting at {x = 0, y = 0}"),
            Self::MissingDirection => formatter.write_str("a pinch needs direction = \"in\" or \"out\""),
            Self::UnexpectedDirection => formatter.write_str("a swipe matches a path, so it cannot also set direction"),
            Self::UnexpectedPath => formatter.write_str("a pinch matches a direction, so it cannot also set path"),
            Self::PathTooShort { points } => {
                write!(formatter, "a path has 2 to 16 points; this one has {points}")
            }
            Self::PathTooLong { points } => {
                write!(formatter, "a path has 2 to 16 points; this one has {points}")
            }
            Self::PathMustStartAtOrigin { x, y } => {
                write!(
                    formatter,
                    "a path starts at {{x = 0, y = 0}}; this one starts at {{x = {x}, y = {y}}}"
                )
            }
            Self::PathPointOutOfRange { x, y } => {
                write!(formatter, "path point {{x = {x}, y = {y}}} is outside -1 to 1")
            }
            Self::ToleranceOutOfRange { tolerance } => {
                write!(formatter, "tolerance = {tolerance} is outside 0.05 to 0.5")
            }
            Self::ThresholdOutOfRange { threshold } => {
                write!(
                    formatter,
                    "threshold = {threshold} is outside its range: 16 to 240 for a swipe, 0.05 to 0.5 for a pinch"
                )
            }
            Self::MissingAction => formatter.write_str("a touchpad gesture needs exactly one of action or command"),
            Self::EmptyCommand => formatter.write_str("command needs at least one argument"),
            Self::Conflict { first, second } => write!(
                formatter,
                "touchpad gesture \"{second}\" claims the same gesture as \"{first}\": they would compete for one movement"
            ),
        }
    }
}

impl std::error::Error for GestureError {}

/// What the router did with a gesture event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureRoute {
    /// No binding claims it, so it belongs to the client.
    Unclaimed,
    /// This binding reserved the gesture at its begin; the movement is not known yet.
    Reserved {
        /// The binding, by position in the router.
        binding: usize,
    },
    /// The movement matched: run this binding's action or command.
    Fired {
        /// The binding, by position in the router.
        binding: usize,
    },
    /// The gesture was reserved but its movement did not match, so nothing runs and
    /// nothing is passed on.
    MatchedNothing {
        /// The binding that reserved it.
        binding: usize,
    },
    /// The gesture was cancelled; any reservation is over.
    Cancelled {
        /// The binding that had reserved it, if any.
        binding: Option<usize>,
    },
}

/// Counters a session can report, and the model test checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GestureStats {
    /// Gestures reserved by a binding.
    pub reserved: u64,
    /// Reserved gestures whose movement matched.
    pub fired: u64,
    /// Reserved gestures whose movement did not match.
    pub unmatched: u64,
    /// Gestures cancelled while reserved.
    pub cancelled: u64,
    /// Gestures no binding claimed.
    pub unclaimed: u64,
}

/// The gesture bindings and the one reservation they share.
#[derive(Clone, Debug)]
pub struct GestureRouter {
    bindings: Vec<GestureBinding>,
    claim: Option<usize>,
    stats: GestureStats,
}

impl GestureRouter {
    /// Build a router, refusing bindings that would compete for one gesture.
    pub fn new(bindings: Vec<GestureBinding>) -> Result<Self, GestureError> {
        for (index, later) in bindings.iter().enumerate() {
            for earlier in bindings.iter().take(index) {
                let same_gesture = earlier.kind() == later.kind() && earlier.fingers() == later.fingers();
                if same_gesture && earlier.when().overlaps(later.when()) {
                    return Err(GestureError::Conflict {
                        first: earlier.name().clone(),
                        second: later.name().clone(),
                    });
                }
            }
        }
        Ok(Self {
            bindings,
            claim: None,
            stats: GestureStats::default(),
        })
    }

    /// The bindings, in declaration order.
    pub fn bindings(&self) -> &[GestureBinding] {
        &self.bindings
    }

    /// The binding at a position, for running a fired action.
    pub fn binding(&self, index: usize) -> Option<&GestureBinding> {
        self.bindings.get(index)
    }

    /// Whether a gesture is currently reserved.
    pub const fn reserved(&self) -> Option<usize> {
        self.claim
    }

    /// The counters so far.
    pub const fn stats(&self) -> GestureStats {
        self.stats
    }

    /// Decide what a recognised gesture means.
    pub fn observe(&mut self, event: &GestureEvent, context: Context) -> GestureRoute {
        match event.phase() {
            GesturePhase::Begin => {
                let candidate = self
                    .bindings
                    .iter()
                    .position(|binding| binding.claims(event.kind(), event.fingers(), context));
                self.claim = candidate;
                match candidate {
                    Some(binding) => {
                        self.stats.reserved += 1;
                        GestureRoute::Reserved { binding }
                    }
                    None => {
                        self.stats.unclaimed += 1;
                        GestureRoute::Unclaimed
                    }
                }
            }
            GesturePhase::Update => match self.claim {
                Some(binding) => GestureRoute::Reserved { binding },
                None => GestureRoute::Unclaimed,
            },
            GesturePhase::End => {
                let Some(binding) = self.claim.take() else {
                    self.stats.unclaimed += 1;
                    return GestureRoute::Unclaimed;
                };
                let matched = self
                    .bindings
                    .get(binding)
                    .is_some_and(|binding| binding.matches_end(event));
                if matched {
                    self.stats.fired += 1;
                    GestureRoute::Fired { binding }
                } else {
                    self.stats.unmatched += 1;
                    GestureRoute::MatchedNothing { binding }
                }
            }
            GesturePhase::Cancel => {
                let binding = self.claim.take();
                if binding.is_some() {
                    self.stats.cancelled += 1;
                } else {
                    self.stats.unclaimed += 1;
                }
                GestureRoute::Cancelled { binding }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Context, GestureAction, GestureError, GestureRoute, GestureRouter, GestureSpec, Matcher, PinchDirection, When,
    };
    use crate::gesture::{GestureEvent, GestureKind, GesturePath};
    use crate::name::Name;
    use gnoblin_types::Tick;

    fn swipe(name: &str, fingers: u8, path: Vec<(f64, f64)>, when: When, action: GestureAction) -> GestureSpec {
        GestureSpec {
            name: name.to_owned(),
            gesture: Some(GestureKind::Swipe),
            fingers: Some(fingers),
            path: Some(path),
            when: Some(when),
            action: Some(action),
            ..GestureSpec::default()
        }
    }

    fn pinch(name: &str, fingers: u8, direction: PinchDirection, when: When) -> GestureSpec {
        GestureSpec {
            name: name.to_owned(),
            gesture: Some(GestureKind::Pinch),
            fingers: Some(fingers),
            direction: Some(direction),
            when: Some(when),
            action: Some(GestureAction::Action("workspace.next".to_owned())),
            ..GestureSpec::default()
        }
    }

    fn command() -> GestureAction {
        GestureAction::Command(vec!["kgx".to_owned()])
    }

    fn begin(kind: GestureKind, fingers: u8) -> GestureEvent {
        GestureEvent::Begin {
            tick: Tick::new(1),
            kind,
            fingers,
            edge: None,
        }
    }

    fn end(kind: GestureKind, fingers: u8, path: &[(f64, f64)], travel: f64, scale: f64) -> GestureEvent {
        GestureEvent::End {
            tick: Tick::new(3),
            kind,
            fingers,
            edge: None,
            path: GesturePath::from_points(path.to_vec()),
            travel,
            scale,
            duration: 2,
        }
    }

    #[test]
    fn two_bindings_that_would_claim_one_gesture_are_refused() {
        let first = swipe("right", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command())
            .validate()
            .expect("first");
        let second = swipe("left", 3, vec![(0.0, 0.0), (-1.0, 0.0)], When::Any, command())
            .validate()
            .expect("second");
        let error = GestureRouter::new(vec![first, second]).expect_err("conflict");
        assert_eq!(
            error,
            GestureError::Conflict {
                first: Name::new("right").expect("name"),
                second: Name::new("left").expect("name")
            }
        );
    }

    #[test]
    fn a_swipe_and_a_pinch_may_share_a_finger_count() {
        let bindings = vec![
            swipe("right", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command())
                .validate()
                .expect("swipe"),
            pinch("out", 3, PinchDirection::Out, When::Unlocked)
                .validate()
                .expect("pinch"),
        ];
        let router = GestureRouter::new(bindings).expect("no conflict");
        assert_eq!(router.bindings().len(), 2);
    }

    #[test]
    fn an_unlocked_and_a_locked_binding_do_not_conflict() {
        let bindings = vec![
            swipe("unlocked", 4, vec![(0.0, 0.0), (0.0, 1.0)], When::Unlocked, command())
                .validate()
                .expect("unlocked"),
            swipe("locked", 4, vec![(0.0, 0.0), (0.0, 1.0)], When::Locked, command())
                .validate()
                .expect("locked"),
        ];
        GestureRouter::new(bindings).expect("no conflict");
        assert!(When::Unlocked.overlaps(When::Any));
        assert!(!When::Unlocked.overlaps(When::Locked));
    }

    #[test]
    fn a_reserved_gesture_fires_only_when_its_path_matches() {
        let spec = swipe(
            "right",
            3,
            vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)],
            When::Unlocked,
            command(),
        );
        let mut router = GestureRouter::new(vec![spec.validate().expect("valid")]).expect("no conflict");

        let route = router.observe(&begin(GestureKind::Swipe, 3), Context::Unlocked);
        assert_eq!(route, GestureRoute::Reserved { binding: 0 });
        let route = router.observe(
            &end(
                GestureKind::Swipe,
                3,
                &[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0)],
                120.0,
                1.0,
            ),
            Context::Unlocked,
        );
        assert_eq!(route, GestureRoute::Fired { binding: 0 });
        assert_eq!(router.stats().fired, 1);

        let route = router.observe(&begin(GestureKind::Swipe, 3), Context::Unlocked);
        assert_eq!(route, GestureRoute::Reserved { binding: 0 });
        let route = router.observe(
            &end(
                GestureKind::Swipe,
                3,
                &[(0.0, 0.0), (0.0, 60.0), (60.0, 60.0)],
                120.0,
                1.0,
            ),
            Context::Unlocked,
        );
        assert_eq!(route, GestureRoute::MatchedNothing { binding: 0 });
        assert_eq!(router.reserved(), None, "the reservation is over either way");
    }

    #[test]
    fn a_short_gesture_does_not_reach_the_threshold() {
        let spec = swipe("right", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        let mut router = GestureRouter::new(vec![spec.validate().expect("valid")]).expect("no conflict");
        router.observe(&begin(GestureKind::Swipe, 3), Context::Unlocked);
        let route = router.observe(
            &end(GestureKind::Swipe, 3, &[(0.0, 0.0), (40.0, 0.0)], 40.0, 1.0),
            Context::Unlocked,
        );
        assert_eq!(route, GestureRoute::MatchedNothing { binding: 0 });
    }

    #[test]
    fn a_pinch_fires_in_its_configured_direction_only() {
        let mut router = GestureRouter::new(vec![
            pinch("out", 4, PinchDirection::Out, When::Unlocked)
                .validate()
                .expect("valid"),
        ])
        .expect("no conflict");
        router.observe(&begin(GestureKind::Pinch, 4), Context::Unlocked);
        let route = router.observe(&end(GestureKind::Pinch, 4, &[], 0.0, 0.5), Context::Unlocked);
        assert_eq!(route, GestureRoute::MatchedNothing { binding: 0 });
        let route = router.observe(&begin(GestureKind::Pinch, 4), Context::Unlocked);
        assert_eq!(route, GestureRoute::Reserved { binding: 0 });
        let route = router.observe(&end(GestureKind::Pinch, 4, &[], 0.0, 1.4), Context::Unlocked);
        assert_eq!(route, GestureRoute::Fired { binding: 0 });
    }

    #[test]
    fn an_unmatched_context_leaves_the_gesture_to_the_client() {
        let spec = swipe("right", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Locked, command());
        let mut router = GestureRouter::new(vec![spec.validate().expect("valid")]).expect("no conflict");
        let route = router.observe(&begin(GestureKind::Swipe, 3), Context::Unlocked);
        assert_eq!(route, GestureRoute::Unclaimed);
        let route = router.observe(
            &end(GestureKind::Swipe, 3, &[(0.0, 0.0), (60.0, 0.0)], 60.0, 1.0),
            Context::Unlocked,
        );
        assert_eq!(route, GestureRoute::Unclaimed);
    }

    #[test]
    fn a_cancel_releases_the_reservation() {
        let spec = swipe("right", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        let mut router = GestureRouter::new(vec![spec.validate().expect("valid")]).expect("no conflict");
        router.observe(&begin(GestureKind::Swipe, 3), Context::Unlocked);
        let cancel = GestureEvent::Cancel {
            tick: Tick::new(2),
            kind: GestureKind::Swipe,
            fingers: 3,
            reason: crate::gesture::CancelReason::Cancelled,
        };
        assert_eq!(
            router.observe(&cancel, Context::Unlocked),
            GestureRoute::Cancelled { binding: Some(0) }
        );
        assert_eq!(router.reserved(), None);
        assert_eq!(router.stats().cancelled, 1);
    }

    #[test]
    fn validation_names_the_documented_limits() {
        let mut spec = swipe("bad name", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        assert!(matches!(spec.validate(), Err(GestureError::Name(_))));

        spec = swipe("fingers", 6, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        assert_eq!(
            spec.validate().expect_err("fingers"),
            GestureError::FingersOutOfRange { fingers: 6 }
        );

        spec = swipe("short", 3, vec![(0.0, 0.0)], When::Unlocked, command());
        assert_eq!(
            spec.validate().expect_err("short"),
            GestureError::PathTooShort { points: 1 }
        );

        spec = swipe("long", 3, vec![(0.0, 0.0); 17], When::Unlocked, command());
        assert_eq!(
            spec.validate().expect_err("long"),
            GestureError::PathTooLong { points: 17 }
        );

        spec = swipe("origin", 3, vec![(0.5, 0.0), (1.0, 0.0)], When::Unlocked, command());
        assert!(matches!(
            spec.validate().expect_err("origin"),
            GestureError::PathMustStartAtOrigin { .. }
        ));

        spec = swipe("outside", 3, vec![(0.0, 0.0), (2.0, 0.0)], When::Unlocked, command());
        assert!(matches!(
            spec.validate().expect_err("range"),
            GestureError::PathPointOutOfRange { .. }
        ));

        spec = swipe("tolerance", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        spec.tolerance = Some(0.9);
        assert!(matches!(
            spec.validate().expect_err("tolerance"),
            GestureError::ToleranceOutOfRange { .. }
        ));

        spec = swipe("threshold", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        spec.threshold = Some(4.0);
        assert!(matches!(
            spec.validate().expect_err("threshold"),
            GestureError::ThresholdOutOfRange { .. }
        ));

        spec = swipe("action", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        spec.action = None;
        assert_eq!(spec.validate().expect_err("action"), GestureError::MissingAction);

        spec = swipe("command", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        spec.action = Some(GestureAction::Command(Vec::new()));
        assert_eq!(spec.validate().expect_err("empty"), GestureError::EmptyCommand);

        let mut pinch_with_path = pinch("path", 3, PinchDirection::In, When::Unlocked);
        pinch_with_path.path = Some(vec![(0.0, 0.0), (1.0, 0.0)]);
        assert_eq!(
            pinch_with_path.validate().expect_err("path on a pinch"),
            GestureError::UnexpectedPath
        );

        let mut swipe_with_direction = swipe("direction", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        swipe_with_direction.direction = Some(PinchDirection::Out);
        assert_eq!(
            swipe_with_direction.validate().expect_err("direction on a swipe"),
            GestureError::UnexpectedDirection
        );

        let mut pinch_without_direction = pinch("no_direction", 3, PinchDirection::In, When::Unlocked);
        pinch_without_direction.direction = None;
        assert_eq!(
            pinch_without_direction.validate().expect_err("no direction"),
            GestureError::MissingDirection
        );

        spec = swipe("gesture", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        spec.gesture = None;
        assert_eq!(spec.validate().expect_err("gesture"), GestureError::MissingGesture);

        spec = swipe(
            "fingers_field",
            3,
            vec![(0.0, 0.0), (1.0, 0.0)],
            When::Unlocked,
            command(),
        );
        spec.fingers = None;
        assert_eq!(
            spec.validate().expect_err("fingers field"),
            GestureError::MissingFingers
        );

        spec = swipe("path_field", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        spec.path = None;
        assert_eq!(spec.validate().expect_err("path field"), GestureError::MissingPath);
    }

    #[test]
    fn the_documented_defaults_apply() {
        let binding = swipe("defaults", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command())
            .validate()
            .expect("valid");
        assert_eq!(binding.threshold(), super::DEFAULT_SWIPE_THRESHOLD);
        assert_eq!(binding.when(), When::Unlocked);
        assert!(matches!(
            binding.matcher(),
            Matcher::Path { tolerance, .. } if *tolerance == super::DEFAULT_TOLERANCE
        ));

        let binding = pinch("defaults", 4, PinchDirection::In, When::Any)
            .validate()
            .expect("valid");
        assert_eq!(binding.threshold(), super::DEFAULT_PINCH_THRESHOLD);

        let mut absent = swipe("when", 3, vec![(0.0, 0.0), (1.0, 0.0)], When::Unlocked, command());
        absent.when = None;
        assert_eq!(
            absent.validate().expect("valid").when(),
            When::Unlocked,
            "an omitted when means unlocked"
        );
    }

    #[test]
    fn the_when_aliases_are_one_table() {
        assert_eq!(When::parse("normal").expect("legacy"), When::Unlocked);
        assert_eq!(When::parse("unlock-screen").expect("legacy"), When::Locked);
        assert_eq!(When::parse("any").expect("any"), When::Any);
        assert!(When::parse("Normal").is_err(), "the spellings are exact");
        assert!(When::parse("sometimes").is_err());
        assert_eq!(When::Unlocked.to_string(), "unlocked");
    }
}
