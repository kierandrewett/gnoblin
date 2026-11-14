//! Workspaces and the layout driver of `JsShell`, over a fake compositor.
//!
//! The compositor is `FakeLink`, a trimmed copy of the one in `tests/shell.rs`: it applies every
//! transaction to a node map at once and echoes it as a change batch. The tree has a root (1) and a primary
//! screen (2) of 1000 by 800; windows are made by `create_window`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, JsShell, JsShellConfig, stock_modules};
use gnoblin_tree_client::tree_mirror::{
    Attached, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeIncoming, TreeLink, TreeTx,
    TxOp, TxReply,
};
use tempfile::TempDir;

struct Node {
    class: String,
    parent: u64,
    props: BTreeMap<String, MirrorValue>,
}

#[derive(Default)]
struct Compositor {
    nodes: BTreeMap<u64, Node>,
    serial: u64,
    /// Transactions the link has received, in order.
    sent: Vec<Vec<TxOp>>,
    queue: Vec<TreeIncoming>,
}

type Comp = Rc<RefCell<Compositor>>;

#[derive(Clone)]
struct FakeLink(Comp);

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
        key: key.into(),
        value: Some(value),
        ..change(MirrorChangeKind::Set, node)
    }
}

impl FakeLink {
    /// Publishes `changes` as a batch of the compositor's own.
    fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }

    /// A native write of `node.key` the shell did not make.
    fn external_set(&self, node: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            entry.props.insert(key.to_owned(), value.clone());
        }
        self.publish(vec![set_change(node, key, value)]);
    }

    /// A window the compositor maps: created, parented under the root, with its title and app id.
    fn create_window(&self, id: u64, title: &str) {
        let props = BTreeMap::from([
            ("title".to_owned(), MirrorValue::Text(title.into())),
            ("app_id".to_owned(), MirrorValue::Text("app".into())),
        ]);
        self.0.borrow_mut().nodes.insert(
            id,
            Node {
                class: "window".into(),
                parent: 1,
                props,
            },
        );
        self.publish(vec![
            MirrorChange {
                class: "window".into(),
                ..change(MirrorChangeKind::Created, id)
            },
            MirrorChange {
                parent: 1,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            },
            set_change(id, "title", MirrorValue::Text(title.into())),
            set_change(id, "app_id", MirrorValue::Text("app".into())),
        ]);
    }

    /// The compositor destroys `node` on its own, as it does a window whose client closed.
    fn destroy(&self, node: u64) {
        self.0.borrow_mut().nodes.remove(&node);
        self.publish(vec![MirrorChange {
            removed: vec![node],
            ..change(MirrorChangeKind::Destroyed, node)
        }]);
    }
}

impl TreeLink for FakeLink {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        let mut c = self.0.borrow_mut();
        let attached = Attached {
            epoch: 1,
            root: 1,
            serial: c.serial,
        };
        let nodes = c
            .nodes
            .iter()
            .map(|(id, node)| SnapshotNode {
                id: *id,
                parent: node.parent,
                index: 0,
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
        let mut c = self.0.borrow_mut();
        c.sent.push(tx.ops.clone());
        let mut changes = Vec::new();
        for op in &tx.ops {
            if let TxOp::Set { node, key, value, .. } = op {
                if let Some(entry) = c.nodes.get_mut(node) {
                    entry.props.insert(key.clone(), value.clone());
                }
                changes.push(set_change(*node, key, value.clone()));
            }
        }
        c.serial = tx.serial;
        c.queue.push(TreeIncoming::Changes {
            serial: tx.serial,
            changes,
        });
        c.queue.push(TreeIncoming::Reply(TxReply {
            serial: tx.serial,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: Vec::new(),
        }));
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.0.borrow_mut().queue))
    }
}

/// A root and a primary screen of 1000 by 800 (and a window `3` at 10,20 sized 300x200 when `boot_window`).
fn compositor(boot_window: bool) -> Comp {
    let mut c = Compositor::default();
    let node = |class: &str, parent: u64, props: Vec<(&str, MirrorValue)>| Node {
        class: class.into(),
        parent,
        props: props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    };
    c.nodes.insert(1, node("node", 0, vec![]));
    c.nodes.insert(
        2,
        node(
            "screen",
            1,
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        ),
    );
    if boot_window {
        c.nodes.insert(
            3,
            node(
                "window",
                1,
                vec![
                    ("title", MirrorValue::Text("term".into())),
                    ("x", MirrorValue::Double(10.0)),
                    ("y", MirrorValue::Double(20.0)),
                    ("width", MirrorValue::Double(300.0)),
                    ("height", MirrorValue::Double(200.0)),
                ],
            ),
        );
    }
    c.serial = 1;
    Rc::new(RefCell::new(c))
}

fn prop(comp: &Comp, node: u64, key: &str) -> f64 {
    match comp.borrow().nodes.get(&node).and_then(|n| n.props.get(key)) {
        Some(MirrorValue::Double(value)) => *value,
        _ => 0.0,
    }
}

/// `x,y,width,height` of a window as the compositor holds it.
fn rect(comp: &Comp, node: u64) -> String {
    ["x", "y", "width", "height"]
        .iter()
        .map(|key| prop(comp, node, key).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn sent(comp: &Comp) -> Vec<Vec<TxOp>> {
    comp.borrow().sent.clone()
}

/// A booted shell with its entry module loaded.
struct Rig {
    shell: JsShell,
    _dir: TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// Frames until the compositor's echo of the last transaction has been taken in.
    fn settle(&mut self) {
        for _ in 0..3 {
            let diagnostics = self.frame();
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        }
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }
}

fn rig(comp: &Comp, init: &str) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), init).expect("init.ts is written");
    let mut shell = JsShell::new(Box::new(FakeLink(comp.clone())), JsShellConfig::new(&root)).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Rig { shell, _dir: dir }
}

