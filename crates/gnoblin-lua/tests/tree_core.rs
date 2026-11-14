//! The Lua `core` table over the shell's tree core (design/node-core.md, slice 10): the calls
//! `node.lua` makes, with the refusals the schema and the transaction builder make.

use gnoblin_lua::tree_core::{ClassSpec, PropSpec, Shared, TreeCore, TreeValueKind, install};
use gnoblin_tree_client::tree_mirror::{
    MirrorChange, MirrorChangeKind, MirrorValue, NIL, SnapshotNode, TreeMirror, TxOp,
};
use mlua::Lua;

fn snap(id: u64, parent: u64, index: i64, class: &str) -> SnapshotNode {
    SnapshotNode {
        id,
        parent,
        index,
        class: class.to_owned(),
        title: String::new(),
        app_id: String::new(),
        props: Vec::new(),
    }
}

fn change(kind: MirrorChangeKind, node: u64) -> MirrorChange {
    MirrorChange {
        kind,
        node,
        class: String::new(),
        parent: NIL,
        index: 0,
        key: String::new(),
        value: None,
        removed: Vec::new(),
    }
}

fn prop(name: &str, kind: TreeValueKind, readonly: bool, default: Option<MirrorValue>) -> PropSpec {
    PropSpec {
        name: name.to_owned(),
        kind,
        readonly,
        default,
    }
}

fn schema() -> Vec<ClassSpec> {
    vec![
        ClassSpec {
            name: "Node".to_owned(),
            props: vec![
                prop("x", TreeValueKind::Double, false, Some(MirrorValue::Double(0.0))),
                prop("title", TreeValueKind::Text, false, None),
                prop("serial", TreeValueKind::Int, true, None),
                prop("visible", TreeValueKind::Bool, false, Some(MirrorValue::Bool(true))),
                prop(
                    "effects",
                    TreeValueKind::ListEffect,
                    false,
                    Some(MirrorValue::Effects(Vec::new())),
                ),
                prop(
                    "skipped",
                    TreeValueKind::ListText,
                    true,
                    Some(MirrorValue::Texts(Vec::new())),
                ),
            ],
            signals: vec!["changed".to_owned()],
            ..ClassSpec::default()
        },
        ClassSpec {
            name: "Window".to_owned(),
            base: Some("Node".to_owned()),
            props: vec![
                prop("w", TreeValueKind::Uint, false, None),
                prop("source", TreeValueKind::Node, false, None),
            ],
            methods: vec!["close".to_owned()],
            ..ClassSpec::default()
        },
    ]
}

/// Root 1 (node) with child 2 (window); epoch 7, serial 5.
fn setup() -> (Lua, Shared) {
    let mirror = TreeMirror::from_snapshot(7, 1, 5, &[snap(1, NIL, 0, "node"), snap(2, 1, 0, "window")])
        .expect("snapshot builds");
    let core = TreeCore::shared(mirror, schema()).expect("schema is valid");
    let lua = Lua::new();
    let table = install(&lua, &core).expect("core table builds");
    lua.globals().set("core", table).expect("core is set");
    (lua, core)
}

/// Runs `source`, which checks itself with Lua's `assert`; a failure fails the test.
fn check(lua: &Lua, source: &str) {
    if let Err(err) = lua.load(source).exec() {
        panic!("{err}");
    }
}

/// The message `call` is refused with. The call must fail.
fn refused(lua: &Lua, call: &str) -> String {
    let (ok, message): (bool, String) = lua
        .load(format!(
            "local ok, e = pcall(function() {call} end) return ok, tostring(e)"
        ))
        .eval()
        .expect("pcall runs");
    assert!(!ok, "{call} should be refused");
    message
}

