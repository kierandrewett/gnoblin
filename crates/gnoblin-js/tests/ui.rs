//! The shell UI toolkit (`data/js/gnoblin/ui.ts`, `titlebar.ts`) over a fake compositor.
//!
//! `FakeCompositor` is an in-memory `TreeLink` that behaves as far as the toolkit needs: it applies `New`,
//! `Insert`, `Set` and `Destroy` the way the compositor does and echoes each as a change, answers a transaction with
//! a reply, answers a `measure` call when a test says how, and destroys a window's canvases with the window. What it
//! cannot show (the compositor decoding the paint list and drawing it) is `crates/gnoblin/tests/js_shell.rs`'s.
//!
//! The Box layout cases are the Lua shell's arithmetic (`Box:arrange` in `data/lua/gnoblin/shell/ui.lua`), worked by
//! hand: the Lua corpus (`crates/gnoblin-lua/tests/lua/ui_test.lua`) tests the paint and the measure but has no
//! layout case, so each expected rectangle below is derived from that function. The paint cases are the Lua
//! test's byte for byte (`string.pack` in that file is `pack` here) and one list of every op is compared with
//! `gnoblin_paint::encode`, which the compositor's own decoder reads.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use gnoblin_js::{JsShell, JsShellConfig};
use gnoblin_paint::{PaintList, PaintOp, Rgba8, decode, encode};
use gnoblin_tree_client::tree_mirror::{
    Attached, EventFields, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeEvent,
    TreeIncoming, TreeLink, TreeTx, TxOp, TxReply,
};
use tempfile::TempDir;

const ROOT: u64 = 1;
const LEFT: u64 = 272;

// ---- the fake compositor ------------------------------------------------------------------------------------------

struct Node {
    class: String,
    parent: u64,
    children: Vec<u64>,
    props: BTreeMap<String, MirrorValue>,
}

/// A `measure` call the shell sent.
#[derive(Clone, Debug, PartialEq)]
struct Measure {
    token: u64,
    node: u64,
    size: f64,
    text: String,
}

struct Compositor {
    nodes: BTreeMap<u64, Node>,
    serial: u64,
    /// The ops of each transaction the shell sent, in order.
    sent: Vec<Vec<TxOp>>,
    measures: Vec<Measure>,
    queue: Vec<TreeIncoming>,
}

#[derive(Clone)]
struct FakeCompositor(Rc<RefCell<Compositor>>);

