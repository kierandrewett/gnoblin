//! Transaction validation and apply (design/node-core.md §4 and §8). Every refusal
//! test asserts that the tree hash is unchanged: a refused transaction changes nothing.

mod common;

use common::tree_hash;
use gnoblin_animation::{PropertyValue, VisualProperty};
use gnoblin_render::scene_ids::{Actor, COMPOSITOR_ID_BASE, MAX_DEPTH, MAX_NODES};
use gnoblin_render::transaction::{
    Change, ClassInfo, Code, MAX_OPS, Op, PropertyInfo, Refusal, Timing, Transaction, Value, ValueType,
    validate_and_apply,
};
use gnoblin_render::{NodeSpec, Scene, SceneError};
use gnoblin_types::{NodeId, Rect};

/// The test schema: one class with every kind of member the transaction layer sees.
struct Table;

impl ClassInfo for Table {
    fn has_class(&self, class: &str) -> bool {
        class == "Window"
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        if class != "Window" {
            return None;
        }
        let (kind, readonly, animated) = match key {
            "opacity" | "offset_x" | "offset_y" | "offset_z" | "rotation_x" | "rotation_y" | "rotation_z"
            | "scale_x" | "scale_y" | "scale_z" => (ValueType::Double, false, true),
            "width" | "height" => (ValueType::Uint, false, false),
            "visible" | "clip" | "input" => (ValueType::Bool, false, false),
            "id" => (ValueType::Uint, true, false),
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly,
            animated,
        })
    }

    fn has_method(&self, class: &str, method: &str) -> bool {
        class == "Window" && method == "focus"
    }
}

const ROOT: u64 = COMPOSITOR_ID_BASE | 1;
const LINEAR: [f64; 3] = [0.0, 0.5, 1.0];

fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
    Rect::new(x, y, width, height)
}

fn id(raw: u64) -> NodeId {
    NodeId::new(raw)
}

/// A scene whose compositor-owned root is the tree's root node.
fn scene() -> Scene {
    let mut scene = Scene::new();
    let root = id(ROOT);
    scene.insert(root, NodeSpec::new(rect(0, 0, 1920, 1080))).unwrap();
    scene.set_class(root, Some("Window".into())).unwrap();
    scene.set_root(Some(root)).unwrap();
    scene
}

/// A shell node under `parent`, of class `Window`.
fn shell_node(scene: &mut Scene, raw: u64, parent: NodeId) -> NodeId {
    let node = id(raw);
    let mut spec = NodeSpec::new(rect(0, 0, 100, 100));
    spec.parent = Some(parent);
    scene.insert_as(Actor::Shell, node, spec).unwrap();
    scene.set_class(node, Some("Window".into())).unwrap();
    node
}

fn set_number(node: NodeId, key: &str, value: f64) -> Op {
    Op::Set {
        node,
        key: key.into(),
        value: Value::Double(value),
        timing: None,
    }
}

fn set_uint(node: NodeId, key: &str, value: u64) -> Op {
    Op::Set {
        node,
        key: key.into(),
        value: Value::Uint(value),
        timing: None,
    }
}

fn set_bool(node: NodeId, key: &str, value: bool) -> Op {
    Op::Set {
        node,
        key: key.into(),
        value: Value::Bool(value),
        timing: None,
    }
}

fn new_window(raw: u64) -> Op {
    Op::New {
        class: "Window".into(),
        id: id(raw),
    }
}

fn insert(node: NodeId, parent: NodeId) -> Op {
    Op::Insert {
        node,
        parent,
        index: None,
    }
}

/// Run `ops` as `actor`, expect a refusal, and check that the tree is unchanged.
fn refused(scene: &mut Scene, actor: Actor, ops: Vec<Op>) -> Refusal {
    let before = tree_hash(scene);
    let result = validate_and_apply(scene, &Table, actor, &Transaction::new(ops));
    assert_eq!(tree_hash(scene), before, "a refused transaction must change nothing");
    result.expect_err("the transaction should be refused")
}

fn assert_refused(refusal: &Refusal, failed_op: i32, code: Code) {
    assert_eq!(refusal.failed_op, failed_op, "failed_op");
    assert_eq!(refusal.code, code, "code");
}

// ---------------------------------------------------------------------------
// Whole-transaction atomicity and the op order
// ---------------------------------------------------------------------------

#[test]
fn f8_op_on_destroyed_id_refuses_whole_transaction() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(
        &mut scene,
        Actor::Shell,
        vec![
            set_number(window, "opacity", 0.5),
            Op::Destroy { node: window },
            set_number(window, "opacity", 0.25),
        ],
    );
    assert_refused(&refusal, 2, Code::UnknownNode);
}

