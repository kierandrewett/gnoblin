//! The head tracker, with the clock and the tree as values: no owner, no runtime and no sleeping.

use std::time::{Duration, Instant};

use xr_scene::head::{CameraPose, camera_pose};
use xr_scene::rig::eyes;
use xr_scene::{IDENTITY, Layer, NodeId, Placement, Pose, Projected, Rig, Space};

use super::{
    BUDGET_RETRY, Budget, CameraState, Driven, IN_FLIGHT_TIMEOUT, Input, MAX_WRITES_PER_SECOND, MIN_INTERVAL,
    REFUSED_BACKOFF, STALE_AFTER, Sample, Status, Step, Tracker, Write,
};

const METRES: f64 = 0.001;
const HEAD: NodeId = 40;
const EXTENT: [f64; 2] = [1600.0, 900.0];

fn head_at(x: f64, y: f64, z: f64) -> Pose {
    Pose {
        position: [x, y, z],
        orientation: IDENTITY,
    }
}

/// A turn of `degrees` about the vertical axis.
fn yawed(x: f64, degrees: f64) -> Pose {
    let half = degrees.to_radians() / 2.0;
    Pose {
        position: [x, 1.6, 0.0],
        orientation: [0.0, half.sin(), 0.0, half.cos()],
    }
}

/// The head camera as the tree holds it when it holds `pose`, or at the origin with `None`.
fn camera_holding(pose: Option<CameraPose>) -> CameraState {
    let pose = pose.unwrap_or(CameraPose {
        position: [0.0; 3],
        rotation_degrees: [0.0; 3],
    });
    CameraState {
        id: HEAD,
        metres: METRES,
        rig: Rig::default(),
        position: pose.position,
        rotation_degrees: pose.rotation_degrees,
        layer_budget: None,
    }
}

/// A View drawn through `camera`, with the eyes the planner states for it and the picture `serial`.
fn driven(view: NodeId, serial: Option<u64>, camera: &CameraState) -> Driven {
    Driven {
        view,
        extent: EXTENT,
        serial,
        eyes: eyes(
            camera.position,
            camera.rotation_degrees,
            camera.metres,
            &camera.rig,
            EXTENT,
        )
        .expect("eyes"),
    }
}

/// A small world to step the tracker in: the clock, the tree's camera and the pictures of the driven Views.
struct World {
    start: Instant,
    elapsed: Duration,
    camera: CameraState,
    serials: Vec<(NodeId, Option<u64>)>,
    tracker: Tracker,
    writes: Vec<(Duration, Write)>,
}

impl World {
    fn new(views: &[NodeId]) -> Self {
        Self {
            start: Instant::now(),
            elapsed: Duration::ZERO,
            camera: camera_holding(None),
            serials: views.iter().map(|view| (*view, Some(1))).collect(),
            tracker: Tracker::default(),
            writes: Vec::new(),
        }
    }

    fn now(&self) -> Instant {
        self.start + self.elapsed
    }

    fn advance(&mut self, by: Duration) {
        self.elapsed += by;
    }

    fn driven(&self) -> Vec<Driven> {
        self.serials
            .iter()
            .map(|(view, serial)| driven(*view, *serial, &self.camera))
            .collect()
    }

    /// Steps with the head measured `age` ago.
    fn step_aged(&mut self, head: Option<Pose>, age: Duration) -> Step {
        let driven = self.driven();
        let now = self.now();
        let step = self.tracker.step(&Input {
            now,
            sample: Sample {
                pose: head,
                measured: now - age,
            },
            camera: Some(self.camera),
            driven: &driven,
        });
        if let Some(write) = step.write {
            self.tracker.wrote(now, &write, &driven);
            self.writes.push((self.elapsed, write));
        }
        step
    }

    fn step(&mut self, head: Pose) -> Step {
        self.step_aged(Some(head), Duration::ZERO)
    }

    /// The owner applied the last write: the tree's camera holds it.
    fn show(&mut self) {
        let (_, write) = self.writes.last().expect("a write");
        self.camera = camera_holding(Some(write.pose));
    }

    /// Every driven View has a new picture.
    fn draw(&mut self) {
        for (_, serial) in &mut self.serials {
            *serial = Some(serial.unwrap_or(0) + 1);
        }
    }
}

