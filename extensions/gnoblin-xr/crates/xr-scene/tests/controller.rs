//! A controller's aim pose as the pose of a `controller` node: in the node units the View nodes use.

use std::collections::BTreeMap;

use xr_scene::controller::{PlacedPose, placed_pose};
use xr_scene::{
    CameraProjection, Class, IDENTITY, Layer, Options, Pose, Rig, TreeNode, TreeView, placement_metres, plan,
};

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

fn same_rotation(a: [f64; 4], b: [f64; 4]) -> bool {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    (dot.abs() - 1.0).abs() < 1e-9
}

/// A room of millimetres with a head camera, and one View of 400 by 300 units placed at `placed`: the same place and
/// turn the program writes for a controller. The pose the planner states for the View is where the controller was.
fn view_at(placed: &PlacedPose, metres: f64) -> Pose {
    let [x, y, z] = placed.position;
    let [a, b, c] = placed.rotation_degrees;
    let mut tree = TreeView {
        nodes: BTreeMap::new(),
        roots: vec![1],
    };
    tree.insert(TreeNode::new(1, Class::Node).named("room").children(&[2, 3]));
    tree.insert(
        TreeNode::new(
            2,
            Class::Camera {
                projection: CameraProjection::Stereo,
                rig: Rig::default(),
            },
        )
        .named("head")
        .metres(metres),
    );
    tree.insert(
        TreeNode::new(
            3,
            Class::View {
                source: Some(9),
                camera: None,
                width: 400.0,
                height: 300.0,
            },
        )
        .at(x, y, z)
        .rotated(a, b, c),
    );
    let planned = plan(&tree, &pose([0.0; 3], IDENTITY), &Options::default());
    match planned.layers.as_slice() {
        [Layer::Quad(placement)] => placement.pose,
        other => panic!("one quad expected, got {other:?}"),
    }
}

#[test]
fn the_identity_aim_at_the_origin_is_a_node_at_the_origin_with_no_turn() {
    let placed = placed_pose(&pose([0.0; 3], IDENTITY), METRES).expect("identity");
    assert_eq!(placed.position, [0.0; 3]);
    assert!(placed.rotation_degrees.iter().all(|angle| angle.abs() < 1e-9));
}

#[test]
fn the_position_is_in_node_units_and_y_is_not_mirrored_as_the_head_cameras_is() {
    let placed = placed_pose(&pose([0.25, 1.1, -0.4], IDENTITY), METRES).expect("pose");
    let want = [250.0, 1100.0, -400.0];
    for (axis, (got, want)) in placed.position.into_iter().zip(want).enumerate() {
        assert!((got - want).abs() < 1e-9, "axis {axis}: {:?}", placed.position);
    }
}

#[test]
fn a_view_placed_where_the_controller_is_comes_out_where_the_controller_is() {
    let turns = [
        [0.0, 0.0, 0.0],
        [30.0, 0.0, 0.0],
        [0.0, -75.0, 0.0],
        [0.0, 0.0, 20.0],
        [25.0, 40.0, -10.0],
    ];
    for [a, b, c] in turns {
        // Turned about x, then y, then z, as the Views are (`xr_scene::math::euler_degrees`).
        let orientation = mul(
            mul(turn([0.0, 0.0, 1.0], c), turn([0.0, 1.0, 0.0], b)),
            turn([1.0, 0.0, 0.0], a),
        );
        let aim = pose([0.3, 1.4, -0.8], orientation);
        let placed = placed_pose(&aim, METRES).expect("a usable pose");
        let comes_out = view_at(&placed, METRES);
        for (axis, (got, want)) in comes_out.position.into_iter().zip(aim.position).enumerate() {
            assert!(
                (got - want).abs() < 1e-9,
                "turn {a} {b} {c}, axis {axis}: {:?}",
                comes_out.position
            );
        }
        assert!(
            same_rotation(comes_out.orientation, aim.orientation),
            "turn {a} {b} {c}"
        );
    }
}

#[test]
fn a_hand_held_straight_up_still_states_the_rotation_it_has() {
    // Near a quarter turn about y the three angles are not unique; the rotation they state is.
    let orientation = turn([0.0, 1.0, 0.0], 90.0);
    let aim = pose([0.0, 1.0, -0.5], orientation);
    let placed = placed_pose(&aim, METRES).expect("a usable pose");
    assert!(same_rotation(view_at(&placed, METRES).orientation, orientation));
}

#[test]
fn a_pose_that_cannot_be_written_is_none() {
    let nan = f64::NAN;
    assert!(placed_pose(&pose([nan, 0.0, 0.0], IDENTITY), METRES).is_none());
    assert!(placed_pose(&pose([0.0, f64::INFINITY, 0.0], IDENTITY), METRES).is_none());
    assert!(placed_pose(&pose([0.0; 3], [nan, 0.0, 0.0, 1.0]), METRES).is_none());
    assert!(
        placed_pose(&pose([0.0; 3], [0.0; 4]), METRES).is_none(),
        "not a rotation"
    );
    assert!(placed_pose(&pose([0.0; 3], [0.0, 0.0, 0.0, 2.0]), METRES).is_none());
    for metres in [0.0, -1.0, nan, f64::INFINITY] {
        assert!(
            placed_pose(&pose([0.0; 3], IDENTITY), metres).is_none(),
            "metres {metres}"
        );
    }
    assert!(
        placed_pose(&pose([1.0e5, 0.0, 0.0], IDENTITY), METRES).is_none(),
        "past the largest coordinate the compositor stores"
    );
}

#[test]
fn the_metres_are_the_planners_own_the_head_cameras_then_the_rooms_then_the_option() {
    let options = Options { default_metres: 0.5 };
    let mut tree = TreeView::default();
    assert!(
        (placement_metres(&tree, &options) - 0.5).abs() < f64::EPSILON,
        "no room: the option"
    );

    tree.roots = vec![1];
    tree.insert(TreeNode::new(1, Class::Node).named("room").children(&[2]));
    assert!(
        (placement_metres(&tree, &options) - 0.5).abs() < f64::EPSILON,
        "a room that sets none"
    );

    tree.insert(TreeNode::new(1, Class::Node).named("room").metres(0.01).children(&[2]));
    assert!(
        (placement_metres(&tree, &options) - 0.01).abs() < f64::EPSILON,
        "the room's"
    );

    tree.insert(
        TreeNode::new(
            2,
            Class::Camera {
                projection: CameraProjection::Stereo,
                rig: Rig::default(),
            },
        )
        .named("head")
        .metres(0.001),
    );
    assert!(
        (placement_metres(&tree, &options) - 0.001).abs() < f64::EPSILON,
        "the head camera's wins"
    );
}
