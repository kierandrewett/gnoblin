//! Tests for key bindings, gestures and drag bindings: the trigger grammar, `gnoblin.input`, the claims it
//! makes on the seat, and the stock move and resize bindings.
//!
//! The grammar is tested against the stock library alone (no host). Everything else runs a real `JsShell`
//! over `FakeLink`, an in-memory compositor that records the seat claims the shell makes and plays back
//! scripted chords, `pointer_chord` drags, gestures, pointer positions and window deaths. Nothing here proves
//! that the real compositor takes these claims: the fake accepts every claim it is sent. The real one is
//! driven by `crates/gnoblin/tests/js_shell.rs`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, JsShell, JsShellConfig, stock_modules};
use gnoblin_tree_client::tree_mirror::{
    Attached, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeChord, TreeGesture,
    TreeIncoming, TreeLink, TreePointerChord, TreeTx, TxOp, TxReply,
};
use tempfile::TempDir;

// -- the trigger grammar -----------------------------------------------------------------------------------

const GRAMMAR_HARNESS: &str = r##"
globalThis.gnoblin = { __register() {} };
export const emit = (value) => { globalThis.out = String(value); };
"##;

/// Evaluates `body` (TypeScript) with the stock library imported and returns what it passed to `emit`.
fn grammar(body: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let source = format!(
        "import {{ emit }} from \"harness\";\n\
         import {{ parseTrigger, claimOf, Binding, BindingCollection, KeyBinding, FloatingLayout, resolveTriggers, ClientMoveBinding, ClientResizeBinding, dragConstraintMode, constrainRect }} from \"gnoblin\";\n\
         const thrown = (fn: () => void) => {{ try {{ fn(); }} catch (e: any) {{ return `${{e.name}}: ${{e.message}} [${{e.token}}]`; }} return \"did not throw\"; }};\n\
         {body}\n"
    );
    fs::write(root.join("init.ts"), source).expect("init.ts is written");
    let mut config = JsConfig::new(&root).embed("harness", GRAMMAR_HARNESS);
    for (name, source) in stock_modules() {
        config = config.embed(name, source);
    }
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    generation.eval_module(Path::new("init.ts")).expect("init.ts evaluates");
    generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.globals().get::<_, String>("out"))
        .expect("the case called emit")
}

fn lines(items: &[&str]) -> String {
    items.join("\n")
}

#[test]
fn accepted_triggers_have_one_canonical_spelling() {
    let got = grammar(
        r#"
        const cases = [
            "Super+Return", "super+return", "Super+Shift+1", "Shift+Super+1", "Super+1", "Ctrl+Alt+Delete",
            "control+alt+delete", "Primary+q", "Super+Q", "Super+F9", "super+f9", "Super+pagedown",
            "Super+Page_Down", "Super+plus", "XF86AudioMute", "Super", "Logo+Return", "Mod1+Tab",
            "Super+MouseLeft+Drag", "super+mouseleft+drag", "Super+MouseLeft", "Super+MouseLeft+Click",
            "MouseMiddle", "Ctrl+Alt+MouseRight+Drag", "Super+Wheel", "Super+wheelup", "Alt+WheelDown",
            "Swipe3Left", "swipe4", "pinch2in", "Pinch5Out", "Swipe2Up", "Super+constructor",
        ];
        emit(cases.map((c) => `${c} => ${parseTrigger(c).text}`).join("\n"));
        "#,
    );
    assert_eq!(
        got,
        lines(&[
            "Super+Return => Super+Return",
            "super+return => Super+Return",
            "Super+Shift+1 => Shift+Super+1",
            "Shift+Super+1 => Shift+Super+1",
            "Super+1 => Super+1",
            "Ctrl+Alt+Delete => Ctrl+Alt+Delete",
            "control+alt+delete => Ctrl+Alt+Delete",
            "Primary+q => Ctrl+q",
            "Super+Q => Super+q",
            "Super+F9 => Super+F9",
            "super+f9 => Super+F9",
            "Super+pagedown => Super+Page_Down",
            "Super+Page_Down => Super+Page_Down",
            "Super+plus => Super+plus",
            "XF86AudioMute => XF86AudioMute",
            "Super => Super",
            "Logo+Return => Super+Return",
            "Mod1+Tab => Alt+Tab",
            "Super+MouseLeft+Drag => Super+MouseLeft+Drag",
            "super+mouseleft+drag => Super+MouseLeft+Drag",
            "Super+MouseLeft => Super+MouseLeft",
            "Super+MouseLeft+Click => Super+MouseLeft",
            "MouseMiddle => MouseMiddle",
            "Ctrl+Alt+MouseRight+Drag => Ctrl+Alt+MouseRight+Drag",
            "Super+Wheel => Super+Wheel",
            "Super+wheelup => Super+WheelUp",
            "Alt+WheelDown => Alt+WheelDown",
            "Swipe3Left => Swipe3Left",
            "swipe4 => Swipe4",
            "pinch2in => Pinch2In",
            "Pinch5Out => Pinch5Out",
            "Swipe2Up => Swipe2Up",
            "Super+constructor => Super+constructor",
        ])
    );
}

#[test]
fn super_1_and_super_shift_1_are_two_triggers_with_two_claims() {
    let got = grammar(
        r#"
        const a = parseTrigger("Super+1");
        const b = parseTrigger("Super+Shift+1");
        const chord = (t: string) => (claimOf(parseTrigger(t)) as any).text;
        const collection = new BindingCollection();
        collection.add(new KeyBinding({ id: "a", trigger: "Super+1" }));
        collection.add(new KeyBinding({ id: "b", trigger: "Super+Shift+1" }));
        emit([
            a.text !== b.text,
            a.modifiers.join(),
            b.modifiers.join(),
            chord("Super+1"),
            chord("Super+Shift+1"),
            chord("Shift+Super+1"),
            chord("Ctrl+Alt+Delete"),
            chord("Super+MouseLeft+Drag"),
            chord("Super+MouseLeft"),
            chord("Super"),
            JSON.stringify(claimOf(parseTrigger("Swipe3Left"))),
            JSON.stringify(claimOf(parseTrigger("Super+Wheel"))),
            collection.size,
        ].join("\n"));
        "#,
    );
    assert_eq!(
        got,
        lines(&[
            "true",
            "Super",
            "Shift,Super",
            "<Super>1",
            "<Shift><Super>1",
            "<Shift><Super>1",
            "<Control><Alt>Delete",
            "<Super>Button1",
            "<Super>Button1",
            "Super",
            r#"{"kind":"gesture","gesture":"swipe","fingers":3}"#,
            "null",
            "2",
        ])
    );
}

#[test]
fn rejected_triggers_name_the_token_that_is_wrong() {
    let got = grammar(
        r#"
        const cases = [
            "", "Super+", "Super++1", "+Super", "Super+Shift", "Shift", "Super+Super+1", "Super+a+b", "a+Super",
            "Super+!", "Super+Mod4+1", "Super+Drag", "MouseLeft+MouseRight", "Super+MouseLeft+a",
            "Swipe1Left", "Swipe6", "Swipe3In", "Pinch3Left", "Super+Swipe3", "Super + 1", "Super+Ctrl-1",
            "Super+MouseLeft+Drag+Drag", "Super+Wheel+Wheel", "Super+Return+Drag",
        ];
        emit(cases.map((c) => `${JSON.stringify(c)}: ${thrown(() => parseTrigger(c, "trigger"))}`).join("\n"));
        "#,
    );
    assert_eq!(
        got,
        lines(&[
            r#""": TypeError: trigger must be a trigger such as "Super+Return", got  [undefined]"#,
            r#""Super+": TypeError: trigger: an empty part after "Super"; write a literal plus as "plus" (token "" in "Super+") []"#,
            r#""Super++1": TypeError: trigger: an empty part after "Super"; write a literal plus as "plus" (token "" in "Super++1") []"#,
            r#""+Super": TypeError: trigger: an empty part after the start; write a literal plus as "plus" (token "" in "+Super") []"#,
            r#""Super+Shift": TypeError: trigger: ends with a modifier and has no key (token "Shift" in "Super+Shift") [Shift]"#,
            r#""Shift": TypeError: trigger: a lone modifier is only a trigger for Super (a tap) (token "Shift" in "Shift") [Shift]"#,
            r#""Super+Super+1": TypeError: trigger: repeats the modifier Super (token "Super" in "Super+Super+1") [Super]"#,
            r#""Super+a+b": TypeError: trigger: is a second key after "a"; a trigger has one key (token "b" in "Super+a+b") [b]"#,
            r#""a+Super": TypeError: trigger: is a modifier after the key; modifiers come first (token "Super" in "a+Super") [Super]"#,
            r#""Super+!": TypeError: trigger: is a character, not a key name; write the keysym name (exclam, minus, plus) or Shift with the key (token "!" in "Super+!") [!]"#,
            r#""Super+Mod4+1": TypeError: trigger: names a keymap-dependent mask; write Super, Alt, Meta or Hyper (token "Mod4" in "Super+Mod4+1") [Mod4]"#,
            r#""Super+Drag": TypeError: trigger: needs a mouse button before it, such as Super+MouseLeft+Drag (token "Drag" in "Super+Drag") [Drag]"#,
            r#""MouseLeft+MouseRight": TypeError: trigger: is a second input; a trigger has one key, button or wheel (token "MouseRight" in "MouseLeft+MouseRight") [MouseRight]"#,
            r#""Super+MouseLeft+a": TypeError: trigger: is a key after a mouse button or wheel; a trigger has one input (token "a" in "Super+MouseLeft+a") [a]"#,
            r#""Swipe1Left": TypeError: trigger: a gesture has 2 to 5 fingers, not 1 (token "Swipe1Left" in "Swipe1Left") [Swipe1Left]"#,
            r#""Swipe6": TypeError: trigger: a gesture has 2 to 5 fingers, not 6 (token "Swipe6" in "Swipe6") [Swipe6]"#,
            r#""Swipe3In": TypeError: trigger: a swipe goes left, right, up, down, not in (token "Swipe3In" in "Swipe3In") [Swipe3In]"#,
            r#""Pinch3Left": TypeError: trigger: a pinch goes in, out, not left (token "Pinch3Left" in "Pinch3Left") [Pinch3Left]"#,
            r#""Super+Swipe3": TypeError: trigger: a gesture takes no modifiers or other parts (token "Swipe3" in "Super+Swipe3") [Swipe3]"#,
            r#""Super + 1": TypeError: trigger: has whitespace; join the parts with plus alone (token "Super " in "Super + 1") [Super ]"#,
            r#""Super+Ctrl-1": TypeError: trigger: is not a key name; a key name is letters, digits and underscores (token "Ctrl-1" in "Super+Ctrl-1") [Ctrl-1]"#,
            r#""Super+MouseLeft+Drag+Drag": TypeError: trigger: is a second action (token "Drag" in "Super+MouseLeft+Drag+Drag") [Drag]"#,
            r#""Super+Wheel+Wheel": TypeError: trigger: is a second input; a trigger has one key, button or wheel (token "Wheel" in "Super+Wheel+Wheel") [Wheel]"#,
            r#""Super+Return+Drag": TypeError: trigger: needs a mouse button before it, such as Super+MouseLeft+Drag (token "Drag" in "Super+Return+Drag") [Drag]"#,
        ])
    );
}

