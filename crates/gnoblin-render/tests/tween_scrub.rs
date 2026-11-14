//! The tween player as a gesture scrubs it: seek to a point of a track, hold it, run it backwards, settle it,
//! and the hit testing that follows the pose that is presented (the spec's AN5).
//!
//! The player has no clock, so every test feeds it the milliseconds it chose. Two runs of one test are one run.

use gnoblin_animation::{Easing, PropertyValue, VisualProperty};
use gnoblin_render::{NodeSpec, Scene, TrackEndReason, TrackError, TrackSettle, TrackSpec, TweenPlayer};
use gnoblin_types::{NodeId, OutputId, PointF, Rect};

const NODE: NodeId = NodeId::new(1);

fn scene() -> Scene {
    let mut scene = Scene::new();
    scene.set_output(OutputId::new(0), Rect::new(0, 0, 1000, 800));
    scene
        .insert(NODE, NodeSpec::new(Rect::new(0, 0, 100, 100)))
        .expect("node inserted");
    scene
}

fn track(
    scene: &mut Scene,
    player: &mut TweenPlayer,
    property: VisualProperty,
    to: f32,
    ms: u32,
) -> gnoblin_types::TrackId {
    player
        .start(
            scene,
            TrackSpec {
                duration_ms: ms,
                easing: Easing::Linear,
                ..TrackSpec::new(NODE, property, PropertyValue::number(to))
            },
        )
        .expect("track started")
}

fn number(scene: &Scene, property: VisualProperty) -> f32 {
    match scene.property(NODE, property).expect("property") {
        PropertyValue::Number(value) => value,
        other => panic!("{property:?} is not a number: {other:?}"),
    }
}

/// An interpolated value is a float lerp away from the exact one; an end value is compared exactly.
fn near(actual: f32, expected: f32, what: &str) {
    assert!((actual - expected).abs() < 1e-4, "{what}: {actual} is not {expected}");
}

/// Golden poses at the four points a gesture's scrub names, for the four things a transition moves.
#[test]
fn seeking_draws_the_golden_pose_at_each_progress() {
    // (property, start, target, the value at 0, 0.25, 0.5 and 1 on a linear curve)
    let cases: [(VisualProperty, f32, f32, [f32; 4]); 5] = [
        (VisualProperty::OffsetX, 0.0, 200.0, [0.0, 50.0, 100.0, 200.0]),
        (VisualProperty::Opacity, 1.0, 0.0, [1.0, 0.75, 0.5, 0.0]),
        (VisualProperty::ScaleX, 1.0, 3.0, [1.0, 1.5, 2.0, 3.0]),
        (VisualProperty::ScaleY, 1.0, 0.5, [1.0, 0.875, 0.75, 0.5]),
        (VisualProperty::RotationZ, 0.0, 90.0, [0.0, 22.5, 45.0, 90.0]),
    ];
    for (property, start, target, golden) in cases {
        let mut scene = scene();
        let mut player = TweenPlayer::new();
        let id = track(&mut scene, &mut player, property, target, 400);
        player.pause(id).expect("paused");
        assert_eq!(
            number(&scene, property),
            start,
            "{property:?} starts at its start value"
        );
        for (progress, expected) in [0.0, 0.25, 0.5, 1.0].into_iter().zip(golden) {
            player.seek(&mut scene, id, progress).expect("seek");
            assert!(
                (number(&scene, property) - expected).abs() < 1e-4,
                "{property:?} at {progress}: {} is not {expected}",
                number(&scene, property)
            );
            assert_eq!(player.progress(id), Some(progress));
        }
        // The two ends are exact, not an interpolation that rounds to almost them.
        player.seek(&mut scene, id, 0.0).expect("seek to the start");
        assert_eq!(number(&scene, property), start);
        player.seek(&mut scene, id, 1.0).expect("seek to the end");
        assert_eq!(number(&scene, property), target);
        assert!(
            player.is_active(id),
            "a seek to 1 does not end a paused track: the gesture may go back"
        );
        assert!(player.take_ended().is_empty());
    }
}

