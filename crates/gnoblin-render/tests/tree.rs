//! Contract tests for the scene tree the node API relies on (design/node-api-refactor.md
//! §5 and §8.2).
//!
//! Failure modes come first, then happy paths. A case whose behaviour the tree does not
//! expose yet is `#[ignore]`d with the reason: it is a finding for the node API, not a
//! bug to fix in step 1.

use gnoblin_animation::{PropertyValue, VisualProperty};
use gnoblin_render::{Mat4, NodeSpec, Scene, SceneError};
use gnoblin_types::{NodeId, OutputId, PointF, Rect, Vec3};

mod common;
use common::tree_hash;

fn id(raw: u64) -> NodeId {
    NodeId::new(raw)
}

fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
    Rect::new(x, y, width, height)
}

fn child_of(parent: NodeId, bounds: Rect) -> NodeSpec {
    let mut spec = NodeSpec::new(bounds);
    spec.parent = Some(parent);
    spec
}

fn scene_with_output() -> Scene {
    let mut scene = Scene::new();
    scene.set_output(OutputId::new(0), rect(0, 0, 1920, 1080));
    scene
}

/// The output-plane point that a local point of `node` projects to.
fn project(scene: &Scene, node: NodeId, local: [f64; 2]) -> PointF {
    let world = scene.world_matrix(node).expect("node exists");
    let [x, y, _, w] = world.apply([local[0], local[1], 0.0, 1.0]);
    PointF::new(x / w, y / w)
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn assert_mat_close(actual: &Mat4, expected: &Mat4) {
    for (row_a, row_b) in actual.m.iter().zip(expected.m.iter()) {
        for (a, b) in row_a.iter().zip(row_b.iter()) {
            assert!((a - b).abs() < 1e-9, "matrix differs: {actual:?} vs {expected:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Failure modes
// ---------------------------------------------------------------------------

#[test]
fn inserting_a_duplicate_id_is_refused_and_the_original_node_is_kept() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 10, 10)))
        .expect("first insert");
    let error = scene
        .insert(id(1), NodeSpec::new(rect(50, 50, 20, 20)))
        .expect_err("a duplicate id must be refused");
    assert!(matches!(error, SceneError::DuplicateNode(node) if node == id(1)));
    assert_eq!(scene.bounds(id(1)), Some(rect(0, 0, 10, 10)));
}

#[test]
fn inserting_under_a_missing_parent_is_refused_and_creates_no_node() {
    let mut scene = Scene::new();
    let error = scene
        .insert(id(2), child_of(id(1), rect(0, 0, 10, 10)))
        .expect_err("a missing parent must be refused");
    assert!(matches!(error, SceneError::UnknownParent(parent) if parent == id(1)));
    assert!(!scene.contains(id(2)));
}

#[test]
fn a_zero_scale_at_insert_is_refused_by_name_and_creates_no_node() {
    let mut scene = Scene::new();
    let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
    spec.transform.base.scale.x = 0.0;
    let error = scene
        .insert(id(1), spec)
        .expect_err("a non-invertible transform must be refused");
    assert!(matches!(error, SceneError::NotInvertible { field: "scale.x" }));
    assert!(!scene.contains(id(1)));
}

#[test]
fn a_zero_scale_written_through_set_property_is_refused_and_the_transform_is_unchanged() {
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).expect("node");
    let before = *scene.transform(id(1)).expect("node");
    let error = scene
        .set_property(id(1), VisualProperty::ScaleX, PropertyValue::Number(0.0))
        .expect_err("a non-invertible write must be refused");
    assert!(matches!(error, SceneError::NotInvertible { field: "scale.x" }));
    assert_eq!(*scene.transform(id(1)).expect("node"), before);
}

#[test]
fn a_nan_offset_at_insert_is_refused_and_creates_no_node() {
    let mut scene = Scene::new();
    let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
    spec.transform.base.offset.x = f64::NAN;
    let error = scene.insert(id(1), spec).expect_err("NaN must be refused");
    assert!(matches!(error, SceneError::NonFinite { .. }));
    assert!(!scene.contains(id(1)));
}

#[test]
fn a_nan_written_through_set_property_is_refused_and_the_value_is_unchanged() {
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).expect("node");
    let result = scene.set_property(id(1), VisualProperty::OffsetX, PropertyValue::Number(f32::NAN));
    assert!(result.is_err(), "NaN must be refused");
    assert_eq!(
        scene.property(id(1), VisualProperty::OffsetX).expect("node"),
        PropertyValue::Number(0.0)
    );
}

