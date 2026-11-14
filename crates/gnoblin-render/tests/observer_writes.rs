//! What an observer's transaction may write beyond its own seat: the XR extension's head pose and layer budget, and the
//! `controller` nodes it makes for the hands.
//!
//! An observer connection (`tree.attach` mode `observer`) is the owner's, but it does not hold the tree. It may set
//! `x`, `y`, `z` and `rotation` on the camera named `head`, and the camera's read-only `layer_budget`. It may make
//! `controller` nodes and write their pose and read-only values, but not their shell-written `claimed` list. Nothing
//! else. Every refusal leaves the scene exactly as it was.

mod common;

use common::tree_hash;
use gnoblin_render::scene_ids::{Actor, COMPOSITOR_ID_BASE};
use gnoblin_render::transaction::{
    ClassInfo, Code, PropertyInfo, TreeChange, TreeOp, TreeTransaction, TreeValue, ValueType, validate_and_apply,
};
use gnoblin_render::{NodeSpec, Scene};
use gnoblin_types::{NodeId, Rect};

struct Table;

impl ClassInfo for Table {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "camera" | "node" | "seat" | "controller")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        let (kind, readonly) = match (class, key) {
            ("camera" | "node", "x" | "y" | "z" | "opacity") => (ValueType::Double, false),
            ("camera" | "node", "rotation") => (ValueType::ListDouble, false),
            ("camera" | "node", "name") => (ValueType::Text, false),
            ("camera", "layer_budget") => (ValueType::Uint, true),
            ("seat", "name") => (ValueType::Text, false),
            ("controller", "x" | "y" | "z") => (ValueType::Double, false),
            ("controller", "trigger" | "grip" | "thumbstick_x" | "thumbstick_y") => (ValueType::Double, true),
            ("controller", "rotation") => (ValueType::ListDouble, false),
            ("controller", "hand") => (ValueType::Text, true),
            ("controller", "tracked" | "thumbstick_click" | "primary" | "secondary") => (ValueType::Bool, true),
            ("controller", "claimed") => (ValueType::ListText, false),
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly,
            animated: matches!(key, "x" | "y" | "z" | "rotation"),
        })
    }

    fn has_method(&self, _class: &str, _method: &str) -> bool {
        false
    }
}

const ROOT: u64 = COMPOSITOR_ID_BASE | 1;
const HEAD: u64 = 10;
const OTHER_CAMERA: u64 = 11;
const PLAIN: u64 = 12;
const HAND: u64 = 13;
const NEW_HAND: u64 = 14;

fn id(raw: u64) -> NodeId {
    NodeId::new(raw)
}

fn named(scene: &mut Scene, raw: u64, class: &str, name: &str) {
    let mut spec = NodeSpec::new(Rect::new(0, 0, 10, 10));
    spec.parent = Some(id(ROOT));
    scene.insert_as(Actor::Shell, id(raw), spec).unwrap();
    scene.set_class(id(raw), Some(class.into())).unwrap();
    scene
        .set_value(id(raw), "name", TreeValue::Text(name.to_owned()))
        .unwrap();
}

/// A scene with the camera named `head`, another camera, and a plain node named `head`.
fn scene() -> Scene {
    let mut scene = Scene::new();
    scene
        .insert(id(ROOT), NodeSpec::new(Rect::new(0, 0, 100, 100)))
        .unwrap();
    scene.set_class(id(ROOT), Some("node".into())).unwrap();
    scene.set_root(Some(id(ROOT))).unwrap();
    named(&mut scene, HEAD, "camera", "head");
    named(&mut scene, OTHER_CAMERA, "camera", "other");
    named(&mut scene, PLAIN, "node", "head");
    named(&mut scene, HAND, "controller", "");
    scene
}

fn set(node: u64, key: &str, value: TreeValue) -> TreeOp {
    TreeOp::Set {
        node: id(node),
        key: key.into(),
        value,
        timing: None,
    }
}

fn pose() -> Vec<TreeOp> {
    vec![
        set(HEAD, "x", TreeValue::Double(1500.0)),
        set(HEAD, "y", TreeValue::Double(-1600.0)),
        set(HEAD, "z", TreeValue::Double(-250.0)),
        set(HEAD, "rotation", TreeValue::ListDouble(vec![0.1, -0.2, 0.0])),
    ]
}

fn as_observer(ops: Vec<TreeOp>) -> TreeTransaction {
    TreeTransaction::new(ops).from_observer()
}

fn refusal(scene: &mut Scene, transaction: TreeTransaction) -> (i32, Code) {
    let before = tree_hash(scene);
    let values_before: Vec<_> = scene.values(id(HEAD)).map(|(k, v)| (k.to_owned(), v.clone())).collect();
    let refused = validate_and_apply(scene, &Table, Actor::Shell, transaction).expect_err("refused");
    assert_eq!(tree_hash(scene), before, "a refused transaction changes nothing");
    let values_after: Vec<_> = scene.values(id(HEAD)).map(|(k, v)| (k.to_owned(), v.clone())).collect();
    assert_eq!(values_before, values_after, "no value of the head camera moved");
    (refused.failed_op, refused.code)
}

