//! The tree session: the pure state machine behind the `tree` namespace of the
//! node API. It takes request values and returns message values. It owns no
//! socket; slice 5b wires it to one.

use gnoblin_render::transaction::{ClassInfo, Code, TreeApplied, TreeRefusal, TreeTransaction, validate_and_apply};
use gnoblin_render::{Actor, Scene};
use gnoblin_types::NodeId;

/// The largest snapshot chunk, in bytes of its estimated encoded size.
pub const SNAPSHOT_CHUNK_BYTES: usize = 1 << 20;

/// Estimated fixed encoded size of one snapshot node: its id, parent, index and length.
const SNAPSHOT_NODE_BYTES: usize = 32;

/// The answer to `attach`: the shell's epoch, the root node and the current serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeAttached {
    /// The attachment's epoch. Each attach raises it, so an older one is stale.
    pub epoch: u32,
    /// The root node the shell places its windows under.
    pub root: NodeId,
    /// The scene serial the attachment starts from.
    pub serial: u64,
}

/// The answer to `transact`: the serial after it, and what it changed or why it was refused.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeReply {
    /// The serial after this transaction. It is unchanged when the transaction was refused
    /// or changed nothing.
    pub serial: u64,
    /// The applied changes, in op order, or the refusal.
    pub result: Result<TreeApplied, TreeRefusal>,
}

/// One node in a snapshot, with its place in the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeSnapshotNode {
    /// The node's id.
    pub id: NodeId,
    /// The parent, or `None` for a root node.
    pub parent: Option<NodeId>,
    /// The node's index among its siblings.
    pub index: usize,
    /// The node's class, or `None` for a node the compositor created without one.
    pub class: Option<String>,
}

/// A snapshot of the whole tree, split into chunks no larger than [`SNAPSHOT_CHUNK_BYTES`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeSnapshot {
    /// The serial the snapshot was taken at.
    pub serial: u64,
    /// The nodes in paint order, parents before children, split into chunks.
    pub chunks: Vec<Vec<TreeSnapshotNode>>,
}

/// The shell's tree session over one scene.
#[derive(Debug)]
pub struct TreeSession<C: ClassInfo> {
    scene: Scene,
    classes: C,
    root: NodeId,
    epoch: u32,
    serial: u64,
    attached: bool,
}

impl<C: ClassInfo> TreeSession<C> {
    /// Start a session over `scene`, rooted at `root`. `None` when `root` is not in the scene.
    #[must_use]
    pub fn new(scene: Scene, classes: C, root: NodeId) -> Option<Self> {
        scene.contains(root).then_some(Self {
            scene,
            classes,
            root,
            epoch: 0,
            serial: 0,
            attached: false,
        })
    }

    /// The scene the session edits.
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The serial of the last committed change.
    #[must_use]
    pub const fn serial(&self) -> u64 {
        self.serial
    }

    /// Attach the shell. Only a privileged peer may attach; any other is refused
    /// `not-privileged`. Each attach raises the epoch.
    pub fn attach(&mut self, privileged: bool) -> Result<TreeAttached, TreeRefusal> {
        if !privileged {
            return Err(refusal(Code::NotPrivileged));
        }
        self.epoch += 1;
        self.attached = true;
        Ok(TreeAttached {
            epoch: self.epoch,
            root: self.root,
            serial: self.serial,
        })
    }

    /// Apply `transaction` as the shell against the scene. `base` must be the current
    /// serial. The batch is all-or-nothing: a refusal leaves the scene and serial untouched.
    /// A committed batch that changed something advances the serial by one.
    pub fn transact(&mut self, base: u64, transaction: &TreeTransaction) -> TreeReply {
        if !self.attached {
            return self.refuse(Code::NotPrivileged);
        }
        if base != self.serial {
            return self.refuse(Code::StaleBase);
        }
        match validate_and_apply(&mut self.scene, &self.classes, Actor::Shell, transaction) {
            Ok(applied) => {
                if !applied.changes.is_empty() {
                    self.serial += 1;
                }
                TreeReply {
                    serial: self.serial,
                    result: Ok(applied),
                }
            }
            Err(refused) => TreeReply {
                serial: self.serial,
                result: Err(refused),
            },
        }
    }

