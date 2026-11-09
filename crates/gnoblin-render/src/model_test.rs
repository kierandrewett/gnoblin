//! The model test: random event sequences with hand-written invariants.
//!
//! The generator is a fixed-seed xorshift, so a failure is reproducible from the
//! seed alone and the sequence never depends on time, threads or the filesystem.
//! The invariants are the two the plan cares about most:
//!
//! 1. Damage covers every node whose projected bounds changed, before and after
//!    the change, clipped to each output.
//! 2. Every started track produces exactly one terminal report, for its own id,
//!    with the node and property it was started for.
//!
//! It also exercises the depth paths: a depth component written through the
//! scene or targeted by a track is accepted, and a value that would make the
//! transform non-finite is refused and leaves the transform untouched, which the
//! damage check then judges as "nothing changed".

use std::collections::BTreeMap;

use gnoblin_animation::{Easing, PropertyValue, VisualProperty};
use gnoblin_types::{Color, NodeId, OutputId, Rect, TrackId, Vec3};

use crate::{NodeSpec, Scene, TrackSpec, TrackStart, TweenPlayer};

/// The seed; the same sequence runs on every build.
const SEED: u64 = 0x5EED_1234_9E37_79B9;

/// How many actions the model performs.
const STEPS: usize = 600;

/// The largest number of live tracks a scene may hold in the model.
const MAX_NODES: usize = 8;

/// The in-plane properties the model animates and writes.
const IN_PLANE: [VisualProperty; 9] = [
    VisualProperty::Opacity,
    VisualProperty::OffsetX,
    VisualProperty::OffsetY,
    VisualProperty::RotationZ,
    VisualProperty::Scale,
    VisualProperty::ScaleX,
    VisualProperty::ScaleY,
    VisualProperty::CornerRadius,
    VisualProperty::Tint,
];

/// A deterministic xorshift generator.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next() % bound as u64) as usize
        }
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    fn coordinate(&mut self, low: f64, high: f64) -> f32 {
        self.range(low, high) as f32
    }
}

fn snapshot(scene: &Scene) -> BTreeMap<NodeId, Rect> {
    scene
        .node_ids()
        .filter_map(|id| scene.absolute_bounds(id).map(|rect| (id, rect)))
        .collect()
}

fn random_value(rng: &mut Rng, property: VisualProperty) -> PropertyValue {
    match property {
        VisualProperty::Opacity => PropertyValue::number(rng.coordinate(0.0, 1.0)),
        VisualProperty::OffsetX | VisualProperty::OffsetY => PropertyValue::number(rng.coordinate(-300.0, 900.0)),
        VisualProperty::RotationZ => PropertyValue::number(rng.coordinate(-180.0, 180.0)),
        VisualProperty::Scale => PropertyValue::uniform_scale(rng.coordinate(0.2, 2.5)),
        VisualProperty::ScaleX | VisualProperty::ScaleY => PropertyValue::number(rng.coordinate(0.2, 2.5)),
        VisualProperty::CornerRadius => PropertyValue::number(rng.coordinate(0.0, 40.0)),
        VisualProperty::Tint => PropertyValue::color(Color::new(
            rng.coordinate(0.0, 1.0),
            rng.coordinate(0.0, 1.0),
            rng.coordinate(0.0, 1.0),
            rng.coordinate(0.0, 1.0),
        )),
        // Depth components are ordinary targets: the model writes them through
        // the same path as every other property.
        _ => PropertyValue::number(0.0),
    }
}

fn random_easing(rng: &mut Rng) -> Easing {
    match rng.below(5) {
        0 => Easing::Linear,
        1 => Easing::EaseOutCubic,
        2 => Easing::EaseInOutCubic,
        3 => Easing::Spring {
            stiffness: rng.range(60.0, 300.0),
            damping: rng.range(2.0, 25.0),
            mass: rng.range(0.5, 2.0),
        },
        _ => Easing::CubicBezier {
            x1: rng.unit(),
            y1: rng.range(-1.5, 1.5),
            x2: rng.unit(),
            y2: rng.range(-1.5, 1.5),
        },
    }
}

