//! The stock lock screen (`data/js/gnoblin/lock.ts`) over a fake compositor.
//!
//! The cases are the Lua lock screen's (`crates/gnoblin-lua/tests/lua/ui_test.lua`, the cases named `lock`), run on
//! the JavaScript shell: the screen engages when the session locks, waits for a screen to cover, shows itself to a shell
//! that attaches to a locked session, comes down on an unlock it did not make and goes up again on a later lock, sends
//! the typed password to the compositor as the call's secret text and never as an argument, waits for the answer, comes
//! down on `ok`, and on a wrong or rate-limited attempt stays up and says why. The cases the Lua screen had no test
//! for: the clock, the keyboard focus it takes and gives back, an Enter that is not an attempt, a key typed after the
//! unlock, and the password never being drawn, stored in the tree or kept in the entry once it is sent.
//!
//! `Compositor` is an in-memory `TreeLink` that applies the ops the way the real one does, answers nothing by itself
//! (a test answers a password check under its token) and keeps what a real compositor would hold. What it cannot show
//! (the cover, the keys the compositor routes, PAM) is `crates/gnoblin/tests/js_lock.rs`'s.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use gnoblin_js::{JsShell, JsShellConfig};
use gnoblin_paint::{PaintOp, decode};
use gnoblin_tree_client::tree_mirror::{
    Attached, EventFields, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeEvent,
    TreeIncoming, TreeLink, TreeTx, TxOp, TxReply,
};
use tempfile::TempDir;

const ROOT: u64 = 1;
const LOCK: u64 = 2;
const SEAT: u64 = 3;
const SCREEN: u64 = 4;
const WINDOW: u64 = 5;

// ---- the fake compositor ------------------------------------------------------------------------------------------

struct Node {
    class: String,
    parent: u64,
    children: Vec<u64>,
    props: BTreeMap<String, MirrorValue>,
}

/// A call the shell sent, with the secret text it carried.
#[derive(Clone, Debug, PartialEq)]
struct Call {
    node: u64,
    method: String,
    args: Vec<u8>,
    token: u64,
    text: String,
}

struct Compositor {
    nodes: BTreeMap<u64, Node>,
    serial: u64,
    sent: Vec<Vec<TxOp>>,
    calls: Vec<Call>,
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

fn node(class: &str, parent: u64, props: Vec<(&str, MirrorValue)>) -> Node {
    Node {
        class: class.to_owned(),
        parent,
        children: Vec::new(),
        props: props.into_iter().map(|(key, value)| (key.to_owned(), value)).collect(),
    }
}

impl Compositor {
    fn subtree(&self, start: u64) -> Vec<u64> {
        let mut out = vec![start];
        let mut at = 0;
        while at < out.len() {
            if let Some(entry) = self.nodes.get(&out[at]) {
                out.extend(entry.children.iter().copied());
            }
            at += 1;
        }
        out
    }

    fn detach(&mut self, id: u64) {
        let parent = self.nodes.get(&id).map_or(0, |entry| entry.parent);
        if let Some(entry) = self.nodes.get_mut(&parent) {
            entry.children.retain(|child| *child != id);
        }
    }

