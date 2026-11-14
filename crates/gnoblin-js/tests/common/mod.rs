//! The fake compositor and the shell rig the UI tests share: an in-memory `TreeLink` that applies `New`, `Insert`,
//! `Set` and `Destroy` as the compositor does, echoes each as a change and answers a transaction with a reply.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_js::{JsManualClock, JsRoleConfig, JsShell, JsShellConfig};
use gnoblin_tree_client::tree_mirror::{
    Attached, EventFields, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeEvent,
    TreeIncoming, TreeLink, TreeTx, TxOp, TxReply,
};
use tempfile::TempDir;

pub mod policy;
mod xr_support;

pub const ROOT: u64 = 1;
pub const LEFT: u64 = 272;

// ---- the fake compositor ------------------------------------------------------------------------------------------

pub struct Node {
    pub class: String,
    pub parent: u64,
    pub children: Vec<u64>,
    pub props: BTreeMap<String, MirrorValue>,
}

/// A `measure` call the shell sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Measure {
    pub token: u64,
    pub node: u64,
    pub size: f64,
    pub text: String,
}

/// A claim the shell holds on a seat: a chord, a gesture or a device, by the id it allocated.
#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub id: i64,
    /// `claim`, `claim_gesture` or `claim_device`.
    pub method: String,
    /// The call's text: the chord, the gesture's kind or the device name, behind `device` and a line feed when scoped.
    pub text: String,
    /// The claim's second integer: a chord's grab flag or a gesture's finger count; 0 for a device.
    pub second: i64,
}

pub struct Compositor {
    pub nodes: BTreeMap<u64, Node>,
    /// The claims made and not given up, in the order they were made.
    pub claims: Vec<Claim>,
    pub serial: u64,
    /// The ops of each transaction the shell sent, in order.
    pub sent: Vec<Vec<TxOp>>,
    pub measures: Vec<Measure>,
    pub queue: Vec<TreeIncoming>,
}

#[derive(Clone)]
pub struct FakeCompositor(Rc<RefCell<Compositor>>);

pub fn change(kind: MirrorChangeKind, node: u64) -> MirrorChange {
    MirrorChange {
        kind,
        node,
        class: String::new(),
        parent: 0,
        index: 0,
        key: String::new(),
        value: None,
        removed: Vec::new(),
    }
}

pub fn set_change(node: u64, key: &str, value: MirrorValue) -> MirrorChange {
    MirrorChange {
        key: key.to_owned(),
        value: Some(value),
        ..change(MirrorChangeKind::Set, node)
    }
}

pub fn text(value: &str) -> MirrorValue {
    MirrorValue::Text(value.to_owned())
}

impl Compositor {
    pub fn new() -> Self {
        let root = Node {
            class: "node".to_owned(),
            parent: 0,
            children: Vec::new(),
            props: BTreeMap::new(),
        };
        Self {
            nodes: BTreeMap::from([(ROOT, root)]),
            serial: 1,
            claims: Vec::new(),
            sent: Vec::new(),
            measures: Vec::new(),
            queue: Vec::new(),
        }
    }

    /// `node` and everything beneath it.
    pub fn subtree(&self, node: u64) -> Vec<u64> {
        let mut out = vec![node];
        let mut at = 0;
        while at < out.len() {
            if let Some(entry) = self.nodes.get(&out[at]) {
                out.extend(entry.children.iter().copied());
            }
            at += 1;
        }
        out
    }

    pub fn detach(&mut self, node: u64) {
        let parent = self.nodes.get(&node).map_or(0, |entry| entry.parent);
        if let Some(entry) = self.nodes.get_mut(&parent) {
            entry.children.retain(|child| *child != node);
        }
    }

