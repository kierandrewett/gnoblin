//! The scenario runner of the oracle: a TypeScript shell over the fake compositor, driven by named actions, recording what it did.
//!
//! A scenario is a world (what the compositor holds at boot), the source of `shell.ts`, and a list of steps. Each step is the actions
//! made since the last `step` call and what followed: the lines the shell logged (`rec(label, value)` in the scenario's source), the
//! transactions it sent, in batches, and what it reported. Nodes the shell made are named `n1`, `n2`... in the order it made them, so a
//! native shell that numbers its nodes otherwise can be compared.
//!
//! The fixture is the scenario's description and its result, so a native lane replays the actions and compares the steps.

#![allow(dead_code)]

use gnoblin_paint::decode;
use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
use gnoblin_testkit::oracle::{Fixtures, Value, Writes, hex, json, number};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

use crate::common::{LIBRARY_GLOBALS, Rig, rig_setup};

/// The oracle records the TypeScript library, which a shell that imports nothing else now does not run on. A module only that library has
/// pins the scenario's shell to it. The import joins the first line of the prelude, so no line of the scenario's source moves and the
/// positions in the fixtures stay.
fn typescript_globals() -> String {
    LIBRARY_GLOBALS.replacen('\n', " import \"gnoblin-std/layout\";\n", 1)
}

/// A property value in a world or an action.
#[derive(Clone, Debug)]
pub enum V {
    Bool(bool),
    Num(f64),
    Text(&'static str),
    Str(String),
    Texts(&'static [&'static str]),
}

impl V {
    fn mirror(&self) -> MirrorValue {
        match self {
            V::Bool(flag) => MirrorValue::Bool(*flag),
            V::Num(value) => MirrorValue::Double(*value),
            V::Text(value) => text(value),
            V::Str(value) => text(value),
            V::Texts(values) => MirrorValue::Texts(values.iter().map(|value| (*value).to_owned()).collect()),
        }
    }

    fn json(&self) -> Value {
        match self {
            V::Bool(flag) => json!(flag),
            V::Num(value) => number(*value),
            V::Text(value) => json!(value),
            V::Str(value) => json!(value),
            V::Texts(values) => json!({ "texts": values }),
        }
    }
}

/// A window of the world.
#[derive(Clone, Debug)]
pub struct Win {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub decoration: String,
    pub props: Vec<(&'static str, V)>,
}

/// What the compositor holds before the shell attaches: the desktop of the other tests (screen 20, seat 21, lock 22), and windows.
#[derive(Clone, Debug)]
pub struct World {
    pub width: f64,
    pub height: f64,
    pub windows: Vec<Win>,
    /// Properties of nodes that are not windows (the screen 20, the seat 21, the lock 22), set before boot.
    pub nodes: Vec<(u64, &'static str, V)>,
    /// The session of `tests/common/policy.rs` instead of the desktop: screens `20` (and `22`), the seat `21`, the lock `23`, and `n` windows of
    /// 300 by 200 at (10 n, 20 n), for the scenarios that tile windows over workspaces `1` to `3`.
    pub policy: Option<(u64, bool)>,
    /// The session of `tests/input.rs`: a workspace, windows 3 and 4, seat A (node 5) with a macro pad and, when the flag is set, seat B (6).
    pub input: Option<bool>,
}

impl World {
    /// A screen of `width` by `height` and no window.
    pub fn desktop(width: f64, height: f64) -> World {
        World {
            width,
            height,
            windows: Vec::new(),
            nodes: Vec::new(),
            policy: None,
            input: None,
        }
    }

    /// The session of `tests/common/policy.rs` with `n` windows (ids 3 on), and a second screen when `second`.
    pub fn policy(n: u64, second: bool) -> World {
        World {
            width: 1000.0,
            height: 800.0,
            windows: Vec::new(),
            nodes: Vec::new(),
            policy: Some((n, second)),
            input: None,
        }
    }

    /// The session of `tests/input.rs`, with a second seat when `second`.
    pub fn input(second: bool) -> World {
        World {
            width: 1000.0,
            height: 800.0,
            windows: Vec::new(),
            nodes: Vec::new(),
            policy: None,
            input: Some(second),
        }
    }

