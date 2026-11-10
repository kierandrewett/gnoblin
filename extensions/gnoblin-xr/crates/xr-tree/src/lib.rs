//! The owner-socket tree client for gnoblin-xr.
//!
//! [`TreeClient::connect`] handshakes with the owner socket, attaches to the `tree`
//! namespace at version 1 and reads the snapshot into an [`xr_scene::TreeView`].
//! [`TreeClient::poll`] applies the `changes` batches that have arrived since.
//!
//! The model keeps only what the planner reads: the node graph, the transform and
//! visibility properties, and the View and camera fields. Other properties are ignored.
//! The wire gives rotation in radians and the model holds degrees, which is what
//! `xr-scene` reads. A camera projection of `perspective` becomes
//! [`CameraProjection::Orthographic`], because `xr-scene` has no third projection.
//! A camera's `metres` defaults to `0.001`, as the schema does.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use gnoblin_client::{Client, UnixTransport};
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, ChangesPayload, SnapshotPayload};
use gnoblin_proto::tree::{change, snapshot_node};
use gnoblin_proto::{Envelope, IPC_VERSION, Kind, LENGTH_PREFIX_LEN, MAX_FRAME_LEN, Version, read_frame, write_frame};
use xr_scene::{CameraProjection, Class, NodeId, TreeNode, TreeView};

/// The owner socket namespace the tree lives in.
pub const NAMESPACE: &str = "tree";

const ATTACH_VERSION: u64 = 1;
const FIRST_REQUEST_ID: u64 = 1;
/// The parent id the wire uses for "no parent": a root.
const NIL: u64 = 0;
const READ_WAIT: Duration = Duration::from_millis(1);
const WRITE_WAIT: Duration = Duration::from_secs(5);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
/// The schema default for a camera's `metres`.
const CAMERA_METRES: f64 = 0.001;

/// Why the tree client stopped.
#[derive(Debug)]
pub enum TreeError {
    /// The socket could not be reached, or the handshake failed.
    Connect(String),
    /// The compositor refused a request with an error code, such as `not-owner`.
    Refused { member: String, code: String },
    /// The compositor sent something the model cannot apply.
    Protocol(String),
    /// The compositor closed the socket.
    Closed,
    /// The snapshot did not arrive in time.
    Timeout,
    /// A socket read or write failed.
    Io(io::Error),
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(message) => write!(f, "cannot connect to the owner socket: {message}"),
            Self::Refused { member, code } => write!(f, "the compositor refused tree {member}: {code}"),
            Self::Protocol(message) => write!(f, "tree protocol error: {message}"),
            Self::Closed => write!(f, "the compositor closed the owner socket"),
            Self::Timeout => write!(f, "the tree snapshot did not arrive in time"),
            Self::Io(error) => write!(f, "owner socket i/o failed: {error}"),
        }
    }
}

impl std::error::Error for TreeError {}

/// An attached tree: the socket, the snapshot's model and the serial it is at.
pub struct TreeClient {
    stream: UnixStream,
    inbox: Vec<u8>,
    attached: bool,
    snapshot_done: bool,
    pending_nodes: Vec<snapshot_node::Properties>,
    serial: u64,
    model: TreeView,
    /// Each node's parent id, `NIL` for a root. `TreeView` does not store it.
    parents: BTreeMap<NodeId, NodeId>,
}

impl TreeClient {
    /// Connects to the owner socket at `path`, attaches and reads the snapshot.
    ///
    /// # Errors
    ///
    /// The socket is unreachable or the handshake fails ([`TreeError::Connect`]); the
    /// compositor refuses the attach, such as `not-owner` or `shell-already-attached`
    /// ([`TreeError::Refused`]); or the snapshot does not complete in time.
    pub fn connect(path: &Path) -> Result<Self, TreeError> {
        let transport =
            UnixTransport::connect(path).map_err(|error| TreeError::Connect(format!("{}: {error}", path.display())))?;
        let client = Client::connect(transport, Version::new(IPC_VERSION.0, IPC_VERSION.1, IPC_VERSION.2))
            .map_err(|error| TreeError::Connect(error.to_string()))?;
        let stream = client
            .into_inner()
            .into_stream()
            .map_err(|error| TreeError::Connect(error.to_string()))?;
        Self::from_stream(stream)
    }

