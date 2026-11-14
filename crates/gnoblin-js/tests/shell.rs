//! Tests for `JsShell`: the JavaScript shell over the tree protocol.
//!
//! The compositor is played by `FakeLink`, an in-memory `TreeLink` modelled on the one in
//! `gnoblin-lua`'s `shell_vm` tests: it applies every transaction to a node map, answers it with a change
//! batch carrying the serial and then the reply, refuses a write to a node a published batch changed since
//! the base (`stale-base`) and a write to a node that is gone (`unknown-node`), and can hold a transaction
//! in flight.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_js::{JsRole, JsRoleConfig, JsRoleLinks, JsShell, JsShellConfig, JsShellError};
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
    /// The serial at which each node last changed in a batch the compositor published.
    changed: BTreeMap<u64, u64>,
    /// Transactions the link has received, in order.
    sent: Vec<Vec<TxOp>>,
    /// The next transaction is refused when set.
    refuse_next: bool,
    /// Transactions are recorded in `held` and applied only by `release`, as a real compositor echoes them later.
    hold: bool,
    held: Vec<TreeTx>,
    queue: Vec<TreeIncoming>,
}

#[derive(Clone)]
struct FakeLink(Rc<RefCell<Compositor>>);

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

fn text(value: &str) -> MirrorValue {
    MirrorValue::Text(value.into())
}

impl FakeLink {
    fn snapshot(&self) -> TreeIncoming {
        let c = self.0.borrow();
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
        TreeIncoming::Snapshot {
            serial: c.serial,
            chunk: 0,
            last: true,
            nodes,
        }
    }

    /// Applies `ops` to the node map and returns the change batch describing them.
    fn apply(&self, ops: &[TxOp], serial: u64) -> Vec<MirrorChange> {
        let mut c = self.0.borrow_mut();
        let mut changes = Vec::new();
        for op in ops {
            match op {
                TxOp::Set { node, key, value, .. } => {
                    if let Some(entry) = c.nodes.get_mut(node) {
                        entry.props.insert(key.clone(), value.clone());
                    }
                    changes.push(set_change(*node, key, value.clone()));
                }
                TxOp::Destroy { node } => {
                    c.nodes.remove(node);
                    changes.push(MirrorChange {
                        removed: vec![*node],
                        ..change(MirrorChangeKind::Destroyed, *node)
                    });
                }
                _ => {}
            }
        }
        for change in &changes {
            c.changed.insert(change.node, serial);
        }
        changes
    }

    /// Lands `tx` as the compositor does: refused `unknown-node` at the first op on a node that is gone, then
    /// `stale-base` at the first op on a node a batch published after the base changed, otherwise applied
    /// whole and echoed.
    fn land(&self, tx: &TreeTx) {
        let refusal = {
            let c = self.0.borrow();
            let gone = tx.ops.iter().position(|op| match op {
                TxOp::Set { node, .. } | TxOp::Destroy { node } | TxOp::Call { node, .. } => {
                    !c.nodes.contains_key(node)
                }
                _ => false,
            });
            let stale = tx.ops.iter().position(|op| match op {
                TxOp::Set { node, .. } | TxOp::Destroy { node } if tx.base_serial != c.serial => {
                    c.changed.get(node).is_some_and(|serial| *serial > tx.base_serial)
                }
                _ => false,
            });
            gone.map(|at| ("unknown-node", at, c.serial))
                .or_else(|| stale.map(|at| ("stale-base", at, tx.serial)))
        };
        if let Some((code, failed_op, serial)) = refusal {
            self.0.borrow_mut().queue.push(TreeIncoming::Reply(TxReply {
                serial,
                applied: false,
                failed_op: failed_op as i64,
                code: code.into(),
                removed: Vec::new(),
            }));
            return;
        }
        let changes = self.apply(&tx.ops, tx.serial);
        let mut c = self.0.borrow_mut();
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
    }

    /// Publishes `changes` as a batch of the compositor's own.
    fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        for change in &changes {
            c.changed.insert(change.node, serial);
        }
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }

    /// A native write of `node.key` the shell did not make.
    fn external_set(&self, node: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            entry.props.insert(key.to_owned(), value.clone());
        }
        self.publish(vec![set_change(node, key, value)]);
    }

    /// A window the compositor maps: created, parented under the root and given its title and app id.
    fn create_window(&self, id: u64, title: &str, app_id: &str) {
        self.0.borrow_mut().nodes.insert(
            id,
            Node {
                class: "window".into(),
                parent: 1,
                props: BTreeMap::from([("title".to_owned(), text(title)), ("app_id".to_owned(), text(app_id))]),
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
            set_change(id, "title", text(title)),
            set_change(id, "app_id", text(app_id)),
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

    /// Lands every held transaction in order, as the compositor echoes them.
    fn release(&self) {
        let held = std::mem::take(&mut self.0.borrow_mut().held);
        for tx in &held {
            self.land(tx);
        }
    }
}

impl TreeLink for FakeLink {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        let attached = Attached {
            epoch: 1,
            root: 1,
            serial: self.0.borrow().serial,
        };
        let snapshot = self.snapshot();
        let mut c = self.0.borrow_mut();
        c.queue.push(TreeIncoming::Attached(attached));
        c.queue.push(snapshot);
        Ok(())
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        {
            let mut c = self.0.borrow_mut();
            c.sent.push(tx.ops.clone());
            if c.hold {
                c.held.push(tx.clone());
                return Ok(());
            }
            if std::mem::replace(&mut c.refuse_next, false) {
                c.queue.push(TreeIncoming::Reply(TxReply {
                    serial: tx.serial,
                    applied: false,
                    failed_op: 0,
                    code: "refused".into(),
                    removed: Vec::new(),
                }));
                return Ok(());
            }
        }
        self.land(tx);
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.0.borrow_mut().queue))
    }
}

type Comp = Rc<RefCell<Compositor>>;

fn node(class: &str, parent: u64, props: Vec<(&str, MirrorValue)>) -> Node {
    Node {
        class: class.into(),
        parent,
        props: props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    }
}

/// A root, a workspace (2) and two windows: 3 `term` (foot) at 10,20 sized 300x200 with the focus, and 4 `web`
/// (firefox).
fn two_windows() -> Comp {
    let mut c = Compositor::default();
    c.nodes.insert(1, node("node", 0, vec![]));
    c.nodes.insert(
        2,
        node(
            "workspace",
            1,
            vec![("index", MirrorValue::Uint(0)), ("active", MirrorValue::Bool(true))],
        ),
    );
    c.nodes.insert(
        3,
        node(
            "window",
            1,
            vec![
                ("title", text("term")),
                ("app_id", text("foot")),
                ("focused", MirrorValue::Bool(true)),
                ("x", MirrorValue::Double(10.0)),
                ("y", MirrorValue::Double(20.0)),
                ("width", MirrorValue::Double(300.0)),
                ("height", MirrorValue::Double(200.0)),
            ],
        ),
    );
    c.nodes.insert(
        4,
        node("window", 1, vec![("title", text("web")), ("app_id", text("firefox"))]),
    );
    c.serial = 1;
    Rc::new(RefCell::new(c))
}

fn sent(comp: &Comp) -> Vec<Vec<TxOp>> {
    comp.borrow().sent.clone()
}

