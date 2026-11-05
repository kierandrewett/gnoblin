//! The scene graph: nodes, their transforms, projected bounds, hit testing and
//! damage.
//!
//! A node draws something the caller owns: a toplevel, a layer surface, the
//! cursor, an overlay. The scene owns what the renderer needs and nothing else:
//! parentage, paint order inside a parent, a local rectangle, a visual
//! transform, an effect chain, and whether the node is a 3D space with a camera.
//!
//! # Coordinates and the 2D subset
//!
//! A node's `bounds` are a rectangle in its own local space (usually
//! `0, 0, width, height`), and its transform places that rectangle in its
//! parent's space. The scene composes the chain from a node up to the root, so a
//! descendant's damage follows every ancestor's scale, rotation and offset
//! instead of the rectangle it was born with.
//!
//! Every transform is a 3D offset, rotation and scale, and every node is placed
//! by a 4x4 matrix. A flat scene is the orthographic case at depth zero. Only a
//! non-finite or non-invertible transform is refused, with the named
//! `unsupported-3d` error, never clamped: a silently repaired matrix would draw
//! the wrong frame while looking successful.
//!
//! # Damage
//!
//! Every mutation records the projected pixel bounds of the node and its
//! descendants before and after the change, so a move damages where the node was
//! and where it is now, and a removal damages the whole subtree it took away.
//! Ancestors are marked dirty because their composed bounds are stale, but they
//! do not add damage rectangles: their own pixels did not move.
//!
//! `take_damage` clips the accumulated rectangles to each output, merges
//! rectangles whose union costs no extra pixels, and degrades an output to full
//! damage when more than [`Scene::damage_rect_limit`] rectangles remain. A node
//! under a perspective camera degrades its outputs to full damage directly,
//! because a projected bound under perspective is not the node's rectangle.
//!
//! # Hit testing
//!
//! [`Scene::hit_test`] walks the paint order from the topmost node down, mapping
//! the point into each node's local space with the inverse of the same transform
//! chain rendering uses. It is therefore impossible for a point to hit a node
//! whose rendered pixels are elsewhere.

use std::collections::{BTreeMap, BTreeSet};

use gnoblin_types::{OutputId, PointF, Rect, RectF, Vec3};

use crate::camera::{Camera, CameraError, Projection};
use crate::damage::{Damage, OutputDamage, merge_rects};
use crate::effect::{EffectError, Effects};
use crate::id::NodeId;
use crate::matrix::Mat4;
use crate::property::{PropertyError, PropertyValue, VisualProperty};
use crate::transform::VisualTransform;

/// The default number of damage rectangles per output.
///
/// Eight is the size at which a frame's rectangle list is still cheaper than a
/// full repaint for a typical 1080p or 4K output, and it bounds the work a
/// renderer has to plan for. A caller with a different budget changes it.
pub const DEFAULT_DAMAGE_RECT_LIMIT: usize = 8;

/// What a node is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum NodeKind {
    /// A node that draws its own content and its children in paint order.
    #[default]
    Visual,
    /// A 3D space: its children are projected through the camera.
    Space(Camera),
}

/// How to create a node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSpec {
    /// The parent, or `None` for a root node.
    pub parent: Option<NodeId>,
    /// Paint position among siblings; higher draws later, on top.
    pub z_order: i32,
    /// The local rectangle the node occupies before its transform.
    pub bounds: Rect,
    /// The visual transform.
    pub transform: VisualTransform,
    /// The effect chain, applied in order.
    pub effects: Effects,
    /// Whether the node draws content or projects a space.
    pub kind: NodeKind,
}

impl NodeSpec {
    /// A plain visual node at the origin with default properties.
    #[must_use]
    pub fn new(bounds: Rect) -> Self {
        Self {
            parent: None,
            z_order: 0,
            bounds,
            transform: VisualTransform::IDENTITY,
            effects: Effects::new(),
            kind: NodeKind::Visual,
        }
    }

    /// Validate every value this build must refuse before the node exists.
    fn validate(&self) -> Result<(), SceneError> {
        validate_transform(&self.transform)?;
        if let NodeKind::Space(camera) = self.kind {
            camera.validate(self.bounds.to_f64())?;
        }
        for pass in self.effects.passes() {
            pass.validate()?;
        }
        Ok(())
    }
}

/// A node as the scene stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    z_order: i32,
    bounds: Rect,
    transform: VisualTransform,
    effects: Effects,
    kind: NodeKind,
}

impl Node {
    /// The parent, if any.
    #[must_use]
    pub const fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// Children in paint order.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// Paint position among siblings.
    #[must_use]
    pub const fn z_order(&self) -> i32 {
        self.z_order
    }

    /// The local rectangle, without the effect padding.
    #[must_use]
    pub const fn bounds(&self) -> Rect {
        self.bounds
    }

    /// The visual transform.
    #[must_use]
    pub const fn transform(&self) -> &VisualTransform {
        &self.transform
    }

    /// The effect chain.
    #[must_use]
    pub const fn effects(&self) -> &Effects {
        &self.effects
    }

    /// What the node is.
    #[must_use]
    pub const fn kind(&self) -> NodeKind {
        self.kind
    }

