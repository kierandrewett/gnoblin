//! `defineConfig` document settings, `on` and `setup`, and `gnoblin.dialogs`, over a fake compositor.
//!
//! The tree is a trimmed copy of the fake of `tests/workspaces.rs`. The configuration document and the slot
//! connection are fakes too: `FakeDoc` keeps the document the compositor would hold and every commit it was sent,
//! and `FakeSlots` records every call the shell makes on the slot link and delivers the offers a test queues.
//! The same behaviour against the real compositor is in `crates/gnoblin/tests/js_shell.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_js::{
    JsConfigCommit, JsConfigLink, JsLinkError, JsRole, JsRoleConfig, JsRoleLinks, JsShell, JsShellConfig, JsSpawnError,
    JsSpawnRequest,
};
use gnoblin_tree_client::slot_link::{SlotEvent, SlotLink};
use gnoblin_tree_client::tree_mirror::{
    Attached, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeIncoming, TreeLink, TreeTx,
    TxOp, TxReply,
};
use tempfile::TempDir;

// --- the fake tree ---------------------------------------------------------------------------------------------

struct Node {
    class: String,
    parent: u64,
    props: BTreeMap<String, MirrorValue>,
}

#[derive(Default)]
struct Compositor {
    nodes: BTreeMap<u64, Node>,
    serial: u64,
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
    fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }

    fn external_set(&self, node: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            entry.props.insert(key.to_owned(), value.clone());
        }
        self.publish(vec![set_change(node, key, value)]);
    }

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

/// A root and a primary screen of 1000 by 800.
fn compositor() -> Comp {
    let mut c = Compositor::default();
    c.nodes.insert(
        1,
        Node {
            class: "node".into(),
            parent: 0,
            props: BTreeMap::new(),
        },
    );
    c.nodes.insert(
        2,
        Node {
            class: "screen".into(),
            parent: 1,
            props: BTreeMap::from([
                ("width".to_owned(), MirrorValue::Double(1000.0)),
                ("height".to_owned(), MirrorValue::Double(800.0)),
                ("primary".to_owned(), MirrorValue::Bool(true)),
            ]),
        },
    );
    c.serial = 1;
    Rc::new(RefCell::new(c))
}

// --- the fake document -----------------------------------------------------------------------------------------

#[derive(Default)]
struct Doc {
    /// The document in force, as JSON text.
    current: Option<String>,
    /// Every document the shell committed, in order.
    commits: Vec<String>,
    refuse: Option<String>,
    notices: Vec<String>,
}

#[derive(Clone, Default)]
struct FakeDoc(Rc<RefCell<Doc>>);

impl FakeDoc {
    fn with(current: &str) -> Self {
        let doc = Self::default();
        doc.0.borrow_mut().current = Some(current.to_owned());
        doc
    }

    fn commits(&self) -> usize {
        self.0.borrow().commits.len()
    }

    fn current(&self) -> serde_json::Value {
        let text = self.0.borrow().current.clone().unwrap_or_else(|| "{}".to_owned());
        serde_json::from_str(&text).expect("the document is JSON")
    }

    /// Another party (the Lua host's reload) replaces the document.
    fn replace(&self, text: &str) {
        self.0.borrow_mut().current = Some(text.to_owned());
    }
}

impl JsConfigLink for FakeDoc {
    fn current(&mut self) -> Result<Option<Vec<u8>>, JsLinkError> {
        Ok(self.0.borrow().current.clone().map(String::into_bytes))
    }

    fn commit(&mut self, document: &[u8]) -> Result<JsConfigCommit, JsLinkError> {
        let mut doc = self.0.borrow_mut();
        if let Some(reason) = doc.refuse.take() {
            return Err(JsLinkError(reason));
        }
        let text = String::from_utf8(document.to_vec()).expect("a UTF-8 document");
        doc.commits.push(text.clone());
        doc.current = Some(text);
        Ok(JsConfigCommit {
            revision: doc.commits.len() as u64,
            notices: doc.notices.clone(),
        })
    }
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("test JSON")
}

// --- the fake slot connection ----------------------------------------------------------------------------------

#[derive(Default)]
struct Slots {
    /// Every call the shell made, as one line each.
    calls: Vec<String>,
    inbox: Vec<SlotEvent>,
    refuse_claim: Option<String>,
    fail_poll: Option<String>,
}

#[derive(Clone, Default)]
struct FakeSlots(Rc<RefCell<Slots>>);

impl FakeSlots {
    fn offer(&self, request: u64, title: &str, message: &str, subject: &str) {
        self.0.borrow_mut().inbox.push(SlotEvent::Offered {
            request,
            deadline: 250,
            slot: "dialogs.observe".to_owned(),
            title: title.to_owned(),
            message: message.to_owned(),
            subject: subject.as_bytes().to_vec(),
        });
    }

    fn calls(&self) -> Vec<String> {
        self.0.borrow().calls.clone()
    }

    fn took(&self, call: &str) -> usize {
        self.0
            .borrow()
            .calls
            .iter()
            .filter(|seen| seen.as_str() == call)
            .count()
    }
}

