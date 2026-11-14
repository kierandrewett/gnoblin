//! TreeTransaction validation and apply (design/node-core.md §4).
//!
//! A transaction is a batch of [`TreeOp`]s sent together. [`validate_and_apply`] checks
//! each op in order against the scene as the earlier ops left it, and it applies the
//! batch all or nothing. The first op that fails refuses the whole batch with a
//! [`TreeRefusal`] that names that op's index; the scene is then exactly as it was.
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
//!   the property's (`wrong-type`); a number is finite (`not-finite`); a text, byte
//!   string or list is within its cap (`too-large`); a node reference names a live node
//!   of the property's class (`unknown-node`, `wrong-type`); then, for a `timing`, the
//!   property is an animated double (`not-animated`), `seconds` is finite and in
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
//! A `set` with a `timing` is validated and recorded in [`TreeApplied::motion`]; the
//! scene's value is not written here. The compositor plays the track later. The
//! change is still listed in [`TreeApplied::changes`] so a mirror learns the target.
//!
//! # TreeRefusal codes
//!
//! [`Code::as_str`] gives each code's spelling in design/node-core.md table 4.4.
//! Codes that belong to other slices (`stale-base`, `invalid-serial`,
//! `not-privileged`, `shell-already-attached`, `not-holder`, `bad-paint`) are declared so the
//! spellings live in one place; this module never returns them. A scene refusal
//! with no table code maps to `bad-value`.

use std::collections::BTreeSet;

use gnoblin_animation::{PropertyValue, VisualProperty};
use gnoblin_secret::SecretText;
use gnoblin_types::{NodeId, Rect};

use crate::effect::{EffectError, Effects, MAX_STACKED, Pass};
use crate::scene::{NodeSpec, Scene, SceneError};
use crate::scene_ids::Actor;

/// The most ops one transaction may carry. A longer batch is refused `too-large`.
pub const MAX_OPS: usize = 4096;

/// The longest animation a `set` may ask for, in seconds.
const MAX_SECONDS: f64 = 3600.0;

/// The longest text value, in bytes. A longer one is refused `too-large`.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// The largest byte string, in bytes. A larger one is refused `too-large`.
pub const MAX_BYTES: usize = 1 << 20;

/// The most items in one list value. A longer list is refused `too-large`.
pub const MAX_LIST: usize = 4096;

/// The most effect passes one node's `effects` list may hold. A longer list is refused
/// `too-large`.
pub const MAX_EFFECTS: usize = 16;

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
    /// The connection is an observer, and the op is one only the tree's holder may send.
    NotHolder,
    /// An observer wrote the head camera's pose faster than the compositor takes it (`ipc::tree::PoseLimiter`).
    RateLimited,
    /// A `Canvas`'s `content` does not decode as a paint list.
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
            Self::NotHolder => "not-holder",
            Self::RateLimited => "rate-limited",
            Self::BadPaint => "bad-paint",
            Self::BadValue => "bad-value",
        }
    }
}

/// A refused transaction: the index of the op that failed (`-1` when the batch as a
/// whole was refused) and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRefusal {
    /// The zero-based index of the failing op, or `-1` for the whole batch.
    pub failed_op: i32,
    /// The refusal code.
    pub code: Code,
}

impl std::fmt::Display for TreeRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "op {} refused: {}", self.failed_op, self.code.as_str())
    }
}

/// A value a `set` writes.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeValue {
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    Int(i64),
    /// An unsigned 64-bit integer.
    Uint(u64),
    /// A floating-point number.
    Double(f64),
    /// A string.
    Text(String),
    /// A byte string.
    Bytes(Vec<u8>),
    /// A list of numbers.
    ListDouble(Vec<f64>),
    /// A list of strings.
    ListText(Vec<String>),
    /// A list of effect passes, at most [`MAX_EFFECTS`] long. Only blur and shadow exist;
    /// the tree has no user shader pass, so any other kind is refused before it gets here.
    Effects(Vec<Pass>),
    /// A node reference, or `None` for nil.
    Node(Option<NodeId>),
}

/// The kind of a property's value. `Node` names the class a reference must be of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
    /// A boolean.
    Bool,
    /// A signed 64-bit integer.
    Int,
    /// An unsigned 64-bit integer.
    Uint,
    /// A finite floating-point number.
    Double,
    /// A UTF-8 string of at most [`MAX_TEXT_BYTES`] bytes.
    Text,
    /// A byte string of at most [`MAX_BYTES`] bytes.
    Bytes,
    /// A list of at most [`MAX_LIST`] finite numbers.
    ListDouble,
    /// A list of at most [`MAX_LIST`] strings, each at most [`MAX_TEXT_BYTES`] bytes.
    ListText,
    /// A list of at most [`MAX_EFFECTS`] blur or shadow passes, each with finite,
    /// in-range values.
    Effects,
    /// A reference to a live node whose class is this class or derives from it, or nil.
    Node(String),
}