#[test]
fn floating_registers_move_and_resize_and_a_trigger_override_keeps_or_drops_them() {
    let got = grammar(
        r#"
        const list = (layout: any) => [...layout.bindings].map((b: any) => `${b.id}=${b.trigger}`).join(",");
        const defaults = new FloatingLayout();
        const noResize = new FloatingLayout({ triggers: { resize: null } });
        const undef = new FloatingLayout({ triggers: { move: undefined } });
        const alt = new FloatingLayout({ triggers: { move: "Alt+MouseLeft+Drag", resize: null } });
        const bindings = defaults.bindings;
        const move = bindings.get("move")!;
        move.trigger = "Alt+MouseLeft+Drag";
        const afterRemap = list(defaults);
        const removed = bindings.remove("resize");
        const gone = bindings.get("resize");
        bindings.clear();
        emit([
            list(new FloatingLayout()),
            list(noResize),
            list(undef),
            list(alt),
            afterRemap,
            removed,
            String(gone),
            bindings.size,
            thrown(() => new FloatingLayout({ triggers: { move: "Super+MouseLeft+Drag", resize: "Super+MouseLeft+Drag" } })),
        ].join("\n"));
        "#,
    );
    assert_eq!(
        got,
        lines(&[
            "move=Super+MouseLeft+Drag,resize=Super+MouseRight+Drag",
            "move=Super+MouseLeft+Drag",
            "move=Super+MouseLeft+Drag,resize=Super+MouseRight+Drag",
            "move=Alt+MouseLeft+Drag",
            "move=Alt+MouseLeft+Drag,resize=Super+MouseRight+Drag",
            "true",
            "undefined",
            "0",
            r#"BindingConflictError: binding "resize" conflicts with "move": both match Super+MouseLeft+Drag at priority 0; give one a different priority or trigger [undefined]"#,
        ])
    );
}

// -- the fake compositor -----------------------------------------------------------------------------------

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
    /// The seat claims the shell holds: id to `(method, text)`.
    claims: BTreeMap<i64, (String, String)>,
    /// Every claim call in the order it arrived: `(method, id, second argument, text)`.
    calls: Vec<(String, i64, i64, String)>,
    /// The seat node each live claim was made on.
    claim_seats: BTreeMap<i64, u64>,
    /// The pointer chord drags in flight, by claim id.
    drags: BTreeMap<i64, FakeDrag>,
}

/// A pointer chord's drag as the fake compositor streams it: where it began and what the chord is.
struct FakeDrag {
    seat: u64,
    origin: (f64, f64),
    button: i64,
    modifiers: String,
}

#[derive(Clone)]
struct FakeLink(Rc<RefCell<Compositor>>);

type Comp = Rc<RefCell<Compositor>>;

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

fn le(bytes: &[u8]) -> i64 {
    i64::from_le_bytes(bytes.try_into().expect("eight bytes"))
}

impl FakeLink {
    /// Applies a transaction: window writes go to the node map, seat claims to the claim table.
    fn land(&self, tx: &TreeTx) {
        let mut c = self.0.borrow_mut();
        let mut changes = Vec::new();
        for op in &tx.ops {
            match op {
                TxOp::Set { node, key, value, .. } => {
                    if let Some(entry) = c.nodes.get_mut(node) {
                        entry.props.insert(key.clone(), value.clone());
                    }
                    changes.push(set_change(*node, key, value.clone()));
                }
                TxOp::Call {
                    node,
                    method,
                    args,
                    text,
                    ..
                } => match method.as_str() {
                    "claim" | "claim_gesture" => {
                        let id = le(&args[0..8]);
                        c.claim_seats.insert(id, *node);
                        let second = le(&args[8..16]);
                        c.claims.insert(id, (method.clone(), text.expose().to_owned()));
                        c.calls.push((method.clone(), id, second, text.expose().to_owned()));
                    }
                    "unclaim" => {
                        let id = le(&args[0..8]);
                        c.claims.remove(&id);
                        c.claim_seats.remove(&id);
                        // A drag whose claim goes ends without a word to the owner.
                        c.drags.remove(&id);
                        c.calls.push((method.clone(), id, 0, String::new()));
                    }
                    _ => {}
                },
                _ => {}
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
    }

    fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        c.queue.push(TreeIncoming::Changes { serial, changes });
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
        self.land(tx);
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        Ok(std::mem::take(&mut self.0.borrow_mut().queue))
    }
}

/// The compositor's side of a scripted session.
trait Script {
    /// Moves the pointer of `seat`. A pointer chord drag on that seat streams an `update` with the new place.
    fn set_pointer(&self, seat: u64, x: f64, y: f64);
    /// A key chord's `press` or `release`. A pointer chord (it ends in `ButtonN`) starts its drag at the seat's
    /// pointer on `press` and ends it there on `release`, as the compositor's `pointer_chord` signal does.
    fn chord(&self, claim: i64, chord: &str, state: &str);
    /// The compositor ending a live drag with `cancel`.
    fn cancel_drag(&self, claim: i64);
    fn gesture(&self, claim: i64, phase: &str, dx: f64, dy: f64);
    fn destroy(&self, node: u64);
    fn claim_id(&self, text: &str) -> i64;
    fn claimed(&self) -> Vec<String>;
    fn prop(&self, node: u64, key: &str) -> f64;
}

impl Script for Comp {
    fn set_pointer(&self, seat: u64, x: f64, y: f64) {
        {
            let mut c = self.borrow_mut();
            let node = c.nodes.get_mut(&seat).expect("the seat exists");
            node.props.insert("pointer_x".into(), MirrorValue::Double(x));
            node.props.insert("pointer_y".into(), MirrorValue::Double(y));
        }
        FakeLink(self.clone()).publish(vec![
            set_change(seat, "pointer_x", MirrorValue::Double(x)),
            set_change(seat, "pointer_y", MirrorValue::Double(y)),
        ]);
        let mut c = self.borrow_mut();
        let updates: Vec<TreePointerChord> = c
            .drags
            .iter()
            .filter(|(_, drag)| drag.seat == seat)
            .map(|(claim, drag)| drag_message(*claim, drag, "update", (x, y)))
            .collect();
        c.queue.extend(updates.into_iter().map(TreeIncoming::PointerChord));
    }

    fn chord(&self, claim: i64, chord: &str, state: &str) {
        let Some((modifiers, button)) = chord.rsplit_once("Button") else {
            self.borrow_mut().queue.push(TreeIncoming::Chord(TreeChord {
                claim,
                chord: chord.into(),
                state: state.into(),
            }));
            return;
        };
        let mut c = self.borrow_mut();
        let seat = *c
            .claim_seats
            .get(&claim)
            .expect("a pointer chord claim was made on a seat");
        let at = pointer_at(&c, seat);
        let message = if state == "press" {
            let drag = FakeDrag {
                seat,
                origin: at,
                button: button.parse().expect("a button number"),
                modifiers: modifiers.to_owned(),
            };
            let message = drag_message(claim, &drag, "start", at);
            c.drags.insert(claim, drag);
            message
        } else {
            let drag = c.drags.remove(&claim).expect("a drag is live");
            drag_message(claim, &drag, "end", at)
        };
        c.queue.push(TreeIncoming::PointerChord(message));
    }