    /// The pivot a scale and rotation use, in the node's local space.
    fn pivot(&self) -> Vec3 {
        self.bounds.to_f64().centre()
    }

    /// The local rectangle expanded by the effect chain's padding.
    fn visual_local(&self) -> RectF {
        self.effects.expand(self.bounds.to_f64())
    }

    /// The matrix that places this node's local plane in its parent's plane.
    fn local_matrix(&self) -> Mat4 {
        Mat4::node_local(&self.transform.base, self.pivot())
    }

    /// The matrix a space applies between its own plane and its children.
    fn content_matrix(&self) -> Mat4 {
        match self.kind {
            NodeKind::Visual => Mat4::IDENTITY,
            NodeKind::Space(camera) => camera.content_matrix(self.bounds.to_f64()),
        }
    }
}

/// One node's projected damage bounds, and whether it is drawn flat.
#[derive(Clone, Copy, Debug)]
struct DamageSample {
    /// The projected pixel bounds, or `None` when a corner lies behind the eye.
    rect: Option<Rect>,
    /// Whether the node is drawn without perspective, so its bounds are exact.
    affine: bool,
}

/// A hit-test result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The node that was hit.
    pub node: NodeId,
    /// The point in that node's local space.
    pub local: PointF,
}

/// A refused scene operation.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneError {
    /// A node with this id already exists.
    DuplicateNode(NodeId),
    /// No such node.
    UnknownNode(NodeId),
    /// A parent was named that does not exist, or the node cannot be its own
    /// parent.
    UnknownParent(NodeId),
    /// A value is NaN or infinite.
    NonFinite {
        /// The value's path.
        field: &'static str,
    },
    /// A transform cannot be inverted, so the scene cannot project through it.
    NotInvertible {
        /// The field that collapsed to zero.
        field: &'static str,
    },
    /// A camera cannot be honoured.
    Camera(CameraError),
    /// A property value was refused.
    Property(PropertyError),
    /// An effect or uniform was refused.
    Effect(EffectError),
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(formatter, "node {id} already exists"),
            Self::UnknownNode(id) => write!(formatter, "no node {id}"),
            Self::UnknownParent(id) => write!(formatter, "no parent {id}"),
            Self::NonFinite { field } => write!(formatter, "`{field}` is not finite"),
            Self::NotInvertible { field } => write!(formatter, "`{field}` is not invertible (code unsupported-3d)"),
            Self::Camera(error) => write!(formatter, "{error}"),
            Self::Property(error) => write!(formatter, "{error}"),
            Self::Effect(error) => write!(formatter, "{error}"),
        }
    }
}

/// Refuse a transform the scene cannot project through.
///
/// A non-finite value is refused, and so is a zero scale: a flattened axis has no
/// inverse, so a matrix built from it would draw nothing sensible.
fn validate_transform(transform: &VisualTransform) -> Result<(), SceneError> {
    if !transform.is_finite() {
        return Err(SceneError::NonFinite { field: "transform" });
    }
    let scale = transform.base.scale;
    for (field, value) in [("scale.x", scale.x), ("scale.y", scale.y), ("scale.z", scale.z)] {
        if value == 0.0 {
            return Err(SceneError::NotInvertible { field });
        }
    }
    Ok(())
}

impl std::error::Error for SceneError {}

impl From<CameraError> for SceneError {
    fn from(error: CameraError) -> Self {
        Self::Camera(error)
    }
}

impl From<PropertyError> for SceneError {
    fn from(error: PropertyError) -> Self {
        Self::Property(error)
    }
}

impl From<EffectError> for SceneError {
    fn from(error: EffectError) -> Self {
        Self::Effect(error)
    }
}