/// An animation's timing: its delay, its duration and its curve's samples.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// The duration in seconds, in `(0, 3600]`.
    pub seconds: f64,
    /// The delay before the animation starts, in seconds, in `[0, 3600]`. The start value is held meanwhile.
    pub delay: f64,
    /// The `tween` node the tracks of this write join, so its `progress`, `paused` and `reverse` drive them; `None`
    /// for a write that only plays. The node must exist and be a `tween` (`unknown-node`, `wrong-type`).
    pub group: Option<NodeId>,
    /// The curve's samples: finite, starting at 0 and ending at 1, monotonic, within `[0, 1]`.
    pub samples: Vec<f64>,
}

impl Timing {
    /// A timing with no delay that joins no `tween`: the animation that only plays.
    #[must_use]
    pub fn new(seconds: f64, samples: Vec<f64>) -> Self {
        Self {
            seconds,
            delay: 0.0,
            group: None,
            samples,
        }
    }
}

/// One operation of a transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeOp {
    /// Write a property, optionally as an animation.
    Set {
        /// The target node.
        node: NodeId,
        /// The property's key, as the schema names it.
        key: String,
        /// The value to write.
        value: TreeValue,
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
    /// Deliver a pointer input to the client of a window, on a seat's pointer. It changes
    /// nothing in the scene, so it is an addition-only op the owner may send.
    Input {
        /// The window node the pointer is over.
        node: NodeId,
        /// The seat node whose pointer carries the input.
        seat: NodeId,
        /// The input itself, in the window's own coordinates.
        input: PointerInput,
    },
    /// TreeCall a method on a node.
    TreeCall {
        /// The target node.
        node: NodeId,
        /// The method's name, as the schema names it.
        method: String,
        /// The arguments.
        args: Vec<i64>,
        /// The text argument, such as a claim's chord; empty for every other call.
        text: SecretText,
        /// The token the compositor answers the call under, echoed back in `answer`.
        token: u64,
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
pub struct TreeTransaction {
    /// The ops, in order.
    pub ops: Vec<TreeOp>,
    /// Whether an observer connection sent it (`tree.attach` mode `observer`), not the tree's holder. An observer's
    /// `set` is checked against what an observer may write (see [`observer_write`]), as well as against the schema.
    pub observer: bool,
}

impl TreeTransaction {
    /// A transaction of `ops`, from the tree's holder.
    #[must_use]
    pub const fn new(ops: Vec<TreeOp>) -> Self {
        Self { ops, observer: false }
    }

    /// The same transaction, sent by an observer connection.
    #[must_use]
    pub const fn from_observer(mut self) -> Self {
        self.observer = true;
        self
    }
}

/// What the schema says about one property.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Whether `class` is `ancestor` or derives from it. A class is its own ancestor.
    fn is_a(&self, class: &str, ancestor: &str) -> bool {
        class == ancestor
    }
    /// The property of `class` named `key`, or `None` when the class has none.
    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo>;
    /// Whether `class` has a method named `method`.
    fn has_method(&self, class: &str, method: &str) -> bool;
}

/// One applied change, in op order, for the mirror batch (design/node-core.md §4.6).
#[derive(Clone, Debug, PartialEq)]
pub enum TreeChange {
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
        value: TreeValue,
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
    pub value: TreeValue,
    /// The duration in seconds.
    pub seconds: f64,
    /// The delay before it starts, in seconds.
    pub delay: f64,
    /// The `tween` node whose `progress`, `paused` and `reverse` drive the tracks of this write, if the write named one.
    pub group: Option<NodeId>,
    /// The curve's samples.
    pub samples: Vec<f64>,
}

/// What a write to an animated property asks of the compositor's tracks.
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    /// An animated `set`: play a track to the target.
    Play(Animation),
    /// A `set` without timing: the value was written at once, so a track running on the
    /// property must end rather than overwrite it.
    Settle {
        /// The node written.
        node: NodeId,
        /// The property's key.
        key: String,
    },
    /// A write to a `tween` node's `progress`, `paused` or `reverse`: the tracks of its group are moved, held or turned.
    Control {
        /// The `tween` node written.
        node: NodeId,
        /// What the write asks.
        control: TweenControl,
    },
}