#[test]
fn a_fresh_pose_is_written_in_the_cameras_units() {
    let mut world = World::new(&[1]);
    let step = world.step(head_at(0.5, 1.6, -2.0));
    let write = step.write.expect("a write");
    assert_eq!(write.camera, HEAD);
    assert_eq!(write.pose.position, [500.0, -1600.0, -2000.0]);
    assert_eq!(step.status, Status::Following);
}

#[test]
fn nothing_is_written_while_no_view_draws_through_the_camera() {
    let mut tracker = Tracker::default();
    let now = Instant::now();
    let step = tracker.step(&Input {
        now,
        sample: Sample {
            pose: Some(head_at(0.0, 1.6, 0.0)),
            measured: now,
        },
        camera: Some(camera_holding(None)),
        driven: &[],
    });
    assert_eq!((step.write, step.status), (None, Status::NoViews));
}

#[test]
fn nothing_is_written_when_the_tree_has_no_head_camera() {
    let mut tracker = Tracker::default();
    let now = Instant::now();
    let step = tracker.step(&Input {
        now,
        sample: Sample {
            pose: Some(head_at(0.0, 1.6, 0.0)),
            measured: now,
        },
        camera: None,
        driven: &[],
    });
    assert_eq!((step.write, step.status), (None, Status::NoCamera));
}

#[test]
fn a_stale_pose_is_dropped_and_the_next_fresh_one_is_written() {
    let mut world = World::new(&[1]);
    let step = world.step_aged(Some(head_at(0.0, 1.6, 0.0)), STALE_AFTER + Duration::from_millis(1));
    assert_eq!((step.write, step.status), (None, Status::Stale));
    assert!(world.writes.is_empty());
    // Exactly the limit is still fresh enough.
    let step = world.step_aged(Some(head_at(0.0, 1.6, 0.0)), STALE_AFTER);
    assert!(step.write.is_some(), "{step:?}");
}

#[test]
fn a_head_the_runtime_does_not_track_writes_nothing() {
    let mut world = World::new(&[1]);
    let step = world.step_aged(None, Duration::ZERO);
    assert_eq!((step.write, step.status), (None, Status::Untracked));
}

#[test]
fn a_pose_that_is_not_usable_is_never_written() {
    let mut world = World::new(&[1]);
    let unusable = [
        head_at(f64::NAN, 1.6, 0.0),
        head_at(0.0, f64::INFINITY, 0.0),
        head_at(0.0, 1.6, f64::NEG_INFINITY),
        Pose {
            position: [0.0; 3],
            orientation: [f64::NAN, 0.0, 0.0, 1.0],
        },
        Pose {
            position: [0.0; 3],
            orientation: [0.0; 4],
        },
        head_at(1e300, 0.0, 0.0),
    ];
    for head in unusable {
        let step = world.step(head);
        assert_eq!((step.write, step.status), (None, Status::Unusable), "{head:?}");
        world.advance(MIN_INTERVAL);
    }
    assert!(world.writes.is_empty());
    assert!(
        world.step(head_at(0.0, 1.6, 0.0)).write.is_some(),
        "a usable pose goes out after"
    );
}

#[test]
fn a_burst_of_poses_is_written_no_faster_than_the_cap() {
    // The head moves every millisecond for two seconds, the owner applies each write at once and the compositor
    // draws at once: the only thing that holds the extension back is its own cap.
    let mut world = World::new(&[1, 2]);
    for millisecond in 0..2000_u32 {
        world.advance(Duration::from_millis(1));
        let step = world.step(yawed(f64::from(millisecond) * 0.001, f64::from(millisecond) * 0.05));
        if step.write.is_some() {
            world.show();
            world.draw();
        }
    }
    let writes = world.writes.len();
    assert!(writes > 0);
    assert!(
        writes <= 2 * MAX_WRITES_PER_SECOND as usize + 1,
        "{writes} writes in two seconds"
    );
    for pair in world.writes.windows(2) {
        assert!(pair[1].0 - pair[0].0 >= MIN_INTERVAL, "{pair:?}");
    }
}