const PRELUDE: &str = "import { Workspace, Layout, FloatingLayout, MasterStackLayout, GridLayout, CollectionListener, \
                       defineConfig } from \"gnoblin\";\n\
                       const g = globalThis as any;\n\
                       g.g = g;\n\
                       g.computes = 0;\n";

fn init(body: &str) -> String {
    format!("{PRELUDE}{body}\n")
}

#[test]
fn workspaces_are_a_mutable_collection_of_objects_a_script_makes() {
    let comp = compositor(false);
    let mut rig = rig(
        &comp,
        &init(
            "const dev = new Workspace({ id: 'dev', name: 'Development', layout: new MasterStackLayout() });\n\
             g.dev = dev;\n\
             gnoblin.workspaces.add(dev);\n\
             gnoblin.workspaces.add(new Workspace({ id: 'web', layout: 'grid' }));\n",
        ),
    );
    assert_eq!(rig.eval("gnoblin.workspaces.length"), "2");
    assert_eq!(rig.eval("gnoblin.workspaces.get('dev') === dev"), "true");
    assert_eq!(
        rig.eval("dev.name + ',' + gnoblin.workspaces.get('web').name"),
        "Development,web"
    );
    assert_eq!(
        rig.eval("[...gnoblin.workspaces].map((w) => w.id + ':' + w.layout.constructor.name + ':' + w.index).join()"),
        "dev:MasterStackLayout:0,web:GridLayout:1"
    );
    assert_eq!(rig.eval("gnoblin.workspaces.find((w) => w.name === 'web').id"), "web");
    assert_eq!(rig.eval("gnoblin.workspaces.where({ active: true }).at(0).id"), "dev");
    assert_eq!(
        rig.eval("typeof gnoblin.clients.add + typeof gnoblin.clients.where({}).add"),
        "undefinedundefined"
    );
    assert_eq!(
        rig.eval("try { gnoblin.workspaces.add(new dev.constructor({ id: 'dev' })); 'no' } catch (e) { e.code }"),
        "duplicate"
    );
    assert_eq!(
        rig.eval("try { new dev.constructor({ id: '' }); 'no' } catch (e) { e.name + ': ' + e.message }"),
        "TypeError: Workspace options.id must be a non-empty string"
    );
    assert_eq!(
        rig.eval("gnoblin.workspaces.remove(dev); gnoblin.workspaces.length"),
        "1"
    );
    assert_eq!(
        rig.eval("gnoblin.workspaces.at(0).active"),
        "true",
        "the active one moved to the first left"
    );
    assert_eq!(rig.eval("dev.index"), "-1");
    assert_eq!(
        rig.eval("gnoblin.workspaces.remove('web'); gnoblin.workspaces.length"),
        "0"
    );
}

#[test]
fn a_config_with_no_workspace_gets_one_floating_desktop_and_define_config_makes_real_ones() {
    let comp = compositor(false);
    let mut bare = rig(&comp, &init("export {};"));
    assert_eq!(
        bare.eval("gnoblin.workspaces.map((w) => w.id + ':' + w.layout.constructor.name + ':' + w.active).join()"),
        "desktop:FloatingLayout:true"
    );
    drop(bare);
    comp.borrow_mut().queue.clear();
    // `defineConfig` layers the stock keys in and binds them on the first seat, so this rig has a seat.
    comp.borrow_mut().nodes.insert(
        100,
        Node {
            class: "seat".into(),
            parent: 1,
            props: BTreeMap::from([
                ("name".to_owned(), MirrorValue::Text("seat0".into())),
                ("pointer_x".to_owned(), MirrorValue::Double(0.0)),
                ("pointer_y".to_owned(), MirrorValue::Double(0.0)),
                ("device".to_owned(), MirrorValue::Texts(Vec::new())),
            ]),
        },
    );
    let mut declared = rig(
        &comp,
        &init(
            "export default defineConfig({\n\
             \x20   gaps: { inner: 4 },\n\
             \x20   workspaces: [{ id: 'a', layout: 'master-stack' }, { id: 'b', layout: new GridLayout() }],\n\
             });\n",
        ),
    );
    assert_eq!(
        declared.eval("gnoblin.workspaces.map((w) => w.id + ':' + w.layout.constructor.name + ':' + w.active).join()"),
        "a:MasterStackLayout:true,b:GridLayout:false"
    );
    assert_eq!(
        declared.eval("JSON.stringify(gnoblin.workspaces.get('b').gaps)"),
        r#"{"inner":4,"outer":0}"#
    );
    assert_eq!(declared.eval("gnoblin.workspaces.get('a') instanceof Object"), "true");
}

