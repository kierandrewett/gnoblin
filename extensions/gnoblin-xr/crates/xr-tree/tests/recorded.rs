//! The planner against a recorded tree.
//!
//! `recorded/stock-ring.frames` is what the owner sent to an observer while the real compositor and the stock retired shell
//! ran under Monado's simulated headset (`XR_SMOKE_RECORD=<file> scripts/xr-smoke.sh`): the room built when the `xr` seat
//! appeared, one window mapped, a second window mapped and the focus moved to it, the second closed, and the room taken
//! down when the extension went. Each test replays it one change batch at a time, so the planner meets the tree at every
//! state the stream passed through, and runs it for a head that walks and turns.
//!
//! The extension is purely translational, and these tests hold it to that: a layer is the View node's own pose and size
//! in metres, the head changes none of them, and where the panels stand is what the shell's ring rules (`docs/xr.md`,
//! "The ring") say, worked out here from the node sizes and not read from the poses.

use xr_scene::head::camera_pose;
use xr_scene::{Class, HeadPose, IDENTITY, Layer, NodeId, Options, Placement, Plan, Space, TreeView, plan};
use xr_tree::TreeClient;
use xr_tree::replay::{Recording, replay};

const RECORDING: &[u8] = include_bytes!("recorded/stock-ring.frames");

/// The metres per unit the shell's room uses: its head camera's `metres`, which is the schema default.
const METRES: f64 = 0.001;
const EPS: f64 = 1e-9;

/// The tree after the snapshot and after each change batch that followed it, in order.
fn states() -> Vec<TreeView> {
    let recording = Recording::parse(RECORDING).expect("the recording parses");
    let (socket, mut stepper) = replay(&recording);
    let mut client = TreeClient::from_stream(socket).expect("attach over the replay");
    let mut states = vec![client.model().clone()];
    while stepper.step() {
        client.poll().expect("the batch applies");
        states.push(client.model().clone());
    }
    assert_eq!(states.len(), recording.steps() + 1);
    states
}

/// A head that walks across the room and turns on the spot: poses from `(-1, 1.6, 0)` to `(2, 1.2, -2)`, with a yaw
/// that goes once round.
fn head_path() -> Vec<HeadPose> {
    (0..16)
        .map(|step| {
            let t = f64::from(step) / 15.0;
            let yaw = t * std::f64::consts::TAU;
            HeadPose {
                position: [-1.0 + 3.0 * t, 1.6 - 0.4 * t, -2.0 * t],
                orientation: [0.0, (yaw / 2.0).sin(), 0.0, (yaw / 2.0).cos()],
            }
        })
        .collect()
}

fn neutral() -> HeadPose {
    HeadPose {
        position: [0.0; 3],
        orientation: IDENTITY,
    }
}

fn the_plan(tree: &TreeView, head: &HeadPose) -> Plan {
    plan(tree, head, &Options::default())
}

fn placements(plan: &Plan) -> Vec<&Placement> {
    plan.layers
        .iter()
        .map(|layer| match layer {
            Layer::Quad(placement) => placement,
            Layer::Projection(_) => panic!("the stock shell makes no stereo View: {layer:?}"),
        })
        .collect()
}

/// `[x, y, z]` of the direction `+z` takes under the unit quaternion `q`.
fn forward(q: [f64; 4]) -> [f64; 3] {
    let [x, y, z, w] = q;
    [
        2.0 * (x * z + w * y),
        2.0 * (y * z - w * x),
        1.0 - 2.0 * (x * x + y * y),
    ]
}

fn close(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 1e-6, "{what}: {actual} != {expected}");
}

/// The room and the View node behind each layer's id.
fn node(tree: &TreeView, id: NodeId) -> &xr_scene::TreeNode {
    &tree.nodes[&id]
}

fn room(tree: &TreeView) -> Option<&xr_scene::TreeNode> {
    tree.nodes.values().find(|node| node.name.as_deref() == Some("room"))
}

#[test]
fn the_stream_builds_the_room_fills_it_with_two_panels_and_takes_it_down() {
    let counts: Vec<usize> = states()
        .iter()
        .map(|tree| the_plan(tree, &neutral()).layers.len())
        .collect();
    let mut changes = counts.clone();
    changes.dedup();
    assert_eq!(
        changes,
        [0, 1, 2, 1, 0],
        "no room, one window, two windows, one window closed, the room gone: {counts:?}"
    );
    let states = states();
    assert!(room(&states[0]).is_none(), "the snapshot has no room yet");
    assert!(
        room(states.last().expect("a last state")).is_none(),
        "the room is gone at the end"
    );
    assert!(
        states.iter().all(|tree| the_plan(tree, &neutral()).skipped.is_empty()),
        "the stock shell never makes a View the planner cannot place"
    );
}

