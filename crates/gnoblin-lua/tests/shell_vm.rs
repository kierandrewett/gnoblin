//! Tests for `ShellVm`: the shell's Lua VM over the real tree core.
//!
//! The compositor is played by `FakeLink`, an in-memory `TreeLink`. It applies
//! every transaction to a node map and answers each applied transaction with a
//! change batch carrying the serial, then the reply.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gnoblin_lua::Limits;
use gnoblin_lua::shell_vm::{LuaEvent, REPORT_INTERVAL, ShellLimits, ShellVm};
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
        methods: vec!["measure".into(), "authenticate".into()],
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
    /// The serial at which each node last changed in a batch the compositor published.
    changed: BTreeMap<u64, u64>,
    /// Transactions the link has received, in order.
    sent: Vec<Vec<LuaOp>>,
    /// The next transaction is refused when set.
    refuse_next: bool,
    /// When set, transactions are recorded in `held` and applied only by `release`, as a real
    /// compositor echoes them later.
    hold: bool,
    held: Vec<LuaTransaction>,
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
    fn apply(&self, ops: &[LuaOp], serial: u64) -> Vec<LuaChange> {
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
                LuaOp::Destroy { node } => {
                    c.nodes.remove(node);
                    changes.push(LuaChange {
                        removed: vec![*node],
                        ..change(MirrorChangeKind::Destroyed, *node)
                    });
                }
                _ => {}
            }
        }
        for change in &changes {
            c.changed.insert(change.node, serial);
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
        {
            let mut c = self.0.borrow_mut();
            c.sent.push(tx.ops.clone());
            if c.hold {
                c.held.push(tx.clone());
                return Ok(());
            }
            if std::mem::replace(&mut c.refuse_next, false) {
                c.queue.push(LuaIncoming::Reply(TxReply {
                    serial: tx.serial,
                    applied: false,
                    failed_op: 0,
                    code: "refused".into(),
                    removed: Vec::new(),
                }));
                return Ok(());
            }
        }
        self.land(tx);
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<LuaIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.0.borrow_mut().queue))
    }
}

impl FakeLink {
    /// Lands `tx` as the compositor does: refused `stale-base` at the first op whose node a batch
    /// published after the base changed, otherwise applied whole and echoed.
    fn land(&self, tx: &LuaTransaction) {
        if let Some(failed_op) = self.missing_op(tx) {
            let serial = self.0.borrow().serial;
            self.0.borrow_mut().queue.push(LuaIncoming::Reply(TxReply {
                serial,
                applied: false,
                failed_op: failed_op as i64,
                code: "unknown-node".into(),
                removed: Vec::new(),
            }));
            return;
        }
        if let Some(failed_op) = self.stale_op(tx) {
            self.0.borrow_mut().queue.push(LuaIncoming::Reply(TxReply {
                serial: tx.serial,
                applied: false,
                failed_op: failed_op as i64,
                code: "stale-base".into(),
                removed: Vec::new(),
            }));
            return;
        }
        let changes = self.apply(&tx.ops, tx.serial);
        let mut c = self.0.borrow_mut();
        c.serial = tx.serial;
        c.queue.push(LuaIncoming::Changes {
            serial: tx.serial,
            changes,
        });
        c.queue.push(LuaIncoming::Reply(TxReply {
            serial: tx.serial,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: Vec::new(),
        }));
    }

    /// The index of the first op of `tx` on a node the compositor no longer has (and `tx` does not make).
    fn missing_op(&self, tx: &LuaTransaction) -> Option<usize> {
        let c = self.0.borrow();
        let mut made = Vec::new();
        tx.ops.iter().position(|op| {
            let node = match op {
                LuaOp::New { id, .. } => {
                    made.push(*id);
                    return false;
                }
                LuaOp::Set { node, .. } | LuaOp::Insert { node, .. } | LuaOp::Destroy { node } => *node,
                LuaOp::Call { node, .. } => *node,
            };
            !c.nodes.contains_key(&node) && !made.contains(&node)
        })
    }

