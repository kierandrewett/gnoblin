//! Transaction validation and apply (design/node-core.md §4).
//!
//! A transaction is a batch of [`Op`]s sent together. [`validate_and_apply`] checks
//! each op in order against the scene as the earlier ops left it, and it applies the
//! batch all or nothing. The first op that fails refuses the whole batch with a
//! [`Refusal`] that names that op's index; the scene is then exactly as it was.
//!
//! # Atomicity
//!
//! The batch runs on a clone of the scene. When every op succeeds the clone replaces
//! the scene; when one fails the clone is dropped. A transaction therefore costs one
//! copy of the scene, O(nodes). [`MAX_OPS`] bounds the ops in one batch; the scene's
//! own caps ([`crate::scene_ids::MAX_NODES`], [`crate::scene_ids::MAX_DEPTH`]) bound
//! its size.
//!
//! # Checks
//!
//! Each op is checked in this order, and the first failing check is its refusal:
//!
//! - `set`: node exists (`unknown-node`); the class has the property
//!   (`unknown-member`); it is not `readonly` (`read-only`); the value's kind matches
//!   the property's (`wrong-type`); a number is finite (`not-finite`); then, for a
//!   `timing`, the property is animated (`not-animated`), `seconds` is finite and in
//!   `(0, 3600]` (`not-finite`, `out-of-range`), the samples are finite
//!   (`not-finite`) and form a curve from 0 to 1, monotonic and within `[0, 1]`
//!   (`bad-curve`).
//! - `call`: node exists; the class has the method (`unknown-member`).
//! - `new`: the class exists (`unknown-class`); the id is free (`duplicate-node`);
//!   the scene's caps hold (`too-many-nodes`).
//! - `insert`: both nodes exist (`unknown-node`); the root is not moved
//!   (`root-immutable`); the actor may move the node (`not-owner`); the move is not
//!   a cycle (`cycle`) and not deeper than the cap (`too-deep`).
//! - `destroy`: the node exists; the root is not destroyed (`root-immutable`); the
//!   actor owns the node (`not-owner`).
//!
//! A batch longer than [`MAX_OPS`] is refused before any op runs, with `failed_op`
//! `-1` and `too-large`.
//!
//! # Ownership and read-only
//!
//! Ownership governs lifecycle and parentage only: `new`, `insert` and `destroy`
//! consult the actor. A property write is governed only by the class's `readonly`
//! flag, whoever owns the node.
//!
//! # Animated `set`
//!
//! A `set` with a `timing` is validated and recorded in [`Applied::animations`]; the
//! scene's value is not written here. The compositor plays the track later. The
//! change is still listed in [`Applied::changes`] so a mirror learns the target.
//!
//! # Refusal codes
//!
//! [`Code::as_str`] gives each code's spelling in design/node-core.md table 4.4.
//! Codes that belong to other slices (`stale-base`, `invalid-serial`,
//! `not-privileged`, `shell-already-attached`, `bad-paint`) are declared so the
//! spellings live in one place; this module never returns them. A scene refusal
//! with no table code maps to `bad-value`.

use gnoblin_animation::{PropertyValue, VisualProperty};
use gnoblin_types::{NodeId, Rect};

use crate::scene::{NodeSpec, Scene, SceneError};
use crate::scene_ids::Actor;

/// The most ops one transaction may carry. A longer batch is refused `too-large`.
pub const MAX_OPS: usize = 4096;

/// The longest animation a `set` may ask for, in seconds.
const MAX_SECONDS: f64 = 3600.0;