    /// Sets `key` of node `id` (the seat is 21) before boot.
    pub fn node(mut self, id: u64, key: &'static str, value: V) -> World {
        self.nodes.push((id, key, value));
        self
    }

    /// `count` server-decorated windows, ids 3 on, titled `win1`...
    pub fn windows(mut self, count: u64) -> World {
        for n in 0..count {
            self.windows.push(Win {
                id: 3 + n,
                title: format!("win{}", n + 1),
                app_id: "foot".to_owned(),
                decoration: "server".to_owned(),
                props: Vec::new(),
            });
        }
        self
    }

    /// A window of the given decoration (`server`, `client` or `none`).
    pub fn window(mut self, id: u64, title: &str, app_id: &str, decoration: &str) -> World {
        self.windows.push(Win {
            id,
            title: title.to_owned(),
            app_id: app_id.to_owned(),
            decoration: decoration.to_owned(),
            props: Vec::new(),
        });
        self
    }

    /// Sets `key` of window `id` before boot.
    pub fn with(mut self, id: u64, key: &'static str, value: V) -> World {
        let win = self
            .windows
            .iter_mut()
            .find(|win| win.id == id)
            .expect("the window is in the world");
        win.props.push((key, value));
        self
    }

    fn build(&self, comp: &FakeCompositor) {
        match (self.policy, self.input) {
            (Some((n, second)), _) => crate::common::policy::populate(comp, n, second),
            (None, Some(_)) => {}
            (None, None) => comp.add_desktop(self.width, self.height),
        }
        comp.borrow_mut().refuse_every = 0;
        if let Some(second) = self.input {
            // The input scenarios' session (`tests/input.rs`): a workspace, window 3 with the focus, window 4, seat A with a macro pad and
            // maybe seat B. A claim another live claim on the seat holds is refused, as the compositor refuses it.
            comp.refuse_duplicate_claims();
            comp.add_node(
                2,
                ROOT,
                "workspace",
                vec![("index", MirrorValue::Uint(0)), ("active", MirrorValue::Bool(true))],
            );
            comp.add_node(
                3,
                ROOT,
                "window",
                vec![
                    ("title", text("term")),
                    ("app_id", text("foot")),
                    ("focused", MirrorValue::Bool(true)),
                    ("x", MirrorValue::Double(10.0)),
                    ("y", MirrorValue::Double(20.0)),
                    ("width", MirrorValue::Double(300.0)),
                    ("height", MirrorValue::Double(200.0)),
                ],
            );
            comp.add_node(
                4,
                ROOT,
                "window",
                vec![("title", text("web")), ("app_id", text("firefox"))],
            );
            let seat = |id: u64, name: &str, devices: &[&str]| {
                comp.add_node(
                    id,
                    ROOT,
                    "seat",
                    vec![
                        ("name", text(name)),
                        ("pointer_x", MirrorValue::Double(0.0)),
                        ("pointer_y", MirrorValue::Double(0.0)),
                        ("focus", MirrorValue::Node(3)),
                        (
                            "device",
                            MirrorValue::Texts(devices.iter().map(|device| (*device).to_owned()).collect()),
                        ),
                    ],
                );
            };
            seat(5, "seatA", &["7:Macro Pad:keyboard"]);
            if second {
                seat(6, "seatB", &[]);
            }
        }
        for win in &self.windows {
            comp.add_window_decorated(win.id, &win.title, &win.app_id, &win.decoration);
            for (key, value) in &win.props {
                if let Some(node) = comp.borrow_mut().nodes.get_mut(&win.id) {
                    node.props.insert((*key).to_owned(), value.mirror());
                }
            }
        }
        for (id, key, value) in &self.nodes {
            if let Some(node) = comp.borrow_mut().nodes.get_mut(id) {
                node.props.insert((*key).to_owned(), value.mirror());
            }
        }
    }