/// A booted shell with its entry module loaded.
struct Rig {
    shell: JsShell,
    /// What loading the entry reported.
    load_diagnostics: Vec<String>,
    _dir: TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// The text of `source`'s value, evaluated as a script.
    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .expect("the script has a value")
    }
}

fn rig_with(comp: &Comp, init: &str, tweak: impl FnOnce(&mut JsShellConfig)) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), init).expect("init.ts is written");
    let mut config = JsShellConfig::new(&root);
    tweak(&mut config);
    let mut shell = JsShell::new(Box::new(FakeLink(comp.clone())), config).expect("the shell boots");
    let load_diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    Rig {
        shell,
        load_diagnostics,
        _dir: dir,
    }
}

fn rig(comp: &Comp, init: &str) -> Rig {
    rig_with(comp, init, |_| {})
}

/// An entry module that keeps a log in `globalThis.log`, for tests to read back.
const LOG: &str = "import { ClientListener, CollectionListener } from \"gnoblin\";\n\
                   const log: string[] = [];\n\
                   (globalThis as any).log = log;\n";

fn with_log(rest: &str) -> String {
    format!("{LOG}{rest}")
}

#[test]
fn boot_reads_two_windows_through_gnoblin_clients() {
    let comp = two_windows();
    let mut rig = rig(
        &comp,
        "import { Client, ClientListener, defineConfig } from \"gnoblin\";\n\
         import * as host from \"gnoblin/host\";\n\
         (globalThis as any).Client = Client;\n\
         (globalThis as any).same = host.Client === Client && host.ClientListener === ClientListener;\n\
         (globalThis as any).stock = typeof defineConfig;\n",
    );
    assert_eq!(
        rig.eval("[same, stock].join()"),
        "true,function",
        "`gnoblin` is the stock library and the host objects in one module"
    );
    assert_eq!(rig.eval("gnoblin.clients.length"), "2");
    assert_eq!(rig.eval("gnoblin.clients.at(0).id + gnoblin.clients.at(-1).id"), "34");
    assert_eq!(rig.eval("gnoblin.clients.get('3').title"), "term");
    assert_eq!(rig.eval("gnoblin.clients.get('3').appId"), "foot");
    assert_eq!(rig.eval("gnoblin.clients.get('3').focused"), "true");
    assert_eq!(rig.eval("gnoblin.clients.get('4').fullscreen"), "false");
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.clients.get('3').bounds)"),
        r#"{"x":10,"y":20,"width":300,"height":200}"#
    );
    assert_eq!(rig.eval("gnoblin.clients.get('4').width"), "0");
    assert_eq!(rig.eval("gnoblin.clients.get('3') instanceof Client"), "true");
    assert_eq!(rig.eval("gnoblin.clients.get('9') === undefined"), "true");
    assert_eq!(rig.eval("gnoblin.clients.get('3') === gnoblin.clients.at(0)"), "true");
    assert_eq!(rig.eval("[...gnoblin.clients].map((c) => c.id).join()"), "3,4");
    assert_eq!(rig.eval("gnoblin.clients.map((c) => c.appId).join()"), "foot,firefox");
    assert_eq!(rig.eval("gnoblin.clients.filter((c) => c.focused).length"), "1");
    assert_eq!(rig.eval("gnoblin.clients.find((c) => c.title === 'web').id"), "4");
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
}

#[test]
fn workspaces_are_the_scripts_and_clients_are_the_compositors() {
    let comp = two_windows();
    let mut rig = rig(&comp, "export {};\n");
    // The tree's own workspace node (2) is not a member: a workspace is a script object, and a config that
    // declares none gets the stock `desktop` as soon as its entry has loaded.
    assert_eq!(rig.eval("gnoblin.workspaces.length"), "1");
    assert_eq!(rig.eval("gnoblin.workspaces.at(0).id"), "desktop");
    assert_eq!(rig.eval("typeof gnoblin.workspaces.add"), "function");
    assert_eq!(rig.eval("typeof gnoblin.clients.add"), "undefined");
}

#[test]
fn a_title_change_calls_on_title_changed_once_with_the_same_client() {
    let comp = two_windows();
    let init = with_log(
        "class Titles extends ClientListener {\n\
         \x20   onTitleChanged(client: any, title: string) {\n\
         \x20       log.push(`${client.id}:${title}:${client === gnoblin.clients.get(\"3\")}`);\n\
         \x20   }\n\
         }\n\
         gnoblin.clients.addListener(new Titles());\n",
    );
    let mut rig = rig(&comp, &init);
    comp_title(&comp, 3, "one");
    comp_title(&comp, 3, "two");
    assert!(rig.frame().is_empty());
    assert_eq!(
        rig.eval("log.join()"),
        "3:two:true",
        "two batches in a frame are one change"
    );
    assert!(rig.frame().is_empty());
    assert_eq!(rig.eval("log.length"), "1", "nothing changed in the next frame");
    comp_title(&comp, 3, "two");
    rig.frame();
    assert_eq!(
        rig.eval("log.length"),
        "1",
        "the same title published again is not a change"
    );
    assert_eq!(rig.eval("gnoblin.clients.get('3').title"), "two");
}

fn comp_title(comp: &Comp, node: u64, title: &str) {
    FakeLink(comp.clone()).external_set(node, "title", text(title));
}

#[test]
fn a_focus_and_a_geometry_change_each_raise_their_own_event_with_their_value() {
    let comp = two_windows();
    let init = with_log(
        "class All extends ClientListener {\n\
         \x20   onFocusChanged(client: any, focused: boolean) { log.push(`focus:${client.id}:${focused}`); }\n\
         \x20   onGeometryChanged(client: any, g: any) { log.push(`geometry:${client.id}:${g.x},${g.y},${g.width},${g.height}`); }\n\
         }\n\
         gnoblin.clients.addListener(new All());\n",
    );
    let mut rig = rig(&comp, &init);
    let link = FakeLink(comp.clone());
    link.external_set(3, "focused", MirrorValue::Bool(false));
    link.external_set(4, "focused", MirrorValue::Bool(true));
    link.external_set(3, "x", MirrorValue::Double(1.0));
    link.external_set(3, "width", MirrorValue::Double(50.0));
    rig.frame();
    assert_eq!(
        rig.eval("log.join(' ')"),
        "focus:3:false geometry:3:1,20,50,200 focus:4:true"
    );
}

#[test]
fn writing_x_queues_one_op_in_one_transaction_at_frame_end() {
    let comp = two_windows();
    let mut rig = rig(&comp, "gnoblin.clients.get('3').x = 10;\n");
    assert!(sent(&comp).is_empty(), "nothing is sent before the frame ends");
    assert_eq!(
        rig.eval("gnoblin.clients.get('3').x"),
        "10",
        "a write reads back at once"
    );
    rig.frame();
    let txs = sent(&comp);
    assert_eq!(txs.len(), 1, "one transaction");
    assert_eq!(txs[0].len(), 1, "with one op");
    match &txs[0][0] {
        TxOp::Set { node, key, value, .. } => {
            assert_eq!((*node, key.as_str(), value), (3, "x", &MirrorValue::Double(10.0)));
        }
        other => panic!("expected a set, got {other:?}"),
    }
    rig.frame();
    rig.frame();
    assert_eq!(sent(&comp).len(), 1, "a frame with no writes sends nothing");
    assert_eq!(rig.eval("gnoblin.clients.get('3').x"), "10");
}