#[test]
fn seeking_follows_the_easing_not_the_clock() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = player
        .start(
            &mut scene,
            TrackSpec {
                duration_ms: 1000,
                easing: Easing::EaseOutCubic,
                ..TrackSpec::new(NODE, VisualProperty::OffsetX, PropertyValue::number(100.0))
            },
        )
        .expect("started");
    player.pause(id).expect("paused");
    player.seek(&mut scene, id, 0.5).expect("seek");
    let eased = Easing::EaseOutCubic.apply(0.5) as f32 * 100.0;
    assert!((number(&scene, VisualProperty::OffsetX) - eased).abs() < 1e-3);
    // Time does not move a paused track.
    player.advance(&mut scene, 900);
    assert!((number(&scene, VisualProperty::OffsetX) - eased).abs() < 1e-3);
}

#[test]
fn a_progress_outside_zero_to_one_is_refused_and_changes_nothing() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = track(&mut scene, &mut player, VisualProperty::OffsetX, 100.0, 100);
    player.pause(id).expect("paused");
    player.seek(&mut scene, id, 0.5).expect("seek");
    for bad in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            player.seek(&mut scene, id, bad),
            Err(TrackError::ProgressRange(_))
        ));
    }
    assert_eq!(number(&scene, VisualProperty::OffsetX), 50.0);
    assert_eq!(player.progress(id), Some(0.5));
}

#[test]
fn pause_holds_a_track_and_resume_goes_on_from_the_same_place() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = track(&mut scene, &mut player, VisualProperty::OffsetX, 100.0, 100);
    player.advance(&mut scene, 40);
    assert_eq!(number(&scene, VisualProperty::OffsetX), 40.0);
    player.pause(id).expect("paused");
    assert!(player.is_paused(id));
    player.advance(&mut scene, 500);
    assert_eq!(
        number(&scene, VisualProperty::OffsetX),
        40.0,
        "a paused track writes nothing"
    );
    assert!(player.is_active(id));
    player.resume(id).expect("resumed");
    player.advance(&mut scene, 30);
    assert_eq!(number(&scene, VisualProperty::OffsetX), 70.0);
    player.advance(&mut scene, 30);
    assert_eq!(number(&scene, VisualProperty::OffsetX), 100.0);
    assert_eq!(player.take_ended()[0].reason, TrackEndReason::Completed);
}

#[test]
fn a_reversed_track_runs_back_to_its_start_and_ends_rolled_back() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = track(&mut scene, &mut player, VisualProperty::OffsetX, 100.0, 100);
    player.advance(&mut scene, 60);
    player.set_reverse(id, true).expect("reversed");
    assert!(player.is_reversed(id));
    near(
        number(&scene, VisualProperty::OffsetX),
        60.0,
        "turning round does not jump",
    );
    player.advance(&mut scene, 40);
    near(
        number(&scene, VisualProperty::OffsetX),
        20.0,
        "back along the same path",
    );
    player.advance(&mut scene, 40);
    assert_eq!(number(&scene, VisualProperty::OffsetX), 0.0, "the start value is exact");
    assert!(!player.is_active(id));
    let ended = player.take_ended();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].reason, TrackEndReason::RolledBack);
    assert_eq!(ended[0].value, PropertyValue::number(0.0));
}

#[test]
fn a_reversed_track_turned_forward_again_completes() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = track(&mut scene, &mut player, VisualProperty::Opacity, 0.0, 100);
    player.advance(&mut scene, 50);
    player.set_reverse(id, true).expect("reversed");
    player.advance(&mut scene, 20);
    player.set_reverse(id, false).expect("forward");
    player.advance(&mut scene, 100);
    assert_eq!(number(&scene, VisualProperty::Opacity), 0.0);
    assert_eq!(player.take_ended()[0].reason, TrackEndReason::Completed);
}