fn change(kind: MirrorChangeKind, node: u64) -> MirrorChange {
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

fn set_change(node: u64, key: &str, value: MirrorValue) -> MirrorChange {
    MirrorChange {
        key: key.to_owned(),
        value: Some(value),
        ..change(MirrorChangeKind::Set, node)
    }
}

fn text(value: &str) -> MirrorValue {
    MirrorValue::Text(value.to_owned())
}

impl Compositor {
    fn new() -> Self {
        let root = Node {
            class: "node".to_owned(),
            parent: 0,
            children: Vec::new(),
            props: BTreeMap::new(),
        };
        Self {
            nodes: BTreeMap::from([(ROOT, root)]),
            serial: 1,
            sent: Vec::new(),
            measures: Vec::new(),
            queue: Vec::new(),
        }
    }

    /// `node` and everything beneath it.
    fn subtree(&self, node: u64) -> Vec<u64> {
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

    fn detach(&mut self, node: u64) {
        let parent = self.nodes.get(&node).map_or(0, |entry| entry.parent);
        if let Some(entry) = self.nodes.get_mut(&parent) {
            entry.children.retain(|child| *child != node);
        }
    }

    /// Applies one op as the compositor does and returns the change it publishes.
    fn apply(&mut self, op: &TxOp) -> Option<MirrorChange> {
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
    fn new() -> Self {
        Self(Rc::new(RefCell::new(Compositor::new())))
    }

    fn land(&self, tx: &TreeTx) {
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
    fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }

    /// A window that is in the tree already, as at boot.
    fn add_window(&self, id: u64, title: &str, app_id: &str) {
        let mut c = self.0.borrow_mut();
        let props = BTreeMap::from([
            ("title".to_owned(), text(title)),
            ("app_id".to_owned(), text(app_id)),
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

    /// A window the compositor maps: created, parented under the root, given its title and app id.
    fn create_window(&self, id: u64, title: &str, app_id: &str) {
        self.add_window(id, title, app_id);
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
            set_change(id, "width", MirrorValue::Double(400.0)),
            set_change(id, "height", MirrorValue::Double(300.0)),
        ]);
    }

    /// The compositor writes a property of its own.
    fn external_set(&self, node: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            entry.props.insert(key.to_owned(), value.clone());
        }
        self.publish(vec![set_change(node, key, value)]);
    }

    /// The compositor destroys a window, and its canvases with it.
    fn destroy(&self, node: u64) {
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
    fn pointer(&self, node: u64, kind: &str, x: f64, y: f64, button: u64) {
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

    fn answer(&self, token: u64, result: &str, error: &str) {
        self.0.borrow_mut().queue.push(TreeIncoming::Answer {
            token,
            result: result.as_bytes().to_vec(),
            error: error.to_owned(),
        });
    }

    fn measures(&self) -> Vec<Measure> {
        self.0.borrow().measures.clone()
    }

    fn sent_count(&self) -> usize {
        self.0.borrow().sent.len()
    }

    /// The `(node, method)` of every call but `measure` the shell sent, in order.
    fn calls(&self) -> Vec<(u64, String)> {
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
    fn destroys_of(&self, node: u64) -> usize {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter(|op| matches!(op, TxOp::Destroy { node: gone } if *gone == node))
            .count()
    }

    fn canvases(&self) -> Vec<u64> {
        let c = self.0.borrow();
        c.nodes
            .iter()
            .filter(|(_, node)| node.class == "canvas")
            .map(|(id, _)| *id)
            .collect()
    }

    fn children(&self, node: u64) -> Vec<u64> {
        self.0
            .borrow()
            .nodes
            .get(&node)
            .map_or_else(Vec::new, |entry| entry.children.clone())
    }

    fn number(&self, node: u64, key: &str) -> f64 {
        match self.0.borrow().nodes.get(&node).and_then(|entry| entry.props.get(key)) {
            Some(MirrorValue::Double(value)) => *value,
            _ => 0.0,
        }
    }

    /// `x,y,width,height` of a canvas, as whole numbers when they are.
    fn rect(&self, node: u64) -> String {
        format!(
            "{},{},{},{}",
            self.number(node, "x"),
            self.number(node, "y"),
            self.number(node, "width"),
            self.number(node, "height")
        )
    }

    fn content(&self, node: u64) -> Option<Vec<u8>> {
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

    fn flag(&self, node: u64, key: &str) -> Option<bool> {
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

// ---- the rig ------------------------------------------------------------------------------------------------------

/// What every init file imports.
const IMPORTS: &str = "import { Box, Canvas, Grid, Icon, Text, Titlebar, defaultTitlebarStyle, defineConfig, \
                       uiCanvasCount } from \"gnoblin\";\n\
                       (globalThis as any).uiCanvasCount = uiCanvasCount;\n";

struct Rig {
    shell: JsShell,
    comp: FakeCompositor,
    load_diagnostics: Vec<String>,
    _dir: TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// Runs frames until the shell and the compositor have nothing left to say, and returns every diagnostic.
    fn settle(&mut self) -> Vec<String> {
        let mut all = Vec::new();
        for _ in 0..6 {
            all.extend(self.frame());
        }
        all
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .expect("the script has a value")
    }

    /// The node ids of the canvases `ids` holds in the script, in order.
    fn ids(&mut self, expression: &str) -> Vec<u64> {
        self.eval(&format!("{expression}.join(',')"))
            .split(',')
            .map(|id| id.parse().expect("a node id"))
            .collect()
    }
}

fn rig(init_body: &str) -> Rig {
    let comp = FakeCompositor::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), format!("{IMPORTS}{init_body}\n")).expect("init.ts is written");
    let mut shell = JsShell::new(Box::new(comp.clone()), JsShellConfig::new(&root)).expect("the shell boots");
    let load_diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    Rig {
        shell,
        comp,
        load_diagnostics,
        _dir: dir,
    }
}

// ---- Box ----------------------------------------------------------------------------------------------------------

#[test]
fn a_row_places_its_children_with_padding_and_gap_and_stretches_them_across() {
    // Derived from `Box:arrange`: the children start at the padding, a gap apart, and are as tall as the row's inside.
    let mut rig = rig(
        "const root = new Box({ width: 300, height: 50, direction: 'row', gap: 10, padding: 5 });\n\
         const a = new Box({ width: 40, height: 10 });\n\
         const b = new Box({ width: 60, height: 10 });\n\
         root.add(a).add(b);\n\
         (globalThis as any).ids = [root, a, b].map((box) => box.node);",
    );
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    assert_eq!(rig.settle(), Vec::<String>::new());
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["0,0,300,50", "5,5,40,40", "55,5,60,40"]);
    assert_eq!(
        rig.comp.children(ids[0]),
        [ids[1], ids[2]],
        "the children are children of the node"
    );
}

#[test]
fn a_row_shares_what_is_left_among_the_children_that_grow() {
    // 300 wide, padding 5 and gap 10 around three children: 300 - 10 - 20 - 40 = 230 left for the two that grow, in
    // the ratio 1 to 2: 230 * 1 // 3 = 76 and 230 * 2 // 3 = 153. The pixel the floors lose stays unused.
    let mut rig = rig(
        "const root = new Box({ width: 300, height: 50, direction: 'row', gap: 10, padding: 5 });\n\
         const a = new Box({ width: 40 });\n\
         const b = new Box({ grow: 1 });\n\
         const c = new Box({ grow: 2 });\n\
         root.add(a).add(b).add(c);\n\
         (globalThis as any).ids = [a, b, c].map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["5,5,40,40", "55,5,76,40", "141,5,153,40"]);
}

#[test]
fn a_column_stacks_its_children_and_stretches_them_across() {
    // 200 high, padding 4, gap 6: 200 - 8 - 12 - 50 = 130 for the one that grows; every child is 100 - 8 wide.
    let mut rig = rig(
        "const root = new Box({ width: 100, height: 200, direction: 'column', gap: 6, padding: 4 });\n\
         const a = new Box({ height: 30 });\n\
         const b = new Box({ grow: 1 });\n\
         const c = new Box({ height: 20 });\n\
         root.add(a).add(b).add(c);\n\
         (globalThis as any).ids = [a, b, c].map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["4,4,92,30", "4,40,92,130", "4,176,92,20"]);
}

#[test]
fn a_box_with_no_direction_leaves_its_children_where_they_are_and_nests() {
    let mut rig = rig("const root = new Box({ width: 100, height: 100 });\n\
         const inner = new Box({ x: 7, y: 9, width: 30, height: 20, direction: 'column', padding: 2 });\n\
         const leaf = new Box({ height: 5 });\n\
         root.add(inner);\n\
         inner.add(leaf);\n\
         (globalThis as any).ids = [inner, leaf].map((box) => box.node);");
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["7,9,30,20", "2,2,26,5"]);
}

#[test]
fn a_grid_puts_its_children_in_a_near_square_of_equal_cells() {
    // Three children: 2 columns and 2 rows; (100 - 8 - 2) // 2 = 45 each, 47 apart from the padding.
    let mut rig = rig(
        "const grid = new Grid({ width: 100, height: 100, padding: 4, gap: 2 });\n\
         const cells = [0, 1, 2].map(() => new Box());\n\
         cells.forEach((cell) => grid.add(cell));\n\
         (globalThis as any).ids = cells.map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["4,4,45,45", "51,4,45,45", "4,51,45,45"]);
}

#[test]
fn nothing_is_written_again_for_a_frame_that_changed_nothing() {
    let mut rig = rig("const root = new Box({ width: 10, height: 5, fill: '#ff0000' });\n\
         (globalThis as any).root = root;");
    rig.settle();
    let sent = rig.comp.sent_count();
    rig.settle();
    assert_eq!(rig.comp.sent_count(), sent, "an unchanged tree sends no transaction");
    rig.eval("root.width = 20; root.x = 3; 1");
    rig.settle();
    assert_eq!(
        rig.comp.sent_count(),
        sent + 1,
        "a changed field is one more transaction"
    );
}

// ---- Text and the measure call ------------------------------------------------------------------------------------

#[test]
fn a_text_asks_the_compositor_to_measure_and_takes_the_answer() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'Hello', size: 10 });");
    // Until the answer the metrics are the estimate: half the size a glyph, 3/2 a line, 11/10 to the baseline.
    assert_eq!(rig.eval("[t.width, t.height, t.ascent].join()"), "25,15,11");
    rig.settle();
    let node = rig.ids("[t.node]")[0];
    let asked = rig.comp.measures();
    assert_eq!(
        asked,
        [Measure {
            token: asked[0].token,
            node,
            size: 10.0,
            text: "sans\nHello".to_owned(),
        }],
        "one call, on the text's own node, with the size as a double and `font\\ntext`"
    );
    rig.comp.answer(asked[0].token, "31 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("[t.width, t.height, t.ascent].join()"), "31,15,12");
    assert_eq!(rig.comp.measures().len(), 1, "an answered text is not asked again");
}

#[test]
fn changing_the_text_falls_back_to_the_estimate_asks_again_and_ignores_a_late_answer() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'Hello', size: 10 });");
    rig.settle();
    let first = rig.comp.measures()[0].token;
    rig.comp.answer(first, "31 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("t.width"), "31");
    rig.eval("t.text = 'Hello, world'; 1");
    assert_eq!(
        rig.eval("t.width"),
        "60",
        "while the new answer is out the width is the estimate"
    );
    rig.settle();
    let asked = rig.comp.measures();
    assert_eq!(asked.len(), 2, "new text asks again");
    assert_eq!(asked[1].text, "sans\nHello, world");
    // The first question's answer arrives late: it is for text the node no longer has.
    rig.comp.answer(first, "99 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("t.width"), "60");
    rig.comp.answer(asked[1].token, "74 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("t.width"), "74");
}

#[test]
fn a_refused_measure_keeps_the_estimate_is_reported_and_is_not_asked_again() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'Hi', size: 10 });");
    rig.settle();
    let token = rig.comp.measures()[0].token;
    rig.comp.answer(token, "", "unavailable");
    let diagnostics = rig.settle();
    assert!(
        diagnostics
            .iter()
            .any(|line| line.contains("measure failed: unavailable")),
        "{diagnostics:?}"
    );
    assert_eq!(rig.eval("t.width"), "10", "the estimate stays");
    rig.settle();
    assert_eq!(
        rig.comp.measures().len(),
        1,
        "a refusal is not asked again for the same text"
    );
}

#[test]
fn a_multi_line_text_is_as_tall_as_its_lines_and_as_wide_as_its_widest() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'a\\nbbbb\\ncc', size: 10 });");
    assert_eq!(rig.eval("[t.width, t.height].join()"), "20,45");
}

