//! The tree session: the pure state machine behind the `tree` namespace of the
//! node API. It takes request values and returns message values. It owns no
//! socket; slice 5b wires it to one.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use gnoblin_render::Scene;
use gnoblin_render::transaction::{
    ClassInfo, Code, OBSERVER_CONTROLLER_KEYS, PropertyInfo, TreeApplied, TreeOp, TreeRefusal, TreeValue, ValueType,
};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

/// How a connection attaches to the tree (`tree.attach`, `mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachMode {
    /// The shell: it holds the tree, and only one connection at a time may.
    Holder,
    /// A reader beside the shell. It hears the snapshot and every change, never holds the tree, and may only make
    /// and destroy its own seats and controllers, write those controllers' values, and send pointer input
    /// (design/node-core.md, F2b; design/js-xr.md).
    Observer,
}

impl AttachMode {
    /// The mode the wire's `mode` text names: empty or `holder`, or `observer`. Anything else is `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "" | "holder" => Some(Self::Holder),
            "observer" => Some(Self::Observer),
            _ => None,
        }
    }
}

/// The tree's attachments: which connection holds the tree, which observe it, and the seats and controllers each
/// observer made. The scene and its serial belong to the frame that applies transactions, so this owns only the claims.
#[derive(Debug, Default)]
pub struct TreeSession {
    epoch: u32,
    holder: Option<TreeHolder>,
    /// Each observing connection, with the seat and controller nodes it made and so may destroy.
    observers: BTreeMap<TreeHolder, BTreeSet<NodeId>>,
    /// How fast each observer may write the head camera.
    limiters: BTreeMap<TreeHolder, PoseLimiter>,
    /// How fast each observer may write its controllers, apart from the head so that one does not starve the other.
    controller_limiters: BTreeMap<TreeHolder, PoseLimiter>,
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
    /// `is_owner` is whether that connection is the session owner's (`ipc::privileged`).
    /// Any other is refused `not-owner`. A second holder while one is live is refused
    /// `shell-already-attached`; the same holder attaching again gets a new epoch. A connection
    /// that observed becomes the holder and stops observing.
    pub fn attach(&mut self, holder: TreeHolder, is_owner: bool) -> Result<TreeAttachment, TreeRefusal> {
        if !is_owner {
            return Err(refusal(Code::NotOwner));
        }
        if self.holder.is_some_and(|live| live != holder) {
            return Err(refusal(Code::ShellAlreadyAttached));
        }
        self.observers.remove(&holder);
        self.epoch += 1;
        self.holder = Some(holder);
        Ok(TreeAttachment { epoch: self.epoch })
    }

    /// Attach `observer` as a reader. Any number may, beside a holder or none, and only the session owner's
    /// connection may (`not-owner` otherwise). The connection that holds the tree cannot also observe it
    /// (`shell-already-attached`). Attaching again keeps the nodes the observer made and raises its epoch.
    pub fn attach_observer(&mut self, observer: TreeHolder, is_owner: bool) -> Result<TreeAttachment, TreeRefusal> {
        if !is_owner {
            return Err(refusal(Code::NotOwner));
        }
        if self.holder == Some(observer) {
            return Err(refusal(Code::ShellAlreadyAttached));
        }
        self.epoch += 1;
        self.observers.entry(observer).or_default();
        Ok(TreeAttachment { epoch: self.epoch })
    }

    /// Release the hold `holder` has, when it is the holder. A detach from any other
    /// connection changes nothing. The server calls this when a connection closes.
    pub fn detach(&mut self, holder: TreeHolder) {
        if self.holder == Some(holder) {
            self.holder = None;
        }
    }

    /// Ends the observation of `observer` and returns the seats and controllers it made, which the caller destroys.
    /// `None` when the connection was not observing.
    pub fn release_observer(&mut self, observer: TreeHolder) -> Option<BTreeSet<NodeId>> {
        self.limiters.remove(&observer);
        self.controller_limiters.remove(&observer);
        self.observers.remove(&observer)
    }