impl SlotLink for FakeSlots {
    fn claim_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        let mut slots = self.0.borrow_mut();
        slots.calls.push(format!("claim_slot {slot}"));
        match slots.refuse_claim.clone() {
            Some(reason) => Err(LinkError(reason)),
            None => Ok(()),
        }
    }

    fn release_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        self.0.borrow_mut().calls.push(format!("release_slot {slot}"));
        Ok(())
    }

    fn claim_request(&mut self, request: u64) -> Result<(), LinkError> {
        self.0.borrow_mut().calls.push(format!("claim_request {request}"));
        Ok(())
    }

    fn answer(&mut self, request: u64, answer: &[u8]) -> Result<(), LinkError> {
        let text = String::from_utf8_lossy(answer).into_owned();
        self.0.borrow_mut().calls.push(format!("answer {request} {text}"));
        Ok(())
    }

    fn decline(&mut self, request: u64, _reason: Option<gnoblin_model::DeclineReason>) -> Result<(), LinkError> {
        self.0.borrow_mut().calls.push(format!("decline {request}"));
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<SlotEvent>, LinkError> {
        let mut slots = self.0.borrow_mut();
        if let Some(reason) = slots.fail_poll.take() {
            return Err(LinkError(reason));
        }
        Ok(std::mem::take(&mut slots.inbox))
    }
}

// --- rigs ------------------------------------------------------------------------------------------------------

struct Rig {
    shell: JsShell,
    link: FakeLink,
    doc: FakeDoc,
    slots: FakeSlots,
    _dir: TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// Frames until the compositor's echo of the last transaction has been taken in.
    fn settle(&mut self) -> Vec<String> {
        let mut all = Vec::new();
        for _ in 0..3 {
            all.extend(self.frame());
        }
        all
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }
}

fn shell_config(root: &Path) -> JsShellConfig {
    JsShellConfig::new(root)
}

/// A shell booted over the fakes with `init` as its entry. `document` is what the compositor holds already.
fn rig_with(init: &str, document: Option<&str>, tune: impl FnOnce(&mut JsShellConfig)) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), init).expect("init.ts is written");
    let comp = compositor();
    let link = FakeLink(comp);
    let doc = document.map_or_else(FakeDoc::default, FakeDoc::with);
    let slots = FakeSlots::default();
    let mut config = shell_config(&root);
    tune(&mut config);
    let mut shell = JsShell::new(Box::new(link.clone()), config).expect("the shell boots");
    shell.set_config_link(Box::new(doc.clone()));
    shell.set_slot_link(Box::new(slots.clone()));
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Rig {
        shell,
        link,
        doc,
        slots,
        _dir: dir,
    }
}

fn rig(init: &str) -> Rig {
    rig_with(init, None, |_| {})
}

const PRELUDE: &str = "import { defineConfig, DialogListener, ClientListener } from \"gnoblin\";\n\
                       const g = globalThis as any;\n\
                       g.g = g;\n\
                       g.log = [];\n";

fn init(body: &str) -> String {
    format!("{PRELUDE}{body}\n")
}

// --- document settings -----------------------------------------------------------------------------------------

#[test]
fn a_document_setting_is_laid_over_the_document_in_force_and_committed_whole() {
    let rig = rig_with(
        &init("defineConfig({ dialogs: { style: { radius: 12 } } });"),
        Some(r#"{"appearance":{"reduced-motion":true}}"#),
        |_| {},
    );
    assert_eq!(rig.doc.commits(), 1, "one commit");
    assert_eq!(
        rig.doc.current(),
        json(r#"{"appearance":{"reduced-motion":true},"dialogs":{"style":{"radius":12}}}"#),
        "the document is the one in force with the patch on it, not the patch alone"
    );
}

#[test]
fn keys_are_camel_case_in_a_config_and_kebab_case_in_the_document() {
    let rig = rig(&init(
        "defineConfig({\n\
         \x20   dialogs: { style: { buttonLayout: 'stretch', font: { size: 14 } } },\n\
         \x20   appearance: { colorScheme: 'prefer-dark', reducedMotion: true },\n\
         });",
    ));
    assert_eq!(
        rig.doc.current(),
        json(
            r#"{"dialogs":{"style":{"button-layout":"stretch","font":{"size":14}}},
                "appearance":{"color-scheme":"prefer-dark","reduced-motion":true}}"#
        )
    );
}

#[test]
fn a_config_with_no_document_setting_commits_nothing() {
    let rig = rig_with(
        &init("defineConfig({ workspaces: [{ id: 'dev', layout: 'floating' }] });"),
        Some(r#"{"appearance":{"reduced-motion":true}}"#),
        |_| {},
    );
    assert_eq!(rig.doc.commits(), 0);
}

#[test]
fn a_refused_value_fails_the_config_with_the_key_path_and_commits_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        init("defineConfig({ dialogs: { style: { radius: 5000 } } });"),
    )
    .expect("init.ts");
    let doc = FakeDoc::default();
    let role = JsRole::over_link(
        Box::new(FakeLink(compositor())),
        JsRoleLinks {
            slots: None,
            config: Some(Box::new(doc.clone())),
            ..JsRoleLinks::default()
        },
        {
            let mut config = JsRoleConfig::new(&root, None);
            config.watch = false;
            config
        },
    )
    .expect("a role boots even when its entry is refused");
    assert_eq!(doc.commits(), 0, "nothing the validator refused was committed");
    let report = role
        .diagnostics()
        .iter()
        .find(|text| text.contains("dialogs.style.radius"))
        .cloned();
    assert!(
        report.is_some_and(|text| text.contains("defineConfig")),
        "the refusal names the key path and where it came from: {:?}",
        role.diagnostics()
    );
}

