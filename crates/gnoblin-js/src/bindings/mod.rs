//! Host objects: the native functions behind the `gnoblin` module.
//!
//! A script never sees the tree. It sees `Client` and `Collection` objects written in TypeScript
//! (`js/gnoblin.ts`), and those call the functions installed here as `globalThis.__gnoblinHost`. The
//! functions answer from the tree mirror, with this frame's queued writes on top, and queue writes into
//! the transaction builder. They return plain numbers, strings and booleans, and report a refusal as a
//! code string that the module turns into a typed error naming the member, so no failure here depends on
//! the engine's error types.

mod host;
mod props;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_tree_client::tree_mirror::{MirrorValue, NodeId, NodeRecord, TreeMirror, TxBuilder};
use rquickjs::{Ctx, IntoJs, Value};
use thiserror::Error;

pub(crate) use host::{install, parents_first};

/// What a script asked the host to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsSpawnRequest {
    /// One command line, for the host to run as it runs a launcher entry or a shell line.
    Command(String),
    /// A program and its arguments, run without a shell.
    Argv(Vec<String>),
}

/// A host's refusal to start a program, with the message the script's author sees.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct JsSpawnError(pub String);

/// What `gnoblin.spawn` calls. The shell does not start processes itself: the host owns that, so a test
/// can record the request and the runtime role can route it through the session's launcher.
pub type JsSpawner = Box<dyn FnMut(&JsSpawnRequest) -> Result<(), JsSpawnError>>;

/// Everything the host functions share with the shell's frame loop.
///
/// It holds no script value, so dropping it never touches the engine: the closures installed in the
/// context keep a handle, and the shell keeps another.
pub(crate) struct HostState {
    /// The mirror as the last applied batch left it. Changed only between frames' dispatches.
    pub mirror: TreeMirror,
    /// This frame's writes, and the sent-but-unanswered writes of earlier frames, readable as written.
    pub tick: TxBuilder,
    /// The records of nodes the compositor destroyed in the batches being planned, so the objects that
    /// wrapped them can read their last values once before they freeze.
    pub graveyard: HashMap<NodeId, NodeRecord>,
    pub spawner: Option<JsSpawner>,
    /// The last call token handed out. The compositor echoes it in an answer, which nothing reads yet.
    pub tokens: u64,
}

pub(crate) type SharedState = Rc<RefCell<HostState>>;

impl HostState {
    pub(crate) fn new(mirror: TreeMirror) -> Self {
        Self {
            mirror,
            tick: TxBuilder::new(),
            graveyard: HashMap::new(),
            spawner: None,
            tokens: 0,
        }
    }
}

/// A property value as a script sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsHostValue {
    Flag(bool),
    Number(f64),
    Text(String),
}

impl JsHostValue {
    /// The script's view of a mirror value, or `None` for a kind no exposed property holds.
    pub(crate) fn from_mirror(value: &MirrorValue) -> Option<Self> {
        match value {
            MirrorValue::Bool(flag) => Some(Self::Flag(*flag)),
            MirrorValue::Int(n) => Some(Self::Number(*n as f64)),
            MirrorValue::Uint(n) | MirrorValue::Node(n) => Some(Self::Number(*n as f64)),
            MirrorValue::Double(n) => Some(Self::Number(*n)),
            MirrorValue::Text(text) => Some(Self::Text(text.clone())),
            _ => None,
        }
    }
}

impl<'js> IntoJs<'js> for JsHostValue {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            Self::Flag(flag) => Ok(Value::new_bool(ctx.clone(), flag)),
            Self::Number(n) => Ok(Value::new_number(ctx.clone(), n)),
            Self::Text(text) => text.into_js(ctx),
        }
    }
}