#[test]
fn failed_op_is_the_first_failing_op() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(
        &mut scene,
        Actor::Shell,
        vec![
            set_number(window, "opacity", 0.5),
            set_number(window, "nope", 1.0),
            set_number(id(99), "opacity", 1.0),
        ],
    );
    assert_refused(&refusal, 1, Code::UnknownMember);
}

#[test]
fn op_sees_an_earlier_new_in_the_same_transaction() {
    let mut scene = scene();
    let root = id(ROOT);
    let applied = validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        &Transaction::new(vec![
            new_window(7),
            insert(id(7), root),
            set_number(id(7), "opacity", 0.5),
        ]),
    )
    .expect("the later ops see the new node");
    assert_eq!(scene.parent(id(7)), Some(root));
    assert_eq!(
        scene.property(id(7), VisualProperty::Opacity).unwrap(),
        PropertyValue::number(0.5)
    );
    assert!(matches!(applied.changes[0], Change::Created { node, .. } if node == id(7)));
    assert!(matches!(applied.changes[1], Change::Parented { node, parent, .. } if node == id(7) && parent == root));
}

#[test]
fn f39_second_transaction_sees_the_first_writes() {
    let mut scene = scene();
    let root = id(ROOT);
    validate_and_apply(&mut scene, &Table, Actor::Shell, &Transaction::new(vec![new_window(7)]))
        .expect("the first transaction creates the node");
    validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        &Transaction::new(vec![insert(id(7), root)]),
    )
    .expect("the second transaction sees the node");
    assert_eq!(scene.parent(id(7)), Some(root));
}

#[test]
fn a_valid_transaction_applies_every_op() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        &Transaction::new(vec![set_number(window, "opacity", 0.5), set_bool(window, "clip", true)]),
    )
    .expect("both ops are valid");
    assert_eq!(scene.clip(window), Some(true));
    assert_eq!(
        scene.property(window, VisualProperty::Opacity).unwrap(),
        PropertyValue::number(0.5)
    );
}

#[test]
fn destroy_reports_the_removed_subtree() {
    let mut scene = scene();
    let root = id(ROOT);
    let parent = shell_node(&mut scene, 1, root);
    let child = shell_node(&mut scene, 2, parent);
    let applied = validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        &Transaction::new(vec![Op::Destroy { node: parent }]),
    )
    .expect("the shell destroys its own subtree");
    assert!(!scene.contains(child));
    assert!(
        matches!(&applied.changes[0], Change::Destroyed { node, removed } if *node == parent && removed.len() == 2)
    );
}

// ---------------------------------------------------------------------------
// §4.4 refusal codes, one named case each
// ---------------------------------------------------------------------------

#[test]
fn f10_reparent_into_own_subtree_is_cycle() {
    let mut scene = scene();
    let root = id(ROOT);
    let parent = shell_node(&mut scene, 1, root);
    let child = shell_node(&mut scene, 2, parent);
    let refusal = refused(
        &mut scene,
        Actor::Shell,
        vec![set_number(parent, "opacity", 0.5), insert(parent, child)],
    );
    assert_refused(&refusal, 1, Code::Cycle);
}

#[test]
fn f11_destroying_root_is_root_immutable() {
    let mut scene = scene();
    let root = id(ROOT);
    let refusal = refused(&mut scene, Actor::Compositor, vec![Op::Destroy { node: root }]);
    assert_refused(&refusal, 0, Code::RootImmutable);
}

#[test]
fn f11_moving_root_is_root_immutable() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(&mut scene, Actor::Compositor, vec![insert(root, window)]);
    assert_refused(&refusal, 0, Code::RootImmutable);
}

#[test]
fn f12_shell_destroying_a_compositor_node_is_not_owner() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = id(COMPOSITOR_ID_BASE | 2);
    let mut spec = NodeSpec::new(rect(0, 0, 100, 100));
    spec.parent = Some(root);
    scene.insert_as(Actor::Compositor, window, spec).unwrap();
    let refusal = refused(&mut scene, Actor::Shell, vec![Op::Destroy { node: window }]);
    assert_refused(&refusal, 0, Code::NotOwner);
}

#[test]
fn f12_shell_moving_a_compositor_parented_node_is_not_owner() {
    let mut scene = scene();
    let root = id(ROOT);
    let other = shell_node(&mut scene, 1, root);
    let window = id(COMPOSITOR_ID_BASE | 2);
    let mut spec = NodeSpec::new(rect(0, 0, 100, 100));
    spec.parent = Some(root);
    scene.insert_as(Actor::Compositor, window, spec).unwrap();
    let refusal = refused(&mut scene, Actor::Shell, vec![insert(window, other)]);
    assert_refused(&refusal, 0, Code::NotOwner);
}