#[test]
fn a_workspaces_clients_are_the_same_client_objects_and_adding_one_transfers_it() {
    let comp = compositor(true);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init(
            "const a = new Workspace({ id: 'a' });\n\
             const b = new Workspace({ id: 'b' });\n\
             g.a = a; g.b = b; g.log = [];\n\
             gnoblin.workspaces.add(a);\n\
             gnoblin.workspaces.add(b);\n\
             b.clients.addListener(new (class extends CollectionListener {\n\
             \x20   onAdded(c: any) { g.log.push('b+' + c.id); }\n\
             \x20   onRemoved(c: any) { g.log.push('b-' + c.id); }\n\
             })());\n",
        ),
    );
    rig.frame();
    // The window that was there at boot joined the shown workspace, and the second one does too.
    link.create_window(4, "web");
    rig.frame();
    assert_eq!(rig.eval("a.clients.length + ',' + b.clients.length"), "2,0");
    assert_eq!(rig.eval("a.clients.get('3') === gnoblin.clients.get('3')"), "true");
    assert_eq!(
        rig.eval("[...a.clients].every((c, i) => c === gnoblin.clients.at(i))"),
        "true"
    );
    assert_eq!(rig.eval("gnoblin.clients.get('4').workspace === a"), "true");

    assert_eq!(
        rig.eval("b.clients.add(gnoblin.clients.get('4')); gnoblin.clients.get('4').workspace === b"),
        "true"
    );
    assert_eq!(rig.eval("a.clients.length + ',' + b.clients.length"), "1,1");
    assert_eq!(rig.eval("String(a.clients.get('4'))"), "undefined");
    assert_eq!(rig.eval("b.clients.get('4') === gnoblin.clients.get('4')"), "true");
    rig.frame();
    assert_eq!(
        rig.eval("g.log.join()"),
        "b+4",
        "the collection listener hears the transfer"
    );

    // Removing the workspace moves its windows to the first one left; nothing is closed.
    assert_eq!(
        rig.eval("gnoblin.workspaces.remove(b); gnoblin.clients.get('4').workspace === a"),
        "true"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.get('4').alive + ',' + gnoblin.clients.length"),
        "true,2"
    );
    // Taking a window out of a workspace leaves it alive and workspace-less.
    assert_eq!(
        rig.eval("a.clients.remove('3'); gnoblin.clients.get('3').workspace"),
        "null"
    );
    assert_eq!(rig.eval("gnoblin.clients.get('3').alive"), "true");
    // A window the compositor destroys leaves its workspace.
    link.destroy(4);
    rig.frame();
    assert_eq!(rig.eval("a.clients.length + ',' + gnoblin.clients.length"), "0,1");
    assert_eq!(rig.eval("gnoblin.clients.get('3').alive"), "true");
}

#[test]
fn a_workspace_can_be_subclassed_and_listened_to_through_its_clients() {
    let comp = compositor(true);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init(
            "g.log = [];\n\
             class Dev extends Workspace {\n\
             \x20   constructor() {\n\
             \x20       super({ id: 'dev', layout: new MasterStackLayout() });\n\
             \x20       this.clients.addListener(new (class extends CollectionListener {\n\
             \x20           onAdded(c: any) { g.log.push('added ' + c.id); }\n\
             \x20           onClosed(c: any) { g.log.push('closed ' + c.id); }\n\
             \x20       })());\n\
             \x20   }\n\
             }\n\
             g.Dev = Dev;\n\
             gnoblin.workspaces.add(new Dev());\n",
        ),
    );
    rig.frame();
    assert_eq!(rig.eval("gnoblin.workspaces.at(0) instanceof g.Dev"), "true");
    assert_eq!(
        rig.eval("g.log.join()"),
        "added 3",
        "the window that was there at boot joined and was announced"
    );
    link.create_window(4, "two");
    rig.frame();
    assert_eq!(rig.eval("g.log.join()"), "added 3,added 4");
    assert_eq!(rect(&comp, 3), "0,0,500,800");
    link.destroy(3);
    rig.settle();
    assert_eq!(rig.eval("g.log.join()"), "added 3,added 4,closed 3");
    assert_eq!(
        rect(&comp, 4),
        "0,0,1000,800",
        "the remaining window takes the whole area"
    );
}

#[test]
fn a_new_window_joins_the_shown_workspace_and_show_switches_it() {
    let comp = compositor(false);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init(
            "g.a = new Workspace({ id: 'a' });\n\
             g.b = new Workspace({ id: 'b' });\n\
             gnoblin.workspaces.add(g.a);\n\
             gnoblin.workspaces.add(g.b);\n",
        ),
    );
    rig.frame();
    link.create_window(4, "one");
    rig.frame();
    assert_eq!(rig.eval("gnoblin.clients.get('4').workspace.id"), "a");
    assert_eq!(rig.eval("b.show(); a.active + ',' + b.active"), "false,true");
    link.create_window(5, "two");
    rig.frame();
    assert_eq!(rig.eval("gnoblin.clients.get('5').workspace.id"), "b");
    assert_eq!(
        rig.eval("gnoblin.show('a'); gnoblin.workspaces.show(b); b.active"),
        "true"
    );
    assert_eq!(
        rig.eval("try { gnoblin.workspaces.show('nowhere'); 'no' } catch (e) { e.code }"),
        "unknown-workspace"
    );
}

const COUNTING: &str = "class Counting extends MasterStackLayout {\n\
                        \x20   compute(context: any) { g.computes++; g.lastContext = context; return super.compute(context); }\n\
                        }\n";

#[test]
fn a_dirty_workspace_computes_once_per_frame_and_its_writes_are_one_transaction() {
    let comp = compositor(false);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init(&format!(
            "{COUNTING}g.tiles = new Workspace({{ id: 'tiles', layout: new Counting() }});\n\
             gnoblin.workspaces.add(g.tiles);\n"
        )),
    );
    rig.settle();
    assert!(sent(&comp).is_empty(), "no window, nothing to write");
    let base = rig.eval("g.computes");

    // Two windows and a field edit in one frame: one compute, one transaction of eight writes.
    link.create_window(3, "one");
    link.create_window(4, "two");
    rig.frame();
    assert_eq!(
        rig.eval("g.computes"),
        (base.parse::<u32>().expect("a number") + 1).to_string()
    );
    let txs = sent(&comp);
    assert_eq!(txs.len(), 1, "one transaction");
    assert_eq!(
        txs[0].len(),
        7,
        "the values that differ: width and height of one, x, width, height of the other, and `tiled` on both"
    );
    assert_eq!(rect(&comp, 3), "0,0,500,800");
    assert_eq!(rect(&comp, 4), "500,0,500,800");
    assert_eq!(
        rig.eval(
            "JSON.stringify(g.lastContext.area) + JSON.stringify(g.lastContext.gap) + g.lastContext.children.length"
        ),
        r#"{"x":0,"y":0,"width":1000,"height":800}{"inner":0,"outer":0}2"#
    );
    rig.settle();
    let computed = rig.eval("g.computes");
    rig.frame();
    rig.frame();
    assert_eq!(
        rig.eval("g.computes"),
        computed,
        "an unchanged workspace is not laid out again"
    );

    // A handler's repeated writes and the layout's are one write per property, the last.
    rig.eval(
        "const c = gnoblin.clients.get('3'); c.x = 5; c.x = 6; c.width = 7;\n\
         g.tiles.layout.ratio = 0.25;",
    );
    let before = sent(&comp).len();
    rig.frame();
    let txs = sent(&comp);
    assert_eq!(txs.len(), before + 1);
    let x_writes: Vec<&TxOp> = txs[before]
        .iter()
        .filter(|op| matches!(op, TxOp::Set { node: 3, key, .. } if key == "x"))
        .collect();
    assert_eq!(x_writes.len(), 1, "one x write for node 3: {x_writes:?}");
    assert_eq!(rect(&comp, 3), "0,0,250,800", "the layout's value is the last write");
    assert_eq!(rect(&comp, 4), "250,0,750,800");
}