#[test]
fn writes_made_by_listeners_during_a_frame_travel_in_that_frames_one_transaction() {
    let comp = two_windows();
    let init = with_log(
        "class Nudge extends ClientListener {\n\
         \x20   onTitleChanged(client: any) { client.y = 5; client.height = 6; }\n\
         }\n\
         gnoblin.clients.addListener(new Nudge());\n",
    );
    let mut rig = rig(&comp, &init);
    comp_title(&comp, 3, "moved");
    comp_title(&comp, 4, "moved too");
    rig.frame();
    let txs = sent(&comp);
    assert_eq!(txs.len(), 1);
    let keys: Vec<(u64, &str)> = txs[0]
        .iter()
        .map(|op| match op {
            TxOp::Set { node, key, .. } => (*node, key.as_str()),
            other => panic!("expected a set, got {other:?}"),
        })
        .collect();
    assert_eq!(keys, [(3, "y"), (3, "height"), (4, "y"), (4, "height")]);
}

#[test]
fn bounds_writes_all_four_values_or_none() {
    let comp = two_windows();
    let mut rig = rig(&comp, "export {};\n");
    assert_eq!(
        rig.eval(
            "(() => { try { gnoblin.clients.get('3').bounds = { x: 1, y: 2, width: NaN, height: 4 }; } \
             catch (e) { return e.name + ':' + e.message; } })()"
        ),
        "TypeError:Client#3.bounds.width: must be a finite number"
    );
    rig.frame();
    assert!(sent(&comp).is_empty(), "a rejected assignment queued nothing");
    rig.eval("gnoblin.clients.get('3').bounds = { x: 1, y: 2, width: 3, height: 4 }");
    rig.frame();
    assert_eq!(sent(&comp)[0].len(), 4);
}

#[test]
fn a_write_of_a_value_that_is_not_a_finite_number_names_the_member() {
    let comp = two_windows();
    let mut rig = rig(&comp, "export {};\n");
    let error = |value: &str| {
        format!(
            "(() => {{ try {{ gnoblin.clients.get('3').x = {value}; }} catch (e) {{ return e.name + ':' + e.message; }} }})()"
        )
    };
    assert_eq!(rig.eval(&error("NaN")), "TypeError:Client#3.x: must be a finite number");
    assert_eq!(rig.eval(&error("'1'")), "TypeError:Client#3.x: must be a finite number");
    assert_eq!(
        rig.eval(&error("Infinity")),
        "TypeError:Client#3.x: must be a finite number"
    );
    rig.frame();
    assert!(sent(&comp).is_empty());
}

#[test]
fn a_destroyed_window_is_not_alive_keeps_its_values_and_rejects_writes_naming_the_member() {
    let comp = two_windows();
    let init = with_log(
        "class Closing extends ClientListener {\n\
         \x20   onClosed(client: any) { log.push(`closed:${client.id}:${client.title}:${client.alive}:${client === (globalThis as any).kept}`); }\n\
         }\n\
         class Members extends CollectionListener {\n\
         \x20   onRemoved(client: any) { log.push(`removed:${client.id}:${gnoblin.clients.length}`); }\n\
         }\n\
         (globalThis as any).kept = gnoblin.clients.get(\"3\");\n\
         gnoblin.clients.addListener(new Closing());\n\
         gnoblin.clients.addListener(new Members());\n",
    );
    let mut rig = rig(&comp, &init);
    rig.eval("gnoblin.clients.get('3').x = 1; 0");
    FakeLink(comp.clone()).destroy(3);
    assert!(rig.frame().is_empty());
    assert_eq!(rig.eval("kept.alive"), "false");
    assert_eq!(rig.eval("kept.title"), "term", "the last values stay readable");
    assert_eq!(rig.eval("kept.width"), "300");
    assert_eq!(rig.eval("gnoblin.clients.length"), "1");
    assert_eq!(rig.eval("gnoblin.clients.get('3') === undefined"), "true");
    assert_eq!(
        rig.eval("log.join(' ')"),
        "closed:3:term:false:true removed:3:1",
        "closed once, then removed, with the client already out of the collection"
    );
    let write =
        "(() => { try { kept.x = 5; } catch (e) { return [e.name, e.code, e.target, e.message].join('|'); } })()";
    assert_eq!(
        rig.eval(write),
        "GnoblinError|destroyed|Client#3.x|Client#3.x: the client is gone"
    );
    assert_eq!(
        rig.eval("(() => { try { kept.close(); } catch (e) { return e.target; } })()"),
        "Client#3.close()"
    );
    rig.frame();
    assert_eq!(
        sent(&comp)
            .iter()
            .flatten()
            .filter(|op| matches!(op, TxOp::Set { .. }))
            .count(),
        1,
        "only the write made before the window went is ever sent, and it is refused, not retried"
    );
}

#[test]
fn close_is_a_call_on_the_window_and_focus_is_a_write_of_the_seats_focus() {
    // The compositor answers no `focus` call on a window (only `close` and `kill`); a shell gives focus by writing
    // the seat's `focus`, as the Lua shell's `seat.focus = window` does (crates/gnoblin/tests/js_shell.rs proves it).
    let comp = two_windows();
    comp.borrow_mut().nodes.insert(
        9,
        node(
            "seat",
            1,
            vec![("name", text("seat0")), ("focus", MirrorValue::Node(3))],
        ),
    );
    let mut rig = rig(
        &comp,
        "gnoblin.clients.get('4').focus();\ngnoblin.clients.get('3').close();\n",
    );
    rig.frame();
    let log = sent(&comp);
    // The stock config claims its keys on the seat in the same frame; those are not what this case looks at.
    let ops: Vec<String> = log[0]
        .iter()
        .filter(|op| !matches!(op, TxOp::Call { method, .. } if method.starts_with("claim")))
        .map(|op| match op {
            TxOp::Call { node, method, .. } => format!("call:{node}:{method}"),
            TxOp::Set {
                node,
                key,
                value: MirrorValue::Node(target),
                ..
            } => format!("set:{node}.{key}={target}"),
            other => panic!("expected a call or a seat write, got {other:?}"),
        })
        .collect();
    assert_eq!(ops, ["set:9.focus=4", "call:3:close"]);
}

#[test]
fn focus_has_nowhere_to_go_in_a_tree_with_no_seat() {
    let comp = two_windows();
    let mut rig = rig(
        &comp,
        "let thrown = 'no error';\n\
         try { gnoblin.clients.get('4').focus(); } catch (e) { thrown = `${(e as Error).name}: ${(e as Error).message}`; }\n\
         (globalThis as any).thrown = thrown;\n",
    );
    assert_eq!(
        rig.eval("thrown"),
        "GnoblinError: Client#4.focus(): the host refused the write (no-seat)"
    );
    rig.frame();
}

/// A listener that records every membership event of a collection, and the one it is attached to.
const MEMBERS: &str = "class Members extends CollectionListener {\n\
    \x20   onAdded(member: any, collection: any) { log.push(`added:${member.id}:${collection.length}`); }\n\
    \x20   onRemoved(member: any) { log.push(`removed:${member.id}`); }\n\
    }\n";