#[test]
fn operations_on_an_unknown_node_are_refused_and_change_nothing() {
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).expect("node");
    let before = scene.node_ids().collect::<Vec<_>>();
    assert!(matches!(scene.remove(id(99)), Err(SceneError::UnknownNode(node)) if node == id(99)));
    assert!(matches!(
        scene.move_to(id(99), 3),
        Err(SceneError::UnknownNode(node)) if node == id(99)
    ));
    assert!(matches!(
        scene.set_property(id(99), VisualProperty::Opacity, PropertyValue::Number(0.5)),
        Err(SceneError::UnknownNode(node)) if node == id(99)
    ));
    assert_eq!(scene.node_ids().collect::<Vec<_>>(), before);
}

#[test]
fn a_non_finite_hit_point_hits_nothing() {
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 100, 100))).expect("node");
    assert_eq!(scene.hit_test(PointF::new(f64::NAN, 50.0)), None);
    assert_eq!(scene.hit_test(PointF::new(50.0, f64::INFINITY)), None);
}

#[test]
fn an_empty_scene_hits_nothing() {
    assert_eq!(Scene::new().hit_test(PointF::new(0.0, 0.0)), None);
}

// Reparenting (design §5 and §8.2).

#[test]
fn reparent_keeps_state_and_moves_paint_order() {
    let mut scene = scene_with_output();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 400, 400)))
        .expect("old parent");
    let mut new_parent = NodeSpec::new(rect(0, 0, 400, 400));
    new_parent.transform.base.offset = Vec3::new(500.0, 0.0, 0.0);
    scene.insert(id(2), new_parent).expect("new parent");
    scene
        .insert(id(3), child_of(id(1), rect(10, 10, 100, 100)))
        .expect("moving node");
    scene
        .insert(id(4), child_of(id(3), rect(0, 0, 20, 20)))
        .expect("its child");
    let above = child_of(id(2), rect(0, 0, 50, 50));
    scene.insert(id(5), above).expect("sibling above");
    scene.set_clip(id(3), true).expect("clip");
    scene
        .set_property(id(3), VisualProperty::Opacity, PropertyValue::Number(0.5))
        .expect("opacity");
    let before = scene.absolute_bounds(id(3)).expect("bounds before the move");
    scene.take_damage();

    scene.reparent_at(id(3), Some(id(2)), 0).expect("reparent");

    assert_eq!(scene.parent(id(3)), Some(id(2)));
    assert!(
        scene.children(id(1)).is_some_and(<[NodeId]>::is_empty),
        "old parent keeps a child"
    );
    assert_eq!(
        scene.children(id(2)),
        Some(&[id(3), id(5)][..]),
        "the moved node is placed first, so the sibling above it paints over it"
    );
    assert_eq!(scene.children(id(3)), Some(&[id(4)][..]), "its subtree moves with it");
    assert_eq!(scene.bounds(id(3)), Some(rect(10, 10, 100, 100)));
    assert_eq!(scene.clip(id(3)), Some(true));
    assert_eq!(scene.composed_opacity(id(3)), Some(0.5));

    let after = scene.absolute_bounds(id(3)).expect("bounds after the move");
    assert_ne!(before, after, "the move must change where the node is");
    let damage = scene.take_damage();
    let output = damage
        .outputs
        .iter()
        .find(|entry| entry.output == OutputId::new(0))
        .expect("the output is damaged");
    assert!(output.covers(before), "old position not damaged");
    assert!(output.covers(after), "new position not damaged");

    // Refused moves change nothing: a cycle, an unknown node and an unknown parent.
    let snapshot = tree_hash(&scene);
    assert!(matches!(
        scene.reparent(id(2), Some(id(4))),
        Err(SceneError::Cycle { node, parent }) if node == id(2) && parent == id(4)
    ));
    assert!(matches!(
        scene.reparent(id(3), Some(id(3))),
        Err(SceneError::Cycle { .. })
    ));
    assert!(matches!(scene.reparent(id(99), None), Err(SceneError::UnknownNode(node)) if node == id(99)));
    assert!(matches!(
        scene.reparent(id(3), Some(id(99))),
        Err(SceneError::UnknownParent(parent)) if parent == id(99)
    ));
    assert_eq!(tree_hash(&scene), snapshot);
}

#[test]
fn a_node_with_visible_false_and_its_subtree_are_not_hit() {
    let mut scene = scene_with_output();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 400, 400))).expect("root");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    scene
        .insert(id(3), child_of(id(2), rect(0, 0, 50, 50)))
        .expect("grandchild");
    let hit = |scene: &Scene| scene.hit_test(PointF::new(10.0, 10.0)).map(|hit| hit.node);
    assert_eq!(hit(&scene), Some(id(3)));

    scene.set_visible(id(2), false).expect("hide the child");
    assert_eq!(scene.visible(id(2)), Some(false));
    // Hiding the child hides its subtree too: the root is what the point reaches.
    assert_eq!(hit(&scene), Some(id(1)));

    scene.set_visible(id(1), false).expect("hide the root");
    assert_eq!(hit(&scene), None);

    scene.set_visible(id(1), true).expect("show the root");
    scene.set_visible(id(2), true).expect("show the child");
    assert_eq!(hit(&scene), Some(id(3)));
    assert!(matches!(scene.set_visible(id(99), false), Err(SceneError::UnknownNode(node)) if node == id(99)));
}