/// A refusal code, spelled as in design/node-core.md table 4.4 by [`Code::as_str`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// A node or parent id does not name a live node.
    UnknownNode,
    /// The actor may not create, move or destroy this node.
    NotOwner,
    /// The root node cannot be destroyed or moved.
    RootImmutable,
    /// A move would make a node its own ancestor.
    Cycle,
    /// `new` names a class the schema does not have.
    UnknownClass,
    /// The class has no such property or method.
    UnknownMember,
    /// The value's kind is not the property's kind.
    WrongType,
    /// The property is read-only.
    ReadOnly,
    /// A timing was given for a property that is not animated.
    NotAnimated,
    /// A value, or a timing, is NaN or infinite.
    NotFinite,
    /// A coordinate or size is beyond the scene's bound.
    OutOfBounds,
    /// `new` names an id that is already live.
    DuplicateNode,
    /// The timing's samples do not form a valid curve.
    BadCurve,
    /// A timing's duration or an index is outside its range.
    OutOfRange,
    /// The transaction has more than [`MAX_OPS`] ops.
    TooLarge,
    /// The scene already holds [`crate::scene_ids::MAX_NODES`] nodes.
    TooManyNodes,
    /// A node would sit deeper than [`crate::scene_ids::MAX_DEPTH`].
    TooDeep,
    /// Declared for another slice: a base revision no longer matches.
    StaleBase,
    /// Declared for another slice: a serial is not valid.
    InvalidSerial,
    /// Declared for another slice: the actor lacks the privilege.
    NotPrivileged,
    /// Declared for another slice: the shell is already attached.
    ShellAlreadyAttached,
    /// Declared for another slice: a paint value is malformed.
    BadPaint,
    /// A scene refusal with no table code of its own.
    BadValue,
}

impl Code {
    /// The code's spelling in design/node-core.md table 4.4.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownNode => "unknown-node",
            Self::NotOwner => "not-owner",
            Self::RootImmutable => "root-immutable",
            Self::Cycle => "cycle",
            Self::UnknownClass => "unknown-class",
            Self::UnknownMember => "unknown-member",
            Self::WrongType => "wrong-type",
            Self::ReadOnly => "read-only",
            Self::NotAnimated => "not-animated",
            Self::NotFinite => "not-finite",
            Self::OutOfBounds => "out-of-bounds",
            Self::DuplicateNode => "duplicate-node",
            Self::BadCurve => "bad-curve",
            Self::OutOfRange => "out-of-range",
            Self::TooLarge => "too-large",
            Self::TooManyNodes => "too-many-nodes",
            Self::TooDeep => "too-deep",
            Self::StaleBase => "stale-base",
            Self::InvalidSerial => "invalid-serial",
            Self::NotPrivileged => "not-privileged",
            Self::ShellAlreadyAttached => "shell-already-attached",
            Self::BadPaint => "bad-paint",
            Self::BadValue => "bad-value",
        }
    }
}

/// A refused transaction: the index of the op that failed (`-1` when the batch as a
/// whole was refused) and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The zero-based index of the failing op, or `-1` for the whole batch.
    pub failed_op: i32,
    /// The refusal code.
    pub code: Code,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "op {} refused: {}", self.failed_op, self.code.as_str())
    }
}

/// A value a `set` writes.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A floating-point number.
    Double(f64),
    /// An unsigned integer.
    Uint(u64),
    /// A boolean.
    Bool(bool),
}

/// The kind of a property's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    /// A floating-point number.
    Double,
    /// An unsigned integer.
    Uint,
    /// A boolean.
    Bool,
}

/// An animation's timing: its duration and its curve's samples.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// The duration in seconds, in `(0, 3600]`.
    pub seconds: f64,
    /// The curve's samples: finite, starting at 0 and ending at 1, monotonic, within `[0, 1]`.
    pub samples: Vec<f64>,
}

/// One operation of a transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Write a property, optionally as an animation.
    Set {
        /// The target node.
        node: NodeId,
        /// The property's key, as the schema names it.
        key: String,
        /// The value to write.
        value: Value,
        /// How to animate the write, or `None` to write it at once.
        timing: Option<Timing>,
    },
    /// Move a node under a parent, at `index` among its children or last.
    Insert {
        /// The node to move.
        node: NodeId,
        /// The new parent.
        parent: NodeId,
        /// The position among the parent's children, or `None` for last.
        index: Option<usize>,
    },
    /// Destroy a node and its subtree.
    Destroy {
        /// The node to destroy.
        node: NodeId,
    },
    /// Call a method on a node.
    Call {
        /// The target node.
        node: NodeId,
        /// The method's name, as the schema names it.
        method: String,
        /// The arguments.
        args: Vec<i64>,
    },
    /// Create a node of a class, with a caller-chosen id. It starts unparented.
    New {
        /// The node's class.
        class: String,
        /// The new node's id.
        id: NodeId,
    },
}

