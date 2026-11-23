//! The model of one generation: its windows, its outputs, its collections and what the compositor told it.
//!
//! The TypeScript library kept this in `host/{adoption,frame,collection_events,collection_state}.ts`. The rules are the same: a batch
//! of changes is read in one go (departures, then arrivals, then changes), a listener of a member is queued before the listeners of
//! the collections that hold it, and a window that is gone keeps the values it last had.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use gnoblin_core_abi as abi;
use gnoblin_object::server::Cx;
use gnoblin_object::{ErrorCode, Fail, Obj, Res, Val};
use gnoblin_tree_client::session::WriteError;
use gnoblin_tree_client::tree_mirror::TxError;

use super::values::{self, class_name, live, nodes_of, published, record, rect, tree_class};
use crate::events::ShellEvent;
use crate::values::OwnedValue;

/// The properties a window keeps readable after it is gone.
pub(super) const CLIENT_KEYS: &[&str] = &[
    "title",
    "app_id",
    "focused",
    "fullscreen",
    "maximised",
    "minimised",
    "x",
    "y",
    "width",
    "height",
    "tiled",
    "visible",
    "opacity",
    "z",
    "rotation",
    "scale",
    "secure",
    "parent_window",
    "from_launcher",
    "decoration",
    "decoration_wanted",
    "frame_policy",
    "frame_rate",
    "report_presented",
    "presented",
    "unresponsive",
];

/// The properties an output keeps readable after it is gone.
pub(super) const OUTPUT_KEYS: &[&str] = &[
    "name",
    "connector",
    "identity",
    "mirror_of",
    "make",
    "model",
    "x",
    "y",
    "width",
    "height",
    "output_scale",
    "supersample",
    "transform",
    "mode",
    "primary",
    "enabled",
    "gpu_memory_target_bytes",
    "gpu_memory_scratch_bytes",
    "gpu_memory_staging_bytes",
    "gpu_memory_views_bytes",
    "gpu_memory_capture_bytes",
    "gpu_memory_bytes",
    "gpu_memory_clients_bytes",
    "gpu_memory_budget_bytes",
    "gpu_memory_refused_total",
    "gpu_memory_evicted_total",
];

/// What `where` compares: a number, text or a flag.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum QueryScalar {
    Text(String),
    Num(f64),
    Flag(bool),
}

impl QueryScalar {
    pub(super) fn from_val(value: &Val) -> Option<QueryScalar> {
        match value {
            Val::Text(text) => Some(QueryScalar::Text(text.clone())),
            Val::Bool(flag) => Some(QueryScalar::Flag(*flag)),
            Val::Double(number) => Some(QueryScalar::Num(*number)),
            Val::Int(number) => Some(QueryScalar::Num(*number as f64)),
            Val::Uint(number) => Some(QueryScalar::Num(*number as f64)),
            _ => None,
        }
    }
}

/// What a field of a query wants.
#[derive(Clone, Debug)]
pub(super) enum QueryWant {
    One(QueryScalar),
    Any(Vec<QueryScalar>),
}

/// A selection's query: every term must hold.
#[derive(Clone, Debug, Default)]
pub(super) struct ModelQuery {
    pub(super) terms: Vec<(String, QueryWant)>,
}

/// What a collection holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CollKind {
    Client,
    Output,
}

impl CollKind {
    pub(super) fn word(self) -> &'static str {
        match self {
            CollKind::Client => "Client",
            CollKind::Output => "Output",
        }
    }
}

/// What listeners were last told about a window, to raise an event only for a real change.
#[derive(Clone, Debug, Default)]
pub(super) struct ClientSeen {
    pub(super) title: String,
    pub(super) app_id: String,
    pub(super) decoration: String,
    pub(super) focused: bool,
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) parent: u64,
}

