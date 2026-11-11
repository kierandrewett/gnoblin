//! A `Canvas`'s `content` is a paint list, and the tree refuses a value that does not decode (design/node-core.md, F42).

mod common;

use common::tree_hash;
use gnoblin_paint::{PaintList, PaintOp, Rgba8, encode};
use gnoblin_render::scene_ids::{Actor, COMPOSITOR_ID_BASE};
use gnoblin_render::transaction::{
    ClassInfo, Code, PropertyInfo, TreeChange, TreeOp, TreeTransaction, TreeValue, ValueType, validate_and_apply,
};
use gnoblin_render::{NodeSpec, Scene};
use gnoblin_types::{NodeId, Rect};

/// The schema: a `canvas` with `content`, and a `node` with a bytes property that is not a canvas's content.
struct Table;

impl ClassInfo for Table {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "canvas" | "node")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        let kind = match (class, key) {
            ("canvas", "content") | ("node", "blob") => ValueType::Bytes,
            ("canvas" | "node", "width" | "height") => ValueType::Double,
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly: false,
            animated: false,
        })
    }

    fn has_method(&self, class: &str, method: &str) -> bool {
        class == "canvas" && method == "measure"
    }
}

fn id(raw: u64) -> NodeId {
    NodeId::new(raw)
}

fn scene() -> Scene {
    let mut scene = Scene::new();
    let root = id(COMPOSITOR_ID_BASE | 1);
    scene
        .insert(root, NodeSpec::new(Rect::new(0, 0, 100, 100)))
        .expect("the root");
    scene.set_class(root, Some("node".into())).expect("the root's class");
    scene.set_root(Some(root)).expect("the root is named");
    scene
}

fn make(class: &str, raw: u64) -> Vec<TreeOp> {
    vec![
        TreeOp::New {
            class: class.into(),
            id: id(raw),
        },
        TreeOp::Insert {
            node: id(raw),
            parent: id(COMPOSITOR_ID_BASE | 1),
            index: None,
        },
    ]
}

fn set_bytes(node: u64, key: &str, bytes: Vec<u8>) -> TreeOp {
    TreeOp::Set {
        node: id(node),
        key: key.into(),
        value: TreeValue::Bytes(bytes),
        timing: None,
    }
}

fn a_list() -> Vec<u8> {
    encode(&PaintList {
        ops: vec![PaintOp::Rect {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 4.0,
            radius: 0.0,
            fill: Rgba8::new(1, 2, 3, 255),
            stroke: Rgba8::TRANSPARENT,
            stroke_width: 0.0,
        }],
    })
}

#[test]
fn a_paint_list_that_decodes_is_stored_and_published() {
    let mut scene = scene();
    let mut ops = make("canvas", 10);
    ops.push(set_bytes(10, "content", a_list()));
    let applied = validate_and_apply(&mut scene, &Table, Actor::Shell, TreeTransaction::new(ops)).expect("applies");
    assert!(applied.changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Bytes(bytes) } if *node == id(10) && key == "content" && *bytes == a_list()
    )));
    assert_eq!(scene.bytes(id(10), "content"), Some(a_list().as_slice()));
}

#[test]
fn empty_content_is_the_empty_list_and_is_accepted() {
    let mut scene = scene();
    let mut ops = make("canvas", 10);
    ops.push(set_bytes(10, "content", Vec::new()));
    validate_and_apply(&mut scene, &Table, Actor::Shell, TreeTransaction::new(ops)).expect("applies");
}

#[test]
fn f42_content_that_does_not_decode_is_bad_paint_and_the_canvas_keeps_its_content() {
    let mut scene = scene();
    let mut ops = make("canvas", 10);
    ops.push(set_bytes(10, "content", a_list()));
    validate_and_apply(&mut scene, &Table, Actor::Shell, TreeTransaction::new(ops)).expect("applies");
    let before = tree_hash(&scene);

    for junk in [b"not a paint list".to_vec(), a_list()[..9].to_vec(), {
        let mut bytes = a_list();
        bytes.push(0);
        bytes
    }] {
        let refusal = validate_and_apply(
            &mut scene,
            &Table,
            Actor::Shell,
            TreeTransaction::new(vec![set_bytes(10, "content", junk)]),
        )
        .expect_err("a value that does not decode is refused");
        assert_eq!(refusal.code, Code::BadPaint);
        assert_eq!(refusal.failed_op, 0);
        assert_eq!(refusal.code.as_str(), "bad-paint");
        assert_eq!(tree_hash(&scene), before, "the canvas keeps its last content");
        assert_eq!(scene.bytes(id(10), "content"), Some(a_list().as_slice()));
    }
}