/// A batch of ops applied together.
#[derive(Clone, Debug, PartialEq)]
pub struct Transaction {
    /// The ops, in order.
    pub ops: Vec<Op>,
}

impl Transaction {
    /// A transaction of `ops`.
    #[must_use]
    pub const fn new(ops: Vec<Op>) -> Self {
        Self { ops }
    }
}

/// What the schema says about one property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyInfo {
    /// The kind of value the property holds.
    pub kind: ValueType,
    /// Whether writes are refused.
    pub readonly: bool,
    /// Whether the property may be animated.
    pub animated: bool,
}

/// The schema facts the transaction layer needs: which classes, properties and
/// methods exist. The compositor implements it from the schema catalog; tests use a
/// table.
pub trait ClassInfo {
    /// Whether the schema has a class of this name.
    fn has_class(&self, class: &str) -> bool;
    /// The property of `class` named `key`, or `None` when the class has none.
    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo>;
    /// Whether `class` has a method named `method`.
    fn has_method(&self, class: &str, method: &str) -> bool;
}

/// One applied change, in op order, for the mirror batch (design/node-core.md §4.6).
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A node was created. It starts unparented, at `index` among the root nodes.
    Created {
        /// The new node.
        node: NodeId,
        /// Its class.
        class: String,
        /// Its position among the root nodes.
        index: usize,
    },
    /// A node was moved under `parent` at `index` among its children.
    Parented {
        /// The moved node.
        node: NodeId,
        /// Its new parent.
        parent: NodeId,
        /// Its position among the parent's children.
        index: usize,
    },
    /// A property was set. For an animated set this is the target value.
    Set {
        /// The target node.
        node: NodeId,
        /// The property's key.
        key: String,
        /// The value.
        value: Value,
    },
    /// A node and its subtree were destroyed.
    Destroyed {
        /// The destroyed node.
        node: NodeId,
        /// Every node the scene removed.
        removed: Vec<NodeId>,
    },
}

/// An animated `set`, validated and handed to the compositor to play.
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    /// The target node.
    pub node: NodeId,
    /// The property's key.
    pub key: String,
    /// The target value.
    pub value: Value,
    /// The duration in seconds.
    pub seconds: f64,
    /// The curve's samples.
    pub samples: Vec<f64>,
}

/// A method call, validated and handed to the compositor to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// The target node.
    pub node: NodeId,
    /// The method's name.
    pub method: String,
    /// The arguments.
    pub args: Vec<i64>,
}

/// What a committed transaction changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// Every change, in op order.
    pub changes: Vec<Change>,
    /// Animated sets, for the compositor to play.
    pub animations: Vec<Animation>,
    /// Method calls, for the compositor to run.
    pub calls: Vec<Call>,
}

/// Validate every op of `transaction` as `actor` against `scene` and, when all pass,
/// apply them. A refusal leaves `scene` exactly as it was.
///
/// # Errors
///
/// Returns the first failing op's [`Refusal`]. A batch longer than [`MAX_OPS`] is
/// refused with `failed_op` `-1` before any op runs.
pub fn validate_and_apply(
    scene: &mut Scene,
    classes: &dyn ClassInfo,
    actor: Actor,
    transaction: &Transaction,
) -> Result<Applied, Refusal> {
    if transaction.ops.len() > MAX_OPS {
        return Err(Refusal {
            failed_op: -1,
            code: Code::TooLarge,
        });
    }
    let mut work = scene.clone();
    let mut applied = Applied::default();
    for (index, op) in transaction.ops.iter().enumerate() {
        apply_op(&mut work, classes, actor, op, &mut applied).map_err(|code| Refusal {
            failed_op: i32::try_from(index).expect("the batch is bounded by MAX_OPS"),
            code,
        })?;
    }
    *scene = work;
    Ok(applied)
}

