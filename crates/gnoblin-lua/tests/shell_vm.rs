//! Tests for `ShellVm`: the shell's Lua VM over the real tree core.
//!
//! The compositor is played by `FakeLink`, an in-memory `TreeLink`. It applies
//! every transaction to a node map and answers each applied transaction with a
//! change batch carrying the serial, then the reply.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gnoblin_lua::shell_vm::{LuaEvent, ShellVm};
use gnoblin_lua::tree_core::{ClassSpec, PropSpec, TreeValueKind};
use gnoblin_lua::tree_mirror::{
    Attached, LinkError, LuaChange, LuaIncoming, LuaOp, LuaTransaction, LuaValue, MirrorChangeKind, SnapshotNode,
    TreeLink, TxReply,
};

fn lua_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

fn classes() -> Vec<ClassSpec> {
    let node = ClassSpec {
        name: "Node".into(),
        base: None,
        props: vec![
            PropSpec {
                name: "children".into(),
                kind: TreeValueKind::Nodes,
                readonly: true,
                default: None,
            },
            PropSpec {
                name: "parent".into(),
                kind: TreeValueKind::Node,
                readonly: true,
                default: None,
            },
            PropSpec {
                name: "title".into(),
                kind: TreeValueKind::Text,
                readonly: false,
                default: None,
            },
        ],
        signals: vec!["ready".into()],
        ..ClassSpec::default()
    };
    let window = ClassSpec {
        name: "Window".into(),
        base: Some("Node".into()),
        ..ClassSpec::default()
    };
    vec![node, window]
}

/// Compositor-side state shared between the test and the fake link.
#[derive(Default)]
struct Compositor {
    /// node id -> (class, parent, title). Parent 0 is no parent.
    nodes: BTreeMap<u64, (String, u64, String)>,
    serial: u64,
    /// Transactions the link has received, in order.
    sent: Vec<Vec<LuaOp>>,
    /// The next transaction is refused when set.
    refuse_next: bool,
    /// Incoming messages waiting for the next `poll`.
    queue: Vec<LuaIncoming>,
}

#[derive(Clone)]
struct FakeLink(Rc<RefCell<Compositor>>);

fn change(kind: MirrorChangeKind, node: u64) -> LuaChange {
    LuaChange {
        kind,
        node,
        class: String::new(),
        parent: 0,
        index: 0,
        key: String::new(),
        value: None,
        removed: Vec::new(),
    }
}

/// A `Set` change for `node.key`, as the compositor reports a native write.
fn set_change(node: u64, key: &str, value: &str) -> LuaChange {
    LuaChange {
        key: key.into(),
        value: Some(LuaValue::Text(value.into())),
        ..change(MirrorChangeKind::Set, node)
    }
}

impl FakeLink {
    fn snapshot(&self) -> LuaIncoming {
        let c = self.0.borrow();
        let nodes = c
            .nodes
            .iter()
            .map(|(id, (class, parent, _))| SnapshotNode {
                id: *id,
                parent: *parent,
                index: 0,
                class: class.clone(),
                title: String::new(),
                app_id: String::new(),
                props: Vec::new(),
            })
            .collect();
        LuaIncoming::Snapshot {
            serial: c.serial,
            chunk: 0,
            last: true,
            nodes,
        }
    }

    /// Applies `ops` to the node map and returns the change batch describing them.
    fn apply(&self, ops: &[LuaOp]) -> Vec<LuaChange> {
        let mut c = self.0.borrow_mut();
        let mut changes = Vec::new();
        for op in ops {
            match op {
                LuaOp::New { id, class } => {
                    c.nodes.insert(*id, (class.clone(), 0, String::new()));
                    changes.push(LuaChange {
                        class: class.clone(),
                        ..change(MirrorChangeKind::Created, *id)
                    });
                }
                LuaOp::Insert { node, parent, index } => {
                    if let Some(entry) = c.nodes.get_mut(node) {
                        entry.1 = *parent;
                    }
                    changes.push(LuaChange {
                        parent: *parent,
                        index: *index,
                        ..change(MirrorChangeKind::Parented, *node)
                    });
                }
                LuaOp::Set { node, key, value, .. } => {
                    if let (Some(entry), LuaValue::Text(text), true) = (c.nodes.get_mut(node), value, key == "title") {
                        entry.2 = text.clone();
                    }
                    changes.push(LuaChange {
                        key: key.clone(),
                        value: Some(value.clone()),
                        ..change(MirrorChangeKind::Set, *node)
                    });
                }
                _ => {}
            }
        }
        changes
    }
}