    fn cancel_drag(&self, claim: i64) {
        let mut c = self.borrow_mut();
        let drag = c.drags.remove(&claim).expect("a drag is live");
        let message = drag_message(claim, &drag, "cancel", pointer_at(&c, drag.seat));
        c.queue.push(TreeIncoming::PointerChord(message));
    }

    fn gesture(&self, claim: i64, phase: &str, dx: f64, dy: f64) {
        self.borrow_mut().queue.push(TreeIncoming::Gesture(TreeGesture {
            claim,
            kind: "swipe".into(),
            fingers: 3,
            phase: phase.into(),
            dx,
            dy,
            scale: 1.0,
            rotation: 0.0,
        }));
    }

    fn destroy(&self, node: u64) {
        self.borrow_mut().nodes.remove(&node);
        FakeLink(self.clone()).publish(vec![MirrorChange {
            removed: vec![node],
            ..change(MirrorChangeKind::Destroyed, node)
        }]);
    }

    fn claim_id(&self, text: &str) -> i64 {
        let c = self.borrow();
        *c.claims
            .iter()
            .find(|(_, (_, held))| held == text)
            .unwrap_or_else(|| panic!("no live claim for {text:?}; live: {:?}", c.claims))
            .0
    }

    fn claimed(&self) -> Vec<String> {
        self.borrow()
            .claims
            .values()
            .map(|(_, text)| text.replace('\n', "|"))
            .collect()
    }

    fn prop(&self, node: u64, key: &str) -> f64 {
        match self.borrow().nodes[&node].props.get(key) {
            Some(MirrorValue::Double(value)) => *value,
            other => panic!("node {node} has no double {key}: {other:?}"),
        }
    }
}

/// The `pointer_chord` signal of `drag` in `phase` with the pointer at `at`.
fn drag_message(claim: i64, drag: &FakeDrag, phase: &str, at: (f64, f64)) -> TreePointerChord {
    TreePointerChord {
        claim,
        phase: phase.to_owned(),
        x: at.0,
        y: at.1,
        dx: at.0 - drag.origin.0,
        dy: at.1 - drag.origin.1,
        button: drag.button,
        modifiers: drag.modifiers.clone(),
        time: 0,
    }
}

/// Where the seat's pointer is, as the fake compositor holds it.
fn pointer_at(c: &Compositor, seat: u64) -> (f64, f64) {
    let props = &c.nodes[&seat].props;
    let axis = |key: &str| match props.get(key) {
        Some(MirrorValue::Double(value)) => *value,
        _ => 0.0,
    };
    (axis("pointer_x"), axis("pointer_y"))
}

fn node(class: &str, parent: u64, props: Vec<(&str, MirrorValue)>) -> Node {
    Node {
        class: class.into(),
        parent,
        props: props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
    }
}

fn seat_node(name: &str, devices: &[&str]) -> Node {
    node(
        "seat",
        1,
        vec![
            ("name", text(name)),
            ("pointer_x", MirrorValue::Double(0.0)),
            ("pointer_y", MirrorValue::Double(0.0)),
            ("focus", MirrorValue::Node(3)),
            (
                "device",
                MirrorValue::Texts(devices.iter().map(|d| (*d).to_owned()).collect()),
            ),
        ],
    )
}

/// A root, a workspace (2), window 3 at 10,20 sized 300x200 with the focus, window 4 with no size, and seats
/// `seatA` (5, with a macro pad) and, when `second`, `seatB` (6).
fn world(second: bool) -> Comp {
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
    c.nodes.insert(5, seat_node("seatA", &["7:Macro Pad:keyboard"]));
    if second {
        c.nodes.insert(6, seat_node("seatB", &[]));
    }
    c.serial = 1;
    Rc::new(RefCell::new(c))
}

struct Rig {
    shell: JsShell,
    comp: Comp,
    _dir: TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// The text of `source`'s value; `undefined` when it has none.
    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }

    /// The `log` the entry module keeps, joined.
    fn log(&mut self) -> String {
        self.eval("log.join('|')")
    }
}

fn rig_with(comp: &Comp, init: &str, tweak: impl FnOnce(&mut JsShellConfig)) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), init).expect("init.ts is written");
    let mut config = JsShellConfig::new(&root);
    tweak(&mut config);
    let mut shell = JsShell::new(Box::new(FakeLink(comp.clone())), config).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut rig = Rig {
        shell,
        comp: comp.clone(),
        _dir: dir,
    };
    // The claims the entry made travel with the first frame.
    rig.frame();
    rig
}

fn rig(comp: &Comp, init: &str) -> Rig {
    rig_with(comp, init, |_| {})
}

/// Imports and a `log` for entries that record what their bindings did. The one workspace has a layout with no
/// bindings, so the stock one a config without workspaces gets (floating, with move and resize) claims nothing here.
const PRELUDE: &str = "import { gnoblin, FloatingLayout, KeyBinding, PointerDragBinding, GestureBinding, Workspace } from \"gnoblin\";\n\
                       const log: string[] = [];\n\
                       (globalThis as any).log = log;\n\
                       gnoblin.workspaces.add(new Workspace({ id: \"main\", layout: \"monocle\" }));\n\
                       const input = gnoblin.input.primary;\n\
                       (globalThis as any).input = input;\n";

fn entry(rest: &str) -> String {
    format!("{PRELUDE}{rest}")
}

// -- key chords --------------------------------------------------------------------------------------------

#[test]
fn a_bound_chord_is_claimed_and_its_handler_runs_in_the_frame_loop() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "const handle = input.bind(\"Super+Shift+1\", (e: any) => { log.push(`fired:${e.trigger}:${e.client?.id}`); });\n\
             (globalThis as any).handle = handle;\n",
        ),
    );
    assert_eq!(comp.claimed(), ["<Shift><Super>1"]);
    let (method, id, grab) = {
        let c = comp.borrow();
        let call = &c.calls[0];
        (call.0.clone(), call.1, call.2)
    };
    assert_eq!(
        (method.as_str(), id, grab),
        ("claim", 1, 0),
        "a key chord is not grabbed"
    );

    comp.chord(id, "<Shift><Super>1", "press");
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        rig.log(),
        "fired:Shift+Super+1:3",
        "the focused client is the event's client"
    );

    comp.chord(id, "<Shift><Super>1", "repeat");
    comp.chord(id, "<Shift><Super>1", "release");
    rig.frame();
    assert_eq!(rig.log(), "fired:Shift+Super+1:3", "only the press runs a key binding");
}

#[test]
fn super_1_and_super_shift_1_claim_and_fire_separately() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "input.bind(\"Super+1\", () => { log.push(\"one\"); });\n\
             input.bind(\"Super+Shift+1\", () => { log.push(\"shifted\"); });\n",
        ),
    );
    assert_eq!(comp.claimed(), ["<Super>1", "<Shift><Super>1"]);
    comp.chord(comp.claim_id("<Super>1"), "<Super>1", "press");
    rig.frame();
    comp.chord(comp.claim_id("<Shift><Super>1"), "<Shift><Super>1", "press");
    rig.frame();
    assert_eq!(rig.log(), "one|shifted");
}

#[test]
fn unbind_releases_the_claim_when_the_last_binding_on_it_goes() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry("(globalThis as any).a = input.bind(\"Super+k\", () => { log.push(\"a\"); });\n"),
    );
    assert_eq!(rig.eval("typeof a"), "object");
    assert_eq!(comp.claimed(), ["<Super>k"]);
    rig.eval("a.unbind()");
    rig.frame();
    assert!(comp.claimed().is_empty(), "the last binding took the claim with it");
    let unclaims = comp.borrow().calls.iter().filter(|c| c.0 == "unclaim").count();
    assert_eq!(unclaims, 1);
    // Unbinding twice, or after the claim is gone, does nothing.
    rig.eval("a.unbind()");
    rig.frame();
    assert_eq!(comp.borrow().calls.iter().filter(|c| c.0 == "unclaim").count(), 1);
}

#[test]
fn a_claim_is_shared_by_bindings_of_different_priority_and_kept_until_the_last_is_gone() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "(globalThis as any).low = input.bind(\"Super+k\", () => { log.push(\"low\"); });\n\
             (globalThis as any).high = input.bind(\n\
                 \"Super+k\",\n\
                 new KeyBinding({ id: \"high\", trigger: \"Super+k\", priority: 5, action: () => { log.push(\"high\"); } }),\n\
             );\n",
        ),
    );
    assert_eq!(comp.claimed(), ["<Super>k"], "one chord is claimed once");
    let id = comp.claim_id("<Super>k");
    comp.chord(id, "<Super>k", "press");
    rig.frame();
    assert_eq!(rig.log(), "high", "the higher priority has it");
    rig.eval("high.unbind()");
    rig.frame();
    assert_eq!(comp.claimed(), ["<Super>k"], "the other binding still needs the claim");
    comp.chord(id, "<Super>k", "press");
    rig.frame();
    assert_eq!(rig.log(), "high|low");
    rig.eval("low.unbind()");
    rig.frame();
    assert!(comp.claimed().is_empty());
}