#[test]
fn hit_test_ignores_nodes_with_input_unset() {
    let mut scene = scene_with_output();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene.insert(id(2), child_of(id(1), rect(0, 0, 50, 50))).expect("child");
    let hit = |scene: &Scene, x: f64, y: f64| scene.hit_test(PointF::new(x, y)).map(|hit| hit.node);

    // Input defaults to set, so existing callers keep hitting the child.
    assert_eq!(scene.input(id(2)), Some(true));
    assert_eq!(hit(&scene, 10.0, 10.0), Some(id(2)));

    scene.set_input(id(2), false).expect("unset the child's input");
    assert_eq!(scene.input(id(2)), Some(false));
    // The child passes the hit through to the parent under it.
    assert_eq!(hit(&scene, 10.0, 10.0), Some(id(1)));

    scene.set_input(id(1), false).expect("unset the parent's input");
    assert_eq!(hit(&scene, 10.0, 10.0), None);
    assert_eq!(hit(&scene, 75.0, 75.0), None);
    assert!(matches!(scene.set_input(id(99), false), Err(SceneError::UnknownNode(node)) if node == id(99)));
}

#[test]
fn clip_true_clips_child_hits_to_the_parent_rectangle() {
    let mut scene = scene_with_output();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    // The child reaches past the parent's right and bottom edges.
    scene
        .insert(id(2), child_of(id(1), rect(50, 50, 200, 200)))
        .expect("child");
    let hit = |scene: &Scene, x: f64, y: f64| scene.hit_test(PointF::new(x, y)).map(|hit| hit.node);

    // Unclipped, a point past the parent's edge reaches the child.
    assert_eq!(hit(&scene, 150.0, 150.0), Some(id(2)));

    scene.set_clip(id(1), true).expect("clip the parent");
    assert_eq!(scene.clip(id(1)), Some(true));
    // Clipped, the same point no longer reaches the child.
    assert_eq!(hit(&scene, 150.0, 150.0), None);
    // A point inside both rectangles still reaches the child.
    assert_eq!(hit(&scene, 60.0, 60.0), Some(id(2)));

    scene.set_clip(id(1), false).expect("lift the clip");
    assert_eq!(hit(&scene, 150.0, 150.0), Some(id(2)));
    assert!(matches!(scene.set_clip(id(99), true), Err(SceneError::UnknownNode(node)) if node == id(99)));
}

#[test]
fn opacity_is_clamped_between_zero_and_one() {
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).expect("node");
    scene
        .set_property(id(1), VisualProperty::Opacity, PropertyValue::Number(2.0))
        .expect("write");
    let opacity = scene.transform(id(1)).expect("node").opacity;
    assert!((0.0..=1.0).contains(&opacity), "opacity {opacity} escaped [0, 1]");
}

#[test]
fn opacity_multiplies_down_the_tree() {
    let mut scene = scene_with_output();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 400, 400))).expect("root");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    scene
        .insert(id(3), child_of(id(2), rect(0, 0, 50, 50)))
        .expect("grandchild");
    for node in [id(1), id(2)] {
        scene
            .set_property(node, VisualProperty::Opacity, PropertyValue::Number(0.5))
            .expect("opacity");
    }
    assert_eq!(scene.composed_opacity(id(1)), Some(0.5));
    assert_eq!(scene.composed_opacity(id(2)), Some(0.25));
    assert_eq!(scene.composed_opacity(id(3)), Some(0.25));
    assert_eq!(scene.composed_opacity(id(99)), None);
}

#[test]
fn coordinates_beyond_two_to_the_twentieth_are_refused() {
    let mut scene = Scene::new();
    let far = NodeSpec::new(rect((1 << 20) + 1, 0, 10, 10));
    assert!(scene.insert(id(1), far).is_err(), "a node at x = 2^20 + 1 was accepted");
}

// ---------------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------------

#[test]
fn a_child_draws_above_its_parent_when_they_overlap() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    let hit = scene.hit_test(PointF::new(50.0, 50.0)).expect("hit");
    assert_eq!(hit.node, id(2));
}

#[test]
fn a_child_stays_above_its_parent_when_the_parent_is_raised() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    scene.raise(id(1)).expect("raise");
    let hit = scene.hit_test(PointF::new(50.0, 50.0)).expect("hit");
    assert_eq!(hit.node, id(2));
}