#[test]
fn a_text_that_ellipsizes_is_cut_to_the_room_its_node_has() {
    // No answer: the estimate is 19 glyphs at 5 pixels, 95 wide; 50 pixels keep 19 * 50 // 95 - 1 = 9 glyphs.
    let mut rig = rig(
        "const roomy = new Text({ text: 'A long window title', size: 10, ellipsize: true });\n\
         const cut = new Text({ text: 'A long window title', size: 10, ellipsize: true, width: 50 });\n\
         const plain = new Text({ text: 'A long window title', size: 10, width: 50 });\n\
         (globalThis as any).ids = [roomy, cut, plain].map((t) => t.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let shown = |id: u64| {
        let list = decode(&rig.comp.content(id).expect("a paint list")).expect("it decodes");
        match &list.ops[..] {
            [PaintOp::Text { text, .. }] => text.clone(),
            other => panic!("one text op, got {other:?}"),
        }
    };
    assert_eq!(
        shown(ids[0]),
        "A long window title",
        "with room enough the whole text is painted"
    );
    assert_eq!(
        shown(ids[1]),
        "A long wi\u{2026}",
        "a text over its room ends in an ellipsis"
    );
    assert_eq!(
        shown(ids[2]),
        "A long window title",
        "a text that does not ellipsize paints all of it"
    );
}