impl TreeLink for FakeLink {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        let attached = {
            let c = self.0.borrow();
            Attached {
                epoch: 1,
                root: 1,
                serial: c.serial,
            }
        };
        let snapshot = self.snapshot();
        let mut c = self.0.borrow_mut();
        c.queue.push(LuaIncoming::Attached(attached));
        c.queue.push(snapshot);
        Ok(())
    }

    fn transact(&mut self, tx: &LuaTransaction) -> Result<(), LinkError> {
        let ops = tx.ops.clone();
        let refuse = {
            let mut c = self.0.borrow_mut();
            c.sent.push(ops.clone());
            std::mem::replace(&mut c.refuse_next, false)
        };
        let serial = tx.serial;
        if refuse {
            self.0.borrow_mut().queue.push(LuaIncoming::Reply(TxReply {
                serial,
                applied: false,
                failed_op: 0,
                code: "refused".into(),
                removed: Vec::new(),
            }));
            return Ok(());
        }
        let changes = self.apply(&ops);
        let mut c = self.0.borrow_mut();
        c.serial = serial;
        c.queue.push(LuaIncoming::Changes { serial, changes });
        c.queue.push(LuaIncoming::Reply(TxReply {
            serial,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: Vec::new(),
        }));
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<LuaIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.0.borrow_mut().queue))
    }
}

fn compositor_with_root_and_window() -> Rc<RefCell<Compositor>> {
    let mut c = Compositor::default();
    c.nodes.insert(1, ("Node".into(), 0, String::new()));
    c.nodes.insert(2, ("Window".into(), 1, "term".into()));
    c.serial = 1;
    Rc::new(RefCell::new(c))
}

fn boot(comp: &Rc<RefCell<Compositor>>) -> ShellVm {
    let link = FakeLink(comp.clone());
    ShellVm::new(Box::new(link), &lua_dir(), classes()).expect("boot ShellVm")
}

fn sent(comp: &Rc<RefCell<Compositor>>) -> Vec<Vec<LuaOp>> {
    comp.borrow().sent.clone()
}

fn event(target: u64, kind: &str) -> LuaEvent {
    LuaEvent {
        target,
        kind: kind.to_string(),
        bubbles: false,
        chord: None,
    }
}

/// Reads `expr` inside the VM and returns its string form.
fn check(vm: &mut ShellVm, name: &str, src: &str) -> String {
    vm.run(name, src).expect("run lua").expect("lua returned a value")
}

// (1) boot from a snapshot: the root and a Window child are visible to the VM
#[test]
fn boots_from_snapshot_with_root_and_window() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("first frame");
    assert_eq!(
        check(&mut vm, "parent", "return tostring(root.children[1].parent == root)"),
        "true"
    );
    assert_eq!(check(&mut vm, "count", "return #root.children"), "1");
}

// (2) a handler creating a node, parenting it under root, and setting a title
// produces ONE transaction with new, insert, set in that order.
#[test]
fn handler_creates_node_in_one_transaction() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "create",
        r#"
        local n = Node()
        n.title = "new"
        root:add(n)
        "#,
    )
    .expect("run create");
    vm.frame(1.0, vec![]).expect("write frame");

    let txs = sent(&comp);
    assert_eq!(txs.len(), 1, "exactly one transaction: {txs:?}");
    let kinds: Vec<&str> = txs[0]
        .iter()
        .map(|op| match op {
            LuaOp::New { .. } => "new",
            LuaOp::Insert { .. } => "insert",
            LuaOp::Set { .. } => "set",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["new", "set", "insert"], "ops follow write order");
}