/// What a write to a `tween` node asks of the tracks that joined it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TweenControl {
    /// `progress`: every track of the group is moved to this fraction of its duration, drawn at once.
    Progress(f64),
    /// `paused`: the group's tracks are held (`true`) or let go on.
    Paused(bool),
    /// `reverse`: the group's tracks run back to their start (`true`) or forward again.
    Reverse(bool),
}

/// The end of one timed `set`, reported to the shell as the `track_ended` signal: the compositor's clock says when an
/// animation is over, not the shell's.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellTrackEnd {
    /// The node the `set` wrote.
    pub node: NodeId,
    /// The property's key, as the schema names it.
    pub key: String,
    /// The `tween` node the write joined, if it named one.
    pub group: Option<NodeId>,
    /// Whether every track of the write reached its target. `false` for any other end: run back to the start,
    /// taken by a plain write or a newer animation, or the node going.
    pub completed: bool,
}

/// A method call, validated and handed to the compositor to run.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeCall {
    /// The target node.
    pub node: NodeId,
    /// The method's name.
    pub method: String,
    /// The arguments.
    pub args: Vec<i64>,
    /// The text argument; empty for a call that takes none. Secret: a password is one.
    pub text: SecretText,
    /// The token the answer carries back.
    pub token: u64,
}

/// A pointer input a window's client receives through a seat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerInput {
    /// The pointer moved to `(x, y)` in the window's coordinates, entering the window first if
    /// it was not over it.
    Motion { x: f64, y: f64 },
    /// A button changed state at `(x, y)`; `pressed` is `true` for a press.
    Button { x: f64, y: f64, button: u32, pressed: bool },
    /// A scroll of `(dx, dy)` at `(x, y)`.
    Scroll { x: f64, y: f64, dx: f64, dy: f64 },
    /// The pointer left the window.
    Leave,
}

/// A pointer input addressed to a window through a seat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeatInput {
    /// The window node.
    pub node: NodeId,
    /// The seat node.
    pub seat: NodeId,
    /// The input.
    pub input: PointerInput,
}

/// What a committed transaction changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeApplied {
    /// Every change, in op order.
    pub changes: Vec<TreeChange>,
    /// What the shell's writes to animated properties ask of the compositor's tracks, in op order: a
    /// track to play, or a track to end because the property was set at once.
    pub motion: Vec<Motion>,
    /// Method calls, for the compositor to run.
    pub calls: Vec<TreeCall>,
    /// Pointer inputs, for the compositor to deliver to clients in op order.
    pub inputs: Vec<SeatInput>,
}