/// The scene graph for one session.
#[derive(Debug)]
pub struct Scene {
    nodes: BTreeMap<NodeId, Node>,
    roots: Vec<NodeId>,
    outputs: BTreeMap<OutputId, Rect>,
    dirty: BTreeSet<NodeId>,
    pending: Vec<Rect>,
    full: BTreeSet<OutputId>,
    damage_rect_limit: usize,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// An empty scene with the default damage budget.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            roots: Vec::new(),
            outputs: BTreeMap::new(),
            dirty: BTreeSet::new(),
            pending: Vec::new(),
            full: BTreeSet::new(),
            damage_rect_limit: DEFAULT_DAMAGE_RECT_LIMIT,
        }
    }

    /// Insert a node, refusing an id that exists and a parent that does not.
    ///
    /// The new node damages only itself: the pixels it adds were not on screen
    /// before, while a parent's existing pixels do not move.
    pub fn insert(&mut self, id: NodeId, spec: NodeSpec) -> Result<(), SceneError> {
        if self.nodes.contains_key(&id) {
            return Err(SceneError::DuplicateNode(id));
        }
        spec.validate()?;
        if let Some(parent) = spec.parent
            && !self.nodes.contains_key(&parent)
        {
            return Err(SceneError::UnknownParent(parent));
        }
        let node = Node {
            parent: spec.parent,
            children: Vec::new(),
            z_order: spec.z_order,
            bounds: spec.bounds,
            transform: spec.transform,
            effects: spec.effects,
            kind: spec.kind,
        };
        self.nodes.insert(id, node);
        match spec.parent {
            Some(parent) => {
                let mut siblings = std::mem::take(
                    &mut self
                        .nodes
                        .get_mut(&parent)
                        .expect("parent existence checked above")
                        .children,
                );
                siblings.push(id);
                sort_siblings(&mut siblings, &self.nodes);
                self.nodes
                    .get_mut(&parent)
                    .expect("parent existence checked above")
                    .children = siblings;
            }
            None => {
                self.roots.push(id);
                sort_siblings(&mut self.roots, &self.nodes);
            }
        }
        let sample = self.sample(id).expect("node was just inserted");
        self.push_sample(sample);
        self.dirty.insert(id);
        self.dirty.extend(self.ancestors(id));
        Ok(())
    }

    /// Remove a node and its whole subtree, returning the removed ids.
    ///
    /// The subtree's current projected bounds become damage: removing a node must
    /// clear every pixel it and its descendants occupied, not just the rectangle
    /// the caller remembers.
    pub fn remove(&mut self, id: NodeId) -> Result<Vec<NodeId>, SceneError> {
        if !self.nodes.contains_key(&id) {
            return Err(SceneError::UnknownNode(id));
        }
        let removed = self.subtree_ids(id);
        let samples: Vec<DamageSample> = removed.iter().filter_map(|node| self.sample(*node)).collect();
        let parent = self.nodes[&id].parent;
        match parent {
            Some(parent) => {
                if let Some(node) = self.nodes.get_mut(&parent) {
                    node.children.retain(|child| *child != id);
                }
            }
            None => self.roots.retain(|root| *root != id),
        }
        for node in &removed {
            self.nodes.remove(node);
        }
        for sample in samples {
            self.push_sample(sample);
        }
        match parent {
            Some(parent) => {
                self.dirty.insert(parent);
                self.dirty.extend(self.ancestors(parent));
            }
            None => self.dirty.extend(self.roots.iter().copied()),
        }
        Ok(removed)
    }

    /// Whether a node exists.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Every node id, in ascending order.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    /// One node.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// A node's transform.
    #[must_use]
    pub fn transform(&self, id: NodeId) -> Option<&VisualTransform> {
        self.nodes.get(&id).map(Node::transform)
    }

    /// A node's effect chain.
    #[must_use]
    pub fn effects(&self, id: NodeId) -> Option<&Effects> {
        self.nodes.get(&id).map(Node::effects)
    }

    /// A node's local rectangle, without effect padding.
    #[must_use]
    pub fn bounds(&self, id: NodeId) -> Option<Rect> {
        self.nodes.get(&id).map(Node::bounds)
    }

    /// A node's parent.
    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(&id).and_then(Node::parent)
    }

    /// A node's children, in paint order.
    #[must_use]
    pub fn children(&self, id: NodeId) -> Option<&[NodeId]> {
        self.nodes.get(&id).map(Node::children)
    }

    /// A node's paint position among its siblings.
    #[must_use]
    pub fn z_order(&self, id: NodeId) -> Option<i32> {
        self.nodes.get(&id).map(Node::z_order)
    }

    /// Set a node's transform.
    pub fn set_transform(&mut self, id: NodeId, transform: VisualTransform) -> Result<(), SceneError> {
        if !self.nodes.contains_key(&id) {
            return Err(SceneError::UnknownNode(id));
        }
        validate_transform(&transform)?;
        self.change(id, |node| node.transform = transform);
        Ok(())
    }

    /// Read a property.
    pub fn property(&self, id: NodeId, property: VisualProperty) -> Result<PropertyValue, SceneError> {
        self.transform(id)
            .map(|transform| property.get(transform))
            .ok_or(SceneError::UnknownNode(id))
    }

    /// Write one property.
    ///
    /// The value is applied to a copy first and the copy is checked for a finite,
    /// invertible transform, so a refused value leaves the node exactly as it was.
    /// Depth components are ordinary targets and are accepted and stored.
    pub fn set_property(
        &mut self,
        id: NodeId,
        property: VisualProperty,
        value: PropertyValue,
    ) -> Result<(), SceneError> {
        let mut candidate = *self.transform(id).ok_or(SceneError::UnknownNode(id))?;
        property.apply(&mut candidate, value)?;
        validate_transform(&candidate)?;
        self.change(id, |node| node.transform = candidate);
        Ok(())
    }

    /// Replace a node's effect chain.
    pub fn set_effects(&mut self, id: NodeId, effects: Effects) -> Result<(), SceneError> {
        if !self.nodes.contains_key(&id) {
            return Err(SceneError::UnknownNode(id));
        }
        for pass in effects.passes() {
            pass.validate()?;
        }
        self.change(id, |node| node.effects = effects);
        Ok(())
    }

    /// Move a node within its siblings' paint order.
    ///
    /// Reordering can change which sibling covers which, so every sibling's
    /// bounds become damage even though their geometry is unchanged.
    pub fn set_z_order(&mut self, id: NodeId, z_order: i32) -> Result<(), SceneError> {
        let parent = self.nodes.get(&id).ok_or(SceneError::UnknownNode(id))?.parent;
        let mut siblings = match parent {
            Some(parent) => self.nodes[&parent].children.clone(),
            None => self.roots.clone(),
        };
        let before: Vec<DamageSample> = siblings.iter().filter_map(|node| self.sample(*node)).collect();
        self.nodes.get_mut(&id).expect("node existence checked above").z_order = z_order;
        sort_siblings(&mut siblings, &self.nodes);
        match parent {
            Some(parent) => {
                self.nodes
                    .get_mut(&parent)
                    .expect("parent of an existing node exists")
                    .children = siblings.clone();
            }
            None => self.roots = siblings.clone(),
        }
        let after: Vec<DamageSample> = siblings.iter().filter_map(|node| self.sample(*node)).collect();
        for sample in before.into_iter().chain(after) {
            self.push_sample(sample);
        }
        self.dirty.insert(id);
        self.dirty.extend(self.ancestors(id));
        Ok(())
    }

    /// Declare or replace an output rectangle.
    ///
    /// A new output has never been drawn, so the caller asks for its first frame
    /// with [`Scene::damage_output`] instead of the scene inventing damage on the
    /// frame an output appears.
    pub fn set_output(&mut self, output: OutputId, rect: Rect) {
        self.outputs.insert(output, rect);
    }

    /// Remove an output.
    pub fn remove_output(&mut self, output: OutputId) -> Option<Rect> {
        self.full.remove(&output);
        self.outputs.remove(&output)
    }

    /// Mark a whole output as needing a redraw, for its first frame or a
    /// renderer reset.
    pub fn damage_output(&mut self, output: OutputId) {
        if self.outputs.contains_key(&output) {
            self.full.insert(output);
        }
    }

    /// Every declared output.
    #[must_use]
    pub fn outputs(&self) -> &BTreeMap<OutputId, Rect> {
        &self.outputs
    }

    /// The damage rectangle budget per output.
    #[must_use]
    pub const fn damage_rect_limit(&self) -> usize {
        self.damage_rect_limit
    }

    /// Set the damage rectangle budget; zero is raised to one.
    pub fn set_damage_rect_limit(&mut self, limit: usize) {
        self.damage_rect_limit = limit.max(1);
    }

    /// Nodes whose composed bounds are stale.
    #[must_use]
    pub fn dirty(&self) -> &BTreeSet<NodeId> {
        &self.dirty
    }

    /// Take the dirty set, leaving it empty.
    pub fn take_dirty(&mut self) -> BTreeSet<NodeId> {
        std::mem::take(&mut self.dirty)
    }

    /// The camera projection that applies to a node's subtree.
    ///
    /// The nearest ancestor space wins, and a plain desktop has no space, so the
    /// answer is orthographic. `None` means the node does not exist.
    #[must_use]
    pub fn projection_for(&self, id: NodeId) -> Option<Projection> {
        let mut current = self.nodes.get(&id)?.parent;
        while let Some(ancestor) = current {
            let node = &self.nodes[&ancestor];
            if let NodeKind::Space(camera) = node.kind {
                return Some(camera.projection);
            }
            current = node.parent;
        }
        Some(Projection::Orthographic)
    }

    /// A node's projected pixel bounds, including effect padding.
    ///
    /// `None` for a missing node, or when a corner of the node lies behind the eye.
    /// The bound is the bounding box of the projected corners.
    #[must_use]
    pub fn absolute_bounds(&self, id: NodeId) -> Option<Rect> {
        let node = self.nodes.get(&id)?;
        let world = self.world_matrix(id)?;
        project_bounds(&world, node.visual_local()).map(|bounds| bounds.to_pixel())
    }

    /// The nearest node containing a point on the output plane; equal depths go to
    /// the topmost node in paint order.
    #[must_use]
    pub fn hit_test(&self, point: PointF) -> Option<Hit> {
        if !point.is_finite() {
            return None;
        }
        let mut best = None;
        for root in self.roots.iter().rev() {
            self.hit_walk(*root, &Mat4::IDENTITY, point, &mut best);
        }
        best.map(|(_, hit)| hit)
    }

    /// Take the accumulated damage, clipped and merged per output.
    pub fn take_damage(&mut self) -> Damage {
        let mut outputs = Vec::new();
        for (output, rect) in &self.outputs {
            if self.full.contains(output) {
                outputs.push(OutputDamage {
                    output: *output,
                    rects: vec![*rect],
                    full: true,
                });
                continue;
            }
            let clipped: Vec<Rect> = self
                .pending
                .iter()
                .filter_map(|pending| pending.intersection(*rect))
                .collect();
            let merged = merge_rects(clipped);
            if merged.len() > self.damage_rect_limit {
                outputs.push(OutputDamage {
                    output: *output,
                    rects: vec![*rect],
                    full: true,
                });
            } else if !merged.is_empty() {
                outputs.push(OutputDamage {
                    output: *output,
                    rects: merged,
                    full: false,
                });
            }
        }
        self.pending.clear();
        self.full.clear();
        Damage { outputs }
    }

    /// Apply a mutation while recording the projected bounds before and after.
    fn change(&mut self, id: NodeId, apply: impl FnOnce(&mut Node)) {
        let scope = self.subtree_ids(id);
        let before: Vec<DamageSample> = scope.iter().filter_map(|node| self.sample(*node)).collect();
        {
            let node = self.nodes.get_mut(&id).expect("caller checked the node exists");
            apply(node);
        }
        let after: Vec<DamageSample> = scope.iter().filter_map(|node| self.sample(*node)).collect();
        for sample in before.into_iter().chain(after) {
            self.push_sample(sample);
        }
        self.dirty.extend(scope);
        self.dirty.extend(self.ancestors(id));
    }

    /// The node and every descendant, in paint order.
    fn subtree_ids(&self, id: NodeId) -> Vec<NodeId> {
        let mut ids = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let Some(node) = self.nodes.get(&current) else {
                continue;
            };
            ids.push(current);
            stack.extend(node.children.iter().rev().copied());
        }
        ids
    }

    /// Every ancestor of a node, excluding the node.
    fn ancestors(&self, id: NodeId) -> BTreeSet<NodeId> {
        let mut ancestors = BTreeSet::new();
        let mut current = self.nodes.get(&id).and_then(Node::parent);
        while let Some(ancestor) = current {
            if !ancestors.insert(ancestor) {
                break;
            }
            current = self.nodes.get(&ancestor).and_then(Node::parent);
        }
        ancestors
    }

    /// The matrix that maps a node's local plane into output space, before the
    /// perspective divide. `None` for a missing node.
    fn world_matrix(&self, id: NodeId) -> Option<Mat4> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(node_id) = current {
            let Some(node) = self.nodes.get(&node_id) else {
                break;
            };
            chain.push(node);
            current = node.parent;
        }
        if chain.is_empty() {
            return None;
        }
        let mut frame = Mat4::IDENTITY;
        let mut world = Mat4::IDENTITY;
        for node in chain.iter().rev() {
            world = frame.mul(&node.local_matrix());
            frame = world.mul(&node.content_matrix());
        }
        Some(world)
    }

    /// A node's current projected damage bounds, and whether it is drawn flat.
    fn sample(&self, id: NodeId) -> Option<DamageSample> {
        let world = self.world_matrix(id)?;
        let node = self.nodes.get(&id)?;
        Some(DamageSample {
            rect: project_bounds(&world, node.visual_local()).map(|bounds| bounds.to_pixel()),
            affine: world.m[3] == [0.0, 0.0, 0.0, 1.0],
        })
    }

    /// Record one node's damage: exact bounds for a flat node, the outputs it
    /// touches for a node under perspective, and every output when a corner lies
    /// behind the eye.
    fn push_sample(&mut self, sample: DamageSample) {
        match (sample.rect, sample.affine) {
            (None, _) => self.full.extend(self.outputs.keys().copied()),
            (Some(rect), true) => self.pending.push(rect),
            (Some(rect), false) => {
                for (output, output_rect) in &self.outputs {
                    if output_rect.intersection(rect).is_some() {
                        self.full.insert(*output);
                    }
                }
            }
        }
    }

    /// Visit a node and its descendants, topmost first, keeping the nearest hit.
    /// A candidate replaces the current hit only when strictly nearer, so an equal
    /// depth goes to the topmost node in paint order.
    fn hit_walk(&self, id: NodeId, frame: &Mat4, point: PointF, best: &mut Option<(f64, Hit)>) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        let world = frame.mul(&node.local_matrix());
        let child_frame = world.mul(&node.content_matrix());
        for child in node.children.iter().rev() {
            self.hit_walk(*child, &child_frame, point, best);
        }
        let Some((local, depth)) = locate(&world, point) else {
            return;
        };
        if !contains_point(node.bounds, local) {
            return;
        }
        let nearer = match best {
            Some((best_depth, _)) => depth > *best_depth,
            None => true,
        };
        if nearer {
            *best = Some((depth, Hit { node: id, local }));
        }
    }
}

