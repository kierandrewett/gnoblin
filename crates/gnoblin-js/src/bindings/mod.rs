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
use std::sync::Arc;

use gnoblin_proto::tree::effect;
use gnoblin_tree_client::tree_mirror::{MirrorValue, NodeId, NodeRecord, TreeMirror, TxBuilder};
use rquickjs::{Array, Ctx, IntoJs, Object, Value};
use thiserror::Error;

use crate::supervisor::JsClock;

pub(crate) use host::{adopt_windows, install, parents_first, spawn};

/// What the shell asked the host to start with the session: one of the config's `autostart` commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsSpawnRequest {
    /// A program and its arguments, run without a shell.
    Argv(Vec<String>),
}

/// A host's refusal to start a program, with the message the script's author sees.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct JsSpawnError(pub String);

/// What starts the config's `autostart` commands. `gnoblin.spawn` is not routed through it: a script's programs are
/// `gnoblin.process`, which the shell owns and reaps. The host owns autostart, so a test can record the requests and
/// the runtime role can route them through the session's launcher.
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
    /// The seat claims this shell holds and the chords and gestures waiting for the frame's dispatch.
    pub input: crate::input::JsInput,
    /// How node ids cross into script numbers (see [`NodeHandles`]).
    pub handles: NodeHandles,
    /// What the UI toolkit keeps between frames: the canvases it made, its measurements and the events for it.
    pub ui: crate::ui::JsUiState,
    /// The scene nodes the XR module made.
    pub xr: crate::xr::JsXrState,
    /// The configuration document: the link to the compositor's, and what the shell last committed to it.
    pub config: crate::config::ConfigState,
    /// The session connection and the processes the script started (`gnoblin.session`, `gnoblin.process`).
    pub api: crate::host_api::JsHostApi,
    /// What the stock lock screen keeps between frames: the password checks it asked and their answers.
    pub lock: crate::lock::JsLockState,
}

pub(crate) type SharedState = Rc<RefCell<HostState>>;

impl HostState {
    /// State over `mirror` that reads the real clock: what the unit tests of the host functions need.
    #[cfg(test)]
    pub(crate) fn new(mirror: TreeMirror) -> Self {
        Self::with_clock(mirror, Arc::new(crate::supervisor::JsSystemClock))
    }

    /// State over `mirror` whose scripts read `clock`.
    pub(crate) fn with_clock(mirror: TreeMirror, clock: Arc<dyn JsClock>) -> Self {
        Self {
            mirror,
            tick: TxBuilder::new(),
            graveyard: HashMap::new(),
            spawner: None,
            tokens: 0,
            input: crate::input::JsInput::new(clock),
            handles: NodeHandles::default(),
            ui: crate::ui::JsUiState::default(),
            xr: crate::xr::JsXrState::default(),
            config: crate::config::ConfigState::new(),
            api: crate::host_api::JsHostApi::default(),
            lock: crate::lock::JsLockState::default(),
        }
    }
}

/// The node ids at or above this cross into script numbers as table handles. A script number holds an integer
/// exactly up to 2^53, and the compositor's own nodes carry ids with bit 63 set, so an id cannot be the number
/// the script sees. The top 2^24 integers below 2^53 are the handle band; every id below it is its own number,
/// so the ids a shell makes and the tests' small ids read as themselves.
const HANDLE_BASE: u64 = (1 << 53) - (1 << 24);

/// Turns node ids into the numbers a script holds, and back.
///
/// An id below [`HANDLE_BASE`] is its own number. Any other id gets the next handle in the band, the first time it
/// crosses, and keeps it for the life of the shell, so a reloaded generation sees the same number for the same
/// window. The table is interior-mutable because a read of a property that names a node interns it.
#[derive(Default)]
pub(crate) struct NodeHandles {
    table: RefCell<HandleTable>,
}

#[derive(Default)]
struct HandleTable {
    ids: Vec<NodeId>,
    index: HashMap<NodeId, u64>,
}

impl NodeHandles {
    /// The script number for `id`.
    pub(crate) fn to_script(&self, id: NodeId) -> f64 {
        if id < HANDLE_BASE {
            return id as f64;
        }
        let mut table = self.table.borrow_mut();
        if let Some(handle) = table.index.get(&id) {
            return (HANDLE_BASE + handle) as f64;
        }
        let handle = table.ids.len() as u64;
        table.ids.push(id);
        table.index.insert(id, handle);
        (HANDLE_BASE + handle) as f64
    }

    /// The node a script number names, or `None` when it is not a whole non-negative number or names a handle
    /// that was never given out.
    pub(crate) fn resolve(&self, raw: f64) -> Option<NodeId> {
        if !(raw.is_finite() && raw >= 0.0 && raw.fract() == 0.0) || raw >= (1_u64 << 53) as f64 {
            return None;
        }
        let number = raw as u64;
        match number.checked_sub(HANDLE_BASE) {
            None => Some(number),
            Some(handle) => self.table.borrow().ids.get(usize::try_from(handle).ok()?).copied(),
        }
    }