#[test]
fn a_throwing_handler_is_reported_and_does_not_stop_the_others() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "input.bind(\"Super+a\", () => { throw new Error(\"boom\"); });\n\
             input.bind(\"Super+b\", () => { log.push(\"b\"); });\n\
             input.bind(\"Super+c\", () => { log.push(\"c\"); });\n",
        ),
    );
    for key in ["a", "b", "c"] {
        comp.chord(comp.claim_id(&format!("<Super>{key}")), "<Super>x", "press");
    }
    let diagnostics = rig.frame();
    assert_eq!(rig.log(), "b|c");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("boom"), "{diagnostics:?}");
    assert!(
        diagnostics[0].contains("KeyBinding"),
        "the site names the binding: {diagnostics:?}"
    );
    // The same failure again is counted, not reported twice.
    comp.chord(comp.claim_id("<Super>a"), "<Super>a", "press");
    assert!(rig.frame().is_empty());
}

#[test]
fn a_binding_that_declines_lets_the_next_tier_have_the_chord() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "const layout = new FloatingLayout({ triggers: { move: null, resize: null } });\n\
             layout.bindings.add(new KeyBinding({ id: \"layout\", trigger: \"Super+x\", action: () => { log.push(\"layout\"); } }));\n\
             input.attachLayout(layout, (c: any) => c.id === \"3\");\n\
             input.bind(\"Super+x\", () => { log.push(\"context\"); });\n\
             input.devices[0].bind(\"Super+x\", () => { log.push(\"device-declines\"); return false; });\n",
        ),
    );
    assert_eq!(
        comp.claimed(),
        ["<Super>x", "Macro Pad|<Super>x"],
        "a device claim is its own claim"
    );
    let all = comp.claim_id("<Super>x");
    let pad = comp.claim_id("Macro Pad\n<Super>x");
    comp.chord(all, "<Super>x", "press");
    rig.frame();
    comp.chord(pad, "<Super>x", "press");
    rig.frame();
    assert_eq!(
        rig.log(),
        "layout|device-declines|layout",
        "layout beats context, the device is asked first and a decline falls through to the layout"
    );
}

#[test]
fn device_beats_layout_beats_context_and_an_equal_priority_conflict_is_refused_at_registration() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "const layout = new FloatingLayout({ triggers: { move: null, resize: null } });\n\
             layout.bindings.add(new KeyBinding({ id: \"layout\", trigger: \"Super+x\", action: () => { log.push(\"layout\"); } }));\n\
             input.attachLayout(layout, () => true);\n\
             input.bind(\"Super+x\", () => { log.push(\"context\"); });\n\
             input.devices[0].bind(\"Super+x\", () => { log.push(\"device\"); });\n\
             const errors: string[] = [];\n\
             (globalThis as any).errors = errors;\n\
             try { input.bind(\"Super+x\", () => {}); } catch (e: any) { errors.push(`${e.name}:${e.message}`); }\n\
             try { layout.bindings.add(new KeyBinding({ id: \"other\", trigger: \"super+X\" })); } catch (e: any) { errors.push(`${e.name}:${e.message}`); }\n\
             input.bind(\"Super+x\", new KeyBinding({ id: \"urgent\", trigger: \"Super+x\", priority: 9, action: () => { log.push(\"urgent\"); } }));\n",
        ),
    );
    assert_eq!(
        rig.eval("errors.join('\\n')"),
        lines(&[
            r#"BindingConflictError:binding "bind-3:Super+x" conflicts with "bind-1:Super+x": both match Super+x at priority 0; give one a different priority or trigger"#,
            r#"BindingConflictError:binding "other" conflicts with "layout": both match Super+x at priority 0; give one a different priority or trigger"#,
        ])
    );
    // The device's own claim and the all-devices claim.
    let pad = comp.claim_id("Macro Pad\n<Super>x");
    let all = comp.claim_id("<Super>x");
    comp.chord(pad, "<Super>x", "press");
    rig.frame();
    comp.chord(all, "<Super>x", "press");
    rig.frame();
    // For the all-devices claim the layout tier beats the context tier, whatever the context's priority.
    assert_eq!(rig.log(), "device|layout");
}

// -- pointer drags -----------------------------------------------------------------------------------------

/// The workspace's layout is a floating one, which the input module attaches by itself.
const FLOATING_ENTRY: &str = "const layout = new FloatingLayout();\n\
                              (gnoblin.workspaces.get(\"main\") as any).layout = layout;\n\
                              (globalThis as any).layout = layout;\n";

#[test]
fn a_super_left_drag_moves_the_client_by_the_pointer_delta_and_a_release_keeps_it() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    assert_eq!(comp.claimed(), ["<Super>Button1", "<Super>Button3"]);
    let id = comp.claim_id("<Super>Button1");
    assert_eq!(
        comp.borrow().calls.iter().map(|c| (c.1, c.2)).collect::<Vec<_>>(),
        [(1, 1), (2, 1)],
        "pointer chords are claimed with grab, so their release comes back"
    );

    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    assert_eq!(
        (comp.prop(3, "x"), comp.prop(3, "y")),
        (10.0, 20.0),
        "a press alone moves nothing"
    );

    comp.set_pointer(5, 52.0, 61.0);
    rig.frame();
    assert_eq!(
        (comp.prop(3, "x"), comp.prop(3, "y")),
        (10.0, 20.0),
        "within the drag threshold"
    );

    comp.set_pointer(5, 80.0, 100.0);
    rig.frame();
    assert_eq!(
        (comp.prop(3, "x"), comp.prop(3, "y")),
        (40.0, 60.0),
        "moved by the delta from the press"
    );
    assert_eq!((comp.prop(3, "width"), comp.prop(3, "height")), (300.0, 200.0));

    comp.set_pointer(5, 90.0, 110.0);
    rig.frame();
    assert_eq!((comp.prop(3, "x"), comp.prop(3, "y")), (50.0, 70.0));

    comp.chord(id, "<Super>Button1", "release");
    rig.frame();
    comp.set_pointer(5, 400.0, 400.0);
    rig.frame();
    assert_eq!(
        (comp.prop(3, "x"), comp.prop(3, "y")),
        (50.0, 70.0),
        "a released drag follows nothing"
    );
    assert_eq!(
        rig.eval("JSON.stringify(gnoblin.clients.get('3').floatingBounds)"),
        r#"{"x":50,"y":70,"width":300,"height":200}"#,
        "the layout is told where the user put the client"
    );
}

#[test]
fn a_drag_that_starts_over_no_client_is_declined() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 600.0, 600.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 650.0, 650.0);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!((comp.prop(3, "x"), comp.prop(3, "y")), (10.0, 20.0));
    comp.chord(id, "<Super>Button1", "release");
    rig.frame();
}

#[test]
fn a_resize_drag_picks_its_corner_from_the_initial_position_and_never_changes_it() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    let id = comp.claim_id("<Super>Button3");
    let bounds = |comp: &Comp| {
        (
            comp.prop(3, "x"),
            comp.prop(3, "y"),
            comp.prop(3, "width"),
            comp.prop(3, "height"),
        )
    };

    // Press near the top-left corner (local 10,10 of a 300x200 client).
    comp.set_pointer(5, 20.0, 30.0);
    comp.chord(id, "<Super>Button3", "press");
    rig.frame();
    comp.set_pointer(5, 60.0, 70.0);
    rig.frame();
    assert_eq!(
        bounds(&comp),
        (50.0, 60.0, 260.0, 160.0),
        "the top-left corner follows the pointer"
    );

    // Far past the opposite corner: the corner is still the top-left one, so the bottom-right stays put at
    // 310,220 and the client is held at its minimum size against it.
    comp.set_pointer(5, 350.0, 230.0);
    rig.frame();
    let (x, y, width, height) = bounds(&comp);
    assert_eq!(
        (x + width, y + height),
        (310.0, 220.0),
        "the anchor is the opposite corner"
    );
    assert_eq!((width, height), (100.0, 80.0));
    comp.chord(id, "<Super>Button3", "release");
    rig.frame();

    // A second drag from the bottom-right quadrant resizes the other corner.
    let (x, y, width, height) = bounds(&comp);
    comp.set_pointer(5, x + width - 5.0, y + height - 5.0);
    comp.chord(id, "<Super>Button3", "press");
    rig.frame();
    comp.set_pointer(5, x + width + 15.0, y + height + 25.0);
    rig.frame();
    assert_eq!(
        bounds(&comp),
        (x, y, width + 20.0, height + 30.0),
        "the bottom-right corner follows the pointer and the top-left stays"
    );
    comp.chord(id, "<Super>Button3", "release");
    rig.frame();
}