    /// The index of the first op of `tx` on a node that a batch published after `tx`'s base
    /// changed, or none when the base is current.
    fn stale_op(&self, tx: &LuaTransaction) -> Option<usize> {
        let c = self.0.borrow();
        if tx.base_serial == c.serial {
            return None;
        }
        tx.ops.iter().position(|op| {
            let node = match op {
                LuaOp::Set { node, .. } | LuaOp::Insert { node, .. } | LuaOp::Destroy { node } => *node,
                _ => return false,
            };
            c.changed.get(&node).is_some_and(|serial| *serial > tx.base_serial)
        })
    }

    /// A native title write the shell did not make: the compositor publishes it as a batch of its own.
    fn external_title(&self, node: u64, text: &str) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        if let Some(entry) = c.nodes.get_mut(&node) {
            entry.2 = text.into();
        }
        c.changed.insert(node, serial);
        c.queue.push(LuaIncoming::Changes {
            serial,
            changes: vec![set_change(node, "title", text)],
        });
    }

    /// The compositor destroys `node` on its own, as it does a window whose client closed.
    fn destroy_natively(&self, node: u64) {
        let mut c = self.0.borrow_mut();
        c.nodes.remove(&node);
        c.serial += 1;
        let serial = c.serial;
        c.changed.insert(node, serial);
        c.queue.push(LuaIncoming::Changes {
            serial,
            changes: vec![LuaChange {
                removed: vec![node],
                ..change(MirrorChangeKind::Destroyed, node)
            }],
        });
    }

    /// Lands every held transaction in order, as the compositor echoes them.
    fn release(&self) {
        let held = std::mem::take(&mut self.0.borrow_mut().held);
        for tx in &held {
            self.land(tx);
        }
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
        args: Vec::new(),
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
    vm.frame(0.0, vec![]).expect("boot frame announces the snapshot");
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

// (N) while a transaction is in flight (its echo not yet back) a later frame holds its ops
// rather than sending a second transaction on the same base, which the compositor refuses as
// stale. After the echo lands the held ops go out on the new base and both nodes are inserted.
#[test]
fn holds_ops_while_transaction_in_flight() {
    let comp = compositor_with_root_and_window();
    comp.borrow_mut().hold = true;
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run("first", "local a = Node()\nroot:add(a)").expect("run first");
    vm.frame(1.0, vec![]).expect("first write frame");
    assert_eq!(sent(&comp).len(), 1, "first transaction sent");

    vm.run("second", "local b = Node()\nroot:add(b)").expect("run second");
    vm.frame(2.0, vec![]).expect("second write frame");
    assert_eq!(sent(&comp).len(), 1, "second transaction held while first in flight");

    FakeLink(comp.clone()).release();
    vm.frame(3.0, vec![]).expect("echo frame");
    vm.frame(4.0, vec![]).expect("send held frame");
    assert_eq!(sent(&comp).len(), 2, "held ops sent after echo");
    FakeLink(comp.clone()).release();
    vm.frame(5.0, vec![]).expect("final echo frame");
    assert_eq!(
        check(&mut vm, "count", "return #root.children"),
        "3",
        "both nodes inserted"
    );
}

// (R) a transaction refused `stale-base` because a native batch changed a node it writes is sent
// again without the refused op, on the new base, and the native change stands.
#[test]
fn stale_base_refusal_resends_the_other_ops_on_the_new_base() {
    let comp = compositor_with_root_and_window();
    comp.borrow_mut().hold = true;
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "write",
        "local w = root.children[1]\nlocal n = Node()\nroot:add(n)\nw.title = \"mine\"",
    )
    .expect("run write");
    let first = vm.frame(1.0, vec![]).expect("write frame");
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    assert_eq!(sent(&comp).len(), 1, "first transaction held in flight");

    FakeLink(comp.clone()).external_title(2, "ext");
    FakeLink(comp.clone()).release();
    let refused = vm.frame(2.0, vec![]).expect("refusal frame");
    assert!(
        refused.diagnostics.is_empty(),
        "a stale-base refusal is retried, not reported: {:?}",
        refused.diagnostics
    );
    let txs = sent(&comp);
    assert_eq!(txs.len(), 2, "the other ops are sent again");
    let kinds: Vec<&str> = txs[1]
        .iter()
        .map(|op| match op {
            LuaOp::New { .. } => "new",
            LuaOp::Insert { .. } => "insert",
            LuaOp::Set { .. } => "set",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["new", "insert"], "the refused title write is dropped");

    FakeLink(comp.clone()).release();
    let done = vm.frame(3.0, vec![]).expect("applied frame");
    assert!(done.diagnostics.is_empty(), "{:?}", done.diagnostics);
    assert_eq!(
        check(&mut vm, "count", "return #root.children"),
        "2",
        "new node inserted"
    );
    assert_eq!(
        check(&mut vm, "title", "return root.children[1].title"),
        "ext",
        "the native title stands"
    );
}

// (R1) a stale-base refusal sends a call whose text is content (a measure's string) again, and drops one whose
// text is a password, saying so: the password is never held past the send.
#[test]
fn stale_base_refusal_resends_a_measure_and_drops_a_password() {
    let comp = compositor_with_root_and_window();
    comp.borrow_mut().hold = true;
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "write",
        "local w = root.children[1]\nw.title = \"mine\"\nw:measure(\"\", \"sans\\nhello\")\nw:authenticate(\"\", \"hunter2\")",
    )
    .expect("run write");
    let first = vm.frame(1.0, vec![]).expect("write frame");
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    assert_eq!(sent(&comp).len(), 1, "first transaction held in flight");

    FakeLink(comp.clone()).external_title(2, "ext");
    FakeLink(comp.clone()).release();
    let refused = vm.frame(2.0, vec![]).expect("refusal frame");
    assert_eq!(
        refused.diagnostics.len(),
        1,
        "only the password call is reported: {:?}",
        refused.diagnostics
    );
    assert!(
        refused.diagnostics[0].contains("was dropped and not sent again"),
        "{:?}",
        refused.diagnostics
    );
    assert!(
        !refused.diagnostics[0].contains("hunter2"),
        "the report does not carry the password: {:?}",
        refused.diagnostics
    );
    let txs = sent(&comp);
    assert_eq!(txs.len(), 2, "the surviving op is sent again");
    let calls: Vec<(&str, &str)> = txs[1]
        .iter()
        .filter_map(|op| match op {
            LuaOp::Call { method, text, .. } => Some((method.as_str(), text.expose())),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        vec![("measure", "sans\nhello")],
        "the measure is sent again with its text, the password is not"
    );
}

// (R2) a transaction refused `unknown-node` because the compositor destroyed a node it writes is sent again without the
// ops on that node: a one-shot op in it, such as a destroy, is not lost with the write that was moot (gnoblin-6zce).
#[test]
fn unknown_node_refusal_resends_the_ops_on_nodes_that_are_still_there() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run("make", "mine = Node()\nroot:add(mine)").expect("run make");
    vm.frame(1.0, vec![]).expect("make frame");
    vm.frame(2.0, vec![]).expect("echo frame");
    assert_eq!(sent(&comp).len(), 1, "the new node went out");

    comp.borrow_mut().hold = true;
    vm.run("write", "root.children[1].title = \"moot\"\nmine:destroy()")
        .expect("run write");
    let held = vm.frame(3.0, vec![]).expect("write frame");
    assert!(held.diagnostics.is_empty(), "{:?}", held.diagnostics);
    assert_eq!(sent(&comp).len(), 2, "the write and the destroy are in flight");

    FakeLink(comp.clone()).destroy_natively(2);
    FakeLink(comp.clone()).release();
    let refused = vm.frame(4.0, vec![]).expect("refusal frame");
    assert!(
        refused.diagnostics.is_empty(),
        "an unknown-node refusal is retried, not reported: {:?}",
        refused.diagnostics
    );
    let txs = sent(&comp);
    assert_eq!(txs.len(), 3, "the surviving op is sent again");
    assert!(
        matches!(txs[2].as_slice(), [LuaOp::Destroy { .. }]),
        "only the destroy of the shell's own node is sent again: {:?}",
        txs[2]
    );
    FakeLink(comp.clone()).release();
    vm.frame(5.0, vec![]).expect("applied frame");
    assert_eq!(
        comp.borrow().nodes.len(),
        1,
        "the shell's node is gone from the compositor, only the root is left"
    );
}