#[test]
fn a_collection_listener_added_late_is_told_of_the_members_already_there_and_then_of_new_ones() {
    let comp = two_windows();
    let init = with_log(&format!(
        "{MEMBERS}(globalThis as any).members = new Members();\ngnoblin.clients.addListener((globalThis as any).members);\n"
    ));
    let mut rig = rig(&comp, &init);
    assert_eq!(
        rig.eval("log.join(' ')"),
        "added:3:2 added:4:2",
        "replay on attach: each existing member, in order, as soon as the listener is added"
    );
    FakeLink(comp.clone()).create_window(5, "new", "kitty");
    assert!(rig.frame().is_empty());
    assert_eq!(rig.eval("log.join(' ')"), "added:3:2 added:4:2 added:5:3");
    assert_eq!(rig.eval("gnoblin.clients.get('5').appId"), "kitty");
    // Adding the same listener twice does not subscribe it twice.
    rig.eval("gnoblin.clients.addListener(members); 0");
    assert_eq!(rig.eval("log.length"), "3");
    FakeLink(comp.clone()).destroy(4);
    rig.frame();
    assert_eq!(rig.eval("log.join(' ')"), "added:3:2 added:4:2 added:5:3 removed:4");
    rig.eval("gnoblin.clients.removeListener(members); 0");
    FakeLink(comp.clone()).create_window(6, "later", "foot");
    FakeLink(comp.clone()).destroy(3);
    rig.frame();
    assert_eq!(rig.eval("log.length"), "4", "a removed listener hears nothing more");
}

#[test]
fn a_client_listener_on_a_collection_follows_current_and_future_members() {
    let comp = two_windows();
    let init = with_log(
        "class Titles extends ClientListener {\n\
         \x20   onTitleChanged(client: any, title: string) { log.push(`${client.id}:${title}`); }\n\
         }\n\
         (globalThis as any).titles = new Titles();\n\
         gnoblin.clients.addListener((globalThis as any).titles);\n",
    );
    let mut rig = rig(&comp, &init);
    assert_eq!(
        rig.eval("log.length"),
        "0",
        "a client listener is not replayed: nothing happened yet"
    );
    comp_title(&comp, 4, "an existing member");
    FakeLink(comp.clone()).create_window(5, "first", "kitty");
    rig.frame();
    comp_title(&comp, 5, "a future member");
    rig.frame();
    assert_eq!(
        rig.eval("log.join()"),
        "4:an existing member,5:a future member",
        "a window made in this frame is born with its title, so it raises no title event"
    );
    rig.eval("gnoblin.clients.removeListener(titles); 0");
    comp_title(&comp, 4, "unheard");
    rig.frame();
    assert_eq!(rig.eval("log.length"), "2");
}

#[test]
fn a_listener_on_one_client_hears_only_that_client() {
    let comp = two_windows();
    let init = with_log(
        "class Titles extends ClientListener {\n\
         \x20   onTitleChanged(client: any, title: string) { log.push(`${client.id}:${title}`); }\n\
         }\n\
         gnoblin.clients.find((c: any) => c.appId === \"firefox\")?.addListener(new Titles());\n",
    );
    let mut rig = rig(&comp, &init);
    comp_title(&comp, 3, "not heard");
    comp_title(&comp, 4, "heard");
    rig.frame();
    assert_eq!(rig.eval("log.join()"), "4:heard");
}

#[test]
fn where_is_a_live_selection_that_follows_appid_changes_and_tells_its_listeners() {
    let comp = two_windows();
    let init = with_log(&format!(
        "{MEMBERS}(globalThis as any).view = gnoblin.clients.where({{ appId: \"firefox\" }});\n\
         view.addListener(new Members());\n"
    ));
    let mut rig = rig(&comp, &init);
    assert_eq!(rig.eval("view.length"), "1");
    assert_eq!(rig.eval("view.at(0).id"), "4");
    assert_eq!(rig.eval("log.join(' ')"), "added:4:1");
    FakeLink(comp.clone()).external_set(3, "app_id", text("firefox"));
    rig.frame();
    assert_eq!(rig.eval("view.length"), "2");
    FakeLink(comp.clone()).external_set(4, "app_id", text("chromium"));
    rig.frame();
    assert_eq!(rig.eval("view.map((c) => c.id).join()"), "3");
    assert_eq!(rig.eval("log.join(' ')"), "added:4:1 added:3:2 removed:4");
    FakeLink(comp.clone()).destroy(3);
    rig.frame();
    assert_eq!(rig.eval("view.length"), "0");
    assert_eq!(rig.eval("log.join(' ')"), "added:4:1 added:3:2 removed:4 removed:3");
    assert_eq!(
        rig.eval("(() => { try { gnoblin.clients.where({ colour: 'red' }); } catch (e) { return e.name; } })()"),
        "TypeError"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.where({ appId: ['foot', 'chromium'] }).length"),
        "1"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.where({ appId: 'chromium' }).where({ id: '4' }).length"),
        "1"
    );
}

#[test]
fn a_listener_that_throws_does_not_stop_its_siblings_and_is_reported_once_per_site() {
    let comp = two_windows();
    let init = "import { ClientListener } from \"gnoblin\";\n\
                const log: string[] = [];\n\
                (globalThis as any).log = log;\n\
                class Bad extends ClientListener {\n\
                \x20   onTitleChanged(client: any, title: string) {\n\
                \x20       throw new Error(\"bad \" + title);\n\
                \x20   }\n\
                }\n\
                class Good extends ClientListener {\n\
                \x20   onTitleChanged(client: any, title: string) {\n\
                \x20       log.push(title);\n\
                \x20   }\n\
                }\n\
                gnoblin.clients.addListener(new Bad());\n\
                gnoblin.clients.addListener(new Good());\n\
                gnoblin.clients.get(\"4\")!.addListener(new Bad());\n";
    let mut rig = rig(&comp, init);
    comp_title(&comp, 3, "a");
    comp_title(&comp, 4, "b");
    let first = rig.frame();
    assert_eq!(rig.eval("log.join()"), "a,b", "the sibling ran for both clients");
    assert_eq!(first.len(), 1, "one site, reported once: {first:?}");
    assert!(first[0].contains("Bad.onTitleChanged threw"), "{}", first[0]);
    assert!(
        first[0].contains("init.ts:6:"),
        "the .ts line of the throw: {}",
        first[0]
    );
    assert!(first[0].contains("bad a"), "{}", first[0]);
    comp_title(&comp, 3, "c");
    let second = rig.frame();
    assert!(second.is_empty(), "the same site is not reported again: {second:?}");
    assert_eq!(rig.eval("log.join()"), "a,b,c");
    assert_eq!(rig.shell.strikes(), 0, "a throw is not a budget strike");
}

