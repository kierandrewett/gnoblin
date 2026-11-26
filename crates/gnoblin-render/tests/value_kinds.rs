//! Each value kind of the transaction layer: a value of the
//! wrong kind is `wrong-type`, a text or list over its cap is `too-large`, a non-finite number
//! is `not-finite`, a readonly property is `read-only`, and a node reference names a live node
//! whose class is the declared class or a subclass of it, or is nil.

use gnoblin_render::scene_ids::{Actor, COMPOSITOR_ID_BASE};
use gnoblin_render::transaction::{
    ClassInfo, Code, MAX_EFFECTS, MAX_LIST, MAX_TEXT_BYTES, PropertyInfo, TreeOp, TreeRefusal, TreeTransaction,
    TreeValue, ValueType, validate_and_apply,
};
use gnoblin_render::{BlurParams, Effects, Pass, Shadow};
use gnoblin_render::{NodeSpec, Scene};
use gnoblin_types::{Color, NodeId, Rect, Vec3};

/// One property of each kind. `Panel` derives from `Window`, so a `Window` reference accepts it.
struct Kinds;

impl ClassInfo for Kinds {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "Window" | "Panel" | "View" | "surface")
    }

    fn is_a(&self, class: &str, ancestor: &str) -> bool {
        class == ancestor || (class == "Panel" && ancestor == "Window")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        if !self.has_class(class) {
            return None;
        }
        let (kind, readonly) = match key {
            "label" | "frame_policy" => (ValueType::Text, false),
            "layer" => (ValueType::Int, false),
            "count" => (ValueType::Uint, false),
            "ratio" | "frame_rate" => (ValueType::Double, false),
            "visible" | "placement_override" => (ValueType::Bool, false),
            "stack" => (ValueType::Int, false),
            "reserve" => (ValueType::ListDouble, false),
            "x" | "width" => {
                return Some(PropertyInfo {
                    kind: ValueType::Double,
                    readonly: false,
                    animated: true,
                });
            }
            "parent_ref" => (ValueType::Node("Window".into()), false),
            "source" | "camera" => (ValueType::Node("Window".into()), false),
            "axes" => (ValueType::ListDouble, false),
            "tags" => (ValueType::ListText, false),
            "effects" => (ValueType::Effects, false),
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
    validate_and_apply(scene, &Kinds, Actor::Shell, TreeTransaction::new(vec![op])).err()
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
fn a_view_sets_and_clears_its_source_and_camera_nodes() {
    let mut scene = scene();
    let view = node(&mut scene, 0x100, "View");
    let source = node(&mut scene, 0x101, "Window");
    let camera = node(&mut scene, 0x102, "Window");
    assert!(apply(&mut scene, set(view, "source", TreeValue::Node(Some(source)))).is_none());
    assert!(apply(&mut scene, set(view, "camera", TreeValue::Node(Some(camera)))).is_none());
    assert_eq!(scene.source(view), Some(source));
    assert_eq!(scene.camera(view), Some(camera));
    assert!(apply(&mut scene, set(view, "source", TreeValue::Node(None))).is_none());
    assert!(apply(&mut scene, set(view, "camera", TreeValue::Node(None))).is_none());
    assert_eq!(scene.source(view), None);
    assert_eq!(scene.camera(view), None);
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
fn effects_at_the_cap_are_accepted() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    let blur = Pass::Blur(BlurParams {
        radius: 4.0,
        passes: 2,
        ..BlurParams::default()
    });
    assert!(
        apply(
            &mut scene,
            set(win, "effects", TreeValue::Effects(vec![blur; MAX_EFFECTS]))
        )
        .is_none()
    );
}

#[test]
fn effects_over_the_cap_are_too_large() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    let blur = Pass::Blur(BlurParams {
        radius: 4.0,
        passes: 2,
        ..BlurParams::default()
    });
    assert_refused(
        apply(
            &mut scene,
            set(win, "effects", TreeValue::Effects(vec![blur; MAX_EFFECTS + 1])),
        ),
        Code::TooLarge,
    );
}

#[test]
fn an_accepted_effects_list_is_the_nodes_effect_chain() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    let blur = Pass::Blur(BlurParams {
        radius: 4.0,
        passes: 2,
        ..BlurParams::default()
    });
    let shadow = Pass::Shadow(Shadow {
        offset: Vec3::new(0.0, 4.0, 0.0),
        blur: 12.0,
        spread: 0.0,
        opacity: 1.0,
        color: Color::BLACK,
        frame: false,
        inset: false,
    });
    assert!(
        scene.effects(win).is_some_and(Effects::is_empty),
        "a new node has no effect"
    );
    assert!(
        apply(
            &mut scene,
            set(win, "effects", TreeValue::Effects(vec![blur.clone(), shadow.clone()]))
        )
        .is_none()
    );
    let chain = scene.effects(win).expect("the node exists");
    assert_eq!(chain.passes(), [blur, shadow], "the chain is the list, in order");
    assert_eq!(scene.effect_nodes().collect::<Vec<_>>(), vec![win]);
    assert!(
        scene.value(win, "effects").is_some(),
        "the snapshot still reads the list the shell set"
    );

    assert!(apply(&mut scene, set(win, "effects", TreeValue::Effects(Vec::new()))).is_none());
    assert!(
        scene.effects(win).is_some_and(Effects::is_empty),
        "setting the empty list clears the chain"
    );
    assert_eq!(scene.effect_nodes().count(), 0);
}