    /// Snapshot the whole tree at the current serial, in paint order, split into chunks.
    #[must_use]
    pub fn snapshot(&self) -> TreeSnapshot {
        let mut nodes = Vec::new();
        for (index, &root) in self.scene.root_ids().iter().enumerate() {
            self.walk(root, None, index, &mut nodes);
        }
        TreeSnapshot {
            serial: self.serial,
            chunks: chunk_nodes(nodes, SNAPSHOT_CHUNK_BYTES),
        }
    }

    fn walk(&self, id: NodeId, parent: Option<NodeId>, index: usize, out: &mut Vec<TreeSnapshotNode>) {
        out.push(TreeSnapshotNode {
            id,
            parent,
            index,
            class: self.scene.class(id).map(str::to_owned),
        });
        if let Some(node) = self.scene.node(id) {
            for (child_index, &child) in node.children().iter().enumerate() {
                self.walk(child, Some(id), child_index, out);
            }
        }
    }

    fn refuse(&self, code: Code) -> TreeReply {
        TreeReply {
            serial: self.serial,
            result: Err(refusal(code)),
        }
    }
}

/// A batch-level refusal: no op failed, the whole batch was turned away.
const fn refusal(code: Code) -> TreeRefusal {
    TreeRefusal { failed_op: -1, code }
}

