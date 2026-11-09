//! Each value kind of the transaction layer (design/node-core.md §2.2 and §4.4): a value of the
//! wrong kind is `wrong-type`, a text or list over its cap is `too-large`, a non-finite number
//! is `not-finite`, a readonly property is `read-only`, and a node reference names a live node
//! whose class is the declared class or a subclass of it, or is nil.

use gnoblin_render::scene_ids::{Actor, COMPOSITOR_ID_BASE};
use gnoblin_render::transaction::{
    ClassInfo, Code, MAX_LIST, MAX_TEXT_BYTES, PropertyInfo, TreeOp, TreeRefusal, TreeTransaction, TreeValue,
    ValueType, validate_and_apply,
};
use gnoblin_render::{NodeSpec, Scene};
use gnoblin_types::{NodeId, Rect};

/// One property of each kind. `Panel` derives from `Window`, so a `Window` reference accepts it.
struct Kinds;

impl ClassInfo for Kinds {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "Window" | "Panel")
    }

    fn is_a(&self, class: &str, ancestor: &str) -> bool {
        class == ancestor || (class == "Panel" && ancestor == "Window")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        if !self.has_class(class) {
            return None;
        }
        let (kind, readonly) = match key {
            "label" => (ValueType::Text, false),
            "layer" => (ValueType::Int, false),
            "count" => (ValueType::Uint, false),
            "ratio" => (ValueType::Double, false),
            "visible" => (ValueType::Bool, false),
            "parent_ref" => (ValueType::Node("Window".into()), false),
            "axes" => (ValueType::ListDouble, false),
            "id" => (ValueType::Uint, true),
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly,
            animated: false,
        })
    }

    fn has_method(&self, _class: &str, _method: &str) -> bool {
        false
    }
}

const ROOT: u64 = COMPOSITOR_ID_BASE | 1;

/// A scene whose compositor-owned root is the tree's root node, of class `Window`.
fn scene() -> Scene {
    let mut scene = Scene::new();
    let root = NodeId::new(ROOT);
    scene.insert(root, NodeSpec::new(Rect::new(0, 0, 1920, 1080))).unwrap();
    scene.set_class(root, Some("Window".into())).unwrap();
    scene.set_root(Some(root)).unwrap();
    scene
}

/// A shell node of `class` under the root.
fn node(scene: &mut Scene, raw: u64, class: &str) -> NodeId {
    let id = NodeId::new(raw);
    let mut spec = NodeSpec::new(Rect::new(0, 0, 100, 100));
    spec.parent = Some(NodeId::new(ROOT));
    scene.insert_as(Actor::Shell, id, spec).unwrap();
    scene.set_class(id, Some(class.into())).unwrap();
    id
}

fn set(node: NodeId, key: &str, value: TreeValue) -> TreeOp {
    TreeOp::Set {
        node,
        key: key.into(),
        value,
        timing: None,
    }
}

/// Apply one `set` as the shell. `None` means it applied; `Some` is its refusal.
fn apply(scene: &mut Scene, op: TreeOp) -> Option<TreeRefusal> {
    validate_and_apply(scene, &Kinds, Actor::Shell, &TreeTransaction::new(vec![op])).err()
}

fn assert_refused(refusal: Option<TreeRefusal>, code: Code) {
    let refusal = refusal.expect("the value should be refused");
    assert_eq!(refusal.failed_op, 0, "failed_op");
    assert_eq!(refusal.code, code, "code");
}

#[test]
fn text_at_the_cap_is_accepted() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(
        apply(
            &mut scene,
            set(win, "label", TreeValue::Text("x".repeat(MAX_TEXT_BYTES)))
        )
        .is_none()
    );
}

#[test]
fn text_over_the_cap_is_too_large() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    let over = "x".repeat(MAX_TEXT_BYTES + 1);
    assert_refused(
        apply(&mut scene, set(win, "label", TreeValue::Text(over))),
        Code::TooLarge,
    );
}