    fn apply(&mut self, op: &TxOp) -> Option<MirrorChange> {
        match op {
            TxOp::New { id, class } => {
                self.nodes.insert(*id, node(class, 0, Vec::new()));
                Some(MirrorChange {
                    class: class.clone(),
                    ..change(MirrorChangeKind::Created, *id)
                })
            }
            TxOp::Insert {
                node: id,
                parent,
                index,
            } => {
                self.detach(*id);
                let held = self.nodes.get_mut(parent)?;
                let at = usize::try_from(*index).map_or(held.children.len(), |i| i.min(held.children.len()));
                held.children.insert(at, *id);
                self.nodes.get_mut(id)?.parent = *parent;
                Some(MirrorChange {
                    parent: *parent,
                    index: at as i64,
                    ..change(MirrorChangeKind::Parented, *id)
                })
            }
            TxOp::Set {
                node: id, key, value, ..
            } => {
                self.nodes.get_mut(id)?.props.insert(key.clone(), value.clone());
                Some(set_change(*id, key, value.clone()))
            }
            TxOp::Destroy { node: id } => {
                let removed = self.subtree(*id);
                self.detach(*id);
                for gone in &removed {
                    self.nodes.remove(gone);
                }
                Some(MirrorChange {
                    removed,
                    ..change(MirrorChangeKind::Destroyed, *id)
                })
            }
            TxOp::Call {
                node: id,
                method,
                args,
                token,
                text,
            } => {
                self.calls.push(Call {
                    node: *id,
                    method: method.clone(),
                    args: args.clone(),
                    token: *token,
                    text: text.expose().to_owned(),
                });
                None
            }
        }
    }
}

impl FakeCompositor {
    /// A session with a lock in `state`, a seat (focused on `focus`), a window and, when `screen` is set, a screen.
    fn new(state: &str, screen: bool, focus: Option<u64>) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(ROOT, node("node", 0, Vec::new()));
        nodes.insert(LOCK, node("lock", ROOT, vec![("state", text(state))]));
        let seat_props = focus.map_or_else(Vec::new, |id| vec![("focus", MirrorValue::Node(id))]);
        nodes.insert(SEAT, node("seat", ROOT, seat_props));
        nodes.insert(WINDOW, node("window", ROOT, vec![("title", text("Editor"))]));
        let mut order = vec![LOCK, SEAT, WINDOW];
        if screen {
            nodes.insert(
                SCREEN,
                node(
                    "screen",
                    ROOT,
                    vec![
                        ("width", MirrorValue::Double(1000.0)),
                        ("height", MirrorValue::Double(800.0)),
                        ("primary", MirrorValue::Bool(true)),
                    ],
                ),
            );
            order.push(SCREEN);
        }
        if let Some(root) = nodes.get_mut(&ROOT) {
            root.children = order;
        }
        Self(Rc::new(RefCell::new(Compositor {
            nodes,
            serial: 1,
            sent: Vec::new(),
            calls: Vec::new(),
            queue: Vec::new(),
        })))
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

    fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }

    /// The compositor writes a property of its own.
    fn external_set(&self, id: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&id) {
            entry.props.insert(key.to_owned(), value.clone());
        }
        self.publish(vec![set_change(id, key, value)]);
    }

    /// The compositor's lock moves to `state`.
    fn lock_state(&self, state: &str) {
        self.external_set(LOCK, "state", text(state));
    }

    /// A screen comes up after the shell attached.
    fn add_screen(&self) {
        {
            let mut c = self.0.borrow_mut();
            c.nodes.insert(
                SCREEN,
                node(
                    "screen",
                    ROOT,
                    vec![
                        ("width", MirrorValue::Double(1000.0)),
                        ("height", MirrorValue::Double(800.0)),
                    ],
                ),
            );
            if let Some(root) = c.nodes.get_mut(&ROOT) {
                root.children.push(SCREEN);
            }
        }
        self.publish(vec![
            MirrorChange {
                class: "screen".to_owned(),
                ..change(MirrorChangeKind::Created, SCREEN)
            },
            MirrorChange {
                parent: ROOT,
                index: 99,
                ..change(MirrorChangeKind::Parented, SCREEN)
            },
            set_change(SCREEN, "width", MirrorValue::Double(1000.0)),
            set_change(SCREEN, "height", MirrorValue::Double(800.0)),
        ]);
    }

    /// The keyboard focus, as the seat holds it.
    fn focus(&self) -> Option<u64> {
        match self.0.borrow().nodes.get(&SEAT)?.props.get("focus") {
            Some(MirrorValue::Node(id)) if *id != 0 => Some(*id),
            _ => None,
        }
    }

    /// A key reaches the node that holds the keyboard, as the compositor routes it; nothing happens with no focus.
    fn key(&self, key: &str, typed: &str, pressed: bool) {
        let Some(target) = self.focus() else { return };
        self.key_to(target, key, typed, pressed);
    }

    fn key_to(&self, target: u64, key: &str, typed: &str, pressed: bool) {
        self.0.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node: target,
            kind: "key".to_owned(),
            fields: EventFields {
                key: key.to_owned(),
                text: typed.to_owned(),
                pressed,
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

    fn calls(&self) -> Vec<Call> {
        self.0.borrow().calls.clone()
    }

    fn checks(&self) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.method == "authenticate")
            .collect()
    }

    fn children(&self, id: u64) -> Vec<u64> {
        self.0
            .borrow()
            .nodes
            .get(&id)
            .map_or_else(Vec::new, |entry| entry.children.clone())
    }

    fn flag(&self, id: u64, key: &str) -> Option<bool> {
        match self.0.borrow().nodes.get(&id).and_then(|entry| entry.props.get(key)) {
            Some(MirrorValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }

    fn exists(&self, id: u64) -> bool {
        self.0.borrow().nodes.contains_key(&id)
    }

    fn content(&self, id: u64) -> Option<Vec<u8>> {
        match self
            .0
            .borrow()
            .nodes
            .get(&id)
            .and_then(|entry| entry.props.get("content"))
        {
            Some(MirrorValue::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        }
    }

    /// The strings a canvas paints, in order.
    fn texts(&self, id: u64) -> Vec<String> {
        let Some(bytes) = self.content(id) else {
            return Vec::new();
        };
        decode(&bytes)
            .expect("the shell wrote a paint list")
            .ops
            .iter()
            .filter_map(|op| match op {
                PaintOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn rect(&self, id: u64) -> (f64, f64, f64, f64) {
        let number = |key: &str| match self.0.borrow().nodes.get(&id).and_then(|entry| entry.props.get(key)) {
            Some(MirrorValue::Double(value)) => *value,
            _ => 0.0,
        };
        (number("x"), number("y"), number("width"), number("height"))
    }

    /// The lock screen's canvases: the scrim under the lock node, the card, then the clock, entry and message.
    fn screen(&self) -> Option<Screen> {
        let scrim = *self.children(LOCK).first()?;
        let card = *self.children(scrim).first()?;
        let parts = self.children(card);
        let [clock, entry, message] = parts[..] else {
            return None;
        };
        Some(Screen {
            scrim,
            card,
            clock,
            entry,
            message,
        })
    }

    /// Whether any property of any node holds `needle` as text, or inside paint bytes.
    fn holds(&self, needle: &str) -> bool {
        let c = self.0.borrow();
        c.nodes
            .values()
            .flat_map(|entry| entry.props.values())
            .any(|value| match value {
                MirrorValue::Text(held) => held.contains(needle),
                MirrorValue::Bytes(bytes) => bytes.windows(needle.len()).any(|window| window == needle.as_bytes()),
                _ => false,
            })
    }
}

#[derive(Clone, Copy, Debug)]
struct Screen {
    scrim: u64,
    card: u64,
    clock: u64,
    entry: u64,
    message: u64,
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
            .map(|(id, entry)| SnapshotNode {
                id: *id,
                parent: entry.parent,
                index: c
                    .nodes
                    .get(&entry.parent)
                    .and_then(|parent| parent.children.iter().position(|child| child == id))
                    .map_or(0, |at| at as i64),
                class: entry.class.clone(),
                title: String::new(),
                app_id: String::new(),
                props: entry.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
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

struct Rig {
    shell: JsShell,
    comp: FakeCompositor,
    _dir: TempDir,
}

impl Rig {
    fn new(comp: FakeCompositor) -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
        fs::write(
            root.join("init.ts"),
            "import { lockScreen } from \"gnoblin\";\n(globalThis as any).lockScreen = lockScreen;\n",
        )
        .expect("init.ts is written");
        let mut shell = JsShell::new(Box::new(comp.clone()), JsShellConfig::new(&root)).expect("the shell boots");
        let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        Self { shell, comp, _dir: dir }
    }

    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// Runs frames until the shell and the compositor have nothing left to say, asserting that nothing went wrong.
    fn settle(&mut self) {
        for _ in 0..6 {
            let diagnostics = self.frame();
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        }
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .expect("the script has a value")
    }

    /// Types `typed` one character at a time to whatever holds the keyboard, then lets the shell see it.
    fn type_text(&mut self, typed: &str) {
        for character in typed.chars() {
            let name = character.to_string();
            self.comp.key(&name, &name, true);
            self.comp.key(&name, "", false);
        }
        self.settle();
    }

    fn enter(&mut self) {
        self.comp.key("Return", "", true);
        self.comp.key("Return", "", false);
        self.settle();
    }

    /// The token of the one password check the shell has sent that nothing answered yet.
    fn check_token(&self) -> u64 {
        self.comp.checks().last().expect("a password check was sent").token
    }
}

/// A shell on a session that is not locked yet, with a screen and the window focused.
fn unlocked() -> Rig {
    let mut rig = Rig::new(FakeCompositor::new("not-locked", true, Some(WINDOW)));
    rig.settle();
    rig
}

/// A shell on a session whose lock is `pending`, with the lock screen up.
fn locked() -> Rig {
    let mut rig = unlocked();
    rig.comp.lock_state("pending");
    rig.settle();
    rig.comp.lock_state("locked");
    rig.settle();
    rig
}

// ---- the screen comes up and goes down ----------------------------------------------------------------------------

#[test]
fn a_lock_request_puts_the_screen_up_and_engages_the_lock() {
    let mut rig = unlocked();
    assert!(rig.comp.screen().is_none(), "an unlocked session has no lock screen");
    assert_ne!(rig.comp.flag(LOCK, "engaged"), Some(true));
    rig.comp.lock_state("pending");
    rig.settle();
    let screen = rig.comp.screen().expect("the lock screen is up");
    assert_eq!(
        rig.comp.flag(LOCK, "engaged"),
        Some(true),
        "the lock records that its screen is up"
    );
    assert_eq!(
        rig.comp.rect(screen.scrim),
        (0.0, 0.0, 1000.0, 800.0),
        "the scrim covers the screen"
    );
    let (x, y, width, _) = rig.comp.rect(screen.card);
    assert_eq!(
        (x, y, width),
        (320.0, 266.0, 360.0),
        "the card is centred, a third of the way down"
    );
    assert_eq!(rig.comp.focus(), Some(screen.entry), "the entry holds the keyboard");
}

#[test]
fn a_lock_requested_before_the_tree_has_a_screen_waits_for_one() {
    let mut rig = Rig::new(FakeCompositor::new("pending", false, None));
    rig.settle();
    assert!(
        rig.comp.screen().is_none(),
        "no lock screen can be up before there is a screen to cover"
    );
    assert_ne!(
        rig.comp.flag(LOCK, "engaged"),
        Some(true),
        "and the lock is not engaged"
    );
    rig.comp.add_screen();
    rig.settle();
    assert!(
        rig.comp.screen().is_some(),
        "the screen comes up once there is a screen"
    );
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn a_shell_that_attaches_to_a_locked_session_shows_the_lock_screen() {
    // The request is one-shot: a shell that comes up after it hears nothing, so the state is the request.
    let mut rig = Rig::new(FakeCompositor::new("locked", true, None));
    rig.settle();
    assert!(
        rig.comp.screen().is_some(),
        "a locked session shows its lock screen on attach"
    );
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn a_lock_that_unlocks_before_the_screen_exists_leaves_no_screen_behind() {
    let mut rig = Rig::new(FakeCompositor::new("pending", false, None));
    rig.settle();
    rig.comp.lock_state("not-locked");
    rig.settle();
    rig.comp.add_screen();
    rig.settle();
    assert!(rig.comp.screen().is_none(), "an unlocked session gets no lock screen");
    assert_ne!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn an_unlock_the_screen_did_not_make_takes_it_down_and_a_later_lock_puts_it_up_again() {
    let mut rig = locked();
    let first = rig.comp.screen().expect("the lock screen is up");
    rig.comp.lock_state("not-locked"); // unlocked through the protocol while the screen is up
    rig.settle();
    assert!(rig.comp.screen().is_none(), "an unlock takes the screen down");
    assert!(!rig.comp.exists(first.scrim), "and destroys its canvases");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(false), "and disengages the lock");
    assert_eq!(
        rig.comp.focus(),
        Some(WINDOW),
        "the keyboard goes back to the window that had it"
    );
    rig.comp.lock_state("pending");
    rig.settle();
    assert!(rig.comp.screen().is_some(), "a later lock draws the screen again");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
}

#[test]
fn focus_is_not_taken_back_from_a_window_that_took_it_while_the_screen_went_down() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(SCREEN)); // something else took the keyboard
    rig.comp.lock_state("not-locked");
    rig.settle();
    assert_eq!(
        rig.comp.focus(),
        Some(SCREEN),
        "the screen held it no longer, so it gives nothing back"
    );
    assert!(!rig.comp.exists(screen.entry));
}

#[test]
fn retiring_the_shell_takes_the_screen_down() {
    let mut rig = locked();
    assert!(rig.comp.screen().is_some());
    rig.shell.retire().expect("the shell retires");
    // No frame follows: the generation is gone, and a frame of the same generation would be a new lock screen.
    assert!(
        rig.comp.screen().is_none(),
        "a generation that goes leaves no canvases on the cover"
    );
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(false));
}

// ---- the clock ----------------------------------------------------------------------------------------------------

#[test]
fn the_screen_shows_the_time_of_day_and_follows_it() {
    let mut rig = unlocked();
    rig.eval("globalThis.fakeNow = 1700000000000; Date.now = () => globalThis.fakeNow; 0");
    rig.comp.lock_state("locked");
    rig.settle();
    let clock = rig.comp.screen().expect("the lock screen is up").clock;
    // The expected text is worked out in the script from the same local time zone, one digit pair at a time.
    let at = "((ms) => { const d = new Date(ms); \
              return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0'); })";
    let expected = rig.eval(&format!("{at}(1700000000000)"));
    assert_eq!(
        rig.comp.texts(clock),
        std::slice::from_ref(&expected),
        "the clock shows HH:MM of the moment"
    );
    assert!(
        expected.len() == 5 && expected.as_bytes()[2] == b':',
        "the time is two digits, a colon and two digits: {expected}"
    );
    rig.eval("globalThis.fakeNow += 60000; 0");
    rig.settle();
    let expected_later = rig.eval(&format!("{at}(1700000060000)"));
    assert_eq!(
        rig.comp.texts(clock),
        std::slice::from_ref(&expected_later),
        "a minute later the clock shows the next minute"
    );
    assert_ne!(expected, expected_later);
}

// ---- the password -------------------------------------------------------------------------------------------------

#[test]
fn enter_sends_the_typed_password_to_the_compositor_as_the_calls_secret_text() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("hunter2");
    assert_eq!(
        rig.comp.texts(screen.entry),
        ["*******"],
        "the entry draws one mask character for each key"
    );
    assert!(rig.comp.checks().is_empty(), "nothing is sent before Enter");
    rig.enter();
    let checks = rig.comp.checks();
    assert_eq!(checks.len(), 1, "Enter sends one check");
    assert_eq!(checks[0].node, LOCK, "the call is the lock node's");
    assert_eq!(checks[0].text, "hunter2", "the typed password reaches the compositor");
    assert!(checks[0].args.is_empty(), "the password is never an argument");
}

#[test]
fn the_password_is_never_drawn_and_never_held_in_the_tree_or_the_entry() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("hunter2");
    assert!(!rig.comp.holds("hunter2"), "the tree holds no copy while it is typed");
    rig.enter();
    assert!(!rig.comp.holds("hunter2"), "nor once it is sent");
    assert_eq!(
        rig.comp.texts(screen.entry),
        [""],
        "the entry was emptied as soon as the call was made, before any answer"
    );
}

#[test]
fn typing_edits_with_backspace_and_ignores_keys_that_type_nothing() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("abc");
    rig.comp.key("BackSpace", "", true);
    rig.comp.key("Shift_L", "", true);
    rig.comp.key("Tab", "\t", true);
    rig.settle();
    assert_eq!(
        rig.comp.texts(screen.entry),
        ["**"],
        "one character was deleted and the others typed nothing"
    );
    rig.type_text("d");
    rig.enter();
    assert_eq!(rig.comp.checks()[0].text, "abd");
}

#[test]
fn a_key_that_came_up_types_nothing_and_a_character_outside_ascii_is_typed_whole() {
    let mut rig = locked();
    rig.comp.key("e", "\u{e9}", true);
    rig.comp.key("e", "\u{e9}", false);
    rig.comp.key("x", "\u{1f511}", true);
    rig.settle();
    rig.enter();
    assert_eq!(rig.comp.checks()[0].text, "\u{e9}\u{1f511}");
}

#[test]
fn an_empty_entry_is_not_an_attempt() {
    let mut rig = locked();
    rig.enter();
    assert!(rig.comp.checks().is_empty(), "a bare Enter sends nothing");
}

#[test]
fn an_enter_pressed_while_the_compositor_checks_is_not_a_second_attempt() {
    let mut rig = locked();
    rig.type_text("one");
    rig.enter();
    rig.type_text("two");
    rig.enter();
    assert_eq!(rig.comp.checks().len(), 1, "one check runs at a time");
    let token = rig.check_token();
    rig.comp.answer(token, "", "denied");
    rig.settle();
    rig.type_text("three");
    rig.enter();
    let checks = rig.comp.checks();
    assert_eq!(checks.len(), 2, "after the answer the user may try again");
    assert_eq!(checks[1].text, "three", "and what was typed while it ran was not kept");
}

#[test]
fn an_ok_answer_takes_the_screen_down_and_disengages_the_lock() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    assert!(
        rig.comp.screen().is_some(),
        "the screen waits for the compositor's answer"
    );
    rig.comp.answer(token, "ok", "");
    rig.settle();
    assert!(rig.comp.screen().is_none(), "an ok answer takes the screen down");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(false));
    assert_eq!(rig.comp.focus(), Some(WINDOW));
    // The compositor's state moves in the same breath: the screen stays down.
    rig.comp.lock_state("not-locked");
    rig.settle();
    assert!(rig.comp.screen().is_none());
}

#[test]
fn a_screen_that_was_accepted_does_not_come_back_before_the_state_says_unlocked() {
    // The answer can arrive before the state change that goes with it: the screen must not flash up again.
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "ok", "");
    rig.settle();
    rig.settle();
    assert!(
        rig.comp.screen().is_none(),
        "the state still reads locked and the screen stays down"
    );
    rig.comp.lock_state("not-locked");
    rig.settle();
    rig.comp.lock_state("pending");
    rig.settle();
    assert!(rig.comp.screen().is_some(), "a later lock puts it up again");
}