/// A role over the fakes with `first` as the entry file. Its `init.ts` is rewritten by `rewrite`.
struct RoleRig {
    role: JsRole,
    doc: FakeDoc,
    root: std::path::PathBuf,
    _dir: TempDir,
}

fn role_rig(first: &str, document: Option<&str>) -> RoleRig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), first).expect("init.ts");
    let doc = document.map_or_else(FakeDoc::default, FakeDoc::with);
    let mut config = JsRoleConfig::new(&root, None);
    config.watch = false;
    let role = JsRole::over_link(
        Box::new(FakeLink(compositor())),
        JsRoleLinks {
            slots: None,
            config: Some(Box::new(doc.clone())),
            ..JsRoleLinks::default()
        },
        config,
    )
    .expect("a role");
    RoleRig {
        role,
        doc,
        root,
        _dir: dir,
    }
}

impl RoleRig {
    fn rewrite(&mut self, source: &str) -> gnoblin_js::ReloadReport {
        fs::write(self.root.join("init.ts"), source).expect("init.ts is rewritten");
        self.role.reload_now()
    }
}

#[test]
fn a_reload_with_a_refused_value_is_reported_with_the_key_path_and_the_running_generation_stays() {
    let mut rig = role_rig(&init("defineConfig({ dialogs: { style: { radius: 12 } } });"), None);
    let generation = rig.role.generation_id();
    assert_eq!(rig.doc.commits(), 1);

    let report = rig.rewrite(&init("defineConfig({ dialogs: { style: { radius: 5000 } } });"));
    assert!(!report.ok, "the refused document fails the reload");
    let errors: Vec<String> = report.errors.iter().map(ToString::to_string).collect();
    assert!(
        errors.iter().any(|error| error.contains("dialogs.style.radius")),
        "the report carries the key path: {errors:?}"
    );
    assert_eq!(rig.role.generation_id(), generation, "the running generation stays");
    assert_eq!(rig.doc.commits(), 1, "nothing was committed for the refused reload");
    assert_eq!(rig.doc.current(), json(r#"{"dialogs":{"style":{"radius":12}}}"#));
}

#[test]
fn a_reload_that_drops_a_key_puts_it_back_to_its_default_and_keeps_the_rest() {
    let mut rig = role_rig(
        &init("defineConfig({ dialogs: { style: { radius: 12, scale: 2 } }, appearance: { reducedMotion: true } });"),
        Some(r#"{"cursor":{"size":32}}"#),
    );
    assert_eq!(
        rig.doc.current(),
        json(
            r#"{"cursor":{"size":32},"appearance":{"reduced-motion":true},
                "dialogs":{"style":{"radius":12,"scale":2}}}"#
        )
    );

    let report = rig.rewrite(&init("defineConfig({ dialogs: { style: { scale: 3 } } });"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32},"dialogs":{"style":{"scale":3}}}"#),
        "radius and the appearance section are gone, scale moved, the setting the shell never made stays"
    );

    let report = rig.rewrite(&init("defineConfig({});"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32}}"#),
        "with no setting left the sections are gone, not left empty"
    );
    assert_eq!(rig.doc.commits(), 3);
}

#[test]
fn a_reload_that_changes_nothing_commits_nothing() {
    let mut rig = role_rig(&init("defineConfig({ dialogs: { style: { radius: 12 } } });"), None);
    assert_eq!(rig.doc.commits(), 1);
    let report = rig.rewrite(&init(
        "defineConfig({ dialogs: { style: { radius: 12 } } });\n// edited",
    ));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(rig.doc.commits(), 1, "the document already says it");
}

#[test]
fn settings_return_after_another_commit_of_the_document() {
    let mut rig = role_rig(&init("defineConfig({ dialogs: { style: { radius: 12 } } });"), None);
    // The Lua host's own reload commits a document of its own, which has none of the shell's settings.
    rig.doc.replace(r#"{"cursor":{"size":24}}"#);
    rig.role.config_committed();
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":24},"dialogs":{"style":{"radius":12}}}"#),
        "the settings go back on the document the other party committed, which is otherwise kept"
    );
}