#[test]
fn a_layer_is_the_view_nodes_own_pose_and_size_in_metres() {
    let mut checked = 0;
    for tree in states() {
        let Some(room) = room(&tree) else { continue };
        assert_eq!(room.rotation, [0.0; 3], "the room is not turned");
        assert_eq!(room.scale, 1.0, "the room is not scaled");
        let layers = the_plan(&tree, &neutral());
        for placement in placements(&layers) {
            let view = node(&tree, placement.id);
            let Class::View {
                source, width, height, ..
            } = view.class
            else {
                panic!("a layer for a node that is not a View");
            };
            assert_eq!(Some(placement.source), source, "the layer shows the View's source");
            // Only the turn about y is used (docs/xr.md), so the node's rotation is a yaw.
            assert_eq!(
                (view.rotation[0], view.rotation[2]),
                (0.0, 0.0),
                "a panel only turns about y"
            );
            for axis in 0..3 {
                close(
                    placement.pose.position[axis],
                    (room.position[axis] + view.position[axis]) * METRES,
                    "the position is the room's and the node's, in metres",
                );
            }
            let yaw = view.rotation[1].to_radians();
            close(
                placement.pose.orientation[1],
                (yaw / 2.0).sin(),
                "the orientation is the node's yaw",
            );
            close(
                placement.pose.orientation[3],
                (yaw / 2.0).cos(),
                "the orientation is the node's yaw",
            );
            close(placement.size[0], width * view.scale * METRES, "the width in metres");
            close(placement.size[1], height * view.scale * METRES, "the height in metres");
            assert_eq!(placement.extent, [width, height]);
            assert_eq!(placement.space, Space::World);
            assert!(placement.input && (placement.opacity - 1.0).abs() < EPS);
            checked += 1;
        }
    }
    assert!(
        checked >= 20,
        "the stream shows panels in many states: {checked} layers checked"
    );
}

#[test]
fn the_head_pose_moves_no_world_layer() {
    let path = head_path();
    for (index, tree) in states().iter().enumerate() {
        let at_rest = the_plan(tree, &neutral());
        for head in &path {
            assert_eq!(
                the_plan(tree, head),
                at_rest,
                "state {index}: the head changed the plan"
            );
        }
    }
}

#[test]
fn a_view_under_the_head_camera_keeps_its_place_in_view_space_as_the_head_walks() {
    // The stock shell puts nothing under the head camera, so the one case a head-locked layer exists in is one made from
    // the recorded tree: a View moved under `head`.
    let tree = states()
        .into_iter()
        .find(|tree| the_plan(tree, &neutral()).layers.len() == 2)
        .expect("a state with two panels");
    let head = tree
        .nodes
        .values()
        .find(|node| node.name.as_deref() == Some("head"))
        .expect("the head camera")
        .id;
    let room_id = room(&tree).expect("a room").id;
    let first_view = node(&tree, room_id)
        .children
        .iter()
        .copied()
        .find(|id| matches!(node(&tree, *id).class, Class::View { .. }))
        .expect("a view");
    let mut moved = tree.clone();
    moved
        .nodes
        .get_mut(&room_id)
        .expect("room")
        .children
        .retain(|id| *id != first_view);
    moved.nodes.get_mut(&head).expect("head").children.push(first_view);

    let at_rest = the_plan(&moved, &neutral());
    let locked = placements(&at_rest)
        .into_iter()
        .find(|placement| placement.id == first_view)
        .expect("the moved View is planned");
    assert_eq!(
        locked.space,
        Space::View,
        "a View beneath the head camera is head-locked"
    );
    for head_pose in head_path() {
        assert_eq!(
            the_plan(&moved, &head_pose),
            at_rest,
            "its place in view space does not depend on the head"
        );
    }
}

/// The ring the shell's rules state (`docs/xr.md`, "The ring"), for the Views of `tree`'s room, in the order the room
/// lists them, with the focus on `focused` if it names one. `None` when it cannot say.
///
/// It is the shell's arithmetic written out again for the two cases the stream holds: one window, and two of which the
/// second is focused. Radius 1.6 m, a focused panel 0.1 m nearer and 15 % larger, 3 degrees between neighbours, 0.75 mm
/// a pixel. It does not read a pose.
struct Ring {
    /// `(angle, radius, scale)` for each View, in room order.
    panels: Vec<(f64, f64, f64)>,
}