    /// The connection holding the tree, if a shell is attached.
    #[must_use]
    pub const fn holder(&self) -> Option<TreeHolder> {
        self.holder
    }

    /// Whether `connection` is an observer.
    #[must_use]
    pub fn is_observer(&self, connection: TreeHolder) -> bool {
        self.observers.contains_key(&connection)
    }

    /// Whether anything is attached: a holder or an observer. The compositor announces its own changes only then.
    #[must_use]
    pub fn has_attachments(&self) -> bool {
        self.holder.is_some() || !self.observers.is_empty()
    }

    /// Every observing connection.
    #[must_use]
    pub fn observers(&self) -> Vec<TreeHolder> {
        self.observers.keys().copied().collect()
    }

    /// The seats and controllers `observer` made and may destroy.
    #[must_use]
    pub fn made_by(&self, observer: TreeHolder) -> BTreeSet<NodeId> {
        self.observers.get(&observer).cloned().unwrap_or_default()
    }

    /// Records the nodes an applied observer transaction made and destroyed.
    pub fn record_made(&mut self, observer: TreeHolder, made: &[NodeId], destroyed: &[NodeId]) {
        if let Some(nodes) = self.observers.get_mut(&observer) {
            nodes.extend(made.iter().copied());
            for node in destroyed {
                nodes.remove(node);
            }
        }
    }

    /// Whether `observer` may write the head camera now. Each observer has its own allowance ([`PoseLimiter`]); one
    /// that is not attached has none.
    pub fn admit_pose(&mut self, observer: TreeHolder, now: Instant) -> bool {
        self.observers.contains_key(&observer) && self.limiters.entry(observer).or_default().admit(now)
    }

    /// Whether `observer` may write its controllers now: an allowance of its own, in the same terms as the head's.
    pub fn admit_controllers(&mut self, observer: TreeHolder, now: Instant) -> bool {
        self.observers.contains_key(&observer) && self.controller_limiters.entry(observer).or_default().admit(now)
    }
}

/// The most head-camera writes an observer may sustain, per second. A headset reports its pose once per frame, at 90
/// or 120 Hz at most, and the extension writes at most 60 a second: the cap leaves room for both and for jitter, and
/// stops a client that writes as fast as the socket takes.
pub const POSE_RATE_HZ: u32 = 120;

/// How many writes an observer may make at once before the rate applies: the slack that lets a late frame catch up.
pub const POSE_BURST: u32 = 4;

/// A token bucket for one observer's head-camera writes: [`POSE_BURST`] tokens, refilled at [`POSE_RATE_HZ`] a second.
#[derive(Clone, Copy, Debug)]
pub struct PoseLimiter {
    tokens: f64,
    at: Option<Instant>,
}

impl Default for PoseLimiter {
    fn default() -> Self {
        Self {
            tokens: f64::from(POSE_BURST),
            at: None,
        }
    }
}