#[test]
fn a_document_the_compositor_refuses_is_a_diagnostic_and_the_shell_keeps_running() {
    let rig = {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
        fs::write(
            root.join("init.ts"),
            init("defineConfig({ dialogs: { style: { radius: 12 } } });"),
        )
        .expect("init");
        let doc = FakeDoc::default();
        doc.0.borrow_mut().refuse = Some("config-rejected: no".to_owned());
        let mut config = JsRoleConfig::new(&root, None);
        config.watch = false;
        let role = JsRole::over_link(
            Box::new(FakeLink(compositor())),
            JsRoleLinks {
                slots: None,
                config: Some(Box::new(doc.clone())),
                ..JsRoleLinks::default()
            },
            config,
        )
        .expect("a role");
        (role, doc, dir)
    };
    let (role, doc, _dir) = rig;
    assert_eq!(doc.commits(), 0);
    assert!(
        role.diagnostics()
            .iter()
            .any(|text| text.contains("not committed") && text.contains("config-rejected: no")),
        "{:?}",
        role.diagnostics()
    );
}

#[test]
fn compositor_notices_are_reported_as_not_applied() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        init("defineConfig({ dialogs: { style: { radius: 12 } } });"),
    )
    .expect("init");
    let doc = FakeDoc::default();
    doc.0.borrow_mut().notices = vec!["compositor.renderer: restart required".to_owned()];
    let mut shell = JsShell::new(Box::new(FakeLink(compositor())), shell_config(&root)).expect("the shell boots");
    shell.set_config_link(Box::new(doc.clone()));
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert_eq!(
        diagnostics,
        vec!["not applied: compositor.renderer: restart required".to_owned()]
    );
}

#[test]
fn without_a_configuration_connection_document_settings_are_reported_not_committed() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        init("defineConfig({ dialogs: { style: { radius: 12 } } });"),
    )
    .expect("init");
    let mut shell = JsShell::new(Box::new(FakeLink(compositor())), shell_config(&root)).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].contains("no configuration connection"),
        "{diagnostics:?}"
    );
}

// --- on and setup ----------------------------------------------------------------------------------------------

#[test]
fn on_handlers_are_listeners_of_the_clients() {
    let mut rig = rig(&init(
        "defineConfig({ on: {\n\
         \x20   clientAdded(client: any) { g.log.push('added:' + client.title); },\n\
         \x20   clientRemoved(client: any) { g.log.push('removed:' + client.title); },\n\
         \x20   titleChanged(client: any, title: string) { g.log.push('title:' + title); },\n\
         \x20   focusChanged(client: any, focused: boolean) { g.log.push('focus:' + focused); },\n\
         } });",
    ));
    rig.link.create_window(3, "term");
    assert!(rig.settle().is_empty());
    assert_eq!(rig.eval("g.log.join(',')"), "added:term");
    rig.link.external_set(3, "title", MirrorValue::Text("shell".into()));
    rig.link.external_set(3, "focused", MirrorValue::Bool(true));
    assert!(rig.settle().is_empty());
    assert_eq!(rig.eval("g.log.join(',')"), "added:term,title:shell,focus:true");
    rig.link.destroy(3);
    assert!(rig.settle().is_empty());
    assert!(
        rig.eval("g.log.join(',')").ends_with(",removed:shell"),
        "{}",
        rig.eval("g.log.join(',')")
    );
}

#[test]
fn an_on_handler_that_throws_is_reported_once_and_the_next_still_runs() {
    let mut rig = rig(&init(
        "defineConfig({ on: {\n\
         \x20   titleChanged() { throw new Error('boom'); },\n\
         \x20   focusChanged(client: any, focused: boolean) { g.log.push('focus:' + focused); },\n\
         } });",
    ));
    rig.link.create_window(3, "term");
    rig.settle();
    rig.link.external_set(3, "title", MirrorValue::Text("a".into()));
    rig.link.external_set(3, "focused", MirrorValue::Bool(true));
    let first = rig.settle();
    rig.link.external_set(3, "title", MirrorValue::Text("b".into()));
    let second = rig.settle();
    assert_eq!(
        first.iter().filter(|text| text.contains("boom")).count(),
        1,
        "{first:?}"
    );
    assert!(second.is_empty(), "logged once, not once per event: {second:?}");
    assert_eq!(rig.eval("g.log.join(',')"), "focus:true");
}

#[test]
fn setup_runs_once_after_the_declarative_part_with_the_object_api() {
    let mut rig = rig(&init(
        "defineConfig({\n\
         \x20   workspaces: [{ id: 'dev', layout: 'floating' }],\n\
         \x20   setup(gnoblin: any) {\n\
         \x20       g.log.push('setup:' + gnoblin.workspaces.length + ':' + (gnoblin === (globalThis as any).gnoblin));\n\
         \x20   },\n\
         });",
    ));
    assert_eq!(
        rig.eval("g.log.join(',')"),
        "setup:1:true",
        "setup saw the workspace the declarative part made, and ran once"
    );
    rig.settle();
    assert_eq!(rig.eval("g.log.length"), "1");
}