// The sandbox and the budgets. The shell VM runs the stock shell and the user's files, so neither
// may reach the host, and neither may hold the frame loop.

/// The limits of a VM whose handlers get `instructions` and `milliseconds`, and whose frame gets
/// far more, so a test stops a runaway handler quickly without the frame backstop firing.
fn tight(instructions: u64, milliseconds: u64) -> ShellLimits {
    let entry = Limits {
        instructions,
        memory_bytes: 64 * 1024 * 1024,
        handler_ticks: milliseconds,
    };
    ShellLimits {
        entry,
        frame: Limits {
            instructions: instructions * 1000,
            memory_bytes: entry.memory_bytes,
            handler_ticks: 60_000,
        },
        ..ShellLimits::default()
    }
}

fn boot_with(comp: &Rc<RefCell<Compositor>>, limits: ShellLimits) -> ShellVm {
    let link = FakeLink(comp.clone());
    ShellVm::with_limits(Box::new(link), &lua_dir(), classes(), limits).expect("boot ShellVm")
}

/// The compositor changes `node`'s title itself, so a binding that reads it runs again.
fn native_title(comp: &Rc<RefCell<Compositor>>, node: u64, title: &str) {
    let mut c = comp.borrow_mut();
    c.nodes.get_mut(&node).expect("node").2 = title.into();
    let serial = c.serial + 1;
    c.serial = serial;
    c.queue.push(LuaIncoming::Changes {
        serial,
        changes: vec![set_change(node, "title", title)],
    });
}