/// What listeners were last told about an output.
#[derive(Clone, Debug, Default)]
pub(super) struct OutputSeen {
    pub(super) detached: bool,
    pub(super) mode: String,
    pub(super) scale: f64,
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

pub(super) use super::listener::Registered;

/// A window.
pub(super) struct ClientState {
    pub(super) node: u64,
    pub(super) class: u32,
    pub(super) alive: bool,
    /// What the compositor last published, for the properties a window keeps.
    pub(super) last: HashMap<String, OwnedValue>,
    pub(super) frozen: Option<HashMap<String, OwnedValue>>,
    pub(super) seen: ClientSeen,
    pub(super) readied: bool,
    pub(super) listeners: Registered,
    pub(super) bindings: Cell<Option<Obj>>,
}

impl ClientState {
    fn new(node: u64) -> ClientState {
        let class = tree_class("window");
        let last = CLIENT_KEYS
            .iter()
            .map(|key| ((*key).to_owned(), live(node, class, key)))
            .collect();
        ClientState {
            node,
            class,
            alive: true,
            last,
            frozen: None,
            seen: client_seen(node, class),
            readied: false,
            listeners: Vec::new(),
            bindings: Cell::new(None),
        }
    }

    /// A property as the window has it: through the tick while it lives, as it was when it went after.
    pub(super) fn read(&self, key: &str) -> OwnedValue {
        match &self.frozen {
            Some(frozen) => frozen.get(key).cloned().unwrap_or_else(|| live(0, self.class, key)),
            None => live(self.node, self.class, key),
        }
    }

    /// The value `where` compares for `key`.
    pub(super) fn scalar(&self, key: &str) -> Option<QueryScalar> {
        Some(match key {
            "id" => QueryScalar::Text(self.node.to_string()),
            "app_id" | "title" => QueryScalar::Text(self.read(key).string()),
            "focused" | "fullscreen" => QueryScalar::Flag(self.read(key).flag()),
            "x" | "y" | "width" | "height" => QueryScalar::Num(self.read(key).number()),
            _ => return None,
        })
    }
}

fn client_seen(node: u64, class: u32) -> ClientSeen {
    ClientSeen {
        title: published(node, class, "title").string(),
        app_id: published(node, class, "app_id").string(),
        decoration: published(node, class, "decoration").string(),
        focused: published(node, class, "focused").flag(),
        x: published(node, class, "x").number(),
        y: published(node, class, "y").number(),
        width: published(node, class, "width").number(),
        height: published(node, class, "height").number(),
        parent: published(node, class, "parent_window").node(),
    }
}

/// An output.
pub(super) struct OutputNode {
    pub(super) node: u64,
    pub(super) class: u32,
    pub(super) alive: bool,
    pub(super) last: HashMap<String, OwnedValue>,
    pub(super) frozen: Option<HashMap<String, OwnedValue>>,
    pub(super) seen: OutputSeen,
    pub(super) listeners: Registered,
    pub(super) bindings: Cell<Option<Obj>>,
}

impl OutputNode {
    fn new(node: u64) -> OutputNode {
        let class = tree_class("screen");
        let last = OUTPUT_KEYS
            .iter()
            .map(|key| ((*key).to_owned(), live(node, class, key)))
            .collect();
        OutputNode {
            node,
            class,
            alive: true,
            last,
            frozen: None,
            seen: output_seen(node, class),
            listeners: Vec::new(),
            bindings: Cell::new(None),
        }
    }

    pub(super) fn read(&self, key: &str) -> OwnedValue {
        match &self.frozen {
            Some(frozen) => frozen.get(key).cloned().unwrap_or_else(|| live(0, self.class, key)),
            None => live(self.node, self.class, key),
        }
    }

    pub(super) fn scalar(&self, key: &str) -> Option<QueryScalar> {
        Some(match key {
            "id" => QueryScalar::Text(self.node.to_string()),
            "name" | "make" | "model" | "mode" => QueryScalar::Text(self.read(key).string()),
            "scale" => QueryScalar::Num(self.read("output_scale").number()),
            "primary" | "enabled" => QueryScalar::Flag(self.read(key).flag()),
            "x" | "y" | "width" | "height" => QueryScalar::Num(self.read(key).number()),
            _ => return None,
        })
    }
}

fn output_seen(node: u64, class: u32) -> OutputSeen {
    OutputSeen {
        detached: !published(node, class, "mirror_of").string().is_empty() || !published(node, class, "enabled").flag(),
        mode: published(node, class, "mode").string(),
        scale: published(node, class, "output_scale").number(),
        x: published(node, class, "x").number(),
        y: published(node, class, "y").number(),
        width: published(node, class, "width").number(),
        height: published(node, class, "height").number(),
    }
}

/// A collection: a root the compositor fills, a list a shell keeps, or a selection of another collection.
pub(super) struct CollState {
    pub(super) kind: CollKind,
    /// The collection a selection narrows.
    pub(super) parent: Option<Obj>,
    pub(super) query: Option<ModelQuery>,
    /// The members of a root or a list, in order.
    pub(super) list: Vec<Obj>,
    /// The members a selection last told its listeners of.
    pub(super) members: HashSet<Obj>,
    pub(super) priority: f64,
    pub(super) mutable: bool,
    pub(super) listeners: Registered,
}

impl CollState {
    pub(super) fn root(kind: CollKind) -> CollState {
        CollState::bare(kind, None, None, false)
    }