#[test]
fn a_wrong_password_keeps_the_screen_up_clears_the_entry_and_says_so() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("nope");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "", "denied");
    rig.settle();
    assert!(rig.comp.screen().is_some(), "a denial keeps the lock screen up");
    assert_eq!(rig.comp.flag(LOCK, "engaged"), Some(true));
    assert_eq!(rig.comp.texts(screen.message), ["Wrong password."]);
    assert_eq!(rig.comp.texts(screen.entry), [""], "the entry is empty again");
    assert_eq!(rig.comp.focus(), Some(screen.entry), "and still holds the keyboard");
    rig.type_text("again");
    assert_eq!(
        rig.comp.texts(screen.entry),
        ["*****"],
        "the next attempt starts from nothing"
    );
}

#[test]
fn a_rate_limited_attempt_keeps_the_screen_up_and_says_how_long_to_wait() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("again");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "1500", "rate-limited");
    rig.settle();
    assert!(rig.comp.screen().is_some());
    assert_eq!(
        rig.comp.texts(screen.message),
        ["Too many attempts. Try again in 2 s."],
        "1500 ms rounds up to 2 s"
    );
}

#[test]
fn a_busy_or_failed_check_says_what_it_can() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("a");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "", "busy");
    rig.settle();
    assert_eq!(rig.comp.texts(screen.message), ["Checking the previous attempt."]);
    rig.type_text("b");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "", "failed: no PAM");
    rig.settle();
    assert_eq!(rig.comp.texts(screen.message), ["The password could not be checked."]);
    assert!(rig.comp.screen().is_some(), "anything but ok keeps the lock");
}