#[test]
fn shell_writes_to_a_compositor_window_apply() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = id(COMPOSITOR_ID_BASE | 2);
    let mut spec = NodeSpec::new(rect(0, 0, 100, 100));
    spec.parent = Some(root);
    scene.insert_as(Actor::Compositor, window, spec).unwrap();
    scene.set_class(window, Some("Window".into())).unwrap();
    validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        &Transaction::new(vec![
            set_number(window, "opacity", 0.25),
            set_uint(window, "width", 300),
        ]),
    )
    .expect("property writes are governed by readonly, not by ownership");
    assert_eq!(
        scene.property(window, VisualProperty::Opacity).unwrap(),
        PropertyValue::number(0.25)
    );
    assert_eq!(scene.bounds(window), Some(rect(0, 0, 300, 100)));
}

#[test]
fn f13_coordinate_beyond_2_pow_20_is_out_of_bounds() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(&mut scene, Actor::Shell, vec![set_uint(window, "width", 2_000_000)]);
    assert_refused(&refusal, 0, Code::OutOfBounds);
}

#[test]
fn f14_nan_value_is_not_finite() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(&mut scene, Actor::Shell, vec![set_number(window, "opacity", f64::NAN)]);
    assert_refused(&refusal, 0, Code::NotFinite);
}

#[test]
fn f15_new_with_a_live_id_is_duplicate_node() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(&mut scene, Actor::Shell, vec![new_window(window.get())]);
    assert_refused(&refusal, 0, Code::DuplicateNode);
}

#[test]
fn f16_more_than_4096_ops_is_too_large() {
    assert_eq!(MAX_OPS, 4096);
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let ops = (0..=MAX_OPS).map(|_| set_number(window, "opacity", 0.5)).collect();
    let refusal = refused(&mut scene, Actor::Shell, ops);
    assert_refused(&refusal, -1, Code::TooLarge);
}

#[test]
fn f16_exactly_4096_ops_are_accepted() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let ops = (0..MAX_OPS).map(|_| set_number(window, "opacity", 0.5)).collect();
    validate_and_apply(&mut scene, &Table, Actor::Shell, &Transaction::new(ops)).expect("the limit is inclusive");
}

#[test]
fn f18_new_past_65536_nodes_is_too_many_nodes() {
    let mut scene = scene();
    let root = id(ROOT);
    let mut raw = 1;
    while raw < MAX_NODES as u64 {
        shell_node(&mut scene, raw, root);
        raw += 1;
    }
    let refusal = refused(&mut scene, Actor::Shell, vec![new_window(raw)]);
    assert_refused(&refusal, 0, Code::TooManyNodes);
}

#[test]
fn f38_insert_past_depth_256_is_too_deep() {
    let mut scene = scene();
    let mut deepest = id(ROOT);
    let mut raw = 1;
    loop {
        let node = id(raw);
        let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
        spec.parent = Some(deepest);
        match scene.insert_as(Actor::Shell, node, spec) {
            Ok(()) => deepest = node,
            Err(SceneError::TooDeep) => break,
            Err(error) => panic!("unexpected error building the chain: {error}"),
        }
        raw += 1;
    }
    let refusal = refused(
        &mut scene,
        Actor::Shell,
        vec![new_window(raw), insert(id(raw), deepest)],
    );
    assert_refused(&refusal, 1, Code::TooDeep);
    assert_eq!(MAX_DEPTH, 256);
}

#[test]
fn f26_wrong_value_kind_is_wrong_type() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(&mut scene, Actor::Shell, vec![set_bool(window, "opacity", true)]);
    assert_refused(&refusal, 0, Code::WrongType);
}

#[test]
fn f27_unknown_class_is_unknown_class() {
    let mut scene = scene();
    let refusal = refused(
        &mut scene,
        Actor::Shell,
        vec![Op::New {
            class: "Nope".into(),
            id: id(7),
        }],
    );
    assert_refused(&refusal, 0, Code::UnknownClass);
}

#[test]
fn f27_unknown_property_is_unknown_member() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(&mut scene, Actor::Shell, vec![set_number(window, "nope", 1.0)]);
    assert_refused(&refusal, 0, Code::UnknownMember);
}

#[test]
fn f27_unknown_method_is_unknown_member() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(
        &mut scene,
        Actor::Shell,
        vec![Op::Call {
            node: window,
            method: "explode".into(),
            args: Vec::new(),
        }],
    );
    assert_refused(&refusal, 0, Code::UnknownMember);
}

#[test]
fn f31_seconds_on_a_non_animated_property_is_not_animated() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let op = Op::Set {
        node: window,
        key: "width".into(),
        value: Value::Uint(300),
        timing: Some(Timing {
            seconds: 0.5,
            samples: LINEAR.to_vec(),
        }),
    };
    let refusal = refused(&mut scene, Actor::Shell, vec![op]);
    assert_refused(&refusal, 0, Code::NotAnimated);
}