    pub(super) fn list(kind: CollKind) -> CollState {
        CollState::bare(kind, None, None, true)
    }

    pub(super) fn view(kind: CollKind, parent: Obj, query: ModelQuery, priority: f64) -> CollState {
        let mut state = CollState::bare(kind, Some(parent), Some(query), false);
        state.priority = priority;
        state
    }

    fn bare(kind: CollKind, parent: Option<Obj>, query: Option<ModelQuery>, mutable: bool) -> CollState {
        CollState {
            kind,
            parent,
            query,
            list: Vec::new(),
            members: HashSet::new(),
            priority: 0.0,
            mutable,
            listeners: Vec::new(),
        }
    }

    /// What a computation of the members needs, taken so the collection's own state is free while it runs.
    pub(super) fn spec(&self) -> CollSpec {
        CollSpec {
            kind: self.kind,
            parent: self.parent,
            query: self.query.clone(),
            list: self.list.clone(),
        }
    }
}

/// The part of a collection that decides its members.
pub(super) struct CollSpec {
    pub(super) kind: CollKind,
    pub(super) parent: Option<Obj>,
    pub(super) query: Option<ModelQuery>,
    pub(super) list: Vec<Obj>,
}

/// Whether `member` satisfies `query`.
pub(super) fn matches(cx: &Cx<'_>, kind: CollKind, query: &ModelQuery, member: Obj) -> bool {
    let scalar = |key: &str| -> Option<QueryScalar> {
        match kind {
            CollKind::Client => cx
                .with_state::<ClientState, _>(member, |state| state.scalar(key))
                .ok()
                .flatten(),
            CollKind::Output => cx
                .with_state::<OutputNode, _>(member, |state| state.scalar(key))
                .ok()
                .flatten(),
        }
    };
    query.terms.iter().all(|(key, want)| {
        let have = scalar(key);
        match want {
            QueryWant::One(wanted) => have.as_ref() == Some(wanted),
            QueryWant::Any(wanted) => have.as_ref().is_some_and(|have| wanted.contains(have)),
        }
    })
}

/// The members of a collection now, in order.
pub(super) fn items_in(cx: &Cx<'_>, spec: &CollSpec) -> Vec<Obj> {
    match spec.parent {
        None => spec.list.clone(),
        Some(parent) => {
            let query = spec.query.clone().unwrap_or_default();
            items(cx, parent)
                .into_iter()
                .filter(|member| matches(cx, spec.kind, &query, *member))
                .collect()
        }
    }
}

/// The members of `coll` now, in order. A collection that is gone has none.
pub(super) fn items(cx: &Cx<'_>, coll: Obj) -> Vec<Obj> {
    match cx.with_state::<CollState, _>(coll, |state| state.spec()) {
        Ok(spec) => items_in(cx, &spec),
        Err(_) => Vec::new(),
    }
}

/// Whether `member` belongs to `coll` now, worked out from the objects and not from what was told.
pub(super) fn contains(cx: &Cx<'_>, coll: Obj, member: Obj) -> bool {
    let Ok((kind, parent, query, listed)) = cx.with_state::<CollState, _>(coll, |state| {
        (
            state.kind,
            state.parent,
            state.query.clone(),
            state.list.contains(&member),
        )
    }) else {
        return false;
    };
    match parent {
        None => listed,
        Some(parent) => contains(cx, parent, member) && matches(cx, kind, &query.unwrap_or_default(), member),
    }
}

/// The collection that owns the members of `coll`: the root or the list a selection narrows, however deep.
pub(super) fn root_of(cx: &Cx<'_>, coll: Obj) -> Obj {
    let mut current = coll;
    for _ in 0..64 {
        match cx.with_state::<CollState, _>(current, |state| state.parent) {
            Ok(Some(parent)) => current = parent,
            _ => break,
        }
    }
    current
}

/// The model of one generation.
pub(super) struct World {
    pub(super) client_root: Obj,
    pub(super) output_root: Obj,
    pub(super) clients: HashMap<u64, Obj>,
    pub(super) outputs: HashMap<u64, Obj>,
    /// The selections, in the order they were made.
    pub(super) tracked: Vec<Obj>,
    /// The lists of windows a shell keeps, in the order they were made.
    pub(super) lists: Vec<Obj>,
    pub(super) booted: bool,
    pub(super) opened: bool,
    /// The windows that were there when the shell started: told `ready` once it has.
    pub(super) preexisting: Vec<Obj>,
    /// The windows in the order they were last focused.
    pub(super) focus: Vec<Obj>,
    /// An `apply` waiting for its answer, by the token it went out under: the output and the operation.
    pub(super) applications: HashMap<u64, (Obj, Obj)>,
    /// Notices the library says once.
    pub(super) noticed: HashSet<String>,
}

impl World {
    pub(super) fn new(cx: &Cx<'_>) -> Res<World> {
        let client_root = cx.spawn("client_collection", CollState::root(CollKind::Client))?;
        cx.hold(client_root)?;
        let output_root = cx.spawn("output_collection", CollState::root(CollKind::Output))?;
        cx.hold(output_root)?;
        Ok(World {
            client_root,
            output_root,
            clients: HashMap::new(),
            outputs: HashMap::new(),
            tracked: Vec::new(),
            lists: Vec::new(),
            booted: false,
            opened: false,
            preexisting: Vec::new(),
            focus: Vec::new(),
            applications: HashMap::new(),
            noticed: HashSet::new(),
        })
    }

