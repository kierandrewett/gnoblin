//! Tests for `plan`: the translation from the `room` subtree to layers.

use std::collections::BTreeMap;

use xr_scene::{
    CameraProjection, Class, Eye, HeadPose, IDENTITY, Layer, NodeId, Options, Plan, Pose, Projected, Rig, SkipReason,
    Space, TreeNode, TreeView, plan,
};

const ROOM: NodeId = 1;
const HEAD: NodeId = 2;
const EPS: f64 = 1e-9;

fn scene(nodes: Vec<TreeNode>, roots: &[NodeId]) -> TreeView {
    let mut tree = TreeView {
        nodes: BTreeMap::new(),
        roots: roots.to_vec(),
    };
    for node in nodes {
        tree.insert(node);
    }
    tree
}

fn room(children: &[NodeId]) -> TreeNode {
    TreeNode::new(ROOM, Class::Node).named("room").children(children)
}

fn view(id: NodeId, width: f64, height: f64) -> TreeNode {
    TreeNode::new(
        id,
        Class::View {
            source: Some(100 + id),
            camera: None,
            width,
            height,
        },
    )
}

fn group(id: NodeId) -> TreeNode {
    TreeNode::new(id, Class::Node)
}

fn camera(id: NodeId, projection: CameraProjection) -> TreeNode {
    TreeNode::new(
        id,
        Class::Camera {
            projection,
            rig: Rig::default(),
        },
    )
}

fn head_pose(x: f64, y: f64, z: f64) -> HeadPose {
    Pose {
        position: [x, y, z],
        orientation: IDENTITY,
    }
}

fn opts() -> Options {
    Options::default()
}

fn ids(plan: &Plan) -> Vec<NodeId> {
    plan.layers.iter().map(layer_id).collect()
}

fn layer_id(layer: &Layer) -> NodeId {
    match layer {
        Layer::Quad(p) | Layer::Projection(Projected { placement: p, .. }) => p.id,
    }
}

fn quad(layer: &Layer) -> &xr_scene::Placement {
    match layer {
        Layer::Quad(p) => p,
        Layer::Projection(p) => panic!("expected a quad, got a projection for {}", p.placement.id),
    }
}

fn assert_vec_close(actual: [f64; 3], expected: [f64; 3]) {
    for (a, e) in actual.iter().zip(expected.iter()) {
        assert!((a - e).abs() < EPS, "{actual:?} != {expected:?}");
    }
}

#[test]
fn placement_carries_the_source_window_node() {
    let nodes = vec![room(&[5]), view(5, 1000.0, 500.0)];
    let plan = plan(&scene(nodes, &[ROOM]), &head_pose(0.0, 0.0, 0.0), &opts());
    let placed = quad(&plan.layers[0]);
    assert_eq!(placed.id, 5);
    assert_eq!(placed.source, 105, "the layer names the window node its View shows");
}