#[test]
fn a_setup_that_throws_fails_the_config_like_any_init_error() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        init("defineConfig({ setup() { throw new Error('nope'); } });"),
    )
    .expect("init");
    let mut shell = JsShell::new(Box::new(FakeLink(compositor())), shell_config(&root)).expect("the shell boots");
    let error = shell.load_entry(Path::new("init.ts")).expect_err("setup threw");
    assert!(error.to_string().contains("nope"), "{error}");
    assert!(error.to_string().contains("init.ts"), "{error}");
}

// --- gnoblin.dialogs -------------------------------------------------------------------------------------------

#[test]
fn the_slot_is_claimed_while_a_listener_is_attached_and_not_before() {
    let mut rig = rig(&init("g.l = new DialogListener();"));
    rig.settle();
    assert!(
        rig.slots.calls().is_empty(),
        "no listener, no claim: {:?}",
        rig.slots.calls()
    );
    rig.eval("gnoblin.dialogs.addListener(g.l)");
    rig.frame();
    assert_eq!(rig.slots.calls(), vec!["claim_slot dialogs.observe"]);
    rig.frame();
    assert_eq!(
        rig.slots.took("claim_slot dialogs.observe"),
        1,
        "claimed once, not every frame"
    );
    rig.eval("gnoblin.dialogs.removeListener(g.l)");
    rig.frame();
    assert_eq!(rig.slots.took("release_slot dialogs.observe"), 1);
}

const REFUSE_SCREENSHOTS: &str = "class Policy extends DialogListener {\n\
    \x20   onConsentRequest(request: any) {\n\
    \x20       g.log.push([request.kind, request.appId, request.subject, request.deadline].join('|'));\n\
    \x20       if (request.subject === 'Screenshot') { request.decline('screenshots are off'); }\n\
    \x20   }\n\
    \x20   onSecretRequest(request: any) {\n\
    \x20       g.log.push(['secret', request.appId, request.subject, typeof request.allow, typeof request.answer].join('|'));\n\
    \x20   }\n\
    }\n\
    gnoblin.dialogs.addListener(new Policy());\n";

#[test]
fn a_listener_that_declines_a_consent_request_refuses_it_and_one_that_does_not_lets_it_through() {
    let mut rig = rig(&init(REFUSE_SCREENSHOTS));
    rig.frame();
    rig.slots
        .offer(7, "consent:Screenshot", "Take a picture?", "org.example.App");
    rig.slots
        .offer(8, "consent:ScreenCast", "Share the screen?", "org.example.App");
    assert!(rig.frame().is_empty());
    assert_eq!(
        rig.eval("g.log.join(';')"),
        "consent|org.example.App|Screenshot|250;consent|org.example.App|ScreenCast|250"
    );
    let calls = rig.slots.calls();
    assert!(calls.contains(&"answer 7 deny".to_owned()), "{calls:?}");
    assert!(calls.contains(&"answer 8 pass".to_owned()), "{calls:?}");
    for request in [7, 8] {
        let claim = calls
            .iter()
            .position(|call| *call == format!("claim_request {request}"));
        let answer = calls
            .iter()
            .position(|call| call.starts_with(&format!("answer {request} ")));
        assert!(
            claim < answer && claim.is_some(),
            "claimed before it was answered: {calls:?}"
        );
    }
}

#[test]
fn a_secret_request_has_no_way_to_allow_it_or_to_read_a_secret() {
    let mut rig = rig(&init(REFUSE_SCREENSHOTS));
    rig.frame();
    rig.slots
        .offer(9, "secret", "Unlock the keyring", "org.example.Keyring");
    assert!(rig.frame().is_empty());
    assert_eq!(
        rig.eval("g.log.join(';')"),
        "secret|org.example.Keyring|Unlock the keyring|undefined|undefined"
    );
    assert!(rig.slots.calls().contains(&"answer 9 pass".to_owned()));
    // The only words the shell ever answers an observation with are these two.
    for call in rig.slots.calls().iter().filter(|call| call.starts_with("answer ")) {
        assert!(call.ends_with(" pass") || call.ends_with(" deny"), "{call}");
    }
}

#[test]
fn a_late_decline_after_the_window_closed_refuses_nothing() {
    let mut rig = rig(&init(
        "g.kept = [];\n\
         class Keep extends DialogListener {\n\
         \x20   onConsentRequest(request: any) { g.kept.push(request); }\n\
         }\n\
         gnoblin.dialogs.addListener(new Keep());",
    ));
    rig.frame();
    rig.slots.offer(5, "consent:Screenshot", "m", "app");
    rig.frame();
    assert!(rig.slots.calls().contains(&"answer 5 pass".to_owned()));
    rig.eval("g.kept[0].decline('too late')");
    rig.frame();
    assert_eq!(rig.slots.took("answer 5 deny"), 0);
    assert_eq!(rig.slots.took("answer 5 pass"), 1, "answered once");
}