    /// Drops the selections and lists that are gone.
    pub(super) fn prune(&mut self, cx: &Cx<'_>) {
        let alive = |obj: &Obj| cx.with_state::<CollState, _>(*obj, |_| ()).is_ok();
        self.tracked.retain(alive);
        self.lists.retain(alive);
    }

    /// Wraps what the compositor already had: its outputs first, then its windows. Nothing is announced.
    pub(super) fn boot(&mut self, cx: &Cx<'_>) -> Res<()> {
        if self.booted || !values::attached(cx) {
            return Ok(());
        }
        for node in nodes_of("screen") {
            self.adopt_output(cx, node, false)?;
        }
        for node in nodes_of("window") {
            self.adopt_client(cx, node, false)?;
        }
        self.booted = true;
        Ok(())
    }

    /// The shell opened: the windows that were there are told `ready`.
    pub(super) fn open(&mut self, cx: &Cx<'_>) {
        if self.opened || !self.booted {
            return;
        }
        self.opened = true;
        for obj in std::mem::take(&mut self.preexisting) {
            let fresh = cx
                .with_state::<ClientState, _>(obj, |state| !std::mem::replace(&mut state.readied, true))
                .unwrap_or(false);
            if fresh {
                let held = self.client_holders(cx, obj);
                emit_member(cx, obj, &held, "ready", vec![cx.object_value(obj)]);
            }
        }
    }

    /// `[top]` and the selections of `top` that hold `member`, in the order the selections were made.
    pub(super) fn holders(&self, cx: &Cx<'_>, top: Obj, member: Obj) -> Vec<Obj> {
        let mut held = vec![top];
        for view in &self.tracked {
            if root_of(cx, *view) == top
                && cx
                    .with_state::<CollState, _>(*view, |s| s.members.contains(&member))
                    .unwrap_or(false)
            {
                held.push(*view);
            }
        }
        held
    }

    /// Every collection that holds `client` now: the root, the lists that have it, and their selections.
    pub(super) fn client_holders(&self, cx: &Cx<'_>, client: Obj) -> Vec<Obj> {
        let mut held = self.holders(cx, self.client_root, client);
        for list in &self.lists {
            if contains(cx, *list, client) {
                held.extend(self.holders(cx, *list, client));
            }
        }
        held
    }

    /// Tells the selections of `top` that `member` joined or left, when their query says so.
    pub(super) fn recompute(&self, cx: &Cx<'_>, top: Obj, member: Obj) {
        for view in &self.tracked {
            if root_of(cx, *view) != top {
                continue;
            }
            let was = cx
                .with_state::<CollState, _>(*view, |s| s.members.contains(&member))
                .unwrap_or(false);
            let is = contains(cx, *view, member);
            if was == is {
                continue;
            }
            let _ = cx.with_state::<CollState, _>(*view, |s| {
                if is {
                    s.members.insert(member);
                } else {
                    s.members.remove(&member);
                }
            });
            let name = if is { "added" } else { "removed" };
            cx.emit_on(*view, name, vec![cx.object_value(member), cx.object_value(*view)]);
        }
    }