#[test]
fn a_listener_that_never_returns_is_stopped_counts_a_strike_and_the_others_still_run() {
    let comp = two_windows();
    let init = with_log(
        "class Spin extends ClientListener {\n\
         \x20   onFocusChanged() { while (true) {} }\n\
         }\n\
         class After extends ClientListener {\n\
         \x20   onFocusChanged(client: any) { log.push(\"after:\" + client.id); }\n\
         }\n\
         gnoblin.clients.addListener(new Spin());\n\
         gnoblin.clients.addListener(new After());\n",
    );
    let mut rig = rig_with(&comp, &init, |config| config.entry_budget = Duration::from_millis(50));
    let link = FakeLink(comp.clone());
    link.external_set(3, "focused", MirrorValue::Bool(false));
    link.external_set(4, "focused", MirrorValue::Bool(true));
    let diagnostics = rig.frame();
    assert_eq!(diagnostics.len(), 2, "each stopped call is named: {diagnostics:?}");
    assert!(diagnostics.iter().all(|d| d.contains("budget")), "{diagnostics:?}");
    assert_eq!(rig.shell.strikes(), 2);
    assert_eq!(rig.eval("log.join()"), "after:3,after:4");
    assert!(rig.frame().is_empty(), "the generation goes on");
}

#[test]
fn ready_is_announced_for_snapshot_nodes_parents_first_after_the_entry_ran_and_for_new_nodes_after_added() {
    let mut c = Compositor::default();
    c.nodes.insert(1, node("node", 0, vec![]));
    // The child has the smaller id, so only a walk of the tree puts its parent first.
    c.nodes.insert(5, node("window", 1, vec![("title", text("parent"))]));
    c.nodes.insert(3, node("window", 5, vec![("title", text("child"))]));
    c.serial = 1;
    let comp = Rc::new(RefCell::new(c));
    let init = with_log(
        "class Lifecycle {\n\
         \x20   onAdded(client: any) { log.push(`added:${client.id}`); }\n\
         \x20   onReady(client: any) { log.push(`ready:${client.id}`); }\n\
         }\n\
         gnoblin.clients.addListener(new Lifecycle());\n\
         log.push(\"entry\");\n",
    );
    let mut rig = rig(&comp, &init);
    assert_eq!(
        rig.eval("log.join()"),
        "added:5,added:3,entry",
        "replay on attach, then the entry's own line"
    );
    rig.frame();
    assert_eq!(rig.eval("log.join()"), "added:5,added:3,entry,ready:5,ready:3");
    FakeLink(comp.clone()).create_window(7, "late", "foot");
    rig.frame();
    assert_eq!(rig.eval("log.slice(5).join()"), "added:7,ready:7");
}

#[test]
fn input_events_with_no_api_yet_are_consumed_without_a_failure() {
    let comp = two_windows();
    let mut rig = rig(&comp, "export {};\n");
    comp.borrow_mut()
        .queue
        .push(TreeIncoming::Chord(gnoblin_tree_client::tree_mirror::TreeChord {
            claim: 1,
            chord: "Super+a".into(),
            state: "press".into(),
        }));
    assert!(rig.frame().is_empty());
}

// The protocol rules the Lua shell keeps, held for JavaScript.

#[test]
fn holds_writes_while_a_transaction_is_in_flight() {
    let comp = two_windows();
    comp.borrow_mut().hold = true;
    let mut rig = rig(&comp, "export {};\n");
    rig.frame();
    rig.eval("gnoblin.clients.get('3').x = 11; 0");
    rig.frame();
    assert_eq!(sent(&comp).len(), 1, "first transaction sent");

    rig.eval("gnoblin.clients.get('3').y = 12; 0");
    rig.frame();
    assert_eq!(
        sent(&comp).len(),
        1,
        "second transaction held while the first is in flight"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.get('3').y"),
        "12",
        "a held write still reads back"
    );

    FakeLink(comp.clone()).release();
    rig.frame();
    rig.frame();
    assert_eq!(sent(&comp).len(), 2, "the held write is sent after the echo");
    FakeLink(comp.clone()).release();
    rig.frame();
    assert_eq!(
        rig.eval("JSON.stringify([gnoblin.clients.get('3').x, gnoblin.clients.get('3').y])"),
        "[11,12]"
    );
    assert_eq!(sent(&comp).len(), 2);
}

#[test]
fn a_stale_base_refusal_sends_every_write_again_on_the_new_base_and_the_scripts_write_stands() {
    let comp = two_windows();
    comp.borrow_mut().hold = true;
    let mut rig = rig(&comp, "export {};\n");
    rig.frame();
    rig.eval("gnoblin.clients.get('3').x = 7; gnoblin.clients.get('4').y = 8; 0");
    let first = rig.frame();
    assert!(first.is_empty(), "{first:?}");
    assert_eq!(sent(&comp).len(), 1, "the first transaction is held in flight");

    FakeLink(comp.clone()).external_set(3, "x", MirrorValue::Double(99.0));
    FakeLink(comp.clone()).release();
    let refused = rig.frame();
    assert!(
        refused.is_empty(),
        "a stale-base refusal is retried, not reported: {refused:?}"
    );
    let txs = sent(&comp);
    assert_eq!(txs.len(), 2, "the writes are sent again");
    let keys: Vec<(u64, &str)> = txs[1]
        .iter()
        .map(|op| match op {
            TxOp::Set { node, key, .. } => (*node, key.as_str()),
            other => panic!("expected a set, got {other:?}"),
        })
        .collect();
    assert_eq!(
        keys,
        [(3, "x"), (4, "y")],
        "the refused write goes again, with the one beside it"
    );

    FakeLink(comp.clone()).release();
    let done = rig.frame();
    assert!(done.is_empty(), "{done:?}");
    assert_eq!(
        rig.eval("gnoblin.clients.get('3').x"),
        "7",
        "sent after the native write, so the script's intent stands"
    );
    assert_eq!(rig.eval("gnoblin.clients.get('4').y"), "8");
}

#[test]
fn a_write_refused_stale_base_every_time_is_dropped_after_its_retries_and_the_others_land() {
    let comp = two_windows();
    comp.borrow_mut().hold = true;
    let mut rig = rig(&comp, "export {};\n");
    rig.frame();
    rig.eval("gnoblin.clients.get('3').x = 7; gnoblin.clients.get('4').y = 8; 0");
    rig.frame();
    // The compositor changes window 3 before every landing, so its write is refused each time.
    for round in 0..=gnoblin_js::STALE_RETRIES {
        FakeLink(comp.clone()).external_set(3, "title", text(&format!("t{round}")));
        FakeLink(comp.clone()).release();
        rig.frame();
    }
    let last: Vec<(u64, String)> = sent(&comp)
        .last()
        .expect("a transaction")
        .iter()
        .map(|op| match op {
            TxOp::Set { node, key, .. } => (*node, key.clone()),
            other => panic!("expected a set, got {other:?}"),
        })
        .collect();
    assert_eq!(
        last,
        [(4, "y".to_owned())],
        "the endlessly refused write is dropped and the other goes on"
    );
}

#[test]
fn an_unknown_node_refusal_sends_the_writes_on_nodes_that_are_still_there() {
    let comp = two_windows();
    comp.borrow_mut().hold = true;
    let mut rig = rig(&comp, "export {};\n");
    rig.frame();
    rig.eval("gnoblin.clients.get('3').x = 7; gnoblin.clients.get('4').y = 8; 0");
    rig.frame();
    FakeLink(comp.clone()).destroy(3);
    FakeLink(comp.clone()).release();
    let refused = rig.frame();
    assert!(refused.is_empty(), "{refused:?}");
    let txs = sent(&comp);
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[1].len(), 1, "the write on the window that is gone is dropped");
    assert!(matches!(&txs[1][0], TxOp::Set { node: 4, key, .. } if key == "y"));
}