    /// Applies one op as the compositor does and returns the change it publishes.
    pub fn apply(&mut self, op: &TxOp) -> Option<MirrorChange> {
        match op {
            TxOp::New { id, class } => {
                self.nodes.insert(
                    *id,
                    Node {
                        class: class.clone(),
                        parent: 0,
                        children: Vec::new(),
                        props: BTreeMap::new(),
                    },
                );
                Some(MirrorChange {
                    class: class.clone(),
                    ..change(MirrorChangeKind::Created, *id)
                })
            }
            TxOp::Insert { node, parent, index } => {
                self.detach(*node);
                let held = self.nodes.get_mut(parent)?;
                let at = usize::try_from(*index).map_or(held.children.len(), |i| i.min(held.children.len()));
                held.children.insert(at, *node);
                self.nodes.get_mut(node)?.parent = *parent;
                Some(MirrorChange {
                    parent: *parent,
                    index: at as i64,
                    ..change(MirrorChangeKind::Parented, *node)
                })
            }
            TxOp::Set { node, key, value, .. } => {
                self.nodes.get_mut(node)?.props.insert(key.clone(), value.clone());
                Some(set_change(*node, key, value.clone()))
            }
            TxOp::Destroy { node } => {
                let removed = self.subtree(*node);
                self.detach(*node);
                for id in &removed {
                    self.nodes.remove(id);
                }
                Some(MirrorChange {
                    removed,
                    ..change(MirrorChangeKind::Destroyed, *node)
                })
            }
            TxOp::Call {
                node,
                method,
                args,
                token,
                text,
            } => {
                let id_bits = args.get(0..8).and_then(|bits| <[u8; 8]>::try_from(bits).ok());
                if matches!(method.as_str(), "claim" | "claim_gesture" | "claim_device")
                    && let Some(bits) = id_bits
                {
                    self.claims.push(Claim {
                        id: i64::from_le_bytes(bits),
                        method: method.clone(),
                        text: text.expose().to_owned(),
                        second: args
                            .get(8..16)
                            .and_then(|bits| <[u8; 8]>::try_from(bits).ok())
                            .map_or(0, i64::from_le_bytes),
                    });
                }
                if method == "unclaim"
                    && let Some(bits) = id_bits
                {
                    let id = i64::from_le_bytes(bits);
                    self.claims.retain(|claim| claim.id != id);
                }
                if method == "measure" {
                    let bits: [u8; 8] = args.as_slice().try_into().ok()?;
                    self.measures.push(Measure {
                        token: *token,
                        node: *node,
                        size: f64::from_le_bytes(bits),
                        text: text.expose().to_owned(),
                    });
                }
                None
            }
        }
    }
}

