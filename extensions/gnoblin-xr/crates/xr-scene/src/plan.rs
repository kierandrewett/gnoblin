//! Translates the `room` subtree into layers.
//!
//! The rules are in `design/node-api-refactor.md`, section 9:
//!
//! - Only Views under the node named `room` are considered. The room's own
//!   ancestors contribute their transform, opacity and visibility, so moving a
//!   parent moves the room.
//! - Views are planned in tree order: a depth-first walk that visits each node
//!   before its children, in child order. That order is the submission order,
//!   and nothing depth-tests, so the last layer drawn is what the user sees.
//! - A visible View with a source becomes a [`Layer::Quad`] in world space at
//!   its global transform, scaled to metres. A View beneath the head camera
//!   becomes a head-locked quad in view space: the head camera's own transform
//!   and everything above it are excluded. A View beneath a stereo camera
//!   becomes a [`Layer::Projection`].
//! - Opacity multiplies down the tree. Invisible nodes hide their subtrees.
//!   `input` is true only when the View and every ancestor accept input.
//!
//! The head pose is accepted so the call matches the runtime's per-frame call,
//! but it is not read: all poses stay in tracking space, and the runtime moves
//! the head.

use std::collections::BTreeSet;

use crate::math::{HeadPose, Pose, Transform, euler_degrees};
use crate::tree::{CameraProjection, Class, NodeId, TreeNode, TreeView};

const ROOM_NAME: &str = "room";
const HEAD_NAME: &str = "head";

/// Settings the translation needs beyond the tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Metres per node unit, used when neither the head camera nor the room
    /// sets `metres`.
    pub default_metres: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self { default_metres: 1.0 }
    }
}

/// The frame a layer's pose is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    /// Tracking space. The runtime moves the head within it.
    World,
    /// The head's space. The layer moves with the head.
    View,
}

/// The placement shared by quad and projection layers.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// The View node this layer draws.
    pub id: NodeId,
    /// The window node the View shows: the `source` of its `Class::View`. Pointer input over
    /// this layer is delivered to this node.
    pub source: NodeId,
    /// Position in metres, orientation as a quaternion `[x, y, z, w]`.
    pub pose: Pose,
    /// Width and height in metres.
    pub size: [f64; 2],
    /// Width and height in the View's node units. Pick maps into these.
    pub extent: [f64; 2],
    /// Opacity in `[0, 1]`, already multiplied down the tree.
    pub opacity: f64,
    pub space: Space,
    /// Whether rays may hit this layer.
    pub input: bool,
}

/// One OpenXR layer to submit.
#[derive(Debug, Clone, PartialEq)]
pub enum Layer {
    /// A flat quad.
    Quad(Placement),
    /// A stereo projection layer.
    Projection(Placement),
}

/// Why a View produced no layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The global transform, the size or the opacity has a NaN or infinity.
    NonFinite,
    /// The global scale is zero, so the placement cannot be inverted.
    Singular,
    /// The View's width or height is not positive.
    InvalidSize,
    /// The metres value is not finite and positive. Every View is skipped.
    InvalidMetres,
}

/// A visible View with a source that could not be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skipped {
    pub id: NodeId,
    pub reason: SkipReason,
}

/// The result of planning one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Layers in submission order.
    pub layers: Vec<Layer>,
    /// Views that were visible but could not be placed, in tree order.
    pub skipped: Vec<Skipped>,
}

/// Plans the `room` subtree of `tree` as layers.
///
/// Returns an empty plan when there is no node named `room`, or when the room
/// or one of its ancestors is hidden. `head` is accepted and not read: see the
/// module docs.
pub fn plan(tree: &TreeView, _head: &HeadPose, opts: &Options) -> Plan {
    let Some(room_path) = path_to(tree, &tree.roots, &|n: &TreeNode| n.name.as_deref() == Some(ROOM_NAME)) else {
        return Plan::default();
    };
    let Some((&room_id, ancestors)) = room_path.split_last() else {
        return Plan::default();
    };
    let Some(room) = tree.nodes.get(&room_id) else {
        return Plan::default();
    };

    let head = path_to(tree, &[room_id], &is_head).and_then(|path| path.last().copied());
    let head_metres = head.and_then(|id| tree.nodes.get(&id)).and_then(|node| node.metres);

    // The room's ancestors contribute their transform, opacity and visibility.
    let mut frame = Frame::ROOT;
    for id in ancestors {
        let Some(node) = tree.nodes.get(id) else {
            return Plan::default();
        };
        if !node.visible {
            return Plan::default();
        }
        frame = frame_for(node, frame, head);
    }

    let mut walker = Walker {
        tree,
        head,
        metres: head_metres.or(room.metres).unwrap_or(opts.default_metres),
        visited: BTreeSet::new(),
        plan: Plan::default(),
    };
    walker.visit(room_id, frame);
    walker.plan
}

