//! The tween player: tracks that animate one property over caller ticks.
//!
//! The player has no clock. A caller advances it with the time that actually
//! passed, so a compositor can drive it from its frame clock and a test can
//! drive it deterministically. Elapsed time is whole milliseconds in a `u64`,
//! which makes completion exact: a track that reaches its total duration applies
//! the target value verbatim, not an interpolation that rounds to almost it.
//!
//! # Composition rule
//!
//! Two tracks may animate one node only when they write disjoint transform
//! components ([`VisualProperty::conflicts_with`]). Starting a track on a
//! property that a live track already owns ends the older track with
//! [`TrackEndReason::Superseded`] before the new one begins, and the new track
//! takes its starting value from the transform as it stands. The rule is
//! therefore deterministic and total: at most one track writes any component,
//! and the surviving track is always the newest. Applying tracks in ascending
//! track id order is enough to make a frame independent of map iteration.
//!
//! # Every track ends
//!
//! The plan's rule is that every animation ends and reports `finished`, even
//! when its target closes. The player enforces it structurally: a track is only
//! ever in one of two places, the live map or the ended queue, and every path
//! that removes a track from the map pushes exactly one [`TrackEnd`] onto the
//! queue. [`TweenPlayer::advance`] notices a node that has left the scene and
//! ends its tracks with [`TrackEndReason::TargetGone`] without writing to the
//! vanished node, and [`TweenPlayer::finish_node`] does the same as soon as the
//! caller knows the closure happened.
//!
//! A track whose target cannot be applied (a non-finite value, or a track that
//! would leave a non-invertible transform) is refused by [`TweenPlayer::start`]
//! and never enters the live map, so the exactly-once rule and the refusal rule
//! do not fight: the caller sees the named error instead of a track that can
//! never finish usefully. Depth components are ordinary targets.

use std::collections::BTreeMap;

use gnoblin_animation::{
    Easing, EasingError, PropertyError, PropertyValue, ValueKind, VisualProperty, VisualTransform,
};
use gnoblin_types::{NodeId, TrackId};

use crate::scene::{Scene, SceneError};

/// Where a track starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackStart {
    /// The value the transform carries when the track starts, which keeps a
    /// superseded or retargeted animation continuous.
    Current,
    /// An explicit value, refused when its kind does not match the target's.
    Value(PropertyValue),
}

/// A track to start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackSpec {
    /// The node the track animates.
    pub node: NodeId,
    /// The property.
    pub property: VisualProperty,
    /// Where the value starts.
    pub from: TrackStart,
    /// The value it ends at.
    pub to: PropertyValue,
    /// Length in milliseconds; zero completes on the first advance.
    pub duration_ms: u32,
    /// Delay before the animation starts, in milliseconds.
    pub delay_ms: u32,
    /// The timing curve.
    pub easing: Easing,
}

impl TrackSpec {
    /// A track from the current value to `to` with no delay and a linear curve.
    ///
    /// A caller sets the fields it needs; the initial duration is zero so a
    /// forgotten duration completes instead of holding a node mid-animation.
    #[must_use]
    pub fn new(node: NodeId, property: VisualProperty, to: PropertyValue) -> Self {
        Self {
            node,
            property,
            from: TrackStart::Current,
            to,
            duration_ms: 0,
            delay_ms: 0,
            easing: Easing::Linear,
        }
    }
}

/// Why a track ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackEndReason {
    /// It reached its duration and its target value is on the node.
    Completed,
    /// The caller cancelled it; the value stays where it was.
    Cancelled,
    /// A newer track took the property; the value stays where it was.
    Superseded,
    /// The caller retargeted it; a replacement track continues from the value
    /// that was on the node.
    Retargeted,
    /// The node left the scene, so there was nothing left to animate.
    TargetGone,
}