impl FakeCompositor {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(Compositor::new())))
    }

    pub fn land(&self, tx: &TreeTx) {
        let mut c = self.0.borrow_mut();
        c.sent.push(tx.ops.clone());
        let mut changes = Vec::new();
        for op in &tx.ops {
            if let Some(applied) = c.apply(op) {
                changes.push(applied);
            }
        }
        c.serial = tx.serial;
        // The compositor sends no change message for a transaction that changed no node.
        if !changes.is_empty() {
            c.queue.push(TreeIncoming::Changes {
                serial: tx.serial,
                changes,
            });
        }
        c.queue.push(TreeIncoming::Reply(TxReply {
            serial: tx.serial,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: Vec::new(),
        }));
    }

    /// Publishes `changes` as a batch of the compositor's own.
    pub fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }

    /// A window that is in the tree already, as at boot. It is server-decorated: the shell's titlebar frames it.
    pub fn add_window(&self, id: u64, title: &str, app_id: &str) {
        self.add_window_decorated(id, title, app_id, "server");
    }

    /// A window that is in the tree already, with the decoration the compositor negotiated (`client`, `server` or
    /// `none`).
    pub fn add_window_decorated(&self, id: u64, title: &str, app_id: &str, decoration: &str) {
        let mut c = self.0.borrow_mut();
        let props = BTreeMap::from([
            ("title".to_owned(), text(title)),
            ("app_id".to_owned(), text(app_id)),
            ("decoration".to_owned(), text(decoration)),
            ("width".to_owned(), MirrorValue::Double(400.0)),
            ("height".to_owned(), MirrorValue::Double(300.0)),
        ]);
        c.nodes.insert(
            id,
            Node {
                class: "window".to_owned(),
                parent: ROOT,
                children: Vec::new(),
                props,
            },
        );
        if let Some(root) = c.nodes.get_mut(&ROOT) {
            root.children.push(id);
        }
    }

    /// A window the compositor maps: created, parented under the root, given its title and app id. It is
    /// server-decorated, as a client that asked for the shell's frame is.
    pub fn create_window(&self, id: u64, title: &str, app_id: &str) {
        self.create_window_decorated(id, title, app_id, "server");
    }

    /// A window the compositor maps with the decoration it negotiated: `client` for one that draws its own title bar.
    pub fn create_window_decorated(&self, id: u64, title: &str, app_id: &str, decoration: &str) {
        self.add_window_decorated(id, title, app_id, decoration);
        self.publish(vec![
            MirrorChange {
                class: "window".to_owned(),
                ..change(MirrorChangeKind::Created, id)
            },
            MirrorChange {
                parent: ROOT,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            },
            set_change(id, "title", text(title)),
            set_change(id, "app_id", text(app_id)),
            set_change(id, "decoration", text(decoration)),
            set_change(id, "width", MirrorValue::Double(400.0)),
            set_change(id, "height", MirrorValue::Double(300.0)),
        ]);
    }

    /// A screen in the tree already, as at boot: named, placed and sized.
    pub fn add_screen(&self, id: u64, name: &str, (x, y, width, height): (f64, f64, f64, f64), primary: bool) {
        self.add_node(
            id,
            ROOT,
            "screen",
            vec![
                ("name", text(name)),
                ("x", MirrorValue::Double(x)),
                ("y", MirrorValue::Double(y)),
                ("width", MirrorValue::Double(width)),
                ("height", MirrorValue::Double(height)),
                ("primary", MirrorValue::Bool(primary)),
            ],
        );
    }

    /// A screen the compositor makes while the shell runs, as a hotplugged output: created, parented under the root,
    /// then given its name, place, size and primary flag.
    pub fn create_screen(&self, id: u64, name: &str, rect: (f64, f64, f64, f64), primary: bool) {
        let (x, y, width, height) = rect;
        self.add_screen(id, name, rect, primary);
        self.publish(vec![
            MirrorChange {
                class: "screen".to_owned(),
                ..change(MirrorChangeKind::Created, id)
            },
            MirrorChange {
                parent: ROOT,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            },
            set_change(id, "name", text(name)),
            set_change(id, "x", MirrorValue::Double(x)),
            set_change(id, "y", MirrorValue::Double(y)),
            set_change(id, "width", MirrorValue::Double(width)),
            set_change(id, "height", MirrorValue::Double(height)),
            set_change(id, "primary", MirrorValue::Bool(primary)),
        ]);
    }

    /// The compositor writes a property of its own.
    pub fn external_set(&self, node: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            entry.props.insert(key.to_owned(), value.clone());
        }
        self.publish(vec![set_change(node, key, value)]);
    }

    /// The compositor destroys a window, and its canvases with it.
    pub fn destroy(&self, node: u64) {
        let removed = {
            let mut c = self.0.borrow_mut();
            let removed = c.subtree(node);
            c.detach(node);
            for id in &removed {
                c.nodes.remove(id);
            }
            removed
        };
        self.publish(vec![MirrorChange {
            removed,
            ..change(MirrorChangeKind::Destroyed, node)
        }]);
    }

    /// A pointer event addressed to `node`.
    pub fn pointer(&self, node: u64, kind: &str, x: f64, y: f64, button: u64) {
        self.0.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: kind.to_owned(),
            fields: EventFields {
                x,
                y,
                button,
                ..EventFields::default()
            },
        }));
    }

    pub fn answer(&self, token: u64, result: &str, error: &str) {
        self.0.borrow_mut().queue.push(TreeIncoming::Answer {
            token,
            result: result.as_bytes().to_vec(),
            error: error.to_owned(),
        });
    }

    pub fn measures(&self) -> Vec<Measure> {
        self.0.borrow().measures.clone()
    }

    pub fn sent_count(&self) -> usize {
        self.0.borrow().sent.len()
    }

    /// The ops of the transactions the shell sent after the first `mark`, in order: `mark` is a `sent_count` taken before.
    pub fn ops_since(&self, mark: usize) -> Vec<TxOp> {
        self.0.borrow().sent.iter().skip(mark).flatten().cloned().collect()
    }

    /// The `(node, method)` of every call but `measure` the shell sent, in order.
    pub fn calls(&self) -> Vec<(u64, String)> {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter_map(|op| match op {
                TxOp::Call { node, method, .. } if method != "measure" => Some((*node, method.clone())),
                _ => None,
            })
            .collect()
    }

    /// How many `Destroy` ops the shell sent for `node`.
    pub fn destroys_of(&self, node: u64) -> usize {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter(|op| matches!(op, TxOp::Destroy { node: gone } if *gone == node))
            .count()
    }

    pub fn canvases(&self) -> Vec<u64> {
        let c = self.0.borrow();
        c.nodes
            .iter()
            .filter(|(_, node)| node.class == "canvas")
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn children(&self, node: u64) -> Vec<u64> {
        self.0
            .borrow()
            .nodes
            .get(&node)
            .map_or_else(Vec::new, |entry| entry.children.clone())
    }

    pub fn number(&self, node: u64, key: &str) -> f64 {
        match self.0.borrow().nodes.get(&node).and_then(|entry| entry.props.get(key)) {
            Some(MirrorValue::Double(value)) => *value,
            _ => 0.0,
        }
    }

    /// `x,y,width,height` of a canvas, as whole numbers when they are.
    pub fn rect(&self, node: u64) -> String {
        format!(
            "{},{},{},{}",
            self.number(node, "x"),
            self.number(node, "y"),
            self.number(node, "width"),
            self.number(node, "height")
        )
    }

    pub fn content(&self, node: u64) -> Option<Vec<u8>> {
        match self
            .0
            .borrow()
            .nodes
            .get(&node)
            .and_then(|entry| entry.props.get("content"))
        {
            Some(MirrorValue::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        }
    }

    pub fn flag(&self, node: u64, key: &str) -> Option<bool> {
        match self.0.borrow().nodes.get(&node).and_then(|entry| entry.props.get(key)) {
            Some(MirrorValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }
}

impl TreeLink for FakeCompositor {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        let mut c = self.0.borrow_mut();
        let attached = Attached {
            epoch: 1,
            root: ROOT,
            serial: c.serial,
        };
        let nodes = c
            .nodes
            .iter()
            .map(|(id, node)| SnapshotNode {
                id: *id,
                parent: node.parent,
                index: c
                    .nodes
                    .get(&node.parent)
                    .and_then(|parent| parent.children.iter().position(|child| child == id))
                    .map_or(0, |at| at as i64),
                class: node.class.clone(),
                title: String::new(),
                app_id: String::new(),
                props: node.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            })
            .collect();
        let snapshot = TreeIncoming::Snapshot {
            serial: c.serial,
            chunk: 0,
            last: true,
            nodes,
        };
        c.queue.push(TreeIncoming::Attached(attached));
        c.queue.push(snapshot);
        Ok(())
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        self.land(tx);
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.0.borrow_mut().queue))
    }
}