    fn json(&self) -> Value {
        json!({
            "screen": [self.width, self.height],
            "policy": self.policy.map(|(n, second)| json!({ "windows": n, "second_screen": second })),
            "input": self.input.map(|second| json!({ "second_seat": second })),
            "windows": self.windows.iter().map(|win| json!({
                "id": win.id,
                "title": win.title,
                "app_id": win.app_id,
                "decoration": win.decoration,
                "props": win.props.iter().map(|(key, value)| (key.to_string(), value.json())).collect::<serde_json::Map<_, _>>(),
            })).collect::<Vec<_>>(),
            "nodes": self.nodes.iter().map(|(id, key, value)| json!({ "id": id, "key": key, "value": value.json() })).collect::<Vec<_>>(),
        })
    }
}

/// A node an action names: by number, or by a TypeScript expression that evaluates to one.
#[derive(Clone, Copy, Debug)]
pub enum At {
    Id(u64),
    Js(&'static str),
    /// The node reached from node `.0` by taking the child at each index of `.1` in turn.
    Under(u64, &'static [usize]),
    /// The node that holds the seat's keyboard focus.
    Focus,
    /// The node a script gave the id `.0`.
    Named(&'static str),
}

impl At {
    fn json(self) -> Value {
        match self {
            At::Id(id) => json!(id),
            At::Js(expression) => json!({ "js": expression }),
            At::Under(id, path) => json!({ "under": id, "children": path }),
            At::Focus => json!("focus"),
            At::Named(name) => json!({ "named": name }),
        }
    }
}

/// The compositor's left, right and middle buttons.
pub const LEFT: u64 = 272;
pub const RIGHT: u64 = 273;
pub const MIDDLE: u64 = 274;

/// What every scenario's source starts with: the library's exports as globals, and `rec`, which logs a value for the step.
const PRELUDE: &str = "(globalThis as any).log = [];\n\
    (globalThis as any).g = globalThis;\n\
    Date.now = () => 1700000000000;\n\
    (globalThis as any).rec = (label: string, value: unknown) => { (globalThis as any).log.push({ label, value }); };\n\
    (globalThis as any).caught = (f: () => unknown) => { try { f(); return 'none'; } catch (e) { return (e as Error).message; } };\n\
    (globalThis as any).fail = (f: () => unknown) => { try { f(); return 'accepted'; } catch (e) { return (e as Error).name + ': ' + (e as Error).message; } };\n";

/// The fixtures of the standard places.
pub fn reference() -> Fixtures {
    Fixtures::new("crates/gnoblin-std-impl/tests/fixtures/reference")
}

pub fn model() -> Fixtures {
    Fixtures::new("crates/gnoblin-core-impl/tests/fixtures/model")
}

pub fn paint() -> Fixtures {
    Fixtures::new("crates/gnoblin-js/tests/fixtures/paint")
}

/// Where a scenario's result goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Home {
    /// `crates/gnoblin-std-impl/tests/fixtures/reference`.
    Reference,
    /// `crates/gnoblin-core-impl/tests/fixtures/model`.
    Model,
}

pub struct Scn {
    rig: Rig,
    name: String,
    home: Home,
    world: Value,
    source: String,
    writes: Writes,
    mark: usize,
    answered: usize,
    ended: usize,
    claims_mark: usize,
    actions: Vec<Value>,
    diagnostics: Vec<String>,
    steps: Vec<Value>,

    boot: Vec<String>,
}

/// Replaces the scratch directory a shell ran in (`/tmp/.tmpAbC123/shell.ts`) with `<root>`, which differs on every run, and
/// drops the checkout's own path from the library's module paths (`/work/crates/gnoblin-js/../../data/js/...` becomes
/// `data/js/...`), which differs on every machine.
fn unroot(line: &str) -> String {
    let mut text = line.replace(concat!(env!("CARGO_MANIFEST_DIR"), "/../../"), "");
    while let Some(at) = text.find("/.tmp") {
        let start = text[..at]
            .rfind(|c: char| c.is_whitespace() || "\"'(:".contains(c))
            .map_or(0, |i| i + 1);
        let tail = at + "/.tmp".len();
        let end = tail
            + text[tail..]
                .find(|c: char| !c.is_ascii_alphanumeric())
                .unwrap_or(text.len() - tail);
        text.replace_range(start..end, "<root>");
    }
    text
}

/// Strips the digits of a node number out of a line the shell wrote: `Camera2D#12` is `Camera2D#N`.
fn unnumber(line: &str) -> String {
    let line = unroot(line);
    let mut out = String::new();
    let mut after_hash = false;
    for ch in line.chars() {
        if ch.is_ascii_digit() && after_hash {
            continue;
        }
        if after_hash && !ch.is_ascii_digit() {
            after_hash = false;
        }
        out.push(ch);
        if ch == '#' {
            after_hash = true;
            out.push('N');
        }
    }
    out
}

fn unnumber_value(value: &mut Value) {
    match value {
        Value::String(line) => *line = unnumber(line),
        Value::Array(items) => items.iter_mut().for_each(unnumber_value),
        Value::Object(map) => map.values_mut().for_each(unnumber_value),
        _ => {}
    }
}

/// A paint list as its ops (for a person) and its bytes (the proof).
fn render_bytes(bytes: &[u8]) -> Value {
    match decode(bytes) {
        Ok(list) => json!({
            "paint": list.ops.iter().map(|op| format!("{op:?}")).collect::<Vec<_>>(),
            "bytes": hex(bytes),
        }),
        Err(_) => json!({ "bytes": hex(bytes) }),
    }
}

impl Scn {
    /// A shell whose `shell.ts` is the library's globals, `rec` and `body`, over `world`; the load is the first step's.
    pub fn new(name: &str, home: Home, world: &World, body: &str) -> Scn {
        let source = format!("{PRELUDE}{body}");
        let build = world.clone();
        let mut rig = rig_setup(&typescript_globals(), &source, move |comp| build.build(comp));
        // What the oracle records is the TypeScript library's: the native library that runs beside it says its own notices, which are not.
        rig.tell_notices = false;
        // The recorded library has no stock focus policy of its own; the shell stops the one the toolkit started on boot.
        gnoblin_std_impl::ui_testing::stop_stock_focus();
        let boot = rig.load_diagnostics.clone();
        let mark = 0;
        Scn {
            rig,
            name: name.to_owned(),
            home,
            world: world.json(),
            source: body.to_owned(),
            writes: Writes::with_bytes(render_bytes),
            mark,
            answered: 0,
            ended: 0,
            claims_mark: 0,
            actions: Vec::new(),
            diagnostics: Vec::new(),
            steps: Vec::new(),

            boot,
        }
    }

    /// The rig, for what a scenario needs to read of the compositor in Rust.
    pub fn rig(&mut self) -> &mut Rig {
        &mut self.rig
    }

    fn settle(&mut self) {
        let diagnostics = self.rig.settle();
        self.diagnostics.extend(diagnostics.iter().map(|line| unnumber(line)));
    }

    fn resolve(&mut self, at: At) -> u64 {
        match at {
            At::Id(id) => id,
            At::Named(name) => self
                .rig
                .comp
                .named(name)
                .unwrap_or_else(|| panic!("no node has the id `{name}`")),
            At::Focus => match self.rig.comp.value(21, "focus") {
                Some(MirrorValue::Node(id)) if id != 0 => id,
                other => panic!("nothing holds the keyboard: {other:?}"),
            },
            At::Under(id, path) => path.iter().fold(id, |node, index| {
                self.rig.comp.children(node).get(*index).copied().unwrap_or_else(|| {
                    panic!("node {node} has no child {index}");
                })
            }),
            At::Js(expression) => {
                self.rig
                    .eval(&format!("String({expression})"))
                    .parse::<f64>()
                    .unwrap_or_else(|_| panic!("`{expression}` is not a node number")) as u64
            }
        }
    }

    /// Lets the shell run its first frames and ends the step `boot`: what it wrote as it started.
    pub fn boot(&mut self) -> &mut Scn {
        self.settle();
        self.step("boot")
    }

    /// Runs `js` in the shell, as a block.
    pub fn eval(&mut self, js: &str) -> &mut Scn {
        self.actions.push(json!({ "action": "eval", "js": js }));
        self.rig.eval(&format!("{{\n{js}\n}}; 'ok'"));
        self.settle();
        self
    }

    /// Runs `js` in the shell and does not settle: the next frame sees what it did together with the next action.
    pub fn eval_now(&mut self, js: &str) -> &mut Scn {
        self.actions.push(json!({ "action": "eval_now", "js": js }));
        self.rig.eval(&format!("{{\n{js}\n}}; 'ok'"));
        self
    }

    /// Runs frames.
    pub fn settle_frames(&mut self) -> &mut Scn {
        self.actions.push(json!({ "action": "settle" }));
        self.settle();
        self
    }

    /// Moves the clock on.
    pub fn advance(&mut self, ms: u64) -> &mut Scn {
        self.actions.push(json!({ "action": "advance", "ms": ms }));
        self.rig.advance(ms);
        self.settle();
        self
    }

    /// The compositor maps a window.
    pub fn open(&mut self, id: u64, title: &str, app_id: &str) -> &mut Scn {
        self.actions
            .push(json!({ "action": "open", "id": id, "title": title, "app_id": app_id }));
        self.rig.comp.create_window(id, title, app_id);
        self.settle();
        self
    }

    /// The compositor maps a window with the decoration it negotiated (`server`, `client` or `none`).
    pub fn open_decorated(&mut self, id: u64, title: &str, app_id: &str, decoration: &str) -> &mut Scn {
        self.actions.push(json!({
            "action": "open", "id": id, "title": title, "app_id": app_id, "decoration": decoration,
        }));
        self.rig.comp.create_window_decorated(id, title, app_id, decoration);
        self.settle();
        self
    }

    /// The compositor destroys a node.
    pub fn destroy(&mut self, id: u64) -> &mut Scn {
        self.actions.push(json!({ "action": "destroy", "id": id }));
        self.rig.comp.destroy(id);
        self.settle();
        self
    }

    /// The compositor changes a property of a node.
    pub fn set(&mut self, id: u64, key: &'static str, value: V) -> &mut Scn {
        self.actions
            .push(json!({ "action": "set", "id": id, "key": key, "value": value.json() }));
        self.rig.comp.external_set(id, key, value.mirror());
        self.settle();
        self
    }

    /// The compositor changes a property of the seat (node 21).
    pub fn seat(&mut self, key: &'static str, value: V) -> &mut Scn {
        self.set(21, key, value)
    }

    /// A key of the seat's focus, or of `at`.
    pub fn key(&mut self, at: At, key: &str, text: &str, pressed: bool) -> &mut Scn {
        self.actions.push(json!({
            "action": "key", "node": at.json(), "key": key, "text": text, "pressed": pressed,
        }));
        let node = self.resolve(at);
        self.rig.comp.key(node, key, text, pressed);
        self.settle();
        self
    }

    /// A key pressed and released.
    pub fn tap(&mut self, at: At, key: &str, text: &str) -> &mut Scn {
        self.key(at, key, text, true).key(at, key, text, false)
    }

    /// A pointer event on a node (`press`, `release`, `motion`, `enter`, `leave`, `scroll`).
    pub fn pointer(&mut self, at: At, kind: &str, x: f64, y: f64, button: u64) -> &mut Scn {
        self.actions.push(json!({
            "action": "pointer", "node": at.json(), "kind": kind, "x": x, "y": y, "button": button,
        }));
        let node = self.resolve(at);
        self.rig.comp.pointer(node, kind, x, y, button);
        self.settle();
        self
    }

    /// The compositor's request of a window (`menu`, `maximize`...).
    pub fn request(&mut self, at: At, name: &str, x: f64, y: f64) -> &mut Scn {
        self.actions
            .push(json!({ "action": "request", "node": at.json(), "name": name, "x": x, "y": y }));
        let node = self.resolve(at);
        self.rig.comp.request(node, name, x, y);
        self.settle();
        self
    }

    /// Types `typed` a character at a time to whatever holds the keyboard. The text is not written to the fixture, only how many characters it was: it
    /// is a password.
    pub fn type_secret(&mut self, typed: &str) -> &mut Scn {
        self.actions
            .push(json!({ "action": "type_secret", "characters": typed.chars().count() }));
        for character in typed.chars() {
            let name = character.to_string();
            let node = self.resolve(At::Focus);
            self.rig.comp.key(node, &name, &name, true);
            self.rig.comp.key(node, &name, "", false);
        }
        self.settle();
        self
    }

    /// The compositor plugs in a screen.
    pub fn plug(&mut self, id: u64, name: &str, rect: (f64, f64, f64, f64), primary: bool) -> &mut Scn {
        self.actions.push(json!({
            "action": "plug", "id": id, "name": name, "rect": [rect.0, rect.1, rect.2, rect.3], "primary": primary,
        }));
        self.rig.comp.create_screen(id, name, rect, primary);
        self.settle();
        self
    }

    /// The compositor answers the last `apply` the shell sent.
    pub fn answer_apply(&mut self, result: &str, error: &str) -> &mut Scn {
        self.actions
            .push(json!({ "action": "answer_apply", "result": result, "error": error }));
        let token = self.rig.comp.applies().last().expect("an apply went out").2;
        self.rig.comp.answer(token, result, error);
        self.settle();
        self
    }

    /// The compositor answers the last password check the shell sent.
    pub fn answer_check(&mut self, result: &str, error: &str) -> &mut Scn {
        self.actions
            .push(json!({ "action": "answer_check", "result": result, "error": error }));
        let token = self
            .rig
            .comp
            .checks()
            .last()
            .copied()
            .expect("a password check was sent");
        self.rig.comp.answer(token, result, error);
        self.settle();
        self
    }

    /// Moves the pointer of `seat` to `x`, `y`. A pointer chord drag live on the seat streams an `update` with the new place.
    pub fn pointer_at(&mut self, seat: u64, x: f64, y: f64) -> &mut Scn {
        self.actions
            .push(json!({ "action": "pointer_at", "seat": seat, "x": x, "y": y }));
        self.rig.comp.set_pointer(seat, x, y);
        self.settle();
        self
    }

    /// A claimed chord changes `state` (`press`, `repeat` or `release`). `claim` is the text of the claim as the compositor holds it (`<Super>1`, or a device
    /// name, a line feed and the chord); a pointer chord (`<Super>Button1`) starts or ends its drag at the seat's pointer.
    pub fn chord(&mut self, claim: &str, state: &str) -> &mut Scn {
        self.actions
            .push(json!({ "action": "chord", "claim": claim.replace('\n', "|"), "state": state }));
        let id = self.rig.comp.claim_id(claim);
        let chord = claim.rsplit('\n').next().unwrap_or(claim);
        if chord.contains("Button") {
            self.rig.comp.drag(id, chord, state);
        } else {
            self.rig.comp.chord(id, chord, state);
        }
        self.settle();
        self
    }

    /// The compositor ends the live drag of the claim `claim` with `cancel`.
    pub fn cancel_drag(&mut self, claim: &str) -> &mut Scn {
        self.actions
            .push(json!({ "action": "cancel_drag", "claim": claim.replace('\n', "|") }));
        let id = self.rig.comp.claim_id(claim);
        self.rig.comp.cancel_drag(id);
        self.settle();
        self
    }

    /// One phase of a touchpad gesture of `kind` (`swipe` or `pinch`) with `fingers`, moved by `dx`, `dy`, scaled and turned.
    #[allow(clippy::too_many_arguments)]
    pub fn gesture(
        &mut self,
        kind: &str,
        fingers: i64,
        phase: &str,
        dx: f64,
        dy: f64,
        scale: f64,
        rotation: f64,
    ) -> &mut Scn {
        self.actions.push(json!({
            "action": "gesture", "kind": kind, "fingers": fingers, "phase": phase, "dx": dx, "dy": dy, "scale": scale, "rotation": rotation,
        }));
        let claim = self.rig.comp.gesture_claim(kind, fingers);
        self.rig
            .comp
            .gesture(claim, kind, fingers, phase, (dx, dy), scale, rotation);
        self.settle();
        self
    }

    /// The compositor says the latest timed write of `node`'s `key` has ended, at its target when `completed`, else run back to its start.
    pub fn end_track(&mut self, node: u64, key: &str, completed: bool) -> &mut Scn {
        self.actions
            .push(json!({ "action": "end_track", "node": node, "key": key, "completed": completed }));
        let tween = self
            .rig
            .comp
            .ops_since(0)
            .into_iter()
            .rev()
            .find_map(|op| match op {
                TxOp::Set {
                    node: target,
                    key: written,
                    seconds: Some(_),
                    tween,
                    ..
                } if target == node && written == key => Some(tween),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no timed write of {node}.{key}"));
        self.rig.comp.track_ended(node, key, completed, tween);
        self.settle();
        self
    }

    /// The compositor says the timed writes sent since it last said so have ended: at their targets when `completed`, else run back to their start.
    pub fn end_tracks(&mut self, completed: bool) -> &mut Scn {
        self.actions
            .push(json!({ "action": "end_tracks", "completed": completed }));
        let mut ended = std::collections::BTreeSet::new();
        for op in self.rig.comp.ops_since(self.ended) {
            if let TxOp::Set {
                node,
                key,
                seconds: Some(_),
                tween: Some(tween),
                ..
            } = op
                && ended.insert((node, key.clone()))
            {
                self.rig.comp.track_ended(node, &key, completed, Some(tween));
            }
        }
        self.ended = self.rig.comp.sent_count();
        self.settle();
        self
    }

    /// The compositor answers every `measure` call not yet answered, as a font with a fixed advance would: 0.6 of the size a glyph, 1.2 a
    /// line, 0.9 to the baseline, each rounded up.
    pub fn answer_measures(&mut self) -> &mut Scn {
        self.actions.push(json!({ "action": "answer_measures" }));
        let asked = self.rig.comp.measures();
        for measure in &asked[self.answered..] {
            let (_, body) = measure.text.split_once('\n').unwrap_or(("", &measure.text));
            let widest = body.lines().map(|line| line.chars().count()).max().unwrap_or(0) as f64;
            let lines = body.lines().count().max(1) as f64;
            let reply = format!(
                "{} {} {}",
                (widest * measure.size * 0.6).ceil(),
                (lines * measure.size * 1.2).ceil(),
                (measure.size * 0.9).ceil()
            );
            self.rig.comp.answer(measure.token, &reply, "");
        }
        self.answered = asked.len();
        self.settle();
        self
    }

    /// Ends a step: what the shell logged, wrote and reported since the last one.
    pub fn step(&mut self, label: &str) -> &mut Scn {
        let calls: Vec<Value> = {
            let borrowed = self.rig.comp.borrow();
            let fresh = borrowed.claim_calls[self.claims_mark.min(borrowed.claim_calls.len())..]
                .iter()
                .map(|(method, id, second, text)| {
                    json!({ "method": method, "id": id, "second": second, "text": text.replace('\n', "|") })
                })
                .collect();
            self.claims_mark = borrowed.claim_calls.len();
            fresh
        };
        let held = self.rig.comp.claimed();
        let mut log: Value =
            serde_json::from_str(&self.rig.eval("JSON.stringify(globalThis.log.splice(0))")).expect("the log is JSON");
        unnumber_value(&mut log);
        let batches = self.rig.comp.sent_since(self.mark);
        self.mark = self.rig.comp.sent_count();
        let writes: Vec<Value> = batches
            .iter()
            .filter(|batch| !batch.is_empty())
            .map(|batch| Value::Array(self.writes.ops(batch)))
            .collect();
        let mut step = serde_json::Map::new();
        step.insert("step".to_owned(), json!(label));
        step.insert("actions".to_owned(), Value::Array(std::mem::take(&mut self.actions)));
        step.insert("log".to_owned(), log);
        if !calls.is_empty() {
            step.insert("claim_calls".to_owned(), Value::Array(calls));
        }
        if !held.is_empty() {
            step.insert("claims_held".to_owned(), json!(held));
        }
        step.insert("writes".to_owned(), Value::Array(writes));
        let mut reported = std::mem::take(&mut self.diagnostics);
        if self.steps.is_empty() {
            let mut first = std::mem::take(&mut self.boot)
                .into_iter()
                .map(|line| unnumber(&line))
                .collect::<Vec<_>>();
            first.append(&mut reported);
            reported = first;
        }
        step.insert("diagnostics".to_owned(), json!(reported));
        self.steps.push(Value::Object(step));
        self
    }

    /// Adds `item` to the list `key` of the step that ended last: a snapshot is the state at the end of a step.
    fn attach(&mut self, key: &str, item: Value) {
        let step = self
            .steps
            .last_mut()
            .and_then(Value::as_object_mut)
            .expect("a snapshot follows a step");
        step.entry(key.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("a list")
            .push(item);
    }

    /// A snapshot of the subtree under `at`: each node's class, parent and properties, a paint list as its ops.
    #[allow(clippy::type_complexity)]
    pub fn tree(&mut self, label: &str, at: At) -> &mut Scn {
        let root = self.resolve(at);
        let nodes: Vec<(u64, String, u64, Vec<(String, MirrorValue)>)> = {
            let borrowed = self.rig.comp.borrow();
            borrowed
                .subtree(root)
                .into_iter()
                .filter_map(|id| {
                    borrowed.nodes.get(&id).map(|node| {
                        (
                            id,
                            node.class.clone(),
                            node.parent,
                            node.props
                                .iter()
                                .map(|(key, value)| (key.clone(), value.clone()))
                                .collect(),
                        )
                    })
                })
                .collect()
        };
        let listed: Vec<Value> = nodes
            .iter()
            .map(|(id, class, parent, props)| {
                let mut map = serde_json::Map::new();
                for (key, value) in props {
                    map.insert(key.clone(), self.writes.value(value));
                }
                json!({
                    "node": self.writes.node(*id),
                    "class": class,
                    "parent": self.writes.node(*parent),
                    "props": map,
                })
            })
            .collect();
        let item = json!({ "tree": label, "root": self.writes.node(root), "nodes": listed });
        self.attach("trees", item);
        self
    }

    /// Holds the paint lists of every canvas under `at`, in tree order, as `<scenario>.<label>.bin`: each is a little-endian `u32` length
    /// and the bytes of a list.
    pub fn paint(&mut self, label: &str, at: At) -> &mut Scn {
        let root = self.resolve(at);
        let canvases: Vec<(u64, f64, f64, f64, f64, Vec<u8>)> = {
            let borrowed = self.rig.comp.borrow();
            borrowed
                .subtree(root)
                .into_iter()
                .filter_map(|id| {
                    let node = borrowed.nodes.get(&id)?;
                    let content = match node.props.get("content") {
                        Some(MirrorValue::Bytes(bytes)) => bytes.clone(),
                        _ => return None,
                    };
                    let double = |key: &str| match node.props.get(key) {
                        Some(MirrorValue::Double(value)) => *value,
                        _ => 0.0,
                    };
                    Some((id, double("x"), double("y"), double("width"), double("height"), content))
                })
                .collect()
        };
        let mut file = Vec::new();
        let mut listed = Vec::new();
        for (id, x, y, width, height, content) in &canvases {
            file.extend_from_slice(&(content.len() as u32).to_le_bytes());
            file.extend_from_slice(content);
            listed.push(json!({
                "node": self.writes.node(*id),
                "rect": [x, y, width, height],
                "bytes": content.len(),
                "paint": render_bytes(content)["paint"].clone(),
            }));
        }
        let name = format!("{}.{label}", self.name);
        paint().bytes(&name, &file);
        let item = json!({ "paint": label, "file": format!("{name}.bin"), "canvases": listed });
        self.attach("paints", item);
        self
    }

    /// Writes the fixture, or checks it.
    pub fn finish(self) {
        let fixtures = match self.home {
            Home::Reference => reference(),
            Home::Model => model(),
        };
        let mut document = serde_json::Map::new();
        document.insert("scenario".to_owned(), json!(self.name));
        document.insert("world".to_owned(), self.world);
        document.insert("source".to_owned(), json!(self.source));
        document.insert("steps".to_owned(), Value::Array(self.steps));

        fixtures.json(&self.name, &Value::Object(document));
    }
}