#[test]
fn the_area_follows_the_screen_and_a_screen_less_tree_lays_nothing_out() {
    let comp = compositor(false);
    let link = FakeLink(comp.clone());
    let mut tiled = rig(
        &comp,
        &init(&format!(
            "{COUNTING}gnoblin.workspaces.add(new Workspace({{ id: 'tiles', layout: new Counting() }}));\n"
        )),
    );
    link.create_window(3, "one");
    tiled.settle();
    assert_eq!(rect(&comp, 3), "0,0,1000,800");
    let before = tiled.eval("g.computes");
    link.external_set(2, "width", MirrorValue::Double(500.0));
    link.external_set(2, "height", MirrorValue::Double(300.0));
    tiled.settle();
    assert_eq!(rect(&comp, 3), "0,0,500,300", "the new area is laid out");
    assert_eq!(
        tiled.eval("g.computes"),
        (before.parse::<u32>().expect("a number") + 1).to_string(),
        "once for the two changes"
    );

    let bare = Rc::new(RefCell::new(Compositor::default()));
    {
        let mut c = bare.borrow_mut();
        c.nodes.insert(
            1,
            Node {
                class: "node".into(),
                parent: 0,
                props: BTreeMap::new(),
            },
        );
        c.serial = 1;
    }
    let mut rig = rig(
        &bare,
        &init(&format!(
            "{COUNTING}gnoblin.workspaces.add(new Workspace({{ id: 'tiles', layout: new Counting() }}));\n"
        )),
    );
    rig.settle();
    assert_eq!(
        rig.eval("g.computes"),
        "0",
        "without a screen there is no area to lay out in"
    );
}