#[test]
fn the_later_sibling_is_hit_first_because_insert_order_is_paint_order() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("first");
    scene
        .insert(id(3), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("second");
    assert_eq!(scene.children(id(1)), Some(&[id(2), id(3)][..]));
    let hit = scene.hit_test(PointF::new(50.0, 50.0)).expect("hit");
    assert_eq!(hit.node, id(3));
}

#[test]
fn raise_moves_a_sibling_last_so_it_is_hit_first() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("first");
    scene
        .insert(id(3), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("second");
    scene.raise(id(2)).expect("raise");
    assert_eq!(scene.children(id(1)), Some(&[id(3), id(2)][..]));
    let hit = scene.hit_test(PointF::new(50.0, 50.0)).expect("hit");
    assert_eq!(hit.node, id(2));
}

#[test]
fn lower_moves_a_sibling_first_so_the_others_are_hit_first() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("first");
    scene
        .insert(id(3), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("second");
    scene.lower(id(3)).expect("lower");
    assert_eq!(scene.children(id(1)), Some(&[id(3), id(2)][..]));
    let hit = scene.hit_test(PointF::new(50.0, 50.0)).expect("hit");
    assert_eq!(hit.node, id(2));
}

#[test]
fn move_to_places_a_sibling_at_the_given_index() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    for child in 2..=4 {
        scene
            .insert(id(child), child_of(id(1), rect(0, 0, 100, 100)))
            .expect("sibling");
    }
    scene.move_to(id(2), 2).expect("move");
    assert_eq!(scene.children(id(1)), Some(&[id(3), id(4), id(2)][..]));
}

#[test]
fn insert_at_places_a_new_sibling_at_the_given_index() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("first");
    scene
        .insert(id(3), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("second");
    scene
        .insert_at(id(4), child_of(id(1), rect(0, 0, 100, 100)), 0)
        .expect("insert at the bottom");
    assert_eq!(scene.children(id(1)), Some(&[id(4), id(2), id(3)][..]));
    let hit = scene.hit_test(PointF::new(50.0, 50.0)).expect("hit");
    assert_eq!(hit.node, id(3));
}

#[test]
fn insert_at_an_index_past_the_end_is_refused_and_creates_no_node() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    let error = scene
        .insert_at(id(3), child_of(id(1), rect(0, 0, 100, 100)), 5)
        .expect_err("an index past the end must be refused");
    assert!(matches!(error, SceneError::IndexOutOfRange { index: 5, len: 1 }));
    assert!(!scene.contains(id(3)));
}

#[test]
fn raise_lower_and_move_to_refuse_an_index_past_the_end_and_change_nothing() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    let before = tree_hash(&scene);
    assert!(matches!(
        scene.move_to(id(2), 1),
        Err(SceneError::IndexOutOfRange { index: 1, len: 1 })
    ));
    assert_eq!(tree_hash(&scene), before);
}

#[test]
fn set_order_permutes_siblings_and_refuses_anything_that_is_not_a_permutation() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    for child in 2..=4 {
        scene
            .insert(id(child), child_of(id(1), rect(0, 0, 100, 100)))
            .expect("sibling");
    }
    scene
        .set_order(Some(id(1)), &[id(4), id(2), id(3)])
        .expect("permutation");
    assert_eq!(scene.children(id(1)), Some(&[id(4), id(2), id(3)][..]));

    for order in [
        &[id(2), id(3)][..],
        &[id(2), id(2), id(3)][..],
        &[id(2), id(3), id(4), id(9)][..],
    ] {
        assert!(matches!(
            scene.set_order(Some(id(1)), order),
            Err(SceneError::NotAPermutation { parent: Some(parent) }) if parent == id(1)
        ));
    }
    assert_eq!(scene.children(id(1)), Some(&[id(4), id(2), id(3)][..]));
}

#[test]
fn set_order_reorders_the_roots() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("first root");
    scene
        .insert(id(2), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("second root");
    scene.set_order(None, &[id(2), id(1)]).expect("permutation");
    let hit = scene.hit_test(PointF::new(50.0, 50.0)).expect("hit");
    assert_eq!(hit.node, id(1));
}

#[test]
fn reparent_at_places_the_moved_node_at_the_given_index() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene
        .insert(id(2), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    scene
        .insert(id(3), child_of(id(1), rect(0, 0, 100, 100)))
        .expect("child");
    scene.insert(id(4), NodeSpec::new(rect(0, 0, 100, 100))).expect("root");
    scene.reparent_at(id(4), Some(id(1)), 0).expect("reparent");
    assert_eq!(scene.children(id(1)), Some(&[id(4), id(2), id(3)][..]));
    assert_eq!(scene.parent(id(4)), Some(id(1)));
    assert!(matches!(
        scene.reparent_at(id(4), None, 3),
        Err(SceneError::IndexOutOfRange { index: 3, len: 1 })
    ));
}

