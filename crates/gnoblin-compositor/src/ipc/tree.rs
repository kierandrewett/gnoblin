//! The tree session: the pure state machine behind the `tree` namespace of the
//! node API. It takes request values and returns message values. It owns no
//! socket; slice 5b wires it to one.

use gnoblin_render::Scene;
use gnoblin_render::transaction::{ClassInfo, Code, PropertyInfo, TreeApplied, TreeRefusal, TreeValue, ValueType};
use gnoblin_schema::{Catalog, DefaultValue, Namespace, ObjectType, SchemaError, Type as SchemaType};
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
    /// The root node the shell places its windows under, or `None` when the scene has none.
    pub root: Option<NodeId>,
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
#[derive(Clone, Debug, PartialEq)]
pub struct TreeSnapshotNode {
    /// The node's id.
    pub id: NodeId,
    /// The parent, or `None` for a root node.
    pub parent: Option<NodeId>,
    /// The node's index among its siblings.
    pub index: usize,
    /// The node's class, or `None` for a node the compositor created without one.
    pub class: Option<String>,
    /// The node's title, or empty when it has none. Only a window carries one.
    pub title: String,
    /// The node's app id, or empty when it has none. Only a window carries one.
    pub app_id: String,
    /// The node's property values that differ from their schema defaults, by schema name. A
    /// value with no schema default is always listed.
    pub props: Vec<(String, TreeValue)>,
}

/// A snapshot of the whole tree, split into chunks no larger than [`SNAPSHOT_CHUNK_BYTES`].
#[derive(Clone, Debug, PartialEq)]
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

    /// The connection number this holder was made from.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The shell's attachment: which connection holds the tree, and the epoch of that hold. The
/// scene and its serial belong to the frame that applies transactions, so this owns only the
/// claim.
#[derive(Debug, Default)]
pub struct TreeSession {
    epoch: u32,
    holder: Option<TreeHolder>,
}

/// What an attach grants: the epoch of the new hold. The serial is read from the frame, so the
/// host adds it to make a [`TreeAttached`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeAttachment {
    /// The attachment's epoch. Each attach raises it, so an older one is stale.
    pub epoch: u32,
}

