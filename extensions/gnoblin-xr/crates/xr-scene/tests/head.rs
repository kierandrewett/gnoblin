//! The head pose as a stereo camera's pose: the inverse of what `rig::eyes` states.

use xr_scene::head::{CameraPose, camera_pose};
use xr_scene::rig::eyes;
use xr_scene::{IDENTITY, Pose, Rig};

const METRES: f64 = 0.001;

fn pose(position: [f64; 3], orientation: [f64; 4]) -> Pose {
    Pose { position, orientation }
}

/// The unit quaternion for a turn of `degrees` about the unit `axis`.
fn turn(axis: [f64; 3], degrees: f64) -> [f64; 4] {
    let half = degrees.to_radians() / 2.0;
    let s = half.sin();
    [axis[0] * s, axis[1] * s, axis[2] * s, half.cos()]
}

fn mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// The quaternions equal up to sign: `q` and `-q` are one rotation.
fn same_rotation(a: [f64; 4], b: [f64; 4]) -> bool {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    (dot.abs() - 1.0).abs() < 1e-9
}

/// The eyes the planner would state for the camera written from `head`, and the check that they are the head's.
fn assert_round_trip(head: Pose) {
    let camera = camera_pose(&head, METRES).expect("a usable pose");
    let rig = Rig::default();
    let [left, right] = eyes(camera.position, camera.rotation_degrees, METRES, &rig, [1600.0, 900.0]).expect("eyes");
    // Both eyes share the head's orientation, and the midpoint between them is the head's position.
    assert!(
        same_rotation(left.pose.orientation, head.orientation),
        "left eye {head:?}"
    );
    assert!(
        same_rotation(right.pose.orientation, head.orientation),
        "right eye {head:?}"
    );
    for axis in 0..3 {
        let middle = (left.pose.position[axis] + right.pose.position[axis]) / 2.0;
        assert!(
            (middle - head.position[axis]).abs() < 1e-9,
            "axis {axis}: {middle} against {}",
            head.position[axis]
        );
    }
}

#[test]
fn the_identity_pose_is_a_camera_at_the_origin_looking_down_negative_z() {
    let camera = camera_pose(&pose([0.0; 3], IDENTITY), METRES).expect("identity");
    assert_eq!(camera.position, [0.0; 3]);
    assert!(camera.rotation_degrees.iter().all(|angle| angle.abs() < 1e-9));
}

#[test]
fn position_is_in_the_cameras_units_with_y_mirrored() {
    // The scene's y grows down and OpenXR's grows up; a camera unit is `METRES` metres.
    let camera = camera_pose(&pose([0.5, 1.6, -2.0], IDENTITY), METRES).expect("pose");
    assert_eq!(
        camera,
        CameraPose {
            position: [500.0, -1600.0, -2000.0],
            rotation_degrees: camera.rotation_degrees
        }
    );
}

#[test]
fn a_walking_turning_head_round_trips_through_the_eyes_the_planner_states() {
    for step in 0..40 {
        let t = f64::from(step);
        let yaw = turn([0.0, 1.0, 0.0], t * 9.0 - 180.0);
        let pitch = turn([1.0, 0.0, 0.0], (t * 3.0).sin() * 40.0);
        let roll = turn([0.0, 0.0, 1.0], (t * 5.0).cos() * 15.0);
        let orientation = mul(yaw, mul(pitch, roll));
        assert_round_trip(pose(
            [t * 0.05 - 1.0, 1.5 + (t * 0.3).sin() * 0.1, -t * 0.02],
            orientation,
        ));
    }
}

#[test]
fn a_head_turned_a_quarter_either_way_round_trips_where_the_angles_are_ambiguous() {
    // Yaw of plus or minus 90 degrees is where Euler angles lose a degree of freedom. The rotation must survive.
    for yaw in [90.0, -90.0, 89.999, -89.999] {
        let q = mul(
            turn([0.0, 1.0, 0.0], yaw),
            mul(turn([1.0, 0.0, 0.0], 20.0), turn([0.0, 0.0, 1.0], -10.0)),
        );
        assert_round_trip(pose([0.1, 1.7, 0.2], q));
        assert_round_trip(pose([0.0; 3], turn([0.0, 1.0, 0.0], yaw)));
    }
}

#[test]
fn a_head_upside_down_round_trips() {
    assert_round_trip(pose([0.0, 1.0, 0.0], turn([0.0, 0.0, 1.0], 180.0)));
    assert_round_trip(pose([0.0, 1.0, 0.0], turn([1.0, 0.0, 0.0], 180.0)));
}

#[test]
fn a_pose_that_is_not_finite_is_refused() {
    let nan = f64::NAN;
    assert!(camera_pose(&pose([nan, 0.0, 0.0], IDENTITY), METRES).is_none());
    assert!(camera_pose(&pose([0.0, f64::INFINITY, 0.0], IDENTITY), METRES).is_none());
    assert!(camera_pose(&pose([0.0; 3], [0.0, 0.0, f64::NEG_INFINITY, 1.0]), METRES).is_none());
    assert!(camera_pose(&pose([0.0; 3], [nan, 0.0, 0.0, 1.0]), METRES).is_none());
}

#[test]
fn an_orientation_that_is_not_a_rotation_is_refused() {
    assert!(
        camera_pose(&pose([0.0; 3], [0.0; 4]), METRES).is_none(),
        "the zero quaternion"
    );
    assert!(
        camera_pose(&pose([0.0; 3], [0.0, 0.0, 0.0, 2.0]), METRES).is_none(),
        "twice the length"
    );
    assert!(
        camera_pose(&pose([0.0; 3], [0.0, 0.0, 0.0, 0.5]), METRES).is_none(),
        "half the length"
    );
}

#[test]
fn a_scale_that_is_not_positive_and_finite_is_refused() {
    for metres in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(camera_pose(&pose([0.0; 3], IDENTITY), metres).is_none(), "{metres}");
    }
}

#[test]
fn a_position_too_large_for_the_cameras_units_is_refused() {
    // 1e300 metres over 0.001 is 1e303: finite, but the wire and the eyes cannot carry it usefully.
    assert!(camera_pose(&pose([1e300, 0.0, 0.0], IDENTITY), METRES).is_none());
    assert!(
        camera_pose(&pose([1e305, 0.0, 0.0], IDENTITY), 1e-9).is_none(),
        "a quotient that overflows"
    );
}