#[test]
fn new_ids_carry_the_epoch_and_are_unique_within_a_tick() {
    let (lua, _) = setup();
    let a: i64 = lua.load(r#"return core.new("Node")"#).eval().expect("new");
    let b: i64 = lua.load(r#"return core.new("Node")"#).eval().expect("new");
    assert_eq!(a >> 32, 7);
    assert_eq!(b >> 32, 7);
    assert_ne!(a, b);
}

#[test]
fn reads_fall_back_to_schema_defaults_and_inherited_properties() {
    let (lua, _) = setup();
    check(
        &lua,
        r#"
        assert(core.get(2, "class") == "Window")
        assert(core.get(2, "parent") == 1)
        assert(core.get(1, "parent") == nil)
        assert(core.get(2, "x") == 0)
        assert(core.get(2, "visible") == true)
        assert(core.get(2, "title") == nil)
        assert(core.get(2, "w") == nil)
        "#,
    );
}

#[test]
fn writes_are_seen_by_this_tick_and_the_mirror_is_unchanged_until_the_tx_is_taken() {
    let (lua, core) = setup();
    check(
        &lua,
        r#"
        core.set(2, "x", 4)
        core.set(2, "title", "main")
        assert(core.get(2, "x") == 4)
        assert(core.get(2, "title") == "main")
        "#,
    );
    assert_eq!(core.borrow().mirror().value(2, "x"), None);

    let tx = core.borrow_mut().take_transaction(9);
    assert_eq!(tx.base_serial, 5);
    assert_eq!(tx.serial, 9);
    assert_eq!(
        tx.ops,
        vec![
            TxOp::Set {
                node: 2,
                key: "x".to_owned(),
                value: MirrorValue::Double(4.0),
                seconds: None,
                delay: None,
                tween: None,
                samples: None,
            },
            TxOp::Set {
                node: 2,
                key: "title".to_owned(),
                value: MirrorValue::Text("main".to_owned()),
                seconds: None,
                delay: None,
                tween: None,
                samples: None,
            },
        ]
    );

    // Sent but not answered: the shell still reads what it wrote, and writes nothing again.
    check(
        &lua,
        r#"
        assert(core.get(2, "x") == 4)
        assert(core.get(2, "title") == "main")
        "#,
    );
    assert!(core.borrow_mut().take_transaction(10).ops.is_empty());

    // Answered: the mirror says what is so.
    core.borrow_mut().begin_tick(true);
    check(
        &lua,
        r#"
        assert(core.get(2, "x") == 0)
        assert(core.get(2, "title") == nil)
        "#,
    );
}

#[test]
fn refused_writes_name_the_check_and_never_reach_the_transaction() {
    let (lua, core) = setup();
    assert!(refused(&lua, r#"core.set(2, "nope", 1)"#).contains("Window has no property nope"));
    assert!(refused(&lua, r#"core.set(2, "serial", 1)"#).contains("Window.serial is read-only"));
    assert!(refused(&lua, r#"core.set(2, "visible", "yes")"#).contains("expected a bool"));
    assert!(refused(&lua, r#"core.set(2, "x", "far")"#).contains("expected a double"));
    assert!(refused(&lua, r#"core.set(2, "w", -1)"#).contains("expected a uint"));
    assert!(refused(&lua, r#"core.set(99, "x", 1)"#).contains("no node 99"));
    assert!(refused(&lua, r#"core.set(-1, "x", 1)"#).contains("no node 18446744073709551615"));
    assert!(refused(&lua, r#"core.set(2, "x", 1, -1)"#).contains("seconds must be"));
    assert!(refused(&lua, r#"core.set(2, "x", 1, 0.5, {0.5})"#).contains("at least two"));
    assert!(refused(&lua, r#"core.set(2, "x", 1, 0.5, {0, 2})"#).contains("[0, 1]"));
    assert!(core.borrow_mut().take_transaction(6).ops.is_empty());
}

#[test]
fn insert_and_destroy_are_checked_and_a_destroyed_node_is_gone_for_the_rest_of_the_tick() {
    let (lua, _) = setup();
    assert!(refused(&lua, "core.destroy(1)").contains("is the root"));
    assert!(refused(&lua, "core.insert(2, 1)").contains("root"));
    assert!(refused(&lua, "core.destroy(99)").contains("no node 99"));
    assert!(refused(&lua, r#"core.new("nope")"#).contains("no class nope"));
    check(
        &lua,
        r#"
        core.destroy(2)
        assert(not pcall(core.get, 2, "class"))
        "#,
    );
}

/// A node the shell made is read by the ticks that follow it until the compositor's batch brings it into the mirror:
/// a binding that runs in between (a native change woke it) must find the node its own tick made.
#[test]
fn a_made_node_stays_readable_after_its_transaction_is_taken_until_the_mirror_holds_it() {
    let (lua, core) = setup();
    check(
        &lua,
        r#"
        made = core.new("Node")
        core.insert(1, made)
        core.set(made, "x", 3)
        core.set(2, "title", "sent")
        "#,
    );
    let made = 7 << 32 | 1;
    core.borrow_mut().take_transaction(6);
    check(
        &lua,
        r#"
        assert(core.get(made, "class") == "Node")
        assert(core.get(made, "parent") == 1)
        assert(core.get(made, "x") == 3)
        assert(core.get(2, "title") == "sent", "a value the shell wrote is read back before the echo")
        core.set(made, "x", 4)
        assert(core.get(made, "x") == 4)
        "#,
    );
    assert_eq!(
        core.borrow_mut().take_transaction(7).ops.len(),
        1,
        "only the new write goes out"
    );
    check(&lua, r#"assert(core.get(2, "title") == "sent")"#);

    // The compositor's batch arrives: the mirror holds the node, and reads come from it again.
    let batch = [
        MirrorChange {
            class: "node".to_owned(),
            parent: 1,
            ..change(MirrorChangeKind::Created, made)
        },
        MirrorChange {
            key: "x".to_owned(),
            value: Some(MirrorValue::Double(4.0)),
            ..change(MirrorChangeKind::Set, made)
        },
    ];
    core.borrow_mut()
        .mirror_mut()
        .apply_changes(6, &batch)
        .expect("the batch applies");
    core.borrow_mut().begin_tick(true);
    check(
        &lua,
        r#"
        assert(core.get(made, "x") == 4)
        assert(core.get(2, "title") == nil, "once answered, the mirror says what is so")
        "#,
    );
    assert!(core.borrow_mut().take_transaction(8).ops.is_empty());
}

/// A transaction the compositor refused for good never makes its nodes: they are not read as there.
#[test]
fn a_made_node_of_a_refused_transaction_is_forgotten() {
    let (lua, core) = setup();
    check(&lua, r#"made = core.new("Node")"#);
    core.borrow_mut().take_transaction(6);
    check(&lua, r#"assert(core.get(made, "class") == "Node")"#);
    core.borrow_mut().begin_tick(true);
    assert!(refused(&lua, r#"core.get(made, "class")"#).contains("no node"));
}

/// The contract `Node:destroy` in `node.lua` relies on, and the fake core in
/// `tests/lua/fake_core.lua` follows: `core.destroy` answers a list of `{holder, key}`, one per
/// stored node reference that became nil, and the nil is visible for the rest of the tick.
#[test]
fn destroy_answers_the_node_references_that_became_nil() {
    let (lua, core) = setup();
    check(
        &lua,
        r#"
        local target = core.new("Node")
        core.insert(1, target)
        core.set(2, "source", target)
        local cleared = core.destroy(target)
        assert(type(cleared) == "table" and #cleared == 1)
        assert(cleared[1][1] == 2 and cleared[1][2] == "source")
        assert(core.get(2, "source") == nil)
        local none = core.destroy(2)
        assert(type(none) == "table" and #none == 0)
        "#,
    );
    let ops = core.borrow_mut().take_transaction(9).ops;
    let kinds: Vec<&str> = ops
        .iter()
        .map(|op| match op {
            TxOp::New { .. } => "new",
            TxOp::Insert { .. } => "insert",
            TxOp::Set { .. } => "set",
            TxOp::Destroy { .. } => "destroy",
            TxOp::Call { .. } => "call",
        })
        .collect();
    assert_eq!(kinds, ["new", "insert", "set", "destroy", "destroy"]);
}

#[test]
fn calls_need_a_declared_method_and_carry_their_argument_bytes() {
    let (lua, core) = setup();
    assert!(refused(&lua, r#"core.call(2, "open")"#).contains("Window has no method open"));
    assert!(refused(&lua, r#"core.call(1, "close")"#).contains("Node has no method close"));
    check(&lua, r#"core.call(2, "close", "ab")"#);
    check(&lua, r#"core.call(2, "close", "cd", "Super+q")"#);
    let tx = core.borrow_mut().take_transaction(6);
    assert_eq!(
        tx.ops,
        vec![
            TxOp::Call {
                node: 2,
                method: "close".to_owned(),
                args: b"ab".to_vec(),
                token: 1,
                text: gnoblin_secret::SecretText::default(),
            },
            TxOp::Call {
                node: 2,
                method: "close".to_owned(),
                args: b"cd".to_vec(),
                token: 2,
                text: gnoblin_secret::SecretText::new("Super+q"),
            },
        ]
    );
}

#[test]
fn classes_lists_the_schema_parent_first() {
    let (lua, _) = setup();
    check(
        &lua,
        r#"
        local cs = core.classes()
        assert(#cs == 2)
        assert(cs[1].name == "Node" and cs[1].base == nil)
        assert(cs[2].name == "Window" and cs[2].base == "Node")
        assert(cs[1].props.x.type == "double" and cs[1].props.x.default == 0)
        assert(cs[1].props.serial.readonly == true and cs[1].props.serial.default == nil)
        assert(cs[1].props.title.readonly == false)
        assert(cs[1].signals[1] == "changed")
        assert(cs[2].methods[1] == "close")
        assert(cs[2].props.w.type == "uint")
        "#,
    );
}

#[test]
fn a_node_effects_list_takes_blur_and_shadow_only_and_skipped_is_read_only() {
    let (lua, _) = setup();
    check(
        &lua,
        r#"
        core.set(2, "effects", { { kind = "blur", passes = 2 }, { kind = "shadow", offset_y = 4 } })
        local list = core.get(2, "effects")
        assert(#list == 2 and list[1].kind == "blur" and list[1].passes == 2)
        assert(list[2].kind == "shadow" and list[2].offset_y == 4)
        assert(list[1].enabled == nil and list[1].source == nil, "the record has no enabled, source or entry")
        assert(#core.get(2, "skipped") == 0, "nothing is skipped until the compositor says so")
        "#,
    );
    let shader = refused(&lua, r#"core.set(2, "effects", { { kind = "shader" } })"#);
    assert!(
        shader.contains("user shaders were removed") && shader.contains("blur, shadow or border"),
        "a shader kind names what replaced it: {shader}"
    );
    let source = refused(
        &lua,
        r#"core.set(2, "effects", { { kind = "shader", source = "void main() {}" } })"#,
    );
    assert!(
        source.contains("user shaders were removed") && source.contains("blur, shadow or border"),
        "a shader's source field names what replaced it: {source}"
    );
    assert!(
        refused(
            &lua,
            r#"core.set(2, "effects", { { kind = "blur", enabled = false } })"#
        )
        .contains("no field enabled")
    );
    refused(&lua, r#"core.set(2, "skipped", { "blur: unsupported-effect-on-3d" })"#);
}

#[test]
fn reports_are_taken_in_order_and_then_cleared() {
    let (lua, core) = setup();
    check(&lua, r#"core.report("first") core.report("second")"#);
    assert_eq!(
        core.borrow_mut().take_reports(),
        vec!["first".to_owned(), "second".to_owned()]
    );
    assert!(core.borrow_mut().take_reports().is_empty());
}

#[test]
fn a_schema_that_repeats_a_class_or_names_an_undeclared_base_is_refused() {
    let dup = vec![
        ClassSpec {
            name: "Node".to_owned(),
            ..ClassSpec::default()
        },
        ClassSpec {
            name: "Node".to_owned(),
            ..ClassSpec::default()
        },
    ];
    let err = TreeCore::shared(TreeMirror::empty(), dup).expect_err("duplicate class is refused");
    assert!(err.contains("declared twice"), "{err}");

    let orphan = vec![ClassSpec {
        name: "Window".to_owned(),
        base: Some("Node".to_owned()),
        ..ClassSpec::default()
    }];
    let err = TreeCore::shared(TreeMirror::empty(), orphan).expect_err("undeclared base is refused");
    assert!(err.contains("is not declared before"), "{err}");
}
