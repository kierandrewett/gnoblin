//! Tests for the runtime role's opt-in shell VM (`GNOBLIN_SHELL_VM`).
//!
//! With the variable unset the role does not touch the shell VM at all. With it set, the
//! role boots a `ShellVm` over a tree link and runs one frame per turn of its loop: a change
//! batch from the compositor is applied in that frame, and the frame sends one transaction
//! when the VM wrote something, none when it did not. A link failure ends the VM and logs why;
//! the role keeps serving.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gnoblin_lua::shell_role::{SHELL_VM_VARIABLE, ShellRole, ShellSlot, shell_vm_dir};
use gnoblin_lua::shell_vm::{ShellVm, ShellVmError};
use gnoblin_lua::tree_core::{ClassSpec, PropSpec, TreeValueKind};
use gnoblin_lua::tree_mirror::{
    Attached, LinkError, LuaChange, LuaIncoming, LuaOp, LuaTransaction, MirrorChangeKind, SnapshotNode, TreeLink,
    TxReply,
};

fn lua_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lua/gnoblin")
}

fn classes() -> Vec<ClassSpec> {
    let node = ClassSpec {
        name: "Node".into(),
        base: None,
        props: vec![PropSpec {
            name: "title".into(),
            kind: TreeValueKind::Text,
            readonly: false,
            default: None,
        }],
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

/// The compositor's side of the link: a node map, the transactions it received, and the
/// messages waiting for the next poll.
#[derive(Default)]
struct Compositor {
    nodes: BTreeMap<u64, (String, u64)>,
    serial: u64,
    sent: Vec<Vec<LuaOp>>,
    queue: Vec<LuaIncoming>,
}

#[derive(Clone)]
struct FakeLink {
    comp: Rc<RefCell<Compositor>>,
    /// When set, `poll` fails the way a closed owner socket does.
    dead: Rc<Cell<bool>>,
}

impl TreeLink for FakeLink {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        let mut c = self.comp.borrow_mut();
        let attached = Attached {
            epoch: 1,
            root: 1,
            serial: c.serial,
        };
        let nodes = c
            .nodes
            .iter()
            .map(|(id, (class, parent))| SnapshotNode {
                id: *id,
                parent: *parent,
                index: 0,
                class: class.clone(),
            })
            .collect();
        c.queue.push(LuaIncoming::Attached(attached));
        let serial = c.serial;
        c.queue.push(LuaIncoming::Snapshot {
            serial,
            chunk: 0,
            last: true,
            nodes,
        });
        Ok(())
    }

    fn transact(&mut self, tx: &LuaTransaction) -> Result<(), LinkError> {
        let mut c = self.comp.borrow_mut();
        c.sent.push(tx.ops.clone());
        let serial = tx.serial;
        c.serial = serial;
        let changes: Vec<LuaChange> = tx
            .ops
            .iter()
            .filter_map(|op| match op {
                LuaOp::New { id, class } => {
                    c.nodes.insert(*id, (class.clone(), 0));
                    Some(change_of(MirrorChangeKind::Created, *id, class))
                }
                LuaOp::Set { node, key, value, .. } => Some(LuaChange {
                    key: key.clone(),
                    value: Some(value.clone()),
                    ..change_of(MirrorChangeKind::Set, *node, "")
                }),
                _ => None,
            })
            .collect();
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
        if self.dead.get() {
            return Err(LinkError("owner socket closed".into()));
        }
        Ok(std::mem::take(&mut self.comp.borrow_mut().queue))
    }
}

fn change_of(kind: MirrorChangeKind, node: u64, class: &str) -> LuaChange {
    LuaChange {
        kind,
        node,
        class: class.into(),
        parent: 0,
        index: 0,
        key: String::new(),
        value: None,
        removed: Vec::new(),
    }
}

/// A compositor with the root (`1`) and one window (`2`), and the link to it.
fn compositor() -> (Rc<RefCell<Compositor>>, FakeLink) {
    let mut c = Compositor::default();
    c.nodes.insert(1, ("Node".into(), 0));
    c.nodes.insert(2, ("Window".into(), 1));
    c.serial = 1;
    let comp = Rc::new(RefCell::new(c));
    let link = FakeLink {
        comp: comp.clone(),
        dead: Rc::new(Cell::new(false)),
    };
    (comp, link)
}

fn boot(link: FakeLink) -> ShellRole {
    let vm = ShellVm::new(Box::new(link), &lua_dir(), classes()).expect("boot the shell VM");
    ShellRole::new(vm)
}

/// The number of transactions the compositor has received so far.
fn sent(comp: &Rc<RefCell<Compositor>>) -> usize {
    comp.borrow().sent.len()
}

/// (a) With the variable unset there is no shell VM to start: the role runs exactly as before.
#[test]
fn unset_variable_wires_no_shell_vm() {
    assert_eq!(shell_vm_dir(&|_| None), None);
    assert_eq!(SHELL_VM_VARIABLE, "GNOBLIN_SHELL_VM");
    let slot = ShellSlot::from_env(&|_| None, |_| -> Result<ShellRole, ShellVmError> {
        panic!("connect must not run when unset")
    });
    assert!(!slot.is_running(), "no shell VM is started without the variable");
}

#[test]
fn set_variable_names_the_directory() {
    let dir = shell_vm_dir(&|name| (name == SHELL_VM_VARIABLE).then(|| "/tmp/shell".to_owned()));
    assert_eq!(dir, Some(PathBuf::from("/tmp/shell")));
}

/// (b) A change batch from the compositor: one transaction when the VM wrote, none when not.
#[test]
fn change_batch_sends_one_transaction_only_when_the_vm_wrote() {
    let (comp, link) = compositor();
    let mut role = boot(link);
    role.tick(0.0).expect("boot frame");
    let before = sent(&comp);

    // The compositor reports a native change; the VM has nothing to write about it.
    comp.borrow_mut().queue.push(LuaIncoming::Changes {
        serial: 1,
        changes: vec![change_of(MirrorChangeKind::Created, 3, "Node")],
    });
    role.tick(1.0).expect("frame on a change batch");
    assert_eq!(sent(&comp), before, "no write, no transaction");

    // Now the VM writes in response to the next change batch.
    role.run("write", r#"local n = Node(); n.title = "shell"; root:add(n)"#)
        .expect("run the write");
    comp.borrow_mut().queue.push(LuaIncoming::Changes {
        serial: 1,
        changes: vec![change_of(MirrorChangeKind::Created, 4, "Node")],
    });
    role.tick(2.0).expect("frame on a change batch");
    assert_eq!(sent(&comp), before + 1, "exactly one transaction for the write");
}

/// (c) A link error ends the VM with a named message; the slot is empty and the role serves on.
#[test]
fn a_link_error_ends_the_vm_and_the_role_stays_alive() {
    let (_comp, link) = compositor();
    let dead = link.dead.clone();
    let mut slot = ShellSlot::running(boot(link));
    slot.tick(0.0).expect("boot frame");
    assert!(slot.is_running());

    dead.set(true);
    let ended = slot.tick(1.0).expect_err("the link error ends the VM");
    let reason = ended.to_string();
    assert!(reason.contains("shell vm link"), "names the VM and the link: {reason}");
    assert!(
        reason.contains("owner socket closed"),
        "names the link failure: {reason}"
    );
    assert!(!slot.is_running());

    // The role keeps serving: a tick with no VM is a no-op, not an error.
    slot.tick(2.0).expect("the role is still alive without a VM");
}