/// The single terminal report a started track produces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackEnd {
    /// The track.
    pub track: TrackId,
    /// The node it animated.
    pub node: NodeId,
    /// The property it animated.
    pub property: VisualProperty,
    /// Why it ended.
    pub reason: TrackEndReason,
    /// The value last applied to the node, or the target value after
    /// completion.
    pub value: PropertyValue,
}

/// A refused track operation.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackError {
    /// The node does not exist.
    UnknownNode(NodeId),
    /// The track does not exist or has already ended.
    UnknownTrack(TrackId),
    /// A property value was refused.
    Property(PropertyError),
    /// A component is NaN or infinite.
    NonFinite {
        /// The value's path.
        field: &'static str,
    },
    /// The curve was refused.
    Easing(EasingError),
    /// The start and target values have different kinds.
    ValueMismatch {
        /// The property.
        property: VisualProperty,
        /// The start value's kind.
        from: ValueKind,
        /// The target value's kind.
        to: ValueKind,
    },
    /// The scene refused the write.
    Scene(SceneError),
}

impl std::fmt::Display for TrackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(formatter, "no node {id}"),
            Self::UnknownTrack(id) => write!(formatter, "no track {id}"),
            Self::Property(error) => write!(formatter, "{error}"),
            Self::NonFinite { field } => write!(formatter, "`{field}` is not finite"),
            Self::Easing(error) => write!(formatter, "{error}"),
            Self::ValueMismatch { property, from, to } => write!(
                formatter,
                "`{}` cannot animate from a {from:?} value to a {to:?} value",
                property.name()
            ),
            Self::Scene(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for TrackError {}

impl From<PropertyError> for TrackError {
    fn from(error: PropertyError) -> Self {
        Self::Property(error)
    }
}

impl From<EasingError> for TrackError {
    fn from(error: EasingError) -> Self {
        Self::Easing(error)
    }
}

impl From<SceneError> for TrackError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

/// One live track.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    node: NodeId,
    property: VisualProperty,
    from: PropertyValue,
    to: PropertyValue,
    duration_ms: u32,
    delay_ms: u32,
    elapsed_ms: u64,
    easing: Easing,
    applied: PropertyValue,
}

impl Track {
    /// Consumes the track and builds the record reported when it ends. It carries
    /// the last applied value, so the caller sees where the property stopped.
    fn end(self, track: TrackId, reason: TrackEndReason) -> TrackEnd {
        TrackEnd {
            track,
            node: self.node,
            property: self.property,
            reason,
            value: self.applied,
        }
    }
}

/// The scene's animation player.
#[derive(Debug, Default)]
pub struct TweenPlayer {
    next_track: u64,
    tracks: BTreeMap<TrackId, Track>,
    ended: Vec<TrackEnd>,
}

impl TweenPlayer {
    /// An empty player.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a track, refusing a node, value or curve this build cannot animate.
    ///
    /// The starting value is applied immediately, so a delayed track holds the
    /// start value rather than the node's previous value.
    pub fn start(&mut self, scene: &mut Scene, spec: TrackSpec) -> Result<TrackId, TrackError> {
        let resolved = self.validate(scene, &spec)?;
        Ok(self.begin(scene, spec, resolved))
    }