fn apply_op(
    work: &mut Scene,
    classes: &dyn ClassInfo,
    actor: Actor,
    op: &Op,
    applied: &mut Applied,
) -> Result<(), Code> {
    match op {
        Op::New { class, id } => new_node(work, classes, actor, class, *id, applied),
        Op::Insert { node, parent, index } => insert_node(work, actor, *node, *parent, *index, applied),
        Op::Destroy { node } => destroy_node(work, actor, *node, applied),
        Op::Call { node, method, args } => call_method(work, classes, *node, method, args, applied),
        Op::Set {
            node,
            key,
            value,
            timing,
        } => set_value(work, classes, *node, key, value, timing.as_ref(), applied),
    }
}

fn new_node(
    work: &mut Scene,
    classes: &dyn ClassInfo,
    actor: Actor,
    class: &str,
    id: NodeId,
    applied: &mut Applied,
) -> Result<(), Code> {
    if !classes.has_class(class) {
        return Err(Code::UnknownClass);
    }
    work.insert_as(actor, id, NodeSpec::new(Rect::new(0, 0, 0, 0)))
        .map_err(|error| scene_code(&error))?;
    work.set_class(id, Some(class.to_owned()))
        .map_err(|error| scene_code(&error))?;
    applied.changes.push(Change::Created {
        node: id,
        class: class.to_owned(),
        index: position(work, id),
    });
    Ok(())
}

fn insert_node(
    work: &mut Scene,
    actor: Actor,
    node: NodeId,
    parent: NodeId,
    index: Option<usize>,
    applied: &mut Applied,
) -> Result<(), Code> {
    if !work.contains(node) || !work.contains(parent) {
        return Err(Code::UnknownNode);
    }
    if work.root() == Some(node) {
        return Err(Code::RootImmutable);
    }
    work.reparent_as_at(actor, node, Some(parent), index)
        .map_err(|error| scene_code(&error))?;
    applied.changes.push(Change::Parented {
        node,
        parent,
        index: position(work, node),
    });
    Ok(())
}

fn destroy_node(work: &mut Scene, actor: Actor, node: NodeId, applied: &mut Applied) -> Result<(), Code> {
    if !work.contains(node) {
        return Err(Code::UnknownNode);
    }
    if work.root() == Some(node) {
        return Err(Code::RootImmutable);
    }
    let removed = work.remove_as(actor, node).map_err(|error| scene_code(&error))?;
    applied.changes.push(Change::Destroyed { node, removed });
    Ok(())
}

fn call_method(
    work: &Scene,
    classes: &dyn ClassInfo,
    node: NodeId,
    method: &str,
    args: &[i64],
    applied: &mut Applied,
) -> Result<(), Code> {
    if !work.contains(node) {
        return Err(Code::UnknownNode);
    }
    let class = work.class(node).unwrap_or("");
    if !classes.has_method(class, method) {
        return Err(Code::UnknownMember);
    }
    applied.calls.push(Call {
        node,
        method: method.to_owned(),
        args: args.to_vec(),
    });
    Ok(())
}

fn set_value(
    work: &mut Scene,
    classes: &dyn ClassInfo,
    node: NodeId,
    key: &str,
    value: &Value,
    timing: Option<&Timing>,
    applied: &mut Applied,
) -> Result<(), Code> {
    if !work.contains(node) {
        return Err(Code::UnknownNode);
    }
    let info = classes
        .property(work.class(node).unwrap_or(""), key)
        .ok_or(Code::UnknownMember)?;
    if info.readonly {
        return Err(Code::ReadOnly);
    }
    if kind_of(value) != info.kind {
        return Err(Code::WrongType);
    }
    if let Value::Double(number) = value
        && !number.is_finite()
    {
        return Err(Code::NotFinite);
    }
    applied.changes.push(Change::Set {
        node,
        key: key.to_owned(),
        value: value.clone(),
    });
    if let Some(timing) = timing {
        if !info.animated {
            return Err(Code::NotAnimated);
        }
        check_timing(timing)?;
        applied.animations.push(Animation {
            node,
            key: key.to_owned(),
            value: value.clone(),
            seconds: timing.seconds,
            samples: timing.samples.clone(),
        });
        return Ok(());
    }
    write_value(work, node, key, value)
}