#[test]
fn a_refused_transaction_is_reported_naming_the_op_and_opens_the_gate() {
    let comp = two_windows();
    comp.borrow_mut().refuse_next = true;
    let mut rig = rig(&comp, "export {};\n");
    rig.frame();
    rig.eval("gnoblin.clients.get('3').x = 7; 0");
    rig.frame();
    let report = rig.frame();
    assert_eq!(report.len(), 1, "{report:?}");
    assert!(report[0].contains("refused at op 0: refused"), "{}", report[0]);
    assert!(
        report[0].contains("set node 3 (Window) property x value kind double"),
        "{}",
        report[0]
    );
    rig.eval("gnoblin.clients.get('3').y = 9; 0");
    rig.frame();
    assert_eq!(sent(&comp).len(), 2, "the refusal did not close the gate");
}

#[test]
fn an_entry_that_throws_is_an_error_with_its_ts_location_and_a_budget_trip_counts_a_strike() {
    let comp = two_windows();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("bad.ts"),
        "const n: number = 1;\nthrow new Error('boom ' + n);\n",
    )
    .expect("bad.ts");
    fs::write(root.join("loop.ts"), "while (true) {}\n").expect("loop.ts");
    let mut config = JsShellConfig::new(&root);
    config.js.stage_budget = Duration::from_millis(100);
    let mut shell = JsShell::new(Box::new(FakeLink(comp)), config).expect("the shell boots");
    match shell.load_entry(Path::new("bad.ts")) {
        Err(JsShellError::Js(error)) => {
            assert_eq!(error.location.expect("a location").line, 2);
        }
        other => panic!("expected a script error, got {other:?}"),
    }
    assert_eq!(shell.strikes(), 0);
    assert!(shell.load_entry(Path::new("loop.ts")).is_err());
    assert_eq!(shell.strikes(), 1);
}

/// Every `x` write the shell has sent, as (node, value), oldest first.
fn x_writes(comp: &Comp) -> Vec<(u64, f64)> {
    sent(comp)
        .into_iter()
        .flatten()
        .filter_map(|op| match op {
            TxOp::Set {
                node,
                key,
                value: MirrorValue::Double(x),
                ..
            } if key == "x" => Some((node, x)),
            _ => None,
        })
        .collect()
}

/// An entry that puts every client at `x`, on add.
fn places_at(x: u32) -> String {
    format!(
        "import {{ gnoblin }} from \"gnoblin\";\n\
         gnoblin.clients.addListener({{ onAdded(client: {{ x: number }}): void {{ client.x = {x}; }} }});\n"
    )
}

#[test]
fn a_role_swaps_in_a_reloaded_generation_over_the_same_windows_and_a_failed_reload_changes_nothing() {
    let comp = two_windows();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), places_at(100)).expect("init.ts");
    let mut role = JsRole::over_link(
        Box::new(FakeLink(comp.clone())),
        JsRoleLinks::default(),
        JsRoleConfig::new(&root, None),
    )
    .expect("a role");
    let booted = role.generation_id();
    assert_eq!(booted, 1, "the entry loaded as the first staged generation");
    role.tick().expect("a frame");
    assert_eq!(
        x_writes(&comp),
        vec![(3, 100.0), (4, 100.0)],
        "the listener placed both windows"
    );

    // A reload: a newer generation, the same window nodes, and its listener is told of the windows already there.
    fs::write(root.join("init.ts"), places_at(200)).expect("init.ts");
    let report = role.reload_now();
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(role.generation_id(), booted + 1);
    role.tick().expect("a frame");
    // The save also reached the watcher, which may reload once more before the tick ends: the same file, so the
    // same writes again. What matters is that the new generation placed the same two nodes.
    let after: Vec<(u64, f64)> = x_writes(&comp)[2..].to_vec();
    assert!(
        after.iter().all(|(_, x)| *x == 200.0) && after.contains(&(3, 200.0)) && after.contains(&(4, 200.0)),
        "the new generation placed the same two windows: {after:?}"
    );
    let running = role.generation_id();
    assert!(running > booted);

    // A syntax error keeps the running generation and reports the position in the author's file.
    fs::write(
        root.join("init.ts"),
        "import { gnoblin } from \"gnoblin\";\n\nlet = = 5;\n",
    )
    .expect("init.ts");
    let report = role.reload_now();
    assert!(!report.ok);
    assert!(
        report.errors[0].to_string().contains("init.ts:3:"),
        "{}",
        report.errors[0]
    );
    assert_eq!(role.generation_id(), running, "the generation that ran still runs");
    assert!(role.diagnostics().iter().any(|text| text.contains("init.ts:3:")));

    // A file that writes and then throws leaves no write behind: the failed stage's top-level write is dropped.
    fs::write(
        root.join("init.ts"),
        "import { gnoblin } from \"gnoblin\";\ngnoblin.clients.at(0).x = 999;\nthrow new Error(\"late\");\n",
    )
    .expect("init.ts");
    let sent_before = x_writes(&comp).len();
    let report = role.reload_now();
    assert!(!report.ok);
    assert_eq!(role.generation_id(), running);
    role.tick().expect("a frame");
    assert_eq!(
        x_writes(&comp).len(),
        sent_before,
        "the failed stage's write is not sent: {:?}",
        x_writes(&comp)
    );
}

// ---- the console ---------------------------------------------------------------------------------------------------

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;

use gnoblin_js::{JsConsole, JsConsoleSite, evaluate_in};
use serde_json::{Value as Json, json};

/// One connection to a console server that the test polls, as the role's loop polls it between frames.
struct Prompt {
    stream: UnixStream,
    pending: Vec<u8>,
    next_id: u64,
}

impl Prompt {
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).expect("the console accepts");
        stream.set_nonblocking(true).expect("a non-blocking socket");
        Self {
            stream,
            pending: Vec::new(),
            next_id: 1,
        }
    }

    /// Sends one line as it is, and waits for the reply line while `serve` polls the server.
    fn raw(&mut self, line: &str, serve: &mut dyn FnMut()) -> Json {
        self.stream.write_all(line.as_bytes()).expect("the request is written");
        self.stream.write_all(b"\n").expect("the newline is written");
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            serve();
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => panic!("the console closed the connection"),
                Ok(count) => self.pending.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("reading the reply: {error}"),
            }
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                return serde_json::from_slice(&line).expect("a reply is one JSON object");
            }
            assert!(std::time::Instant::now() < deadline, "no reply to {line}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Evaluates `expr` and returns the whole reply; its id is checked to be the request's.
    fn ask(&mut self, expr: &str, serve: &mut dyn FnMut()) -> Json {
        let id = self.next_id;
        self.next_id += 1;
        let reply = self.raw(&json!({"id": id, "expr": expr}).to_string(), serve);
        assert_eq!(reply["id"], json!(id), "the reply answers its request: {reply}");
        reply
    }
}

/// The value of an expression that is expected to succeed.
fn value_of(reply: &Json) -> &Json {
    assert_eq!(reply["ok"], json!(true), "the expression failed: {reply}");
    &reply["value"]
}