    fn adopt_client(&mut self, cx: &Cx<'_>, node: u64, announce: bool) -> Res<()> {
        if self.clients.contains_key(&node) {
            return Ok(());
        }
        let obj = cx.spawn("client", ClientState::new(node))?;
        cx.hold(obj)?;
        self.clients.insert(node, obj);
        let root = self.client_root;
        cx.with_state::<CollState, _>(root, |state| state.list.push(obj))?;
        if announce {
            cx.emit_on(root, "added", vec![cx.object_value(obj), cx.object_value(root)]);
            self.recompute(cx, root, obj);
            let _ = cx.with_state::<ClientState, _>(obj, |state| state.readied = true);
            let held = self.holders(cx, root, obj);
            emit_member(cx, obj, &held, "ready", vec![cx.object_value(obj)]);
        } else {
            self.preexisting.push(obj);
        }
        Ok(())
    }

    fn adopt_output(&mut self, cx: &Cx<'_>, node: u64, announce: bool) -> Res<()> {
        if self.outputs.contains_key(&node) {
            return Ok(());
        }
        let obj = cx.spawn("output", OutputNode::new(node))?;
        cx.hold(obj)?;
        self.outputs.insert(node, obj);
        let root = self.output_root;
        cx.with_state::<CollState, _>(root, |state| state.list.push(obj))?;
        if announce {
            cx.emit_on(root, "added", vec![cx.object_value(obj), cx.object_value(root)]);
            self.recompute(cx, root, obj);
            let held = self.holders(cx, root, obj);
            emit_member(cx, obj, &held, "connected", vec![cx.object_value(obj)]);
        }
        Ok(())
    }

    /// Takes a destroyed window out of every collection and tells its listeners.
    fn release_client(&mut self, cx: &Cx<'_>, node: u64) {
        let Some(obj) = self.clients.remove(&node) else {
            return;
        };
        let root = self.client_root;
        let mut roots = Vec::new();
        for list in &self.lists {
            if contains(cx, *list, obj) {
                roots.extend(self.holders(cx, *list, obj));
            }
        }
        let held = self.holders(cx, root, obj);
        let _ = cx.with_state::<ClientState, _>(obj, |state| {
            state.frozen = Some(state.last.clone());
            state.alive = false;
        });
        let _ = cx.with_state::<CollState, _>(root, |state| state.list.retain(|member| *member != obj));
        for list in &self.lists {
            let _ = cx.with_state::<CollState, _>(*list, |state| state.list.retain(|member| *member != obj));
        }
        for view in &self.tracked {
            let _ = cx.with_state::<CollState, _>(*view, |state| state.members.remove(&obj));
        }
        self.preexisting.retain(|member| *member != obj);
        super::properties::dispose_of(cx, obj);
        super::input::client_gone(cx, obj);
        let all: Vec<Obj> = held.into_iter().chain(roots).collect();
        emit_member(cx, obj, &all, "closed", vec![cx.object_value(obj)]);
        for coll in &all {
            cx.emit_on(*coll, "removed", vec![cx.object_value(obj), cx.object_value(*coll)]);
        }
        // The library's own reference goes after the listeners were told: a window nobody holds is freed then.
        cx.later_on(obj, move |cx| cx.unhold(obj));
    }

    fn release_output(&mut self, cx: &Cx<'_>, node: u64) {
        let Some(obj) = self.outputs.remove(&node) else {
            return;
        };
        let root = self.output_root;
        let held = self.holders(cx, root, obj);
        let _ = cx.with_state::<OutputNode, _>(obj, |state| {
            state.frozen = Some(state.last.clone());
            state.alive = false;
        });
        let _ = cx.with_state::<CollState, _>(root, |state| state.list.retain(|member| *member != obj));
        for view in &self.tracked {
            let _ = cx.with_state::<CollState, _>(*view, |state| state.members.remove(&obj));
        }
        super::properties::dispose_of(cx, obj);
        emit_member(cx, obj, &held, "disconnected", vec![cx.object_value(obj)]);
        for coll in &held {
            cx.emit_on(*coll, "removed", vec![cx.object_value(obj), cx.object_value(*coll)]);
        }
        cx.later_on(obj, move |cx| cx.unhold(obj));
    }

