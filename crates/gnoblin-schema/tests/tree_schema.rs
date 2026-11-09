//! The tree classes and messages as stock schema (design/node-core.md, sections 2.3, 2.4, 4.1).
//!
//! The classes live in `schema/tree/`, one file each, namespace `tree`, and the messages in
//! `schema/tree/messages.toml`. These tests read the stock schema directory and pin the
//! surface the generators see.

use std::path::Path;

use gnoblin_schema::{Catalog, DefaultValue, ObjectType, Type};

fn stock() -> Catalog {
    Catalog::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("schema")).expect("the stock schema loads")
}

fn tree_objects(catalog: &Catalog) -> Vec<&ObjectType> {
    catalog
        .schemas()
        .iter()
        .filter(|schema| schema.namespace.as_str() == "tree")
        .flat_map(|schema| schema.objects.iter())
        .collect()
}

fn object<'a>(catalog: &'a Catalog, name: &str) -> &'a ObjectType {
    tree_objects(catalog)
        .into_iter()
        .find(|object| object.name == name)
        .unwrap_or_else(|| panic!("no tree object `{name}`"))
}

const CLASSES: &[&str] = &[
    "node",
    "surface",
    "window",
    "canvas",
    "view",
    "screen",
    "camera",
    "seat",
    "workspace",
    "lock",
];

#[test]
fn the_ten_classes_exist_in_the_tree_namespace() {
    let catalog = stock();
    for class in CLASSES {
        object(&catalog, class);
    }
}

#[test]
fn the_classes_derive_as_node_core_says() {
    let catalog = stock();
    let bases: &[(&str, Option<&str>)] = &[
        ("node", None),
        ("surface", Some("node")),
        ("window", Some("node")),
        ("canvas", Some("node")),
        ("view", Some("node")),
        ("screen", Some("view")),
        ("camera", Some("node")),
        ("seat", Some("node")),
        ("workspace", Some("node")),
        ("lock", Some("node")),
    ];
    for (name, base) in bases {
        assert_eq!(object(&catalog, name).base.as_deref(), *base, "the base of `{name}`");
    }
}

#[test]
fn node_carries_the_section_2_4_properties_with_their_defaults() {
    let catalog = stock();
    let node = object(&catalog, "node");
    let property = |name: &str| {
        node.properties
            .iter()
            .find(|property| property.name == name)
            .unwrap_or_else(|| panic!("node has no `{name}`"))
    };
    let defaults = [
        ("name", DefaultValue::Text(String::new())),
        ("x", DefaultValue::Double(0.0)),
        ("opacity", DefaultValue::Double(1.0)),
        ("visible", DefaultValue::Bool(true)),
        ("clip", DefaultValue::Bool(false)),
        ("input", DefaultValue::Bool(true)),
        ("secure", DefaultValue::Bool(false)),
    ];
    for (name, default) in defaults {
        assert_eq!(
            property(name).default.as_ref(),
            Some(&default),
            "the default of `{name}`"
        );
    }
    assert!(property("id").readonly, "id is readonly");
    assert!(property("parent").readonly, "parent is readonly");
    assert!(property("x").animated, "x is animated");
    assert!(property("opacity").animated, "opacity is animated");
    assert!(!property("visible").animated, "visible is not animated");
    assert!(!property("name").readonly, "name is settable");
}

#[test]
fn node_declares_the_rotation_and_scale_lists_as_animated_doubles() {
    let catalog = stock();
    let node = object(&catalog, "node");
    for name in ["rotation", "scale"] {
        let property = node
            .properties
            .iter()
            .find(|property| property.name == name)
            .unwrap_or_else(|| panic!("node has no `{name}`"));
        assert_eq!(
            property.ty,
            Type::parse("list<double>").unwrap(),
            "`{name}` is a list of doubles"
        );
        assert!(property.animated, "`{name}` is animated");
    }
}

#[test]
fn node_declares_the_destroyed_signal() {
    let catalog = stock();
    let node = object(&catalog, "node");
    assert!(node.signals.iter().any(|signal| signal.name == "destroyed"));
}

#[test]
fn a_window_inherits_every_node_property() {
    let catalog = stock();
    let node = object(&catalog, "node");
    let window = object(&catalog, "window");
    for property in &node.properties {
        assert!(
            window
                .properties
                .iter()
                .any(|candidate| candidate.name == property.name),
            "window has the node property `{}`",
            property.name
        );
    }
    for own in ["title", "focused", "fullscreen", "workspace"] {
        assert!(
            window.properties.iter().any(|property| property.name == own),
            "window declares `{own}`"
        );
    }
}

#[test]
fn the_tree_messages_carry_the_section_4_1_methods_and_signals() {
    let catalog = stock();
    let messages = object(&catalog, "tree");
    for name in ["attach", "transact", "call"] {
        assert!(
            messages.methods.iter().any(|method| method.name == name),
            "tree has the method `{name}`"
        );
    }
    for name in ["attached", "snapshot", "reply", "changes", "event", "answer"] {
        assert!(
            messages.signals.iter().any(|signal| signal.name == name),
            "tree has the signal `{name}`"
        );
    }
}

#[test]
fn the_op_object_has_every_section_4_1_field() {
    let catalog = stock();
    let op = object(&catalog, "op");
    let fields = [
        "kind",
        "node",
        "class",
        "id",
        "key",
        "value_kind",
        "value_bool",
        "value_int",
        "value_uint",
        "value_double",
        "value_text",
        "value_node",
        "value_list",
        "seconds",
        "samples",
        "parent",
        "index",
        "method",
        "args",
    ];
    for field in fields {
        assert!(
            op.properties.iter().any(|property| property.name == field),
            "op has `{field}`"
        );
    }
}

#[test]
fn no_tree_member_is_sensitive() {
    let catalog = stock();
    let text = gnoblin_schema::proto::sensitive_table(&catalog);
    assert!(
        !text.lines().any(|line| line.contains("\"tree\"")),
        "no tree member is in the sensitive table: the tree is owner-socket only"
    );
}
#[test]
fn the_owner_socket_only_tree_is_excluded_by_load_dir_except() {
    // The Lua build excludes `tree` with `load_dir_except` (design/node-core.md section 2.3). The
    // stock schema must still hold it, and the excluded catalog must hold nothing from it.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
    let full = stock();
    assert!(!tree_objects(&full).is_empty(), "the stock schema holds the tree");
    let public = Catalog::load_dir_except(&dir, &["tree"]).expect("the schema loads without the tree");
    assert!(
        tree_objects(&public).is_empty(),
        "the excluded catalog holds no tree object"
    );
    assert!(
        public
            .schemas()
            .iter()
            .all(|schema| schema.namespace.as_str() != "tree"),
        "the excluded catalog holds no tree namespace"
    );
}