#[test]
fn the_console_evaluates_in_the_running_generation_and_renders_what_it_finds() {
    let comp = two_windows();
    let rig = rig(&comp, "export {};\n");
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("js.sock");
    let mut console = JsConsole::bind(&path).expect("the console listens");
    let mut prompt = Prompt::connect(&path);
    let mut serve = || console.serve(&rig.shell);

    let reply = prompt.ask("gnoblin.clients.length", &mut serve);
    assert_eq!(
        (value_of(&reply), &reply["type"], &reply["text"]),
        (&json!(2), &json!("number"), &json!("2"))
    );
    assert_eq!(reply["truncated"], json!(false));
    assert_eq!(
        value_of(&prompt.ask("gnoblin.clients.map(c => c.appId)", &mut serve)),
        &json!(["foot", "firefox"])
    );
    assert_eq!(
        value_of(&prompt.ask("gnoblin.clients.focused?.title", &mut serve)),
        &json!("term"),
        "a tree with no seat reads the focus from the window's own `focused`"
    );

    // A client prints as its kind, its id and its members; a collection is an array of summaries.
    let client = prompt.ask("gnoblin.clients.at(0)", &mut serve);
    assert_eq!(client["type"], json!("Client"));
    let text = client["text"].as_str().expect("text");
    assert!(
        text.starts_with("Client#3 { appId: \"foot\", title: \"term\", focused: true"),
        "{text}"
    );
    assert_eq!(
        value_of(&client),
        &json!({"$class": "Client", "id": "3", "appId": "foot", "title": "term", "focused": true,
            "fullscreen": false, "floating": false, "x": 10, "y": 20, "width": 300, "height": 200,
            "workspace": client["value"]["workspace"]})
    );
    let collection = prompt.ask("gnoblin.clients", &mut serve);
    assert_eq!(collection["type"], json!("Collection"));
    assert_eq!(
        value_of(&collection),
        &json!([
            {"$class": "Client", "id": "3", "appId": "foot", "title": "term"},
            {"$class": "Client", "id": "4", "appId": "firefox", "title": "web"},
        ])
    );
    assert!(
        collection["text"]
            .as_str()
            .expect("text")
            .starts_with("Collection(2) [ Client#3 {")
    );

    // Functions, cycles, undefined, an object literal and a settled promise.
    let function = prompt.ask("(function greet() {})", &mut serve);
    assert_eq!(
        (value_of(&function), &function["text"]),
        (&json!("[Function greet]"), &json!("[Function greet]"))
    );
    assert_eq!(
        value_of(&prompt.ask("(() => 1)", &mut serve)),
        &json!("[Function (anonymous)]")
    );
    assert_eq!(
        value_of(&prompt.ask(
            "(() => { const a = { name: 'a' }; a.self = a; return a; })()",
            &mut serve
        )),
        &json!({"name": "a", "self": "[Circular]"})
    );
    let nothing = prompt.ask("undefined", &mut serve);
    assert_eq!(
        (&nothing["ok"], &nothing["type"], nothing.get("value")),
        (&json!(true), &json!("undefined"), None)
    );
    assert_eq!(
        value_of(&prompt.ask("{a: 1, b: [1, 2]}", &mut serve)),
        &json!({"a": 1, "b": [1, 2]})
    );
    assert_eq!(value_of(&prompt.ask("Promise.resolve(7)", &mut serve)), &json!(7));
    assert_eq!(value_of(&prompt.ask("NaN", &mut serve)), &json!("NaN"));

    // Variables defined at the prompt stay for the next expression.
    // A declaration has no value, as at any prompt.
    assert_eq!(prompt.ask("var counter = 41", &mut serve)["type"], json!("undefined"));
    assert_eq!(value_of(&prompt.ask("counter + 1", &mut serve)), &json!(42));
    assert_eq!(
        prompt.ask("let held = gnoblin.clients.at(1)", &mut serve)["ok"],
        json!(true)
    );
    assert_eq!(value_of(&prompt.ask("held.title", &mut serve)), &json!("web"));
    assert_eq!(
        value_of(&prompt.ask("sloppy = 5", &mut serve)),
        &json!(5),
        "an undeclared name is a global, not an error"
    );
}

#[test]
fn a_rendering_that_outgrows_the_cap_is_cut_and_says_so() {
    let comp = two_windows();
    let rig = rig(&comp, "export {};\n");
    let big = evaluate_in(&rig.shell, "Array.from({ length: 5000 }, (_, i) => i)", 200).expect("it renders");
    let big: Json = serde_json::from_str(&big).expect("JSON");
    assert_eq!(big["truncated"], json!(true));
    let items = big["value"].as_array().expect("an array");
    assert!(items.len() < 100, "the array is cut: {}", items.len());
    assert!(
        items
            .last()
            .and_then(Json::as_str)
            .is_some_and(|text| text.contains("more")),
        "{items:?}"
    );
    let long = evaluate_in(&rig.shell, "'x'.repeat(100000)", 65536).expect("it renders");
    let long: Json = serde_json::from_str(&long).expect("JSON");
    assert_eq!(long["truncated"], json!(true));
    assert!(long["value"].as_str().expect("a string").len() <= 4096);
}