#[test]
fn a_refused_effects_list_leaves_the_chain_alone() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    let blur = Pass::Blur(BlurParams {
        radius: 4.0,
        passes: 2,
        ..BlurParams::default()
    });
    assert!(apply(&mut scene, set(win, "effects", TreeValue::Effects(vec![blur.clone()]))).is_none());
    let too_blurry = Pass::Blur(BlurParams {
        passes: 9,
        ..BlurParams::default()
    });
    assert_refused(
        apply(
            &mut scene,
            set(win, "effects", TreeValue::Effects(vec![blur.clone(), too_blurry])),
        ),
        Code::OutOfRange,
    );
    assert_eq!(
        scene.effects(win).expect("the node exists").passes(),
        [blur],
        "the chain the shell had is still the chain"
    );
}

#[test]
fn tags_at_the_cap_are_accepted_and_over_it_are_too_large() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert!(
        apply(
            &mut scene,
            set(win, "tags", TreeValue::ListText(vec!["t".into(); MAX_LIST]))
        )
        .is_none()
    );
    assert_refused(
        apply(
            &mut scene,
            set(win, "tags", TreeValue::ListText(vec!["t".into(); MAX_LIST + 1])),
        ),
        Code::TooLarge,
    );
}

#[test]
fn a_readonly_property_refuses_its_own_kind_as_read_only() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    assert_refused(apply(&mut scene, set(win, "id", TreeValue::Uint(1))), Code::ReadOnly);
}

/// A window's `frame_policy` is one of three names; any other text is `bad-value` and the window keeps the policy it had.
#[test]
fn frame_policy_takes_auto_hold_and_throttle_and_refuses_any_other_text() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    for policy in ["hold", "throttle", "auto"] {
        assert!(
            apply(&mut scene, set(win, "frame_policy", TreeValue::Text(policy.into()))).is_none(),
            "{policy}"
        );
        assert_eq!(scene.value(win, "frame_policy"), Some(&TreeValue::Text(policy.into())));
    }
    for policy in ["", "Hold", "pause", "throttle:30"] {
        assert_refused(
            apply(&mut scene, set(win, "frame_policy", TreeValue::Text(policy.into()))),
            Code::BadValue,
        );
    }
    assert_eq!(
        scene.value(win, "frame_policy"),
        Some(&TreeValue::Text("auto".into())),
        "a refused write leaves the policy as it was"
    );
}

/// A window's `frame_rate` is above 0 and at most 1000 hertz; anything else is `out-of-range`.
#[test]
fn frame_rate_takes_a_rate_above_zero_up_to_a_thousand_and_refuses_the_rest() {
    let mut scene = scene();
    let win = node(&mut scene, 0x100, "Window");
    for rate in [0.5, 30.0, 144.0, 1000.0] {
        assert!(
            apply(&mut scene, set(win, "frame_rate", TreeValue::Double(rate))).is_none(),
            "{rate}"
        );
    }
    for rate in [0.0, -1.0, 1000.5, 1e9] {
        assert_refused(
            apply(&mut scene, set(win, "frame_rate", TreeValue::Double(rate))),
            Code::OutOfRange,
        );
    }
    assert_refused(
        apply(&mut scene, set(win, "frame_rate", TreeValue::Double(f64::NAN))),
        Code::NotFinite,
    );
    assert_eq!(scene.value(win, "frame_rate"), Some(&TreeValue::Double(1000.0)));
}