#[test]
fn a_layouts_own_fields_are_diffed_and_private_fields_are_not_seen() {
    let comp = compositor(true);
    let mut rig = rig(
        &comp,
        &init(
            "class R extends Layout {\n\
             \x20   #hidden = 0;\n\
             \x20   constructor() { super(); (this as any).radius = 9; }\n\
             \x20   bump() { this.#hidden++; return this.#hidden; }\n\
             \x20   compute(context: any) {\n\
             \x20       g.computes++;\n\
             \x20       g.gap = context.gap;\n\
             \x20       return context.children.map((node: any, i: number) => ({\n\
             \x20           node,\n\
             \x20           bounds: { x: (this as any).radius * i, y: this.#hidden, width: 100, height: 100 },\n\
             \x20       }));\n\
             \x20   }\n\
             }\n\
             g.R = R;\n\
             g.ws = new Workspace({ id: 'r', layout: new R() });\n\
             gnoblin.workspaces.add(g.ws);\n",
        ),
    );
    rig.settle();
    assert_eq!(rig.eval("g.computes"), "1");
    assert_eq!(rect(&comp, 3), "0,0,100,100");
    rig.frame();
    assert_eq!(rig.eval("g.computes"), "1", "nothing changed");

    rig.eval("g.ws.layout.radius = 12");
    rig.settle();
    assert_eq!(
        rig.eval("g.computes"),
        "2",
        "a changed own field marks the workspace dirty"
    );
    assert_eq!(rig.eval("g.ws.layout.bump()"), "1");
    rig.settle();
    assert_eq!(
        rig.eval("g.computes"),
        "2",
        "a #private field is not an own enumerable field and is not seen"
    );
    assert_eq!(
        rect(&comp, 3),
        "0,0,100,100",
        "so the old placement stands until something else changes"
    );

    rig.eval("g.ws.gaps.inner = 8");
    rig.settle();
    assert_eq!(rig.eval("g.computes"), "3", "a gap change is a change");
    assert_eq!(rig.eval("JSON.stringify(g.gap)"), r#"{"inner":8,"outer":0}"#);
    assert_eq!(
        rect(&comp, 3),
        "0,1,100,100",
        "and the private field is read by the compute that follows"
    );

    rig.eval("g.ws.layout = new g.R()");
    rig.settle();
    assert_eq!(rig.eval("g.computes"), "4", "a new layout instance is laid out");
    rig.eval("g.ws.layout = 'monocle'");
    rig.settle();
    assert_eq!(rect(&comp, 3), "0,0,1000,800", "a stock layout by name");
}

#[test]
fn a_workspace_that_is_not_shown_is_laid_out_when_it_is() {
    let comp = compositor(true);
    let mut rig = rig(
        &comp,
        &init(&format!(
            "{COUNTING}g.a = new Workspace({{ id: 'a' }});\n\
             g.b = new Workspace({{ id: 'b', layout: new Counting() }});\n\
             gnoblin.workspaces.add(g.a);\n\
             gnoblin.workspaces.add(g.b);\n"
        )),
    );
    rig.settle();
    assert_eq!(rig.eval("g.computes"), "0", "b is not shown");
    rig.eval("g.b.clients.add(gnoblin.clients.get('3'))");
    rig.settle();
    assert_eq!(rig.eval("g.computes"), "0", "still not shown");
    assert_eq!(rect(&comp, 3), "10,20,300,200");
    rig.eval("g.b.show()");
    rig.settle();
    assert_eq!(rig.eval("g.computes"), "1");
    assert_eq!(rect(&comp, 3), "0,0,1000,800");
}

#[test]
fn floating_places_a_window_on_arrival_only_and_leaves_a_moved_window_alone() {
    let comp = compositor(true);
    let link = FakeLink(comp.clone());
    {
        let mut c = comp.borrow_mut();
        let window = c.nodes.get_mut(&3).expect("the boot window");
        for (key, value) in [("x", 900.0), ("y", 700.0), ("width", 50.0), ("height", 50.0)] {
            window.props.insert(key.into(), MirrorValue::Double(value));
        }
    }
    let mut rig = rig(
        &comp,
        &init("gnoblin.workspaces.add(new Workspace({ id: 'desktop' }));\n"),
    );
    rig.settle();
    assert!(
        sent(&comp).is_empty(),
        "the window that was there at boot is not placed again"
    );
    assert_eq!(rect(&comp, 3), "900,700,50,50");

    link.create_window(4, "new");
    rig.settle();
    assert_eq!(
        rect(&comp, 4),
        "0,0,800,600",
        "a new window is placed, at the default size until it has one"
    );
    let placed = sent(&comp).len();

    // The user moves it; the layout does not pull it back, nor does it touch the other window.
    link.external_set(4, "x", MirrorValue::Double(300.0));
    link.external_set(4, "y", MirrorValue::Double(40.0));
    link.create_window(5, "third");
    rig.settle();
    assert_eq!(rect(&comp, 4), "300,40,800,600");
    assert_eq!(rect(&comp, 3), "900,700,50,50");
    let txs = sent(&comp);
    assert_eq!(txs.len(), placed + 1, "only the third window was placed");
    assert!(
        txs[placed].iter().all(|op| matches!(op, TxOp::Set { node: 5, .. })),
        "{:?}",
        txs[placed]
    );
}

/// A shell boots, the entry declares `desktop` and `dev`, window 3 goes to `dev`: the first generation.
fn first_generation(comp: &Comp) -> gnoblin_js::JsWorkspaceSnapshot {
    let mut rig = rig(
        comp,
        &init(
            "gnoblin.workspaces.add(new Workspace({ id: 'desktop' }));\n\
             gnoblin.workspaces.add(new Workspace({ id: 'dev', layout: new MasterStackLayout() }));\n",
        ),
    );
    FakeLink(comp.clone()).create_window(4, "web");
    rig.settle();
    assert_eq!(
        rig.eval("gnoblin.clients.get('3').workspace.id + gnoblin.clients.get('4').workspace.id"),
        "desktopdesktop"
    );
    rig.eval("gnoblin.workspaces.get('dev').clients.add(gnoblin.clients.get('3'))");
    rig.settle();
    rig.shell.workspace_snapshot().expect("a snapshot")
}

#[test]
fn a_reload_keeps_the_windows_on_the_workspace_of_the_same_id() {
    let comp = compositor(true);
    let snapshot = first_generation(&comp);
    assert_eq!(
        snapshot,
        gnoblin_js::JsWorkspaceSnapshot {
            active: Some("desktop".into()),
            workspaces: vec![
                gnoblin_js::JsWorkspaceEntry {
                    id: "desktop".into(),
                    clients: vec![4],
                },
                gnoblin_js::JsWorkspaceEntry {
                    id: "dev".into(),
                    clients: vec![3],
                },
            ],
        }
    );
    // The first shell is gone, as a restarted runtime's connection is: its undelivered echoes go with it.
    comp.borrow_mut().queue.clear();
    let before = sent(&comp).len();
    let mut second = rig(
        &comp,
        &init(
            "gnoblin.workspaces.add(new Workspace({ id: 'desktop' }));\n\
             gnoblin.workspaces.add(new Workspace({ id: 'dev', layout: new MasterStackLayout() }));\n",
        ),
    );
    second.shell.reconcile(&snapshot).expect("reconciled");
    second.settle();
    assert_eq!(second.eval("gnoblin.workspaces.length"), "2", "no second desktop");
    assert_eq!(second.eval("gnoblin.clients.get('3').workspace.id"), "dev");
    assert_eq!(second.eval("gnoblin.clients.get('4').workspace.id"), "desktop");
    assert_eq!(second.eval("gnoblin.workspaces.get('dev').clients.length"), "1");
    assert_eq!(second.eval("gnoblin.workspaces.get('desktop').active"), "true");
    assert_eq!(
        sent(&comp).len(),
        before,
        "nothing is placed again, so nothing is written"
    );
}

#[test]
fn a_workspace_missing_from_the_new_config_drops_and_its_windows_go_to_the_first_one() {
    let comp = compositor(true);
    let snapshot = first_generation(&comp);
    let four = rect(&comp, 4);
    comp.borrow_mut().queue.clear();
    let mut second = rig(
        &comp,
        &init("gnoblin.workspaces.add(new Workspace({ id: 'desktop' }));\n"),
    );
    second.shell.reconcile(&snapshot).expect("reconciled");
    second.settle();
    assert_eq!(second.eval("gnoblin.workspaces.length"), "1");
    assert_eq!(second.eval("String(gnoblin.workspaces.get('dev'))"), "undefined");
    assert_eq!(
        second.eval("gnoblin.clients.map((c) => c.workspace.id).join()"),
        "desktop,desktop"
    );
    assert_eq!(second.eval("gnoblin.workspaces.get('desktop').clients.length"), "2");
    // The window that moved arrives at a floating workspace and is placed there again; the other stays.
    assert_ne!(rect(&comp, 3), "10,20,300,200");
    assert_eq!(rect(&comp, 4), four);
}

/// Runs `body` with the stock library imported, for helpers that need no host.
fn pure(body: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let source = format!(
        "import {{ checkPlacements, checkWorkspaceOptions, fieldsChanged, initialWorkspace, layoutFields }} \
         from \"gnoblin/workspace\";\n\
         import {{ Layout, MasterStackLayout }} from \"gnoblin\";\n\
         const emit = (value: unknown) => {{ (globalThis as any).out = String(value); }};\n{body}\n"
    );
    fs::write(root.join("init.ts"), source).expect("init.ts is written");
    let mut config = JsConfig::new(&root);
    for (name, source) in stock_modules() {
        config = config.embed(name, source);
    }
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    generation.eval_module(Path::new("init.ts")).expect("init.ts evaluates");
    generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.globals().get::<_, String>("out"))
        .expect("the case called emit")
}