fn ring(widths: &[f64], focused: Option<usize>) -> Ring {
    const RADIUS: f64 = 1600.0;
    const PULL: f64 = 100.0;
    const FOCUS_SCALE: f64 = 1.15;
    const MM_PER_PX: f64 = 0.75;
    let gap = 3.0_f64.to_radians();
    let span = |index: usize, big: bool| (if big { FOCUS_SCALE } else { 1.0 }) * widths[index] * MM_PER_PX / RADIUS;
    let mut panels = vec![(0.0, RADIUS, MM_PER_PX); widths.len()];
    match (widths.len(), focused) {
        (1, None) => {}
        (1, Some(0)) => panels[0] = (0.0, RADIUS - PULL, FOCUS_SCALE * MM_PER_PX),
        (2, Some(front)) => {
            let other = 1 - front;
            panels[front] = (0.0, RADIUS - PULL, FOCUS_SCALE * MM_PER_PX);
            // The neighbour stands to the right: past the front panel's half, a gap, and its own half.
            let angle = span(front, true) / 2.0 + gap + span(other, false) / 2.0;
            panels[other] = (angle, RADIUS, MM_PER_PX);
        }
        other => panic!("the stream holds no case for {other:?}"),
    }
    Ring { panels }
}

/// Which View the shell had focused: the one drawn nearest, if one is.
fn focused_view(layers: &[&Placement]) -> Option<usize> {
    let radius = |placement: &Placement| placement.pose.position[0].hypot(placement.pose.position[2]);
    layers.iter().position(|placement| radius(placement) < 1.55)
}

#[test]
fn the_panels_stand_where_the_rings_rules_put_them() {
    let mut settled = 0;
    for (index, tree) in states().iter().enumerate() {
        let layers = the_plan(tree, &neutral());
        let panels = placements(&layers);
        if panels.is_empty() {
            continue;
        }
        let widths: Vec<f64> = panels.iter().map(|placement| placement.extent[0]).collect();
        let expected = ring(&widths, focused_view(&panels));
        for (placement, (angle, radius, scale)) in panels.iter().zip(&expected.panels) {
            let [x, y, z] = placement.pose.position;
            close(
                y,
                0.0,
                &format!("state {index}: a panel's centre is at the viewer's height"),
            );
            close(x, radius * angle.sin() * METRES, &format!("state {index}: x"));
            close(z, -radius * angle.cos() * METRES, &format!("state {index}: z"));
            // A panel faces the viewer: its front, +z after the turn, points back at the origin.
            let [fx, fy, fz] = forward(placement.pose.orientation);
            let (to_viewer_x, to_viewer_z) = (-x / x.hypot(z), -z / x.hypot(z));
            close(
                fx,
                to_viewer_x,
                &format!("state {index}: the panel faces the viewer (x)"),
            );
            close(
                fz,
                to_viewer_z,
                &format!("state {index}: the panel faces the viewer (z)"),
            );
            close(fy, 0.0, &format!("state {index}: the panel stands upright"));
            close(
                placement.size[0],
                placement.extent[0] * scale * METRES,
                &format!("state {index}: the panel's width follows the focus"),
            );
        }
        // Nothing depth-tests, so the nearest panel, the focused one, is the last layer drawn.
        if let Some(front) = focused_view(&panels) {
            assert_eq!(
                front,
                panels.len() - 1,
                "state {index}: the focused panel is drawn last"
            );
        }
        settled += 1;
    }
    assert!(settled >= 20, "the stream holds many states with panels: {settled}");
}

#[test]
fn closing_the_focused_window_puts_the_one_left_at_the_front_unfocused() {
    let states = states();
    let two = states
        .iter()
        .rposition(|tree| the_plan(tree, &neutral()).layers.len() == 2)
        .expect("a state with two panels");
    let one = states[two..]
        .iter()
        .position(|tree| the_plan(tree, &neutral()).layers.len() == 1)
        .map(|offset| two + offset)
        .expect("a later state with one panel");
    let before = the_plan(&states[two], &neutral());
    let after = the_plan(&states[one], &neutral());
    let before = placements(&before);
    let after = placements(&after);
    assert!(
        before.iter().any(|p| p.pose.position[0].abs() > 0.1),
        "one of two panels stands to the side"
    );
    let [alone] = after[..] else { panic!("one panel left") };
    close(alone.pose.position[0], 0.0, "the panel left is at the front");
    close(
        alone.pose.position[2],
        -1.6,
        "and at the ring's radius: it is not pulled in, it has no focus",
    );
    close(alone.size[0], alone.extent[0] * 0.75 * METRES, "and it is not enlarged");
}

// ---------------------------------------------------------------------------
// The head-tracked variant: the same recorded rooms, with one stereo View through the head camera and the camera written
// with the head's pose, as the extension writes it (`xr_scene::head::camera_pose`).