#[test]
fn layers_follow_tree_order() {
    // room -> [a, b -> [c], d]: pre-order, so the parent's view precedes its children.
    let tree = scene(
        vec![
            room(&[10, 11, 14]),
            view(10, 1.0, 1.0),
            group(11).children(&[12]),
            view(12, 1.0, 1.0),
            view(14, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_eq!(ids(&p), vec![10, 12, 14]);
    assert!(p.skipped.is_empty());
}

#[test]
fn reordering_siblings_reorders_layers() {
    let before = scene(vec![room(&[10, 11]), view(10, 1.0, 1.0), view(11, 1.0, 1.0)], &[ROOM]);
    let after = scene(vec![room(&[11, 10]), view(10, 1.0, 1.0), view(11, 1.0, 1.0)], &[ROOM]);
    let h = head_pose(0.0, 0.0, 0.0);
    assert_eq!(ids(&plan(&before, &h, &opts())), vec![10, 11]);
    assert_eq!(ids(&plan(&after, &h, &opts())), vec![11, 10]);
}

#[test]
fn nested_transform_composes_parent_then_child() {
    // parent at (1,0,0) scaled by 2; child at (1,0,0) inside it: world (3,0,0).
    let tree = scene(
        vec![
            room(&[20]),
            group(20).at(1.0, 0.0, 0.0).scaled(2.0).children(&[21]),
            view(21, 10.0, 5.0).at(1.0, 0.0, 0.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    let q = quad(&p.layers[0]);
    assert_vec_close(q.pose.position, [3.0, 0.0, 0.0]);
    // Size is the View's width and height times the global scale (2) and metres (1).
    assert!(
        (q.size[0] - 20.0).abs() < EPS && (q.size[1] - 10.0).abs() < EPS,
        "{:?}",
        q.size
    );
}

#[test]
fn rotation_composes_into_child_position() {
    // A parent turned 90 degrees about Z maps the child's +X offset onto +Y.
    let tree = scene(
        vec![
            room(&[30]),
            group(30).rotated(0.0, 0.0, 90.0).children(&[31]),
            view(31, 1.0, 1.0).at(1.0, 0.0, 0.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    let q = quad(&p.layers[0]);
    assert_vec_close(q.pose.position, [0.0, 1.0, 0.0]);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    assert!((q.pose.orientation[2] - half).abs() < EPS && (q.pose.orientation[3] - half).abs() < EPS);
}

#[test]
fn metres_scale_pose_and_size() {
    let tree = scene(
        vec![
            TreeNode::new(ROOM, Class::Node)
                .named("room")
                .metres(0.5)
                .children(&[40]),
            view(40, 100.0, 50.0).at(2.0, 0.0, 0.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    let q = quad(&p.layers[0]);
    assert_vec_close(q.pose.position, [1.0, 0.0, 0.0]);
    assert!(
        (q.size[0] - 50.0).abs() < EPS && (q.size[1] - 25.0).abs() < EPS,
        "{:?}",
        q.size
    );
    assert!((q.extent[0] - 100.0).abs() < EPS && (q.extent[1] - 50.0).abs() < EPS);
}

#[test]
fn head_camera_metres_overrides_room_metres() {
    // The head camera's metres (0.001) win over the room's (0.5); the world View sits under the room.
    let tree = scene(
        vec![
            TreeNode::new(ROOM, Class::Node)
                .named("room")
                .metres(0.5)
                .children(&[HEAD, 41]),
            camera(HEAD, CameraProjection::Stereo).named("head").metres(0.001),
            view(41, 100.0, 100.0).at(1000.0, 0.0, 0.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_vec_close(quad(&p.layers[0]).pose.position, [1.0, 0.0, 0.0]);
}

#[test]
fn default_metres_applies_without_a_metres_property() {
    let tree = scene(vec![room(&[42]), view(42, 1.0, 1.0).at(2.0, 0.0, 0.0)], &[ROOM]);
    let h = head_pose(0.0, 0.0, 0.0);
    let p = plan(&tree, &h, &Options { default_metres: 0.01 });
    assert_vec_close(quad(&p.layers[0]).pose.position, [0.02, 0.0, 0.0]);
    let p = plan(&tree, &h, &opts());
    assert_vec_close(quad(&p.layers[0]).pose.position, [2.0, 0.0, 0.0]);
}

#[test]
fn head_locked_layers_ignore_the_head_pose() {
    // A View under the head camera is relative to the head. The head camera's own transform (1.6 m up) is
    // excluded, so the pose is the View's local position only, and it does not change with the head.
    let tree = scene(
        vec![
            room(&[HEAD]),
            camera(HEAD, CameraProjection::Stereo)
                .named("head")
                .at(0.0, 1.6, 0.0)
                .children(&[50]),
            view(50, 10.0, 10.0).at(0.0, 0.0, -1.0),
        ],
        &[ROOM],
    );
    let o = opts();
    let a = plan(&tree, &head_pose(0.0, 1.6, 0.0), &o);
    let b = plan(&tree, &head_pose(5.0, 0.0, 3.0), &o);
    assert_eq!(a.layers, b.layers);
    let q = quad(&a.layers[0]);
    assert_eq!(q.space, Space::View);
    assert_vec_close(q.pose.position, [0.0, 0.0, -1.0]);
}

#[test]
fn world_layers_ignore_the_head_pose() {
    let tree = scene(vec![room(&[51]), view(51, 10.0, 10.0).at(0.0, 0.0, -2.0)], &[ROOM]);
    let o = opts();
    let a = plan(&tree, &head_pose(0.0, 0.0, 0.0), &o);
    let b = plan(&tree, &head_pose(0.0, 1.6, 4.0), &o);
    assert_eq!(a.layers, b.layers);
    let q = quad(&a.layers[0]);
    assert_eq!(q.space, Space::World);
    assert_vec_close(q.pose.position, [0.0, 0.0, -2.0]);
}

#[test]
fn invisible_subtree_produces_nothing() {
    let tree = scene(
        vec![
            room(&[60, 61]),
            group(60).hidden().children(&[62]),
            view(62, 1.0, 1.0),
            view(61, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_eq!(ids(&p), vec![61]);
    assert!(p.skipped.is_empty(), "a hidden subtree is not reported as skipped");
}

#[test]
fn hidden_room_produces_nothing() {
    let tree = scene(
        vec![
            TreeNode::new(ROOM, Class::Node).named("room").hidden().children(&[70]),
            view(70, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(p.layers.is_empty() && p.skipped.is_empty());
}

#[test]
fn opacity_multiplies_down_the_tree() {
    // 0.5 * 0.5 * 0.8 = 0.2
    let tree = scene(
        vec![
            room(&[80]),
            group(80).opacity(0.5).children(&[81]),
            group(81).opacity(0.5).children(&[82]),
            view(82, 1.0, 1.0).opacity(0.8),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!((quad(&p.layers[0]).opacity - 0.2).abs() < EPS);
}

#[test]
fn opacity_is_clamped_to_unit_range() {
    let tree = scene(vec![room(&[83]), view(83, 1.0, 1.0).opacity(2.0)], &[ROOM]);
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!((quad(&p.layers[0]).opacity - 1.0).abs() < EPS);
}

#[test]
fn head_locked_view_under_stereo_head_is_quad() {
    // The head camera is stereo, but no View names it: the View is a head-locked quad.
    let tree = scene(
        vec![
            room(&[HEAD]),
            camera(HEAD, CameraProjection::Stereo).named("head").children(&[92]),
            view(92, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(matches!(p.layers.as_slice(), [Layer::Quad(q)] if q.id == 92 && q.space == Space::View));
}

#[test]
fn orthographic_camera_view_is_world_quad() {
    let tree = scene(
        vec![
            room(&[93]),
            camera(93, CameraProjection::Orthographic).children(&[94]),
            view(94, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(matches!(p.layers.as_slice(), [Layer::Quad(q)] if q.id == 94 && q.space == Space::World));
}

#[test]
fn unknown_class_is_transparent_to_its_views() {
    // The unknown node produces no layer, but its transform still applies to the View below it.
    let tree = scene(
        vec![
            room(&[95]),
            TreeNode::new(95, Class::Other).at(5.0, 0.0, 0.0).children(&[96]),
            view(96, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_eq!(ids(&p), vec![96]);
    assert_vec_close(quad(&p.layers[0]).pose.position, [5.0, 0.0, 0.0]);
}

#[test]
fn views_outside_room_produce_nothing() {
    let tree = scene(
        vec![group(97).children(&[98]), view(98, 1.0, 1.0), room(&[])],
        &[97, ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(p.layers.is_empty() && p.skipped.is_empty());
}

#[test]
fn no_room_produces_nothing() {
    let tree = scene(vec![group(99).children(&[100]), view(100, 1.0, 1.0)], &[99]);
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(p.layers.is_empty() && p.skipped.is_empty());
}

#[test]
fn same_inputs_give_identical_plans() {
    let tree = scene(
        vec![
            room(&[HEAD, 110, 111]),
            camera(HEAD, CameraProjection::Stereo)
                .named("head")
                .at(0.1, 1.7, 0.0)
                .rotated(10.0, 20.0, 30.0)
                .children(&[112]),
            view(112, 3.0, 2.0).at(0.3, -0.2, -1.1).rotated(7.0, 0.5, 91.0),
            group(110)
                .at(1.1, 2.2, -3.3)
                .rotated(45.0, 12.0, 6.0)
                .scaled(1.7)
                .children(&[113]),
            view(113, 640.0, 480.0).at(0.25, 0.5, 0.75).opacity(0.9),
            view(111, 10.0, 10.0).rotated(33.0, 0.0, 0.0),
        ],
        &[ROOM],
    );
    let h = head_pose(0.2, 1.6, 0.1);
    let o = opts();
    assert_eq!(plan(&tree, &h, &o), plan(&tree, &h, &o));
}

#[test]
fn non_finite_transform_is_skipped_with_reason() {
    // A NaN on the View itself, and a NaN on an ancestor, both skip the View and report it.
    let tree = scene(
        vec![
            room(&[120, 121]),
            view(120, 1.0, 1.0).at(f64::NAN, 0.0, 0.0),
            group(121).at(0.0, f64::INFINITY, 0.0).children(&[122]),
            view(122, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(p.layers.is_empty());
    assert_eq!(p.skipped.len(), 2);
    assert_eq!((p.skipped[0].id, p.skipped[0].reason), (120, SkipReason::NonFinite));
    assert_eq!((p.skipped[1].id, p.skipped[1].reason), (122, SkipReason::NonFinite));
}

#[test]
fn zero_scale_is_skipped_as_singular() {
    let tree = scene(vec![room(&[130]), view(130, 1.0, 1.0).scaled(0.0)], &[ROOM]);
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(p.layers.is_empty());
    assert_eq!((p.skipped[0].id, p.skipped[0].reason), (130, SkipReason::Singular));
}

#[test]
fn non_positive_size_is_skipped_as_invalid() {
    let tree = scene(
        vec![room(&[131, 132]), view(131, 0.0, 1.0), view(132, -1.0, 1.0)],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(p.layers.is_empty());
    assert_eq!((p.skipped[0].id, p.skipped[0].reason), (131, SkipReason::InvalidSize));
    assert_eq!((p.skipped[1].id, p.skipped[1].reason), (132, SkipReason::InvalidSize));
}

#[test]
fn input_flag_is_carried_to_the_layer() {
    let tree = scene(
        vec![room(&[140, 141]), view(140, 1.0, 1.0), view(141, 1.0, 1.0).no_input()],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(quad(&p.layers[0]).input);
    assert!(!quad(&p.layers[1]).input);
}

// ---- Stereo cameras ----

const RIG: NodeId = 50;

/// A room with a 1920x1080 View (id 51) that names the stereo camera 50, and the camera in the room.
fn stereo_room(rig_node: TreeNode) -> TreeView {
    scene(
        vec![room(&[RIG, 51]), rig_node, view(51, 1920.0, 1080.0).through(RIG)],
        &[ROOM],
    )
}

fn stereo_camera() -> TreeNode {
    camera(RIG, CameraProjection::Stereo).at(0.0, 0.0, 600.0)
}

fn projection(plan: &Plan) -> &Projected {
    match plan.layers.as_slice() {
        [Layer::Projection(projected)] => projected,
        other => panic!("one projection layer, got {other:?} (skipped {:?})", plan.skipped),
    }
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < EPS, "{actual} != {expected}");
}

fn close3(actual: [f64; 3], expected: [f64; 3]) {
    for (a, e) in actual.into_iter().zip(expected) {
        close(a, e);
    }
}

fn close_fov(eye: &Eye, left: f64, right: f64, up: f64, down: f64) {
    let got = [eye.fov.left, eye.fov.right, eye.fov.up, eye.fov.down].map(f64::from);
    for (g, e) in got.into_iter().zip([left, right, up, down]) {
        assert!((g - e).abs() < 1e-6, "{g} != {e}");
    }
}

#[test]
fn a_view_that_names_a_stereo_camera_is_a_projection_layer_with_the_cameras_eyes() {
    let p = plan(&stereo_room(stereo_camera()), &head_pose(0.0, 0.0, 0.0), &opts());
    let projected = projection(&p);
    assert_eq!(projected.placement.id, 51);
    assert_eq!(projected.placement.space, Space::World);
    // 0.063 m at 0.001 m a unit is 63 units: each eye is 31.5 units (0.0315 m) from the camera's x = 0, and the
    // camera is 600 units (0.6 m) from the plane.
    let [left, right] = &projected.eyes;
    close3(left.pose.position, [-0.0315, 0.0, 0.6]);
    close3(right.pose.position, [0.0315, 0.0, 0.6]);
    assert_eq!(left.pose.orientation, IDENTITY);
    assert_eq!(right.pose.orientation, IDENTITY);
}

#[test]
fn the_default_frustum_is_symmetric_with_the_eyes_aspect() {
    let p = plan(&stereo_room(stereo_camera()), &head_pose(0.0, 0.0, 0.0), &opts());
    let [left, right] = &projection(&p).eyes;
    // The default vertical field is 60 degrees; one eye is 960 by 1080.
    let up = (30.0_f64).to_radians();
    let across = ((960.0 / 1080.0) * up.tan()).atan();
    close_fov(left, -across, across, up, -up);
    close_fov(right, -across, across, up, -up);
}

#[test]
fn the_right_eye_has_the_mirror_of_the_left_eyes_off_axis_frustum() {
    let rig = Rig {
        angles_degrees: [-52.0, 42.0, 43.0, -47.0],
        ..Rig::default()
    };
    let p = plan(
        &stereo_room(stereo_camera().rig(rig)),
        &head_pose(0.0, 0.0, 0.0),
        &opts(),
    );
    let [left, right] = &projection(&p).eyes;
    close_fov(
        left,
        (-52.0_f64).to_radians(),
        42.0_f64.to_radians(),
        43.0_f64.to_radians(),
        (-47.0_f64).to_radians(),
    );
    close_fov(
        right,
        (-42.0_f64).to_radians(),
        52.0_f64.to_radians(),
        43.0_f64.to_radians(),
        (-47.0_f64).to_radians(),
    );
}

#[test]
fn the_eye_separation_follows_ipd_and_the_cameras_metres() {
    let rig = Rig {
        ipd: 0.1,
        ..Rig::default()
    };
    // 0.1 m at 0.002 m a unit is 50 units, so each eye is 25 units, or 0.05 m, from the camera.
    let camera = stereo_camera().rig(rig).metres(0.002).at(0.0, 0.0, 300.0);
    let p = plan(&stereo_room(camera), &head_pose(0.0, 0.0, 0.0), &opts());
    let [left, right] = &projection(&p).eyes;
    close3(left.pose.position, [-0.05, 0.0, 0.6]);
    close3(right.pose.position, [0.05, 0.0, 0.6]);
    close(right.pose.position[0] - left.pose.position[0], 0.1);
}

#[test]
fn a_cameras_y_grows_down_in_the_scene_and_up_in_the_headset() {
    let camera = stereo_camera().at(0.0, 250.0, 600.0);
    let p = plan(&stereo_room(camera), &head_pose(0.0, 0.0, 0.0), &opts());
    let [left, _] = &projection(&p).eyes;
    close(left.pose.position[1], -0.25);
}

#[test]
fn turning_the_camera_about_y_turns_the_eye_pair_and_its_orientation() {
    // A quarter turn about y takes the camera's x axis to the scene's -z, so the left eye is the one nearer the
    // plane's far side: z = 600 + 31.5 units for the left eye, 600 - 31.5 for the right.
    let camera = stereo_camera().rotated(0.0, 90.0, 0.0);
    let p = plan(&stereo_room(camera), &head_pose(0.0, 0.0, 0.0), &opts());
    let [left, right] = &projection(&p).eyes;
    close3(left.pose.position, [0.0, 0.0, 0.6315]);
    close3(right.pose.position, [0.0, 0.0, 0.5685]);
    let quarter = std::f64::consts::FRAC_PI_4;
    for eye in [left, right] {
        close(eye.pose.orientation[1], quarter.sin());
        close(eye.pose.orientation[3], quarter.cos());
    }
}

#[test]
fn a_turn_about_x_or_z_reverses_when_y_is_mirrored() {
    for (rotated, mirrored) in [
        ((30.0, 0.0, 0.0), (-30.0, 0.0, 0.0)),
        ((0.0, 0.0, 20.0), (0.0, 0.0, -20.0)),
    ] {
        let camera = stereo_camera().rotated(rotated.0, rotated.1, rotated.2);
        let p = plan(&stereo_room(camera), &head_pose(0.0, 0.0, 0.0), &opts());
        let [left, _] = &projection(&p).eyes;
        let half = |degrees: f64| (degrees.to_radians() / 2.0, (degrees.to_radians() / 2.0).sin());
        let (_, x) = half(mirrored.0);
        let (_, z) = half(mirrored.2);
        close(left.pose.orientation[0], x);
        close(left.pose.orientation[2], z);
    }
}

#[test]
fn a_view_under_a_stereo_camera_is_still_a_quad_unless_it_names_the_camera() {
    let tree = scene(
        vec![room(&[RIG]), stereo_camera().children(&[51]), view(51, 1920.0, 1080.0)],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(matches!(p.layers.as_slice(), [Layer::Quad(q)] if q.id == 51));
}

#[test]
fn a_view_that_names_a_missing_or_flat_camera_is_a_quad() {
    for tree in [
        scene(vec![room(&[51]), view(51, 1920.0, 1080.0).through(999)], &[ROOM]),
        stereo_room(camera(RIG, CameraProjection::Orthographic)),
        scene(
            vec![room(&[RIG, 51]), group(RIG), view(51, 1920.0, 1080.0).through(RIG)],
            &[ROOM],
        ),
    ] {
        let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
        assert!(matches!(p.layers.as_slice(), [Layer::Quad(q)] if q.id == 51), "{p:?}");
    }
}

#[test]
fn a_projection_layer_under_the_head_camera_is_head_locked() {
    let tree = scene(
        vec![
            room(&[HEAD]),
            camera(HEAD, CameraProjection::Stereo)
                .named("head")
                .children(&[RIG, 51]),
            stereo_camera(),
            view(51, 1920.0, 1080.0).through(RIG),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_eq!(projection(&p).placement.space, Space::View);
}

#[test]
fn a_stereo_camera_that_cannot_be_drawn_skips_its_view() {
    let cases: Vec<(&str, TreeNode)> = vec![
        (
            "negative ipd",
            stereo_camera().rig(Rig {
                ipd: -0.01,
                ..Rig::default()
            }),
        ),
        (
            "nan ipd",
            stereo_camera().rig(Rig {
                ipd: f64::NAN,
                ..Rig::default()
            }),
        ),
        (
            "no field of view",
            stereo_camera().rig(Rig {
                fov_degrees: 0.0,
                ..Rig::default()
            }),
        ),
        (
            "a flat field of view",
            stereo_camera().rig(Rig {
                fov_degrees: 180.0,
                ..Rig::default()
            }),
        ),
        (
            "left past right",
            stereo_camera().rig(Rig {
                angles_degrees: [30.0, -30.0, 30.0, -30.0],
                ..Rig::default()
            }),
        ),
        (
            "an angle of 90 degrees",
            stereo_camera().rig(Rig {
                angles_degrees: [-90.0, 30.0, 30.0, -30.0],
                ..Rig::default()
            }),
        ),
        ("zero metres", stereo_camera().metres(0.0)),
        ("a position at infinity", stereo_camera().at(f64::INFINITY, 0.0, 0.0)),
    ];
    for (name, camera) in cases {
        let p = plan(&stereo_room(camera), &head_pose(0.0, 0.0, 0.0), &opts());
        assert!(p.layers.is_empty(), "{name}: {p:?}");
        assert_eq!(
            p.skipped.iter().map(|s| (s.id, s.reason)).collect::<Vec<_>>(),
            vec![(51, SkipReason::InvalidCamera)],
            "{name}"
        );
    }
}

#[test]
fn a_zero_size_stereo_view_is_skipped_as_an_invalid_size_not_a_bad_camera() {
    let tree = scene(
        vec![room(&[RIG, 51]), stereo_camera(), view(51, 0.0, 1080.0).through(RIG)],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_eq!(
        p.skipped.iter().map(|s| s.reason).collect::<Vec<_>>(),
        vec![SkipReason::InvalidSize]
    );
}

#[test]
fn a_one_pixel_wide_stereo_view_gives_its_eyes_no_area() {
    // Half of one column is no column: the compositor draws a blank picture and the planner skips the layer.
    let tree = scene(
        vec![room(&[RIG, 51]), stereo_camera(), view(51, 1.0, 1080.0).through(RIG)],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_eq!(
        p.skipped.iter().map(|s| s.reason).collect::<Vec<_>>(),
        vec![SkipReason::InvalidCamera]
    );
}

#[test]
fn the_head_pose_does_not_move_a_projection_layers_eyes() {
    let tree = stereo_room(stereo_camera());
    let first = plan(&tree, &head_pose(0.0, 1.6, 0.0), &opts());
    let moved = plan(&tree, &head_pose(3.0, 1.2, -2.0), &opts());
    assert_eq!(first, moved);
}

#[test]
fn two_views_that_name_the_same_camera_are_both_projection_layers_in_tree_order() {
    let tree = scene(
        vec![
            room(&[RIG, 51, 52]),
            stereo_camera(),
            view(51, 1920.0, 1080.0).through(RIG),
            view(52, 1920.0, 1080.0).through(RIG),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert_eq!(ids(&p), vec![51, 52]);
    assert!(p.layers.iter().all(|layer| matches!(layer, Layer::Projection(_))));
    let (Layer::Projection(first), Layer::Projection(second)) = (&p.layers[0], &p.layers[1]) else {
        unreachable!("both are projection layers");
    };
    assert_eq!(first.eyes, second.eyes, "the same camera gives the same eyes");
}