#[test]
fn the_pure_helpers_diff_fields_pick_the_initial_workspace_and_check_placements() {
    let got = pure(
        r#"
        const a = new MasterStackLayout();
        const before = layoutFields(a);
        const same = !fieldsChanged(before, layoutFields(a));
        a.ratio = 0.3;
        const edited = fieldsChanged(before, layoutFields(a));
        (a as any).extra = 1;
        const added = fieldsChanged(before, layoutFields(a));
        const thrown = (fn: () => void) => { try { fn(); return "ok"; } catch (e: any) { return e.name + ": " + e.message; } };
        const nodes = [{}, {}];
        emit([
            same, edited, added, fieldsChanged(null, before),
            initialWorkspace([{ active: false, n: 1 }, { active: true, n: 2 }])!.n,
            initialWorkspace([{ active: false, n: 1 }, { active: false, n: 2 }])!.n,
            String(initialWorkspace([])),
            JSON.stringify(checkPlacements([{ node: nodes[1], bounds: { x: 1, y: 2, width: 3, height: 4 } }, { node: nodes[0] }], nodes, "w")),
            thrown(() => checkPlacements({}, nodes, "w")),
            thrown(() => checkPlacements([{ node: {} }], nodes, "w")),
            thrown(() => checkPlacements([{ node: nodes[0], bounds: { x: NaN, y: 0, width: 1, height: 1 } }], nodes, "w")),
            thrown(() => checkPlacements([{ node: nodes[0], bounds: { x: 0, y: 0, width: -1, height: 1 } }], nodes, "w")),
            JSON.stringify(checkWorkspaceOptions({ id: "x", gaps: { inner: 2 } }).gaps),
            thrown(() => checkWorkspaceOptions({ id: "x", color: 1 } as any)),
        ].join("\n"));
        "#,
    );
    assert_eq!(
        got,
        [
            "true",
            "true",
            "true",
            "true",
            "2",
            "1",
            "null",
            r#"[{"node":{},"bounds":{"x":1,"y":2,"width":3,"height":4},"visible":true,"tiled":false,"z":0,"rotation":[0,0,0],"scale":[1,1,1]},{"node":{},"bounds":null,"visible":true,"tiled":false,"z":0,"rotation":[0,0,0],"scale":[1,1,1]}]"#,
            "TypeError: w: compute must return an array of placements, got [object Object]",
            "TypeError: w: placement 0 is not for one of the workspace's clients",
            "TypeError: w: placement 0 bounds.x must be a finite number",
            "TypeError: w: placement 0 bounds.width must be a finite number of at least 0",
            r#"{"inner":2,"outer":0}"#,
            "TypeError: Workspace options has no \"color\"; expected one of id, name, layout, gaps",
        ]
        .join("\n")
    );
}

/// The kind of a property the compositor holds as a flag.
fn flag(comp: &Comp, node: u64, key: &str) -> Option<bool> {
    match comp.borrow().nodes.get(&node).and_then(|n| n.props.get(key)) {
        Some(MirrorValue::Bool(value)) => Some(*value),
        _ => None,
    }
}

/// The three numbers of a list property as the compositor holds them.
fn list(comp: &Comp, node: u64, key: &str) -> Option<Vec<f64>> {
    match comp.borrow().nodes.get(&node).and_then(|n| n.props.get(key)) {
        Some(MirrorValue::List(value)) => Some(value.clone()),
        _ => None,
    }
}

/// A shell with `layout` on the one workspace `w` and the windows 10, 11 and 12 mapped.
fn three_windows(layout: &str) -> (Comp, FakeLink, Rig) {
    let comp = compositor(false);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init(&format!(
            "gnoblin.workspaces.add(new Workspace({{ id: 'w', layout: '{layout}' }}));\n"
        )),
    );
    rig.settle();
    for (id, title) in [(10, "a"), (11, "b"), (12, "c")] {
        link.create_window(id, title);
    }
    rig.settle();
    (comp, link, rig)
}