#[test]
fn one_pose_stays_in_flight_until_it_is_shown_in_the_tree_and_drawn() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    world.advance(MIN_INTERVAL * 2);
    // Neither shown nor drawn.
    assert_eq!(world.step(head_at(0.1, 1.6, 0.0)).status, Status::Waiting);
    // Shown, not drawn.
    world.show();
    world.advance(MIN_INTERVAL);
    assert_eq!(world.step(head_at(0.1, 1.6, 0.0)).status, Status::Waiting);
    // Drawn: the next pose may go.
    world.draw();
    let step = world.step(head_at(0.1, 1.6, 0.0));
    assert!(step.write.is_some(), "{step:?}");
    assert_eq!(world.writes.len(), 2);
}

#[test]
fn a_pose_that_is_drawn_before_it_is_shown_still_waits_for_the_tree() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    world.draw();
    world.advance(MIN_INTERVAL * 2);
    assert_eq!(world.step(head_at(0.1, 1.6, 0.0)).status, Status::Waiting);
    world.show();
    world.advance(MIN_INTERVAL);
    assert!(world.step(head_at(0.1, 1.6, 0.0)).write.is_some());
}

#[test]
fn a_write_that_is_never_drawn_is_given_up_after_the_timeout() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    world.show();
    world.advance(IN_FLIGHT_TIMEOUT - Duration::from_millis(1));
    assert_eq!(world.step(head_at(0.1, 1.6, 0.0)).status, Status::Waiting);
    world.advance(Duration::from_millis(1));
    let step = world.step(head_at(0.1, 1.6, 0.0));
    assert_eq!(step.status, Status::Lost);
    assert!(step.write.is_some(), "the next pose goes out");
}

#[test]
fn a_refused_write_backs_off_and_is_not_left_in_flight() {
    let mut world = World::new(&[1]);
    let step = world.step(head_at(0.0, 1.6, 0.0));
    assert!(step.write.is_some());
    let now = world.now();
    world.tracker.refused(now);
    world.advance(MIN_INTERVAL * 2);
    assert_eq!(world.step(head_at(0.1, 1.6, 0.0)).status, Status::Refused);
    world.advance(REFUSED_BACKOFF);
    let step = world.step(head_at(0.1, 1.6, 0.0));
    assert!(
        step.write.is_some(),
        "after the back-off the extension tries again: {step:?}"
    );
    assert_eq!(
        world.writes.len(),
        2,
        "one refused write and the retry, nothing between"
    );
}

#[test]
fn a_write_built_on_a_tree_that_had_moved_on_is_tried_again_at_once_without_a_back_off() {
    let mut world = World::new(&[1]);
    assert!(world.step(head_at(0.0, 1.6, 0.0)).write.is_some());
    let now = world.now();
    world.tracker.lagged(now);
    // Still inside the rate cap: it waits for that and for nothing else.
    world.advance(MIN_INTERVAL / 2);
    assert_eq!(world.step(head_at(0.1, 1.6, 0.0)).status, Status::Throttled);
    world.advance(MIN_INTERVAL);
    let step = world.step(head_at(0.1, 1.6, 0.0));
    assert!(step.write.is_some(), "{step:?}");
    assert_eq!(world.writes.len(), 2);
}

#[test]
fn a_refusal_belongs_to_the_camera_it_was_given_for() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    let now = world.now();
    world.tracker.refused(now);
    world.advance(MIN_INTERVAL * 2);
    assert_eq!(world.step(head_at(0.1, 1.6, 0.0)).status, Status::Refused);
    // The refused camera is destroyed and the shell makes another: the new one is written at once.
    world.camera = CameraState {
        id: HEAD + 1,
        ..camera_holding(None)
    };
    let step = world.step(head_at(0.1, 1.6, 0.0));
    assert_eq!(step.write.map(|write| write.camera), Some(HEAD + 1), "{step:?}");
}