const PROBE: &str = "class Probe extends PointerDragBinding {\n\
                         label: string;\n\
                         constructor(label: string, trigger: string) { super({ id: label, trigger }); this.label = label; }\n\
                         onStart(e: any) { log.push(`${this.label}:start:${e.client?.id}:${e.position.x},${e.position.y}`); return { n: 0 }; }\n\
                         onUpdate(e: any, s: any) { s.n++; log.push(`${this.label}:update:${e.position.x},${e.position.y}`); }\n\
                         onEnd(e: any, s: any) { log.push(`${this.label}:end:${s.n}`); }\n\
                         onCancel(e: any, s: any) { log.push(`${this.label}:cancel:${s.n}`); }\n\
                     }\n";

#[test]
fn a_window_dying_mid_drag_calls_on_cancel_once_and_the_drag_follows_nothing_after() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bind(\"Super+MouseLeft+Drag\", new Probe(\"p\", \"Super+MouseLeft+Drag\"));\n"
        )),
    );
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    rig.frame();
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80");

    comp.destroy(3);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");

    comp.set_pointer(5, 90.0, 100.0);
    rig.frame();
    comp.chord(id, "<Super>Button1", "release");
    rig.frame();
    assert_eq!(
        rig.log(),
        "p:start:3:50,60|p:update:70,80|p:cancel:1",
        "no second cancel, no update, no end"
    );
}

#[test]
fn the_stock_move_binding_does_not_throw_when_its_window_dies_mid_drag() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 80.0, 90.0);
    rig.frame();
    comp.destroy(3);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    comp.chord(id, "<Super>Button1", "release");
    assert!(rig.frame().is_empty());
}

#[test]
fn a_workspace_layout_is_live_while_in_use_and_leaving_it_cancels_the_drag_in_flight() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    assert_eq!(comp.claimed(), ["<Super>Button1", "<Super>Button3"]);
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 80.0, 100.0);
    rig.frame();
    assert_eq!((comp.prop(3, "x"), comp.prop(3, "y")), (40.0, 60.0));

    // The workspace takes another layout: the floating layout's bindings go with it, and the move that was
    // running is cancelled once, which puts the client back.
    rig.eval("gnoblin.workspaces.get('main').layout = 'monocle'");
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(comp.claimed().is_empty(), "{:?}", comp.claimed());
    assert_eq!((comp.prop(3, "x"), comp.prop(3, "y")), (10.0, 20.0));
    // The claim is gone, so the compositor ends the drag silently: no end, no cancel.
    comp.set_pointer(5, 90.0, 110.0);
    rig.frame();
    assert_eq!(
        (comp.prop(3, "x"), comp.prop(3, "y")),
        (10.0, 20.0),
        "nothing follows the pointer now"
    );

    // And back: a new instance claims again.
    rig.eval("gnoblin.workspaces.get('main').layout = new (globalThis.layout.constructor)()");
    rig.frame();
    assert_eq!(comp.claimed(), ["<Super>Button1", "<Super>Button3"]);
}

#[test]
fn an_unbound_or_moved_binding_cancels_the_drag_it_runs_once() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}const probe = new Probe(\"p\", \"Super+MouseLeft+Drag\");\n\
             input.bind(\"Super+MouseLeft+Drag\", probe);\n\
             (globalThis as any).probe = probe;\n"
        )),
    );
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    rig.frame();
    // Cookbook H: remap the trigger of a binding in use. The old claim goes, the new one is made.
    rig.eval("probe.trigger = 'Alt+MouseLeft+Drag'");
    rig.frame();
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");
    assert_eq!(comp.claimed(), ["<Alt>Button1"]);
    rig.eval("probe.unbind()");
    rig.frame();
    assert!(comp.claimed().is_empty());
    assert_eq!(
        rig.log(),
        "p:start:3:50,60|p:update:70,80|p:cancel:1",
        "cancelled once, not again"
    );
}

#[test]
fn a_drag_that_is_behind_reads_the_latest_pointer_and_never_queues_a_second_update() {
    let comp = world(false);
    let mut rig = rig_with(
        &comp,
        &entry(&format!(
            "{PROBE}input.bind(\"Super+MouseLeft+Drag\", new Probe(\"p\", \"Super+MouseLeft+Drag\"));\n\
             input.bind(\"Super+k\", () => {{ const t = Date.now(); while (Date.now() - t < 60) {{}} log.push(\"slow\"); }});\n"
        )),
        |config| config.frame_budget = Duration::from_millis(25),
    );
    let drag = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(drag, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    rig.frame();
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80");

    // This frame the slow key handler runs first and uses up the frame, so the sample for 100,100 waits.
    comp.chord(comp.claim_id("<Super>k"), "<Super>k", "press");
    comp.set_pointer(5, 100.0, 100.0);
    let diagnostics = rig.frame();
    assert!(
        diagnostics.iter().any(|d| d.contains("budget is used up")),
        "{diagnostics:?}"
    );
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|slow");

    // While it waits the pointer moves again. The waiting sample reads where the pointer is now.
    comp.set_pointer(5, 120.0, 140.0);
    rig.frame();
    comp.set_pointer(5, 130.0, 150.0);
    rig.frame();
    assert_eq!(
        rig.log(),
        "p:start:3:50,60|p:update:70,80|slow|p:update:120,140|p:update:130,150",
        "100,100 was skipped and no sample was ever queued behind another"
    );
    comp.chord(drag, "<Super>Button1", "release");
    rig.frame();
    assert!(rig.log().ends_with("p:end:3"), "{}", rig.log());
}

#[test]
fn a_drag_gets_its_own_state_per_gesture_and_per_input_context() {
    let comp = world(true);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}for (const context of gnoblin.input.contexts) {{\n\
                 context.bind(\"Super+MouseLeft+Drag\", new Probe(context.name, \"Super+MouseLeft+Drag\"));\n\
             }}\n\
             (globalThis as any).names = gnoblin.input.contexts.map((c: any) => c.name).sort().join();\n"
        )),
    );
    assert_eq!(rig.eval("names"), "seatA,seatB");
    assert_eq!(
        comp.claimed(),
        ["<Super>Button1", "<Super>Button1"],
        "one claim per seat"
    );
    let claim_on = |seat: u64| -> i64 {
        let c = comp.borrow();
        *c.claim_seats
            .iter()
            .find(|(_, held)| **held == seat)
            .unwrap_or_else(|| panic!("no claim on seat {seat}"))
            .0
    };
    let (first, second) = (claim_on(5), claim_on(6));
    assert_ne!(first, second);

    comp.set_pointer(5, 50.0, 60.0);
    comp.set_pointer(6, 20.0, 30.0);
    comp.chord(first, "<Super>Button1", "press");
    comp.chord(second, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    comp.set_pointer(6, 20.0, 90.0);
    rig.frame();
    comp.chord(first, "<Super>Button1", "release");
    rig.frame();
    comp.set_pointer(6, 20.0, 120.0);
    rig.frame();
    comp.chord(second, "<Super>Button1", "release");
    rig.frame();
    let log = rig.log();
    let of = |label: &str| -> Vec<String> {
        log.split('|')
            .filter(|entry| entry.starts_with(label))
            .map(|entry| entry.replacen(label, "", 1))
            .collect()
    };
    // Each context's drag saw only its own pointer, and `n` counts updates per gesture.
    assert_eq!(of("seatA"), [":start:3:50,60", ":update:70,80", ":end:1"]);
    assert_eq!(
        of("seatB"),
        [":start:3:20,30", ":update:20,90", ":update:20,120", ":end:2"]
    );
}

// -- the pointer_chord stream ------------------------------------------------------------------------------

/// Logs everything a pointer drag's event carries.
const CONTEXT_PROBE: &str = "class Ctx extends PointerDragBinding {\n\
                                 onStart(e: any) { log.push(`start:${e.client?.id}:${e.x},${e.y}:${e.dx},${e.dy}:b${e.button}:${[...e.modifiers].join('+')}:${e.context === input}:${e.trigger}`); return {}; }\n\
                                 onUpdate(e: any) { log.push(`update:${e.x},${e.y}:${e.dx},${e.dy}:${e.position.x},${e.position.y}`); }\n\
                                 onEnd(e: any) { log.push(`end:${e.x},${e.y}:${e.dx},${e.dy}`); }\n\
                                 onCancel(e: any) { log.push(`cancel:${e.x},${e.y}`); }\n\
                             }\n";

/// A screen of `width` by `height`, which is the work area of a drag.
fn add_screen(comp: &Comp, width: f64, height: f64) {
    comp.borrow_mut().nodes.insert(
        7,
        node(
            "screen",
            1,
            vec![
                ("width", MirrorValue::Double(width)),
                ("height", MirrorValue::Double(height)),
                ("primary", MirrorValue::Bool(true)),
            ],
        ),
    );
}

#[test]
fn a_pointer_chord_stream_calls_start_update_update_end_with_the_context_of_the_drag() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{CONTEXT_PROBE}input.bind(\"Super+MouseLeft+Drag\", new Ctx({{ id: \"c\", trigger: \"Super+MouseLeft+Drag\" }}));\n"
        )),
    );
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.log(), "", "a press alone starts nothing");
    comp.set_pointer(5, 70.0, 80.0);
    rig.frame();
    comp.set_pointer(5, 75.0, 90.0);
    rig.frame();
    comp.chord(id, "<Super>Button1", "release");
    rig.frame();
    assert_eq!(
        rig.log(),
        "start:3:50,60:0,0:b1:Super:true:Super+MouseLeft+Drag|update:70,80:20,20:70,80|update:75,90:25,30:75,90|end:75,90:25,30"
    );
}