fn run_error(vm: &mut ShellVm, src: &str) -> String {
    match vm.run("probe", src) {
        Ok(value) => panic!("`{src}` was meant to fail and returned {value:?}"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_shell_script_cannot_reach_the_filesystem_or_a_library_the_sandbox_withholds() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    for name in ["io", "debug", "package", "dofile", "loadfile"] {
        assert_eq!(
            check(&mut vm, name, &format!("return type({name})")),
            "nil",
            "`{name}` must not exist in the shell VM"
        );
    }
    assert_eq!(
        check(
            &mut vm,
            "os",
            "return tostring(os.execute == nil and os.getenv == nil and os.remove == nil \
             and os.rename == nil and os.tmpname == nil and os.exit == nil)"
        ),
        "true",
        "`os` keeps only its time functions"
    );
    assert_eq!(
        check(&mut vm, "os time", "return type(os.time()) .. type(os.clock())"),
        "numbernumber"
    );
    assert_eq!(
        check(
            &mut vm,
            "bytecode",
            "return tostring(load(string.dump(function() end)) == nil)"
        ),
        "true",
        "`load` refuses binary chunks"
    );
    for name in [
        "io",
        "debug",
        "package",
        "os",
        "..",
        "../../etc/passwd",
        "a/b",
        "gnoblin.nothing_here",
    ] {
        let message = run_error(&mut vm, &format!("return require('{name}')"));
        assert!(
            message.contains("module"),
            "require('{name}') must be refused as a module, got: {message}"
        );
    }
}

#[test]
fn a_handler_that_never_returns_is_stopped_and_the_frame_goes_on() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot_with(&comp, tight(300_000, 10_000));
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "handlers",
        r#"
        function Window:on_ready() while true do end end
        function Window:on_ready() root.children[1].title = "survived" end
        "#,
    )
    .expect("install handlers");
    let frame = vm.frame(1.0, vec![event(2, "ready")]).expect("the frame is not lost");
    assert_eq!(frame.diagnostics.len(), 1, "one report: {:?}", frame.diagnostics);
    assert!(
        frame.diagnostics[0].contains("Window on_ready handler") && frame.diagnostics[0].contains("instruction budget"),
        "the report names the handler and the budget: {:?}",
        frame.diagnostics
    );
    assert_eq!(sent(&comp).len(), 1, "the frame still sent its writes");
    vm.frame(2.0, vec![]).expect("the next frame applies the echo");
    assert_eq!(
        check(&mut vm, "after", "return tostring(root.children[1].title)"),
        "survived",
        "the handler after the runaway one still ran in the same frame"
    );
    assert_eq!(check(&mut vm, "alive", "return 1 + 1"), "2");
}

#[test]
fn a_runaway_that_catches_every_error_is_still_stopped() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot_with(&comp, tight(300_000, 10_000));
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "handler",
        "function Window:on_ready() while true do pcall(error, 'again') end end",
    )
    .expect("install handler");
    let frame = vm.frame(1.0, vec![event(2, "ready")]).expect("the frame is not lost");
    assert!(
        frame.diagnostics.iter().any(|d| d.contains("instruction budget")),
        "a pcall loop cannot outrun the budget: {:?}",
        frame.diagnostics
    );
}

