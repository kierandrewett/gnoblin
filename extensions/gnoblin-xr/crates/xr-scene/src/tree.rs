//! The node model the planner reads.
//!
//! This is the mirror of the Gnoblin node graph that `gnoblin-xr` keeps from the
//! owner socket. A [`TreeView`] holds every node by id and the ordered list of
//! roots. Children are ordered: the planner's layer order is the depth-first
//! order of these child lists.

use std::collections::BTreeMap;

/// A node id, as the owner socket assigns it.
pub type NodeId = u64;

/// How a camera projects the View that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraProjection {
    /// A stereo camera: a View that names it holds two eyes side by side, and is shown as an OpenXR projection layer.
    Stereo,
    /// Any other projection: a View that names it holds one flat picture, and is shown as a quad.
    Orthographic,
}

/// What a stereo camera says about its eyes, as the schema states it.
///
/// The defaults are the schema's: an eye separation of 0.063 m, a vertical field of view of 60 degrees, and no
/// off-axis angles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rig {
    /// The distance between the eyes, in metres (`ipd`).
    pub ipd: f64,
    /// The vertical field of view in degrees (`fov`), used when the four angles are all zero.
    pub fov_degrees: f64,
    /// The off-axis frustum of the left eye in degrees: left, right, up and down (`fov_left`, `fov_right`, `fov_up`,
    /// `fov_down`). All zero means the symmetric frustum from `fov_degrees`.
    pub angles_degrees: [f64; 4],
}

impl Default for Rig {
    fn default() -> Self {
        Self {
            ipd: 0.063,
            fov_degrees: 60.0,
            angles_degrees: [0.0; 4],
        }
    }
}

/// What a node is. Only `View` produces a layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Class {
    /// A grouping node: a transform and no content of its own.
    Node,
    /// A surface drawn from `source`, `width` by `height` in node units, through `camera` when it names one.
    /// A View with no source has nothing to show and produces no layer.
    View {
        source: Option<NodeId>,
        camera: Option<NodeId>,
        width: f64,
        height: f64,
    },
    /// A camera. A camera named `head` makes the Views beneath it head-locked. A View that names a stereo camera is
    /// drawn with its eyes.
    Camera { projection: CameraProjection, rig: Rig },
    /// A seat: the input device group the owner delivers pointer input on. It draws nothing.
    Seat,
    /// Any other class. It draws nothing, but its transform and opacity still
    /// apply to the nodes beneath it.
    Other,
}

/// One node of the mirrored tree, with the properties the planner reads.
///
/// Build one with [`TreeNode::new`] and the builder methods, or set the fields
/// directly. The defaults are the identity transform, opacity 1, visible and
/// accepting input.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub id: NodeId,
    pub class: Class,
    /// The node's name. The planner looks for `room` and `head`.
    pub name: Option<String>,
    /// Child ids, in drawing order.
    pub children: Vec<NodeId>,
    /// Translation in node units, in the parent's space.
    pub position: [f64; 3],
    /// Euler angles in degrees, applied about X, then Y, then Z.
    pub rotation: [f64; 3],
    /// Uniform scale in the parent's space.
    pub scale: f64,
    /// A camera's `layer_budget`: how many layers the runtime lets one frame carry, as the extension last wrote it.
    /// `None` until a write is seen.
    pub layer_budget: Option<u64>,
    /// A seat's keyboard `focus`: the window node it names, or `None` for no focus.
    pub focus: Option<NodeId>,
    /// A controller's `claimed` inputs by name, which a shell binding owns: the program withholds them from windows.
    /// Empty until a write is seen.
    pub claimed: Vec<String>,
    /// Metres per node unit, set on the room or on the head camera.
    pub metres: Option<f64>,
    /// Opacity in `[0, 1]`. Opacity multiplies down the tree.
    pub opacity: f64,
    /// A hidden node hides its whole subtree.
    pub visible: bool,
    /// A node without input passes its subtree to the input mapper without
    /// rays hitting it.
    pub input: bool,
}

impl TreeNode {
    /// A node of `class` with the identity transform, opacity 1, visible and
    /// accepting input.
    pub fn new(id: NodeId, class: Class) -> Self {
        Self {
            id,
            class,
            name: None,
            children: Vec::new(),
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: 1.0,
            metres: None,
            layer_budget: None,
            focus: None,
            claimed: Vec::new(),
            opacity: 1.0,
            visible: true,
            input: true,
        }
    }

    pub fn named(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    pub fn children(mut self, ids: &[NodeId]) -> Self {
        self.children = ids.to_vec();
        self
    }

    pub fn at(mut self, x: f64, y: f64, z: f64) -> Self {
        self.position = [x, y, z];
        self
    }

    /// Euler angles in degrees, about X, then Y, then Z.
    pub fn rotated(mut self, x: f64, y: f64, z: f64) -> Self {
        self.rotation = [x, y, z];
        self
    }

    pub fn scaled(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    pub fn metres(mut self, metres: f64) -> Self {
        self.metres = Some(metres);
        self
    }

    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }

    /// Makes a View draw through `camera`. Other classes are unchanged.
    pub fn through(mut self, camera: NodeId) -> Self {
        if let Class::View { camera: slot, .. } = &mut self.class {
            *slot = Some(camera);
        }
        self
    }

    /// Sets a camera's stereo rig. Other classes are unchanged.
    pub fn rig(mut self, rig: Rig) -> Self {
        if let Class::Camera { rig: slot, .. } = &mut self.class {
            *slot = rig;
        }
        self
    }

    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }

    pub fn no_input(mut self) -> Self {
        self.input = false;
        self
    }
}

/// The mirrored tree: every node by id, and the roots in order.
///
/// A child id with no node, or a node listed under two parents, is planned once
/// under the first parent that reaches it. A cycle is cut where it closes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeView {
    pub nodes: BTreeMap<NodeId, TreeNode>,
    pub roots: Vec<NodeId>,
}

impl TreeView {
    /// Adds or replaces the node with `node.id`.
    pub fn insert(&mut self, node: TreeNode) {
        self.nodes.insert(node.id, node);
    }
}