/// Whether a continuous point lies inside a pixel rectangle, far edges excluded.
fn contains_point(rect: Rect, point: PointF) -> bool {
    let bounds = rect.to_f64();
    point.x >= bounds.x && point.y >= bounds.y && point.x < bounds.right() && point.y < bounds.bottom()
}

/// Map a point on the output plane into a node's local plane.
///
/// Returns the local point and its depth, or `None` when the point lies behind
/// the eye or the plane is edge-on. The depth is `Z / W`, so a nearer point has
/// the larger depth.
fn locate(world: &Mat4, point: PointF) -> Option<(PointF, f64)> {
    let m = world.m;
    let h = [
        [m[0][0], m[0][1], m[0][3]],
        [m[1][0], m[1][1], m[1][3]],
        [m[3][0], m[3][1], m[3][3]],
    ];
    let det = det3(h);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let p = [point.x, point.y, 1.0];
    let column = |index: usize| {
        let mut replaced = h;
        for (row, values) in replaced.iter_mut().enumerate() {
            values[index] = p[row];
        }
        det3(replaced)
    };
    let (d0, d1, d2) = (column(0), column(1), column(2));
    // Cramer's rule: the point's homogeneous weight is `d2 / det`, which must be
    // positive for the point to lie in front of the eye.
    if d2 / det <= 0.0 {
        return None;
    }
    let u = d0 / d2;
    let v = d1 / d2;
    let depth = (m[2][0] * d0 + m[2][1] * d1 + m[2][3] * d2) / det;
    if !(u.is_finite() && v.is_finite() && depth.is_finite()) {
        return None;
    }
    Some((PointF { x: u, y: v }, depth))
}

