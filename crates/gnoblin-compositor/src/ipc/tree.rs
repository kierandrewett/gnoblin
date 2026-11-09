//! The tree session: the pure state machine behind the `tree` namespace of the
//! node API. It takes request values and returns message values. It owns no
//! socket; slice 5b wires it to one.

use gnoblin_render::transaction::{
    ClassInfo, Code, PropertyInfo, TreeApplied, TreeRefusal, TreeTransaction, ValueType, validate_and_apply,
};
use gnoblin_render::tree_revision::TreeRevisions;
use gnoblin_render::{Actor, Scene};
use gnoblin_schema::{Catalog, Namespace, ObjectType, SchemaError, Type as SchemaType};
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

/// The connection that holds the tree. The server issues one per accepted connection, and
/// a session compares them by value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeHolder(u64);

impl TreeHolder {
    /// The holder for the connection numbered `id`.
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// The shell's tree session over one scene.
#[derive(Debug)]
pub struct TreeSession<C: ClassInfo> {
    scene: Scene,
    classes: C,
    root: NodeId,
    epoch: u32,
    serial: u64,
    /// The nodes each recent serial changed, so a stale base is checked, not refused.
    revisions: TreeRevisions,
    holder: Option<TreeHolder>,
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
            revisions: TreeRevisions::new(),
            holder: None,
        })
    }

    /// The scene the session edits. The unit tests read the scene directly to check the edit.
    #[cfg(test)]
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The serial of the last committed change.
    #[must_use]
    pub const fn serial(&self) -> u64 {
        self.serial
    }

    /// Attach the shell from `holder`. Only the session owner's connection may attach, and
    /// `is_owner` is that connection's classification from the owner grant (`owner_grants`).
    /// Any other is refused `not-privileged`. A second holder while one is live is refused
    /// `shell-already-attached`; the same holder attaching again gets a new epoch.
    pub fn attach(&mut self, holder: TreeHolder, is_owner: bool) -> Result<TreeAttached, TreeRefusal> {
        if !is_owner {
            return Err(refusal(Code::NotPrivileged));
        }
        if self.holder.is_some_and(|live| live != holder) {
            return Err(refusal(Code::ShellAlreadyAttached));
        }
        self.epoch += 1;
        self.holder = Some(holder);
        Ok(TreeAttached {
            epoch: self.epoch,
            root: self.root,
            serial: self.serial,
        })
    }

    /// Release the hold `holder` has, when it is the holder. A detach from any other
    /// connection changes nothing. The server calls this when a connection closes.
    pub fn detach(&mut self, holder: TreeHolder) {
        if self.holder == Some(holder) {
            self.holder = None;
        }
    }

    /// The connection holding the tree, if a shell is attached.
    #[must_use]
    pub const fn holder(&self) -> Option<TreeHolder> {
        self.holder
    }

    /// Apply `transaction` as the shell against the scene. `base` is the serial the shell
    /// built it against. A stale base is refused only when the transaction names a node
    /// changed since then (see [`TreeRevisions::check`]). The batch is all-or-nothing: a
    /// refusal leaves the scene and serial untouched. A committed batch that changed
    /// something advances the serial by one.
    pub fn transact(&mut self, base: u64, transaction: &TreeTransaction) -> TreeReply {
        if self.holder.is_none() {
            return self.refuse(Code::NotPrivileged);
        }
        if let Err(refused) = self.revisions.check(base, self.serial, &[], transaction) {
            return TreeReply {
                serial: self.serial,
                result: Err(refused),
            };
        }
        match validate_and_apply(&mut self.scene, &self.classes, Actor::Shell, transaction) {
            Ok(applied) => {
                if !applied.changes.is_empty() {
                    self.serial += 1;
                    self.revisions.record(self.serial, &applied.changes);
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

/// The schema a connection may list. The owner's connection (`is_owner`) sees the whole
/// catalog; any other gets the public catalog, which leaves `tree/` out, so it cannot name
/// a tree member. The socket's schema listing reads this.
pub fn tree_listing(is_owner: bool) -> Result<Catalog, SchemaError> {
    if is_owner {
        crate::schema::catalog()
    } else {
        crate::schema::public_catalog()
    }
}

/// The transaction kind a schema type maps to, or `None` when the tree cannot hold it.
fn value_type(ty: &SchemaType) -> Option<ValueType> {
    Some(match ty {
        SchemaType::Bool => ValueType::Bool,
        SchemaType::Int => ValueType::Int,
        SchemaType::Uint => ValueType::Uint,
        SchemaType::Double => ValueType::Double,
        SchemaType::Text => ValueType::Text,
        SchemaType::Bytes => ValueType::Bytes,
        SchemaType::ObjectRef(class) => ValueType::Node(class.clone()),
        SchemaType::List(inner) => match inner.as_ref() {
            SchemaType::Double => ValueType::ListDouble,
            SchemaType::Text => ValueType::ListText,
            _ => return None,
        },
    })
}

/// The schema facts for the `tree` namespace, read from the catalog. The shell hands this
/// to the transaction layer as its [`ClassInfo`].
#[derive(Debug)]
pub struct TreeClasses {
    catalog: Catalog,
    namespace: Namespace,
}

impl TreeClasses {
    /// Read the `tree` namespace of `catalog`. Every property of every class must have a
    /// [`ValueType`]; `Err` names the first one that does not, and `Err` also covers an
    /// invalid namespace name.
    pub fn new(catalog: Catalog) -> Result<Self, String> {
        let classes = Self {
            catalog,
            namespace: Namespace::parse("tree")?,
        };
        for object in classes.catalog.objects(&classes.namespace) {
            if !classes.is_a(&object.name, "node") {
                continue;
            }
            for property in &object.properties {
                if value_type(&property.ty).is_none() {
                    return Err(format!(
                        "{}.{} has schema type {:?}, which the tree cannot hold",
                        object.name, property.name, property.ty
                    ));
                }
            }
        }
        Ok(classes)
    }

    /// The object type named `class` in the tree namespace. Its members already include
    /// every inherited member, so the lookup never walks the chain.
    fn object(&self, class: &str) -> Option<&ObjectType> {
        self.catalog
            .objects(&self.namespace)
            .find(|object| object.name == class)
    }
}

impl ClassInfo for TreeClasses {
    fn has_class(&self, class: &str) -> bool {
        self.object(class).is_some()
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        let property = self
            .object(class)?
            .properties
            .iter()
            .find(|property| property.name == key)?;
        Some(PropertyInfo {
            kind: value_type(&property.ty)?,
            readonly: property.readonly,
            animated: property.animated,
        })
    }

    fn is_a(&self, class: &str, ancestor: &str) -> bool {
        // The chain is bounded by the object count, so a cyclic schema cannot loop here.
        let mut current = Some(class);
        for _ in 0..=self.catalog.objects(&self.namespace).count() {
            match current {
                Some(name) if name == ancestor => return true,
                Some(name) => current = self.object(name).and_then(|object| object.base.as_deref()),
                None => return false,
            }
        }
        false
    }

    fn has_method(&self, class: &str, method: &str) -> bool {
        self.object(class)
            .is_some_and(|object| object.methods.iter().any(|candidate| candidate.name == method))
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
    use gnoblin_schema::DefaultValue;
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

    const HOLDER: TreeHolder = TreeHolder::new(1);
    const OTHER_HOLDER: TreeHolder = TreeHolder::new(2);

    /// A session whose scene holds one compositor-owned root, attached as privileged.
    fn attached_session() -> TreeSession<Table> {
        let mut scene = Scene::new();
        scene
            .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("root inserts");
        scene.set_root(Some(ROOT)).expect("root names");
        let mut session = TreeSession::new(scene, Table, ROOT).expect("root is live");
        session.attach(HOLDER, true).expect("privileged attach");
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
        let refused = fresh.attach(HOLDER, false).expect_err("unprivileged attach refused");
        assert_eq!(refused.code, Code::NotPrivileged);
        assert_eq!(refused.failed_op, -1);
        // A refused attach leaves the session unattached: a transaction is still refused.
        let reply = fresh.transact(0, &TreeTransaction::new(vec![create(CHILD)]));
        assert_eq!(reply.result.expect_err("unattached").code, Code::NotPrivileged);
        assert!(session.attach(HOLDER, true).is_ok());
    }

    #[test]
    fn attach_answers_with_epoch_root_and_serial_and_each_attach_raises_the_epoch() {
        let mut session = attached_session();
        let first = session.attach(HOLDER, true).expect("attach");
        let second = session.attach(HOLDER, true).expect("re-attach");
        assert_eq!(first.root, ROOT);
        assert_eq!(first.serial, 0);
        assert_eq!(second.epoch, first.epoch + 1);
    }

    #[test]
    fn a_second_holder_is_refused_while_the_first_holds_the_tree() {
        let mut session = attached_session();
        let refused = session
            .attach(OTHER_HOLDER, true)
            .expect_err("a second holder is refused");
        assert_eq!(refused.code, Code::ShellAlreadyAttached);
        assert_eq!(session.holder(), Some(HOLDER));
    }

    #[test]
    fn detach_by_the_holder_frees_the_tree_for_the_next_attach() {
        let mut session = attached_session();
        session.detach(OTHER_HOLDER);
        assert_eq!(session.holder(), Some(HOLDER), "only the holder can release");
        session.detach(HOLDER);
        assert_eq!(session.holder(), None);
        let next = session.attach(OTHER_HOLDER, true).expect("the freed tree attaches");
        assert_eq!(next.epoch, 2);
        assert_eq!(session.holder(), Some(OTHER_HOLDER));
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

    #[test]
    fn node_class_matches_schema() {
        let classes = TreeClasses::new(crate::schema::catalog().expect("schema")).expect("tree namespace");
        assert!(classes.has_class("node"));
        assert!(!classes.has_class("no-such-class"));
        let node = classes.object("node").expect("node class");

        let defaults = [
            ("id", None),
            ("parent", None),
            ("name", Some(DefaultValue::Text(String::new()))),
            ("x", Some(DefaultValue::Double(0.0))),
            ("y", Some(DefaultValue::Double(0.0))),
            ("z", Some(DefaultValue::Double(0.0))),
            ("rotation", Some(DefaultValue::List(vec![DefaultValue::Double(0.0); 3]))),
            ("scale", Some(DefaultValue::List(vec![DefaultValue::Double(1.0); 3]))),
            ("width", Some(DefaultValue::Double(0.0))),
            ("height", Some(DefaultValue::Double(0.0))),
            ("opacity", Some(DefaultValue::Double(1.0))),
            ("visible", Some(DefaultValue::Bool(true))),
            ("clip", Some(DefaultValue::Bool(false))),
            ("input", Some(DefaultValue::Bool(true))),
            ("secure", Some(DefaultValue::Bool(false))),
            ("tags", Some(DefaultValue::List(Vec::new()))),
        ];
        assert_eq!(node.properties.len(), defaults.len());
        for (key, default) in defaults {
            let property = node.properties.iter().find(|property| property.name == key).expect(key);
            assert_eq!(property.default, default, "{key}");
        }

        let readonly = ["id", "parent"];
        let animated = ["x", "y", "z", "rotation", "scale", "width", "height", "opacity"];
        for property in &node.properties {
            assert_eq!(
                property.readonly,
                readonly.contains(&property.name.as_str()),
                "{}",
                property.name
            );
            assert_eq!(
                property.animated,
                animated.contains(&property.name.as_str()),
                "{}",
                property.name
            );
        }

        assert_eq!(
            classes.property("node", "x"),
            Some(PropertyInfo {
                kind: ValueType::Double,
                readonly: false,
                animated: true
            })
        );
        assert_eq!(
            classes.property("node", "id"),
            Some(PropertyInfo {
                kind: ValueType::Uint,
                readonly: true,
                animated: false
            })
        );
        assert_eq!(
            classes.property("node", "visible"),
            Some(PropertyInfo {
                kind: ValueType::Bool,
                readonly: false,
                animated: false
            })
        );
        // Every schema type of a class property has a kind.
        assert_eq!(
            classes.property("node", "name"),
            Some(PropertyInfo {
                kind: ValueType::Text,
                readonly: false,
                animated: false
            })
        );
        assert_eq!(
            classes.property("node", "parent"),
            Some(PropertyInfo {
                kind: ValueType::Node("node".to_owned()),
                readonly: true,
                animated: false
            })
        );
        assert_eq!(
            classes.property("node", "tags"),
            Some(PropertyInfo {
                kind: ValueType::ListText,
                readonly: false,
                animated: false
            })
        );
        assert_eq!(classes.property("node", "no-such-property"), None);
    }

    #[test]
    fn tree_listing_hides_tree_from_peers() {
        let tree = Namespace::parse("tree").expect("namespace");
        let owner = tree_listing(true).expect("owner catalog");
        assert!(
            owner.objects(&tree).any(|object| object.name == "node"),
            "the owner must see the tree classes"
        );
        let peer = tree_listing(false).expect("peer catalog");
        assert_eq!(
            peer.objects(&tree).count(),
            0,
            "a connection that is not the session owner must not list tree members"
        );
    }

    /// Every property of every tree class in the real catalog maps to a [`ValueType`], so a
    /// `set` can name any of them, and the mapped kind and flags match the schema.
    #[test]
    fn every_tree_property_has_a_property_info() {
        let classes = TreeClasses::new(crate::schema::catalog().expect("the schema catalog loads"))
            .expect("the tree namespace is valid");
        let mut checked = 0;
        for object in classes.catalog.objects(&classes.namespace) {
            if !classes.is_a(&object.name, "node") {
                continue;
            }
            for property in &object.properties {
                let info = classes.property(&object.name, &property.name).unwrap_or_else(|| {
                    panic!(
                        "{}.{} ({:?}) has no PropertyInfo",
                        object.name, property.name, property.ty
                    )
                });
                assert_eq!(info.readonly, property.readonly, "{}.{}", object.name, property.name);
                assert_eq!(info.animated, property.animated, "{}.{}", object.name, property.name);
                checked += 1;
            }
        }
        assert!(checked > 0, "the tree namespace has class properties");
    }

    /// The default value of each kind, for a `set` that names a property of that kind.
    fn default_value(kind: &ValueType) -> TreeValue {
        match kind {
            ValueType::Bool => TreeValue::Bool(false),
            ValueType::Int => TreeValue::Int(0),
            ValueType::Uint => TreeValue::Uint(0),
            ValueType::Double => TreeValue::Double(1.0),
            ValueType::Text => TreeValue::Text(String::new()),
            ValueType::Bytes => TreeValue::Bytes(Vec::new()),
            ValueType::Node(_) => TreeValue::Node(None),
            ValueType::ListDouble => TreeValue::ListDouble(vec![1.0; 3]),
            ValueType::ListText => TreeValue::ListText(Vec::new()),
        }
    }

    /// Every property of every node class in the real catalog accepts a value of its own kind
    /// through `set`, and a readonly property refuses it as `read-only`. Each class is created
    /// under the root, then its properties are set one transaction at a time.
    #[test]
    fn every_tree_property_accepts_a_value_of_its_own_kind() {
        let classes = TreeClasses::new(crate::schema::catalog().expect("the schema catalog loads"))
            .expect("the tree namespace is valid");
        let mut scene = Scene::new();
        scene
            .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("root inserts");
        scene.set_root(Some(ROOT)).expect("root names");
        let owner = TreeClasses::new(crate::schema::catalog().expect("the schema catalog loads"))
            .expect("the tree namespace is valid");
        let mut session = TreeSession::new(scene, owner, ROOT).expect("root is live");
        session.attach(true).expect("privileged attach");
        let mut next = 0x1000_u64;
        let mut checked = 0;
        for object in classes.catalog.objects(&classes.namespace) {
            if !classes.is_a(&object.name, "node") {
                continue;
            }
            let node = NodeId::new(next);
            next += 1;
            let base = session.serial();
            let create = TreeTransaction::new(vec![
                TreeOp::New {
                    class: object.name.clone(),
                    id: node,
                },
                TreeOp::Insert {
                    node,
                    parent: ROOT,
                    index: None,
                },
            ]);
            let created = session.transact(base, &create).result;
            assert!(created.is_ok(), "{} new: {created:?}", object.name);
            for property in &object.properties {
                let info = classes
                    .property(&object.name, &property.name)
                    .unwrap_or_else(|| panic!("{}.{} has no PropertyInfo", object.name, property.name));
                let base = session.serial();
                let set = TreeTransaction::new(vec![TreeOp::Set {
                    node,
                    key: property.name.clone(),
                    value: default_value(&info.kind),
                    timing: None,
                }]);
                let result = session.transact(base, &set).result;
                if info.readonly {
                    let refusal = result.expect_err("a readonly property refuses set");
                    assert_eq!(refusal.code, Code::ReadOnly, "{}.{}", object.name, property.name);
                } else {
                    assert!(
                        result.is_ok(),
                        "{}.{} ({:?}) refused its own kind: {result:?}",
                        object.name,
                        property.name,
                        info.kind
                    );
                }
                checked += 1;
            }
        }
        assert!(checked > 0, "the tree namespace has class properties");
    }

    /// A catalog built from one inline schema file, for cases the real catalog does not show.
    fn synthetic_catalog(source: &str) -> Catalog {
        let schema = gnoblin_schema::parse_schema("synthetic.toml", source).expect("the schema parses");
        Catalog::new(vec![schema]).expect("the catalog builds")
    }

    /// A namespace function is a method the host installs without a receiver. `has_method`
    /// reports it like any other method, so a call to it is not refused as unknown.
    #[test]
    fn has_method_sees_namespace_functions() {
        let classes = TreeClasses::new(synthetic_catalog(
            r#"
namespace = "tree"
version = "0.2.0"

[[object]]
name = "node"
since = "0.2.0"

[[object.method]]
name = "ping"
since = "0.2.0"
function = true
"#,
        ))
        .expect("the tree namespace is valid");
        assert!(classes.has_method("node", "ping"), "a namespace function is a method");
        assert!(!classes.has_method("node", "missing"), "an unknown method is not one");
    }

    /// A property whose schema type has no value kind fails the catalog load, so the gap
    /// cannot reach a `set` at run time.
    #[test]
    fn unmapped_schema_type_is_a_construction_error() {
        let error = TreeClasses::new(synthetic_catalog(
            r#"
namespace = "tree"
version = "0.2.0"

[[object]]
name = "node"
since = "0.2.0"

[[object.property]]
name = "nums"
type = "list<uint>"
since = "0.2.0"
"#,
        ))
        .expect_err("list<uint> has no value kind");
        assert!(error.contains("node.nums"), "the error names the property: {error}");
    }
}