#[test]
fn a_handler_that_runs_out_its_time_is_stopped_by_the_deadline() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot_with(&comp, tight(u64::MAX / 4_000, 30));
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run("handler", "function Window:on_ready() while true do end end")
        .expect("install handler");
    let started = std::time::Instant::now();
    let frame = vm.frame(1.0, vec![event(2, "ready")]).expect("the frame is not lost");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the frame took {:?}",
        started.elapsed()
    );
    assert!(
        frame.diagnostics.iter().any(|d| d.contains("deadline")),
        "the report names the deadline: {:?}",
        frame.diagnostics
    );
}

#[test]
fn a_binding_that_never_returns_is_stopped_and_named() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot_with(&comp, tight(300_000, 10_000));
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "bind",
        "local n = Node() root:add(n) n.title = function() while true do end end",
    )
    .expect("a runaway binding does not fail the chunk that made it");
    let frame = vm.frame(1.0, vec![]).expect("the frame is not lost");
    assert!(
        frame
            .diagnostics
            .iter()
            .any(|d| d.contains("Node.title binding") && d.contains("instruction budget")),
        "the report names the binding: {:?}",
        frame.diagnostics
    );
    assert_eq!(check(&mut vm, "alive", "return 1 + 1"), "2");
}

#[test]
fn a_loop_in_the_shells_own_code_is_bounded_by_the_frame_budget() {
    let comp = compositor_with_root_and_window();
    let mut options = tight(300_000, 10_000);
    options.frame.instructions = 300_000;
    let mut vm = boot_with(&comp, options);
    let message = run_error(&mut vm, "while true do end");
    assert!(message.contains("instruction budget"), "{message}");
    assert_eq!(
        check(&mut vm, "next entry", "return 1 + 1"),
        "2",
        "the next entry gets a fresh budget"
    );
}

// A handler that fails is a report, not a dead shell. `Workspace:arrange` is the case the design names:
// a user layout that throws must not blank the desktop, so the nodes keep what they were last given.

#[test]
fn a_failing_arrange_is_reported_and_the_nodes_keep_their_last_titles() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "pile",
        r#"
        FAIL = false
        Node:extend("Pile")({})
        function Pile:arrange()
            local kids = self.children
            if FAIL then error("layout exploded") end
            for i, c in ipairs(kids) do c.title = "slot" .. i end
        end
        PILE = Pile()
        root:add(PILE)
        PILE:add(Node())
        PILE:add(Node())
        "#,
    )
    .expect("build the pile");
    for t in 1..=3 {
        let frame = vm.frame(f64::from(t), vec![]).expect("frame");
        assert!(frame.diagnostics.is_empty(), "{:?}", frame.diagnostics);
    }
    let titles = "local out = {} for i, c in ipairs(PILE.children) do out[i] = tostring(c.title) end return table.concat(out, ',')";
    assert_eq!(check(&mut vm, "titles", titles), "slot1,slot2");

    vm.run("fail", "FAIL = true PILE:add(Node())")
        .expect("break the layout");
    let mut reports = Vec::new();
    for t in 4..=7 {
        let frame = vm
            .frame(f64::from(t), vec![])
            .expect("a failing layout does not end the VM");
        reports.extend(frame.diagnostics);
    }
    assert_eq!(reports.len(), 1, "one report, not one per frame: {reports:?}");
    assert!(
        reports[0].contains("Pile:arrange") && reports[0].contains("layout exploded"),
        "the report names the layout and the error: {reports:?}"
    );
    assert_eq!(
        check(&mut vm, "titles", titles),
        "slot1,slot2,nil",
        "the nodes laid out before keep their titles and the failing pass wrote none"
    );
    assert_eq!(check(&mut vm, "alive", "return #root.children"), "2");
}