#[test]
fn finish_settles_on_the_target_the_start_or_where_it_is() {
    for (settle, expected, reason) in [
        (TrackSettle::Forward, 100.0, TrackEndReason::Completed),
        (TrackSettle::Back, 0.0, TrackEndReason::RolledBack),
        (TrackSettle::Cancel, 25.0, TrackEndReason::Cancelled),
    ] {
        let mut scene = scene();
        let mut player = TweenPlayer::new();
        let id = track(&mut scene, &mut player, VisualProperty::OffsetX, 100.0, 100);
        player.advance(&mut scene, 25);
        player.finish(&mut scene, id, settle).expect("settled");
        assert_eq!(number(&scene, VisualProperty::OffsetX), expected, "{settle:?}");
        let ended = player.take_ended();
        assert_eq!(ended.len(), 1, "{settle:?} ends the track once");
        assert_eq!(ended[0].reason, reason);
        assert_eq!(ended[0].value, PropertyValue::number(expected));
        assert_eq!(player.active(), 0);
        assert_eq!(
            player.finish(&mut scene, id, settle).expect_err("already ended"),
            TrackError::UnknownTrack(id)
        );
        player.advance(&mut scene, 500);
        assert_eq!(
            number(&scene, VisualProperty::OffsetX),
            expected,
            "a settled track stays put"
        );
        assert!(player.take_ended().is_empty());
    }
}

#[test]
fn a_seek_clears_the_rest_of_the_delay_and_a_reverse_never_waits_one() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = player
        .start(
            &mut scene,
            TrackSpec {
                duration_ms: 100,
                delay_ms: 500,
                ..TrackSpec::new(NODE, VisualProperty::OffsetX, PropertyValue::number(100.0))
            },
        )
        .expect("started");
    player.advance(&mut scene, 200);
    assert_eq!(number(&scene, VisualProperty::OffsetX), 0.0, "still in the delay");
    player.seek(&mut scene, id, 0.5).expect("seek");
    player.advance(&mut scene, 10);
    near(
        number(&scene, VisualProperty::OffsetX),
        60.0,
        "the position is the caller's: no delay is left",
    );

    let id = player
        .start(
            &mut scene,
            TrackSpec {
                duration_ms: 100,
                delay_ms: 500,
                ..TrackSpec::new(NODE, VisualProperty::OffsetY, PropertyValue::number(100.0))
            },
        )
        .expect("started");
    player.set_reverse(id, true).expect("reversed");
    player.advance(&mut scene, 1);
    assert_eq!(
        player.take_ended().last().map(|end| end.reason),
        Some(TrackEndReason::RolledBack)
    );
}

#[test]
fn the_same_ticks_in_any_split_draw_the_same_pose() {
    let run = |ticks: &[u32]| {
        let mut scene = scene();
        let mut player = TweenPlayer::new();
        let id = track(&mut scene, &mut player, VisualProperty::OffsetX, 100.0, 1000);
        for &tick in ticks {
            player.advance(&mut scene, tick);
        }
        player.seek(&mut scene, id, 0.4).ok();
        player.advance(&mut scene, 100);
        number(&scene, VisualProperty::OffsetX)
    };
    assert_eq!(run(&[250, 250]), run(&[100, 400]));
    assert_eq!(run(&[250, 250]), run(&[1, 499]));
}

#[test]
fn width_and_height_tween_in_whole_pixels() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let wide = track(&mut scene, &mut player, VisualProperty::Width, 300.0, 100);
    track(&mut scene, &mut player, VisualProperty::Height, 50.0, 100);
    assert_eq!(
        player.active(),
        2,
        "width and height are two tracks that do not exclude each other"
    );
    player.advance(&mut scene, 25);
    assert_eq!(
        scene.bounds(NODE),
        Some(Rect::new(0, 0, 150, 88)),
        "150 and 87.5 rounded"
    );
    player.pause(wide).expect("paused");
    player.advance(&mut scene, 25);
    assert_eq!(scene.bounds(NODE), Some(Rect::new(0, 0, 150, 75)));
    player.advance(&mut scene, 50);
    assert_eq!(scene.bounds(NODE), Some(Rect::new(0, 0, 150, 50)));
    player.finish(&mut scene, wide, TrackSettle::Forward).expect("settled");
    assert_eq!(
        scene.bounds(NODE),
        Some(Rect::new(0, 0, 300, 50)),
        "the target is exact"
    );
}