// ---- the paint list -----------------------------------------------------------------------------------------------

fn f32le(value: f32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn u32le(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

/// `string.pack("<s2", s)`.
fn str2(value: &str) -> Vec<u8> {
    let mut out = u16::try_from(value.len()).expect("short").to_le_bytes().to_vec();
    out.extend(value.as_bytes());
    out
}

/// `string.pack("<s4", s)`.
fn str4(value: &str) -> Vec<u8> {
    let mut out = u32le(u32::try_from(value.len()).expect("short"));
    out.extend(value.as_bytes());
    out
}

fn list(ops: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = b"GNPL".to_vec();
    out.push(1);
    out.extend(u32le(ops.len() as u32));
    out.extend(ops.into_iter().flatten());
    out
}

#[test]
fn a_box_paints_its_fill_and_paints_again_when_the_fill_changes() {
    // `a Box paints its fill into its content` of the Lua tests, byte for byte.
    let mut rig = rig(
        "const b = new Box({ width: 10, height: 5, fill: '#ff0000', radius: 2 });\n\
         (globalThis as any).b = b;\n\
         (globalThis as any).ids = [b.node];",
    );
    rig.settle();
    let id = rig.ids("ids")[0];
    let rect = |fill: u32| {
        let mut op = vec![1_u8];
        for value in [0.0, 0.0, 10.0, 5.0, 2.0] {
            op.extend(f32le(value));
        }
        op.extend(u32le(fill));
        op.extend(u32le(0));
        op.extend(f32le(0.0));
        list(vec![op])
    };
    assert_eq!(rig.comp.content(id), Some(rect(0xff00_00ff)));
    rig.eval("b.fill = '#00ff0080'; 1");
    rig.settle();
    assert_eq!(
        rig.comp.content(id),
        Some(rect(0x00ff_0080)),
        "a new fill is painted again"
    );
}

#[test]
fn a_text_and_an_icon_paint_themselves_with_their_colours() {
    let mut rig = rig(
        "const t = new Text({ text: 'ab', size: 13, color: '#c6d0f5', font: 'mono' });\n\
         const i = new Icon({ icon: 'window-close-symbolic', size: 16, color: '#fff' });\n\
         (globalThis as any).ids = [t, i].map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let mut text_op = vec![3_u8];
    for value in [0.0, 0.0, 13.0] {
        text_op.extend(f32le(value));
    }
    text_op.extend(u32le(0xc6d0_f5ff));
    text_op.extend(str2("mono"));
    text_op.extend(str4("ab"));
    let mut icon_op = vec![4_u8];
    for value in [0.0, 0.0, 16.0] {
        icon_op.extend(f32le(value));
    }
    icon_op.extend(u32le(0xffff_ffff));
    icon_op.extend(str2("window-close-symbolic"));
    assert_eq!(rig.comp.content(ids[0]), Some(list(vec![text_op])));
    assert_eq!(rig.comp.content(ids[1]), Some(list(vec![icon_op])));
    assert_eq!(rig.comp.rect(ids[1]), "0,0,16,16", "an icon is as big as its size");
}

#[test]
fn a_text_squeezed_shorter_than_its_line_is_centred_in_its_node() {
    // The estimate of a 13 pixel line is 19 high; a node 13 high trims 3 from each end: y is -3.
    let mut rig = rig(
        "const t = new Text({ text: 'ab', size: 13, color: '#fff', height: 13 });\n\
         (globalThis as any).ids = [t.node];",
    );
    rig.settle();
    let id = rig.ids("ids")[0];
    let mut op = vec![3_u8];
    for value in [0.0, -3.0, 13.0] {
        op.extend(f32le(value));
    }
    op.extend(u32le(0xffff_ffff));
    op.extend(str2("sans"));
    op.extend(str4("ab"));
    assert_eq!(rig.comp.content(id), Some(list(vec![op])));
}

#[test]
fn a_canvas_paints_the_ops_its_draw_returns_in_the_encoding_the_compositor_decodes() {
    let mut rig = rig("class Chart extends Canvas {\n\
             draw() {\n\
                 return [\n\
                     { op: 'rect', x: 1, y: 2, width: 30, height: 40, radius: 3, fill: '#102030', stroke: '#f0a8', strokeWidth: 1.5 },\n\
                     { op: 'line', x1: 0, y1: 0, x2: 10, y2: 20, width: 2, color: '#ffffff80' },\n\
                     { op: 'text', text: 'h\\u00e9llo', x: 4, y: 5, size: 13, color: '#c6d0f5', font: 'mono' },\n\
                     { op: 'icon', name: 'window-close-symbolic', x: 1, y: 1, size: 16, color: '#fff' },\n\
                     { op: 'pushClip', x: 0, y: 0, width: 8, height: 8, radius: 2 },\n\
                     { op: 'popClip' },\n\
                     { op: 'pushOpacity', alpha: 0.5 },\n\
                     { op: 'popOpacity' },\n\
                 ];\n\
             }\n\
         }\n\
         const chart = new Chart({ width: 40, height: 40 });\n\
         (globalThis as any).ids = [chart.node];");
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    rig.settle();
    let id = rig.ids("ids")[0];
    let expected = PaintList {
        ops: vec![
            PaintOp::Rect {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 40.0,
                radius: 3.0,
                fill: Rgba8::new(0x10, 0x20, 0x30, 0xff),
                stroke: Rgba8::new(0xff, 0x00, 0xaa, 0x88),
                stroke_width: 1.5,
            },
            PaintOp::TextLine {
                x1: 0.0,
                y1: 0.0,
                x2: 10.0,
                y2: 20.0,
                width: 2.0,
                colour: Rgba8::new(0xff, 0xff, 0xff, 0x80),
            },
            PaintOp::Text {
                x: 4.0,
                y: 5.0,
                size: 13.0,
                colour: Rgba8::new(0xc6, 0xd0, 0xf5, 0xff),
                font: "mono".to_owned(),
                text: "h\u{e9}llo".to_owned(),
            },
            PaintOp::Icon {
                x: 1.0,
                y: 1.0,
                size: 16.0,
                colour: Rgba8::new(0xff, 0xff, 0xff, 0xff),
                name: "window-close-symbolic".to_owned(),
            },
            PaintOp::PushClip {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
                radius: 2.0,
            },
            PaintOp::PopClip,
            PaintOp::PushOpacity(0.5),
            PaintOp::PopOpacity,
        ],
    };
    let content = rig.comp.content(id).expect("a paint list");
    assert_eq!(
        content,
        encode(&expected),
        "the same bytes the compositor's encoder writes"
    );
    assert_eq!(decode(&content).expect("it decodes"), expected);
    assert_eq!(rig.comp.rect(id), "0,0,40,40");
}

#[test]
fn an_op_that_cannot_be_encoded_is_reported_and_leaves_the_content_alone() {
    let mut rig = rig("class Bad extends Canvas {\n\
             fixed = false;\n\
             draw() {\n\
                 return [{ op: 'rect', x: 0, y: 0, width: 4, height: 4, fill: this.fixed ? '#000' : 'red' }];\n\
             }\n\
         }\n\
         const bad = new Bad({ width: 4, height: 4 });\n\
         (globalThis as any).bad = bad;\n\
         (globalThis as any).ids = [bad.node];");
    let diagnostics = rig.settle();
    let id = rig.ids("ids")[0];
    assert!(
        diagnostics.iter().any(|line| line.contains("draw op 1")),
        "the bad op is named: {diagnostics:?}"
    );
    assert_eq!(rig.comp.content(id), None, "nothing is written for a bad op");
    rig.eval("bad.fixed = true; 1");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert!(
        rig.comp.content(id).is_some(),
        "a good list is written once it is fixed"
    );
}

// ---- the titlebar -------------------------------------------------------------------------------------------------

/// A client the titlebar can decorate, kept in the script.
const STUB_CLIENT: &str = "const client = { title: 'My title', focused: true, fullscreen: false, width: 400, x: 10, y: 20, \
                           maximised: false, minimised: false, closed: 0, close() { this.closed += 1; } };\n\
                           (globalThis as any).client = client;\n";

fn texts_of(list: &PaintList) -> Vec<String> {
    list.ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_titlebar_is_a_box_of_a_drag_area_with_the_title_and_the_three_buttons() {
    let mut rig = rig(&format!(
        "{STUB_CLIENT}const bar = new Titlebar(client);\n\
         bar.mount({ROOT});\n\
         (globalThis as any).bar = bar;\n\
         (globalThis as any).ids = [bar, bar.dragArea, bar.title, ...bar.buttons.values()].map((c) => c.node);"
    ));
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    rig.settle();
    let ids = rig.ids("ids");
    let (bar, drag, title, minimise, maximise, close) = (ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]);
    assert_eq!(
        rig.comp.children(ROOT),
        [bar],
        "the bar hangs under the node it was mounted on"
    );
    assert_eq!(rig.comp.children(bar), [drag, minimise, maximise, close]);
    assert_eq!(rig.comp.children(drag), [title]);
    // 400 wide, padding 6: the three 16 pixel buttons and the gaps of 8 leave 316 for the drag area.
    assert_eq!(rig.comp.rect(bar), "0,0,400,28");
    assert_eq!(rig.comp.rect(drag), "6,6,316,16");
    assert_eq!(rig.comp.rect(title), "0,0,316,16");
    assert_eq!(
        [rig.comp.rect(minimise), rig.comp.rect(maximise), rig.comp.rect(close)],
        ["330,6,16,16", "354,6,16,16", "378,6,16,16"]
    );
    let title_list = decode(&rig.comp.content(title).expect("the title is painted")).expect("it decodes");
    assert_eq!(texts_of(&title_list), ["My title"]);
    let bar_list = decode(&rig.comp.content(bar).expect("the bar is painted")).expect("it decodes");
    assert!(
        matches!(&bar_list.ops[..], [PaintOp::Rect { fill, .. }] if *fill == Rgba8::new(0x41, 0x45, 0x59, 0xff)),
        "a focused bar is filled with the focused colour: {bar_list:?}"
    );
    let close_list = decode(&rig.comp.content(close).expect("the button is painted")).expect("it decodes");
    assert!(matches!(&close_list.ops[..], [PaintOp::Icon { name, .. }] if name == "window-close-symbolic"));
}

#[test]
fn a_titlebar_follows_its_client_and_takes_its_look_from_plain_fields() {
    let mut rig = rig(&format!(
        "{STUB_CLIENT}const style = {{ ...defaultTitlebarStyle, height: 40, buttons: ['close'], focusedFill: '#112233', \
         unfocusedFill: '#445566' }};\n\
         const bar = new Titlebar(client, style);\n\
         bar.mount({ROOT});\n\
         (globalThis as any).ids = [bar, bar.title].map((c) => c.node);"
    ));
    rig.settle();
    let ids = rig.ids("ids");
    let (bar, title) = (ids[0], ids[1]);
    assert_eq!(rig.comp.rect(bar), "0,0,400,40");
    assert_eq!(
        rig.comp.children(bar).len(),
        2,
        "the drag area and the one button the style names"
    );
    let fill = |rig: &Rig| match &decode(&rig.comp.content(bar).expect("painted")).expect("decodes").ops[..] {
        [PaintOp::Rect { fill, .. }] => *fill,
        other => panic!("one rect, got {other:?}"),
    };
    assert_eq!(fill(&rig), Rgba8::new(0x11, 0x22, 0x33, 0xff));
    rig.eval("client.focused = false; client.width = 600; client.title = 'Renamed'; 1");
    rig.settle();
    assert_eq!(
        fill(&rig),
        Rgba8::new(0x44, 0x55, 0x66, 0xff),
        "the focus changed the fill"
    );
    assert_eq!(rig.comp.rect(bar), "0,0,600,40", "the bar is as wide as its client");
    let title_list = decode(&rig.comp.content(title).expect("painted")).expect("decodes");
    assert_eq!(texts_of(&title_list), ["Renamed"]);
    rig.eval("client.fullscreen = true; 1");
    rig.settle();
    assert_eq!(
        rig.comp.flag(bar, "visible"),
        Some(false),
        "a fullscreen window has no titlebar"
    );
}

#[test]
fn the_buttons_and_the_bar_answer_a_press() {
    let mut rig = rig(&format!(
        "{STUB_CLIENT}const bar = new Titlebar(client);\n\
         bar.mount({ROOT});\n\
         (globalThis as any).clock = 0;\n\
         bar.now = () => (globalThis as any).clock;\n\
         (globalThis as any).ids = [bar, bar.title, bar.buttons.get('minimise'), bar.buttons.get('maximise'), \
         bar.buttons.get('close')].map((c) => c.node);"
    ));
    rig.settle();
    let ids = rig.ids("ids");
    let (title, minimise, maximise, close) = (ids[1], ids[2], ids[3], ids[4]);
    let state =
        |rig: &mut Rig| rig.eval("[client.closed, client.maximised, client.minimised, client.x, client.y].join()");

    rig.comp.pointer(close, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,false,false,10,20",
        "the close button closes the client"
    );

    rig.comp.pointer(minimise, "press", 5.0, 5.0, LEFT);
    rig.comp.pointer(maximise, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,true,true,10,20",
        "minimise and maximise write the client's state"
    );

    // A press on the title starts a drag: it moves the client by how far the pointer goes in the bar's coordinates.
    // The title sits 6 pixels in and down in the bar, and the event carries the title's own coordinates.
    rig.comp.pointer(title, "press", 3.0, 3.0, LEFT);
    rig.comp.pointer(title, "motion", 13.0, 5.0, 0);
    rig.settle();
    assert_eq!(state(&mut rig), "1,true,true,20,22");
    rig.comp.pointer(title, "release", 13.0, 5.0, LEFT);
    rig.comp.pointer(title, "motion", 50.0, 50.0, 0);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,true,true,20,22",
        "a released drag does not move the client"
    );

    // A second press on the bar within half a second toggles the maximised state instead of starting a drag.
    rig.eval("clock = 1000; 1");
    rig.comp.pointer(title, "press", 3.0, 3.0, LEFT);
    rig.comp.pointer(title, "release", 3.0, 3.0, LEFT);
    rig.eval("clock = 1200; 1");
    rig.comp.pointer(title, "press", 3.0, 3.0, LEFT);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,false,true,20,22",
        "a double click restores the maximised window"
    );
}

// ---- decorations --------------------------------------------------------------------------------------------------

#[test]
fn a_window_gets_one_titlebar_and_loses_it_when_the_window_closes() {
    let mut rig = rig("defineConfig({ decorations: true });");
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    rig.settle();
    assert!(rig.comp.canvases().is_empty(), "no window, no titlebar");

    rig.comp.create_window(3, "term", "foot");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    let bars = rig.comp.children(3);
    assert_eq!(bars.len(), 1, "exactly one titlebar node under the window");
    assert_eq!(
        rig.comp.canvases().len(),
        6,
        "the bar, its drag area, its title and its three buttons"
    );
    assert_eq!(rig.comp.rect(bars[0]), "0,0,400,28", "the bar is as wide as the window");
    assert_eq!(rig.eval("uiCanvasCount()"), "6");

    rig.settle();
    assert_eq!(rig.comp.children(3).len(), 1, "later frames do not add another");

    let title = rig.comp.children(rig.comp.children(bars[0])[0])[0];
    rig.comp.external_set(3, "title", text("renamed"));
    rig.settle();
    let list = decode(&rig.comp.content(title).expect("painted")).expect("decodes");
    assert_eq!(texts_of(&list), ["renamed"], "the title follows the window's");

    rig.comp.destroy(3);
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert!(
        rig.comp.canvases().is_empty(),
        "the compositor took the titlebar's nodes with the window"
    );
    assert_eq!(rig.eval("uiCanvasCount()"), "0", "and the toolkit forgot them");
    assert_eq!(
        rig.comp.destroys_of(bars[0]),
        0,
        "the shell does not destroy what is already gone"
    );
}

#[test]
fn every_window_has_a_titlebar_of_its_own_including_the_ones_there_at_boot() {
    let comp = FakeCompositor::new();
    comp.add_window(3, "one", "a");
    comp.add_window(4, "two", "b");
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        format!("{IMPORTS}defineConfig({{ decorations: true }});\n"),
    )
    .expect("init.ts");
    let mut shell = JsShell::new(Box::new(comp.clone()), JsShellConfig::new(&root)).expect("the shell boots");
    shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    for _ in 0..6 {
        shell.frame().expect("a frame");
    }
    assert_eq!(comp.children(3).len(), 1);
    assert_eq!(comp.children(4).len(), 1);
    assert_ne!(comp.children(3), comp.children(4));
    assert_eq!(comp.canvases().len(), 12);
}