#[test]
fn listener_errors_are_isolated_and_logged_once() {
    let mut rig = rig(&init(
        "class Bad extends DialogListener {\n\
         \x20   onConsentRequest() { throw new Error('first listener broke'); }\n\
         }\n\
         class Good extends DialogListener {\n\
         \x20   onConsentRequest(request: any) { request.decline('policy'); }\n\
         }\n\
         gnoblin.dialogs.addListener(new Bad());\n\
         gnoblin.dialogs.addListener(new Good());",
    ));
    rig.frame();
    rig.slots.offer(1, "consent:Screenshot", "m", "app");
    let first = rig.frame();
    rig.slots.offer(2, "consent:Screenshot", "m", "app");
    let second = rig.frame();
    assert_eq!(
        first
            .iter()
            .filter(|text| text.contains("first listener broke"))
            .count(),
        1,
        "{first:?}"
    );
    assert!(second.is_empty(), "the same failure is not reported again: {second:?}");
    assert_eq!(
        rig.slots.took("answer 1 deny"),
        1,
        "the listener after the broken one still ran"
    );
    assert_eq!(rig.slots.took("answer 2 deny"), 1);
}

#[test]
fn a_listener_that_runs_out_its_budget_is_skipped_and_the_request_passes() {
    let mut rig = rig_with(
        &init(
            "class Spin extends DialogListener {\n\
             \x20   onConsentRequest() { while (true) {} }\n\
             }\n\
             class After extends DialogListener {\n\
             \x20   onConsentRequest(request: any) { g.log.push('after:' + request.subject); }\n\
             }\n\
             gnoblin.dialogs.addListener(new Spin());\n\
             gnoblin.dialogs.addListener(new After());",
        ),
        None,
        |config| config.entry_budget = Duration::from_millis(30),
    );
    rig.frame();
    rig.slots.offer(3, "consent:Screenshot", "m", "app");
    let diagnostics = rig.frame();
    assert!(
        diagnostics
            .iter()
            .any(|text| text.contains("a listener call was stopped")),
        "{diagnostics:?}"
    );
    assert_eq!(rig.shell.strikes(), 1, "the runaway is a strike on the generation");
    assert_eq!(
        rig.eval("g.log.join(',')"),
        "after:Screenshot",
        "the listener after it still ran"
    );
    assert_eq!(
        rig.slots.took("answer 3 pass"),
        1,
        "the built-in dialog is left to serve it"
    );
    assert_eq!(rig.slots.took("answer 3 deny"), 0);
}

#[test]
fn retiring_answers_a_held_request_pass_and_releases_the_slot() {
    // A listener that takes long enough to use the frame's budget leaves the request waiting for the next frame,
    // which is when a swap or a shutdown can find it still held.
    let mut rig = rig_with(
        &init(
            "class Slow extends DialogListener {\n\
             \x20   onConsentRequest() { const until = Date.now() + 25; while (Date.now() < until) {} }\n\
             }\n\
             gnoblin.dialogs.addListener(new Slow());",
        ),
        None,
        |config| config.frame_budget = Duration::from_millis(10),
    );
    rig.frame();
    rig.slots.offer(4, "consent:Screenshot", "m", "app");
    rig.frame();
    assert_eq!(
        rig.slots.took("answer 4 pass"),
        0,
        "the settle call is waiting for the next frame"
    );
    rig.shell.retire().expect("retire");
    let calls = rig.slots.calls();
    let tail: Vec<&str> = calls.iter().rev().take(3).map(String::as_str).rev().collect();
    assert_eq!(
        tail,
        vec!["claim_request 4", "answer 4 pass", "release_slot dialogs.observe"],
        "{calls:?}"
    );
}

#[test]
fn a_slot_the_compositor_refuses_is_reported_once_and_the_shell_runs_on() {
    let mut rig = rig(&init("gnoblin.dialogs.addListener(new DialogListener());"));
    rig.slots.0.borrow_mut().refuse_claim = Some("slot-held: another connection holds it".to_owned());
    let first = rig.frame();
    let second = rig.frame();
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(
        first[0].contains("dialogs.observe") && first[0].contains("slot-held"),
        "{first:?}"
    );
    assert!(second.is_empty(), "{second:?}");
    assert_eq!(
        rig.slots.took("claim_slot dialogs.observe"),
        1,
        "not retried every frame"
    );
}

#[test]
fn a_lost_slot_connection_is_reported_and_listeners_stop_hearing_of_requests() {
    let mut rig = rig(&init(REFUSE_SCREENSHOTS));
    rig.frame();
    rig.slots.0.borrow_mut().fail_poll = Some("broken pipe".to_owned());
    let diagnostics = rig.frame();
    assert!(
        diagnostics.iter().any(|text| text.contains("slot connection is lost")),
        "{diagnostics:?}"
    );
    rig.slots.offer(6, "consent:Screenshot", "m", "app");
    assert!(rig.frame().is_empty());
    assert_eq!(
        rig.eval("g.log.length"),
        "0",
        "nothing reaches a listener once the link is gone"
    );
}