    /// Raises the events one window's changed properties imply, to the listeners that held it before the change.
    fn client_changed(&self, cx: &Cx<'_>, obj: Obj) {
        let held = self.client_holders(cx, obj);
        let Ok(told) = cx.with_state::<ClientState, _>(obj, |state| -> Vec<(&'static str, Vec<Val>)> {
            let (node, class) = (state.node, state.class);
            let now = client_seen(node, class);
            let seen = &mut state.seen;
            let me = cx.object_value(obj);
            let mut told: Vec<(&'static str, Vec<Val>)> = Vec::new();
            if now.title != seen.title {
                seen.title = now.title.clone();
                told.push(("title_changed", vec![me.clone(), Val::Text(now.title.clone())]));
            }
            if now.app_id != seen.app_id {
                seen.app_id = now.app_id.clone();
                told.push(("app_id_changed", vec![me.clone(), Val::Text(now.app_id.clone())]));
            }
            if now.decoration != seen.decoration {
                seen.decoration = now.decoration.clone();
                told.push((
                    "decoration_changed",
                    vec![me.clone(), Val::Text(now.decoration.clone())],
                ));
            }
            if now.focused != seen.focused {
                seen.focused = now.focused;
                told.push(("focus_changed", vec![me.clone(), Val::Bool(now.focused)]));
            }
            if now.x != seen.x || now.y != seen.y || now.width != seen.width || now.height != seen.height {
                seen.x = now.x;
                seen.y = now.y;
                seen.width = now.width;
                seen.height = now.height;
                if let Ok(geometry) = rect(cx, now.x, now.y, now.width, now.height) {
                    told.push(("geometry_changed", vec![me.clone(), geometry]));
                }
            }
            seen.parent = now.parent;
            told
        }) else {
            return;
        };
        for (signal, args) in told {
            emit_member(cx, obj, &held, signal, args);
        }
    }

    fn output_changed(&self, cx: &Cx<'_>, obj: Obj) {
        let held = self.holders(cx, self.output_root, obj);
        let Ok(told) = cx.with_state::<OutputNode, _>(obj, |state| -> Vec<(&'static str, Vec<Val>)> {
            let now = output_seen(state.node, state.class);
            let seen = &mut state.seen;
            let me = cx.object_value(obj);
            let mut told: Vec<(&'static str, Vec<Val>)> = Vec::new();
            seen.detached = now.detached;
            if now.mode != seen.mode {
                seen.mode = now.mode.clone();
                told.push(("mode_changed", vec![me.clone(), Val::Text(now.mode.clone())]));
            }
            if now.scale != seen.scale {
                seen.scale = now.scale;
                told.push(("scale_changed", vec![me.clone(), Val::Double(now.scale)]));
            }
            if now.x != seen.x || now.y != seen.y || now.width != seen.width || now.height != seen.height {
                seen.x = now.x;
                seen.y = now.y;
                seen.width = now.width;
                seen.height = now.height;
                if let Ok(geometry) = rect(cx, now.x, now.y, now.width, now.height) {
                    told.push(("geometry_changed", vec![me.clone(), geometry]));
                }
            }
            told
        }) else {
            return;
        };
        for (signal, args) in told {
            emit_member(cx, obj, &held, signal, args);
        }
    }

    /// Settles the operations whose answer came: the operation, then the output's listeners.
    fn settle_applications(&mut self, cx: &Cx<'_>, answers: &[(u64, String)]) {
        for (token, error) in answers {
            let Some((output, operation)) = self.applications.remove(token) else {
                continue;
            };
            let outcome = if error.is_empty() { Ok(()) } else { Err(error.as_str()) };
            super::operation::settle(cx, operation, outcome);
            cx.later_on(operation, move |cx| cx.unhold(operation));
            let alive = cx
                .with_state::<OutputNode, _>(output, |state| state.alive)
                .unwrap_or(false);
            if alive {
                let held = self.holders(cx, self.output_root, output);
                emit_member(
                    cx,
                    output,
                    &held,
                    "applied",
                    vec![cx.object_value(output), Val::Text(error.clone())],
                );
            }
        }
    }

    /// The first seat's focus, as a window: the one the seat names, or with no seat the one whose own `focused` is set.
    pub(super) fn focused(&self, cx: &Cx<'_>, among: &[Obj]) -> Option<Obj> {
        let seat = nodes_of("seat").first().copied();
        match seat {
            None => among.iter().copied().find(|obj| {
                cx.with_state::<ClientState, _>(*obj, |state| state.read("focused").flag())
                    .unwrap_or(false)
            }),
            Some(seat) => {
                let focus = live(seat, tree_class("seat"), "focus").node();
                let held = self.clients.get(&focus).copied()?;
                among.contains(&held).then_some(held)
            }
        }
    }

    /// Notes who has the focus now.
    fn observe_focus(&mut self, cx: &Cx<'_>) {
        let everyone = items(cx, self.client_root);
        let focused = self.focused(cx, &everyone);
        let alive = |obj: &Obj| {
            cx.with_state::<ClientState, _>(*obj, |state| state.alive)
                .unwrap_or(false)
        };
        self.focus.retain(|obj| alive(obj) && Some(*obj) != focused);
        if let Some(focused) = focused
            && alive(&focused)
        {
            self.focus.insert(0, focused);
        }
    }
}

/// Queues a member's event for its own listeners, then for every collection that holds it.
pub(super) fn emit_member(cx: &Cx<'_>, member: Obj, held: &[Obj], signal: &str, args: Vec<Val>) {
    cx.emit_on(member, signal, args.clone());
    for coll in held {
        cx.emit_on(*coll, signal, args.clone());
    }
}

/// One batch of what the compositor sent: the order inside a list is the order it arrived in.
#[derive(Default)]
pub(super) struct PumpBatch {
    created: Vec<u64>,
    touched: Vec<u64>,
    destroyed: Vec<(u64, String)>,
    changes: Vec<(u64, String, OwnedValue)>,
    answers: Vec<(u64, String)>,
    pointer: Vec<(u64, String, f64, f64, u64)>,
}

impl PumpBatch {
    pub(super) fn of(events: &[ShellEvent]) -> PumpBatch {
        let mut batch = PumpBatch::default();
        let tree = crate::catalog::class_id("tree");
        let pointer = tree.and_then(|class| crate::catalog::signal_id(class, "event"));
        for event in events {
            match event {
                ShellEvent::Node(notice) => {
                    let Some(class) = crate::catalog::class(notice.class) else {
                        continue;
                    };
                    match notice.kind {
                        abi::NODE_CREATED => batch.created.push(notice.node),
                        abi::NODE_DESTROYED => batch.destroyed.push((notice.node, class.name.to_owned())),
                        abi::NODE_CHANGED => {
                            if !batch.touched.contains(&notice.node) {
                                batch.touched.push(notice.node);
                            }
                            if let Some(value) = &notice.value
                                && let Some(def) = crate::catalog::property_def(notice.class, notice.property)
                            {
                                batch.changes.push((notice.node, def.name.to_owned(), value.clone()));
                            }
                        }
                        _ => {}
                    }
                }
                ShellEvent::Answer(answer) => batch.answers.push((answer.token, answer.error.clone())),
                ShellEvent::Signal(signal) if Some(signal.class) == tree && Some(signal.signal) == pointer => {
                    // The schema's field order: frame, seq, node, kind, x, y, button.
                    if let [_, _, _, OwnedValue::Text(kind), x, y, button, ..] = signal.values.as_slice() {
                        batch.pointer.push((
                            signal.node,
                            kind.clone(),
                            x.number(),
                            y.number(),
                            button.number() as u64,
                        ));
                    }
                }
                _ => {}
            }
        }
        batch
    }

    fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.touched.is_empty()
            && self.destroyed.is_empty()
            && self.answers.is_empty()
            && self.pointer.is_empty()
    }
}

/// The signal a pointer edge raises on a window.
fn pointer_signal(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "enter" => "pointer_enter",
        "motion" => "pointer_motion",
        "leave" => "pointer_leave",
        "press" => "pointer_press",
        "release" => "pointer_release",
        _ => return None,
    })
}