/// Validate every op of `transaction` as `actor` against `scene` and, when all pass,
/// apply them. A refusal leaves `scene` exactly as it was. The transaction is consumed: its
/// ops move into the applied result, so a call's secret text is never copied, and is wiped when
/// a refusal drops it.
///
/// # Errors
///
/// Returns the first failing op's [`TreeRefusal`]. A batch longer than [`MAX_OPS`] is
/// refused with `failed_op` `-1` before any op runs.
pub fn validate_and_apply(
    scene: &mut Scene,
    classes: &dyn ClassInfo,
    actor: Actor,
    transaction: TreeTransaction,
) -> Result<TreeApplied, TreeRefusal> {
    if transaction.ops.len() > MAX_OPS {
        return Err(TreeRefusal {
            failed_op: -1,
            code: Code::TooLarge,
        });
    }
    let observer = transaction.observer;
    let mut work = scene.clone();
    let mut applied = TreeApplied::default();
    let mut created = BTreeSet::new();
    for (index, op) in transaction.ops.into_iter().enumerate() {
        apply_op(&mut work, classes, actor, observer, op, &mut created, &mut applied).map_err(|code| TreeRefusal {
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
    observer: bool,
    op: TreeOp,
    created: &mut BTreeSet<NodeId>,
    applied: &mut TreeApplied,
) -> Result<(), Code> {
    match op {
        TreeOp::New { class, id } => new_node(work, classes, actor, &class, id, created, applied),
        TreeOp::Insert { node, parent, index } => insert_node(work, actor, node, parent, index, applied),
        TreeOp::Destroy { node } => destroy_node(work, actor, node, applied),
        TreeOp::Input { node, seat, input } => pointer_input(work, actor, node, seat, input, applied),
        TreeOp::TreeCall {
            node,
            method,
            args,
            text,
            token,
        } => call_method(
            work,
            classes,
            TreeCall {
                node,
                method,
                args,
                text,
                token,
            },
            applied,
        ),
        TreeOp::Set {
            node,
            key,
            value,
            timing,
        } => {
            let exempt = observer && observer_write(work, created, node, &key, timing.is_some())?;
            if !exempt {
                check_writable(work, classes, node, &key, created)?;
            }
            set_value(work, classes, node, &key, &value, timing.as_ref(), applied)
        }
    }
}

fn new_node(
    work: &mut Scene,
    classes: &dyn ClassInfo,
    actor: Actor,
    class: &str,
    id: NodeId,
    created: &mut BTreeSet<NodeId>,
    applied: &mut TreeApplied,
) -> Result<(), Code> {
    if !classes.has_class(class) {
        return Err(Code::UnknownClass);
    }
    work.insert_as(actor, id, NodeSpec::new(Rect::new(0, 0, 0, 0)))
        .map_err(|error| scene_code(&error))?;
    work.set_class(id, Some(class.to_owned()))
        .map_err(|error| scene_code(&error))?;
    created.insert(id);
    applied.changes.push(TreeChange::Created {
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
    applied: &mut TreeApplied,
) -> Result<(), Code> {
    if !work.contains(node) || !work.contains(parent) {
        return Err(Code::UnknownNode);
    }
    if work.root() == Some(node) {
        return Err(Code::RootImmutable);
    }
    work.reparent_as_at(actor, node, Some(parent), index)
        .map_err(|error| scene_code(&error))?;
    applied.changes.push(TreeChange::Parented {
        node,
        parent,
        index: position(work, node),
    });
    Ok(())
}

fn destroy_node(work: &mut Scene, actor: Actor, node: NodeId, applied: &mut TreeApplied) -> Result<(), Code> {
    if !work.contains(node) {
        return Err(Code::UnknownNode);
    }
    if work.root() == Some(node) {
        return Err(Code::RootImmutable);
    }
    let removed = work.remove_as(actor, node).map_err(|error| scene_code(&error))?;
    applied.changes.push(TreeChange::Destroyed { node, removed });
    Ok(())
}

fn call_method(work: &Scene, classes: &dyn ClassInfo, call: TreeCall, applied: &mut TreeApplied) -> Result<(), Code> {
    if !work.contains(call.node) {
        return Err(Code::UnknownNode);
    }
    let class = work.class(call.node).unwrap_or("");
    if !classes.has_method(class, &call.method) {
        return Err(Code::UnknownMember);
    }
    applied.calls.push(call);
    Ok(())
}

/// A pointer input to a window through a seat. Only the shell may send one: it is not a scene
/// change, so the compositor delivers it and records nothing.
fn pointer_input(
    work: &Scene,
    actor: Actor,
    node: NodeId,
    seat: NodeId,
    input: PointerInput,
    applied: &mut TreeApplied,
) -> Result<(), Code> {
    if actor != Actor::Shell {
        return Err(Code::NotOwner);
    }
    match work.class(node) {
        None => return Err(Code::UnknownNode),
        Some("window") => {}
        Some(_) => return Err(Code::BadValue),
    }
    if !work.seat_names().any(|(id, _)| id == seat) {
        return Err(Code::BadValue);
    }
    let finite = match input {
        PointerInput::Motion { x, y } | PointerInput::Button { x, y, .. } => x.is_finite() && y.is_finite(),
        PointerInput::Scroll { x, y, dx, dy } => [x, y, dx, dy].iter().all(|v| v.is_finite()),
        PointerInput::Leave => true,
    };
    if !finite {
        return Err(Code::NotFinite);
    }
    applied.inputs.push(SeatInput { node, seat, input });
    Ok(())
}

/// A read-only property is set only in the transaction that creates its node.
fn check_writable(
    work: &Scene,
    classes: &dyn ClassInfo,
    node: NodeId,
    key: &str,
    created: &BTreeSet<NodeId>,
) -> Result<(), Code> {
    if !work.contains(node) {
        return Err(Code::UnknownNode);
    }
    let info = classes
        .property(work.class(node).unwrap_or(""), key)
        .ok_or(Code::UnknownMember)?;
    if info.readonly && !created.contains(&node) {
        return Err(Code::ReadOnly);
    }
    Ok(())
}

/// The class and name of the one camera an observer may write: the XR extension's head camera.
const OBSERVER_CAMERA: (&str, &str) = ("camera", "head");

/// The properties of a `controller` an observer writes: its pose and the values of the hand. `claimed` is not among
/// them, the shell writes it.
pub const OBSERVER_CONTROLLER_KEYS: [&str; 13] = [
    "x",
    "y",
    "z",
    "rotation",
    "hand",
    "tracked",
    "trigger",
    "grip",
    "thumbstick_x",
    "thumbstick_y",
    "thumbstick_click",
    "primary",
    "secondary",
];

/// What an observer's `set` may touch, beyond naming the seat it made in the same transaction: the `x`, `y`, `z` and
/// `rotation` of the camera named `head` (the head pose the XR extension measures) and that camera's read-only
/// `layer_budget` (the runtime's layer limit), and the pose and values of a `controller` (the XR extension's hands,
/// which are read-only for everyone else). It may not animate, and it may write nothing else. `Ok(true)` means the
/// write is one of the observer's own read-only properties, so the schema's `readonly` does not refuse it.
///
/// The pure check that runs before the transaction is queued (`ipc::tree::observer_ops`) cannot see the scene, so
/// it admits the pose keys on any node; this is the check that holds them to the head camera and to controllers. The
/// pure check is the one that holds a controller's other values to the controllers the observer made.
fn observer_write(
    work: &Scene,
    created: &BTreeSet<NodeId>,
    node: NodeId,
    key: &str,
    timed: bool,
) -> Result<bool, Code> {
    if !work.contains(node) {
        return Err(Code::UnknownNode);
    }
    if timed {
        return Err(Code::NotHolder);
    }
    let class = work.class(node);
    if class == Some("controller") && OBSERVER_CONTROLLER_KEYS.contains(&key) {
        return Ok(true);
    }
    if created.contains(&node) {
        // The seat the observer is making: only its name, which the pure check already holds to.
        return if class == Some("seat") && key == "name" {
            Ok(false)
        } else {
            Err(Code::NotHolder)
        };
    }
    let head = class == Some(OBSERVER_CAMERA.0) && work.text(node, "name") == Some(OBSERVER_CAMERA.1);
    match key {
        "x" | "y" | "z" | "rotation" if head => Ok(false),
        "layer_budget" if head => Ok(true),
        _ => Err(Code::NotHolder),
    }
}

fn set_value(
    work: &mut Scene,
    classes: &dyn ClassInfo,
    node: NodeId,
    key: &str,
    value: &TreeValue,
    timing: Option<&Timing>,
    applied: &mut TreeApplied,
) -> Result<(), Code> {
    if !work.contains(node) {
        return Err(Code::UnknownNode);
    }
    let info = classes
        .property(work.class(node).unwrap_or(""), key)
        .ok_or(Code::UnknownMember)?;
    if work.class(node) == Some("seat") && key == "name" {
        return set_seat_name(work, node, value, applied);
    }
    if work.class(node) == Some("seat") && key == "focus" {
        return set_seat_focus(work, node, value, applied);
    }
    check_value(work, classes, value, &info.kind)?;
    check_paint(work, node, key, value)?;
    applied.changes.push(TreeChange::Set {
        node,
        key: key.to_owned(),
        value: value.clone(),
    });
    let control = if work.class(node) == Some("tween") {
        tween_control(key, value)?
    } else {
        None
    };
    if let Some(timing) = timing {
        let list = info.kind == ValueType::ListDouble && matches!(key, "rotation" | "scale");
        if !info.animated || !(info.kind == ValueType::Double || list) {
            return Err(Code::NotAnimated);
        }
        check_timing(timing)?;
        check_group(work, timing.group)?;
        match value {
            TreeValue::ListDouble(axes) if axes.len() != 3 => return Err(Code::BadValue),
            // A size is whole pixels whether it lands at once or in steps: a fraction is refused here, not
            // when the track ends.
            TreeValue::Double(amount) if matches!(key, "width" | "height") => {
                whole_size(*amount)?;
            }
            _ => {}
        }
        applied.motion.push(Motion::Play(Animation {
            node,
            key: key.to_owned(),
            value: value.clone(),
            seconds: timing.seconds,
            delay: timing.delay,
            group: timing.group,
            samples: timing.samples.clone(),
        }));
        return Ok(());
    }
    write_value(work, node, key, value)?;
    if info.animated {
        applied.motion.push(Motion::Settle {
            node,
            key: key.to_owned(),
        });
    }
    if let Some(control) = control {
        applied.motion.push(Motion::Control { node, control });
    }
    Ok(())
}

/// Writes an animated set's target into `scene` at once, as an untimed `set` would. The
/// compositor uses it when a track is not played: under reduced motion, or for a property the
/// scene cannot tween.
///
/// # Errors
///
/// Returns the refusal an untimed `set` of the same value would have drawn, such as a width
/// that is not a whole number.
pub fn land_at_once(scene: &mut Scene, animation: &Animation) -> Result<(), Code> {
    write_value(scene, animation.node, &animation.key, &animation.value)
}

/// Set the keyboard focus of a seat node to a window node, or clear it with null. The target
/// must exist and be a window; anything else is refused as a bad value.
fn set_seat_focus(work: &mut Scene, node: NodeId, value: &TreeValue, applied: &mut TreeApplied) -> Result<(), Code> {
    let TreeValue::Node(target) = value else {
        return Err(Code::WrongType);
    };
    if let Some(target) = target
        && (!work.contains(*target) || (work.class(*target) != Some("window") && work.input(*target) != Some(true)))
    {
        return Err(Code::BadValue);
    }
    work.set_reference(node, "focus", *target)
        .map_err(|error| scene_code(&error))?;
    applied.changes.push(TreeChange::Set {
        node,
        key: "focus".to_owned(),
        value: TreeValue::Node(*target),
    });
    Ok(())
}

/// Name a seat node. The name is non-empty, at most [`MAX_TEXT_BYTES`] bytes and unique among
/// the seats, so a name set twice or taken by another seat is refused.
fn set_seat_name(work: &mut Scene, node: NodeId, value: &TreeValue, applied: &mut TreeApplied) -> Result<(), Code> {
    let TreeValue::Text(name) = value else {
        return Err(Code::WrongType);
    };
    if name.is_empty() || name.len() > MAX_TEXT_BYTES {
        return Err(Code::BadValue);
    }
    let taken = work.seat_names().any(|(id, other)| id != node && other == name);
    if taken {
        return Err(Code::BadValue);
    }
    work.set_seat_name(node, name.clone());
    applied.changes.push(TreeChange::Set {
        node,
        key: "name".to_owned(),
        value: TreeValue::Text(name.clone()),
    });
    Ok(())
}

/// Check a timing: a duration in `(0, 3600]`, a delay in `[0, 3600]` and a curve from 0 to 1 that is
/// monotonic and stays within `[0, 1]`.
fn check_timing(timing: &Timing) -> Result<(), Code> {
    if !timing.seconds.is_finite() || !timing.delay.is_finite() {
        return Err(Code::NotFinite);
    }
    if !(timing.seconds > 0.0 && timing.seconds <= MAX_SECONDS) || !(0.0..=MAX_SECONDS).contains(&timing.delay) {
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

/// The `tween` a timed `set` joins must be a live `tween` node.
fn check_group(work: &Scene, group: Option<NodeId>) -> Result<(), Code> {
    match group {
        None => Ok(()),
        Some(node) if !work.contains(node) => Err(Code::UnknownNode),
        Some(node) if work.class(node) == Some("tween") => Ok(()),
        Some(_) => Err(Code::WrongType),
    }
}

/// What a write to a `tween` node's `progress`, `paused` or `reverse` asks of the tracks of its group; `None` for any
/// other key. A `progress` outside `[0, 1]` is refused `out-of-range`.
fn tween_control(key: &str, value: &TreeValue) -> Result<Option<TweenControl>, Code> {
    Ok(match (key, value) {
        ("progress", TreeValue::Double(progress)) => {
            if !(0.0..=1.0).contains(progress) {
                return Err(Code::OutOfRange);
            }
            Some(TweenControl::Progress(*progress))
        }
        ("paused", TreeValue::Bool(paused)) => Some(TweenControl::Paused(*paused)),
        ("reverse", TreeValue::Bool(reverse)) => Some(TweenControl::Reverse(*reverse)),
        _ => None,
    })
}

/// Check a value against its property's kind and caps. `work` resolves node references.
fn check_value(work: &Scene, classes: &dyn ClassInfo, value: &TreeValue, kind: &ValueType) -> Result<(), Code> {
    match (value, kind) {
        (TreeValue::Bool(_), ValueType::Bool)
        | (TreeValue::Int(_), ValueType::Int)
        | (TreeValue::Uint(_), ValueType::Uint) => Ok(()),
        (TreeValue::Double(number), ValueType::Double) => finite(*number),
        (TreeValue::Text(text), ValueType::Text) => size_within(text.len(), MAX_TEXT_BYTES),
        (TreeValue::Bytes(bytes), ValueType::Bytes) => size_within(bytes.len(), MAX_BYTES),
        (TreeValue::ListDouble(items), ValueType::ListDouble) => {
            size_within(items.len(), MAX_LIST)?;
            items.iter().try_for_each(|item| finite(*item))
        }
        (TreeValue::ListText(items), ValueType::ListText) => {
            size_within(items.len(), MAX_LIST)?;
            items
                .iter()
                .try_for_each(|item| size_within(item.len(), MAX_TEXT_BYTES))
        }
        (TreeValue::Effects(passes), ValueType::Effects) => {
            size_within(passes.len(), MAX_EFFECTS)?;
            for kind in ["border", "shadow"] {
                size_within(passes.iter().filter(|pass| pass.name() == kind).count(), MAX_STACKED)?;
            }
            passes.iter().try_for_each(check_effect_pass)
        }
        (TreeValue::Node(target), ValueType::Node(class)) => match target {
            None => Ok(()),
            Some(id) if !work.contains(*id) => Err(Code::UnknownNode),
            Some(id) if classes.is_a(work.class(*id).unwrap_or(""), class) => Ok(()),
            Some(_) => Err(Code::WrongType),
        },
        _ => Err(Code::WrongType),
    }
}

/// A `Canvas`'s `content` must decode as a paint list. A value that does not is refused `bad-paint` (F42), the
/// transaction with it, so the canvas keeps the content it had.
fn check_paint(work: &Scene, node: NodeId, key: &str, value: &TreeValue) -> Result<(), Code> {
    if let TreeValue::Bytes(bytes) = value
        && key == "content"
        && work.class(node) == Some("canvas")
    {
        gnoblin_paint::decode(bytes).map_err(|_| Code::BadPaint)?;
    }
    Ok(())
}

fn finite(number: f64) -> Result<(), Code> {
    if number.is_finite() {
        Ok(())
    } else {
        Err(Code::NotFinite)
    }
}

fn size_within(len: usize, cap: usize) -> Result<(), Code> {
    if len <= cap { Ok(()) } else { Err(Code::TooLarge) }
}

/// Check one pass of an `effects` list: its values must be finite and in range. A value that
/// is NaN or infinite is `not-finite`; any other out-of-range value is `out-of-range`.
fn check_effect_pass(pass: &Pass) -> Result<(), Code> {
    pass.validate().map_err(|error| match error {
        EffectError::Unsupported3d(_) => Code::BadValue,
        _ if pass_has_non_finite(pass) => Code::NotFinite,
        _ => Code::OutOfRange,
    })
}

/// Whether any number in a blur or shadow pass is NaN or infinite.
fn pass_has_non_finite(pass: &Pass) -> bool {
    match pass {
        Pass::Blur(blur) => [
            blur.radius,
            blur.downscale,
            blur.brightness,
            blur.contrast,
            blur.saturation,
            blur.noise,
            blur.tint.r,
            blur.tint.g,
            blur.tint.b,
            blur.tint.a,
        ]
        .iter()
        .any(|value| !value.is_finite()),
        Pass::Shadow(shadow) => [
            shadow.offset.x,
            shadow.offset.y,
            shadow.offset.z,
            f64::from(shadow.blur),
            f64::from(shadow.spread),
            f64::from(shadow.opacity),
            f64::from(shadow.color.r),
            f64::from(shadow.color.g),
            f64::from(shadow.color.b),
            f64::from(shadow.color.a),
        ]
        .iter()
        .any(|value| !value.is_finite()),
        Pass::Border(border) => [
            border.width,
            border.offset,
            border.color.r,
            border.color.g,
            border.color.b,
            border.color.a,
        ]
        .iter()
        .any(|value| !value.is_finite()),
    }
}

/// Write one validated, non-animated value into the scene, then keep it as the node's value for
/// `key` so the snapshot can carry it. A key the scene draws from is written to its typed field.
fn write_value(work: &mut Scene, node: NodeId, key: &str, value: &TreeValue) -> Result<(), Code> {
    if let ("corner_radius", TreeValue::Double(radius)) = (key, value)
        && *radius < 0.0
    {
        return Err(Code::OutOfRange);
    }
    if let TreeValue::ListDouble(axes) = value
        && matches!(key, "rotation" | "scale")
        && axes.len() != 3
    {
        return Err(Code::BadValue);
    }
    let result = match (key, value) {
        ("width" | "height", TreeValue::Double(amount)) => {
            let amount = whole_size(*amount)?;
            let mut bounds = work.bounds(node).ok_or(Code::UnknownNode)?;
            if key == "width" {
                bounds.width = amount;
            } else {
                bounds.height = amount;
            }
            work.set_bounds(node, bounds)
        }
        ("visible", TreeValue::Bool(on)) => work.set_visible(node, *on),
        ("clip", TreeValue::Bool(on)) => work.set_clip(node, *on),
        ("input", TreeValue::Bool(on)) => work.set_input(node, *on),
        ("rotation", TreeValue::ListDouble(axes)) => set_axes(
            work,
            node,
            axes,
            [
                VisualProperty::RotationX,
                VisualProperty::RotationY,
                VisualProperty::RotationZ,
            ],
        ),
        ("scale", TreeValue::ListDouble(axes)) => set_axes(
            work,
            node,
            axes,
            [VisualProperty::ScaleX, VisualProperty::ScaleY, VisualProperty::ScaleZ],
        ),
        ("source", TreeValue::Node(target)) => work.set_source(node, *target),
        ("camera", TreeValue::Node(target)) => work.set_camera(node, *target),
        ("effects", TreeValue::Effects(passes)) => effect_chain(passes).and_then(|chain| work.set_effects(node, chain)),
        (key, TreeValue::Double(number)) => match visual_property(key) {
            Some(property) => work.set_property(node, property, PropertyValue::Number(*number as f32)),
            None => Ok(()),
        },
        _ if !is_stored(key) => Ok(()),
        _ => return Err(Code::WrongType),
    };
    result.map_err(|error| scene_code(&error))?;
    work.set_value(node, key, value.clone())
        .map_err(|error| scene_code(&error))
}

/// The scene's effect chain for the passes of an `effects` list. The list was checked against
/// [`MAX_EFFECTS`] and each pass was validated, so a refusal here is a bug in that check, not
/// a client's value.
fn effect_chain(passes: &[Pass]) -> Result<Effects, SceneError> {
    let mut chain = Effects::new();
    for pass in passes {
        chain.push(pass.clone())?;
    }
    Ok(chain)
}

/// Whether the scene stores `key`. A `set` to any other key is accepted and listed as changed.
fn is_stored(key: &str) -> bool {
    matches!(
        key,
        "width" | "height" | "visible" | "clip" | "input" | "rotation" | "scale" | "source" | "camera"
    ) || visual_property(key).is_some()
}

fn set_axes(work: &mut Scene, node: NodeId, axes: &[f64], properties: [VisualProperty; 3]) -> Result<(), SceneError> {
    for (property, value) in properties.into_iter().zip(axes) {
        work.set_property(node, property, PropertyValue::Number(*value as f32))?;
    }
    Ok(())
}

/// A width or height: a whole, non-negative number that fits the scene's bounds.
fn whole_size(amount: f64) -> Result<u32, Code> {
    if amount.fract() != 0.0 || amount < 0.0 {
        return Err(Code::BadValue);
    }
    u32::try_from(amount as u64).map_err(|_| Code::OutOfBounds)
}

/// The scene's visual property a schema key maps to, or `None` for a key the scene does
/// not store as one number of the transform.
pub fn visual_property(key: &str) -> Option<VisualProperty> {
    Some(match key {
        "opacity" => VisualProperty::Opacity,
        "corner_radius" => VisualProperty::CornerRadius,
        "x" => VisualProperty::OffsetX,
        "y" => VisualProperty::OffsetY,
        "z" => VisualProperty::OffsetZ,
        _ => return None,
    })
}

/// The scene properties a timed `set` of `key` moves, one track each: a number is one property, a width or a height
/// the node's size, and the three-axis `rotation` and `scale` one property per axis (the node model's own rule:
/// rotation is stored per axis, so it tweens per axis, and a scale is linear). Empty for a key that cannot be
/// tweened.
#[must_use]
pub fn animated_properties(key: &str) -> &'static [VisualProperty] {
    match key {
        "opacity" => &[VisualProperty::Opacity],
        "corner_radius" => &[VisualProperty::CornerRadius],
        "x" => &[VisualProperty::OffsetX],
        "y" => &[VisualProperty::OffsetY],
        "z" => &[VisualProperty::OffsetZ],
        "width" => &[VisualProperty::Width],
        "height" => &[VisualProperty::Height],
        "rotation" => &[
            VisualProperty::RotationX,
            VisualProperty::RotationY,
            VisualProperty::RotationZ,
        ],
        "scale" => &[VisualProperty::ScaleX, VisualProperty::ScaleY, VisualProperty::ScaleZ],
        _ => &[],
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