#[test]
fn a_report_that_repeats_is_passed_on_once_per_interval_with_its_count() {
    let comp = compositor_with_root_and_window();
    let mut vm = boot(&comp);
    vm.frame(0.0, vec![]).expect("boot frame");
    vm.run(
        "bind",
        r#"
        local n = Node()
        root:add(n)
        n.title = function() local _ = root.children[1].title error("title binding failed") end
        "#,
    )
    .expect("bind");
    let mut seen: Vec<(f64, String)> = Vec::new();
    let mut at = 1.0;
    while at < REPORT_INTERVAL * 2.0 + 2.0 {
        native_title(&comp, 2, &format!("t{at}"));
        let frame = vm.frame(at, vec![]).expect("frame");
        seen.extend(frame.diagnostics.into_iter().map(|d| (at, d)));
        at += 1.0;
    }
    let failures: Vec<&(f64, String)> = seen
        .iter()
        .filter(|(_, d)| d.contains("title binding failed"))
        .collect();
    assert!(
        failures.len() >= 2 && failures.len() <= 3,
        "one report per interval, not per frame: {failures:?}"
    );
    assert!(
        failures[1].1.contains("repeated") && failures[1].0 - failures[0].0 >= REPORT_INTERVAL,
        "the second report counts what it held back: {failures:?}"
    );
}

// Loading. The stock library and the user's shell files run once at startup under the load budget
// (`ShellLimits::load`), which is far wider than a frame's: a slow or busy machine must not fail the shell's
// start for what a frame could never be allowed.

/// A stock library (`node.lua` and a one-file shell) and a config directory with `files` in it.
struct Loading {
    _dir: tempfile::TempDir,
    stock: PathBuf,
    config: PathBuf,
}

