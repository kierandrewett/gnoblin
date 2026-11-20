//! Staged writes: what a transition's callback does to the tree is held here until the transition commits.
//!
//! This is `data/js/gnoblin-std/transition/staging.ts`. While a transition is being staged, a write of the shell's (`transition_set`)
//! and a write of the library's own passes (the layout pass puts a window's rectangle through `nodes::write_num`) are kept by
//! `(node, key)` in the order the properties were first written, and a read sees the staged value first. The map is thread-local:
//! `nodes.rs` has no state of the generation in its hand, and a shell has one thread.
//!
//! A plain write made outside staging tells [`released`], so that a transition that owned the property lets it go: the engine takes the
//! queue at the start of its next call and of every turn.
//!
//! A transition begun through the object protocol also holds an [`Undo`]: the shell's `rollback` callback, and what the library holds
//! that a failed callback may have changed (the styles). The TypeScript engine took the same two things around its callback
//! (`host/rollback.ts`): a snapshot of the workspaces and the styles, put back when the callback or the commit threw. Here the library
//! puts back its own, and the shell's callback puts back what only the shell knows.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::{Gnoblin, NodeId, Value};
use gnoblin_object::Callable;

use crate::error::{Fail, Res};
use crate::style::cascade::StyleSnapshot;

/// A value as it goes to the tree.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Prop {
    Number(f64),
    Bool(bool),
    Triple([f64; 3]),
    Text(String),
    Node(u64),
}

impl Prop {
    pub(crate) fn to_value(&self) -> Value {
        match self {
            Prop::Number(number) => Value::Double(*number),
            Prop::Bool(flag) => Value::Bool(*flag),
            Prop::Triple(list) => Value::List(list.to_vec()),
            Prop::Text(text) => Value::Text(text.clone()),
            Prop::Node(node) => Value::Node(NodeId(*node)),
        }
    }

    pub(crate) fn from_value(value: &Value) -> Option<Prop> {
        Some(match value {
            Value::Bool(flag) => Prop::Bool(*flag),
            Value::Int(_) | Value::Uint(_) | Value::Double(_) => Prop::Number(value.as_f64()?),
            Value::Text(text) => Prop::Text(text.clone()),
            Value::Node(node) => Prop::Node(node.0),
            Value::List(list) if list.len() == 3 => Prop::Triple([list[0], list[1], list[2]]),
            _ => return None,
        })
    }
}

/// The timed write a staged value is sent as when it has a curve of its own.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Curve {
    pub(crate) duration_ms: f64,
    pub(crate) samples: Vec<f64>,
}

/// A write held until the callback completes.
#[derive(Clone, Debug)]
pub(crate) struct Staged {
    pub(crate) node: u64,
    pub(crate) key: String,
    pub(crate) value: Prop,
    pub(crate) curve: Option<Curve>,
}

thread_local! {
    static STAGE: RefCell<Option<Vec<Staged>>> = const { RefCell::new(None) };
    static RELEASED: RefCell<Vec<(u64, String)>> = const { RefCell::new(Vec::new()) };
}

/// Whether property reads and writes are being staged.
pub(crate) fn active() -> bool {
    STAGE.with(|stage| stage.borrow().is_some())
}

/// Starts collecting writes.
pub(crate) fn begin() {
    STAGE.with(|stage| *stage.borrow_mut() = Some(Vec::new()));
}

/// Stops staging and hands over what it kept, in the order the properties were first written.
pub(crate) fn end() -> Vec<Staged> {
    STAGE.with(|stage| stage.borrow_mut().take().unwrap_or_default())
}

/// The staged value of `key` of `node`, if there is one.
pub(crate) fn peek(node: u64, key: &str) -> Option<Prop> {
    STAGE.with(|stage| {
        stage
            .borrow()
            .as_ref()?
            .iter()
            .find(|staged| staged.node == node && staged.key == key)
            .map(|staged| staged.value.clone())
    })
}

/// Keeps a write: a second write of the same property replaces the first where it stands.
pub(crate) fn keep(entry: Staged) {
    STAGE.with(|stage| {
        if let Some(list) = stage.borrow_mut().as_mut() {
            match list
                .iter_mut()
                .find(|staged| staged.node == entry.node && staged.key == entry.key)
            {
                Some(held) => *held = entry,
                None => list.push(entry),
            }
        }
    });
}

/// A write that goes to the tree: held when a transition is being staged (true), else the caller makes it (false).
pub(crate) fn capture(node: u64, key: &str, value: &Prop) -> bool {
    if !active() {
        return false;
    }
    keep(Staged {
        node,
        key: key.to_owned(),
        value: value.clone(),
        curve: None,
    });
    true
}

/// A plain write of `key` of `node` was made outside staging: a transition that owns it lets it go.
pub(crate) fn released(node: u64, key: &str) {
    RELEASED.with(|queue| queue.borrow_mut().push((node, key.to_owned())));
}

/// The plain writes made since the last call.
pub(crate) fn take_released() -> Vec<(u64, String)> {
    RELEASED.with(|queue| std::mem::take(&mut *queue.borrow_mut()))
}

/// The properties of a window that a timed write can move: `host/client_animation.ts`.
pub(crate) const ANIMATED_KEYS: [&str; 8] = ["x", "y", "width", "height", "opacity", "z", "rotation", "scale"];

/// Whether the tree takes a timed write of `key`.
pub(crate) fn animated(key: &str) -> bool {
    ANIMATED_KEYS.contains(&key)
}

/// What `set` would answer for this write, without queuing it: the node is there, the property exists and can be written, the value is
/// of its kind and, for a timed write, the tree can animate it.
pub(crate) fn check(g: &Gnoblin, node: u64, key: &str, value: &Prop, timed: bool) -> Res<()> {
    let handle = g.node(NodeId(node));
    let class = handle.class().map_err(|_| {
        Fail::new(
            gnoblin_core_abi::E_NO_SUCH_NODE,
            format!("node {node} is not in the tree"),
        )
    })?;
    let member = g
        .member(class, gnoblin_core::MemberKind::Property, key)
        .map_err(|_| Fail::no_such(format!("node {node} has no property `{key}`")))?;
    if member.readonly {
        return Err(Fail::refused(format!("`{key}` of node {node} is read-only")));
    }
    if timed && !animated(key) {
        return Err(Fail::refused(format!("`{key}` of node {node} cannot be animated")));
    }
    use gnoblin_core_abi as abi;
    let fits = match (member.value_kind, value) {
        (abi::VALUE_DOUBLE | abi::VALUE_INT | abi::VALUE_UINT, Prop::Number(number)) => number.is_finite(),
        (abi::VALUE_BOOL, Prop::Bool(_)) | (abi::VALUE_TEXT, Prop::Text(_)) | (abi::VALUE_NODE, Prop::Node(_)) => true,
        (abi::VALUE_LIST, Prop::Triple(list)) => list.iter().all(|part| part.is_finite()),
        _ => false,
    };
    if fits {
        Ok(())
    } else {
        Err(Fail::type_error(format!(
            "`{key}` of node {node} does not take {value:?}"
        )))
    }
}

/// What a transition begun through the object protocol can put back when it does not commit (the callback failed, the commit was
/// refused, or the shell aborted it).
pub(crate) struct Undo {
    /// The shell's own rollback, called after the library has put back its own state and before the failing call answers.
    pub(crate) rollback: Option<Rc<dyn Callable>>,
    /// The styles as they were when staging began.
    pub(crate) styles: StyleSnapshot,
}

impl std::fmt::Debug for Undo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Undo")
            .field("rollback", &self.rollback.is_some())
            .finish_non_exhaustive()
    }
}