#[test]
fn a_dwindle_workspace_places_its_windows_in_the_split_tree_and_marks_them_tiled() {
    // tiling_test.lua "Dwindle: 2 and 3 windows split the last cell by aspect" through the layout driver: the window
    // that maps second splits the first's cell; the third splits the second's, top and bottom.
    let comp = compositor(false);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init("gnoblin.workspaces.add(new Workspace({ id: 'd', layout: 'dwindle' }));\n"),
    );
    rig.settle();
    link.create_window(10, "a");
    rig.settle();
    assert_eq!(rect(&comp, 10), "0,0,1000,800");
    link.create_window(11, "b");
    rig.settle();
    assert_eq!(
        (rect(&comp, 10), rect(&comp, 11)),
        ("0,0,500,800".into(), "500,0,500,800".into())
    );
    link.create_window(12, "c");
    rig.settle();
    assert_eq!(rect(&comp, 12), "500,400,500,400");
    assert_eq!(rect(&comp, 11), "500,0,500,400");
    for id in [10, 11, 12] {
        assert_eq!(flag(&comp, id, "tiled"), Some(true), "window {id} is tiled");
    }
    // The window that closes gives its cell to its sibling.
    link.destroy(11);
    rig.settle();
    assert_eq!(rect(&comp, 12), "500,0,500,800");
}