// (3) the fake applies it; the next frame's change batch updates the mirror
#[test]
fn next_frame_mirror_sees_created_node() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run("create", r#"local n = Node(); n.title = "new"; root:add(n)"#)
        .expect("run create");
    vm.frame(1.0, vec![]).expect("write frame");
    vm.frame(2.0, vec![]).expect("change frame");
    assert_eq!(check(&mut vm, "count", "return #root.children"), "2");
    assert_eq!(check(&mut vm, "title", "return root.children[2].title"), "new");
}

// (4) a refused transaction leaves the mirror unchanged and the VM keeps running
#[test]
fn refused_transaction_leaves_mirror_unchanged() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    comp.borrow_mut().refuse_next = true;
    vm.run("create", r#"local n = Node(); root:add(n)"#)
        .expect("run create");
    let refused = vm.frame(1.0, vec![]).expect("refused frame must not error");
    assert_eq!(refused.diagnostics.len(), 0, "refusal is reported on the next frame");
    let next = vm.frame(2.0, vec![]).expect("VM keeps running after refusal");
    assert_eq!(next.diagnostics.len(), 1, "refusal reported: {:?}", next.diagnostics);
    assert_eq!(
        check(&mut vm, "count", "return #root.children"),
        "1",
        "mirror still has only the window"
    );
    assert_eq!(check(&mut vm, "alive", "return 'alive'"), "alive");
}

// (5) two identical runs produce identical sent transactions
#[test]
fn identical_inputs_produce_identical_transactions() {
    let script = r#"local n = Node(); n.title = "det"; root:add(n)"#;
    let run_once = || {
        let comp = compositor_with_root_and_window();
        let mut vm = boot(&comp);
        vm.frame(0.0, vec![]).expect("boot frame");
        vm.run("create", script).expect("run");
        vm.frame(1.0, vec![]).expect("write frame");
        sent(&comp)
    };
    assert_eq!(run_once(), run_once());
}

// (6) a function-valued property binding re-runs when the mirror changes
#[test]
fn function_binding_reruns_on_mirror_change() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "bind",
        r#"
        local w = root.children[1]
        SRC = w
        OUT = Node()
        root:add(OUT)
        OUT.title = function() return "bound:" .. tostring(SRC.title) end
        "#,
    )
    .expect("bind");
    vm.frame(1.0, vec![]).expect("frame after bind");
    {
        let mut c = comp.borrow_mut();
        c.nodes.get_mut(&2).expect("window").2 = "changed".into();
        let serial = c.serial + 1;
        c.serial = serial;
        c.queue.push(LuaIncoming::Changes {
            serial,
            changes: vec![set_change(2, "title", "changed")],
        });
    }
    vm.frame(2.0, vec![]).expect("frame after change");
    vm.frame(3.0, vec![])
        .expect("frame that applies the echo of the binding's write");
    assert_eq!(
        check(&mut vm, "src", "return SRC.title"),
        "changed",
        "mirror applied the change"
    );
    assert_eq!(check(&mut vm, "read", "return OUT.title"), "bound:changed");
}

// (7) a class handler fires when a `ready` event arrives
#[test]
fn class_handler_fires_on_ready_event() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.run(
        "handler",
        r#"
        READY = 0
        function Window:on_ready() READY = READY + 1 end
        "#,
    )
    .expect("install handler");
    vm.frame(0.0, vec![event(2, "ready")]).expect("ready frame");
    assert_eq!(check(&mut vm, "read", "return READY"), "1");
}

// (8) restart: a new ShellVm over the same compositor rebuilds its mirror from
// a fresh snapshot and sends nothing for a node the first VM already made.
#[test]
fn restart_rebuilds_mirror_without_recreating_nodes() {
    let comp = compositor_with_root_and_window();
    {
        let mut vm = boot(&comp);
        vm.frame(0.0, vec![]).expect("boot frame");
        vm.run("create", r#"local n = Node(); n.title = "kept"; root:add(n)"#)
            .expect("create");
        vm.frame(1.0, vec![]).expect("write frame");
        vm.frame(2.0, vec![]).expect("change frame");
    }
    let before = sent(&comp).len();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("second boot frame");
    vm.frame(1.0, vec![]).expect("second idle frame");
    assert_eq!(sent(&comp).len(), before, "restart sends no transactions");
    assert_eq!(check(&mut vm, "count", "return #root.children"), "2");
}