impl TreeSession {
    /// Attach the shell from `holder`. Only the session owner's connection may attach, and
    /// `is_owner` is that connection's classification from the owner grant (`owner_grants`).
    /// Any other is refused `not-owner`. A second holder while one is live is refused
    /// `shell-already-attached`; the same holder attaching again gets a new epoch.
    pub fn attach(&mut self, holder: TreeHolder, is_owner: bool) -> Result<TreeAttachment, TreeRefusal> {
        if !is_owner {
            return Err(refusal(Code::NotOwner));
        }
        if self.holder.is_some_and(|live| live != holder) {
            return Err(refusal(Code::ShellAlreadyAttached));
        }
        self.epoch += 1;
        self.holder = Some(holder);
        Ok(TreeAttachment { epoch: self.epoch })
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
}

/// Snapshot every node of `scene` at `serial`, in paint order, split into chunks. The frame
/// calls this with its own scene, so the snapshot and the serial always agree. `classes`
/// decides which values differ from their schema defaults.
#[must_use]
pub fn snapshot_of(scene: &Scene, classes: &TreeClasses, serial: u64) -> TreeSnapshot {
    let mut nodes = Vec::new();
    for (index, &root) in scene.root_ids().iter().enumerate() {
        walk(scene, classes, root, None, index, &mut nodes);
    }
    TreeSnapshot {
        serial,
        chunks: chunk_nodes(nodes, SNAPSHOT_CHUNK_BYTES),
    }
}

fn walk(
    scene: &Scene,
    classes: &TreeClasses,
    id: NodeId,
    parent: Option<NodeId>,
    index: usize,
    out: &mut Vec<TreeSnapshotNode>,
) {
    let class = scene.class(id).map(str::to_owned);
    out.push(TreeSnapshotNode {
        id,
        parent,
        index,
        props: props_of(scene, classes, id, class.as_deref()),
        class,
        title: scene.text(id, "title").unwrap_or_default().to_owned(),
        app_id: scene.text(id, "app_id").unwrap_or_default().to_owned(),
    });
    if let Some(node) = scene.node(id) {
        for (child_index, &child) in node.children().iter().enumerate() {
            walk(scene, classes, child, Some(id), child_index, out);
        }
    }
}

/// The values of `id` that differ from their schema defaults, in name order. A value with no
/// schema default is always listed.
fn props_of(scene: &Scene, classes: &TreeClasses, id: NodeId, class: Option<&str>) -> Vec<(String, TreeValue)> {
    scene
        .values(id)
        .filter(
            |(key, value)| match class.and_then(|class| classes.default_of(class, key)) {
                Some(default) => &default != *value,
                None => true,
            },
        )
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect()
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
            SchemaType::ObjectRef(class) if class == "effect" => ValueType::Effects,
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

    /// The default of property `key` of `class`, as a value of its kind, or `None` when the
    /// schema gives it no default or the default does not convert.
    fn default_of(&self, class: &str, key: &str) -> Option<TreeValue> {
        let property = self
            .object(class)?
            .properties
            .iter()
            .find(|property| property.name == key)?;
        tree_default(&value_type(&property.ty)?, property.default.as_ref()?)
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

/// A schema default as the value a tree property of kind `kind` holds, or `None` when the two
/// do not match.
fn tree_default(kind: &ValueType, default: &DefaultValue) -> Option<TreeValue> {
    Some(match (kind, default) {
        (ValueType::Bool, DefaultValue::Bool(value)) => TreeValue::Bool(*value),
        (ValueType::Int, DefaultValue::Int(value)) => TreeValue::Int(*value),
        (ValueType::Uint, DefaultValue::Uint(value)) => TreeValue::Uint(*value),
        (ValueType::Double, DefaultValue::Double(value)) => TreeValue::Double(*value),
        (ValueType::Text, DefaultValue::Text(value)) => TreeValue::Text(value.clone()),
        (ValueType::ListDouble, DefaultValue::List(items)) => TreeValue::ListDouble(
            items
                .iter()
                .map(|item| match item {
                    DefaultValue::Double(value) => Some(*value),
                    _ => None,
                })
                .collect::<Option<Vec<f64>>>()?,
        ),
        (ValueType::ListText, DefaultValue::List(items)) => TreeValue::ListText(
            items
                .iter()
                .map(|item| match item {
                    DefaultValue::Text(value) => Some(value.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<String>>>()?,
        ),
        _ => return None,
    })
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
        let size = SNAPSHOT_NODE_BYTES
            + node.class.as_ref().map_or(0, String::len)
            + node.title.len()
            + node.app_id.len()
            + node
                .props
                .iter()
                .map(|(key, value)| key.len() + value_bytes(value))
                .sum::<usize>();
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

/// The estimated encoded size of one value's payload.
fn value_bytes(value: &TreeValue) -> usize {
    match value {
        TreeValue::Text(text) => text.len(),
        TreeValue::Bytes(bytes) => bytes.len(),
        TreeValue::ListDouble(items) => items.len() * 8,
        TreeValue::ListText(items) => items.iter().map(|item| item.len() + 8).sum(),
        TreeValue::Effects(passes) => std::mem::size_of_val(passes.as_slice()),
        TreeValue::Bool(_) | TreeValue::Int(_) | TreeValue::Uint(_) | TreeValue::Double(_) | TreeValue::Node(_) => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render::transaction::{PropertyInfo, ValueType};
    use gnoblin_schema::DefaultValue;

    const HOLDER: TreeHolder = TreeHolder::new(1);
    const OTHER_HOLDER: TreeHolder = TreeHolder::new(2);

    /// A session attached as the privileged shell.
    fn attached_session() -> TreeSession {
        let mut session = TreeSession::default();
        session.attach(HOLDER, true).expect("privileged attach");
        session
    }

    #[test]
    fn attach_refuses_a_non_owner_peer() {
        let mut session = TreeSession::default();
        let refused = session.attach(HOLDER, false).expect_err("non-owner attach refused");
        assert_eq!(refused.code, Code::NotOwner);
        assert_eq!(refused.failed_op, -1);
        // A refused attach leaves the session unattached, so the next privileged attach is first.
        assert_eq!(session.holder(), None);
        assert_eq!(session.attach(HOLDER, true).expect("privileged attach").epoch, 1);
    }

    #[test]
    fn attach_answers_with_epoch_and_each_attach_raises_the_epoch() {
        let mut session = TreeSession::default();
        let first = session.attach(HOLDER, true).expect("attach");
        let second = session.attach(HOLDER, true).expect("re-attach");
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
    fn chunking_keeps_each_chunk_within_the_budget_and_never_splits_a_node() {
        let nodes: Vec<TreeSnapshotNode> = (0..10)
            .map(|raw| TreeSnapshotNode {
                id: NodeId::new(raw),
                parent: None,
                index: 0,
                class: Some("node".to_owned()),
                title: String::new(),
                app_id: String::new(),
                props: Vec::new(),
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
            title: String::new(),
            app_id: String::new(),
            props: Vec::new(),
        };
        let chunks = chunk_nodes(vec![big.clone(), big], SNAPSHOT_CHUNK_BYTES);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1);
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
            ("effects", Some(DefaultValue::List(Vec::new()))),
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
        assert_eq!(
            classes.property("node", "effects"),
            Some(PropertyInfo {
                kind: ValueType::Effects,
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