#[test]
fn a_camera_destroyed_while_a_write_is_in_flight_is_forgotten_and_a_new_one_is_followed() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    // The room is taken down: the camera is gone.
    let now = world.now();
    let gone = world.tracker.step(&Input {
        now,
        sample: Sample {
            pose: Some(head_at(0.0, 1.6, 0.0)),
            measured: now,
        },
        camera: None,
        driven: &[],
    });
    assert_eq!((gone.write, gone.status), (None, Status::NoCamera));
    // The shell builds a room again, with a new camera at the origin.
    world.advance(MIN_INTERVAL * 2);
    world.camera = CameraState {
        id: HEAD + 100,
        ..camera_holding(None)
    };
    let step = world.step(head_at(0.0, 1.6, 0.0));
    let write = step
        .write
        .expect("the new camera is written at once: nothing is in flight for it");
    assert_eq!(write.camera, HEAD + 100);
}

#[test]
fn a_camera_replaced_by_another_node_drops_what_was_paired() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    world.show();
    world.draw();
    world.advance(MIN_INTERVAL);
    world.step(head_at(0.0, 1.6, 0.0));
    let mut layers = [projection(1, HEAD)];
    world.tracker.pair(&mut layers);
    let paired = eyes_of(&layers[0]);
    // A different camera node takes the name: the old pairing must not be applied to a layer that names it.
    world.camera = CameraState {
        id: HEAD + 1,
        ..camera_holding(None)
    };
    world.step(head_at(0.0, 1.6, 0.0));
    let mut layers = [projection(1, HEAD + 1)];
    world.tracker.pair(&mut layers);
    assert_ne!(eyes_of(&layers[0]), paired);
}

fn placement(id: NodeId) -> Placement {
    Placement {
        id,
        source: 100 + id,
        pose: Pose {
            position: [0.0, 0.0, -1.0],
            orientation: IDENTITY,
        },
        size: [1.6, 0.9],
        extent: EXTENT,
        opacity: 1.0,
        space: Space::World,
        input: true,
    }
}

/// A projection layer for `view` through `camera`, with the eyes of a camera at the origin.
fn projection(view: NodeId, camera: NodeId) -> Layer {
    Layer::Projection(Projected {
        placement: placement(view),
        camera,
        eyes: eyes([0.0; 3], [0.0; 3], METRES, &Rig::default(), EXTENT).expect("eyes"),
    })
}

fn eyes_of(layer: &Layer) -> [xr_scene::Eye; 2] {
    match layer {
        Layer::Projection(projected) => projected.eyes,
        Layer::Quad(_) => panic!("a quad has no eyes"),
    }
}

/// The eyes of the camera holding `pose`.
fn eyes_at(pose: &CameraPose) -> [xr_scene::Eye; 2] {
    eyes(pose.position, pose.rotation_degrees, METRES, &Rig::default(), EXTENT).expect("eyes")
}

#[test]
fn two_views_through_the_camera_follow_one_pose_and_both_must_draw_before_the_next() {
    let mut world = World::new(&[1, 2]);
    let first = world.step(head_at(0.3, 1.6, 0.0)).write.expect("a write");
    world.show();
    // One View draws, the other has not: the pose is still in flight.
    world.serials[0].1 = Some(2);
    world.advance(MIN_INTERVAL * 2);
    assert_eq!(world.step(head_at(0.4, 1.6, 0.0)).status, Status::Waiting);
    world.serials[1].1 = Some(2);
    // Both drawn.
    let second = world
        .step(head_at(0.4, 1.6, 0.0))
        .write
        .expect("the next pose goes out");
    assert_ne!(first.pose, second.pose);
    assert_eq!(world.writes.len(), 2, "one write each time, not one per View");

    // The two layers were drawn from the first pose, and both are paired with it.
    let mut layers = [projection(1, HEAD), projection(2, HEAD)];
    world.tracker.pair(&mut layers);
    for layer in &layers {
        assert_eq!(eyes_of(layer), eyes_at(&first.pose));
    }
}

#[test]
fn a_picture_first_seen_with_a_write_in_flight_is_paired_with_the_written_pose() {
    let mut world = World::new(&[1]);
    // The first picture, seen before any write, is paired with the camera in the tree: the origin.
    world.step(head_at(0.0, 1.6, 0.0));
    let mut layers = [projection(1, HEAD)];
    world.tracker.pair(&mut layers);
    assert_eq!(
        eyes_of(&layers[0]),
        eyes_at(&CameraPose {
            position: [0.0; 3],
            rotation_degrees: [0.0; 3]
        }),
        "a picture seen before the write is drawn from the tree's camera"
    );
    let (_, write) = *world.writes.last().expect("the write");
    // A new picture, after the write was sent, is taken to be drawn from it, even before the tree shows it.
    world.draw();
    world.advance(MIN_INTERVAL);
    world.step(head_at(0.0, 1.6, 0.0));
    let mut layers = [projection(1, HEAD)];
    world.tracker.pair(&mut layers);
    assert_eq!(eyes_of(&layers[0]), eyes_at(&write.pose));
}