fn loading(shell_init: &str, files: &[(&str, &str)]) -> Loading {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let stock = dir.path().join("lua/gnoblin");
    let config = dir.path().join("config");
    std::fs::create_dir_all(stock.join("shell")).expect("the stock directory");
    std::fs::create_dir_all(&config).expect("the config directory");
    std::fs::copy(lua_dir().join("node.lua"), stock.join("node.lua")).expect("node.lua");
    std::fs::write(stock.join("shell/init.lua"), shell_init).expect("the stock shell");
    for (name, text) in files {
        let path = config.join(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("a user file");
    }
    Loading {
        _dir: dir,
        stock,
        config,
    }
}

impl Loading {
    fn boot(&self, comp: &Rc<RefCell<Compositor>>, limits: ShellLimits) -> ShellVm {
        let link = FakeLink(comp.clone());
        ShellVm::with_limits(Box::new(link), &self.stock, classes(), limits).expect("boot ShellVm")
    }
}

/// Limits whose entry and frame budgets could run nothing of any size, and whose load budget is the shipped one.
fn starved_frames() -> ShellLimits {
    let starved = Limits {
        instructions: 1_000,
        memory_bytes: 64 * 1024 * 1024,
        handler_ticks: 0,
    };
    ShellLimits {
        entry: starved,
        frame: starved,
        ..ShellLimits::default()
    }
}

/// A busy shipped VM with a frame deadline of nothing: the stock shell and `node.lua` are still loaded, because
/// loading is not a frame.
#[test]
fn the_vm_boots_and_loads_under_the_load_budget_when_a_frame_could_run_nothing() {
    let comp = compositor_with_root_and_window();
    let shell = loading(
        "local n = 0\nfor i = 1, 200000 do n = n + i end\nstock_sum = n\n",
        &[("shell.lua", "user_ran = true")],
    );
    let mut vm = shell.boot(&comp, starved_frames());
    let errors = vm
        .load_shell(&shell.stock, Some(&shell.config))
        .expect("the stock shell loads");
    assert!(errors.is_empty(), "{errors:?}");
    // The same work as a frame's entry is refused: this is what the load budget is for.
    let refused = run_error(&mut vm, "local n = 0 for i = 1, 200000 do n = n + i end");
    assert!(refused.contains("instruction budget"), "{refused}");
}

#[test]
fn the_shipped_load_budget_is_wide_and_stays_a_budget() {
    let shipped = ShellLimits::default();
    assert_eq!(shipped.load.handler_ticks, 10_000, "ten seconds of wall clock");
    assert!(
        shipped.load.instructions >= shipped.entry.instructions * 10,
        "the instruction budget is several times a handler's"
    );
    assert!(
        shipped.load.instructions >= shipped.frame.instructions,
        "startup is never narrower than a frame"
    );
    assert_eq!(shipped.entry.handler_ticks, 100, "the handler deadline is unchanged");
    assert_eq!(shipped.frame.handler_ticks, 500, "the frame deadline is unchanged");
}

/// A user file that never ends is stopped inside the load budget (the instruction budget, which counts the
/// same on a busy machine), logged once naming the file, and costs neither the stock shell nor later files.
#[test]
fn a_user_file_that_never_ends_is_stopped_by_the_load_budget_and_logged_once() {
    let shell = loading(
        "stock_loaded = true\n",
        &[
            ("shell/10-fine.lua", "before_loop = true"),
            ("shell/20-loop.lua", "while true do end"),
            ("shell/30-fine.lua", "after_loop = true"),
        ],
    );
    // Test-sized: the instruction budget stops the loop long before the wall clock does.
    let limits = ShellLimits {
        load: Limits {
            instructions: 2_000_000,
            memory_bytes: 64 * 1024 * 1024,
            handler_ticks: 600_000,
        },
        ..ShellLimits::default()
    };
    // Tracing caches each call site's interest per process, so the first capture can miss an event another thread's
    // call registered first. Capture again until it is seen.
    let mut attempts = 0;
    let (mut vm, errors, logged, took) = loop {
        let comp = compositor_with_root_and_window();
        let mut vm = shell.boot(&comp, limits);
        let mut errors = Vec::new();
        let started = std::time::Instant::now();
        let logged = gnoblin_log::event::capture::logged(|| {
            errors = vm
                .load_shell(&shell.stock, Some(&shell.config))
                .expect("the stock shell loads");
        });
        attempts += 1;
        if !logged.is_empty() || attempts >= 50 {
            break (vm, errors, logged, started.elapsed());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(
        took < std::time::Duration::from_secs(30),
        "the loop was stopped by its budget: {took:?}"
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("20-loop.lua") && errors[0].contains("instruction budget"),
        "{errors:?}"
    );
    assert_eq!(logged.len(), 1, "reported once: {logged:?}");
    assert!(
        format!("{} {}", logged[0].message, logged[0].fields_line()).contains("20-loop.lua"),
        "the report names the file: {:?}",
        logged[0]
    );
    for name in ["stock_loaded", "before_loop", "after_loop"] {
        assert_eq!(
            check(&mut vm, name, &format!("return tostring({name})")),
            "true",
            "{name} was set: the rest of the load went on"
        );
    }
}

/// The wall clock is the backstop for what the instruction count cannot see; it stops a file on its own.
#[test]
fn the_wall_clock_stops_a_file_the_instruction_budget_would_not() {
    let shell = loading(
        "stock_loaded = true\n",
        &[
            ("shell/10-loop.lua", "while true do end"),
            ("shell/20-fine.lua", "after_loop = true"),
        ],
    );
    let limits = ShellLimits {
        load: Limits {
            instructions: 1 << 50,
            memory_bytes: 64 * 1024 * 1024,
            handler_ticks: 100,
        },
        ..ShellLimits::default()
    };
    let comp = compositor_with_root_and_window();
    let mut vm = shell.boot(&comp, limits);
    let errors = vm
        .load_shell(&shell.stock, Some(&shell.config))
        .expect("the stock shell loads");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("10-loop.lua") && errors[0].contains("deadline"),
        "{errors:?}"
    );
    assert_eq!(check(&mut vm, "after", "return tostring(after_loop)"), "true");
}

/// The budget is each file's own, and a file's work does not count against the next.
#[test]
fn each_user_file_gets_the_whole_load_budget() {
    let spin = "local n = 0 for i = 1, 1000000 do n = n + i end";
    let shell = loading(
        "stock_loaded = true\n",
        &[
            ("shell/10-a.lua", spin),
            ("shell/20-b.lua", spin),
            ("shell/30-c.lua", spin),
        ],
    );
    // One file costs a few million instructions; three files would not fit one budget of five million.
    let limits = ShellLimits {
        load: Limits {
            instructions: 5_000_000,
            memory_bytes: 64 * 1024 * 1024,
            handler_ticks: 600_000,
        },
        ..ShellLimits::default()
    };
    let comp = compositor_with_root_and_window();
    let mut vm = shell.boot(&comp, limits);
    let errors = vm
        .load_shell(&shell.stock, Some(&shell.config))
        .expect("the stock shell loads");
    assert!(errors.is_empty(), "{errors:?}");
}