#[test]
fn without_a_slot_connection_a_listener_is_reported_once() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        init("gnoblin.dialogs.addListener(new DialogListener());"),
    )
    .expect("init");
    let mut shell = JsShell::new(Box::new(FakeLink(compositor())), shell_config(&root)).expect("the shell boots");
    shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    let first = shell.frame().expect("a frame").diagnostics;
    let second = shell.frame().expect("a frame").diagnostics;
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("no slot connection"), "{first:?}");
    assert!(second.is_empty());
}

// --- the list and map keys -------------------------------------------------------------------------------------

/// The errors of a reload, one text each.
fn errors_of(report: &gnoblin_js::ReloadReport) -> Vec<String> {
    report.errors.iter().map(ToString::to_string).collect()
}

#[test]
fn protocols_are_added_changed_and_put_back_to_their_default_by_a_reload() {
    let mut rig = role_rig(
        &init("defineConfig({ protocols: { wlrLayerShell: false } });"),
        Some(r#"{"cursor":{"size":32}}"#),
    );
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32},"protocols":{"wlr-layer-shell":false}}"#),
        "added: the kebab-case table of the document, on top of what was there"
    );

    let report = rig.rewrite(&init(
        "defineConfig({ protocols: { extIdleNotify: false, extBackgroundEffectV1: true } });",
    ));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32},"protocols":{"ext-idle-notify":false,"ext-background-effect-v1":true}}"#),
        "changed: the protocol the new config leaves out is back to on"
    );

    let report = rig.rewrite(&init("defineConfig({});"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32}}"#),
        "removed: the section goes and every protocol is advertised again"
    );
}

#[test]
fn a_protocol_value_or_name_the_validator_refuses_fails_the_reload_with_the_key_path() {
    let mut rig = role_rig(&init("defineConfig({ protocols: { wlrLayerShell: false } });"), None);
    let generation = rig.role.generation_id();
    let commits = rig.doc.commits();

    let report = rig.rewrite(&init("defineConfig({ protocols: { wlrLayerShell: 'off' } });"));
    assert!(!report.ok);
    assert!(
        errors_of(&report)
            .iter()
            .any(|error| error.contains("protocols.wlr-layer-shell")),
        "{:?}",
        errors_of(&report)
    );

    let report = rig.rewrite(&init("defineConfig({ protocols: { wlrLayerShel: false } });"));
    assert!(!report.ok);
    assert!(
        errors_of(&report).iter().any(|error| error.contains("wlr-layer-shel")),
        "{:?}",
        errors_of(&report)
    );
    assert_eq!(rig.role.generation_id(), generation, "the running generation stays");
    assert_eq!(rig.doc.commits(), commits, "nothing was committed for a refused reload");
    assert_eq!(rig.doc.current(), json(r#"{"protocols":{"wlr-layer-shell":false}}"#));
}

const GESTURE: &str = "{ name: 'launcher', gesture: 'pinch', fingers: 4, direction: 'out', command: ['fuzzel'] }";

#[test]
fn touchpad_gestures_are_added_changed_and_put_back_to_their_default_by_a_reload() {
    let mut rig = role_rig(
        &init(&format!("defineConfig({{ touchpadGestures: [{GESTURE}] }});")),
        Some(r#"{"cursor":{"size":32}}"#),
    );
    assert_eq!(
        rig.doc.current(),
        json(
            r#"{"cursor":{"size":32},"touchpad-gestures":[
                {"name":"launcher","gesture":"pinch","fingers":4,"direction":"out","command":["fuzzel"]}]}"#
        ),
        "added: the list is one value of the document"
    );

    let changed = GESTURE.replace("fingers: 4", "fingers: 3");
    let report = rig.rewrite(&init(&format!("defineConfig({{ touchpadGestures: [{changed}] }});")));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current()["touchpad-gestures"][0]["fingers"],
        json("3"),
        "changed: a new list replaces the old one whole"
    );

    let report = rig.rewrite(&init("defineConfig({});"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32}}"#),
        "removed: no gesture is left"
    );
}

#[test]
fn a_gesture_the_validator_refuses_fails_the_reload_with_the_key_path() {
    let mut rig = role_rig(
        &init(&format!("defineConfig({{ touchpadGestures: [{GESTURE}] }});")),
        None,
    );
    let commits = rig.doc.commits();
    let report = rig.rewrite(&init(&format!(
        "defineConfig({{ touchpadGestures: [{}] }});",
        GESTURE.replace("fingers: 4", "fingers: 9")
    )));
    assert!(!report.ok);
    let errors = errors_of(&report);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("touchpad-gestures") && error.contains("fingers")),
        "{errors:?}"
    );
    assert_eq!(rig.doc.commits(), commits);
}

/// The commands a shell started, as `program arg ...`, and the programs it is to refuse.
#[derive(Clone, Default)]
struct Launches {
    started: Rc<RefCell<Vec<String>>>,
    refuse: Rc<RefCell<Vec<String>>>,
}