#[test]
fn raise_damages_the_sibling_at_its_old_and_new_position() {
    let mut scene = scene_with_output();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 400, 400)))
        .expect("parent");
    scene.insert(id(2), child_of(id(1), rect(0, 0, 50, 50))).expect("first");
    scene
        .insert(id(3), child_of(id(1), rect(100, 100, 50, 50)))
        .expect("second");
    scene.take_damage();

    scene.move_to(id(2), 1).expect("move");

    let damage = scene.take_damage();
    let output = damage
        .outputs
        .iter()
        .find(|entry| entry.output == OutputId::new(0))
        .expect("the output is damaged");
    assert!(output.covers(rect(0, 0, 50, 50)), "old position not damaged");
    assert!(
        output.covers(rect(100, 100, 50, 50)),
        "the sibling it passed is not damaged"
    );
}

#[test]
fn a_hit_on_a_child_reports_the_point_in_the_childs_own_space() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 200, 200)))
        .expect("parent");
    let mut child = child_of(id(1), rect(0, 0, 100, 50));
    child.transform.base.offset = Vec3::new(50.0, 25.0, 0.0);
    scene.insert(id(2), child).expect("child");
    let hit = scene.hit_test(PointF::new(75.0, 40.0)).expect("hit");
    assert_eq!(hit.node, id(2));
    assert_close(hit.local.x, 25.0);
    assert_close(hit.local.y, 15.0);
}

#[test]
fn removing_a_node_returns_and_removes_its_whole_subtree() {
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 100, 100))).expect("root");
    scene.insert(id(2), child_of(id(1), rect(0, 0, 50, 50))).expect("child");
    scene
        .insert(id(3), child_of(id(2), rect(0, 0, 10, 10)))
        .expect("grandchild");
    let mut removed = scene.remove(id(1)).expect("remove");
    removed.sort();
    assert_eq!(removed, vec![id(1), id(2), id(3)]);
    for raw in 1..=3 {
        assert!(!scene.contains(id(raw)), "node {raw} survived its subtree's removal");
    }
    assert_eq!(scene.hit_test(PointF::new(5.0, 5.0)), None);
}

#[test]
fn after_a_child_is_removed_hits_on_its_area_go_to_its_parent() {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(rect(0, 0, 100, 100)))
        .expect("parent");
    scene.insert(id(2), child_of(id(1), rect(0, 0, 50, 50))).expect("child");
    assert_eq!(scene.hit_test(PointF::new(10.0, 10.0)).map(|hit| hit.node), Some(id(2)));
    scene.remove(id(2)).expect("remove");
    assert_eq!(scene.hit_test(PointF::new(10.0, 10.0)).map(|hit| hit.node), Some(id(1)));
}

#[test]
fn a_root_world_matrix_is_its_own_local_matrix() {
    let mut scene = Scene::new();
    let mut root = NodeSpec::new(rect(0, 0, 100, 60));
    root.transform.base.offset = Vec3::new(7.0, 8.0, 0.0);
    root.transform.base.rotation_degrees = Vec3::new(0.0, 0.0, 30.0);
    let base = root.transform.base;
    scene.insert(id(1), root).expect("root");
    let local = Mat4::node_local(&base, Vec3::new(50.0, 30.0, 0.0));
    assert_mat_close(&scene.world_matrix(id(1)).expect("root"), &local);
}

#[test]
fn a_2d_child_global_transform_is_its_parents_times_its_local() {
    let mut scene = Scene::new();
    let mut parent = NodeSpec::new(rect(0, 0, 100, 100));
    parent.transform.base.offset = Vec3::new(10.0, 20.0, 0.0);
    scene.insert(id(1), parent).expect("parent");
    let mut child = child_of(id(1), rect(0, 0, 50, 50));
    child.transform.base.offset = Vec3::new(5.0, 5.0, 0.0);
    let child_base = child.transform.base;
    scene.insert(id(2), child).expect("child");
    let parent_world = scene.world_matrix(id(1)).expect("parent");
    let child_world = scene.world_matrix(id(2)).expect("child");
    let local = Mat4::node_local(&child_base, Vec3::new(25.0, 25.0, 0.0));
    assert_mat_close(&child_world, &parent_world.mul(&local));
    // The child's origin sits at the parent's offset plus its own: (10 + 5, 20 + 5).
    let origin = child_world.apply([0.0, 0.0, 0.0, 1.0]);
    assert_close(origin[0], 15.0);
    assert_close(origin[1], 25.0);
}

