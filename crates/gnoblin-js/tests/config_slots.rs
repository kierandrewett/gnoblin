//! The document settings a script's device objects write, `ClientListener`s on `gnoblin.clients`, and `gnoblin.dialogs`,
//! over a fake compositor.
//!
//! The tree is the shared fake compositor of `tests/common`. The configuration document and the slot connection are
//! fakes too: `FakeDoc` keeps the document the compositor would hold and every commit it was sent, and `FakeSlots`
//! records every call the shell makes on the slot link and delivers the offers a test queues.
//! The same behaviour against the real compositor is in `crates/gnoblin/tests/js_shell.rs`.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_js::{JsConfigCommit, JsConfigLink, JsLinkError, JsRole, JsRoleLinks, JsShell, JsShellConfig};
use gnoblin_tree_client::slot_link::{SlotEvent, SlotLink};
use gnoblin_tree_client::tree_mirror::{LinkError, MirrorValue};
use tempfile::TempDir;

mod common;
use common::{FakeCompositor, ROOT, RUNAWAY_BUDGET, frame_runs, go_quiet, honest_config, honest_role_config, text};

// --- the fake tree ---------------------------------------------------------------------------------------------

type Comp = FakeCompositor;

/// The compositor's script of these tests, over the shared fake: what the compositor does on its own while the shell
/// runs.
#[derive(Clone)]
struct Script(Comp);

impl Script {
    fn external_set(&self, node: u64, key: &str, value: MirrorValue) {
        self.0.external_set(node, key, value);
    }

    /// A window the compositor maps: created, parented under the root, with its title and app id.
    fn create_window(&self, id: u64, title: &str) {
        self.0.create_bare_window(id, title, "app");
    }

    fn destroy(&self, node: u64) {
        self.0.destroy(node);
    }
}

