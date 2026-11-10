//! Tests for `plan`: the translation from the `room` subtree to layers.

use std::collections::BTreeMap;

use xr_scene::{
    CameraProjection, Class, HeadPose, IDENTITY, Layer, NodeId, Options, Plan, Pose, SkipReason, Space, TreeNode,
    TreeView, plan,
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
            width,
            height,
        },
    )
}

fn group(id: NodeId) -> TreeNode {
    TreeNode::new(id, Class::Node)
}

fn camera(id: NodeId, projection: CameraProjection) -> TreeNode {
    TreeNode::new(id, Class::Camera { projection })
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
        Layer::Quad(p) | Layer::Projection(p) => p.id,
    }
}

fn quad(layer: &Layer) -> &xr_scene::Placement {
    match layer {
        Layer::Quad(p) => p,
        Layer::Projection(p) => panic!("expected a quad, got a projection for {}", p.id),
    }
}

fn assert_vec_close(actual: [f64; 3], expected: [f64; 3]) {
    for (a, e) in actual.iter().zip(expected.iter()) {
        assert!((a - e).abs() < EPS, "{actual:?} != {expected:?}");
    }
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
fn stereo_camera_view_becomes_projection() {
    let tree = scene(
        vec![
            room(&[90]),
            camera(90, CameraProjection::Stereo).children(&[91]),
            view(91, 1.0, 1.0),
        ],
        &[ROOM],
    );
    let p = plan(&tree, &head_pose(0.0, 0.0, 0.0), &opts());
    assert!(matches!(p.layers.as_slice(), [Layer::Projection(q)] if q.id == 91 && q.space == Space::World));
}

#[test]
fn head_locked_view_under_stereo_head_is_quad() {
    // The head camera is stereo, but head-locked wins: the View is a quad in the View space.
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