// ---- time ---------------------------------------------------------------------------------------------------------
//
// The rule for every test in this crate (design/js-runtime.md, "Unit test timing"): no assertion may rest on how fast
// the machine is. A script that is not meant to run out of time gets `HONEST_BUDGET`, and anything that waits
// (an arming window, a fade, a delay, `gnoblin.timers.after`) waits on the injected clock of `timed_config`, which
// moves only when the test moves it. A test that is about a budget runs a script that never ends, keeps the budget
// short and asserts that it was interrupted, never when.

/// What one honest script of a test may use per call. The product default is 100 ms for a listener call, which a
/// machine that is busy for a tenth of a second would exhaust in a script that is doing nothing wrong.
pub const HONEST_BUDGET: Duration = Duration::from_secs(5);

/// The budget of a test about a script that never ends. Short, so the loop is cheap; no shorter than half a second,
/// because whatever honest script the same budget covers (the shell's own calls in a frame, a listener beside the
/// runaway one) must not be able to trip it. Such a test asserts that the script was interrupted, never when.
pub const RUNAWAY_BUDGET: Duration = Duration::from_millis(500);

/// A shell config for a test whose scripts are honest: `root`, with call and frame budgets a loaded machine cannot
/// exhaust, and the real clock.
pub fn honest_config(root: impl Into<PathBuf>) -> JsShellConfig {
    let mut config = JsShellConfig::new(root);
    config.entry_budget = HONEST_BUDGET;
    config.frame_budget = HONEST_BUDGET * 2;
    config
}