impl Launches {
    fn attach(&self, role: &mut JsRole) {
        let (started, refuse) = (Rc::clone(&self.started), Rc::clone(&self.refuse));
        role.shell_mut().set_spawner(Box::new(move |request| {
            let JsSpawnRequest::Argv(argv) = request else {
                return Err(JsSpawnError("autostart starts argv lists".to_owned()));
            };
            if refuse.borrow().contains(&argv[0]) {
                return Err(JsSpawnError(format!("cannot start {}: not found", argv[0])));
            }
            started.borrow_mut().push(argv.join(" "));
            Ok(())
        }));
    }

    fn started(&self) -> Vec<String> {
        self.started.borrow().clone()
    }
}

#[test]
fn autostart_commands_start_once_and_a_reload_starts_only_the_ones_it_adds() {
    let mut rig = role_rig(&init("defineConfig({});"), None);
    let launches = Launches::default();
    launches.attach(&mut rig.role);

    let report = rig.rewrite(&init("defineConfig({ autostart: ['waybar', ['foot', '--server']] });"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        launches.started(),
        ["waybar", "foot --server"],
        "added: each starts, in order"
    );

    let report = rig.rewrite(&init(
        "defineConfig({ autostart: ['waybar', ['foot', '--server']] });\n// edited",
    ));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(launches.started().len(), 2, "a reload does not start a command again");

    let report = rig.rewrite(&init(
        "defineConfig({ autostart: ['waybar', ['foot', '--server'], 'nm-applet'] });",
    ));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        launches.started(),
        ["waybar", "foot --server", "nm-applet"],
        "changed: only the entry the reload adds starts"
    );

    let report = rig.rewrite(&init("defineConfig({});"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        launches.started().len(),
        3,
        "removed: nothing starts and nothing is stopped"
    );

    let report = rig.rewrite(&init("defineConfig({ autostart: ['waybar'] });"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        launches.started().len(),
        3,
        "a command that was started this session is not started again when it is listed again"
    );
}

#[test]
fn a_command_is_known_by_its_arguments_not_only_its_program() {
    let mut rig = role_rig(&init("defineConfig({ autostart: [] });"), None);
    let launches = Launches::default();
    launches.attach(&mut rig.role);
    rig.rewrite(&init("defineConfig({ autostart: [['foot', '--server']] });"));
    rig.rewrite(&init("defineConfig({ autostart: [['foot', '--server'], 'foot'] });"));
    assert_eq!(launches.started(), ["foot --server", "foot"]);
}

#[test]
fn a_config_that_fails_to_load_starts_nothing() {
    let mut rig = role_rig(&init("defineConfig({});"), None);
    let launches = Launches::default();
    launches.attach(&mut rig.role);
    let report = rig.rewrite(&init(
        "defineConfig({ autostart: ['waybar'] });\nthrow new Error('after the config');",
    ));
    assert!(!report.ok);
    assert!(
        launches.started().is_empty(),
        "the generation never ran, so its commands did not start"
    );

    let report = rig.rewrite(&init("defineConfig({ autostart: [5] });"));
    assert!(!report.ok);
    assert!(
        errors_of(&report)
            .iter()
            .any(|error| error.contains("config.autostart[0] must be a command line or an argv array of strings")),
        "{:?}",
        errors_of(&report)
    );
    assert!(launches.started().is_empty());
}

#[test]
fn a_command_that_cannot_be_started_is_reported_and_tried_again_at_the_next_reload() {
    let mut rig = role_rig(&init("defineConfig({});"), None);
    let launches = Launches::default();
    launches.attach(&mut rig.role);
    launches.refuse.borrow_mut().push("nope".to_owned());

    let report = rig.rewrite(&init("defineConfig({ autostart: ['nope --flag', 'waybar'] });"));
    assert!(
        report.ok,
        "a missing program does not fail the config: {:?}",
        report.errors
    );
    assert_eq!(launches.started(), ["waybar"], "the others still start");
    assert!(
        rig.role
            .diagnostics()
            .iter()
            .any(|text| text.contains("autostart nope --flag") && text.contains("not found")),
        "{:?}",
        rig.role.diagnostics()
    );

    launches.refuse.borrow_mut().clear();
    let report = rig.rewrite(&init(
        "defineConfig({ autostart: ['nope --flag', 'waybar'] });\n// installed it",
    ));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        launches.started(),
        ["waybar", "nope --flag"],
        "the one that failed starts now; waybar does not repeat"
    );
}

#[test]
fn a_session_that_starts_with_an_entry_starts_its_autostart_commands_at_load() {
    // The first load is not a reload: the role loads the entry and the commands start with it. Nothing can be
    // swapped in before the spawner exists, so the first command goes to the role's own spawner (a real process);
    // the test names a program that does not exist and reads the refusal, which is the proof it was tried.
    let rig = role_rig(
        &init("defineConfig({ autostart: ['/nonexistent/gnoblin-test-autostart'] });"),
        None,
    );
    assert!(
        rig.role
            .diagnostics()
            .iter()
            .any(|text| text.contains("autostart /nonexistent/gnoblin-test-autostart")),
        "{:?}",
        rig.role.diagnostics()
    );
}