impl World {
    /// Reads one batch: departures, then arrivals, then changes, and last the pointer's edges. The destroyed list is deepest first,
    /// and departures go before arrivals, so a collection never holds a dead member while it is told of a new one.
    pub(super) fn plan(&mut self, cx: &Cx<'_>, batch: PumpBatch) -> Res<()> {
        self.boot(cx)?;
        self.prune(cx);
        if !self.booted {
            return Ok(());
        }
        // What the compositor published is kept for the window that leaves, whatever its last change.
        for (node, key, value) in &batch.changes {
            if let Some(obj) = self.clients.get(node) {
                let _ = cx.with_state::<ClientState, _>(*obj, |state| state.last.insert(key.clone(), value.clone()));
            } else if let Some(obj) = self.outputs.get(node) {
                let _ = cx.with_state::<OutputNode, _>(*obj, |state| state.last.insert(key.clone(), value.clone()));
            }
        }
        self.settle_applications(cx, &batch.answers);
        for (node, class) in &batch.destroyed {
            match class.as_str() {
                "screen" => self.release_output(cx, *node),
                "window" => self.release_client(cx, *node),
                _ => {}
            }
        }
        for node in &batch.created {
            match class_name(*node).as_deref() {
                Some("screen") => self.adopt_output(cx, *node, true)?,
                Some("window") => self.adopt_client(cx, *node, true)?,
                _ => {}
            }
        }
        for node in &batch.touched {
            if let Some(output) = self.outputs.get(node).copied() {
                self.output_changed(cx, output);
                self.recompute(cx, self.output_root, output);
            }
            if let Some(client) = self.clients.get(node).copied() {
                self.client_changed(cx, client);
                self.recompute(cx, self.client_root, client);
                for list in self.lists.clone() {
                    if contains(cx, list, client) {
                        self.recompute(cx, list, client);
                    }
                }
            }
        }
        for (node, kind, x, y, button) in &batch.pointer {
            let (Some(client), Some(signal)) = (self.clients.get(node).copied(), pointer_signal(kind)) else {
                continue;
            };
            let alive = cx
                .with_state::<ClientState, _>(client, |state| state.alive)
                .unwrap_or(false);
            if alive {
                let event = record(
                    cx,
                    "pointer_event",
                    vec![
                        ("x", Val::Double(*x)),
                        ("y", Val::Double(*y)),
                        ("button", Val::Uint(*button)),
                    ],
                )?;
                let held = self.client_holders(cx, client);
                emit_member(cx, client, &held, signal, vec![cx.object_value(client), event]);
            }
        }
        Ok(())
    }