#[test]
fn text_refuses_a_number() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(&mut scene, set(win, "label", TreeValue::Double(1.0))),
        Code::WrongType,
    );
}

#[test]
fn int_accepts_a_negative_value() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(apply(&mut scene, set(win, "layer", TreeValue::Int(-3))).is_none());
}

#[test]
fn int_refuses_an_unsigned_value() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(&mut scene, set(win, "layer", TreeValue::Uint(3))),
        Code::WrongType,
    );
}

#[test]
fn uint_accepts_its_own_kind() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(apply(&mut scene, set(win, "count", TreeValue::Uint(7))).is_none());
}

#[test]
fn uint_refuses_a_signed_value() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(apply(&mut scene, set(win, "count", TreeValue::Int(7))), Code::WrongType);
}

#[test]
fn double_accepts_a_finite_number() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(apply(&mut scene, set(win, "ratio", TreeValue::Double(0.5))).is_none());
}

#[test]
fn double_refuses_nan_and_infinity_as_not_finite() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(&mut scene, set(win, "ratio", TreeValue::Double(f64::NAN))),
        Code::NotFinite,
    );
    assert_refused(
        apply(&mut scene, set(win, "ratio", TreeValue::Double(f64::INFINITY))),
        Code::NotFinite,
    );
}

#[test]
fn double_refuses_an_int() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(apply(&mut scene, set(win, "ratio", TreeValue::Int(1))), Code::WrongType);
}

#[test]
fn bool_accepts_its_own_kind() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(apply(&mut scene, set(win, "visible", TreeValue::Bool(true))).is_none());
}

#[test]
fn bool_refuses_a_uint() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(&mut scene, set(win, "visible", TreeValue::Uint(1))),
        Code::WrongType,
    );
}

#[test]
fn a_node_reference_may_be_nil() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(apply(&mut scene, set(win, "parent_ref", TreeValue::Node(None))).is_none());
}

#[test]
fn a_node_reference_accepts_its_class_and_a_subclass() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    let panel = node(&mut scene, 0x101, "Panel");
    assert!(apply(&mut scene, set(win, "parent_ref", TreeValue::Node(Some(win)))).is_none());
    assert!(apply(&mut scene, set(win, "parent_ref", TreeValue::Node(Some(panel)))).is_none());
}

#[test]
fn a_node_reference_refuses_an_unrelated_class_as_wrong_type() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    let other = node(&mut scene, 0x101, "Other");
    assert_refused(
        apply(&mut scene, set(win, "parent_ref", TreeValue::Node(Some(other)))),
        Code::WrongType,
    );
}

#[test]
fn a_node_reference_to_a_missing_node_is_unknown_node() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(
            &mut scene,
            set(win, "parent_ref", TreeValue::Node(Some(NodeId::new(0x9999)))),
        ),
        Code::UnknownNode,
    );
}

#[test]
fn list_double_at_the_cap_is_accepted() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(apply(&mut scene, set(win, "axes", TreeValue::ListDouble(vec![0.0; MAX_LIST]))).is_none());
}

#[test]
fn list_double_over_the_cap_is_too_large() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(
            &mut scene,
            set(win, "axes", TreeValue::ListDouble(vec![0.0; MAX_LIST + 1])),
        ),
        Code::TooLarge,
    );
}

#[test]
fn list_double_refuses_a_non_finite_item() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(&mut scene, set(win, "axes", TreeValue::ListDouble(vec![0.0, f64::NAN]))),
        Code::NotFinite,
    );
}

#[test]
fn list_double_refuses_a_text_value() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(
        apply(&mut scene, set(win, "axes", TreeValue::Text("0 0 0".into()))),
        Code::WrongType,
    );
}

#[test]
fn a_readonly_property_refuses_its_own_kind_as_read_only() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(apply(&mut scene, set(win, "id", TreeValue::Uint(1))), Code::ReadOnly);
}