#[test]
fn an_answer_to_another_call_is_not_the_checks() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token + 1000, "ok", "");
    rig.settle();
    assert!(
        rig.comp.screen().is_some(),
        "an answer under a token the check never had changes nothing"
    );
}

// ---- keys after the unlock ----------------------------------------------------------------------------------------

#[test]
fn keys_typed_after_the_unlock_are_ignored() {
    let mut rig = locked();
    let screen = rig.comp.screen().expect("the lock screen is up");
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.answer(token, "ok", "");
    rig.settle();
    let sent = rig.comp.calls().len();
    // The compositor may still have the keys in flight for the entry it knew: they go nowhere.
    rig.comp.key_to(screen.entry, "x", "x", true);
    rig.comp.key_to(screen.entry, "Return", "", true);
    rig.settle();
    assert_eq!(
        rig.comp.calls().len(),
        sent,
        "no call was made for a key typed after the unlock"
    );
    assert!(rig.comp.screen().is_none());
}

#[test]
fn keys_that_reach_another_node_are_not_the_screens() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.comp.key_to(WINDOW, "Return", "", true);
    rig.settle();
    assert!(
        rig.comp.checks().is_empty(),
        "an Enter for the window is not an Enter in the entry"
    );
}

#[test]
fn a_late_answer_after_an_external_unlock_is_dropped() {
    let mut rig = locked();
    rig.type_text("pw");
    rig.enter();
    let token = rig.check_token();
    rig.comp.lock_state("not-locked");
    rig.settle();
    rig.comp.answer(token, "", "denied");
    rig.settle();
    assert!(
        rig.comp.screen().is_none(),
        "a screen that is down has nothing to say about an answer"
    );
}