#[test]
fn a_console_error_carries_its_message_and_where_in_the_expression_it_was() {
    let comp = two_windows();
    let rig = rig(&comp, "export {};\n");
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("js.sock");
    let mut console = JsConsole::bind(&path).expect("the console listens");
    let mut prompt = Prompt::connect(&path);
    let mut serve = || console.serve(&rig.shell);

    let syntax = prompt.ask("1 +", &mut serve);
    assert_eq!(syntax["ok"], json!(false), "{syntax}");
    assert_eq!(syntax["error"]["kind"], json!("exception"));
    assert!(!syntax["error"]["message"].as_str().expect("a message").is_empty());
    assert_eq!(syntax["error"]["location"]["file"], json!("<console>"), "{syntax}");
    assert_eq!(syntax["error"]["location"]["line"], json!(1), "{syntax}");

    let thrown = prompt.ask("1;\n2;\nnull.x", &mut serve);
    assert_eq!(thrown["ok"], json!(false), "{thrown}");
    assert_eq!(thrown["error"]["location"]["file"], json!("<console>"), "{thrown}");
    assert_eq!(thrown["error"]["location"]["line"], json!(3), "{thrown}");
    assert!(
        thrown["error"]["stack"]
            .as_str()
            .expect("a stack")
            .contains("<console>")
    );

    let custom = prompt.ask("throw new Error('boom')", &mut serve);
    assert_eq!(custom["error"]["message"], json!("boom"));

    let bad = prompt.raw("this is not json", &mut serve);
    assert_eq!(
        (&bad["ok"], &bad["error"]["kind"]),
        (&json!(false), &json!("request")),
        "{bad}"
    );
    let missing = prompt.raw(r#"{"id": 9}"#, &mut serve);
    assert_eq!(
        (&missing["id"], &missing["error"]["kind"]),
        (&json!(9), &json!("request")),
        "{missing}"
    );
    assert_eq!(
        value_of(&prompt.ask("1 + 1", &mut serve)),
        &json!(2),
        "the connection survives a bad request"
    );
}

#[test]
fn a_runaway_expression_is_interrupted_and_the_shell_and_the_console_go_on() {
    let comp = two_windows();
    let mut rig = rig_with(&comp, "export {};\n", |config| {
        config.entry_budget = Duration::from_millis(50)
    });
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("js.sock");
    let mut console = JsConsole::bind(&path).expect("the console listens");
    let mut prompt = Prompt::connect(&path);
    {
        let mut serve = || console.serve(&rig.shell);
        assert_eq!(prompt.ask("var kept = 'still here'", &mut serve)["ok"], json!(true));
        let runaway = prompt.ask("while (true) {}", &mut serve);
        assert_eq!(runaway["ok"], json!(false), "{runaway}");
        assert_eq!(runaway["error"]["kind"], json!("budget"), "{runaway}");
        assert!(runaway["error"]["message"].as_str().expect("message").contains("50 ms"));
        // A loop that catches what it can still cannot swallow the interrupt.
        let stubborn = prompt.ask("for (;;) { try { while (true) {} } catch (e) {} }", &mut serve);
        assert_eq!(stubborn["error"]["kind"], json!("budget"), "{stubborn}");
        assert_eq!(
            value_of(&prompt.ask("kept", &mut serve)),
            &json!("still here"),
            "the session survives the interrupt"
        );
        assert_eq!(value_of(&prompt.ask("gnoblin.clients.length", &mut serve)), &json!(2));
    }
    assert_eq!(
        rig.shell.strikes(),
        0,
        "a console expression is not a strike against the generation"
    );
    rig.frame();
    assert_eq!(rig.eval("gnoblin.clients.length"), "2", "the shell still answers");
}

#[test]
fn a_write_made_at_the_prompt_travels_in_the_next_frames_transaction() {
    let comp = two_windows();
    let mut rig = rig(&comp, "export {};\n");
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("js.sock");
    let mut console = JsConsole::bind(&path).expect("the console listens");
    let mut prompt = Prompt::connect(&path);
    assert_eq!(
        value_of(&prompt.ask("gnoblin.clients.at(0).x = 100", &mut || console.serve(&rig.shell))),
        &json!(100)
    );
    rig.frame();
    let wrote = sent(&comp).into_iter().flatten().any(
        |op| matches!(op, TxOp::Set { node: 3, ref key, ref value, .. } if key == "x" && *value == MirrorValue::Double(100.0)),
    );
    assert!(wrote, "{:?}", sent(&comp));
}

#[test]
fn the_inspector_helpers_return_plain_data() {
    let comp = two_windows();
    let mut rig = rig(
        &comp,
        "import { defineConfig } from \"gnoblin\";\ndefineConfig({ gaps: { inner: 7 } });\n",
    );
    rig.frame();
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("js.sock");
    let mut console = JsConsole::bind(&path).expect("the console listens");
    let mut prompt = Prompt::connect(&path);
    let mut serve = || console.serve(&rig.shell);

    let clients = prompt.ask("inspect.clients()", &mut serve);
    let clients = value_of(&clients).as_array().expect("an array");
    assert_eq!(clients.len(), 2);
    assert_eq!(clients[0]["appId"], json!("foot"));
    assert_eq!(clients[0]["focused"], json!(true));
    assert_eq!(clients[1]["focused"], json!(false));
    assert_eq!(clients[0]["workspace"], json!("1"));
    assert_eq!(clients[0]["x"], json!(10));

    let workspaces = prompt.ask("inspect.workspaces()", &mut serve);
    let workspaces = value_of(&workspaces).as_array().expect("an array");
    assert_eq!(workspaces[0]["id"], json!("1"));
    assert_eq!(workspaces[0]["layout"], json!("FloatingLayout"));
    assert_eq!(workspaces[0]["clients"], json!(["3", "4"]));
    assert_eq!(
        value_of(&prompt.ask("gnoblin.workspaces.map(ws => [ws.id, ws.clients.length])", &mut serve)),
        &json!([["1", 2], ["2", 0], ["3", 0], ["4", 0]])
    );

    assert_eq!(
        value_of(&prompt.ask("inspect.bindings()", &mut serve)),
        &json!([]),
        "a tree with no seat binds nothing"
    );

    let config = prompt.ask("inspect.config()", &mut serve);
    let config = value_of(&config);
    assert_eq!(config["gaps"]["inner"], json!(7), "the user's value");
    assert_eq!(
        config["workspaces"][0]["id"],
        json!("1"),
        "the stock workspace the user left alone"
    );
    assert_eq!(config["keys"]["XF86AudioMute"]["kind"], json!("spawn"), "a stock key");
}

#[test]
fn a_shell_whose_entry_never_called_define_config_has_no_effective_config() {
    let comp = two_windows();
    let rig = rig(&comp, "export {};\n");
    let config = evaluate_in(&rig.shell, "inspect.config()", 65536).expect("it renders");
    let config: Json = serde_json::from_str(&config).expect("JSON");
    assert_eq!(config["type"], json!("null"));
}

#[test]
fn the_role_serves_its_console_between_frames_and_a_reload_drops_what_the_prompt_defined() {
    let comp = two_windows();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), "export {};\n").expect("init.ts");
    let site = root.join("js.sock");
    let mut config = JsRoleConfig::new(&root, None);
    config.watch = false;
    config.console = JsConsoleSite::At(site.clone());
    let role = std::cell::RefCell::new(
        JsRole::over_link(Box::new(FakeLink(comp.clone())), JsRoleLinks::default(), config).expect("the role boots"),
    );
    let mut prompt = Prompt::connect(&site);
    let mut serve = || role.borrow_mut().tick().expect("a frame");

    assert_eq!(
        value_of(&prompt.ask("globalThis.mark = 'set at the prompt'", &mut serve)),
        &json!("set at the prompt")
    );
    assert_eq!(value_of(&prompt.ask("mark", &mut serve)), &json!("set at the prompt"));
    assert_eq!(
        value_of(&prompt.ask("gnoblin.clients.at(0).y = 55", &mut serve)),
        &json!(55)
    );
    let wrote = sent(&comp).into_iter().flatten().any(
        |op| matches!(op, TxOp::Set { node: 3, ref key, ref value, .. } if key == "y" && *value == MirrorValue::Double(55.0)),
    );
    assert!(
        wrote,
        "the write the prompt made left in the frame that followed it: {:?}",
        sent(&comp)
    );

    let report = role.borrow_mut().reload_now();
    assert!(report.ok, "{:?}", report.errors);
    let after = prompt.ask("typeof mark", &mut serve);
    assert_eq!(
        value_of(&after),
        &json!("undefined"),
        "a reload starts a new generation, so the prompt's variables are gone"
    );
    assert_eq!(
        value_of(&prompt.ask("gnoblin.clients.length", &mut serve)),
        &json!(2),
        "the console follows the new generation"
    );
}

#[test]
fn two_shells_do_not_share_a_console_and_the_socket_goes_with_its_server() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("js.sock");
    let first = JsConsole::bind(&path).expect("the first listens");
    assert!(JsConsole::bind(&path).is_err(), "a live console is not replaced");
    drop(first);
    assert!(
        UnixStream::connect(&path).is_err(),
        "nothing answers once the server is gone"
    );
}