#[test]
fn a_3d_child_global_transform_is_its_parents_times_its_local() {
    let mut scene = Scene::new();
    let mut parent = NodeSpec::new(rect(0, 0, 200, 100));
    parent.transform.base.offset = Vec3::new(10.0, 20.0, 30.0);
    parent.transform.base.rotation_degrees = Vec3::new(0.0, 40.0, 0.0);
    parent.transform.base.depth = 5.0;
    scene.insert(id(1), parent).expect("parent");
    let mut child = child_of(id(1), rect(0, 0, 50, 50));
    child.transform.base.offset = Vec3::new(5.0, 6.0, 7.0);
    child.transform.base.rotation_degrees = Vec3::new(30.0, 0.0, 0.0);
    child.transform.base.scale = Vec3::new(1.0, 2.0, 1.0);
    child.transform.base.depth = 3.0;
    let child_base = child.transform.base;
    scene.insert(id(2), child).expect("child");
    let parent_world = scene.world_matrix(id(1)).expect("parent");
    let child_world = scene.world_matrix(id(2)).expect("child");
    let local = Mat4::node_local(&child_base, Vec3::new(25.0, 25.0, 0.0));
    assert_mat_close(&child_world, &parent_world.mul(&local));
}

#[test]
fn a_node_rotated_40_degrees_about_y_is_hit_at_the_point_in_its_own_coordinates() {
    let mut scene = Scene::new();
    let mut node = NodeSpec::new(rect(0, 0, 100, 100));
    node.transform.base.rotation_degrees = Vec3::new(0.0, 40.0, 0.0);
    scene.insert(id(1), node).expect("node");
    let screen = project(&scene, id(1), [30.0, 20.0]);
    let hit = scene.hit_test(screen).expect("the projected point is inside the node");
    assert_eq!(hit.node, id(1));
    assert_close(hit.local.x, 30.0);
    assert_close(hit.local.y, 20.0);
}

#[test]
fn an_opacity_write_is_stored_on_the_node() {
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).expect("node");
    scene
        .set_property(id(1), VisualProperty::Opacity, PropertyValue::Number(0.5))
        .expect("opacity");
    assert_eq!(scene.transform(id(1)).expect("node").opacity, 0.5);
}

#[test]
fn damage_after_an_opacity_write_names_exactly_that_nodes_bounds() {
    let mut scene = scene_with_output();
    scene
        .insert(id(1), NodeSpec::new(rect(100, 100, 50, 40)))
        .expect("node");
    scene
        .insert(id(2), NodeSpec::new(rect(500, 500, 10, 10)))
        .expect("other");
    scene.take_damage();
    scene
        .set_property(id(1), VisualProperty::Opacity, PropertyValue::Number(0.5))
        .expect("opacity");
    let damage = scene.take_damage();
    let output = damage
        .outputs
        .iter()
        .find(|entry| entry.output == OutputId::new(0))
        .expect("the output is damaged");
    assert!(!output.full);
    assert_eq!(output.rects, vec![rect(100, 100, 50, 40)]);
}

#[test]
fn damage_after_an_offset_write_covers_both_the_old_and_the_new_bounds() {
    let mut scene = scene_with_output();
    scene
        .insert(id(1), NodeSpec::new(rect(100, 100, 50, 40)))
        .expect("node");
    scene.take_damage();
    scene
        .set_property(id(1), VisualProperty::OffsetX, PropertyValue::Number(10.0))
        .expect("offset");
    let damage = scene.take_damage();
    let output = damage
        .outputs
        .iter()
        .find(|entry| entry.output == OutputId::new(0))
        .expect("the output is damaged");
    assert!(output.covers(rect(100, 100, 50, 40)), "old bounds not damaged");
    assert!(output.covers(rect(110, 100, 50, 40)), "new bounds not damaged");
}

/// A fixed mutation sequence; applying it twice must build the same tree.
fn apply_sequence(scene: &mut Scene) {
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 640, 480))).expect("root");
    let mut panel = child_of(id(1), rect(10, 10, 200, 100));
    panel.transform.base.offset = Vec3::new(4.0, 4.0, 0.0);
    scene.insert(id(2), panel).expect("panel");
    scene.insert(id(3), child_of(id(2), rect(0, 0, 50, 50))).expect("child");
    scene
        .set_property(id(3), VisualProperty::RotationZ, PropertyValue::Number(15.0))
        .expect("rotation");
    scene.raise(id(3)).expect("raise");
    scene
        .set_property(id(2), VisualProperty::Opacity, PropertyValue::Number(0.5))
        .expect("opacity");
    scene
        .insert(id(4), child_of(id(1), rect(0, 0, 30, 30)))
        .expect("sibling");
    scene.remove(id(2)).expect("remove");
    scene
        .insert(id(5), child_of(id(1), rect(20, 20, 30, 30)))
        .expect("late");
}