/// Check a timing: a duration in `(0, 3600]` and a curve from 0 to 1 that is
/// monotonic and stays within `[0, 1]`.
fn check_timing(timing: &Timing) -> Result<(), Code> {
    if !timing.seconds.is_finite() {
        return Err(Code::NotFinite);
    }
    if !(timing.seconds > 0.0 && timing.seconds <= MAX_SECONDS) {
        return Err(Code::OutOfRange);
    }
    if timing.samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Code::NotFinite);
    }
    let (Some(first), Some(last)) = (timing.samples.first(), timing.samples.last()) else {
        return Err(Code::BadCurve);
    };
    if *first != 0.0 || *last != 1.0 {
        return Err(Code::BadCurve);
    }
    if timing.samples.iter().any(|sample| !(0.0..=1.0).contains(sample)) {
        return Err(Code::BadCurve);
    }
    if timing.samples.windows(2).any(|pair| pair[1] < pair[0]) {
        return Err(Code::BadCurve);
    }
    Ok(())
}

/// Write one validated, non-animated value into the scene.
fn write_value(work: &mut Scene, node: NodeId, key: &str, value: &Value) -> Result<(), Code> {
    let result = match (key, value) {
        ("width" | "height", Value::Uint(amount)) => {
            let amount = u32::try_from(*amount).map_err(|_| Code::OutOfBounds)?;
            let mut bounds = work.bounds(node).ok_or(Code::UnknownNode)?;
            if key == "width" {
                bounds.width = amount;
            } else {
                bounds.height = amount;
            }
            work.set_bounds(node, bounds)
        }
        ("visible", Value::Bool(on)) => work.set_visible(node, *on),
        ("clip", Value::Bool(on)) => work.set_clip(node, *on),
        ("input", Value::Bool(on)) => work.set_input(node, *on),
        (key, Value::Double(number)) => {
            let property = visual_property(key).ok_or(Code::UnknownMember)?;
            work.set_property(node, property, PropertyValue::Number(*number as f32))
        }
        _ => return Err(Code::WrongType),
    };
    result.map_err(|error| scene_code(&error))
}

/// The visual property a schema key names, or `None` for a key the scene does not store.
fn visual_property(key: &str) -> Option<VisualProperty> {
    Some(match key {
        "opacity" => VisualProperty::Opacity,
        "offset_x" => VisualProperty::OffsetX,
        "offset_y" => VisualProperty::OffsetY,
        "offset_z" => VisualProperty::OffsetZ,
        "rotation_x" => VisualProperty::RotationX,
        "rotation_y" => VisualProperty::RotationY,
        "rotation_z" => VisualProperty::RotationZ,
        "scale_x" => VisualProperty::ScaleX,
        "scale_y" => VisualProperty::ScaleY,
        "scale_z" => VisualProperty::ScaleZ,
        _ => return None,
    })
}

const fn kind_of(value: &Value) -> ValueType {
    match value {
        Value::Double(_) => ValueType::Double,
        Value::Uint(_) => ValueType::Uint,
        Value::Bool(_) => ValueType::Bool,
    }
}

/// The position of `node` among its parent's children, or among the root nodes.
fn position(scene: &Scene, node: NodeId) -> usize {
    let siblings = match scene.parent(node) {
        Some(parent) => scene.children(parent).unwrap_or(&[]),
        None => scene.root_ids(),
    };
    siblings
        .iter()
        .position(|child| *child == node)
        .expect("a node is among its parent's children")
}

/// The table 4.4 code for a scene refusal.
const fn scene_code(error: &SceneError) -> Code {
    match error {
        SceneError::DuplicateNode(_) => Code::DuplicateNode,
        SceneError::UnknownNode(_) | SceneError::UnknownParent(_) => Code::UnknownNode,
        SceneError::NotOwner(_) => Code::NotOwner,
        SceneError::Cycle { .. } => Code::Cycle,
        SceneError::OutOfBounds { .. } => Code::OutOfBounds,
        SceneError::TooManyNodes => Code::TooManyNodes,
        SceneError::TooDeep => Code::TooDeep,
        SceneError::NonFinite { .. } => Code::NotFinite,
        SceneError::IndexOutOfRange { .. } => Code::OutOfRange,
        _ => Code::BadValue,
    }
}