/// Split `nodes` into chunks whose estimated encoded size stays within `budget`.
/// A single node larger than `budget` still gets a chunk of its own.
fn chunk_nodes(nodes: Vec<TreeSnapshotNode>, budget: usize) -> Vec<Vec<TreeSnapshotNode>> {
    let mut chunks: Vec<Vec<TreeSnapshotNode>> = Vec::new();
    let mut used = 0;
    for node in nodes {
        let size = SNAPSHOT_NODE_BYTES + node.class.as_ref().map_or(0, String::len);
        match chunks.last_mut() {
            Some(chunk) if !chunk.is_empty() && used + size <= budget => {
                chunk.push(node);
                used += size;
            }
            _ => {
                chunks.push(vec![node]);
                used = size;
            }
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render::NodeSpec;
    use gnoblin_render::transaction::{PropertyInfo, TreeOp, TreeValue, ValueType};
    use gnoblin_types::Rect;

    /// A schema with one class, `node`, holding one animatable double and one read-only double.
    struct Table;

    impl ClassInfo for Table {
        fn has_class(&self, class: &str) -> bool {
            class == "node"
        }

        fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
            if class != "node" {
                return None;
            }
            match key {
                "opacity" => Some(PropertyInfo {
                    kind: ValueType::Double,
                    readonly: false,
                    animated: true,
                }),
                "serial" => Some(PropertyInfo {
                    kind: ValueType::Uint,
                    readonly: true,
                    animated: false,
                }),
                _ => None,
            }
        }

        fn has_method(&self, _class: &str, _method: &str) -> bool {
            false
        }
    }

    const ROOT: NodeId = NodeId::new(1);
    const CHILD: NodeId = NodeId::new(0x100);
    const OTHER: NodeId = NodeId::new(0x101);

    /// A session whose scene holds one compositor-owned root, attached as privileged.
    fn attached_session() -> TreeSession<Table> {
        let mut scene = Scene::new();
        scene
            .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("root inserts");
        scene.set_root(Some(ROOT)).expect("root names");
        let mut session = TreeSession::new(scene, Table, ROOT).expect("root is live");
        session.attach(true).expect("privileged attach");
        session
    }

    fn create(id: NodeId) -> TreeOp {
        TreeOp::New {
            class: "node".to_owned(),
            id,
        }
    }

    fn insert(node: NodeId, parent: NodeId) -> TreeOp {
        TreeOp::Insert {
            node,
            parent,
            index: None,
        }
    }

    fn set_opacity(node: NodeId, value: f64) -> TreeOp {
        TreeOp::Set {
            node,
            key: "opacity".to_owned(),
            value: TreeValue::Double(value),
            timing: None,
        }
    }

    #[test]
    fn attach_refuses_an_unprivileged_peer() {
        let mut session = attached_session();
        let mut scene = Scene::new();
        scene
            .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 1, 1)))
            .expect("root inserts");
        let mut fresh = TreeSession::new(scene, Table, ROOT).expect("root is live");
        let refused = fresh.attach(false).expect_err("unprivileged attach refused");
        assert_eq!(refused.code, Code::NotPrivileged);
        assert_eq!(refused.failed_op, -1);
        // A refused attach leaves the session unattached: a transaction is still refused.
        let reply = fresh.transact(0, &TreeTransaction::new(vec![create(CHILD)]));
        assert_eq!(reply.result.expect_err("unattached").code, Code::NotPrivileged);
        assert!(session.attach(true).is_ok());
    }

    #[test]
    fn attach_answers_with_epoch_root_and_serial_and_each_attach_raises_the_epoch() {
        let mut session = attached_session();
        let first = session.attach(true).expect("attach");
        let second = session.attach(true).expect("re-attach");
        assert_eq!(first.root, ROOT);
        assert_eq!(first.serial, 0);
        assert_eq!(second.epoch, first.epoch + 1);
    }

    #[test]
    fn transact_applies_a_batch_and_advances_the_serial_by_one() {
        let mut session = attached_session();
        let reply = session.transact(0, &TreeTransaction::new(vec![create(CHILD), insert(CHILD, ROOT)]));
        let applied = reply.result.expect("batch applies");
        assert_eq!(applied.changes.len(), 2);
        assert_eq!(reply.serial, 1);
        assert_eq!(session.serial(), 1);
        assert_eq!(session.scene().node(ROOT).expect("root").children(), &[CHILD]);
    }

    #[test]
    fn transact_with_a_stale_base_is_refused_and_changes_nothing() {
        let mut session = attached_session();
        let reply = session.transact(7, &TreeTransaction::new(vec![create(CHILD)]));
        assert_eq!(reply.result.expect_err("stale base").code, Code::StaleBase);
        assert_eq!(reply.serial, 0);
        assert!(!session.scene().contains(CHILD));
    }

    #[test]
    fn a_refused_op_leaves_the_whole_batch_unapplied() {
        let mut session = attached_session();
        let bad = TreeOp::Set {
            node: CHILD,
            key: "no-such-key".to_owned(),
            value: TreeValue::Double(1.0),
            timing: None,
        };
        let reply = session.transact(0, &TreeTransaction::new(vec![create(CHILD), bad]));
        let refused = reply.result.expect_err("second op refused");
        assert_eq!(refused.failed_op, 1);
        assert_eq!(refused.code, Code::UnknownMember);
        assert_eq!(reply.serial, 0);
        assert!(!session.scene().contains(CHILD));
    }

    #[test]
    fn a_read_only_property_is_refused_as_read_only() {
        let mut session = attached_session();
        // The root carries no class, so the class's readonly flag is only reachable on a `node`.
        let write = TreeOp::Set {
            node: CHILD,
            key: "serial".to_owned(),
            value: TreeValue::Uint(3),
            timing: None,
        };
        let reply = session.transact(0, &TreeTransaction::new(vec![create(CHILD), write]));
        let refused = reply.result.expect_err("readonly");
        assert_eq!(refused.code, Code::ReadOnly);
        assert_eq!(refused.failed_op, 1);
    }

    #[test]
    fn a_shell_may_not_destroy_the_root_or_a_compositor_node() {
        let mut session = attached_session();
        let root = TreeOp::Destroy { node: ROOT };
        let reply = session.transact(0, &TreeTransaction::new(vec![root]));
        assert_eq!(reply.result.expect_err("root").code, Code::RootImmutable);
    }

    #[test]
    fn a_transaction_that_changes_nothing_keeps_the_serial() {
        let mut session = attached_session();
        let reply = session.transact(0, &TreeTransaction::new(Vec::new()));
        assert!(reply.result.expect("empty batch").changes.is_empty());
        assert_eq!(reply.serial, 0);
    }

    #[test]
    fn set_applies_and_advances_the_serial() {
        let mut session = attached_session();
        let reply = session.transact(0, &TreeTransaction::new(vec![create(CHILD), set_opacity(CHILD, 0.5)]));
        reply.result.expect("set applies");
        // One applied transaction advances the serial by one. Reading the value back needs
        // gnoblin-animation types the compositor does not depend on, so it is not asserted here.
        assert_eq!(session.serial(), 1);
    }

    #[test]
    fn a_transaction_before_attach_is_refused_as_not_privileged() {
        let mut scene = Scene::new();
        scene
            .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 1, 1)))
            .expect("root inserts");
        let mut session = TreeSession::new(scene, Table, ROOT).expect("root is live");
        let reply = session.transact(0, &TreeTransaction::new(vec![create(CHILD)]));
        assert_eq!(reply.result.expect_err("before attach").code, Code::NotPrivileged);
    }

    #[test]
    fn snapshot_lists_every_node_with_its_parent_and_index_in_paint_order() {
        let mut session = attached_session();
        session
            .transact(
                0,
                &TreeTransaction::new(vec![
                    create(CHILD),
                    insert(CHILD, ROOT),
                    create(OTHER),
                    insert(OTHER, ROOT),
                ]),
            )
            .result
            .expect("batch applies");
        let snapshot = session.snapshot();
        assert_eq!(snapshot.serial, 1);
        let nodes: Vec<&TreeSnapshotNode> = snapshot.chunks.iter().flatten().collect();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].id, ROOT);
        assert_eq!(nodes[0].parent, None);
        assert_eq!(nodes[1].id, CHILD);
        assert_eq!(nodes[1].parent, Some(ROOT));
        assert_eq!(nodes[1].index, 0);
        assert_eq!(nodes[2].id, OTHER);
        assert_eq!(nodes[2].index, 1);
        assert_eq!(nodes[1].class.as_deref(), Some("node"));
    }

    #[test]
    fn chunking_keeps_each_chunk_within_the_budget_and_never_splits_a_node() {
        let nodes: Vec<TreeSnapshotNode> = (0..10)
            .map(|raw| TreeSnapshotNode {
                id: NodeId::new(raw),
                parent: None,
                index: 0,
                class: Some("node".to_owned()),
            })
            .collect();
        let per_node = SNAPSHOT_NODE_BYTES + "node".len();
        let chunks = chunk_nodes(nodes, per_node * 3);
        assert_eq!(chunks.len(), 4);
        assert!(chunks.iter().all(|chunk| chunk.len() <= 3));
        assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 10);
    }

    #[test]
    fn an_oversized_node_still_gets_a_chunk_of_its_own() {
        let big = TreeSnapshotNode {
            id: NodeId::new(9),
            parent: None,
            index: 0,
            class: Some("x".repeat(SNAPSHOT_CHUNK_BYTES + 1)),
        };
        let chunks = chunk_nodes(vec![big.clone(), big], SNAPSHOT_CHUNK_BYTES);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1);
    }

    #[test]
    fn new_refuses_a_root_that_is_not_in_the_scene() {
        let scene = Scene::new();
        assert!(TreeSession::new(scene, Table, ROOT).is_none());
    }
}
