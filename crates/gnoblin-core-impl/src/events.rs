//! What the shell host's mailbox carries: one event type for everything a shell can wait for.
//!
//! The compositor side publishes [`SignalDelivery`]s (any thread, through `host::publish`). The shell thread's own pump adds the
//! rest, from what the tree link and the clock bring: a change of the tree, an event of an input claim, the answer to a call, a
//! timer that fell due, the end of a program. Each registration's `matches` function picks the events that are for it, so a new
//! kind of registration needs no change to the host.

use crate::values::OwnedValue;

/// A signal to deliver to the shell, by class and signal id.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalDelivery {
    /// The catalog class id.
    pub class: u32,
    /// The signal id within the class.
    pub signal: u32,
    /// The node it came from, or zero.
    pub node: u64,
    /// The payload fields.
    pub values: Vec<OwnedValue>,
}

/// A change of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeNotice {
    /// `NODE_CREATED`, `NODE_DESTROYED` or `NODE_CHANGED`.
    pub kind: u32,
    /// The catalog class id of the node, zero when the class is not in the catalog.
    pub class: u32,
    /// For a change: the property's id in the class, zero when the catalog does not know it.
    pub property: u32,
    /// The node.
    pub node: u64,
    /// The node's parent when the batch that made it parented it.
    pub parent: u64,
    /// For a change: the new value.
    pub value: Option<OwnedValue>,
}

/// An event of an input claim.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimNotice {
    /// `CLAIM_EVENT_CHORD`, `CLAIM_EVENT_GESTURE`, `CLAIM_EVENT_POINTER_CHORD` or `CLAIM_EVENT_POINTER_BUTTON`.
    pub kind: u32,
    /// The claim's id, as the core allocated it.
    pub claim: i64,
    /// A chord's state, a phase, or a button's state.
    pub state: String,
    /// A chord's text, a gesture's kind, a pointer chord's modifiers.
    pub text: String,
    /// A gesture's finger count.
    pub fingers: i64,
    /// A button number or an evdev code.
    pub button: i64,
    /// The pointer's global place.
    pub x: f64,
    /// The pointer's global place.
    pub y: f64,
    /// Movement.
    pub dx: f64,
    /// Movement.
    pub dy: f64,
    /// A pinch's scale.
    pub scale: f64,
    /// A pinch's rotation.
    pub rotation: f64,
    /// The input event's time in milliseconds.
    pub time: i64,
}

/// The answer to a call (or to a flush: its token is the flush's).
#[derive(Clone, Debug, PartialEq)]
pub struct AnswerNotice {
    /// The token the call went out under.
    pub token: u64,
    /// The answer's bytes.
    pub result: Vec<u8>,
    /// The failure, empty on success.
    pub error: String,
}

/// How a program the shell started ended.
#[derive(Clone, Debug, PartialEq)]
pub struct ExitNotice {
    /// The process, as `spawn` numbered it.
    pub process: u64,
    /// The exit status after a normal exit.
    pub code: Option<i32>,
    /// The signal that ended it.
    pub signal: Option<String>,
    /// Whether the timeout ended it.
    pub timed_out: bool,
    /// Captured standard output.
    pub stdout: Option<String>,
    /// Captured standard error.
    pub stderr: Option<String>,
    /// Bit 0: standard output was cut at the limit. Bit 1: standard error was.
    pub truncated: u32,
}

/// Everything a registration can be told.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellEvent {
    /// A signal of the compositor or of the tree.
    Signal(SignalDelivery),
    /// A change of the tree.
    Node(NodeNotice),
    /// An event of an input claim.
    Claim(ClaimNotice),
    /// The answer to a call or a flush.
    Answer(AnswerNotice),
    /// A timer fell due, by the id the core gave it.
    Timer(u64),
    /// A program ended.
    Exit(ExitNotice),
}