fn is_head(node: &TreeNode) -> bool {
    matches!(node.class, Class::Camera { .. }) && node.name.as_deref() == Some(HEAD_NAME)
}

/// The ids from one of `from` down to the first node matching `want`, in
/// depth-first order. The last id is the match.
fn path_to(tree: &TreeView, from: &[NodeId], want: &dyn Fn(&TreeNode) -> bool) -> Option<Vec<NodeId>> {
    let mut path = Vec::new();
    let mut seen = BTreeSet::new();
    find_path(tree, from, want, &mut seen, &mut path).then_some(path)
}

fn find_path(
    tree: &TreeView,
    ids: &[NodeId],
    want: &dyn Fn(&TreeNode) -> bool,
    seen: &mut BTreeSet<NodeId>,
    path: &mut Vec<NodeId>,
) -> bool {
    for &id in ids {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = tree.nodes.get(&id) else {
            continue;
        };
        path.push(id);
        if want(node) || find_path(tree, &node.children, want, seen, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// What a node passes to its children.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// The global transform, in node units.
    transform: Transform,
    /// Opacity multiplied down the tree.
    opacity: f64,
    /// View once the walk has passed the head camera.
    space: Space,
    /// Set by the nearest enclosing camera: true for a stereo camera.
    stereo: bool,
    /// False once any ancestor or the node itself refuses input.
    input: bool,
}

impl Frame {
    const ROOT: Self = Self {
        transform: Transform::IDENTITY,
        opacity: 1.0,
        space: Space::World,
        stereo: false,
        input: true,
    };
}

/// The frame a child of `parent` sees, given `node`.
fn frame_for(node: &TreeNode, parent: Frame, head: Option<NodeId>) -> Frame {
    let mut frame = parent;
    frame.opacity = parent.opacity * node.opacity.clamp(0.0, 1.0);
    frame.input = parent.input && node.input;
    if Some(node.id) == head {
        // The head is the origin of view space: its own transform and everything
        // above it are excluded, so head-locked poses do not move with the head.
        frame.transform = Transform::IDENTITY;
        frame.space = Space::View;
    } else {
        let local = Transform {
            position: node.position,
            rotation: euler_degrees(node.rotation),
            scale: node.scale,
        };
        frame.transform = parent.transform.compose(local);
        if let Class::Camera { projection } = node.class {
            frame.stereo = projection == CameraProjection::Stereo;
        }
    }
    frame
}

struct Walker<'a> {
    tree: &'a TreeView,
    head: Option<NodeId>,
    metres: f64,
    /// Nodes already reached. A node listed under two parents is planned once,
    /// and a cycle is cut where it closes.
    visited: BTreeSet<NodeId>,
    plan: Plan,
}

impl Walker<'_> {
    fn visit(&mut self, id: NodeId, parent: Frame) {
        let tree = self.tree;
        if !self.visited.insert(id) {
            return;
        }
        let Some(node) = tree.nodes.get(&id) else {
            return;
        };
        if !node.visible {
            return;
        }
        let frame = frame_for(node, parent, self.head);
        if let Class::View {
            source: Some(source),
            width,
            height,
        } = node.class
        {
            self.emit(node.id, source, frame, width, height);
        }
        for &child in &node.children {
            self.visit(child, frame);
        }
    }

    fn emit(&mut self, id: NodeId, source: NodeId, frame: Frame, width: f64, height: f64) {
        let metres = self.metres;
        if !(metres.is_finite() && metres > 0.0) {
            self.skip(id, SkipReason::InvalidMetres);
            return;
        }
        let t = frame.transform;
        if !(width.is_finite() && height.is_finite() && frame.opacity.is_finite() && t.is_finite()) {
            self.skip(id, SkipReason::NonFinite);
            return;
        }
        if t.scale == 0.0 {
            self.skip(id, SkipReason::Singular);
            return;
        }
        if width <= 0.0 || height <= 0.0 {
            self.skip(id, SkipReason::InvalidSize);
            return;
        }

        // A negative scale mirrors the content. The placement keeps the position,
        // and the size is taken by magnitude.
        let magnitude = t.scale.abs();
        let placement = Placement {
            id,
            source,
            pose: Pose {
                position: t.position.map(|c| c * metres),
                orientation: t.rotation,
            },
            size: [width * magnitude * metres, height * magnitude * metres],
            extent: [width, height],
            opacity: frame.opacity,
            space: frame.space,
            input: frame.input,
        };
        let layer = if frame.space == Space::View || !frame.stereo {
            Layer::Quad(placement)
        } else {
            Layer::Projection(placement)
        };
        self.plan.layers.push(layer);
    }

    fn skip(&mut self, id: NodeId, reason: SkipReason) {
        self.plan.skipped.push(Skipped { id, reason });
    }
}