/// Like [`honest_config`], with a clock that stands at zero until the test advances it, and the handle to advance it.
pub fn timed_config(root: impl Into<PathBuf>) -> (JsShellConfig, JsManualClock) {
    let clock = JsManualClock::default();
    let mut config = honest_config(root);
    config.clock = Arc::new(clock.clone());
    (config, clock)
}

/// A runtime role over `root` for a test whose scripts are honest, with the budgets of [`honest_config`].
pub fn honest_role_config(root: impl Into<PathBuf>) -> JsRoleConfig {
    let mut config = JsRoleConfig::new(root, None);
    config.shell.entry_budget = HONEST_BUDGET;
    config.shell.frame_budget = HONEST_BUDGET * 2;
    config
}

// ---- the rig ------------------------------------------------------------------------------------------------------
pub struct Rig {
    pub shell: JsShell,
    pub comp: FakeCompositor,
    pub load_diagnostics: Vec<String>,
    pub clock: JsManualClock,
    pub _dir: TempDir,
}

impl Rig {
    /// Moves the shell's clock forward by `ms` milliseconds. The frames that follow see the time pass; no real time
    /// does.
    pub fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    pub fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// Runs frames until the shell and the compositor have nothing left to say, and returns every diagnostic.
    pub fn settle(&mut self) -> Vec<String> {
        let mut all = Vec::new();
        for _ in 0..6 {
            all.extend(self.frame());
        }
        all
    }

    pub fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .expect("the script has a value")
    }

    /// The node ids of the canvases `ids` holds in the script, in order.
    pub fn ids(&mut self, expression: &str) -> Vec<u64> {
        self.eval(&format!("{expression}.join(',')"))
            .split(',')
            .map(|id| id.parse().expect("a node id"))
            .collect()
    }
}

/// A shell over a fresh fake compositor whose `init.ts` is `imports` followed by `init_body`.
pub fn rig_with(imports: &str, init_body: &str) -> Rig {
    rig_setup(imports, init_body, |_| {})
}

/// Like [`rig_with`], with `setup` run on the compositor before the shell attaches: the nodes that are there at boot.
pub fn rig_setup(imports: &str, init_body: &str, setup: impl FnOnce(&FakeCompositor)) -> Rig {
    let comp = FakeCompositor::new();
    setup(&comp);
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), format!("{imports}{init_body}\n")).expect("init.ts is written");
    let (config, clock) = timed_config(&root);
    let mut shell = JsShell::new(Box::new(comp.clone()), config).expect("the shell boots");
    let load_diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    Rig {
        shell,
        comp,
        load_diagnostics,
        clock,
        _dir: dir,
    }
}

impl FakeCompositor {
    /// A node of `class` under `parent`, in the tree already, as at boot.
    pub fn add_node(&self, id: u64, parent: u64, class: &str, props: Vec<(&str, MirrorValue)>) {
        let mut c = self.0.borrow_mut();
        c.nodes.insert(
            id,
            Node {
                class: class.to_owned(),
                parent,
                children: Vec::new(),
                props: props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            },
        );
        if let Some(entry) = c.nodes.get_mut(&parent) {
            entry.children.push(id);
        }
    }