#[test]
fn the_scene_and_the_player_keep_their_invariants() {
    let mut rng = Rng::new(SEED);
    let mut scene = Scene::new();
    scene.set_output(OutputId::new(0), Rect::new(0, 0, 1280, 720));
    scene.set_output(OutputId::new(1), Rect::new(1280, 0, 2560, 1440));
    let mut player = TweenPlayer::new();
    let mut nodes: Vec<NodeId> = Vec::new();
    let mut next_node: u64 = 0;
    let mut started: BTreeMap<TrackId, (NodeId, VisualProperty)> = BTreeMap::new();
    let mut ended: BTreeMap<TrackId, usize> = BTreeMap::new();
    let mut mutations: u64 = 0;
    let mut changed_bound_checks: u64 = 0;

    for _ in 0..STEPS {
        let before = snapshot(&scene);
        let action = rng.below(10);
        match action {
            0 | 1 => {
                if nodes.len() < MAX_NODES {
                    let id = NodeId::new(next_node);
                    next_node += 1;
                    let parent = if nodes.is_empty() || rng.below(2) == 0 {
                        None
                    } else {
                        Some(nodes[rng.below(nodes.len())])
                    };
                    let mut spec = NodeSpec::new(Rect::new(
                        rng.below(200) as i32,
                        rng.below(200) as i32,
                        20 + rng.below(300) as u32,
                        20 + rng.below(300) as u32,
                    ));
                    spec.parent = parent;
                    spec.transform.base.offset = Vec3::new(rng.range(-100.0, 300.0), rng.range(-100.0, 300.0), 0.0);
                    spec.transform.base.scale = Vec3::new(rng.range(0.5, 1.5), rng.range(0.5, 1.5), 1.0);
                    spec.transform.base.rotation_degrees = Vec3::new(0.0, 0.0, rng.range(-30.0, 30.0));
                    spec.transform.opacity = rng.coordinate(0.2, 1.0);
                    scene.insert(id, spec).expect("the generator builds a valid node");
                    nodes.push(id);
                    mutations += 1;
                }
            }
            2 => {
                if !nodes.is_empty() {
                    let id = nodes[rng.below(nodes.len())];
                    let removed = scene.remove(id).expect("the node exists");
                    nodes.retain(|candidate| !removed.contains(candidate));
                    mutations += 1;
                }
            }
            3 => {
                if !nodes.is_empty() {
                    let id = nodes[rng.below(nodes.len())];
                    let mut transform = *scene.transform(id).expect("the node exists");
                    transform.base.offset = Vec3::new(rng.range(-400.0, 900.0), rng.range(-400.0, 900.0), 0.0);
                    transform.base.scale = Vec3::new(rng.range(0.3, 2.5), rng.range(0.3, 2.5), 1.0);
                    transform.base.rotation_degrees = Vec3::new(0.0, 0.0, rng.range(-180.0, 180.0));
                    scene.set_transform(id, transform).expect("the transform is 2D");
                    mutations += 1;
                }
            }
            4 => {
                if !nodes.is_empty() {
                    let id = nodes[rng.below(nodes.len())];
                    if rng.below(8) == 0 {
                        scene
                            .set_property(id, VisualProperty::OffsetZ, PropertyValue::number(1.0))
                            .expect("a depth write is accepted");
                    } else {
                        let property = IN_PLANE[rng.below(IN_PLANE.len())];
                        let value = random_value(&mut rng, property);
                        scene
                            .set_property(id, property, value)
                            .expect("an in-plane property is accepted");
                    }
                    mutations += 1;
                }
            }
            5 => {
                if !nodes.is_empty() {
                    let id = nodes[rng.below(nodes.len())];
                    if rng.below(10) == 0 {
                        let track = player
                            .start(
                                &mut scene,
                                TrackSpec::new(id, VisualProperty::ScaleZ, PropertyValue::number(2.0)),
                            )
                            .expect("a depth track is accepted");
                        assert_eq!(
                            started.insert(track, (id, VisualProperty::ScaleZ)),
                            None,
                            "track ids are unique"
                        );
                    } else {
                        let property = IN_PLANE[rng.below(IN_PLANE.len())];
                        let mut spec = TrackSpec::new(id, property, random_value(&mut rng, property));
                        spec.duration_ms = rng.below(300) as u32;
                        spec.delay_ms = rng.below(30) as u32;
                        spec.easing = random_easing(&mut rng);
                        if rng.below(3) == 0 {
                            spec.from = TrackStart::Value(random_value(&mut rng, property));
                        }
                        let track = player.start(&mut scene, spec).expect("the generated track is valid");
                        assert_eq!(started.insert(track, (id, property)), None, "track ids are unique");
                    }
                    mutations += 1;
                }
            }
            6 => {
                let elapsed = 1 + rng.below(32) as u32;
                player.advance(&mut scene, elapsed);
            }
            7 => {
                let live: Vec<TrackId> = started
                    .keys()
                    .copied()
                    .filter(|track| player.is_active(*track))
                    .collect();
                if !live.is_empty() {
                    let track = live[rng.below(live.len())];
                    player.cancel(track).expect("the track is live");
                }
            }
            8 => {
                let live: Vec<TrackId> = started
                    .keys()
                    .copied()
                    .filter(|track| player.is_active(*track))
                    .collect();
                if !live.is_empty() {
                    let track = live[rng.below(live.len())];
                    let (node, property) = started[&track];
                    let mut spec = TrackSpec::new(node, property, random_value(&mut rng, property));
                    spec.duration_ms = rng.below(200) as u32;
                    spec.easing = random_easing(&mut rng);
                    // The node may have been removed; the old track then ends
                    // with `TargetGone` and there is nothing to retarget.
                    if let Ok(replacement) = player.retarget(&mut scene, track, spec.to, spec.duration_ms, spec.easing)
                    {
                        assert_eq!(started.insert(replacement, (node, property)), None);
                    }
                }
            }
            _ => {
                if !nodes.is_empty() {
                    let id = nodes[rng.below(nodes.len())];
                    if rng.below(2) == 0 {
                        scene.raise(id).expect("the node exists");
                    } else {
                        scene.lower(id).expect("the node exists");
                    }
                    mutations += 1;
                }
            }
        }

        // 1. Damage covers every changed node's bounds, before and after.
        let after = snapshot(&scene);
        let damage = scene.take_damage();
        let mut ids: Vec<NodeId> = before.keys().chain(after.keys()).copied().collect();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            let old = before.get(&id);
            let new = after.get(&id);
            if old == new {
                continue;
            }
            for rect in old.into_iter().chain(new) {
                for (output, output_rect) in scene.outputs() {
                    let Some(clipped) = rect.intersection(*output_rect) else {
                        continue;
                    };
                    let entry = damage
                        .for_output(*output)
                        .unwrap_or_else(|| panic!("no damage for {output} after {id} changed from {old:?} to {new:?}"));
                    assert!(
                        entry.covers(clipped),
                        "damage for {output} misses {clipped:?} of node {id} \
                         (old {old:?}, new {new:?}, rects {:?})",
                        entry.rects
                    );
                }
            }
            changed_bound_checks += 1;
        }

        // 2. Every terminal report belongs to a started track and arrives once.
        for end in player.take_ended() {
            assert_eq!(
                ended.get(&end.track).copied().unwrap_or(0),
                0,
                "track {} reported twice",
                end.track
            );
            ended.insert(end.track, 1);
            let (node, property) = *started
                .get(&end.track)
                .unwrap_or_else(|| panic!("track {} ended without starting", end.track));
            assert_eq!(node, end.node, "track {} reported the wrong node", end.track);
            assert_eq!(
                property, end.property,
                "track {} reported the wrong property",
                end.track
            );
            assert!(end.value.is_finite(), "track {} ended on a non-finite value", end.track);
        }
    }

    // Every track still live at the end is cancelled, so the exactly-once rule is
    // checked for the whole run, not only for tracks that happened to finish.
    let live: Vec<TrackId> = started
        .keys()
        .copied()
        .filter(|track| player.is_active(*track))
        .collect();
    for track in live {
        player.cancel(track).expect("the track is live");
    }
    for end in player.take_ended() {
        assert_eq!(
            ended.get(&end.track).copied().unwrap_or(0),
            0,
            "track {} reported twice",
            end.track
        );
        ended.insert(end.track, 1);
        assert!(started.contains_key(&end.track));
    }

    assert_eq!(player.active(), 0, "every track ends");
    assert_eq!(started.len(), ended.len(), "every started track reports exactly once");
    for track in started.keys() {
        assert_eq!(ended.get(track), Some(&1), "track {track} did not report");
    }
    for id in scene.node_ids() {
        assert!(
            scene.transform(id).expect("node").is_finite(),
            "node {id} holds a non-finite transform"
        );
    }
    assert!(mutations > 0 && changed_bound_checks > 0, "the model exercised nothing");

    eprintln!(
        "[model] seed={SEED:#x} steps={STEPS} mutations={mutations} bound-changes={changed_bound_checks} \
         tracks-started={} tracks-ended={}",
        started.len(),
        ended.len()
    );
}