#[test]
fn the_same_mutation_sequence_applied_twice_gives_an_identical_tree_hash() {
    let mut first = Scene::new();
    apply_sequence(&mut first);
    let mut second = Scene::new();
    apply_sequence(&mut second);
    assert_eq!(tree_hash(&first), tree_hash(&second));
}

#[test]
fn the_tree_hash_tells_apart_trees_that_differ_in_one_sibling_order() {
    let mut first = Scene::new();
    apply_sequence(&mut first);
    let mut second = Scene::new();
    apply_sequence(&mut second);
    second.raise(id(4)).expect("raise");
    assert_ne!(tree_hash(&first), tree_hash(&second));
}

// ---- Slice 3: ownership, caps, subtree removal (design/node-core.md §3, §8) ----
//
// Ids: the compositor allocates from `COMPOSITOR_ID_BASE` (bit 63 set); the shell's ids
// are below `SHELL_ID_LIMIT` (bit 63 clear). Owner is the actor that inserted the node.
// A node's `parented_by` is the actor that last gave it a parent (None when unparented).

use gnoblin_render::Actor;
use gnoblin_render::scene_ids::{COMPOSITOR_ID_BASE, MAX_DEPTH, MAX_NODES, SHELL_ID_LIMIT};

fn cid(n: u64) -> NodeId {
    NodeId::new(COMPOSITOR_ID_BASE + n)
}

#[test]
fn id_ranges_cannot_collide() {
    assert_eq!(SHELL_ID_LIMIT, 1 << 63);
    assert_eq!(COMPOSITOR_ID_BASE, 1 << 63);
    assert!(cid(1).get() >= COMPOSITOR_ID_BASE);
    assert!(id(1).get() < SHELL_ID_LIMIT);
}

#[test]
fn f8_op_on_destroyed_id_is_unknown_node() {
    let mut scene = scene_with_output();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).unwrap();
    scene.remove(id(1)).unwrap();
    assert_eq!(scene.set_visible(id(1), false), Err(SceneError::UnknownNode(id(1))));
}

#[test]
fn f10_reparent_into_own_subtree_is_cycle() {
    let mut scene = scene_with_output();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).unwrap();
    scene.insert(id(2), child_of(id(1), rect(0, 0, 5, 5))).unwrap();
    assert_eq!(
        scene.reparent(id(1), Some(id(2))),
        Err(SceneError::Cycle {
            node: id(1),
            parent: id(2)
        })
    );
}

#[test]
fn f13_coordinate_beyond_bound_is_out_of_bounds() {
    let mut scene = scene_with_output();
    let far = NodeSpec::new(rect(0, 0, 1 << 21, 10));
    assert!(matches!(scene.insert(id(1), far), Err(SceneError::OutOfBounds { .. })));
    assert!(!scene.contains(id(1)));
}

#[test]
fn f14_nan_is_not_finite() {
    let mut scene = scene_with_output();
    let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
    spec.transform.base.offset = Vec3::new(f64::NAN, 0.0, 0.0);
    assert!(matches!(scene.insert(id(1), spec), Err(SceneError::NonFinite { .. })));
    assert!(!scene.contains(id(1)));
}

#[test]
fn f15_insert_with_live_id_is_duplicate_node() {
    let mut scene = scene_with_output();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).unwrap();
    assert_eq!(
        scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))),
        Err(SceneError::DuplicateNode(id(1)))
    );
}

#[test]
fn f18_node_cap_refuses_with_too_many_nodes() {
    assert_eq!(MAX_NODES, 65536);
    let mut scene = Scene::new();
    for n in 1..=MAX_NODES as u64 {
        scene.insert(id(n), NodeSpec::new(rect(0, 0, 1, 1))).unwrap();
    }
    assert_eq!(
        scene.insert(id(MAX_NODES as u64 + 1), NodeSpec::new(rect(0, 0, 1, 1))),
        Err(SceneError::TooManyNodes)
    );
}

#[test]
fn f38_depth_cap_refuses_with_too_deep() {
    assert_eq!(MAX_DEPTH, 256);
    let mut scene = Scene::new();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 1, 1))).unwrap();
    for depth in 2..=MAX_DEPTH as u64 {
        scene
            .insert(id(depth), child_of(id(depth - 1), rect(0, 0, 1, 1)))
            .unwrap();
    }
    let too_deep = child_of(id(MAX_DEPTH as u64), rect(0, 0, 1, 1));
    assert_eq!(scene.insert(id(1000), too_deep), Err(SceneError::TooDeep));
    assert!(!scene.contains(id(1000)));
}