#[test]
fn a_kiosk_workspace_shows_the_focused_window_and_hides_the_rest_as_focus_moves() {
    // tiling_test.lua "Kiosk" and "Monocle: refocusing shows another window and hides the rest": a change of focus is a
    // reason to lay out again, and the placement's `visible` reaches the window.
    let (comp, link, mut rig) = three_windows("kiosk");
    for id in [10, 11, 12] {
        assert_eq!(rect(&comp, id), "0,0,1000,800");
    }
    assert_eq!(
        [10, 11, 12].map(|id| flag(&comp, id, "visible").unwrap_or(true)),
        [true, false, false],
        "with no focus the first window is shown"
    );
    link.external_set(11, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(
        [10, 11, 12].map(|id| flag(&comp, id, "visible").unwrap_or(true)),
        [false, true, false]
    );
    link.external_set(11, "focused", MirrorValue::Bool(false));
    link.external_set(12, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(
        [10, 11, 12].map(|id| flag(&comp, id, "visible").unwrap_or(true)),
        [false, false, true]
    );
}

#[test]
fn a_strip_workspace_scrolls_to_keep_the_focused_column_in_view_as_focus_moves() {
    // strip_test.lua "a strip lays out one column per window and scrolls to keep the focused one in view": columns are
    // half the strip wide; the focused third column is brought in (x = 500) and the first when focus goes back.
    let (comp, link, mut rig) = three_windows("strip");
    assert_eq!(
        [10, 11, 12].map(|id| rect(&comp, id)),
        ["0,0,500,800", "500,0,500,800", "1000,0,500,800"],
        "with no focus the view stays at the start"
    );
    link.external_set(12, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(
        [10, 11, 12].map(|id| rect(&comp, id)),
        ["-500,0,500,800", "0,0,500,800", "500,0,500,800"]
    );
    link.external_set(12, "focused", MirrorValue::Bool(false));
    link.external_set(10, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(rect(&comp, 10), "0,0,500,800");
    for id in [10, 11, 12] {
        assert!(!flag(&comp, id, "tiled").unwrap_or(false), "a strip does not tile");
    }
}

#[test]
fn a_carousel_workspace_writes_the_depth_and_rotation_of_its_cards_and_a_layout_change_clears_them() {
    let (comp, link, mut rig) = three_windows("carousel-3d");
    link.external_set(10, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(
        list(&comp, 10, "rotation"),
        None,
        "the front card has the default rotation"
    );
    let side = list(&comp, 11, "rotation").expect("the next card is turned");
    assert_eq!((side[0], side[2]), (0.0, 0.0));
    assert!(
        (side[1] - std::f64::consts::TAU / 3.0).abs() < 1e-9,
        "a third of the ring: {side:?}"
    );
    assert!(
        prop(&comp, 11, "z") < prop(&comp, 10, "z"),
        "the side card is further back than the front one"
    );
    assert_eq!(rig.eval("gnoblin.clients.get('11').rotation[1] > 0"), "true");
    // Another layout puts every card back.
    rig.eval("gnoblin.workspaces.get('w').layout = 'floating'");
    rig.settle();
    for id in [10, 11, 12] {
        assert_eq!(prop(&comp, id, "z"), 0.0);
    }
    assert_eq!(rig.eval("gnoblin.clients.get('11').rotation.join()"), "0,0,0");
    assert_eq!(rig.eval("gnoblin.clients.get('11').scale.join()"), "1,1,1");
}

#[test]
fn showing_a_workspace_hides_the_windows_of_the_one_it_replaces() {
    let comp = compositor(false);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init(
            "gnoblin.workspaces.add(new Workspace({ id: 'a' }));\n\
             gnoblin.workspaces.add(new Workspace({ id: 'b' }));\n",
        ),
    );
    rig.settle();
    link.create_window(10, "a");
    link.create_window(11, "b");
    rig.settle();
    rig.eval("gnoblin.workspaces.get('b').clients.add(gnoblin.clients.get('11'))");
    rig.settle();
    let visible = |comp: &Comp| [10, 11].map(|id| flag(comp, id, "visible").unwrap_or(true));
    assert_eq!(
        visible(&comp),
        [true, false],
        "only the shown workspace's windows are drawn"
    );
    rig.eval("gnoblin.workspaces.show('b')");
    rig.settle();
    assert_eq!(visible(&comp), [false, true]);
    rig.eval("gnoblin.workspaces.get('a').show()");
    rig.settle();
    assert_eq!(visible(&comp), [true, false]);
}

#[test]
fn a_slide_animates_both_workspaces_with_the_lua_curve_and_hides_the_old_one_when_it_ends() {
    // Desk:show: the new workspace comes in from a screen's width away and the old one leaves the other way, over
    // 0.25 seconds on `ease.out_cubic` sampled 33 times; the old one is hidden when that is over.
    let comp = compositor(false);
    let link = FakeLink(comp.clone());
    let mut rig = rig(
        &comp,
        &init(
            "gnoblin.workspaces.add(new Workspace({ id: 'a' }));\n\
             gnoblin.workspaces.add(new Workspace({ id: 'b' }));\n",
        ),
    );
    rig.settle();
    link.create_window(10, "a");
    link.create_window(11, "b");
    rig.settle();
    rig.eval("gnoblin.workspaces.get('b').clients.add(gnoblin.clients.get('11'))");
    rig.settle();
    let (rest_a, rest_b) = (prop(&comp, 10, "x"), prop(&comp, 11, "x"));
    let before = sent(&comp).len();
    rig.eval("gnoblin.workspaces.show('b', { duration: 0.3 })");
    rig.settle();
    let animated: Vec<(u64, f64, f64, usize, f64, f64)> = sent(&comp)[before..]
        .iter()
        .flatten()
        .filter_map(|op| match op {
            TxOp::Set {
                node,
                key,
                value: MirrorValue::Double(to),
                seconds: Some(seconds),
                samples: Some(samples),
            } if key == "x" => Some((
                *node,
                *to,
                *seconds,
                samples.len(),
                samples[0],
                samples[samples.len() - 1],
            )),
            _ => None,
        })
        .collect();
    // The new workspace's window goes to rest, the old one a screen's width left of it.
    assert!(animated.contains(&(11, rest_b, 0.3, 33, 0.0, 1.0)), "{animated:?}");
    assert!(
        animated.contains(&(10, rest_a - 1000.0, 0.3, 33, 0.0, 1.0)),
        "{animated:?}"
    );
    assert!(
        flag(&comp, 10, "visible").unwrap_or(true),
        "the leaving window is drawn while it slides"
    );
    std::thread::sleep(Duration::from_millis(450));
    rig.settle();
    assert_eq!(
        flag(&comp, 10, "visible"),
        Some(false),
        "and hidden when the slide is over"
    );
    assert_eq!(prop(&comp, 10, "x"), rest_a, "back at its rest place");
    assert!(flag(&comp, 11, "visible").unwrap_or(true));
    // The curve is `1 - (1 - t)^3`: the middle sample is 0.875.
    let middle = sent(&comp)[before..]
        .iter()
        .flatten()
        .find_map(|op| match op {
            TxOp::Set {
                samples: Some(samples), ..
            } => Some(samples[16]),
            _ => None,
        })
        .expect("an animated write");
    assert!((middle - 0.875).abs() < 1e-12, "{middle}");
}

#[test]
fn animate_checks_its_arguments_and_writes_nothing_when_one_is_wrong() {
    let comp = compositor(true);
    let mut rig = rig(&comp, &init("export {};"));
    rig.settle();
    let thrown = |rig: &mut Rig, body: &str| {
        rig.eval(&format!(
            "try {{ {body}; 'no' }} catch (e) {{ e.name + ': ' + e.message }}"
        ))
    };
    let before = sent(&comp).len();
    let c = "gnoblin.clients.get('3')";
    assert_eq!(
        thrown(&mut rig, &format!("{c}.animate({{ x: 1 }}, {{ duration: -1 }})")),
        "TypeError: Client#3.animate(): duration must be a finite number of seconds of at least 0"
    );
    assert_eq!(
        thrown(
            &mut rig,
            &format!("{c}.animate({{ x: 1 }}, {{ duration: 1, easing: 'bouncy' }})")
        ),
        "TypeError: Client#3.animate(): easing must be one of linear, out, out-cubic, in-cubic, in-out"
    );
    assert_eq!(
        thrown(
            &mut rig,
            &format!("{c}.animate({{ visible: false }}, {{ duration: 1 }})")
        ),
        "TypeError: Client#3.animate(): visible cannot be animated"
    );
    assert_eq!(
        thrown(
            &mut rig,
            &format!("{c}.animate({{ x: 1, rotation: [1, 2] }}, {{ duration: 1 }})")
        ),
        "TypeError: Client#3.animate(): rotation must be three finite numbers"
    );
    assert_eq!(
        thrown(&mut rig, &format!("{c}.animate({{ x: NaN }}, {{ duration: 1 }})")),
        "TypeError: Client#3.animate(): x must be a finite number"
    );
    rig.settle();
    assert_eq!(sent(&comp).len(), before, "a refused animation queued nothing");
    // A zero duration is an instant write, and rotation and scale take three numbers.
    rig.eval(&format!(
        "{c}.animate({{ x: 42, rotation: [0, 1, 0], scale: [2, 2, 1] }}, {{ duration: 0 }})"
    ));
    rig.settle();
    assert_eq!(prop(&comp, 3, "x"), 42.0);
    assert_eq!(list(&comp, 3, "rotation"), Some(vec![0.0, 1.0, 0.0]));
    assert_eq!(list(&comp, 3, "scale"), Some(vec![2.0, 2.0, 1.0]));
    assert_eq!(
        thrown(&mut rig, &format!("{c}.rotation = [1, 2]")),
        "TypeError: Client#3.rotation: must be three finite numbers"
    );
}

#[test]
fn after_runs_its_function_once_at_the_first_frame_past_the_time() {
    let comp = compositor(false);
    let mut rig = rig(&comp, &init("g.ran = 0;\ngnoblin.after(0.2, () => { g.ran += 1; });\n"));
    rig.settle();
    assert_eq!(rig.eval("g.ran"), "0", "not before its time");
    std::thread::sleep(Duration::from_millis(300));
    rig.settle();
    assert_eq!(rig.eval("g.ran"), "1");
    rig.settle();
    assert_eq!(rig.eval("g.ran"), "1", "once");
    assert_eq!(
        rig.eval("try { gnoblin.after(-1, () => {}); 'no' } catch (e) { e.message }"),
        "gnoblin.after: seconds must be a finite number of at least 0"
    );
}