#[test]
fn f32_bad_samples_are_bad_curve() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    for samples in [
        Vec::new(),
        vec![0.0, 1.0, 0.5],
        vec![0.5, 0.0, 1.0],
        vec![0.0, 1.5],
        vec![-0.5, 1.0],
    ] {
        let op = Op::Set {
            node: window,
            key: "opacity".into(),
            value: Value::Double(1.0),
            timing: Some(Timing {
                seconds: 0.5,
                samples: samples.clone(),
            }),
        };
        let refusal = refused(&mut scene, Actor::Shell, vec![op]);
        assert_eq!(refusal.code, Code::BadCurve, "samples {samples:?}");
    }
}

#[test]
fn f33_seconds_out_of_range_is_out_of_range() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    for seconds in [-1.0, 3600.5] {
        let op = Op::Set {
            node: window,
            key: "opacity".into(),
            value: Value::Double(1.0),
            timing: Some(Timing {
                seconds,
                samples: LINEAR.to_vec(),
            }),
        };
        let refusal = refused(&mut scene, Actor::Shell, vec![op]);
        assert_eq!(refusal.code, Code::OutOfRange, "seconds {seconds}");
    }
}

#[test]
fn f41_set_on_a_read_only_property_is_read_only() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let refusal = refused(&mut scene, Actor::Shell, vec![set_uint(window, "id", 5)]);
    assert_refused(&refusal, 0, Code::ReadOnly);
}

#[test]
fn animated_set_is_validated_and_stored_in_applied() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let op = Op::Set {
        node: window,
        key: "opacity".into(),
        value: Value::Double(0.0),
        timing: Some(Timing {
            seconds: 0.5,
            samples: LINEAR.to_vec(),
        }),
    };
    let applied = validate_and_apply(&mut scene, &Table, Actor::Shell, &Transaction::new(vec![op]))
        .expect("a valid animated set applies");
    assert_eq!(applied.animations.len(), 1);
    assert_eq!(applied.animations[0].node, window);
    assert_eq!(applied.animations[0].seconds, 0.5);
    // The compositor plays the track (slice 7); the scene value is not written here.
    assert_eq!(
        scene.property(window, VisualProperty::Opacity).unwrap(),
        PropertyValue::number(1.0)
    );
}

#[test]
fn call_is_validated_and_recorded_in_applied() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    let applied = validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        &Transaction::new(vec![Op::Call {
            node: window,
            method: "focus".into(),
            args: vec![1, 2],
        }]),
    )
    .expect("a known method is accepted");
    assert_eq!(applied.calls.len(), 1);
    assert_eq!(applied.calls[0].method, "focus");
}

#[test]
fn set_width_and_height_apply_through_the_bounds_setter() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        &Transaction::new(vec![set_uint(window, "width", 300), set_uint(window, "height", 200)]),
    )
    .expect("width and height are settable");
    assert_eq!(scene.bounds(window), Some(rect(0, 0, 300, 200)));
}

#[test]
fn f13_bounds_setter_refuses_beyond_2_pow_20() {
    let mut scene = scene();
    let root = id(ROOT);
    let window = shell_node(&mut scene, 1, root);
    assert!(matches!(
        scene.set_bounds(window, rect(0, 0, 2_000_000, 10)),
        Err(SceneError::OutOfBounds { .. })
    ));
    assert_eq!(scene.bounds(window), Some(rect(0, 0, 100, 100)));
}

// ---------------------------------------------------------------------------
// Codes whose rule lives outside the transaction layer (see the module docs)
// ---------------------------------------------------------------------------

#[test]
fn every_refusal_code_has_the_table_4_4_spelling() {
    let table = [
        (Code::UnknownNode, "unknown-node"),
        (Code::NotOwner, "not-owner"),
        (Code::RootImmutable, "root-immutable"),
        (Code::Cycle, "cycle"),
        (Code::UnknownClass, "unknown-class"),
        (Code::UnknownMember, "unknown-member"),
        (Code::WrongType, "wrong-type"),
        (Code::ReadOnly, "read-only"),
        (Code::NotAnimated, "not-animated"),
        (Code::NotFinite, "not-finite"),
        (Code::OutOfBounds, "out-of-bounds"),
        (Code::DuplicateNode, "duplicate-node"),
        (Code::BadCurve, "bad-curve"),
        (Code::OutOfRange, "out-of-range"),
        (Code::TooLarge, "too-large"),
        (Code::TooManyNodes, "too-many-nodes"),
        (Code::TooDeep, "too-deep"),
        (Code::StaleBase, "stale-base"),
        (Code::InvalidSerial, "invalid-serial"),
        (Code::NotPrivileged, "not-privileged"),
        (Code::ShellAlreadyAttached, "shell-already-attached"),
        (Code::BadPaint, "bad-paint"),
        (Code::BadValue, "bad-value"),
    ];
    for (code, spelling) in table {
        assert_eq!(code.as_str(), spelling);
    }
}