#[test]
fn an_observer_writes_the_pose_of_the_camera_named_head() {
    let mut scene = scene();
    let applied = validate_and_apply(&mut scene, &Table, Actor::Shell, as_observer(pose())).expect("applied");
    let keys: Vec<&str> = applied
        .changes
        .iter()
        .map(|change| match change {
            TreeChange::Set { key, .. } => key.as_str(),
            other => panic!("a set only: {other:?}"),
        })
        .collect();
    assert_eq!(keys, ["x", "y", "z", "rotation"]);
    assert!(scene.values(id(HEAD)).count() > 0);
}

#[test]
fn an_observer_writes_the_read_only_layer_budget_of_the_head_camera() {
    let mut scene = scene();
    let write = as_observer(vec![set(HEAD, "layer_budget", TreeValue::Uint(16))]);
    validate_and_apply(&mut scene, &Table, Actor::Shell, write).expect("applied");
}

#[test]
fn the_holder_still_cannot_write_the_layer_budget() {
    let mut scene = scene();
    let write = TreeTransaction::new(vec![set(HEAD, "layer_budget", TreeValue::Uint(16))]);
    let (failed, code) = refusal(&mut scene, write);
    assert_eq!((failed, code), (0, Code::ReadOnly));
}

#[test]
fn an_observer_may_not_write_a_camera_that_is_not_named_head() {
    let mut scene = scene();
    let write = as_observer(vec![set(OTHER_CAMERA, "x", TreeValue::Double(1.0))]);
    assert_eq!(refusal(&mut scene, write), (0, Code::NotHolder));
    let write = as_observer(vec![set(OTHER_CAMERA, "layer_budget", TreeValue::Uint(4))]);
    assert_eq!(refusal(&mut scene, write), (0, Code::NotHolder));
}

#[test]
fn an_observer_may_not_write_a_node_that_is_not_a_camera_even_when_it_is_named_head() {
    let mut scene = scene();
    let write = as_observer(vec![set(PLAIN, "x", TreeValue::Double(1.0))]);
    assert_eq!(refusal(&mut scene, write), (0, Code::NotHolder));
}

#[test]
fn an_observer_may_not_write_anything_else_of_the_head_camera() {
    let mut scene = scene();
    for (key, value) in [
        ("opacity", TreeValue::Double(0.5)),
        ("name", TreeValue::Text("gone".into())),
    ] {
        let write = as_observer(vec![set(HEAD, key, value)]);
        assert_eq!(refusal(&mut scene, write), (0, Code::NotHolder), "{key}");
    }
}

#[test]
fn an_observer_write_that_is_refused_refuses_the_whole_transaction() {
    let mut scene = scene();
    let mut ops = pose();
    ops.push(set(HEAD, "opacity", TreeValue::Double(0.5)));
    assert_eq!(refusal(&mut scene, as_observer(ops)), (4, Code::NotHolder));
}

#[test]
fn a_pose_with_a_number_that_is_not_finite_is_refused() {
    let mut scene = scene();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let write = as_observer(vec![set(HEAD, "x", TreeValue::Double(bad))]);
        assert_eq!(refusal(&mut scene, write), (0, Code::NotFinite), "x {bad}");
        let write = as_observer(vec![set(HEAD, "rotation", TreeValue::ListDouble(vec![0.0, bad, 0.0]))]);
        assert_eq!(refusal(&mut scene, write), (0, Code::NotFinite), "rotation {bad}");
    }
}

#[test]
fn a_pose_in_the_middle_of_a_batch_that_is_not_finite_leaves_the_earlier_sets_unapplied() {
    let mut scene = scene();
    let mut ops = pose();
    ops[2] = set(HEAD, "z", TreeValue::Double(f64::NAN));
    assert_eq!(refusal(&mut scene, as_observer(ops)), (2, Code::NotFinite));
}

#[test]
fn a_rotation_that_is_not_three_angles_is_refused() {
    let mut scene = scene();
    let write = as_observer(vec![set(HEAD, "rotation", TreeValue::ListDouble(vec![0.0, 0.0]))]);
    assert_eq!(refusal(&mut scene, write), (0, Code::BadValue));
}

#[test]
fn a_layer_budget_of_the_wrong_kind_is_refused() {
    let mut scene = scene();
    let write = as_observer(vec![set(HEAD, "layer_budget", TreeValue::Double(16.0))]);
    assert_eq!(refusal(&mut scene, write), (0, Code::WrongType));
}

#[test]
fn an_observer_write_to_a_node_that_is_gone_is_refused() {
    let mut scene = scene();
    let write = as_observer(vec![set(999, "x", TreeValue::Double(1.0))]);
    assert_eq!(refusal(&mut scene, write), (0, Code::UnknownNode));
}