    /// Advance every live track by the time that passed.
    ///
    /// Tracks are visited in ascending id order. A track whose node has left the
    /// scene ends without a write, which is how a closed window's animation ends
    /// even though nobody told the player about the closure.
    pub fn advance(&mut self, scene: &mut Scene, elapsed_ms: u32) {
        let ids: Vec<TrackId> = self.tracks.keys().copied().collect();
        for id in ids {
            let Some(mut track) = self.tracks.remove(&id) else {
                continue;
            };
            if !scene.contains(track.node) {
                self.ended.push(track.end(id, TrackEndReason::TargetGone));
                continue;
            }
            track.elapsed_ms = track.elapsed_ms.saturating_add(u64::from(elapsed_ms));
            let total = u64::from(track.delay_ms) + u64::from(track.duration_ms);
            if track.elapsed_ms >= total {
                match scene.set_property(track.node, track.property, track.to) {
                    Ok(()) => {
                        track.applied = track.to;
                        self.ended.push(track.end(id, TrackEndReason::Completed));
                    }
                    Err(_) => self.ended.push(track.end(id, TrackEndReason::TargetGone)),
                }
                continue;
            }
            if track.elapsed_ms <= u64::from(track.delay_ms) {
                self.tracks.insert(id, track);
                continue;
            }
            let progress = (track.elapsed_ms - u64::from(track.delay_ms)) as f64 / f64::from(track.duration_ms);
            let eased = track.easing.apply(progress) as f32;
            let Some(value) = track.from.lerp(track.to, eased) else {
                self.ended.push(track.end(id, TrackEndReason::TargetGone));
                continue;
            };
            match scene.set_property(track.node, track.property, value) {
                Ok(()) => {
                    track.applied = value;
                    self.tracks.insert(id, track);
                }
                Err(_) => self.ended.push(track.end(id, TrackEndReason::TargetGone)),
            }
        }
    }

    /// Cancel a track, leaving the value where it is.
    pub fn cancel(&mut self, id: TrackId) -> Result<(), TrackError> {
        let track = self.tracks.remove(&id).ok_or(TrackError::UnknownTrack(id))?;
        self.ended.push(track.end(id, TrackEndReason::Cancelled));
        Ok(())
    }