#[test]
fn destroy_returns_removed_subtree_in_tree_order() {
    let mut scene = scene_with_output();
    scene.insert(id(1), NodeSpec::new(rect(0, 0, 10, 10))).unwrap();
    scene.insert(id(2), child_of(id(1), rect(0, 0, 5, 5))).unwrap();
    scene.insert(id(3), child_of(id(1), rect(0, 0, 5, 5))).unwrap();
    scene.insert(id(4), child_of(id(2), rect(0, 0, 2, 2))).unwrap();
    assert_eq!(scene.remove(id(1)).unwrap(), vec![id(1), id(2), id(4), id(3)]);
    assert_eq!(scene.node_ids().count(), 0);
}

#[test]
fn shell_can_destroy_its_own_node() {
    let mut scene = scene_with_output();
    scene
        .insert_as(Actor::Shell, id(10), NodeSpec::new(rect(0, 0, 10, 10)))
        .unwrap();
    assert_eq!(scene.remove_as(Actor::Shell, id(10)), Ok(vec![id(10)]));
}

#[test]
fn shell_destroy_of_compositor_node_is_not_owner() {
    let mut scene = scene_with_output();
    scene.insert(cid(1), NodeSpec::new(rect(0, 0, 10, 10))).unwrap();
    assert_eq!(scene.remove_as(Actor::Shell, cid(1)), Err(SceneError::NotOwner(cid(1))));
    assert!(scene.contains(cid(1)));
}

#[test]
fn shell_cannot_insert_under_compositor_id_range() {
    let mut scene = scene_with_output();
    assert_eq!(
        scene.insert_as(Actor::Shell, cid(5), NodeSpec::new(rect(0, 0, 1, 1))),
        Err(SceneError::NotOwner(cid(5)))
    );
}

#[test]
fn shell_moves_unparented_compositor_node() {
    let mut scene = scene_with_output();
    scene
        .insert_as(Actor::Shell, id(10), NodeSpec::new(rect(0, 0, 100, 100)))
        .unwrap();
    scene.insert(cid(1), NodeSpec::new(rect(3, 4, 20, 30))).unwrap();
    scene.reparent_as(Actor::Shell, cid(1), Some(id(10))).unwrap();
    assert_eq!(scene.parent(cid(1)), Some(id(10)));
}

#[test]
fn shell_cannot_move_compositor_parented_node_not_owner() {
    let mut scene = scene_with_output();
    scene.insert(cid(1), NodeSpec::new(rect(0, 0, 100, 100))).unwrap();
    scene.insert(cid(2), child_of(cid(1), rect(0, 0, 10, 10))).unwrap();
    scene
        .insert_as(Actor::Shell, id(10), NodeSpec::new(rect(0, 0, 100, 100)))
        .unwrap();
    assert_eq!(
        scene.reparent_as(Actor::Shell, cid(2), Some(id(10))),
        Err(SceneError::NotOwner(cid(2)))
    );
    assert_eq!(scene.parent(cid(2)), Some(cid(1)));
}

#[test]
fn shell_moves_node_it_parented() {
    let mut scene = scene_with_output();
    scene
        .insert_as(Actor::Shell, id(10), NodeSpec::new(rect(0, 0, 100, 100)))
        .unwrap();
    scene
        .insert_as(Actor::Shell, id(11), child_of(id(10), rect(0, 0, 5, 5)))
        .unwrap();
    scene.reparent_as(Actor::Shell, id(11), None).unwrap();
    assert_eq!(scene.parent(id(11)), None);
}

#[test]
fn reparent_keeps_id_and_geometry() {
    let mut scene = scene_with_output();
    scene
        .insert_as(Actor::Shell, id(10), NodeSpec::new(rect(0, 0, 100, 100)))
        .unwrap();
    scene.insert(cid(1), NodeSpec::new(rect(7, 8, 20, 30))).unwrap();
    let before = scene.bounds(cid(1));
    scene.reparent_as(Actor::Shell, cid(1), Some(id(10))).unwrap();
    assert!(scene.contains(cid(1)));
    assert_eq!(scene.bounds(cid(1)), before);
}

#[test]
fn shell_write_to_compositor_window_property_applies() {
    // Ownership governs lifecycle and parentage only: the shell places and shows a
    // compositor-owned window by writing its properties, and the write applies.
    let mut scene = scene_with_output();
    scene.insert(cid(1), NodeSpec::new(rect(0, 0, 100, 100))).unwrap();
    scene
        .set_property(cid(1), VisualProperty::OffsetX, PropertyValue::Number(42.0))
        .unwrap();
    scene.set_visible(cid(1), false).unwrap();
    assert_eq!(scene.visible(cid(1)), Some(false));
    assert_eq!(
        scene.property(cid(1), VisualProperty::OffsetX),
        Ok(PropertyValue::Number(42.0))
    );
}