/// The recorded tree with a View of 1600 by 900 units added last under the room, drawn through the head camera: the tree,
/// the View and the camera. `None` while there is no room.
fn with_stereo_view(tree: &TreeView) -> Option<(TreeView, NodeId, NodeId)> {
    let room_id = room(tree)?.id;
    let camera = tree
        .nodes
        .values()
        .find(|node| matches!(node.class, Class::Camera { .. }) && node.name.as_deref() == Some("head"))?
        .id;
    let mut tree = tree.clone();
    let view = 9_000_000_001;
    tree.insert(xr_scene::TreeNode::new(
        view,
        Class::View {
            source: Some(camera),
            camera: Some(camera),
            width: 1600.0,
            height: 900.0,
        },
    ));
    tree.nodes.get_mut(&room_id)?.children.push(view);
    Some((tree, view, camera))
}

fn flat_panels(plan: &Plan) -> Vec<Placement> {
    plan.layers
        .iter()
        .filter_map(|layer| match layer {
            Layer::Quad(placement) => Some(placement.clone()),
            Layer::Projection(_) => None,
        })
        .collect()
}

/// The rotations are equal up to sign: `q` and `-q` are one rotation.
fn same_rotation(a: [f64; 4], b: [f64; 4]) -> bool {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    (dot.abs() - 1.0).abs() < EPS
}

#[test]
fn a_stereo_view_through_the_head_camera_follows_a_walking_turning_head_in_every_recorded_state() {
    let mut checked = 0;
    for tree in states() {
        let Some((mut tree, view, camera)) = with_stereo_view(&tree) else {
            continue;
        };
        let flat = flat_panels(&the_plan(&tree, &neutral()));
        for head in head_path() {
            let metres = tree.nodes[&camera].metres.unwrap_or(METRES);
            let pose = camera_pose(&head, metres).expect("the walking head is a usable pose");
            let node = tree.nodes.get_mut(&camera).expect("the camera");
            node.position = pose.position;
            node.rotation = pose.rotation_degrees;

            let layers = the_plan(&tree, &head);
            let projected = layers
                .layers
                .iter()
                .find_map(|layer| match layer {
                    Layer::Projection(projected) if projected.placement.id == view => Some(projected),
                    _ => None,
                })
                .expect("the View through a stereo camera is a projection layer");
            assert_eq!(projected.camera, camera);
            for axis in 0..3 {
                let middle = (projected.eyes[0].pose.position[axis] + projected.eyes[1].pose.position[axis]) / 2.0;
                assert!(
                    (middle - head.position[axis]).abs() < EPS,
                    "axis {axis}: the eyes meet at {middle}, the head is at {}",
                    head.position[axis]
                );
            }
            for eye in &projected.eyes {
                assert!(
                    same_rotation(eye.pose.orientation, head.orientation),
                    "the eyes turn with the head"
                );
            }
            let eye_distance: f64 = (0..3)
                .map(|axis| (projected.eyes[1].pose.position[axis] - projected.eyes[0].pose.position[axis]).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(
                (eye_distance - 0.063).abs() < 1e-9,
                "the eyes keep the camera's separation: {eye_distance}"
            );
            assert_eq!(
                flat_panels(&layers),
                flat,
                "the head moves no flat panel: the extension stays translational"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 16 * 3,
        "every state that has a room was walked through: {checked}"
    );
}

#[test]
fn the_head_tracked_tree_plans_the_same_layers_every_time() {
    for tree in states() {
        let Some((mut tree, _, camera)) = with_stereo_view(&tree) else {
            continue;
        };
        for head in head_path() {
            let pose = camera_pose(&head, METRES).expect("usable");
            let node = tree.nodes.get_mut(&camera).expect("the camera");
            node.position = pose.position;
            node.rotation = pose.rotation_degrees;
            assert_eq!(the_plan(&tree, &head), the_plan(&tree, &head));
        }
    }
}

#[test]
fn a_head_camera_that_is_gone_leaves_the_stereo_view_a_flat_panel() {
    let mut checked = 0;
    for tree in states() {
        let Some((mut tree, view, camera)) = with_stereo_view(&tree) else {
            continue;
        };
        // The shell took the camera down while the extension was still writing to it.
        tree.nodes.remove(&camera);
        for node in tree.nodes.values_mut() {
            node.children.retain(|child| *child != camera);
        }
        let layers = the_plan(&tree, &neutral());
        assert!(
            flat_panels(&layers).iter().any(|panel| panel.id == view),
            "the View is a flat panel now, and nothing panics"
        );
        assert!(layers.layers.iter().all(|layer| matches!(layer, Layer::Quad(_))));
        checked += 1;
    }
    assert!(checked > 0);
}