    /// End every track on a node that is going away.
    ///
    /// A caller that knows about the closure uses this so the `finished` report
    /// is produced in the same turn as the closure, not at the next advance.
    pub fn finish_node(&mut self, node: NodeId) {
        let ids: Vec<TrackId> = self
            .tracks
            .iter()
            .filter(|(_, track)| track.node == node)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(track) = self.tracks.remove(&id) {
                self.ended.push(track.end(id, TrackEndReason::TargetGone));
            }
        }
    }

    /// Replace a track's target from the value currently applied.
    ///
    /// The replacement starts from `applied`, the value that is on the node right
    /// now, so retargeting cannot jump. Velocity is not preserved: the new track
    /// begins at rest on its curve, which is the predictable choice for a user
    /// action that interrupts an animation.
    pub fn retarget(
        &mut self,
        scene: &mut Scene,
        id: TrackId,
        to: PropertyValue,
        duration_ms: u32,
        easing: Easing,
    ) -> Result<TrackId, TrackError> {
        let Some(track) = self.tracks.get(&id) else {
            return Err(TrackError::UnknownTrack(id));
        };
        let (node, property, current) = (track.node, track.property, track.applied);
        if !scene.contains(node) {
            if let Some(track) = self.tracks.remove(&id) {
                self.ended.push(track.end(id, TrackEndReason::TargetGone));
            }
            return Err(TrackError::UnknownNode(node));
        }
        let spec = TrackSpec {
            node,
            property,
            from: TrackStart::Value(current),
            to,
            duration_ms,
            delay_ms: 0,
            easing,
        };
        let resolved = self.validate(scene, &spec)?;
        if let Some(track) = self.tracks.remove(&id) {
            self.ended.push(track.end(id, TrackEndReason::Retargeted));
        }
        Ok(self.begin(scene, spec, resolved))
    }

    /// Take the terminal reports produced since the last call.
    pub fn take_ended(&mut self) -> Vec<TrackEnd> {
        std::mem::take(&mut self.ended)
    }

    /// How many tracks are live.
    #[must_use]
    pub fn active(&self) -> usize {
        self.tracks.len()
    }

    /// Whether a track is live.
    #[must_use]
    pub fn is_active(&self, id: TrackId) -> bool {
        self.tracks.contains_key(&id)
    }

    /// Validate a spec against the scene without changing anything.
    fn validate(&self, scene: &Scene, spec: &TrackSpec) -> Result<(PropertyValue, PropertyValue), TrackError> {
        if !scene.contains(spec.node) {
            return Err(TrackError::UnknownNode(spec.node));
        }
        spec.easing.validate()?;
        let transform = scene.transform(spec.node).ok_or(TrackError::UnknownNode(spec.node))?;
        let to = apply_candidate(transform, spec.property, spec.to)?;
        let from = match spec.from {
            TrackStart::Current => spec.property.get(transform),
            TrackStart::Value(value) => apply_candidate(transform, spec.property, value)?,
        };
        if from.kind() != to.kind() {
            return Err(TrackError::ValueMismatch {
                property: spec.property,
                from: from.kind(),
                to: to.kind(),
            });
        }
        Ok((from, to))
    }

    /// Start a spec that [`TweenPlayer::validate`] already accepted.
    fn begin(&mut self, scene: &mut Scene, spec: TrackSpec, (from, to): (PropertyValue, PropertyValue)) -> TrackId {
        let id = TrackId::new(self.next_track);
        self.next_track = self.next_track.saturating_add(1);
        let superseded = self.supersede(spec.node, spec.property);
        for track in superseded {
            self.ended.push(track);
        }
        if scene.set_property(spec.node, spec.property, from).is_err() {
            // The candidate check above proved this writable, so a failure here
            // means the node vanished between the two steps; report it as ended
            // rather than keeping a track that can never advance.
            self.ended.push(TrackEnd {
                track: id,
                node: spec.node,
                property: spec.property,
                reason: TrackEndReason::TargetGone,
                value: from,
            });
            return id;
        }
        if spec.duration_ms == 0 && spec.delay_ms == 0 {
            let mut applied = from;
            if scene.set_property(spec.node, spec.property, to).is_ok() {
                applied = to;
            }
            self.ended.push(TrackEnd {
                track: id,
                node: spec.node,
                property: spec.property,
                reason: TrackEndReason::Completed,
                value: applied,
            });
            return id;
        }
        self.tracks.insert(
            id,
            Track {
                node: spec.node,
                property: spec.property,
                from,
                to,
                duration_ms: spec.duration_ms,
                delay_ms: spec.delay_ms,
                elapsed_ms: 0,
                easing: spec.easing,
                applied: from,
            },
        );
        id
    }

    /// End live tracks that write a component the new track will own.
    fn supersede(&mut self, node: NodeId, property: VisualProperty) -> Vec<TrackEnd> {
        let conflicting: Vec<TrackId> = self
            .tracks
            .iter()
            .filter(|(_, track)| track.node == node && track.property.conflicts_with(property))
            .map(|(id, _)| *id)
            .collect();
        let mut ended = Vec::with_capacity(conflicting.len());
        for id in conflicting {
            if let Some(track) = self.tracks.remove(&id) {
                ended.push(track.end(id, TrackEndReason::Superseded));
            }
        }
        ended
    }
}