    /// The turn: what the compositor sent and the shell's own turn, in the order the TypeScript library took them.
    pub(super) fn turn(&mut self, cx: &Cx<'_>, batch: PumpBatch) -> Res<()> {
        if !batch.is_empty() || !self.booted {
            self.plan(cx, batch)?;
        }
        self.open(cx);
        self.observe_focus(cx);
        Ok(())
    }
}

/// The failure for a window or an output that is gone.
pub(super) fn gone(what: &str, id: u64, member: &str, noun: &str) -> Fail {
    Fail::new(abi::E_NO_SUCH_NODE, format!("{what}#{id}.{member}: the {noun} is gone"))
}

/// The failure of a write or a call the tree refused before it was sent. `target` names the node and member; the error of the tree says
/// why, and the code follows it: a node that is not there is `not_found`, an id that is in use `already_exists`, a move that would make a
/// node its own ancestor `invalid_argument`, the root `denied` and a shell with no ids left `unavailable`. The ABI code stays
/// `E_NO_SUCH_NODE`, as it was for each of them.
pub(super) fn write_fail(target: &str, error: &WriteError) -> Fail {
    let code = match error {
        WriteError::Tx(TxError::UnknownNode(_)) => ErrorCode::NotFound,
        WriteError::Tx(TxError::Exists(_)) => ErrorCode::AlreadyExists,
        WriteError::Tx(TxError::Cycle(_)) => ErrorCode::InvalidArgument,
        WriteError::Tx(TxError::Root(_)) => ErrorCode::Denied,
        WriteError::OutOfIds => ErrorCode::Unavailable,
    };
    Fail::new(abi::E_NO_SUCH_NODE, format!("{target}: {error}")).with_code(code)
}

/// The failure of a call made before the shell attached to the tree (`invalid_state`): the code and the words the session gave.
pub(super) fn detached((code, text): (i32, String)) -> Fail {
    Fail::new(code, text).with_code(ErrorCode::InvalidState)
}