#[test]
fn a_drag_starts_on_the_topmost_client_under_the_press_and_a_press_over_nothing_has_no_client() {
    let comp = world(false);
    // Window 8 comes after window 3 in tree order, so it is above it where they overlap.
    comp.borrow_mut().nodes.insert(
        8,
        node(
            "window",
            1,
            vec![
                ("title", text("on top")),
                ("x", MirrorValue::Double(40.0)),
                ("y", MirrorValue::Double(50.0)),
                ("width", MirrorValue::Double(100.0)),
                ("height", MirrorValue::Double(100.0)),
            ],
        ),
    );
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bind(\"Super+MouseLeft+Drag\", new Probe(\"p\", \"Super+MouseLeft+Drag\"));\n"
        )),
    );
    let id = comp.claim_id("<Super>Button1");
    // A context's binding hears every drag, so a press over no client starts one with no client.
    for (at, owner) in [((50.0, 60.0), "8"), ((20.0, 30.0), "3"), ((600.0, 600.0), "undefined")] {
        comp.set_pointer(5, at.0, at.1);
        comp.chord(id, "<Super>Button1", "press");
        rig.frame();
        comp.set_pointer(5, at.0 + 10.0, at.1 + 10.0);
        rig.frame();
        comp.chord(id, "<Super>Button1", "release");
        rig.frame();
        let wanted = format!("p:start:{owner}:{},{}", at.0, at.1);
        assert!(rig.log().contains(&wanted), "{wanted} in {}", rig.log());
    }
}

#[test]
fn a_minimised_client_is_not_under_the_press() {
    let comp = world(false);
    comp.borrow_mut().nodes.insert(
        8,
        node(
            "window",
            1,
            vec![
                ("x", MirrorValue::Double(40.0)),
                ("y", MirrorValue::Double(50.0)),
                ("width", MirrorValue::Double(100.0)),
                ("height", MirrorValue::Double(100.0)),
                ("minimised", MirrorValue::Bool(true)),
            ],
        ),
    );
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bind(\"Super+MouseLeft+Drag\", new Probe(\"p\", \"Super+MouseLeft+Drag\"));\n"
        )),
    );
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 60.0, 70.0);
    rig.frame();
    assert_eq!(
        rig.log(),
        "p:start:3:50,60|p:update:60,70",
        "the window below takes the press: the one above is minimised"
    );
}

#[test]
fn a_maximised_client_declines_a_move_and_a_resize_and_the_press_is_consumed() {
    let comp = world(false);
    comp.borrow_mut()
        .nodes
        .get_mut(&3)
        .expect("window 3")
        .props
        .insert("maximised".into(), MirrorValue::Bool(true));
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    for chord in ["<Super>Button1", "<Super>Button3"] {
        let id = comp.claim_id(chord);
        comp.set_pointer(5, 50.0, 60.0);
        comp.chord(id, chord, "press");
        rig.frame();
        comp.set_pointer(5, 150.0, 160.0);
        let diagnostics = rig.frame();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        comp.chord(id, chord, "release");
        let diagnostics = rig.frame();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            (
                comp.prop(3, "x"),
                comp.prop(3, "y"),
                comp.prop(3, "width"),
                comp.prop(3, "height")
            ),
            (10.0, 20.0, 300.0, 200.0),
            "{chord} left the maximised window alone"
        );
    }
}

#[test]
fn the_compositors_cancel_calls_on_cancel_once_and_nothing_follows() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bind(\"Super+MouseLeft+Drag\", new Probe(\"p\", \"Super+MouseLeft+Drag\"));\n"
        )),
    );
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    rig.frame();
    comp.cancel_drag(id);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");
    // The drag is over: a later press starts a new one, and nothing is called twice.
    rig.frame();
    comp.set_pointer(5, 90.0, 90.0);
    rig.frame();
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");
}

#[test]
fn a_throwing_on_update_cancels_that_drag_only_and_logs_once() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}class Bad extends PointerDragBinding {{\n\
                 onStart() {{ return {{}}; }}\n\
                 onUpdate() {{ log.push(\"bad:update\"); throw new Error(\"nope\"); }}\n\
                 onEnd() {{ log.push(\"bad:end\"); }}\n\
                 onCancel() {{ log.push(\"bad:cancel\"); }}\n\
             }}\n\
             input.bind(\"Super+MouseLeft+Drag\", new Bad({{ id: \"bad\", trigger: \"Super+MouseLeft+Drag\" }}));\n\
             input.bind(\"Super+MouseRight+Drag\", new Probe(\"good\", \"Super+MouseRight+Drag\"));\n"
        )),
    );
    let (bad, good) = (comp.claim_id("<Super>Button1"), comp.claim_id("<Super>Button3"));
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(bad, "<Super>Button1", "press");
    comp.chord(good, "<Super>Button3", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    let diagnostics = rig.frame();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("nope"), "{diagnostics:?}");
    comp.set_pointer(5, 90.0, 100.0);
    let diagnostics = rig.frame();
    assert!(
        diagnostics.is_empty(),
        "the failure is not reported again: {diagnostics:?}"
    );
    comp.chord(bad, "<Super>Button1", "release");
    comp.chord(good, "<Super>Button3", "release");
    rig.frame();
    let log = rig.log();
    let count = |needle: &str| log.split('|').filter(|entry| *entry == needle).count();
    assert_eq!(count("bad:update"), 1, "{log}");
    assert_eq!(count("bad:cancel"), 1, "{log}");
    assert_eq!(count("bad:end"), 0, "{log}");
    assert_eq!(count("good:update:70,80"), 1, "{log}");
    assert_eq!(count("good:update:90,100"), 1, "{log}");
    assert_eq!(count("good:end:2"), 1, "the other drag ran to its end: {log}");
}

#[test]
fn the_stock_move_stays_in_the_work_area_and_a_minimum_strip_lets_the_window_leave_it() {
    for (constraint, far, near) in [
        ("work_area", (700.0, 600.0), (0.0, 0.0)),
        ("minimum_strip", (925.0, 725.0), (-225.0, -125.0)),
    ] {
        let comp = world(false);
        add_screen(&comp, 1000.0, 800.0);
        comp.borrow_mut()
            .nodes
            .get_mut(&5)
            .expect("the seat")
            .props
            .insert("drag_constraint".into(), text(constraint));
        let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
        let id = comp.claim_id("<Super>Button1");
        comp.set_pointer(5, 50.0, 60.0);
        comp.chord(id, "<Super>Button1", "press");
        rig.frame();
        comp.set_pointer(5, 2000.0, 2000.0);
        let diagnostics = rig.frame();
        assert!(diagnostics.is_empty(), "{constraint}: {diagnostics:?}");
        assert_eq!((comp.prop(3, "x"), comp.prop(3, "y")), far, "{constraint}, far corner");
        comp.set_pointer(5, -2000.0, -2000.0);
        rig.frame();
        assert_eq!(
            (comp.prop(3, "x"), comp.prop(3, "y")),
            near,
            "{constraint}, near corner"
        );
        assert_eq!((comp.prop(3, "width"), comp.prop(3, "height")), (300.0, 200.0));
        comp.chord(id, "<Super>Button1", "release");
        rig.frame();
    }
}

#[test]
fn the_drag_constraint_and_the_size_rules_agree_with_the_lua_drag_tests() {
    let got = grammar(
        r#"
        const area = { x: 0, y: 32, width: 1920, height: 1048 };
        const at = (x: number, y: number, width: number, height: number) => ({ x, y, width, height });
        const show = (r: any) => `${r.x},${r.y},${r.width},${r.height}`;
        const client = (extra: any = {}): any => ({ alive: true, fullscreen: false, bounds: at(10, 20, 300, 200), ...extra });
        const move = new ClientMoveBinding();
        const size = new ClientResizeBinding();
        const declines = (binding: any, c: any) =>
            String(binding.onStart({ client: c, position: { x: 20, y: 30 }, localPosition: { x: 10, y: 10 } }) === false);
        // Presses at `from`, drags to `to` and reports where the client ended up.
        const resize = (c: any, from: [number, number], to: [number, number]) => {
            const state = size.onStart({ client: c, position: { x: from[0], y: from[1] }, localPosition: { x: from[0] - c.bounds.x, y: from[1] - c.bounds.y } });
            size.onUpdate({ client: c, position: { x: to[0], y: to[1] } }, state);
            return show(c.bounds);
        };
        const slide = (c: any, to: [number, number], constraint: string) => {
            const state = move.onStart({ client: c, position: { x: 50, y: 60 } });
            move.onUpdate({ client: c, position: { x: to[0], y: to[1] }, workArea: at(0, 0, 1000, 800), dragConstraint: constraint }, state);
            return show(c.bounds);
        };
        emit([
            dragConstraintMode(undefined), dragConstraintMode("work_area"), dragConstraintMode("minimum_strip"),
            thrown(() => dragConstraintMode("loose")), thrown(() => dragConstraintMode(true)),
            show(constrainRect(at(-400, -400, 600, 400), area, "work_area")),
            show(constrainRect(at(1800, 1000, 600, 400), area, "work_area")),
            show(constrainRect(at(100, 200, 600, 400), area, "work_area")),
            show(constrainRect(at(500, 500, 2000, 1200), area, "work_area")),
            show(constrainRect(at(-5000, -5000, 600, 400), area, "minimum_strip")),
            show(constrainRect(at(5000, 5000, 600, 400), area, "minimum_strip")),
            show(constrainRect(at(100, 200, 600, 400), area, "minimum_strip")),
            declines(move, client()), declines(move, client({ fullscreen: true })), declines(move, client({ maximised: true })),
            declines(size, client()), declines(size, client({ fullscreen: true })), declines(size, client({ maximised: true })),
            resize(client(), [20, 30], [350, 230]),
            resize(client({ minSize: { width: 150, height: 120 } }), [20, 30], [350, 230]),
            resize(client({ maxSize: { width: 400, height: 300 } }), [300, 210], [800, 710]),
            resize(client({ maxSize: { width: 400, height: 300 } }), [20, 30], [-480, -470]),
            slide(client(), [2000, 2000], "work_area"),
            slide(client(), [2000, 2000], "minimum_strip"),
        ].join("\n"));
        "#,
    );
    assert_eq!(
        got,
        lines(&[
            "work_area",
            "work_area",
            "minimum_strip",
            "TypeError: drag_constraint must be work_area or minimum_strip, got loose [undefined]",
            "TypeError: drag_constraint must be work_area or minimum_strip, got true [undefined]",
            "0,32,600,400",
            "1320,680,600,400",
            "100,200,600,400",
            "0,32,2000,1200",
            "-525,-293,600,400",
            "1845,1005,600,400",
            "100,200,600,400",
            "false",
            "true",
            "true",
            "false",
            "true",
            "true",
            "210,140,100,80",
            "160,100,150,120",
            "10,20,400,300",
            "-90,-80,400,300",
            "700,600,300,200",
            "925,725,300,200",
        ])
    );
}

// -- touchpad gestures -------------------------------------------------------------------------------------

const GESTURE_PROBE: &str = "class GProbe extends GestureBinding {\n\
                                 label: string;\n\
                                 constructor(label: string, trigger: string) { super({ id: label, trigger }); this.label = label; }\n\
                                 onStart(e: any) { log.push(`${this.label}:start:${e.progress.toFixed(2)}`); return { n: 0 }; }\n\
                                 onUpdate(e: any, s: any) { s.n++; log.push(`${this.label}:update:${e.progress.toFixed(2)}`); }\n\
                                 onEnd(e: any, s: any) { log.push(`${this.label}:end:${e.progress.toFixed(2)}:${s.n}`); }\n\
                                 onCancel(e: any, s: any) { log.push(`${this.label}:cancel:${s.n}`); }\n\
                             }\n";

#[test]
fn a_directed_swipe_starts_when_its_direction_is_known_and_only_the_matching_binding_runs() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{GESTURE_PROBE}input.bind(\"Swipe3Left\", new GProbe(\"L\", \"Swipe3Left\"));\n\
             input.bind(\"Swipe3Right\", new GProbe(\"R\", \"Swipe3Right\"));\n"
        )),
    );
    assert_eq!(comp.claimed(), ["swipe"], "both directions share one gesture claim");
    assert_eq!(comp.borrow().calls[0].2, 3, "the finger count");
    let id = comp.claim_id("swipe");

    comp.gesture(id, "begin", 0.0, 0.0);
    rig.frame();
    assert_eq!(rig.log(), "", "no direction yet");
    comp.gesture(id, "update", -4.0, 1.0);
    rig.frame();
    assert_eq!(rig.log(), "", "still under the decision distance");
    comp.gesture(id, "update", -20.0, 2.0);
    rig.frame();
    comp.gesture(id, "update", -100.0, 5.0);
    rig.frame();
    comp.gesture(id, "end", -120.0, 5.0);
    rig.frame();
    assert_eq!(rig.log(), "L:start:0.05|L:update:0.05|L:update:0.25|L:end:0.30:2");

    // Rightwards: the other binding, from the start.
    comp.gesture(id, "begin", 0.0, 0.0);
    comp.gesture(id, "update", 30.0, 0.0);
    comp.gesture(id, "end", 40.0, 0.0);
    rig.frame();
    assert!(
        rig.log().ends_with("R:start:0.10|R:update:0.10|R:end:0.10:1"),
        "{}",
        rig.log()
    );
}