#[test]
fn a_window_has_no_titlebar_unless_the_config_asks_for_one() {
    for init in [
        "export {};",
        "defineConfig({});",
        "defineConfig({ decorations: false });",
    ] {
        let mut rig = rig(init);
        rig.settle();
        rig.comp.create_window(3, "term", "foot");
        rig.settle();
        assert!(rig.comp.children(3).is_empty(), "{init}");
        assert!(rig.comp.canvases().is_empty(), "{init}");
    }
}

#[test]
fn the_close_button_of_a_decorated_window_closes_that_window() {
    let mut rig = rig("defineConfig({ decorations: true });");
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let bar = rig.comp.children(3)[0];
    let close = *rig.comp.children(bar).last().expect("the close button is last");
    rig.comp.pointer(close, "press", 4.0, 4.0, LEFT);
    rig.settle();
    assert_eq!(rig.comp.calls(), [(3, "close".to_owned())]);
}

#[test]
fn retiring_a_generation_takes_its_titlebars_away() {
    let mut rig = rig("defineConfig({ decorations: true });");
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let bar = rig.comp.children(3)[0];
    rig.shell.retire().expect("retire");
    rig.settle();
    assert_eq!(rig.comp.destroys_of(bar), 1);
    assert!(
        rig.comp.canvases().is_empty(),
        "no canvas of the old generation is left behind"
    );
}