    /// A key sent to `node`, which holds the seat's focus.
    pub fn key(&self, node: u64, key: &str, text: &str, pressed: bool) {
        self.0.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: "key".to_owned(),
            fields: EventFields {
                key: key.to_owned(),
                text: text.to_owned(),
                pressed,
                ..EventFields::default()
            },
        }));
    }

    /// The id of the live claim whose call text is `text` (a device and a line feed when scoped).
    pub fn claim_id(&self, text: &str) -> i64 {
        let c = self.0.borrow();
        c.claims
            .iter()
            .find(|claim| claim.text == text)
            .unwrap_or_else(|| panic!("no live claim for {text:?}; live: {:?}", c.claims))
            .id
    }

    /// The id of the live claim of `kind` (`swipe` or `pinch`) with `fingers` contacts.
    pub fn gesture_claim(&self, kind: &str, fingers: i64) -> i64 {
        let c = self.0.borrow();
        c.claims
            .iter()
            .find(|claim| claim.method == "claim_gesture" && claim.text == kind && claim.second == fingers)
            .unwrap_or_else(|| panic!("no live claim for {kind}{fingers}; live: {:?}", c.claims))
            .id
    }

    /// How many `set` ops the shell sent with a duration: what an animation is, as opposed to an instant write.
    pub fn timed_sets(&self) -> usize {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter(|op| matches!(op, TxOp::Set { seconds: Some(_), .. }))
            .count()
    }

    /// A claimed chord changing `state` (`press`, `release` or `repeat`).
    pub fn chord(&self, claim: i64, chord: &str, state: &str) {
        self.0
            .borrow_mut()
            .queue
            .push(TreeIncoming::Chord(gnoblin_tree_client::tree_mirror::TreeChord {
                claim,
                chord: chord.to_owned(),
                state: state.to_owned(),
            }));
    }

    /// The texts of the live claims, in the order they were made, a line feed shown as `|`.
    pub fn claimed(&self) -> Vec<String> {
        self.0
            .borrow()
            .claims
            .iter()
            .map(|claim| claim.text.replace('\n', "|"))
            .collect()
    }

    /// One phase of a touchpad gesture the shell claimed: `kind` is `swipe` or `pinch`.
    #[allow(clippy::too_many_arguments)]
    pub fn gesture(
        &self,
        claim: i64,
        kind: &str,
        fingers: i64,
        phase: &str,
        (dx, dy): (f64, f64),
        scale: f64,
        rotation: f64,
    ) {
        self.0
            .borrow_mut()
            .queue
            .push(TreeIncoming::Gesture(gnoblin_tree_client::tree_mirror::TreeGesture {
                claim,
                kind: kind.to_owned(),
                fingers,
                phase: phase.to_owned(),
                dx,
                dy,
                scale,
                rotation,
            }));
    }

    /// The compositor's request of a window (`menu`), anchored at `x`, `y`.
    pub fn request(&self, node: u64, name: &str, x: f64, y: f64) {
        self.0.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: "request".to_owned(),
            fields: EventFields {
                x,
                y,
                request: name.to_owned(),
                ..EventFields::default()
            },
        }));
    }

    /// The value of a property of a node, as the compositor holds it.
    pub fn value(&self, node: u64, key: &str) -> Option<MirrorValue> {
        self.0
            .borrow()
            .nodes
            .get(&node)
            .and_then(|entry| entry.props.get(key).cloned())
    }

    /// The text of every `text` op of the canvas `node`'s paint list.
    pub fn text_of(&self, node: u64) -> Vec<String> {
        self.0
            .borrow()
            .nodes
            .get(&node)
            .and_then(|entry| entry.props.get("content"))
            .map_or_else(Vec::new, |content| match content {
                MirrorValue::Bytes(bytes) => gnoblin_paint::decode(bytes).map_or_else(
                    |_| Vec::new(),
                    |list| {
                        list.ops
                            .iter()
                            .filter_map(|op| match op {
                                gnoblin_paint::PaintOp::Text { text, .. } => Some(text.clone()),
                                _ => None,
                            })
                            .collect()
                    },
                ),
                _ => Vec::new(),
            })
    }

    /// Every canvas whose paint list holds `text`, in id order.
    pub fn canvases_showing(&self, text: &str) -> Vec<u64> {
        self.canvases()
            .into_iter()
            .filter(|id| self.text_of(*id).iter().any(|t| t == text))
            .collect()
    }
}