#[test]
fn a_picture_that_does_not_change_keeps_the_eyes_it_was_paired_with() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    let mut before = [projection(1, HEAD)];
    world.tracker.pair(&mut before);
    // The camera moves in the tree, but the picture is the same one.
    world.show();
    world.advance(MIN_INTERVAL * 2);
    world.step(head_at(0.0, 1.6, 0.0));
    let mut after = [projection(1, HEAD)];
    world.tracker.pair(&mut after);
    assert_eq!(eyes_of(&before[0]), eyes_of(&after[0]));
}

#[test]
fn only_layers_through_the_head_camera_are_paired() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    let other = projection(2, HEAD + 7);
    let quad = Layer::Quad(placement(3));
    let mut layers = [other.clone(), quad.clone()];
    world.tracker.pair(&mut layers);
    assert_eq!(layers, [other, quad]);
}

#[test]
fn a_head_inside_the_deadband_is_not_written_again() {
    let mut world = World::new(&[1]);
    world.step(head_at(0.0, 1.6, 0.0));
    world.show();
    world.draw();
    world.advance(MIN_INTERVAL * 2);
    // 0.05 mm of jitter, the camera already holds the pose.
    let step = world.step(head_at(0.00005, 1.6, 0.0));
    assert_eq!((step.write, step.status), (None, Status::Following));
    world.advance(MIN_INTERVAL * 2);
    let step = world.step(head_at(0.01, 1.6, 0.0));
    assert!(step.write.is_some(), "a centimetre is a move");
}

#[test]
fn the_pose_written_is_the_one_the_camera_pose_conversion_makes() {
    let mut world = World::new(&[1]);
    let head = yawed(0.2, 33.0);
    let write = world.step(head).write.expect("a write");
    assert_eq!(Some(write.pose), camera_pose(&head, METRES));
    assert_eq!(write.head, head);
}

// ---------------------------------------------------------------------------
// The budget writer.

fn budget_camera(budget: Option<u64>) -> CameraState {
    CameraState {
        layer_budget: budget,
        ..camera_holding(None)
    }
}

#[test]
fn the_budget_is_written_when_the_camera_does_not_hold_it() {
    let mut budget = Budget::default();
    let now = Instant::now();
    assert_eq!(budget.due(now, Some(budget_camera(None)), 16), Some((HEAD, 16)));
    assert_eq!(
        budget.due(now + Duration::from_millis(5), Some(budget_camera(Some(16))), 16),
        None,
        "once it holds it, nothing is written"
    );
    assert_eq!(
        budget.due(now + Duration::from_millis(10), Some(budget_camera(Some(16))), 8),
        Some((HEAD, 8)),
        "a changed limit is written"
    );
}

#[test]
fn a_budget_that_was_just_tried_is_not_tried_every_pass() {
    let mut budget = Budget::default();
    let now = Instant::now();
    assert!(budget.due(now, Some(budget_camera(None)), 16).is_some());
    assert_eq!(
        budget.due(now + Duration::from_millis(16), Some(budget_camera(None)), 16),
        None
    );
    assert!(
        budget.due(now + BUDGET_RETRY, Some(budget_camera(None)), 16).is_some(),
        "and is tried again after the retry time"
    );
}

#[test]
fn no_head_camera_means_no_budget_write() {
    let mut budget = Budget::default();
    assert_eq!(budget.due(Instant::now(), None, 16), None);
}

#[test]
fn a_budget_refused_stale_is_tried_again_on_the_next_pass() {
    let mut budget = Budget::default();
    let now = Instant::now();
    assert!(budget.due(now, Some(budget_camera(None)), 16).is_some());
    budget.lagged();
    assert!(
        budget
            .due(now + Duration::from_millis(16), Some(budget_camera(None)), 16)
            .is_some()
    );
}