/// AN5: while a tween runs the node is hit where it is presented, not where it is going.
#[test]
fn hit_testing_follows_the_presented_rectangle_of_a_size_tween() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = track(&mut scene, &mut player, VisualProperty::Width, 300.0, 400);
    player.pause(id).expect("paused");
    player.seek(&mut scene, id, 0.5).expect("seek");
    // Presented: 0..200. Target: 0..300.
    assert_eq!(scene.hit_test(PointF::new(150.0, 50.0)).map(|hit| hit.node), Some(NODE));
    assert_eq!(
        scene.hit_test(PointF::new(250.0, 50.0)),
        None,
        "a point inside the target rectangle only is not on the node yet"
    );
    player.seek(&mut scene, id, 1.0).expect("seek");
    assert_eq!(scene.hit_test(PointF::new(250.0, 50.0)).map(|hit| hit.node), Some(NODE));
    player.seek(&mut scene, id, 0.0).expect("seek");
    assert_eq!(
        scene.hit_test(PointF::new(150.0, 50.0)),
        None,
        "and back at the start it is gone again"
    );
}

#[test]
fn hit_testing_follows_the_presented_position_of_a_move() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let id = track(&mut scene, &mut player, VisualProperty::OffsetX, 200.0, 400);
    player.pause(id).expect("paused");
    player.seek(&mut scene, id, 0.5).expect("seek");
    // Presented: x 100..200. Start: 0..100. Target: 200..300.
    assert_eq!(scene.hit_test(PointF::new(150.0, 50.0)).map(|hit| hit.node), Some(NODE));
    assert_eq!(scene.hit_test(PointF::new(50.0, 50.0)), None, "not where it started");
    assert_eq!(scene.hit_test(PointF::new(250.0, 50.0)), None, "not where it ends");
    // The point comes back local to the presented node.
    let hit = scene.hit_test(PointF::new(150.0, 50.0)).expect("hit");
    assert!((hit.local.x - 50.0).abs() < 1e-9 && (hit.local.y - 50.0).abs() < 1e-9);
}

#[test]
fn hit_testing_follows_the_presented_scale_and_rotation() {
    let mut scene = scene();
    let mut player = TweenPlayer::new();
    let grow = track(&mut scene, &mut player, VisualProperty::ScaleX, 3.0, 400);
    player.pause(grow).expect("paused");
    // Scale is about the node's centre (50, 50). At 0.5 the scale is 2: the node covers -50..150.
    player.seek(&mut scene, grow, 0.5).expect("seek");
    assert_eq!(scene.hit_test(PointF::new(140.0, 50.0)).map(|hit| hit.node), Some(NODE));
    assert_eq!(scene.hit_test(PointF::new(-40.0, 50.0)).map(|hit| hit.node), Some(NODE));
    assert_eq!(scene.hit_test(PointF::new(160.0, 50.0)), None);
    player.seek(&mut scene, grow, 0.0).expect("seek");
    assert_eq!(
        scene.hit_test(PointF::new(140.0, 50.0)),
        None,
        "the target scale is not the presented one"
    );
    player.finish(&mut scene, grow, TrackSettle::Cancel).expect("settled");

    // A quarter turn about the centre makes a 100 by 100 square cover the same area, but a half-height node
    // turned 90 degrees covers a tall strip: check on a node that is wide.
    let mut scene = Scene::new();
    scene.set_output(OutputId::new(0), Rect::new(0, 0, 1000, 800));
    scene
        .insert(NODE, NodeSpec::new(Rect::new(0, 0, 200, 20)))
        .expect("wide node");
    let mut player = TweenPlayer::new();
    let turn = track(&mut scene, &mut player, VisualProperty::RotationZ, 90.0, 400);
    player.pause(turn).expect("paused");
    // The centre is (100, 10). Unturned, (100, 70) is outside; turned a quarter it is inside.
    player.seek(&mut scene, turn, 0.0).expect("seek");
    assert_eq!(scene.hit_test(PointF::new(100.0, 70.0)), None);
    player.seek(&mut scene, turn, 1.0).expect("seek");
    assert_eq!(scene.hit_test(PointF::new(100.0, 70.0)).map(|hit| hit.node), Some(NODE));
    assert_eq!(
        scene.hit_test(PointF::new(10.0, 10.0)),
        None,
        "and the old end of the node is not there now"
    );
    player.seek(&mut scene, turn, 0.5).expect("seek");
    assert!(player.is_active(turn));
}