#[test]
fn gesture_updates_behind_are_merged_but_a_cancel_always_arrives_once() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{GESTURE_PROBE}input.bind(\"Swipe3Left\", new GProbe(\"L\", \"Swipe3Left\"));\n"
        )),
    );
    let id = comp.claim_id("swipe");
    comp.gesture(id, "begin", 0.0, 0.0);
    for dx in [-20.0, -40.0, -60.0, -80.0, -100.0] {
        comp.gesture(id, "update", dx, 0.0);
    }
    rig.frame();
    assert_eq!(
        rig.log(),
        "L:start:0.25|L:update:0.25",
        "five updates in one frame are one call that reads the last"
    );
    comp.gesture(id, "cancel", -100.0, 0.0);
    comp.gesture(id, "end", -100.0, 0.0);
    rig.frame();
    assert_eq!(
        rig.log(),
        "L:start:0.25|L:update:0.25|L:cancel:1",
        "the end after a cancel is nothing"
    );
}

#[test]
fn a_function_bound_to_a_gesture_fires_once_when_it_ends_far_enough_its_own_way() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry("input.bind(\"Swipe3Left\", (e: any) => { log.push(`left:${e.progress.toFixed(2)}`); });\n"),
    );
    let id = comp.claim_id("swipe");
    // Too short: 30 units is under the commit distance.
    comp.gesture(id, "begin", 0.0, 0.0);
    comp.gesture(id, "update", -30.0, 0.0);
    comp.gesture(id, "end", -30.0, 0.0);
    rig.frame();
    // The wrong way.
    comp.gesture(id, "begin", 0.0, 0.0);
    comp.gesture(id, "update", 90.0, 0.0);
    comp.gesture(id, "end", 90.0, 0.0);
    rig.frame();
    assert_eq!(rig.log(), "");
    comp.gesture(id, "begin", 0.0, 0.0);
    comp.gesture(id, "update", -60.0, 0.0);
    comp.gesture(id, "end", -80.0, 0.0);
    rig.frame();
    assert_eq!(rig.log(), "left:0.20");
}

// -- registration errors and reads -------------------------------------------------------------------------

#[test]
fn a_bad_registration_is_refused_with_a_message_that_says_why() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "const errors: string[] = [];\n\
             (globalThis as any).errors = errors;\n\
             const attempt = (fn: () => void) => { try { fn(); } catch (e: any) { errors.push(`${e.name}${e.code ? `(${e.code})` : \"\"}: ${e.message}`); } };\n\
             attempt(() => input.bind(\"Super+Wheel\", () => {}));\n\
             attempt(() => input.bind(\"Super+MouseLeft+Drag\", () => {}));\n\
             attempt(() => input.bind(\"Super+!\", () => {}));\n\
             attempt(() => input.bind(\"Super+z\", \"not a function\" as any));\n",
        ),
    );
    assert_eq!(
        rig.eval("errors.join('\\n')"),
        lines(&[
            r#"GnoblinError(unsupported): Binding "bind-1:Super+Wheel": the compositor has no claim for Super+Wheel; wheel triggers cannot be bound yet"#,
            r#"TypeError: InputContext "seatA".bind: "Super+MouseLeft+Drag" is a drag; bind a PointerDragBinding with onStart, onUpdate and onEnd"#,
            r#"TypeError: InputContext "seatA".bind: is a character, not a key name; write the keysym name (exclam, minus, plus) or Shift with the key (token "!" in "Super+!")"#,
            r#"TypeError: InputContext "seatA".bind: the action must be a function or a Binding"#,
        ])
    );
    assert!(comp.claimed().is_empty(), "a refused binding holds no claim");
}

#[test]
fn the_context_reads_through_to_the_seat() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry("(globalThis as any).touch = 0;\n"));
    assert_eq!(rig.eval("input.name"), "seatA");
    assert_eq!(rig.eval("input.focusedClient.id"), "3");
    assert_eq!(rig.eval("input.activeDisplay === null"), "true");
    assert_eq!(rig.eval("gnoblin.input.primary === input"), "true");
    assert_eq!(
        rig.eval("JSON.stringify(input.devices.map((d) => [d.id, d.name, d.capabilities.join()]))"),
        r#"[["7","Macro Pad","keyboard"]]"#
    );
    comp.set_pointer(5, 5.5, 6.5);
    comp.borrow_mut()
        .nodes
        .get_mut(&5)
        .expect("seat")
        .props
        .insert("focus".into(), MirrorValue::Node(0));
    FakeLink(comp.clone()).publish(vec![set_change(5, "focus", MirrorValue::Node(0))]);
    rig.frame();
    assert_eq!(rig.eval("JSON.stringify(input.pointer)"), r#"{"x":5.5,"y":6.5}"#);
    assert_eq!(rig.eval("input.focusedClient === null"), "true");
    assert_eq!(
        rig.eval("JSON.stringify(input.hitTest({ x: 20, y: 30 })?.localPosition)"),
        r#"{"x":10,"y":10}"#
    );
    assert_eq!(rig.eval("input.hitTest({ x: 900, y: 900 })"), "null");
}