/// A root, a primary screen of 1000 by 800 (2) and a seat (9) with a keyboard and a mouse.
fn compositor() -> Comp {
    let comp = FakeCompositor::new();
    comp.add_node(
        2,
        ROOT,
        "screen",
        vec![
            ("width", MirrorValue::Double(1000.0)),
            ("height", MirrorValue::Double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(
        9,
        ROOT,
        "seat",
        vec![
            ("name", text("seat0")),
            (
                "device",
                MirrorValue::Texts(vec![
                    "1:USB Keyboard:keyboard".to_owned(),
                    "2:Logitech MX Master:pointer".to_owned(),
                ]),
            ),
        ],
    );
    comp
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

    /// Another party (the retired host's reload) replaces the document.
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
    link: Script,
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
    honest_config(root)
}

/// A shell booted over the fakes with `init` as its entry. `document` is what the compositor holds already.
fn rig_with(init: &str, document: Option<&str>, tune: impl FnOnce(&mut JsShellConfig)) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), init).expect("init.ts is written");
    let comp = compositor();
    let link = Script(comp);
    let doc = document.map_or_else(FakeDoc::default, FakeDoc::with);
    let slots = FakeSlots::default();
    let mut config = shell_config(&root);
    tune(&mut config);
    let mut shell = JsShell::new(Box::new(link.0.clone()), config).expect("the shell boots");
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

const PRELUDE: &str = "import { Keyboard, Mouse, Workspace, DialogListener, ClientListener, CollectionListener } from \"gnoblin\";\n\
                       const g = globalThis as any;\n\
                       g.g = g;\n\
                       g.log = [];\n\
                       const mouse = (): any => gnoblin.input.devices.find((device) => device instanceof Mouse);\n\
                       const keyboard = (): any => gnoblin.input.devices.find((device) => device instanceof Keyboard);\n";

fn init(body: &str) -> String {
    format!("{PRELUDE}{body}\n")
}

// --- document settings -----------------------------------------------------------------------------------------

#[test]
fn a_device_setting_is_laid_over_the_document_in_force_and_committed_whole() {
    let rig = rig_with(
        &init("mouse().sensitivity = 0.5;"),
        Some(r#"{"appearance":{"reduced-motion":true}}"#),
        |_| {},
    );
    assert_eq!(rig.doc.commits(), 1, "one commit");
    assert_eq!(
        rig.doc.current(),
        json(r#"{"appearance":{"reduced-motion":true},"input":{"mouse":{"speed":0.5}}}"#),
        "the document is the one in force with the patch on it, not the patch alone"
    );
}

#[test]
fn device_settings_are_camel_case_in_a_script_and_kebab_case_in_the_document() {
    let rig = rig(&init(
        "mouse().acceleration = 'flat'; mouse().naturalScroll = true; keyboard().repeatDelay = 250;",
    ));
    assert_eq!(
        rig.doc.current(),
        json(r#"{"input":{"keyboard":{"delay":250},"mouse":{"accel-profile":"flat","natural-scroll":true}}}"#)
    );
}

#[test]
fn a_script_that_writes_no_document_setting_commits_nothing() {
    let rig = rig_with(
        &init("gnoblin.workspaces.add(new Workspace({ id: 'dev' }));"),
        Some(r#"{"appearance":{"reduced-motion":true}}"#),
        |_| {},
    );
    assert_eq!(rig.doc.commits(), 0);
}

#[test]
fn a_refused_value_fails_the_entry_with_the_device_and_the_setting_and_commits_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), init("mouse().sensitivity = 5000;")).expect("init.ts");
    let doc = FakeDoc::default();
    let role = JsRole::over_link(
        Box::new(compositor()),
        JsRoleLinks {
            slots: None,
            config: Some(Box::new(doc.clone())),
            ..JsRoleLinks::default()
        },
        {
            let mut config = honest_role_config(&root);
            config.watch = false;
            config
        },
    )
    .expect("a role boots even when its entry is refused");
    assert_eq!(doc.commits(), 0, "nothing the write refused was committed");
    assert!(
        role.diagnostics()
            .iter()
            .any(|text| text.contains("Mouse#2.sensitivity")),
        "the refusal names the device and the setting: {:?}",
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
    let mut config = honest_role_config(&root);
    config.watch = false;
    let role = JsRole::over_link(
        Box::new(compositor()),
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
fn a_reload_with_a_refused_value_is_reported_with_the_setting_and_the_running_generation_stays() {
    let mut rig = role_rig(&init("mouse().sensitivity = 0.5;"), None);
    let generation = rig.role.generation_id();
    assert_eq!(rig.doc.commits(), 1);

    let report = rig.rewrite(&init("mouse().sensitivity = 5000;"));
    assert!(!report.ok, "the refused write fails the reload");
    let errors: Vec<String> = report.errors.iter().map(ToString::to_string).collect();
    assert!(
        errors.iter().any(|error| error.contains("Mouse#2.sensitivity")),
        "the report carries the setting: {errors:?}"
    );
    assert_eq!(rig.role.generation_id(), generation, "the running generation stays");
    assert_eq!(rig.doc.commits(), 1, "nothing was committed for the refused reload");
    assert_eq!(rig.doc.current(), json(r#"{"input":{"mouse":{"speed":0.5}}}"#));
}

#[test]
fn a_reload_that_drops_a_setting_puts_it_back_to_its_default_and_keeps_the_rest() {
    let mut rig = role_rig(
        &init("mouse().sensitivity = 0.5; keyboard().repeatDelay = 250;"),
        Some(r#"{"cursor":{"size":32}}"#),
    );
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32},"input":{"keyboard":{"delay":250},"mouse":{"speed":0.5}}}"#)
    );

    let report = rig.rewrite(&init("mouse().sensitivity = 0.25;"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32},"input":{"mouse":{"speed":0.25}}}"#),
        "the keyboard delay is gone, the speed moved, the setting the shell never made stays"
    );

    let report = rig.rewrite(&init("export {};"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":32}}"#),
        "with no setting left the section is gone, not left empty"
    );
    assert_eq!(rig.doc.commits(), 3);
}

#[test]
fn a_reload_that_changes_nothing_commits_nothing() {
    let mut rig = role_rig(&init("mouse().sensitivity = 0.5;"), None);
    assert_eq!(rig.doc.commits(), 1);
    let report = rig.rewrite(&init("mouse().sensitivity = 0.5;\n// edited"));
    assert!(report.ok, "{:?}", report.errors);
    assert_eq!(rig.doc.commits(), 1, "the document already says it");
}

#[test]
fn settings_return_after_another_commit_of_the_document() {
    let mut rig = role_rig(&init("mouse().sensitivity = 0.5;"), None);
    // The retired host's own reload commits a document of its own, which has none of the shell's settings.
    rig.doc.replace(r#"{"cursor":{"size":24}}"#);
    rig.role.config_committed();
    assert_eq!(
        rig.doc.current(),
        json(r#"{"cursor":{"size":24},"input":{"mouse":{"speed":0.5}}}"#),
        "the settings go back on the document the other party committed, which is otherwise kept"
    );
}

#[test]
fn a_document_the_compositor_refuses_is_a_diagnostic_and_the_shell_keeps_running() {
    let rig = {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
        fs::write(root.join("init.ts"), init("mouse().sensitivity = 0.5;")).expect("init");
        let doc = FakeDoc::default();
        doc.0.borrow_mut().refuse = Some("config-rejected: no".to_owned());
        let mut config = honest_role_config(&root);
        config.watch = false;
        let role = JsRole::over_link(
            Box::new(compositor()),
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
    fs::write(root.join("init.ts"), init("mouse().sensitivity = 0.5;")).expect("init");
    let doc = FakeDoc::default();
    doc.0.borrow_mut().notices = vec!["compositor.renderer: restart required".to_owned()];
    let mut shell = JsShell::new(Box::new(compositor()), shell_config(&root)).expect("the shell boots");
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
    fs::write(root.join("init.ts"), init("mouse().sensitivity = 0.5;")).expect("init");
    let mut shell = JsShell::new(Box::new(compositor()), shell_config(&root)).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].contains("no configuration connection"),
        "{diagnostics:?}"
    );
}

// --- clients ---------------------------------------------------------------------------------------------------

/// A listener on the clients that logs the four things the retired `on` table heard.
const LOG_CLIENTS: &str = "const members = new CollectionListener();\n\
    members.onAdded = (client: any) => { g.log.push('added:' + client.title); };\n\
    members.onRemoved = (client: any) => { g.log.push('removed:' + client.title); };\n\
    gnoblin.clients.addListener(members);\n\
    const clients = new ClientListener();\n\
    clients.onTitleChanged = (client: any, title: string) => { g.log.push('title:' + title); };\n\
    clients.onFocusChanged = (client: any, focused: boolean) => { g.log.push('focus:' + focused); };\n\
    gnoblin.clients.addListener(clients);\n";

#[test]
fn listeners_on_the_clients_hear_a_window_come_change_and_go() {
    let mut rig = rig(&init(LOG_CLIENTS));
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
fn a_client_listener_that_throws_is_reported_once_and_the_next_still_runs() {
    let mut rig = rig(&init(
        "const breaks = new ClientListener();\n\
         breaks.onTitleChanged = () => { throw new Error('boom'); };\n\
         gnoblin.clients.addListener(breaks);\n\
         const logs = new ClientListener();\n\
         logs.onFocusChanged = (client: any, focused: boolean) => { g.log.push('focus:' + focused); };\n\
         gnoblin.clients.addListener(logs);",
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
fn an_entry_that_throws_fails_the_load_naming_the_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), init("throw new Error('nope');")).expect("init");
    let mut shell = JsShell::new(Box::new(compositor()), shell_config(&root)).expect("the shell boots");
    let error = shell.load_entry(Path::new("init.ts")).expect_err("the entry threw");
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
        |config| config.entry_budget = RUNAWAY_BUDGET,
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
             \x20   onConsentRequest() { const until = Date.now() + 600; while (Date.now() < until) {} }\n\
             }\n\
             gnoblin.dialogs.addListener(new Slow());",
        ),
        None,
        // The listener outlasts the frame budget by a wide margin, and every other call of every frame uses a small
        // part of it, so no other frame of this test can be the one a busy machine tips over.
        |config| config.frame_budget = Duration::from_millis(300),
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
    let mut shell = JsShell::new(Box::new(compositor()), shell_config(&root)).expect("the shell boots");
    shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    let first = shell.frame().expect("a frame").diagnostics;
    let second = shell.frame().expect("a frame").diagnostics;
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("no slot connection"), "{first:?}");
    assert!(second.is_empty());
}

#[test]
fn a_reload_keeps_the_dialog_slot_for_a_generation_with_a_listener_and_releases_it_for_one_without() {
    // The slot is claimed on the shell's own connection, so a reload that released it and claimed it again would leave
    // the compositor a moment with nobody observing its dialogs. The slot a generation inherits is the one it keeps.
    let slots = FakeSlots::default();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let listening = init("gnoblin.dialogs.addListener(new DialogListener());");
    fs::write(root.join("init.ts"), &listening).expect("init.ts");
    let mut config = honest_role_config(&root);
    config.watch = false;
    let mut role = JsRole::over_link(
        Box::new(compositor()),
        JsRoleLinks {
            slots: Some(Box::new(slots.clone())),
            config: Some(Box::new(FakeDoc::default())),
            ..JsRoleLinks::default()
        },
        config,
    )
    .expect("a role");
    role.tick().expect("a frame");
    assert_eq!(slots.calls(), vec!["claim_slot dialogs.observe"]);

    // The same listener in the next generation: the claim stays, and nothing is sent.
    fs::write(root.join("init.ts"), format!("{listening}// touched\n")).expect("init.ts");
    assert!(role.reload_now().ok);
    role.tick().expect("a frame");
    role.tick().expect("a frame");
    assert_eq!(
        slots.calls(),
        vec!["claim_slot dialogs.observe"],
        "a reload that keeps its listener neither releases the slot nor claims it again"
    );

    // A generation with no listener gives the slot up in its first frame.
    fs::write(root.join("init.ts"), init("")).expect("init.ts");
    assert!(role.reload_now().ok);
    role.tick().expect("a frame");
    assert_eq!(
        slots.calls(),
        vec!["claim_slot dialogs.observe", "release_slot dialogs.observe"]
    );

    // And a later one with a listener claims it afresh.
    fs::write(root.join("init.ts"), &listening).expect("init.ts");
    assert!(role.reload_now().ok);
    role.tick().expect("a frame");
    assert_eq!(
        slots.calls(),
        vec![
            "claim_slot dialogs.observe",
            "release_slot dialogs.observe",
            "claim_slot dialogs.observe"
        ]
    );
}

// --- change gating -----------------------------------------------------------------------------------------------
//
// A quiet turn runs no frame (`tests/frame_gating.rs`). The compositor waits a few hundred milliseconds for the shell's
// verdict on an offer, so an offer has to wake the very next turn and be answered by it.

#[test]
fn an_offer_wakes_the_next_frame_and_is_answered_by_it() {
    let mut rig = rig(&init(REFUSE_SCREENSHOTS));
    go_quiet(&mut rig.shell);
    for _ in 0..25 {
        assert!(!frame_runs(&mut rig.shell), "an idle slot woke a frame");
    }
    rig.slots
        .offer(7, "consent:Screenshot", "Take a picture?", "org.example.App");
    assert!(frame_runs(&mut rig.shell), "the offer did not wake a frame");
    assert!(
        rig.slots.calls().contains(&"answer 7 deny".to_owned()),
        "the verdict went back in the frame the offer woke: {:?}",
        rig.slots.calls()
    );
    go_quiet(&mut rig.shell);
}

#[test]
fn a_lost_slot_connection_wakes_the_next_frame() {
    let mut rig = rig(&init(REFUSE_SCREENSHOTS));
    go_quiet(&mut rig.shell);
    rig.slots.0.borrow_mut().fail_poll = Some("broken pipe".to_owned());
    let before = rig.shell.frame_stats().run;
    let diagnostics = rig.frame();
    assert!(rig.shell.frame_stats().run > before, "the loss did not wake a frame");
    assert!(
        diagnostics.iter().any(|text| text.contains("slot connection is lost")),
        "the frame it woke reported it: {diagnostics:?}"
    );
    go_quiet(&mut rig.shell);
}

#[test]
fn the_end_of_a_request_wakes_the_next_frame_and_closes_it_for_the_listener() {
    let mut rig = rig(&init(
        "g.kept = [];\n\
         class Keep extends DialogListener {\n\
         \x20   onConsentRequest(request: any) { g.kept.push(request); }\n\
         }\n\
         gnoblin.dialogs.addListener(new Keep());",
    ));
    go_quiet(&mut rig.shell);
    rig.slots.offer(5, "consent:Screenshot", "m", "app");
    assert!(frame_runs(&mut rig.shell));
    go_quiet(&mut rig.shell);
    rig.slots.0.borrow_mut().inbox.push(SlotEvent::Finished {
        request: 5,
        completion: "ended",
    });
    assert!(
        frame_runs(&mut rig.shell),
        "the end of the request did not wake a frame"
    );
    rig.eval("g.kept[0].decline('too late')");
    go_quiet(&mut rig.shell);
    assert_eq!(
        rig.slots.took("answer 5 deny"),
        0,
        "the frame the end woke closed the request, so a decline after it refuses nothing"
    );
}