#[test]
fn a_bad_paint_refuses_the_whole_transaction() {
    let mut scene = scene();
    let before = tree_hash(&scene);
    let mut ops = make("canvas", 10);
    ops.push(set_bytes(10, "content", vec![1, 2, 3]));
    let refusal = validate_and_apply(&mut scene, &Table, Actor::Shell, TreeTransaction::new(ops)).expect_err("refused");
    assert_eq!((refusal.failed_op, refusal.code), (2, Code::BadPaint));
    assert_eq!(tree_hash(&scene), before, "the canvas the transaction made is not kept");
}

#[test]
fn bytes_on_a_node_that_is_not_a_canvas_are_not_checked_as_a_paint_list() {
    let mut scene = scene();
    let mut ops = make("node", 10);
    ops.push(set_bytes(10, "blob", vec![1, 2, 3]));
    validate_and_apply(&mut scene, &Table, Actor::Shell, TreeTransaction::new(ops)).expect("any bytes are fine");
}

#[test]
fn writing_content_bumps_the_revision_and_damages_the_node() {
    let mut scene = scene();
    scene.set_output(gnoblin_types::OutputId::new(1), Rect::new(0, 0, 100, 100));
    let mut ops = make("canvas", 10);
    ops.push(TreeOp::Set {
        node: id(10),
        key: "width".into(),
        value: TreeValue::Double(20.0),
        timing: None,
    });
    ops.push(TreeOp::Set {
        node: id(10),
        key: "height".into(),
        value: TreeValue::Double(10.0),
        timing: None,
    });
    validate_and_apply(&mut scene, &Table, Actor::Shell, TreeTransaction::new(ops)).expect("applies");
    assert_eq!(scene.content_revision(id(10)), Some(0));
    scene.take_damage();
    scene.take_dirty();

    validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        TreeTransaction::new(vec![set_bytes(10, "content", a_list())]),
    )
    .expect("applies");
    assert_eq!(scene.content_revision(id(10)), Some(1));
    assert!(scene.take_dirty().contains(&id(10)), "a view of the canvas must redraw");
    let damage = scene.take_damage();
    assert_eq!(damage.outputs.len(), 1);
    assert!(
        damage.outputs[0]
            .rects
            .iter()
            .any(|rect| rect.intersection(Rect::new(0, 0, 20, 10)).is_some()),
        "the node's rectangle is damaged: {damage:?}"
    );

    // The same bytes again are a new revision: the painter cannot tell, and does not compare.
    validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        TreeTransaction::new(vec![set_bytes(10, "content", a_list())]),
    )
    .expect("applies");
    assert_eq!(scene.content_revision(id(10)), Some(2));
}

#[test]
fn measure_is_a_method_of_a_canvas_only() {
    let mut scene = scene();
    let mut ops = make("canvas", 10);
    ops.extend(make("node", 11));
    ops.push(TreeOp::TreeCall {
        node: id(10),
        method: "measure".into(),
        args: vec![0],
        text: gnoblin_secret::SecretText::new("sans\nHello".to_owned()),
        token: 7,
    });
    let applied = validate_and_apply(&mut scene, &Table, Actor::Shell, TreeTransaction::new(ops)).expect("applies");
    assert_eq!(applied.calls.len(), 1);
    assert_eq!(applied.calls[0].method, "measure");

    let refusal = validate_and_apply(
        &mut scene,
        &Table,
        Actor::Shell,
        TreeTransaction::new(vec![TreeOp::TreeCall {
            node: id(11),
            method: "measure".into(),
            args: vec![0],
            text: gnoblin_secret::SecretText::new(String::new()),
            token: 8,
        }]),
    )
    .expect_err("a plain node has no measure");
    assert_eq!(refusal.code, Code::UnknownMember);
}