    /// The decimal text of the node `raw` names: what the compositor's own tools print, so a script's `client.id`
    /// matches `gnoblinctl`.
    pub(crate) fn label(&self, raw: f64) -> Option<String> {
        self.resolve(raw).map(|id| id.to_string())
    }
}

/// A property value as a script sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsHostValue {
    Flag(bool),
    Number(f64),
    Text(String),
    /// A list of numbers: a window's `rotation` and `scale`.
    Numbers(Vec<f64>),
    /// A list of texts: a node's `skipped` effects.
    Texts(Vec<String>),
    /// A node's `effects`, one object per effect with the schema's property names.
    Effects(Vec<effect::Properties>),
}

impl JsHostValue {
    /// The script's view of a mirror value, or `None` for a kind no exposed property holds. A node reference
    /// is the number `handles` gives the node.
    pub(crate) fn from_mirror(value: &MirrorValue, handles: &NodeHandles) -> Option<Self> {
        match value {
            MirrorValue::Bool(flag) => Some(Self::Flag(*flag)),
            MirrorValue::Int(n) => Some(Self::Number(*n as f64)),
            MirrorValue::Uint(n) => Some(Self::Number(*n as f64)),
            MirrorValue::Node(id) => Some(Self::Number(handles.to_script(*id))),
            MirrorValue::Double(n) => Some(Self::Number(*n)),
            MirrorValue::Text(text) => Some(Self::Text(text.clone())),
            MirrorValue::List(numbers) => Some(Self::Numbers(numbers.clone())),
            MirrorValue::Texts(texts) => Some(Self::Texts(texts.clone())),
            MirrorValue::Effects(effects) => Some(Self::Effects(effects.clone())),
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
            Self::Numbers(numbers) => numbers.into_js(ctx),
            Self::Texts(texts) => texts.into_js(ctx),
            Self::Effects(effects) => {
                let list = Array::new(ctx.clone())?;
                for (at, entry) in effects.iter().enumerate() {
                    list.set(at, effect_to_js(ctx, entry)?)?;
                }
                Ok(list.into_value())
            }
        }
    }
}

/// The keys of an `effect` object a script writes, named as the schema names them.
const EFFECT_KEYS: [&str; 14] = [
    "kind",
    "downscale",
    "passes",
    "algorithm",
    "noise",
    "brightness",
    "contrast",
    "saturation",
    "tint",
    "offset_x",
    "offset_y",
    "blur_radius",
    "spread",
    "colour",
];

fn effect_to_js<'js>(ctx: &Ctx<'js>, entry: &effect::Properties) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("kind", entry.kind.as_str())?;
    object.set("downscale", entry.downscale as f64)?;
    object.set("passes", entry.passes as f64)?;
    object.set("algorithm", entry.algorithm.as_str())?;
    object.set("noise", entry.noise)?;
    object.set("brightness", entry.brightness)?;
    object.set("contrast", entry.contrast)?;
    object.set("saturation", entry.saturation)?;
    object.set("tint", entry.tint.as_str())?;
    object.set("offset_x", entry.offset_x)?;
    object.set("offset_y", entry.offset_y)?;
    object.set("blur_radius", entry.blur_radius)?;
    object.set("spread", entry.spread)?;
    object.set("colour", entry.colour.as_str())?;
    Ok(object)
}

/// The effect a script's object describes, or `None` when it has a key the schema does not, a field of the wrong
/// type, a number that is not finite, or a negative or fractional `downscale` or `passes`. A field the object
/// leaves out keeps the wire's zero value, so a script that wants the documented defaults writes them.
pub(crate) fn effect_from_js(object: &Object<'_>) -> Option<effect::Properties> {
    for key in object.keys::<String>() {
        if !EFFECT_KEYS.contains(&key.ok()?.as_str()) {
            return None;
        }
    }
    let text = |name: &str| -> Option<String> {
        object
            .get::<_, Option<String>>(name)
            .ok()
            .map(Option::unwrap_or_default)
    };
    let number = |name: &str| -> Option<f64> {
        let n = object.get::<_, Option<f64>>(name).ok()?.unwrap_or(0.0);
        n.is_finite().then_some(n)
    };
    let whole = |name: &str| -> Option<u64> {
        let n = number(name)?;
        (n >= 0.0 && n.fract() == 0.0).then_some(n as u64)
    };
    Some(effect::Properties {
        kind: text("kind")?,
        downscale: whole("downscale")?,
        passes: whole("passes")?,
        algorithm: text("algorithm")?,
        noise: number("noise")?,
        brightness: number("brightness")?,
        contrast: number("contrast")?,
        saturation: number("saturation")?,
        tint: text("tint")?,
        offset_x: number("offset_x")?,
        offset_y: number("offset_y")?,
        blur_radius: number("blur_radius")?,
        spread: number("spread")?,
        colour: text("colour")?,
    })
}