// -- retiring the generation -------------------------------------------------------------------------------

#[test]
fn retiring_the_shell_cancels_what_is_running_once_and_releases_every_claim() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bind(\"Super+MouseLeft+Drag\", new Probe(\"p\", \"Super+MouseLeft+Drag\"));\n\
             input.bind(\"Super+k\", () => {{ log.push(\"key\"); }});\n"
        )),
    );
    assert_eq!(comp.claimed().len(), 2);
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    rig.frame();
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80");

    let report = rig.shell.retire().expect("retire runs");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");
    assert!(
        rig.comp.claimed().is_empty(),
        "every claim is released: {:?}",
        rig.comp.claimed()
    );
    // A second retire has nothing left to do.
    rig.shell.retire().expect("retire runs again");
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");
}

// -- the stock config as a runtime config ------------------------------------------------------------------

/// A shell over `comp` that has run no entry.
fn bare_shell(comp: &Comp) -> (JsShell, TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let shell = JsShell::new(Box::new(FakeLink(comp.clone())), JsShellConfig::new(&root)).expect("the shell boots");
    (shell, dir)
}

#[test]
fn with_no_entry_the_stock_config_runs_one_floating_desktop_and_claims_the_stock_keys() {
    let comp = world(false);
    let (mut shell, _dir) = bare_shell(&comp);
    let loaded = shell.load_default_config().expect("the stock config loads");
    assert!(loaded.is_empty(), "{loaded:?}");
    let frame = shell.frame().expect("a frame");
    assert!(frame.diagnostics.is_empty(), "{:?}", frame.diagnostics);

    assert_eq!(
        shell
            .eval("gnoblin.workspaces.map((w) => w.id + ':' + w.layout.constructor.name + ':' + w.active).join()")
            .expect("the script runs"),
        Some("desktop:FloatingLayout:true".to_owned())
    );
    let claimed = comp.claimed();
    for chord in [
        "<Super>1",
        "<Super>9",
        // The Lua shell's strip focus keys, and (below) its three-finger workspace slide.
        "<Super>Left",
        "<Super>Right",
        "XF86AudioMute",
        "XF86AudioPlay",
        // The floating layout's own move and resize bindings.
        "<Super>Button1",
        "<Super>Button3",
    ] {
        assert!(
            claimed.iter().any(|held| held.contains(chord)),
            "the stock config claims {chord}: {claimed:?}"
        );
    }
    assert!(
        claimed.iter().any(|held| held == "swipe"),
        "the stock config claims the three-finger swipe: {claimed:?}"
    );
    assert_eq!(
        comp.borrow()
            .calls
            .iter()
            .filter(|call| call.0 == "claim_gesture")
            .count(),
        1
    );
}

#[test]
fn a_defined_config_is_layered_over_the_stock_one_and_its_keys_are_claimed() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        "import { defineConfig, spawn } from \"gnoblin\";\n\
         export default defineConfig({\n\
         \x20   workspaces: [{ id: \"a\", layout: \"monocle\" }, { id: \"b\", layout: \"monocle\" }],\n\
         \x20   keys: { \"Super+Return\": spawn(\"true\"), \"XF86AudioMute\": spawn(\"true\") },\n\
         });\n",
    );
    assert_eq!(
        rig.eval("gnoblin.workspaces.map((w) => w.id).join()"),
        "a,b",
        "the user's workspaces replace the stock one"
    );
    let claimed = rig.comp.claimed();
    assert!(
        claimed.iter().any(|held| held.contains("<Super>Return")),
        "the user's key is claimed: {claimed:?}"
    );
    assert!(
        claimed.iter().any(|held| held.contains("XF86AudioPlay")),
        "a stock key the user did not mention survives: {claimed:?}"
    );
    assert!(
        claimed.iter().any(|held| held.contains("<Super>1")),
        "the stock Super+1 shows the workspace at position 1, whatever the user called it: {claimed:?}"
    );
    assert_eq!(
        claimed.iter().filter(|held| held.contains("XF86AudioMute")).count(),
        1,
        "a key the user redefined is claimed once: {claimed:?}"
    );
}

/// Three floating workspaces `a`, `b` and `c` on a 1000 by 800 screen; window 3 is on `a` and window 4 on `b`.
fn slide_rig(comp: &Comp) -> Rig {
    add_screen(comp, 1000.0, 800.0);
    let mut rig = rig(
        comp,
        "import { defineConfig } from \"gnoblin\";\n\
         export default defineConfig({ workspaces: [{ id: \"a\" }, { id: \"b\" }, { id: \"c\" }] });\n",
    );
    rig.eval("gnoblin.workspaces.get('b').clients.add(gnoblin.clients.at(1))");
    for _ in 0..3 {
        rig.frame();
    }
    rig
}

#[test]
fn the_stock_swipe_pulls_the_next_workspace_in_under_the_fingers_and_a_long_swipe_slides_on() {
    // The Lua shell's three-finger slide (shell_vm.rs "a three-finger swipe on the input path slides the stock shell to
    // the next workspace"): the fingers move left and the next workspace's windows follow them in before they lift;
    // lifting past a quarter of the travel finishes the slide: `b` is shown, its window slides in over the rest of
    // 0.25 seconds, and the windows of `a` leave and are hidden when that ends.
    let comp = world(false);
    let mut rig = slide_rig(&comp);
    assert_eq!(rig.eval("gnoblin.workspaces.where({ active: true }).at(0).id"), "a");
    assert_eq!(rig.eval("gnoblin.clients.at(0).visible"), "true");
    assert_eq!(
        rig.eval("gnoblin.clients.at(1).visible"),
        "false",
        "a workspace that is not shown hides its windows"
    );
    let (rest_a, rest_b) = (comp.prop(3, "x"), comp.prop(4, "x"));

    let id = comp.claim_id("swipe");
    comp.gesture(id, "begin", 0.0, 0.0);
    comp.gesture(id, "update", -300.0, 4.0);
    rig.frame();
    assert_eq!(
        comp.prop(3, "x"),
        rest_a - 750.0,
        "the shown workspace follows the fingers: 0.75 of 1000 to the left"
    );
    assert_eq!(
        comp.prop(4, "x"),
        rest_b + 250.0,
        "the next workspace comes in from the right"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.at(1).visible"),
        "true",
        "the workspace the fingers bring in is showing"
    );
    assert_eq!(rig.eval("gnoblin.workspaces.where({ active: true }).at(0).id"), "a");

    comp.gesture(id, "end", -300.0, 4.0);
    rig.frame();
    assert_eq!(
        rig.eval("gnoblin.workspaces.where({ active: true }).at(0).id"),
        "b",
        "lifting finished the slide"
    );
    rig.frame();
    assert_eq!(
        rig.eval("gnoblin.clients.at(0).visible"),
        "true",
        "the leaving windows stay drawn while they slide"
    );
    std::thread::sleep(Duration::from_millis(120));
    rig.frame();
    rig.frame();
    assert_eq!(
        rig.eval("gnoblin.clients.at(0).visible"),
        "false",
        "and are hidden when the slide ends"
    );
    assert_eq!(
        comp.prop(3, "x"),
        rest_a,
        "back at rest, so the next `show` starts from where the layout put it"
    );
    assert_eq!(rig.eval("gnoblin.clients.at(1).visible"), "true");
    assert_eq!(comp.prop(4, "x"), rest_b);
}

#[test]
fn a_short_swipe_slides_back_and_leaves_the_shown_workspace_alone() {
    // shell_vm.rs "a swipe that is short of the commit slides back": 50 of 400 units is under a quarter.
    let comp = world(false);
    let mut rig = slide_rig(&comp);
    let (rest_a, rest_b) = (comp.prop(3, "x"), comp.prop(4, "x"));
    let id = comp.claim_id("swipe");
    comp.gesture(id, "begin", 0.0, 0.0);
    comp.gesture(id, "update", -50.0, 0.0);
    rig.frame();
    assert_eq!(comp.prop(4, "x"), rest_b + 875.0);
    comp.gesture(id, "end", -50.0, 0.0);
    rig.frame();
    std::thread::sleep(Duration::from_millis(280));
    rig.frame();
    rig.frame();
    assert_eq!(rig.eval("gnoblin.workspaces.where({ active: true }).at(0).id"), "a");
    assert_eq!(comp.prop(3, "x"), rest_a);
    assert_eq!(comp.prop(4, "x"), rest_b);
    assert_eq!(
        rig.eval("gnoblin.clients.at(1).visible"),
        "false",
        "the workspace that was pulled in is hidden again"
    );
    assert_eq!(rig.eval("gnoblin.clients.at(0).visible"), "true");
}