/// A compositor-made `surface` node of `role`, as the compositor makes a popup, a subsurface or a layer surface.
fn compositor_surface(scene: &mut Scene, raw: u64, role: Option<&str>) -> NodeId {
    let id = NodeId::new(COMPOSITOR_ID_BASE | raw);
    scene.insert(id, NodeSpec::new(Rect::new(0, 0, 100, 100))).unwrap();
    scene.set_class(id, Some("surface".into())).unwrap();
    if let Some(role) = role {
        scene.set_text(id, "role", role.into()).unwrap();
    }
    id
}

/// A shell may place a popup, a subsurface or a layer surface, and the refusal for any other node is `not-owner`: a lock
/// surface, the cursor and a drag icon are the compositor's, and so is a node that names no role. Writing false is never
/// refused, and a refused write leaves the node as it was.
#[test]
fn placement_override_is_refused_on_a_surface_the_shell_may_not_place() {
    let mut scene = scene();
    for (raw, role) in [(0x200, "popup"), (0x201, "subsurface"), (0x202, "layer")] {
        let surface = compositor_surface(&mut scene, raw, Some(role));
        assert!(
            apply(&mut scene, set(surface, "placement_override", TreeValue::Bool(true))).is_none(),
            "{role}"
        );
        assert_eq!(
            scene.value(surface, "placement_override"),
            Some(&TreeValue::Bool(true)),
            "{role}"
        );
    }
    for (raw, role) in [
        (0x210, Some("lock")),
        (0x211, Some("cursor")),
        (0x212, Some("drag")),
        (0x213, None),
    ] {
        let surface = compositor_surface(&mut scene, raw, role);
        assert_refused(
            apply(&mut scene, set(surface, "placement_override", TreeValue::Bool(true))),
            Code::NotOwner,
        );
        assert_eq!(
            scene.value(surface, "placement_override"),
            None,
            "{role:?}: a refused write leaves no value"
        );
        assert!(
            apply(&mut scene, set(surface, "placement_override", TreeValue::Bool(false))).is_none(),
            "{role:?}: false asks for nothing"
        );
    }
}

/// `reserve` is empty or four whole non-negative insets; anything else is `bad-value`.
#[test]
fn reserve_takes_no_insets_or_four_whole_ones_and_refuses_the_rest() {
    let mut scene = scene();
    let layer = compositor_surface(&mut scene, 0x300, Some("layer"));
    for insets in [vec![], vec![0.0, 0.0, 0.0, 0.0], vec![32.0, 0.0, 0.0, 8.0]] {
        assert!(
            apply(&mut scene, set(layer, "reserve", TreeValue::ListDouble(insets.clone()))).is_none(),
            "{insets:?}"
        );
        assert_eq!(scene.value(layer, "reserve"), Some(&TreeValue::ListDouble(insets)));
    }
    for insets in [
        vec![1.0],
        vec![1.0, 2.0, 3.0],
        vec![1.0, 2.0, 3.0, 4.0, 5.0],
        vec![-1.0, 0.0, 0.0, 0.0],
        vec![1.5, 0.0, 0.0, 0.0],
        vec![f64::from(i32::MAX) + 1.0, 0.0, 0.0, 0.0],
    ] {
        assert_refused(
            apply(&mut scene, set(layer, "reserve", TreeValue::ListDouble(insets.clone()))),
            Code::BadValue,
        );
    }
    assert_eq!(
        scene.value(layer, "reserve"),
        Some(&TreeValue::ListDouble(vec![32.0, 0.0, 0.0, 8.0])),
        "a refused write leaves the insets as they were"
    );
}

/// A surface's place and size are read once a frame and acted on, so a timed write of them is `not-animated`, while the
/// same write at once applies.
#[test]
fn a_timed_write_of_a_surfaces_place_is_not_animated() {
    let mut scene = scene();
    let popup = compositor_surface(&mut scene, 0x400, Some("popup"));
    let timed = |node: NodeId, key: &str| TreeOp::Set {
        node,
        key: key.into(),
        value: TreeValue::Double(40.0),
        timing: Some(gnoblin_render::transaction::Timing {
            seconds: 1.0,
            delay: 0.0,
            group: None,
            samples: vec![0.0, 1.0],
        }),
    };
    for key in ["x", "width"] {
        assert_refused(apply(&mut scene, timed(popup, key)), Code::NotAnimated);
        assert!(
            apply(&mut scene, set(popup, key, TreeValue::Double(40.0))).is_none(),
            "{key} at once"
        );
    }
    let window = node(&mut scene, 0x401, "Window");
    assert!(
        apply(&mut scene, timed(window, "x")).is_none(),
        "a window's x still animates"
    );
}