    /// Attaches over a socket whose handshake is already done, and reads the snapshot.
    ///
    /// # Errors
    ///
    /// The same as [`TreeClient::connect`], without the connect step.
    pub fn from_stream(stream: UnixStream) -> Result<Self, TreeError> {
        stream.set_read_timeout(Some(READ_WAIT)).map_err(TreeError::Io)?;
        stream.set_write_timeout(Some(WRITE_WAIT)).map_err(TreeError::Io)?;
        let mut client = Self {
            stream,
            inbox: Vec::new(),
            attached: false,
            snapshot_done: false,
            pending_nodes: Vec::new(),
            serial: 0,
            model: TreeView::default(),
            parents: BTreeMap::new(),
        };
        client.send(&Envelope::request_message(
            NAMESPACE,
            "attach",
            FIRST_REQUEST_ID,
            &AttachArgs {
                version: ATTACH_VERSION,
            },
        ))?;
        let deadline = Instant::now() + CONNECT_WAIT;
        // One envelope at a time: a change that follows the snapshot stays queued for `poll`.
        while !(client.attached && client.snapshot_done) {
            if Instant::now() >= deadline {
                return Err(TreeError::Timeout);
            }
            match client.next_envelope()? {
                Some(envelope) => {
                    client.handle(envelope)?;
                }
                None => {
                    client.fill()?;
                }
            }
        }
        Ok(client)
    }

    /// Reads what has arrived since the last call and applies it. Returns `true` when the
    /// model changed. Nothing arriving is `Ok(false)`.
    ///
    /// # Errors
    ///
    /// The socket closed ([`TreeError::Closed`]), a read failed, or a message cannot be
    /// applied: a serial gap, an unknown node, or an undecodable payload.
    pub fn poll(&mut self) -> Result<bool, TreeError> {
        let mut changed = false;
        loop {
            while let Some(envelope) = self.next_envelope()? {
                changed |= self.handle(envelope)?;
            }
            if !self.fill()? {
                return Ok(changed);
            }
        }
    }

    /// The tree as the planner reads it.
    #[must_use]
    pub fn model(&self) -> &TreeView {
        &self.model
    }

    fn send(&mut self, envelope: &Envelope) -> Result<(), TreeError> {
        write_frame(&mut self.stream, envelope)
            .map_err(|error| TreeError::Protocol(format!("writing to the tree socket: {error}")))
    }