impl PoseLimiter {
    /// Takes one write's token at `now`, or refuses it. A `now` before the last write (the clock stepped back) adds
    /// no tokens.
    pub fn admit(&mut self, now: Instant) -> bool {
        let elapsed = self.at.map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        self.at = Some(self.at.map_or(now, |at| at.max(now)));
        self.tokens = elapsed
            .as_secs_f64()
            .mul_add(f64::from(POSE_RATE_HZ), self.tokens)
            .min(f64::from(POSE_BURST));
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The properties of the camera named `head` an observer may set: the head pose and the runtime's layer limit. The
/// frame holds them to that camera (`gnoblin_render::transaction`); this is the check that needs no scene.
const OBSERVER_CAMERA_KEYS: [&str; 5] = ["x", "y", "z", "rotation", "layer_budget"];

/// The nodes a transaction's `new` ops make, in no particular order.
fn made_in(ops: &[TreeOp]) -> BTreeSet<NodeId> {
    ops.iter()
        .filter_map(|op| match op {
            TreeOp::New { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

/// The index of the first op in `ops` that writes the head camera, if any: a pose or budget key on a node the
/// observer did not make (`owned`, or made by this transaction), which the frame then holds to the camera named `head`.
/// Such a transaction spends the observer's [`PoseLimiter`] allowance.
#[must_use]
pub fn writes_head_camera(ops: &[TreeOp], owned: &BTreeSet<NodeId>) -> Option<usize> {
    let made = made_in(ops);
    ops.iter().position(|op| {
        matches!(op, TreeOp::Set { node, key, .. }
            if OBSERVER_CAMERA_KEYS.contains(&key.as_str()) && !owned.contains(node) && !made.contains(node))
    })
}

/// The index of the first op in `ops` that writes a value of a node the observer made (`owned`, or made by this
/// transaction): its controllers. Such a transaction spends the observer's controller allowance.
#[must_use]
pub fn writes_controller(ops: &[TreeOp], owned: &BTreeSet<NodeId>) -> Option<usize> {
    let made = made_in(ops);
    ops.iter().position(|op| {
        matches!(op, TreeOp::Set { node, key, .. }
            if OBSERVER_CONTROLLER_KEYS.contains(&key.as_str()) && (owned.contains(node) || made.contains(node)))
    })
}

/// What an observer's transaction makes and destroys, or the index of the first op an observer may not send.
///
/// An observer may send a `new` of class `seat` or `controller`, a `set` of a seat's `name` in the same transaction, a
/// `set` of the pose and values of a controller it made (`owned`, or made in this transaction; never its `claimed`
/// list, which is the shell's), a `destroy` of a node it made, an `input` on a seat it made, a `lock` call, and a
/// `set` of the head camera's pose and `layer_budget` (the frame checks that the node is that camera). Everything else
/// is the holder's. The returned nodes are the ones made and destroyed, for the caller to record once the transaction
/// applies.
///
/// # Errors
///
/// The index of the first op the observer may not send; the whole transaction is refused.
pub fn observer_ops(ops: &[TreeOp], owned: &BTreeSet<NodeId>) -> Result<(Vec<NodeId>, Vec<NodeId>), usize> {
    let mut made: Vec<NodeId> = Vec::new();
    let mut destroyed: Vec<NodeId> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        let allowed = match op {
            TreeOp::New { class, id } if class == "seat" || class == "controller" => {
                made.push(*id);
                true
            }
            TreeOp::Set {
                node,
                key,
                timing: None,
                ..
            } => {
                let own = owned.contains(node) || made.contains(node);
                (key == "name" && made.contains(node))
                    || (own && OBSERVER_CONTROLLER_KEYS.contains(&key.as_str()))
                    || OBSERVER_CAMERA_KEYS.contains(&key.as_str())
            }
            TreeOp::Destroy { node } => {
                let own = owned.contains(node) || made.contains(node);
                if own {
                    destroyed.push(*node);
                }
                own
            }
            // Input goes out on a seat the observer made itself, never on the shell's or another observer's.
            TreeOp::Input { seat, .. } => owned.contains(seat) || made.contains(seat),
            // The one call an observer may make: take the session lock, the screensaver's `Lock`. It touches
            // no node the shell owns; the frame runs it only on the lock node. `authenticate` stays the shell's.
            TreeOp::TreeCall { method, .. } => method == "lock",
            _ => false,
        };
        if !allowed {
            return Err(index);
        }
    }
    Ok((made, destroyed))
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
    let mut props: Vec<(String, TreeValue)> = scene
        .values(id)
        .filter(
            |(key, value)| match class.and_then(|class| classes.default_of(class, key)) {
                Some(default) => &default != *value,
                None => true,
            },
        )
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect();
    // A seat's name lives beside the node's values (it is what the wl_seat global is named from), so a snapshot would
    // otherwise never carry it and a client that attaches after the seat was made could not find the seat by name.
    if let Some(name) = scene.seat_name(id)
        && !props.iter().any(|(key, _)| key == "name")
    {
        props.push(("name".to_owned(), TreeValue::Text(name.to_owned())));
        props.sort_by(|a, b| a.0.cmp(&b.0));
    }
    props
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
    fn observers_attach_beside_a_holder_without_taking_the_tree() {
        let mut session = attached_session();
        let first = session
            .attach_observer(OTHER_HOLDER, true)
            .expect("an observer attaches");
        let second = session
            .attach_observer(TreeHolder::new(3), true)
            .expect("any number of observers attach");
        assert_eq!(first.epoch, 2);
        assert_eq!(second.epoch, 3);
        assert_eq!(session.holder(), Some(HOLDER), "an observer never holds the tree");
        assert!(session.is_observer(OTHER_HOLDER));
        assert!(!session.is_observer(HOLDER));
        assert_eq!(session.observers(), vec![OTHER_HOLDER, TreeHolder::new(3)]);
    }

    #[test]
    fn an_observer_attaches_before_any_holder_and_the_holder_is_not_refused() {
        let mut session = TreeSession::default();
        session
            .attach_observer(OTHER_HOLDER, true)
            .expect("an observer attaches");
        assert!(session.has_attachments());
        assert_eq!(session.holder(), None);
        session
            .attach(HOLDER, true)
            .expect("the shell attaches after an observer");
        assert_eq!(session.holder(), Some(HOLDER));
    }

    #[test]
    fn a_non_owner_cannot_observe_and_the_holder_cannot_also_observe() {
        let mut session = attached_session();
        assert_eq!(
            session.attach_observer(OTHER_HOLDER, false).expect_err("refused").code,
            Code::NotOwner
        );
        assert_eq!(
            session.attach_observer(HOLDER, true).expect_err("refused").code,
            Code::ShellAlreadyAttached
        );
        assert!(!session.is_observer(OTHER_HOLDER));
    }

    #[test]
    fn releasing_an_observer_returns_the_nodes_it_made() {
        let mut session = attached_session();
        session.attach_observer(OTHER_HOLDER, true).expect("attach");
        session.record_made(OTHER_HOLDER, &[NodeId::new(7), NodeId::new(8)], &[]);
        session.record_made(OTHER_HOLDER, &[], &[NodeId::new(7)]);
        assert_eq!(session.made_by(OTHER_HOLDER), BTreeSet::from([NodeId::new(8)]));
        assert_eq!(
            session.release_observer(OTHER_HOLDER),
            Some(BTreeSet::from([NodeId::new(8)]))
        );
        assert_eq!(session.release_observer(OTHER_HOLDER), None);
        assert!(!session.has_attachments() || session.holder().is_some());
    }

    #[test]
    fn a_connection_that_observed_and_then_holds_stops_observing() {
        let mut session = TreeSession::default();
        session.attach_observer(HOLDER, true).expect("observe");
        session.attach(HOLDER, true).expect("hold");
        assert!(!session.is_observer(HOLDER));
    }

    fn new_seat(id: u64) -> TreeOp {
        TreeOp::New {
            class: "seat".to_owned(),
            id: NodeId::new(id),
        }
    }

    fn set_name(node: u64, key: &str) -> TreeOp {
        TreeOp::Set {
            node: NodeId::new(node),
            key: key.to_owned(),
            value: TreeValue::Text("xr".to_owned()),
            timing: None,
        }
    }

    #[test]
    fn an_observer_may_make_a_named_seat_destroy_its_own_and_send_input() {
        let owned = BTreeSet::from([NodeId::new(9)]);
        let input = TreeOp::Input {
            node: NodeId::new(1),
            seat: NodeId::new(5),
            input: gnoblin_render::transaction::PointerInput::Leave,
        };
        let ops = [
            new_seat(5),
            set_name(5, "name"),
            input,
            TreeOp::Destroy { node: NodeId::new(9) },
        ];
        let (made, destroyed) = observer_ops(&ops, &owned).expect("every op is allowed");
        assert_eq!(made, vec![NodeId::new(5)]);
        assert_eq!(destroyed, vec![NodeId::new(9)]);
    }

    #[test]
    fn an_observer_is_refused_at_the_first_op_that_is_not_its_own() {
        let owned = BTreeSet::from([NodeId::new(9)]);
        let refused = |ops: Vec<TreeOp>| observer_ops(&ops, &owned).expect_err("refused");
        // Another class, a destroy of a node it did not make, a set of another key or of a node it did not make.
        assert_eq!(
            refused(vec![TreeOp::New {
                class: "node".to_owned(),
                id: NodeId::new(5)
            }]),
            0
        );
        assert_eq!(refused(vec![new_seat(5), TreeOp::Destroy { node: NodeId::new(4) }]), 1);
        assert_eq!(refused(vec![new_seat(5), set_name(5, "focus")]), 1);
        assert_eq!(
            refused(vec![set_name(9, "name")]),
            0,
            "the name is set only when the seat is made"
        );
        // Input on a seat the observer did not make.
        assert_eq!(
            refused(vec![TreeOp::Input {
                node: NodeId::new(1),
                seat: NodeId::new(4),
                input: gnoblin_render::transaction::PointerInput::Leave,
            }]),
            0
        );
        assert_eq!(
            refused(vec![TreeOp::TreeCall {
                node: NodeId::new(9),
                method: "claim".to_owned(),
                args: Vec::new(),
                text: gnoblin_secret::SecretText::default(),
                token: 1,
            }]),
            0
        );
        assert_eq!(
            refused(vec![TreeOp::Insert {
                node: NodeId::new(5),
                parent: NodeId::new(1),
                index: None
            }]),
            0
        );
    }

    fn call(method: &str) -> TreeOp {
        TreeOp::TreeCall {
            node: NodeId::new(2),
            method: method.to_owned(),
            args: Vec::new(),
            text: gnoblin_secret::SecretText::default(),
            token: 1,
        }
    }

    /// An observer (the screensaver service, a compat adapter) may take the lock and may never answer its
    /// password prompt: a `lock` call is its one call, and `authenticate` (which unlocks) is refused at once.
    #[test]
    fn an_observer_may_lock_and_may_not_authenticate() {
        let owned = BTreeSet::new();
        assert_eq!(observer_ops(&[call("lock")], &owned), Ok((Vec::new(), Vec::new())));
        assert_eq!(observer_ops(&[call("authenticate")], &owned), Err(0));
        assert_eq!(observer_ops(&[call("lock"), call("authenticate")], &owned), Err(1));
        assert_eq!(observer_ops(&[call("close")], &owned), Err(0));
    }

    #[test]
    fn an_observer_set_may_not_animate() {
        let animated = TreeOp::Set {
            node: NodeId::new(5),
            key: "name".to_owned(),
            value: TreeValue::Text("xr".to_owned()),
            timing: Some(gnoblin_render::transaction::Timing {
                seconds: 1.0,
                samples: Vec::new(),
            }),
        };
        assert_eq!(observer_ops(&[new_seat(5), animated], &BTreeSet::new()), Err(1));
    }

    fn camera_set(node: u64, key: &str) -> TreeOp {
        TreeOp::Set {
            node: NodeId::new(node),
            key: key.to_owned(),
            value: TreeValue::Double(1.0),
            timing: None,
        }
    }

    #[test]
    fn an_observer_may_write_the_head_cameras_pose_and_layer_budget_keys_and_no_others() {
        let owned = BTreeSet::new();
        for key in ["x", "y", "z", "rotation", "layer_budget"] {
            assert_eq!(
                observer_ops(&[camera_set(3, key)], &owned),
                Ok((Vec::new(), Vec::new())),
                "{key}"
            );
        }
        for key in [
            "scale",
            "opacity",
            "visible",
            "projection",
            "metres",
            "ipd",
            "fov",
            "name",
            "source",
        ] {
            assert_eq!(observer_ops(&[camera_set(3, key)], &owned), Err(0), "{key}");
        }
    }

    #[test]
    fn a_pose_write_is_found_by_its_first_op() {
        let none = BTreeSet::new();
        assert_eq!(writes_head_camera(&[new_seat(5), camera_set(3, "y")], &none), Some(1));
        assert_eq!(writes_head_camera(&[new_seat(5), set_name(5, "name")], &none), None);
        assert_eq!(writes_head_camera(&[], &none), None);
    }

    fn new_controller(id: u64) -> TreeOp {
        TreeOp::New {
            class: "controller".to_owned(),
            id: NodeId::new(id),
        }
    }

    #[test]
    fn an_observer_may_make_a_controller_write_its_values_and_destroy_it() {
        let owned = BTreeSet::from([NodeId::new(9)]);
        let ops = [
            new_controller(5),
            camera_set(5, "x"),
            camera_set(5, "rotation"),
            camera_set(5, "hand"),
            camera_set(5, "trigger"),
            camera_set(9, "grip"),
            camera_set(9, "thumbstick_click"),
            TreeOp::Destroy { node: NodeId::new(9) },
        ];
        let (made, destroyed) = observer_ops(&ops, &owned).expect("every op is allowed");
        assert_eq!(made, vec![NodeId::new(5)]);
        assert_eq!(destroyed, vec![NodeId::new(9)]);
    }

    #[test]
    fn an_observer_may_not_write_the_values_of_a_controller_it_did_not_make_nor_its_claimed_list() {
        let owned = BTreeSet::from([NodeId::new(9)]);
        for key in [
            "hand",
            "tracked",
            "trigger",
            "grip",
            "thumbstick_x",
            "primary",
            "secondary",
        ] {
            assert_eq!(
                observer_ops(&[camera_set(3, key)], &owned),
                Err(0),
                "{key} of a node that is not its own"
            );
        }
        assert_eq!(observer_ops(&[camera_set(9, "claimed")], &owned), Err(0));
        assert_eq!(
            observer_ops(&[new_controller(5), camera_set(5, "claimed")], &owned),
            Err(1)
        );
        assert_eq!(observer_ops(&[camera_set(9, "opacity")], &owned), Err(0));
    }

    #[test]
    fn a_controller_write_does_not_spend_the_head_cameras_allowance() {
        let owned = BTreeSet::from([NodeId::new(9)]);
        let hands = [camera_set(9, "x"), camera_set(9, "rotation"), camera_set(9, "grip")];
        assert_eq!(writes_head_camera(&hands, &owned), None);
        assert_eq!(writes_controller(&hands, &owned), Some(0));
        // A controller that is made and written in one transaction is the same.
        let made = [new_controller(5), camera_set(5, "y")];
        assert_eq!(writes_head_camera(&made, &BTreeSet::new()), None);
        assert_eq!(writes_controller(&made, &BTreeSet::new()), Some(1));
        // The head's pose is the head's, whoever else the observer made.
        let head = [camera_set(3, "x")];
        assert_eq!(writes_head_camera(&head, &owned), Some(0));
        assert_eq!(writes_controller(&head, &owned), None);
    }

    #[test]
    fn the_controllers_have_an_allowance_apart_from_the_head() {
        let mut session = TreeSession::default();
        session.attach_observer(HOLDER, true).expect("observe");
        let t0 = Instant::now();
        assert_eq!(
            (0..20).filter(|_| session.admit_pose(HOLDER, t0)).count(),
            POSE_BURST as usize
        );
        assert_eq!(
            (0..20).filter(|_| session.admit_controllers(HOLDER, t0)).count(),
            POSE_BURST as usize,
            "the head's spent allowance leaves the hands theirs"
        );
        assert!(
            !session.admit_controllers(TreeHolder::new(77), t0),
            "a connection that is not observing has none"
        );
    }

    #[test]
    fn a_burst_over_the_allowance_is_refused_and_the_rate_refills_it() {
        let mut limiter = PoseLimiter::default();
        let t0 = Instant::now();
        let admitted = (0..20).filter(|_| limiter.admit(t0)).count();
        assert_eq!(
            admitted, POSE_BURST as usize,
            "only the burst gets through at one instant"
        );
        // One token comes back every 1 / POSE_RATE_HZ seconds.
        let step = Duration::from_secs_f64(1.0 / f64::from(POSE_RATE_HZ));
        // A hair over, so the refill is not decided by the last bit of a float.
        let whole = step + Duration::from_micros(10);
        assert!(!limiter.admit(t0 + step / 2), "half a token is not a write");
        assert!(limiter.admit(t0 + whole), "a whole one is");
        assert!(!limiter.admit(t0 + whole), "and it is spent");
    }

    #[test]
    fn a_steady_rate_below_the_cap_is_never_refused_and_one_above_it_is() {
        let t0 = Instant::now();
        let mut slow = PoseLimiter::default();
        // 60 writes a second, the extension's own cap, for ten seconds.
        let every = Duration::from_secs_f64(1.0 / 60.0);
        assert!(
            (0..600).all(|n| slow.admit(t0 + every * n)),
            "the extension's rate passes"
        );
        let mut fast = PoseLimiter::default();
        // 1000 writes a second for one second: the cap, plus the burst, gets through.
        let every = Duration::from_millis(1);
        let admitted = (0..1000).filter(|n| fast.admit(t0 + every * *n)).count();
        assert!(
            admitted <= (POSE_RATE_HZ + POSE_BURST) as usize && admitted + 2 >= POSE_RATE_HZ as usize,
            "{admitted} of 1000 got through"
        );
    }

    #[test]
    fn a_long_idle_gives_back_the_burst_and_no_more() {
        let mut limiter = PoseLimiter::default();
        let t0 = Instant::now();
        assert!((0..POSE_BURST).all(|_| limiter.admit(t0)));
        let later = t0 + Duration::from_secs(3600);
        let admitted = (0..20).filter(|_| limiter.admit(later)).count();
        assert_eq!(admitted, POSE_BURST as usize);
    }

    #[test]
    fn a_clock_that_steps_back_adds_no_tokens() {
        let mut limiter = PoseLimiter::default();
        let t0 = Instant::now() + Duration::from_secs(10);
        assert_eq!((0..20).filter(|_| limiter.admit(t0)).count(), POSE_BURST as usize);
        assert!(
            !limiter.admit(t0 - Duration::from_secs(5)),
            "an earlier instant refills nothing"
        );
        assert!(
            !limiter.admit(t0),
            "and the next call from the real time does not either"
        );
    }

    #[test]
    fn each_observer_has_its_own_allowance_and_a_stranger_has_none() {
        let mut session = TreeSession::default();
        session.attach_observer(HOLDER, true).expect("observe");
        session.attach_observer(OTHER_HOLDER, true).expect("observe");
        let t0 = Instant::now();
        assert_eq!(
            (0..20).filter(|_| session.admit_pose(HOLDER, t0)).count(),
            POSE_BURST as usize
        );
        assert!(
            session.admit_pose(OTHER_HOLDER, t0),
            "the other observer's allowance is untouched"
        );
        assert!(
            !session.admit_pose(TreeHolder::new(77), t0),
            "a connection that is not observing has none"
        );
        // Attaching again after a release starts a fresh allowance.
        session.release_observer(HOLDER);
        session.attach_observer(HOLDER, true).expect("observe");
        assert!(session.admit_pose(HOLDER, t0));
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
            ("skipped", Some(DefaultValue::List(Vec::new()))),
        ];
        assert_eq!(node.properties.len(), defaults.len());
        for (key, default) in defaults {
            let property = node.properties.iter().find(|property| property.name == key).expect(key);
            assert_eq!(property.default, default, "{key}");
        }

        let readonly = ["id", "parent", "skipped"];
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
        assert_eq!(
            classes.property("node", "skipped"),
            Some(PropertyInfo {
                kind: ValueType::ListText,
                readonly: true,
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