fn det3(a: [[f64; 3]; 3]) -> f64 {
    a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
}

/// The bounding box of a rectangle's corners after projection, or `None` when a
/// corner lies behind the eye, where the projected box is not meaningful.
fn project_bounds(world: &Mat4, rect: RectF) -> Option<RectF> {
    let corners = [
        (rect.x, rect.y),
        (rect.right(), rect.y),
        (rect.x, rect.bottom()),
        (rect.right(), rect.bottom()),
    ];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in corners {
        let [sx, sy, _, w] = world.apply([x, y, 0.0, 1.0]);
        if w.is_nan() || w <= 0.0 {
            return None;
        }
        let (px, py) = (sx / w, sy / w);
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px);
        max_y = max_y.max(py);
    }
    Some(RectF {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    })
}

/// Sort sibling ids by paint order, breaking a tie by id so the result is a
/// function of the scene and not of insertion order.
fn sort_siblings(ids: &mut [NodeId], nodes: &BTreeMap<NodeId, Node>) {
    ids.sort_by_key(|id| (nodes[id].z_order, *id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Blur, Pass, Shadow};

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect::new(x, y, width, height)
    }

    fn scene_with_output() -> Scene {
        let mut scene = Scene::new();
        scene.set_output(OutputId::new(0), rect(0, 0, 1920, 1080));
        scene
    }

    fn damage_covers(scene: &mut Scene, expected: Rect) -> bool {
        let damage = scene.take_damage();
        damage
            .outputs
            .iter()
            .any(|entry| entry.output == OutputId::new(0) && entry.covers(expected))
    }

    #[test]
    fn damage_covers_a_child_of_a_transformed_parent() {
        let mut scene = scene_with_output();
        let parent = NodeId::new(1);
        let child = NodeId::new(2);
        scene
            .insert(parent, NodeSpec::new(rect(0, 0, 400, 300)))
            .expect("parent");
        // The child sits below and right of the parent's centre, so scaling the
        // parent by two moves it outwards while keeping it on the output.
        let mut spec = NodeSpec::new(rect(200, 150, 100, 100));
        spec.parent = Some(parent);
        scene.insert(child, spec).expect("child");
        scene.take_damage();
        // The parent scales by two about its centre, so the child's projection is
        // not the rectangle it was born with.
        let transform = VisualTransform {
            base: gnoblin_types::Transform::scaled(2.0, 2.0),
            ..VisualTransform::IDENTITY
        };
        scene.set_transform(parent, transform).expect("2D transform");
        let expected = scene.absolute_bounds(child).expect("projected");
        // Scaling about the parent's centre pushes the child's edges outwards,
        // so the projected rectangle is not the local one.
        assert_eq!(expected, rect(200, 150, 200, 200));
        assert!(damage_covers(&mut scene, expected));
    }

    #[test]
    fn damage_covers_where_a_node_was_before_it_moved() {
        let mut scene = scene_with_output();
        let node = NodeId::new(1);
        scene.insert(node, NodeSpec::new(rect(0, 0, 100, 100))).expect("node");
        scene.take_damage();
        let transform = VisualTransform {
            base: gnoblin_types::Transform::translate(500.0, 400.0),
            ..VisualTransform::IDENTITY
        };
        scene.set_transform(node, transform).expect("2D transform");
        let damage = scene.take_damage();
        let output = damage.for_output(OutputId::new(0)).expect("output damage");
        assert!(output.covers(rect(0, 0, 100, 100)));
        assert!(output.covers(rect(500, 400, 100, 100)));
    }

    #[test]
    fn removing_a_node_damages_its_whole_subtree() {
        let mut scene = scene_with_output();
        let parent = NodeId::new(1);
        let child = NodeId::new(2);
        scene
            .insert(parent, NodeSpec::new(rect(0, 0, 200, 200)))
            .expect("parent");
        let mut spec = NodeSpec::new(rect(300, 300, 50, 50));
        spec.parent = Some(parent);
        scene.insert(child, spec).expect("child");
        scene.take_damage();
        scene.remove(parent).expect("removed");
        let damage = scene.take_damage();
        let output = damage.for_output(OutputId::new(0)).expect("output damage");
        assert!(output.covers(rect(0, 0, 200, 200)));
        assert!(output.covers(rect(300, 300, 50, 50)));
    }

    #[test]
    fn the_damage_cap_degrades_to_full_output_instead_of_dropping_rectangles() {
        let mut scene = scene_with_output();
        scene.set_damage_rect_limit(2);
        for index in 0..6 {
            let node = NodeId::new(index);
            let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
            spec.transform.base.offset = Vec3::new(index as f64 * 200.0, 10.0, 0.0);
            scene.insert(node, spec).expect("node");
        }
        let damage = scene.take_damage();
        let output = damage.for_output(OutputId::new(0)).expect("output damage");
        assert!(output.full);
        assert_eq!(output.rects, vec![rect(0, 0, 1920, 1080)]);
    }

    #[test]
    fn a_depth_transform_is_accepted_and_stored() {
        let mut scene = Scene::new();
        let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
        spec.transform.base.depth = 1.0;
        scene.insert(NodeId::new(1), spec.clone()).expect("depth is accepted");
        assert_eq!(scene.transform(NodeId::new(1)), Some(&spec.transform));

        let node = NodeId::new(2);
        scene.insert(node, NodeSpec::new(rect(0, 0, 10, 10))).expect("node");
        let transform = VisualTransform {
            base: gnoblin_types::Transform {
                rotation_degrees: Vec3::new(5.0, 0.0, 0.0),
                ..gnoblin_types::Transform::IDENTITY
            },
            ..VisualTransform::IDENTITY
        };
        scene.set_transform(node, transform).expect("rotation is accepted");
        assert_eq!(scene.transform(node), Some(&transform));
    }

    #[test]
    fn a_depth_property_write_is_stored() {
        let mut scene = Scene::new();
        let node = NodeId::new(1);
        scene.insert(node, NodeSpec::new(rect(0, 0, 10, 10))).expect("node");
        scene
            .set_property(node, VisualProperty::OffsetZ, PropertyValue::number(4.0))
            .expect("offset-z is accepted");
        assert_eq!(
            scene.property(node, VisualProperty::OffsetZ).expect("readable"),
            PropertyValue::number(4.0)
        );
    }

    #[test]
    fn a_perspective_space_is_accepted_and_a_bad_field_of_view_is_refused() {
        let mut scene = Scene::new();
        let mut spec = NodeSpec::new(rect(0, 0, 100, 100));
        spec.kind = NodeKind::Space(Camera::perspective(60.0, 0.1, 100.0));
        scene.insert(NodeId::new(1), spec).expect("perspective is accepted");

        let mut bad = NodeSpec::new(rect(0, 0, 100, 100));
        bad.kind = NodeKind::Space(Camera::perspective(0.0, 0.1, 100.0));
        let error = scene.insert(NodeId::new(2), bad).expect_err("must refuse");
        assert!(matches!(error, SceneError::Camera(CameraError::FieldOfView { .. })));
        assert!(!scene.contains(NodeId::new(2)));
    }

    #[test]
    fn a_duplicate_node_and_a_missing_parent_are_refused() {
        let mut scene = Scene::new();
        scene
            .insert(NodeId::new(1), NodeSpec::new(rect(0, 0, 10, 10)))
            .expect("first insert");
        assert_eq!(
            scene
                .insert(NodeId::new(1), NodeSpec::new(rect(0, 0, 10, 10)))
                .expect_err("duplicate"),
            SceneError::DuplicateNode(NodeId::new(1))
        );
        let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
        spec.parent = Some(NodeId::new(9));
        assert_eq!(
            scene.insert(NodeId::new(2), spec).expect_err("missing parent"),
            SceneError::UnknownParent(NodeId::new(9))
        );
    }

    #[test]
    fn a_non_finite_transform_is_refused() {
        let mut scene = Scene::new();
        let mut spec = NodeSpec::new(rect(0, 0, 10, 10));
        spec.transform.opacity = f32::NAN;
        let error = scene.insert(NodeId::new(1), spec).expect_err("must refuse");
        assert!(matches!(error, SceneError::NonFinite { .. }));
    }

    #[test]
    fn hit_testing_goes_through_the_inverse_transform() {
        let mut scene = Scene::new();
        let parent = NodeId::new(1);
        let child = NodeId::new(2);
        scene
            .insert(parent, NodeSpec::new(rect(0, 0, 200, 200)))
            .expect("parent");
        let mut spec = NodeSpec::new(rect(0, 0, 100, 50));
        spec.parent = Some(parent);
        spec.transform.base.offset = Vec3::new(50.0, 25.0, 0.0);
        scene.insert(child, spec).expect("child");
        // Inside the child's transformed rectangle.
        let hit = scene.hit_test(PointF::new(75.0, 40.0)).expect("hit");
        assert_eq!(hit.node, child);
        assert!((hit.local.x - 25.0).abs() < 1e-9);
        assert!((hit.local.y - 15.0).abs() < 1e-9);
        // A point inside the parent but outside the child selects the parent.
        let hit = scene.hit_test(PointF::new(190.0, 190.0)).expect("hit");
        assert_eq!(hit.node, parent);
        // Outside every node, no hit.
        assert_eq!(scene.hit_test(PointF::new(1000.0, 1000.0)), None);
    }

    #[test]
    fn a_rotated_parent_is_hit_through_the_inverse_rotation() {
        let mut scene = Scene::new();
        let node = NodeId::new(1);
        let mut spec = NodeSpec::new(rect(0, 0, 100, 100));
        spec.transform.base.rotation_degrees = Vec3::new(0.0, 0.0, 90.0);
        scene.insert(node, spec).expect("node");
        // A 100x100 square rotated 90 degrees about its centre still occupies
        // 0..100, so a point at the centre hits it and one just outside misses.
        assert!(scene.hit_test(PointF::new(50.0, 50.0)).is_some());
        assert_eq!(scene.hit_test(PointF::new(140.0, 50.0)), None);
    }

    #[test]
    fn a_z_order_change_damages_every_sibling() {
        let mut scene = scene_with_output();
        let parent = NodeId::new(1);
        scene
            .insert(parent, NodeSpec::new(rect(0, 0, 400, 400)))
            .expect("parent");
        let mut first = NodeSpec::new(rect(0, 0, 100, 100));
        first.parent = Some(parent);
        scene.insert(NodeId::new(2), first).expect("first");
        let mut second = NodeSpec::new(rect(50, 50, 100, 100));
        second.parent = Some(parent);
        second.z_order = 5;
        scene.insert(NodeId::new(3), second).expect("second");
        scene.take_damage();
        scene.set_z_order(NodeId::new(2), 10).expect("reordered");
        assert_eq!(scene.children(parent), Some(&[NodeId::new(3), NodeId::new(2)][..]));
        let damage = scene.take_damage();
        let output = damage.for_output(OutputId::new(0)).expect("output damage");
        assert!(output.covers(rect(0, 0, 100, 100)));
        assert!(output.covers(rect(50, 50, 100, 100)));
    }

    #[test]
    fn an_effect_change_damages_the_padding_a_shadow_draws_outside_the_bounds() {
        let mut scene = scene_with_output();
        let node = NodeId::new(1);
        scene
            .insert(node, NodeSpec::new(rect(100, 100, 100, 100)))
            .expect("node");
        scene.take_damage();
        let mut effects = Effects::new();
        effects
            .push(Pass::Shadow(Shadow {
                offset: Vec3::new(20.0, 20.0, 0.0),
                blur: 10.0,
                opacity: 0.5,
                color: crate::color::Color::BLACK,
            }))
            .expect("shadow");
        scene.set_effects(node, effects).expect("effects");
        assert_eq!(scene.absolute_bounds(node), Some(rect(100, 100, 130, 130)));
        let damage = scene.take_damage();
        let output = damage.for_output(OutputId::new(0)).expect("output damage");
        assert!(output.covers(rect(100, 100, 130, 130)));
    }

    #[test]
    fn a_perspective_camera_falls_back_to_full_damage() {
        // The scene accepts a perspective camera, so this state is built
        // directly to pin the damage branch on its own terms: a projected bound
        // under perspective is not the node's rectangle, so the whole output is
        // redrawn.
        let mut scene = scene_with_output();
        let space = NodeId::new(1);
        let node = NodeId::new(2);
        scene.nodes.insert(
            space,
            Node {
                parent: None,
                children: vec![node],
                z_order: 0,
                bounds: rect(0, 0, 1920, 1080),
                transform: VisualTransform::IDENTITY,
                effects: Effects::new(),
                kind: NodeKind::Space(Camera::perspective(60.0, 0.1, 100.0)),
            },
        );
        scene.roots.push(space);
        scene.nodes.insert(
            node,
            Node {
                parent: Some(space),
                children: Vec::new(),
                z_order: 0,
                bounds: rect(0, 0, 100, 100),
                transform: VisualTransform::IDENTITY,
                effects: Effects::new(),
                kind: NodeKind::Visual,
            },
        );
        scene.dirty.clear();
        scene.pending.clear();
        scene
            .set_property(node, VisualProperty::Opacity, PropertyValue::number(0.5))
            .expect("opacity");
        let damage = scene.take_damage();
        let output = damage.for_output(OutputId::new(0)).expect("output damage");
        assert!(output.full, "a perspective projection degrades to full damage");
        assert_eq!(output.rects, vec![rect(0, 0, 1920, 1080)]);
    }

    #[test]
    fn the_dirty_set_names_the_node_its_descendants_and_its_ancestors() {
        let mut scene = Scene::new();
        scene
            .insert(NodeId::new(1), NodeSpec::new(rect(0, 0, 100, 100)))
            .expect("root");
        let mut spec = NodeSpec::new(rect(0, 0, 50, 50));
        spec.parent = Some(NodeId::new(1));
        scene.insert(NodeId::new(2), spec).expect("child");
        scene.take_dirty();
        scene
            .set_property(NodeId::new(2), VisualProperty::Opacity, PropertyValue::number(0.5))
            .expect("opacity");
        let dirty = scene.take_dirty();
        assert!(dirty.contains(&NodeId::new(1)));
        assert!(dirty.contains(&NodeId::new(2)));
        assert!(scene.take_dirty().is_empty());
    }

    #[test]
    fn a_blur_pass_does_not_extend_the_bounds() {
        let mut scene = Scene::new();
        let node = NodeId::new(1);
        let mut spec = NodeSpec::new(rect(30, 40, 100, 100));
        spec.effects.push(Pass::Blur(Blur { radius: 24.0 })).expect("blur");
        scene.insert(node, spec).expect("node");
        assert_eq!(scene.absolute_bounds(node), Some(rect(30, 40, 100, 100)));
    }
}