    /// Reads whatever bytes are waiting. `Ok(false)` means nothing arrived before the read timed out.
    fn fill(&mut self) -> Result<bool, TreeError> {
        let mut chunk = [0u8; 4096];
        match self.stream.read(&mut chunk) {
            Ok(0) => Err(TreeError::Closed),
            Ok(read) => {
                self.inbox.extend_from_slice(&chunk[..read]);
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(TreeError::Io(error)),
        }
    }

    /// Takes the next complete frame out of the inbox, if one has fully arrived.
    fn next_envelope(&mut self) -> Result<Option<Envelope>, TreeError> {
        let Some(prefix) = self.inbox.first_chunk::<LENGTH_PREFIX_LEN>() else {
            return Ok(None);
        };
        let len = u32::from_be_bytes(*prefix) as usize;
        if len > MAX_FRAME_LEN {
            return Err(TreeError::Protocol(format!(
                "a tree frame of {len} bytes is over the {MAX_FRAME_LEN} byte limit"
            )));
        }
        let total = LENGTH_PREFIX_LEN + len;
        if self.inbox.len() < total {
            return Ok(None);
        }
        let frame: Vec<u8> = self.inbox.drain(..total).collect();
        match read_frame(&mut frame.as_slice()) {
            Ok(Some(envelope)) => Ok(Some(envelope)),
            Ok(None) => Err(TreeError::Protocol("an empty tree frame".to_owned())),
            Err(error) => Err(TreeError::Protocol(format!("a tree frame does not decode: {error}"))),
        }
    }

    /// Applies one envelope. Returns `true` when the model changed.
    fn handle(&mut self, envelope: Envelope) -> Result<bool, TreeError> {
        if envelope.namespace != NAMESPACE {
            return Err(TreeError::Protocol(format!(
                "a message on the {} namespace came over the tree socket",
                envelope.namespace
            )));
        }
        let member = envelope.member.clone();
        match (&envelope.kind, member.as_str()) {
            (Some(Kind::Error(_)), _) => Err(TreeError::Refused {
                member: member.clone(),
                code: envelope.error_code().unwrap_or("unknown").to_owned(),
            }),
            (Some(Kind::Reply(_)), "attach") => {
                envelope
                    .payload_as::<AttachedPayload>()
                    .ok_or_else(|| undecodable(&member))?;
                self.attached = true;
                Ok(false)
            }
            (Some(Kind::Event(_)), "snapshot") => {
                let payload = envelope
                    .payload_as::<SnapshotPayload>()
                    .ok_or_else(|| undecodable(&member))?;
                self.pending_nodes.extend(payload.nodes);
                if !payload.last {
                    return Ok(false);
                }
                let nodes = std::mem::take(&mut self.pending_nodes);
                self.load(payload.serial, &nodes)?;
                self.snapshot_done = true;
                Ok(true)
            }
            (Some(Kind::Event(_)), "changes") => {
                if !self.snapshot_done {
                    return Err(TreeError::Protocol(
                        "tree changes arrived before the snapshot was complete".to_owned(),
                    ));
                }
                let payload = envelope
                    .payload_as::<ChangesPayload>()
                    .ok_or_else(|| undecodable(&member))?;
                self.apply_batch(payload.serial, &payload.changes)
            }
            _ => Err(TreeError::Protocol(format!(
                "an unexpected tree message {member} came over the tree socket"
            ))),
        }
    }

    /// Replaces the model with the snapshot. Children are ordered by their index, then id.
    fn load(&mut self, serial: u64, nodes: &[snapshot_node::Properties]) -> Result<(), TreeError> {
        let mut model = TreeView::default();
        let mut parents = BTreeMap::new();
        for node in nodes {
            let mut tree_node = new_node(node.id, &node.class);
            for prop in &node.props {
                set_prop(&mut tree_node, prop)?;
            }
            if model.nodes.insert(node.id, tree_node).is_some() {
                return Err(TreeError::Protocol(format!(
                    "the snapshot lists node {} twice",
                    node.id
                )));
            }
            parents.insert(node.id, node.parent);
        }
        let mut placed: Vec<(u64, usize, NodeId)> = Vec::with_capacity(nodes.len());
        for node in nodes {
            if node.parent != NIL && !model.nodes.contains_key(&node.parent) {
                return Err(missing(node.parent));
            }
            placed.push((node.parent, index_of(node.index), node.id));
        }
        placed.sort_unstable();
        for (parent, _, id) in placed {
            if parent == NIL {
                model.roots.push(id);
            } else {
                model
                    .nodes
                    .get_mut(&parent)
                    .ok_or_else(|| missing(parent))?
                    .children
                    .push(id);
            }
        }
        self.model = model;
        self.parents = parents;
        self.serial = serial;
        Ok(())
    }

    /// Applies one `changes` batch. A batch at or below the current serial is already in the
    /// model and is ignored; a batch past the next serial is a gap, which is an error.
    fn apply_batch(&mut self, serial: u64, changes: &[change::Properties]) -> Result<bool, TreeError> {
        if serial <= self.serial {
            return Ok(false);
        }
        if serial != self.serial + 1 {
            return Err(TreeError::Protocol(format!(
                "tree changes jump from serial {} to {serial}",
                self.serial
            )));
        }
        for change in changes {
            self.apply_change(change)?;
        }
        self.serial = serial;
        Ok(true)
    }

    fn apply_change(&mut self, change: &change::Properties) -> Result<(), TreeError> {
        match change.kind.as_str() {
            "created" => {
                if self.model.nodes.contains_key(&change.node) {
                    return Err(TreeError::Protocol(format!(
                        "tree node {} was created twice",
                        change.node
                    )));
                }
                self.model.insert(new_node(change.node, &change.class));
                self.parents.insert(change.node, NIL);
                Ok(())
            }
            "parented" => self.reparent(change.node, change.parent, index_of(change.index)),
            "set" => {
                let node = self
                    .model
                    .nodes
                    .get_mut(&change.node)
                    .ok_or_else(|| missing(change.node))?;
                set_prop(node, change)
            }
            "destroyed" => {
                self.detach(change.node);
                for id in std::iter::once(change.node).chain(change.removed.iter().copied()) {
                    self.model.nodes.remove(&id);
                    self.parents.remove(&id);
                }
                Ok(())
            }
            other => Err(TreeError::Protocol(format!("an unknown tree change kind {other}"))),
        }
    }

    fn reparent(&mut self, id: NodeId, parent: NodeId, index: usize) -> Result<(), TreeError> {
        if !self.model.nodes.contains_key(&id) {
            return Err(missing(id));
        }
        if parent == id {
            return Err(TreeError::Protocol(format!("tree node {id} was parented to itself")));
        }
        if parent != NIL && !self.model.nodes.contains_key(&parent) {
            return Err(missing(parent));
        }
        self.detach(id);
        self.parents.insert(id, parent);
        let siblings = if parent == NIL {
            &mut self.model.roots
        } else {
            &mut self
                .model
                .nodes
                .get_mut(&parent)
                .ok_or_else(|| missing(parent))?
                .children
        };
        let at = index.min(siblings.len());
        siblings.insert(at, id);
        Ok(())
    }

    /// Removes `id` from its parent's children, or from the roots.
    fn detach(&mut self, id: NodeId) {
        let parent = self.parents.get(&id).copied().unwrap_or(NIL);
        if parent == NIL {
            self.model.roots.retain(|&root| root != id);
        } else if let Some(node) = self.model.nodes.get_mut(&parent) {
            node.children.retain(|&child| child != id);
        }
    }
}

fn undecodable(member: &str) -> TreeError {
    TreeError::Protocol(format!("the tree {member} payload does not decode"))
}

fn missing(id: NodeId) -> TreeError {
    TreeError::Protocol(format!("tree node {id} is not in the model"))
}

/// A wire index as a position. An index too large for `usize` sorts last.
fn index_of<T>(index: T) -> usize
where
    usize: TryFrom<T>,
{
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// A new node of the schema class `class`, with the schema defaults.
fn new_node(id: NodeId, class: &str) -> TreeNode {
    match class {
        "node" => TreeNode::new(id, Class::Node),
        "view" => TreeNode::new(
            id,
            Class::View {
                source: None,
                width: 0.0,
                height: 0.0,
            },
        ),
        "camera" => {
            let mut node = TreeNode::new(
                id,
                Class::Camera {
                    projection: CameraProjection::Orthographic,
                },
            );
            node.metres = Some(CAMERA_METRES);
            node
        }
        _ => TreeNode::new(id, Class::Other),
    }
}

fn expect_kind(prop: &change::Properties, kind: &str) -> Result<(), TreeError> {
    if prop.value_kind == kind {
        Ok(())
    } else {
        Err(TreeError::Protocol(format!(
            "tree property {} carries {}, expected {kind}",
            prop.key, prop.value_kind
        )))
    }
}

/// Applies one property to `node`. Keys the planner does not read are ignored.
fn set_prop(node: &mut TreeNode, prop: &change::Properties) -> Result<(), TreeError> {
    match prop.key.as_str() {
        "name" => {
            expect_kind(prop, "text")?;
            node.name = (!prop.value_text.is_empty()).then(|| prop.value_text.clone());
        }
        "x" | "y" | "z" => {
            expect_kind(prop, "double")?;
            let axis = match prop.key.as_str() {
                "x" => 0,
                "y" => 1,
                _ => 2,
            };
            node.position[axis] = prop.value_double;
        }
        "rotation" => {
            expect_kind(prop, "list_double")?;
            let &[x, y, z] = prop.value_list.as_slice() else {
                return Err(TreeError::Protocol("tree rotation is not three angles".to_owned()));
            };
            node.rotation = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
        }
        "scale" => {
            expect_kind(prop, "list_double")?;
            node.scale = *prop
                .value_list
                .first()
                .ok_or_else(|| TreeError::Protocol("tree scale has no components".to_owned()))?;
        }
        "opacity" => {
            expect_kind(prop, "double")?;
            node.opacity = prop.value_double;
        }
        "visible" => {
            expect_kind(prop, "bool")?;
            node.visible = prop.value_bool;
        }
        "input" => {
            expect_kind(prop, "bool")?;
            node.input = prop.value_bool;
        }
        "metres" => {
            expect_kind(prop, "double")?;
            node.metres = Some(prop.value_double);
        }
        "width" => {
            expect_kind(prop, "double")?;
            if let Class::View { width, .. } = &mut node.class {
                *width = prop.value_double;
            }
        }
        "height" => {
            expect_kind(prop, "double")?;
            if let Class::View { height, .. } = &mut node.class {
                *height = prop.value_double;
            }
        }
        "source" => {
            expect_kind(prop, "node")?;
            if let Class::View { source, .. } = &mut node.class {
                *source = (prop.value_node != NIL).then_some(prop.value_node);
            }
        }
        "projection" => {
            expect_kind(prop, "text")?;
            if let Class::Camera { projection } = &mut node.class {
                *projection = match prop.value_text.as_str() {
                    "stereo" => CameraProjection::Stereo,
                    "orthographic" | "perspective" => CameraProjection::Orthographic,
                    other => {
                        return Err(TreeError::Protocol(format!("an unknown camera projection {other}")));
                    }
                };
            }
        }
        _ => {}
    }
    Ok(())
}