/// Coerce a value, apply it to a copy of the transform and check the result.
///
/// The copy is the point of this helper: a refused value leaves the scene
/// untouched, so a failed write never half-applies a transform.
fn apply_candidate(
    transform: &VisualTransform,
    property: VisualProperty,
    value: PropertyValue,
) -> Result<PropertyValue, TrackError> {
    let mut candidate = *transform;
    let value = property.coerce(value)?;
    property.apply(&mut candidate, value)?;
    if !candidate.is_finite() {
        return Err(TrackError::NonFinite { field: "transform" });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_types::{OutputId, Rect};

    use crate::scene::NodeSpec;

    fn scene_with_node() -> (Scene, NodeId) {
        let mut scene = Scene::new();
        scene.set_output(OutputId::new(0), Rect::new(0, 0, 800, 600));
        let node = NodeId::new(1);
        scene
            .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("node");
        (scene, node)
    }

    fn opacity(scene: &Scene, node: NodeId) -> f32 {
        match scene.property(node, VisualProperty::Opacity).expect("opacity") {
            PropertyValue::Number(value) => value,
            _ => panic!("opacity is a number"),
        }
    }

    #[test]
    fn a_track_advances_by_caller_ticks_and_completes_exactly_once() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let id = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    easing: Easing::Linear,
                    ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0))
                },
            )
            .expect("started");
        player.advance(&mut scene, 50);
        assert!((opacity(&scene, node) - 0.5).abs() < 0.01);
        assert!(player.is_active(id));
        player.advance(&mut scene, 50);
        assert_eq!(opacity(&scene, node), 0.0);
        assert!(!player.is_active(id));
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].reason, TrackEndReason::Completed);
        assert_eq!(ended[0].track, id);
        assert!(player.take_ended().is_empty());
    }

    #[test]
    fn closing_a_node_finishes_its_tracks_without_writing_to_it() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let id = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0))
                },
            )
            .expect("started");
        scene.remove(node).expect("removed");
        player.advance(&mut scene, 16);
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].track, id);
        assert_eq!(ended[0].reason, TrackEndReason::TargetGone);
        assert_eq!(player.active(), 0);
        player.advance(&mut scene, 100);
        assert!(player.take_ended().is_empty(), "a track reports only once");
    }

    #[test]
    fn finish_node_reports_a_closure_in_the_same_turn() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0))
                },
            )
            .expect("started");
        player.finish_node(node);
        assert_eq!(player.take_ended().len(), 1);
        assert_eq!(player.active(), 0);
    }

    #[test]
    fn a_new_track_supersedes_the_previous_one_deterministically() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let first = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0))
                },
            )
            .expect("first");
        player.advance(&mut scene, 50);
        let midpoint = opacity(&scene, node);
        let second = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(1.0))
                },
            )
            .expect("second");
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].track, first);
        assert_eq!(ended[0].reason, TrackEndReason::Superseded);
        assert_eq!(ended[0].value, PropertyValue::number(midpoint));
        assert_eq!(player.active(), 1);
        assert!(player.is_active(second));
        player.advance(&mut scene, 100);
        assert_eq!(opacity(&scene, node), 1.0);
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].track, second);
        assert_eq!(ended[0].reason, TrackEndReason::Completed);
    }

    #[test]
    fn a_uniform_scale_track_supersedes_an_axis_track_and_the_other_axis_survives() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let axis = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::ScaleY, PropertyValue::number(2.0))
                },
            )
            .expect("axis");
        let x = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::ScaleX, PropertyValue::number(0.5))
                },
            )
            .expect("x");
        let uniform = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::Scale, PropertyValue::uniform_scale(3.0))
                },
            )
            .expect("uniform");
        let ended = player.take_ended();
        let ended_ids: Vec<TrackId> = ended.iter().map(|end| end.track).collect();
        assert_eq!(ended_ids, vec![axis, x]);
        assert!(ended.iter().all(|end| end.reason == TrackEndReason::Superseded));
        assert!(player.is_active(uniform));
        assert_eq!(player.active(), 1);
    }

    #[test]
    fn retarget_never_jumps_the_value() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let first = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0))
                },
            )
            .expect("started");
        player.advance(&mut scene, 50);
        let before = opacity(&scene, node);
        let second = player
            .retarget(&mut scene, first, PropertyValue::number(1.0), 100, Easing::Linear)
            .expect("retargeted");
        assert_eq!(opacity(&scene, node), before, "retarget applies no time");
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].track, first);
        assert_eq!(ended[0].reason, TrackEndReason::Retargeted);
        assert_eq!(ended[0].value, PropertyValue::number(before));
        player.advance(&mut scene, 0);
        assert_eq!(opacity(&scene, node), before);
        player.advance(&mut scene, 100);
        assert_eq!(opacity(&scene, node), 1.0);
        assert!(
            player
                .take_ended()
                .iter()
                .any(|end| end.track == second && end.reason == TrackEndReason::Completed)
        );
    }

    #[test]
    fn cancel_reports_once_and_leaves_the_value() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let id = player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0))
                },
            )
            .expect("started");
        player.advance(&mut scene, 40);
        let before = opacity(&scene, node);
        player.cancel(id).expect("cancelled");
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].reason, TrackEndReason::Cancelled);
        assert_eq!(ended[0].value, PropertyValue::number(before));
        player.advance(&mut scene, 100);
        assert_eq!(opacity(&scene, node), before);
        assert!(player.take_ended().is_empty());
        assert_eq!(
            player.cancel(id).expect_err("already ended"),
            TrackError::UnknownTrack(id)
        );
    }

    #[test]
    fn a_depth_track_starts_and_reaches_its_target() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 100,
                    ..TrackSpec::new(node, VisualProperty::OffsetZ, PropertyValue::number(5.0))
                },
            )
            .expect("a depth track is accepted");
        assert_eq!(player.active(), 1);
        player.advance(&mut scene, 100);
        assert_eq!(player.active(), 0);
        assert_eq!(
            scene.property(node, VisualProperty::OffsetZ).expect("readable"),
            PropertyValue::number(5.0)
        );
    }

    #[test]
    fn a_target_of_the_wrong_kind_is_refused() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let error = player
            .start(
                &mut scene,
                TrackSpec::new(node, VisualProperty::Tint, PropertyValue::number(0.5)),
            )
            .expect_err("must refuse");
        assert!(matches!(error, TrackError::Property(PropertyError::WrongKind { .. })));
    }

    #[test]
    fn a_zero_duration_track_completes_at_start() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let id = player
            .start(
                &mut scene,
                TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.25)),
            )
            .expect("started");
        assert_eq!(opacity(&scene, node), 0.25);
        assert_eq!(player.active(), 0);
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].track, id);
        assert_eq!(ended[0].reason, TrackEndReason::Completed);
    }

    #[test]
    fn a_delayed_track_holds_its_start_value_until_the_delay_passes() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        let mut spec = TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0));
        spec.duration_ms = 100;
        spec.delay_ms = 50;
        player.start(&mut scene, spec).expect("started");
        player.advance(&mut scene, 50);
        assert_eq!(opacity(&scene, node), 1.0, "still at the start value");
        player.advance(&mut scene, 50);
        assert!((opacity(&scene, node) - 0.5).abs() < 0.01);
    }

    #[test]
    fn an_unknown_node_or_track_is_refused() {
        let (mut scene, _) = scene_with_node();
        let mut player = TweenPlayer::new();
        assert_eq!(
            player
                .start(
                    &mut scene,
                    TrackSpec::new(NodeId::new(99), VisualProperty::Opacity, PropertyValue::number(0.0))
                )
                .expect_err("unknown node"),
            TrackError::UnknownNode(NodeId::new(99))
        );
        assert_eq!(
            player
                .retarget(
                    &mut scene,
                    TrackId::new(7),
                    PropertyValue::number(0.0),
                    10,
                    Easing::Linear
                )
                .expect_err("unknown track"),
            TrackError::UnknownTrack(TrackId::new(7))
        );
    }

    #[test]
    fn a_translation_track_writes_the_axis_it_was_started_for() {
        let (mut scene, node) = scene_with_node();
        let mut player = TweenPlayer::new();
        player
            .start(
                &mut scene,
                TrackSpec {
                    duration_ms: 10,
                    ..TrackSpec::new(node, VisualProperty::OffsetX, PropertyValue::number(20.0))
                },
            )
            .expect("started");
        // The start value is applied immediately; the target lands at the end.
        assert_eq!(scene.transform(node).expect("transform").base.offset.x, 0.0);
        player.advance(&mut scene, 10);
        assert_eq!(scene.transform(node).expect("transform").base.offset.x, 20.0);
        assert_eq!(scene.transform(node).expect("transform").base.offset.y, 0.0);
    }
}