#[test]
fn an_observer_may_not_animate_the_pose() {
    // A timed set is the shell's: the observer states a measurement, and the compositor does not tween it.
    let mut scene = scene();
    let timed = TreeOp::Set {
        node: id(HEAD),
        key: "x".into(),
        value: TreeValue::Double(1.0),
        timing: Some(gnoblin_render::transaction::Timing::new(1.0, vec![0.0, 1.0])),
    };
    assert_eq!(refusal(&mut scene, as_observer(vec![timed])), (0, Code::NotHolder));
}

fn make_hand(node: u64) -> TreeOp {
    TreeOp::New {
        class: "controller".into(),
        id: id(node),
    }
}

fn hand_values(node: u64) -> Vec<TreeOp> {
    vec![
        set(node, "hand", TreeValue::Text("right".into())),
        set(node, "tracked", TreeValue::Bool(true)),
        set(node, "x", TreeValue::Double(0.25)),
        set(node, "y", TreeValue::Double(1.1)),
        set(node, "z", TreeValue::Double(-0.4)),
        set(node, "rotation", TreeValue::ListDouble(vec![0.1, 0.2, 0.3])),
        set(node, "trigger", TreeValue::Double(0.5)),
        set(node, "grip", TreeValue::Double(1.0)),
        set(node, "thumbstick_x", TreeValue::Double(-1.0)),
        set(node, "thumbstick_y", TreeValue::Double(0.0)),
        set(node, "thumbstick_click", TreeValue::Bool(true)),
        set(node, "primary", TreeValue::Bool(true)),
        set(node, "secondary", TreeValue::Bool(false)),
    ]
}

#[test]
fn an_observer_makes_a_controller_and_writes_every_value_of_it_in_one_transaction() {
    let mut scene = scene();
    let mut ops = vec![make_hand(NEW_HAND)];
    ops.extend(hand_values(NEW_HAND));
    let applied = validate_and_apply(&mut scene, &Table, Actor::Shell, as_observer(ops)).expect("applied");
    assert!(matches!(applied.changes.first(), Some(TreeChange::Created { class, .. }) if class == "controller"));
    assert_eq!(
        scene.value(id(NEW_HAND), "grip"),
        Some(&TreeValue::Double(1.0)),
        "the value is in the scene"
    );
}

#[test]
fn an_observer_writes_the_values_of_a_controller_it_made_in_a_later_transaction() {
    let mut scene = scene();
    validate_and_apply(&mut scene, &Table, Actor::Shell, as_observer(hand_values(HAND))).expect("applied");
    assert_eq!(scene.value(id(HAND), "trigger"), Some(&TreeValue::Double(0.5)));
}

#[test]
fn an_observer_may_not_write_the_claimed_list_of_a_controller() {
    let mut scene = scene();
    let claim = set(HAND, "claimed", TreeValue::ListText(vec!["grip".into()]));
    assert_eq!(refusal(&mut scene, as_observer(vec![claim])), (0, Code::NotHolder));
    // Not even in the transaction that makes the node.
    let ops = vec![
        make_hand(NEW_HAND),
        set(NEW_HAND, "claimed", TreeValue::ListText(vec!["grip".into()])),
    ];
    assert_eq!(refusal(&mut scene, as_observer(ops)), (1, Code::NotHolder));
}

#[test]
fn the_holder_writes_the_claimed_list_and_nothing_else_of_a_controller() {
    let mut scene = scene();
    let claim = TreeTransaction::new(vec![set(HAND, "claimed", TreeValue::ListText(vec!["grip".into()]))]);
    validate_and_apply(&mut scene, &Table, Actor::Shell, claim).expect("the shell claims");
    assert_eq!(
        scene.value(id(HAND), "claimed"),
        Some(&TreeValue::ListText(vec!["grip".into()]))
    );
    for (key, value) in [("grip", TreeValue::Double(1.0)), ("tracked", TreeValue::Bool(true))] {
        let write = TreeTransaction::new(vec![set(HAND, key, value)]);
        assert_eq!(refusal(&mut scene, write), (0, Code::ReadOnly), "{key}");
    }
}

#[test]
fn an_observer_may_not_write_controller_values_to_a_node_that_is_not_a_controller() {
    let mut scene = scene();
    for node in [HEAD, PLAIN, OTHER_CAMERA] {
        let write = as_observer(vec![set(node, "trigger", TreeValue::Double(1.0))]);
        assert_eq!(refusal(&mut scene, write), (0, Code::NotHolder), "node {node}");
    }
}

#[test]
fn an_observer_may_not_animate_a_controller() {
    let mut scene = scene();
    let timed = TreeOp::Set {
        node: id(HAND),
        key: "x".into(),
        value: TreeValue::Double(1.0),
        timing: Some(gnoblin_render::transaction::Timing::new(1.0, vec![0.0, 1.0])),
    };
    assert_eq!(refusal(&mut scene, as_observer(vec![timed])), (0, Code::NotHolder));
}

#[test]
fn an_observer_may_not_write_anything_else_of_a_controller() {
    let mut scene = scene();
    let write = as_observer(vec![set(HAND, "opacity", TreeValue::Double(0.5))]);
    assert_eq!(refusal(&mut scene, write), (0, Code::NotHolder));
}
