//! The Lua `core` table over the shell's tree core (design/node-core.md, slice 10): the calls
//! `node.lua` makes, with the refusals the schema and the transaction builder make.

use gnoblin_lua::tree_core::{ClassSpec, PropSpec, Shared, TreeCore, TreeValueKind, install};
use gnoblin_lua::tree_mirror::{LuaMirror, LuaOp, LuaValue, NIL, SnapshotNode};
use mlua::Lua;

fn snap(id: u64, parent: u64, index: i64, class: &str) -> SnapshotNode {
    SnapshotNode {
        id,
        parent,
        index,
        class: class.to_owned(),
    }
}

fn prop(name: &str, kind: TreeValueKind, readonly: bool, default: Option<LuaValue>) -> PropSpec {
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
            name: "node".to_owned(),
            props: vec![
                prop("x", TreeValueKind::Double, false, Some(LuaValue::Double(0.0))),
                prop("title", TreeValueKind::Text, false, None),
                prop("serial", TreeValueKind::Int, true, None),
                prop("visible", TreeValueKind::Bool, false, Some(LuaValue::Bool(true))),
            ],
            signals: vec!["changed".to_owned()],
            ..ClassSpec::default()
        },
        ClassSpec {
            name: "window".to_owned(),
            base: Some("node".to_owned()),
            props: vec![prop("w", TreeValueKind::Uint, false, None)],
            methods: vec!["close".to_owned()],
            ..ClassSpec::default()
        },
    ]
}

/// Root 1 (node) with child 2 (window); epoch 7, serial 5.
fn setup() -> (Lua, Shared) {
    let mirror = LuaMirror::from_snapshot(7, 1, 5, &[snap(1, NIL, 0, "node"), snap(2, 1, 0, "window")])
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
    let a: i64 = lua.load(r#"return core.new("node")"#).eval().expect("new");
    let b: i64 = lua.load(r#"return core.new("node")"#).eval().expect("new");
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
        assert(core.get(2, "class") == "window")
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
            LuaOp::Set {
                node: 2,
                key: "x".to_owned(),
                value: LuaValue::Double(4.0),
                seconds: None,
                samples: None,
            },
            LuaOp::Set {
                node: 2,
                key: "title".to_owned(),
                value: LuaValue::Text("main".to_owned()),
                seconds: None,
                samples: None,
            },
        ]
    );

    check(
        &lua,
        r#"
        assert(core.get(2, "x") == 0)
        assert(core.get(2, "title") == nil)
        "#,
    );
    assert!(core.borrow_mut().take_transaction(10).ops.is_empty());
}

#[test]
fn refused_writes_name_the_check_and_never_reach_the_transaction() {
    let (lua, core) = setup();
    assert!(refused(&lua, r#"core.set(2, "nope", 1)"#).contains("window has no property nope"));
    assert!(refused(&lua, r#"core.set(2, "serial", 1)"#).contains("window.serial is read-only"));
    assert!(refused(&lua, r#"core.set(2, "visible", "yes")"#).contains("expected a bool"));
    assert!(refused(&lua, r#"core.set(2, "x", "far")"#).contains("expected a double"));
    assert!(refused(&lua, r#"core.set(2, "w", -1)"#).contains("expected a uint"));
    assert!(refused(&lua, r#"core.set(99, "x", 1)"#).contains("no node 99"));
    assert!(refused(&lua, r#"core.set(-1, "x", 1)"#).contains("no node -1"));
    assert!(refused(&lua, r#"core.set(2, "x", 1, -1)"#).contains("seconds must be"));
    assert!(refused(&lua, r#"core.set(2, "x", 1, 0.5, {0.5})"#).contains("at least two"));
    assert!(refused(&lua, r#"core.set(2, "x", 1, 0.5, {0, 2})"#).contains("[0, 1]"));
    assert!(core.borrow_mut().take_transaction(6).ops.is_empty());
}

#[test]
fn insert_and_destroy_are_checked_and_a_destroyed_node_is_gone_for_the_rest_of_the_tick() {
    let (lua, _) = setup();
    assert!(refused(&lua, "core.destroy(1)").contains("is the root"));
    assert!(refused(&lua, "core.insert(1, 2)").contains("root"));
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

#[test]
fn calls_need_a_declared_method_and_carry_their_argument_bytes() {
    let (lua, core) = setup();
    assert!(refused(&lua, r#"core.call(2, "open")"#).contains("window has no method open"));
    assert!(refused(&lua, r#"core.call(1, "close")"#).contains("node has no method close"));
    check(&lua, r#"core.call(2, "close", "ab")"#);
    let tx = core.borrow_mut().take_transaction(6);
    assert_eq!(
        tx.ops,
        vec![LuaOp::Call {
            node: 2,
            method: "close".to_owned(),
            args: b"ab".to_vec(),
        }]
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
        assert(cs[1].name == "node" and cs[1].base == nil)
        assert(cs[2].name == "window" and cs[2].base == "node")
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
            name: "node".to_owned(),
            ..ClassSpec::default()
        },
        ClassSpec {
            name: "node".to_owned(),
            ..ClassSpec::default()
        },
    ];
    let err = TreeCore::shared(LuaMirror::empty(), dup).expect_err("duplicate class is refused");
    assert!(err.contains("declared twice"), "{err}");

    let orphan = vec![ClassSpec {
        name: "window".to_owned(),
        base: Some("node".to_owned()),
        ..ClassSpec::default()
    }];
    let err = TreeCore::shared(LuaMirror::empty(), orphan).expect_err("undeclared base is refused");
    assert!(err.contains("is not declared before"), "{err}");
}
