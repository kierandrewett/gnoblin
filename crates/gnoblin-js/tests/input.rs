//! Tests for key bindings, gestures and drag bindings: the trigger grammar, `gnoblin.input`, the claims it
//! makes on the seat, and the stock move and resize bindings.
//!
//! The grammar is tested against the stock library alone (no host). Everything else runs a real `JsShell`
//! over the shared fake compositor of `tests/common`, an in-memory compositor that records the seat claims the shell
//! makes and plays back scripted chords, `pointer_chord` drags, gestures, pointer positions and window deaths. Like the
//! real one, it refuses a claim for a chord that another claim already holds (`Compositor::refused_claims` lists them),
//! so a shell that claims in the wrong order is seen. Nothing here proves that the real compositor takes these claims.
//! The real one is driven by `crates/gnoblin/tests/js_shell.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_js::{JsConfig, JsGeneration, JsRole, JsRoleLinks, JsShell, JsShellConfig, stock_modules_in};
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_tree_client::tree_mirror::{MirrorValue, TreeGesture, TreeIncoming};
use tempfile::TempDir;

mod common;
use common::{Claim, FakeCompositor, ROOT, honest_config, honest_role_config, native, set_change, text, timed_config};

// -- the trigger grammar -----------------------------------------------------------------------------------

const GRAMMAR_HARNESS: &str = r##"
globalThis.gnoblin = {};
export const emit = (value) => { globalThis.out = String(value); };
"##;

/// Evaluates `body` (TypeScript) with the stock library imported and returns what it passed to `emit`.
fn grammar(body: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let source = format!(
        "import {{ emit }} from \"harness\";\n\
         import {{ parseTrigger, claimOf, Binding, BindingCollection, KeyBinding, resolveTriggers }} from \"gnoblin-core\"; import {{ FloatingLayout, ClientMoveBinding, ClientResizeBinding, dragConstraintMode, constrainRect }} from \"gnoblin-std\";\n\
         const thrown = (fn: () => void) => {{ try {{ fn(); }} catch (e: any) {{ return `${{e.name}}: ${{e.message}} [${{e.token}}]`; }} return \"did not throw\"; }};\n\
         {body}\n"
    );
    fs::write(root.join("shell.ts"), source).expect("shell.ts is written");
    let mut config = JsConfig::new(&root).embed("harness", GRAMMAR_HARNESS);
    for (name, source) in stock_modules_in(&[std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js")])
        .expect("the stock library reads")
    {
        config = config.embed(name, source);
    }
    let generation = JsGeneration::new(&config, 1).expect("a generation");
    generation
        .eval_module(Path::new("shell.ts"))
        .expect("shell.ts evaluates");
    generation
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.globals().get::<_, String>("out"))
        .expect("the case called emit")
}

/// Like [`grammar`], in a shell over a core and the std library: for a case of a drag function that the library computes.
fn grammar_native(body: &str) -> String {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let source = format!(
        "import {{ ClientMoveBinding, ClientResizeBinding, dragConstraintMode, constrainRect }} from \"gnoblin-std\";\n\
         const emit = (value: unknown) => {{ (globalThis as any).out = String(value); }};\n\
         const thrown = (fn: () => void) => {{ try {{ fn(); }} catch (e: any) {{ return `${{e.name}}: ${{e.message}} [${{e.token}}]`; }} return \"did not throw\"; }};\n\
         {body}\n"
    );
    fs::write(root.join("shell.ts"), source).expect("shell.ts is written");
    let (config, clock) = timed_config(&root);
    let (mut shell, core) = native::shell_over_core(&FakeCompositor::new(), Arc::new(clock), config);
    let diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let out = shell
        .eval("globalThis.out")
        .expect("the script runs")
        .expect("the case called emit");
    drop(core);
    out
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

// -- the compositor's script --------------------------------------------------------------------------------

/// The shared fake compositor of `tests/common`, with the claims it refuses when another live claim on the seat holds
/// the same chord, gesture or device, and the words these cases use for what the compositor does while the shell runs.
#[derive(Clone)]
struct Comp(FakeCompositor);

/// What the compositor holds of the shell's seat claims, as [`Comp::borrow`] reads it.
struct Held {
    /// Every claim call in the order it arrived: `(method, id, second argument, text)`.
    calls: Vec<(String, i64, i64, String)>,
    /// The seat claims the shell holds.
    claims: Vec<Claim>,
    /// The seat node each live claim was made on.
    claim_seats: BTreeMap<i64, u64>,
    /// The claims the compositor refused because another live claim holds the same chord, gesture or device:
    /// `(method, text)`.
    refused: Vec<(String, String)>,
}

impl Comp {
    fn borrow(&self) -> Held {
        let c = self.0.borrow();
        Held {
            calls: c.claim_calls.clone(),
            claims: c.claims.clone(),
            claim_seats: c.claims.iter().map(|claim| (claim.id, claim.seat)).collect(),
            refused: c.refused_claims.clone(),
        }
    }

    /// The fake's own state, for a test that writes a property before the shell attaches.
    fn borrow_mut(&self) -> std::cell::RefMut<'_, common::Compositor> {
        self.0.borrow_mut()
    }

    /// Moves the pointer of `seat`. A pointer chord drag on that seat streams an `update` with the new place.
    fn set_pointer(&self, seat: u64, x: f64, y: f64) {
        self.0.set_pointer(seat, x, y);
    }

    /// A key chord's `press` or `release`. A pointer chord (it ends in `ButtonN`) starts its drag at the seat's
    /// pointer on `press` and ends it there on `release`, as the compositor's `pointer_chord` signal does.
    fn chord(&self, claim: i64, chord: &str, state: &str) {
        if chord.contains("Button") {
            self.0.drag(claim, chord, state);
        } else {
            self.0.chord(claim, chord, state);
        }
    }

    /// The compositor ending a live drag with `cancel`.
    fn cancel_drag(&self, claim: i64) {
        self.0.cancel_drag(claim);
    }

    fn gesture(&self, claim: i64, phase: &str, dx: f64, dy: f64) {
        self.0.push(TreeIncoming::Gesture(TreeGesture {
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
        self.0.destroy(node);
    }

    fn claim_id(&self, text: &str) -> i64 {
        self.0.claim_id(text)
    }

    fn claimed(&self) -> Vec<String> {
        self.0.claimed()
    }

    fn prop(&self, node: u64, key: &str) -> f64 {
        match self.0.value(node, key) {
            Some(MirrorValue::Double(value)) => value,
            other => panic!("node {node} has no double {key}: {other:?}"),
        }
    }
}

fn seat_node(comp: &FakeCompositor, id: u64, name: &str, devices: &[&str]) {
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
                MirrorValue::Texts(devices.iter().map(|d| (*d).to_owned()).collect()),
            ),
        ],
    );
}

/// A root, a workspace (2), window 3 at 10,20 sized 300x200 with the focus, window 4 with no size, and seats
/// `seatA` (5, with a macro pad) and, when `second`, `seatB` (6). Like the real compositor, it refuses a claim for a
/// chord that another live claim on the seat holds.
fn world(second: bool) -> Comp {
    let c = FakeCompositor::new();
    c.refuse_duplicate_claims();
    c.add_node(
        2,
        ROOT,
        "workspace",
        vec![("index", MirrorValue::Uint(0)), ("active", MirrorValue::Bool(true))],
    );
    c.add_node(
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
    c.add_node(
        4,
        ROOT,
        "window",
        vec![("title", text("web")), ("app_id", text("firefox"))],
    );
    seat_node(&c, 5, "seatA", &["7:Macro Pad:keyboard"]);
    if second {
        seat_node(&c, 6, "seatB", &[]);
    }
    Comp(c)
}

struct Rig {
    shell: JsShell,
    comp: Comp,
    clock: ManualClock,
    /// The core and the std library the shell runs over: the stock bindings and the snap rules are the library's.
    core: native::NativeCore,
    _dir: TempDir,
}

impl Rig {
    /// Moves the shell's clock forward by `ms` milliseconds: a slide, a fade or a delay is over when the test says so,
    /// not when the machine got there.
    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    fn frame(&mut self) -> Vec<String> {
        let diagnostics = self.shell.frame().expect("a frame").diagnostics;
        self.core.dispatch(&self.shell);
        self.core.dispatch(&self.shell);
        diagnostics
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
    fs::write(root.join("shell.ts"), init).expect("shell.ts is written");
    let (mut config, clock) = timed_config(&root);
    tweak(&mut config);
    let (mut shell, core) = native::shell_over_core(&comp.0, Arc::new(clock.clone()), config);
    let diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut rig = Rig {
        shell,
        comp: comp.clone(),
        clock,
        core,
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
const PRELUDE: &str = "import { gnoblin, BindingCollection, KeyBinding, PointerDragBinding, GestureBinding } from \"gnoblin-core\"; import { FloatingLayout, MonocleLayout, MoveBinding, ResizeBinding, Workspace } from \"gnoblin-std\";\n\
                       class Act extends KeyBinding { fn: any; constructor(fn: any) { super(); this.fn = fn; } onPress(e: any) { return this.fn(e); } }\n\
                       const log: string[] = [];\n\
                       (globalThis as any).log = log;\n\
                       gnoblin.workspaces.add(new Workspace(\"main\", { layout: new MonocleLayout() }));\n\
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
    // The claim's id is the library's own number now, not the fake compositor's first: the test reads it back.
    assert_eq!((method.as_str(), grab), ("claim", 0), "a key chord is not grabbed");

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
             (globalThis as any).high = input.bindBinding(\n\
                 \"Super+k\",\n\
                 new Act(() => { log.push(\"high\"); }),\n\
                 { id: \"high\", priority: 5 },\n\
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
fn a_throwing_handler_does_not_stop_the_others() {
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
    rig.frame();
    assert_eq!(rig.log(), "b|c");
}

#[test]
#[ignore = "vrnw: TS internal: a throwing native-dispatched handler is logged by the library, not in the frame's diagnostics (the TS input stage reported it once per site)"]
fn a_throwing_handler_is_reported_once() {
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
            "const layout = new BindingCollection();\n\
             layout.bindBinding(\"Super+x\", new Act(() => { log.push(\"layout\"); }), { id: \"layout\" });\n\
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
fn a_device_bound_with_only_is_claimed_once_and_its_chords_are_claimed_for_it() {
    // Before: "Seat:bind with only = true makes the device binding-only once, and refuses only without a device".
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry("const pad = input.devices[0];\n(globalThis as any).pad = pad;\n"),
    );
    let before = comp.borrow().calls.len();
    // A refused bind sends no call: `only: false` is not a way to say nothing.
    assert_eq!(
        rig.eval("try { pad.bind('F13', () => {}, { only: false }); 'bound' } catch (e) { `${e.name}:${e.message}` }"),
        "TypeError:InputDevice \"Macro Pad\".bind: only must be true to make the device binding-only, got false"
    );
    assert_eq!(comp.borrow().calls.len(), before, "a refused bind sent a call");
    assert_eq!(rig.eval("String(pad.bindingOnly)"), "false");

    rig.eval("pad.bind('F13', () => {}, { only: true }); pad.bind('F14', () => {}, { only: true }); 'ok'");
    rig.frame();
    let calls: Vec<(String, String)> = comp.borrow().calls[before..]
        .iter()
        .map(|call| (call.0.clone(), call.3.replace('\n', "|")))
        .collect();
    assert_eq!(
        calls,
        [
            ("claim_device".to_owned(), "Macro Pad".to_owned()),
            ("claim".to_owned(), "Macro Pad|F13".to_owned()),
            ("claim".to_owned(), "Macro Pad|F14".to_owned()),
        ],
        "the device is claimed once, by its bare name, then each chord for it"
    );
    assert_eq!(rig.eval("String(pad.bindingOnly)"), "true");
    assert_eq!(rig.eval("String(input.isBindingOnly('Macro Pad'))"), "true");
    assert_eq!(comp.borrow().claims.len(), 3, "the claim's id is held with the chords'");
}

#[test]
fn binding_only_is_given_up_by_assigning_false_and_by_the_shell_going() {
    // New: retired kept the id in `seat.data.only` for `seat:unclaim`; the object has a property for it.
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry("const pad = input.devices[0];\n(globalThis as any).pad = pad;\npad.bindingOnly = true;\n"),
    );
    assert_eq!(comp.claimed(), ["Macro Pad"]);
    rig.eval("pad.bindingOnly = true; 'again'");
    rig.frame();
    assert_eq!(comp.claimed(), ["Macro Pad"], "a second assignment claims nothing more");
    rig.eval("pad.bindingOnly = false; 'off'");
    rig.frame();
    assert!(comp.claimed().is_empty(), "false gives the claim up");
    assert_eq!(comp.borrow().calls.iter().filter(|c| c.0 == "unclaim").count(), 1);
    assert_eq!(rig.eval("String(input.releaseBindingOnly('Macro Pad'))"), "false");
    assert_eq!(
        rig.eval("try { pad.bindingOnly = 1; 'set' } catch (e) { e.message }"),
        "`Keyboard.bindingOnly` must be a boolean, got an integer"
    );
}

#[test]
fn a_device_name_is_one_non_empty_line_and_a_refused_claim_sends_nothing() {
    // Before: "Seat:claim_device is a one-line device name".
    let comp = world(false);
    let mut rig = rig(&comp, &entry(""));
    let before = comp.borrow().calls.len();
    for bad in ["''", "'two\\nlines'", "7"] {
        let outcome = rig.eval(&format!(
            "try {{ input.makeBindingOnly({bad}); 'claimed' }} catch (e) {{ e.name }}"
        ));
        assert_eq!(outcome, "TypeError", "{bad}");
    }
    assert_eq!(comp.borrow().calls.len(), before, "a refused name sent a call");
    rig.eval("input.makeBindingOnly('Not Plugged In'); 'ok'");
    rig.frame();
    assert_eq!(
        comp.claimed(),
        ["Not Plugged In"],
        "a device that is not there yet can be claimed"
    );
}

#[test]
fn a_false_action_claims_the_trigger_and_swallows_it_and_a_function_that_returns_false_declines() {
    // Before: "a gesture claimed with a fn of false claims it and runs nothing", and `seat:bind({ chord = false })`.
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "input.bind('Super+z');\n\
             input.bind('Swipe5');\n\
             input.bind('Super+d', () => false);\n\
             input.bindBinding('Super+d', new Act(() => { log.push('second'); }), { id: 'second', priority: -1 });\n",
        ),
    );
    assert_eq!(
        comp.claimed(),
        ["<Super>z", "swipe", "<Super>d"],
        "the chord, the gesture and the declining chord are all claimed"
    );
    comp.chord(comp.claim_id("<Super>z"), "<Super>z", "press");
    let swipe = comp.claim_id("swipe");
    comp.gesture(swipe, "begin", 0.0, 0.0);
    comp.gesture(swipe, "update", 5.0, 5.0);
    comp.gesture(swipe, "end", 400.0, 0.0);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.log(), "", "nothing ran for the swallowed chord and gesture");
    comp.chord(comp.claim_id("<Super>d"), "<Super>d", "press");
    rig.frame();
    assert_eq!(
        rig.log(),
        "second",
        "a handler that returns false lets the next binding have the chord"
    );
}

#[test]
fn device_beats_layout_beats_context_and_an_equal_priority_conflict_is_refused_at_registration() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "const layout = new BindingCollection();\n\
             layout.bindBinding(\"Super+x\", new Act(() => { log.push(\"layout\"); }), { id: \"layout\" });\n\
             input.attachLayout(layout, () => true);\n\
             input.bind(\"Super+x\", () => { log.push(\"context\"); });\n\
             input.devices[0].bind(\"Super+x\", () => { log.push(\"device\"); });\n\
             const errors: string[] = [];\n\
             (globalThis as any).errors = errors;\n\
             try { input.bind(\"Super+x\", () => {}); } catch (e: any) { errors.push(`${e.name}:${e.message}`); }\n\
             try { layout.bindBinding(\"super+X\", new Act(() => {}), { id: \"other\" }); } catch (e: any) { errors.push(`${e.name}:${e.message}`); }\n\
             input.bindBinding(\"Super+x\", new Act(() => { log.push(\"urgent\"); }), { id: \"urgent\", priority: 9 });\n",
        ),
    );
    assert_eq!(
        rig.eval("errors.join('\\n')"),
        lines(&[
            r#"GnoblinError:InputContext#seatA.bind: binding "bind-3:Super+x" conflicts with "bind-1:Super+x": both match Super+x at priority 0; give one a different priority or trigger"#,
            r#"GnoblinError:BindingCollection.bindBinding: binding "other" conflicts with "layout": both match Super+x at priority 0; give one a different priority or trigger"#,
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
                              gnoblin.workspaces.byId(\"main\")!.layout = layout;\n\
                              (globalThis as any).layout = layout;\n\
                              (globalThis as any).move = new MoveBinding({ trigger: \"Super+MouseLeft+Drag\" });\n\
                              (globalThis as any).resize = new ResizeBinding({ trigger: \"Super+MouseRight+Drag\" });\n\
                              (globalThis as any).move.install();\n\
                              (globalThis as any).resize.install();\n";

#[test]
fn a_super_left_drag_moves_the_client_by_the_pointer_delta_and_a_release_keeps_it() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    assert_eq!(comp.claimed(), ["<Super>Button1", "<Super>Button3"]);
    let id = comp.claim_id("<Super>Button1");
    assert_eq!(
        comp.borrow().calls.iter().map(|c| c.2).collect::<Vec<_>>(),
        [1, 1],
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
}

#[test]
fn a_released_drag_tells_the_layout_where_the_user_put_the_client() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry(FLOATING_ENTRY));
    let id = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(id, "<Super>Button1", "press");
    rig.frame();
    comp.set_pointer(5, 90.0, 110.0);
    rig.frame();
    comp.chord(id, "<Super>Button1", "release");
    rig.frame();
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
                         n: number = 0;\n\
                         constructor(label: string) { super(); this.label = label; }\n\
                         onStart(e: any) { this.n = 0; log.push(`${this.label}:start:${e.client?.id}:${e.position.x},${e.position.y}`); return true; }\n\
                         onUpdate(e: any) { this.n++; log.push(`${this.label}:update:${e.position.x},${e.position.y}`); }\n\
                         onEnd(e: any) { log.push(`${this.label}:end:${this.n}`); }\n\
                         onCancel(e: any) { log.push(`${this.label}:cancel:${this.n}`); }\n\
                     }\n";

#[test]
fn a_window_dying_mid_drag_calls_on_cancel_once_and_the_drag_follows_nothing_after() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bindDrag(\"Super+MouseLeft+Drag\", new Probe(\"p\"));\n"
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
#[ignore = "vrnw: TS internal: the generated FloatingLayout has no bindings of its own; move and resize are installed by MoveBinding/ResizeBinding, so a layout change cancels nothing"]
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
    rig.eval("gnoblin.workspaces.byId('main').layout = new MonocleLayout()");
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
    rig.eval("gnoblin.workspaces.byId('main').layout = new (globalThis.layout.constructor)()");
    rig.frame();
    assert_eq!(comp.claimed(), ["<Super>Button1", "<Super>Button3"]);
}

#[test]
fn an_unbound_or_moved_binding_cancels_the_drag_it_runs_once() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}const probe = input.bindDrag(\"Super+MouseLeft+Drag\", new Probe(\"p\"));\n\
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
#[ignore = "vrnw: TS internal: the frame budget and the merging of a late drag update were the TS input stage's; native dispatch has neither"]
fn a_drag_that_is_behind_reads_the_latest_pointer_and_never_queues_a_second_update() {
    let comp = world(false);
    let mut rig = rig_with(
        &comp,
        &entry(&format!(
            "{PROBE}input.bindDrag(\"Super+MouseLeft+Drag\", new Probe(\"p\"));\n\
             input.bind(\"Super+k\", () => {{ const t = Date.now(); while (Date.now() - t < 600) {{}} log.push(\"slow\"); }});\n"
        )),
        // The slow handler outlasts the frame budget by a wide margin, and every other frame uses a small part of it, so
        // no frame of this test can be the one a busy machine tips over.
        |config| config.frame_budget = Duration::from_millis(300),
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
                 context.bindDrag(\"Super+MouseLeft+Drag\", new Probe(context.name));\n\
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
                                 onStart(e: any) { log.push(`start:${e.client?.id}:${e.position.x},${e.position.y}:${e.dx},${e.dy}:b${e.button}:${[...e.modifiers].join('+')}:${e.context === input}:${e.trigger}`); return true; }\n\
                                 onUpdate(e: any) { log.push(`update:${e.position.x},${e.position.y}:${e.dx},${e.dy}:${e.position.x},${e.position.y}`); }\n\
                                 onEnd(e: any) { log.push(`end:${e.position.x},${e.position.y}:${e.dx},${e.dy}`); }\n\
                                 onCancel(e: any) { log.push(`cancel:${e.position.x},${e.position.y}`); }\n\
                             }\n";

/// A screen of `width` by `height`, which is the work area of a drag.
fn add_screen(comp: &Comp, width: f64, height: f64) {
    comp.0.add_node(
        7,
        ROOT,
        "screen",
        vec![
            ("width", MirrorValue::Double(width)),
            ("height", MirrorValue::Double(height)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
}

#[test]
fn a_pointer_chord_stream_calls_start_update_update_end_with_the_context_of_the_drag() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{CONTEXT_PROBE}input.bindDrag(\"Super+MouseLeft+Drag\", new Ctx());\n"
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
    comp.0.add_node(
        8,
        ROOT,
        "window",
        vec![
            ("title", text("on top")),
            ("x", MirrorValue::Double(40.0)),
            ("y", MirrorValue::Double(50.0)),
            ("width", MirrorValue::Double(100.0)),
            ("height", MirrorValue::Double(100.0)),
        ],
    );
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bindDrag(\"Super+MouseLeft+Drag\", new Probe(\"p\"));\n"
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
    comp.0.add_node(
        8,
        ROOT,
        "window",
        vec![
            ("x", MirrorValue::Double(40.0)),
            ("y", MirrorValue::Double(50.0)),
            ("width", MirrorValue::Double(100.0)),
            ("height", MirrorValue::Double(100.0)),
            ("minimised", MirrorValue::Bool(true)),
        ],
    );
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bindDrag(\"Super+MouseLeft+Drag\", new Probe(\"p\"));\n"
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
            "{PROBE}input.bindDrag(\"Super+MouseLeft+Drag\", new Probe(\"p\"));\n"
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
fn a_throwing_on_update_cancels_that_drag_only() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}class Bad extends PointerDragBinding {{\n\
                 onStart() {{ return true; }}\n\
                 onUpdate() {{ log.push(\"bad:update\"); throw new Error(\"nope\"); }}\n\
                 onEnd() {{ log.push(\"bad:end\"); }}\n\
                 onCancel() {{ log.push(\"bad:cancel\"); }}\n\
             }}\n\
             input.bindDrag(\"Super+MouseLeft+Drag\", new Bad());\n\
             input.bindDrag(\"Super+MouseRight+Drag\", new Probe(\"good\"));\n"
        )),
    );
    let (bad, good) = (comp.claim_id("<Super>Button1"), comp.claim_id("<Super>Button3"));
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(bad, "<Super>Button1", "press");
    comp.chord(good, "<Super>Button3", "press");
    rig.frame();
    comp.set_pointer(5, 70.0, 80.0);
    rig.frame();
    comp.set_pointer(5, 90.0, 100.0);
    rig.frame();
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
#[ignore = "vrnw: TS internal: a throwing native-dispatched drag handler is logged by the library, not in the frame's diagnostics"]
fn a_throwing_on_update_is_reported_once() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "class Bad extends PointerDragBinding {\n\
                 onStart() { return true; }\n\
                 onUpdate() { throw new Error(\"nope\"); }\n\
             }\n\
             input.bindDrag(\"Super+MouseLeft+Drag\", new Bad());\n",
        ),
    );
    let bad = comp.claim_id("<Super>Button1");
    comp.set_pointer(5, 50.0, 60.0);
    comp.chord(bad, "<Super>Button1", "press");
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
#[ignore = "vrnw: TS internal: ClientMoveBinding/ClientResizeBinding.onStart/onUpdate, dragConstraintMode and constrainRect are data/js functions; the generated MoveBinding/ResizeBinding have only the native step"]
fn the_drag_constraint_and_the_size_rules_agree_with_the_retired_drag_tests() {
    let got = grammar_native(
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
                                 n: number = 0;\n\
                                 constructor(label: string) { super(); this.label = label; }\n\
                                 onBegin(e: any) { this.n = 0; log.push(`${this.label}:start:${e.progress.toFixed(2)}`); return true; }\n\
                                 onUpdate(e: any) { this.n++; log.push(`${this.label}:update:${e.progress.toFixed(2)}`); }\n\
                                 onEnd(e: any) { log.push(`${this.label}:end:${e.progress.toFixed(2)}:${this.n}`); }\n\
                                 onCancel(e: any) { log.push(`${this.label}:cancel:${this.n}`); }\n\
                             }\n";

#[test]
fn a_directed_swipe_starts_when_its_direction_is_known_and_only_the_matching_binding_runs() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{GESTURE_PROBE}input.bindGesture(\"Swipe3Left\", new GProbe(\"L\"));\n\
             input.bindGesture(\"Swipe3Right\", new GProbe(\"R\"));\n"
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
            "{GESTURE_PROBE}input.bindGesture(\"Swipe3Left\", new GProbe(\"L\"));\n"
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
             attempt(() => input.bind(\"Super+!\", () => {}));\n\
             attempt(() => input.bind(\"Super+z\", \"not a function\" as any));\n",
        ),
    );
    // The texts are the generated classes' own: the `InputContext#seatA.bind:` site prefix, `expected a function, got string`,
    // and no `invalid_argument` code on the TypeError (see the report).
    assert_eq!(
        rig.eval("errors.join('\\n')"),
        lines(&[
            r#"GnoblinError(unsupported): InputContext#seatA.bind: Binding "bind-1:Super+Wheel": the compositor has no claim for Super+Wheel; wheel triggers cannot be bound yet"#,
            r#"TypeError: InputContext "seatA".bind: is a character, not a key name; write the keysym name (exclam, minus, plus) or Shift with the key (token "!" in "Super+!")"#,
            r#"TypeError: expected a function, got string"#,
        ])
    );
    assert!(comp.claimed().is_empty(), "a refused binding holds no claim");
}

#[test]
fn a_drag_trigger_bound_to_a_plain_function_is_accepted_and_claims_the_chord() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(
            "const errors: string[] = [];\n\
             (globalThis as any).errors = errors;\n\
             try { input.bind(\"Super+MouseLeft+Drag\", () => {}); } catch (e: any) { errors.push(`${e.name}: ${e.message}`); }\n",
        ),
    );
    // A function serves a drag as it serves any trigger (the shipped examples bind one); the TypeScript library refused it.
    assert_eq!(rig.eval("errors.join('\\n')"), "");
    assert!(!comp.claimed().is_empty(), "the function holds the claim of the drag");
}

#[test]
fn the_context_reads_through_to_the_seat() {
    let comp = world(false);
    let mut rig = rig(&comp, &entry("(globalThis as any).touch = 0;\n"));
    assert_eq!(rig.eval("input.name"), "seatA");
    assert_eq!(rig.eval("input.focusedClient.id"), "3");
    assert_eq!(rig.eval("input.activeDisplay == null"), "true");
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
    comp.0.publish(vec![set_change(5, "focus", MirrorValue::Node(0))]);
    rig.frame();
    assert_eq!(rig.eval("JSON.stringify(input.pointer)"), r#"{"x":5.5,"y":6.5}"#);
    assert_eq!(rig.eval("input.focusedClient == null"), "true");
    assert_eq!(
        rig.eval("JSON.stringify(input.hitTest({ x: 20, y: 30 })?.localPosition)"),
        r#"{"x":10,"y":10}"#
    );
    assert_eq!(rig.eval("String(input.hitTest({ x: 900, y: 900 }) ?? null)"), "null");
}

// -- retiring the generation -------------------------------------------------------------------------------

/// The shell ends as the loader ends it (`NativeModule::end`, then `retire`): the core's shutdown hooks run while the script is still
/// there, so the drag in flight hears `onCancel` once. The claims are not given back here: on exit the compositor drops them with the
/// connection, and on a reload the next generation's take-over releases them
/// (`a_reload_releases_the_old_claims_before_it_claims_the_new_ones_so_no_chord_is_refused_or_left_unheld`).
#[test]
fn ending_the_generation_cancels_what_is_running_once() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        &entry(&format!(
            "{PROBE}input.bindDrag(\"Super+MouseLeft+Drag\", new Probe(\"p\"));\n\
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

    assert!(
        rig.core.close(&rig.shell) > 0,
        "the generation's registrations are removed"
    );
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");
    // Ending it again has nothing left to do.
    assert_eq!(rig.core.close(&rig.shell), 0, "the generation is already closed");
    assert_eq!(rig.log(), "p:start:3:50,60|p:update:70,80|p:cancel:1");
}

// -- the stock config as a runtime config ------------------------------------------------------------------

/// A shell over `comp` that has run no entry.
fn bare_shell(comp: &Comp) -> (native::CoreShell, TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let shell = native::CoreShell::new(&comp.0, honest_config(&root));
    (shell, dir)
}

#[test]
fn with_no_entry_the_stock_config_runs_four_floating_workspaces_and_claims_the_stock_keys() {
    let comp = world(false);
    let (mut shell, _dir) = bare_shell(&comp);
    let loaded = shell.load_default_config().expect("the stock config loads");
    assert!(loaded.is_empty(), "{loaded:?}");
    shell.frame_with_core();
    let diagnostics = shell.frame_with_core();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    assert_eq!(
        shell
            .eval("gnoblin.workspaces.list.map((w) => w.id + ':' + w.layout.constructor.name + ':' + w.active).join()")
            .expect("the script runs"),
        Some("1:FloatingLayout:true,2:FloatingLayout:false,3:FloatingLayout:false,4:FloatingLayout:false".to_owned()),
        "the retired stock config's four workspaces, the first on show"
    );
    assert_eq!(
        shell
            .eval("gnoblin.workspaces.list.map((w) => w.name).join()")
            .expect("the script runs"),
        Some("Workspace 1,Workspace 2,Workspace 3,Workspace 4".to_owned())
    );
    let claimed = comp.claimed();
    for chord in [
        "<Super>1",
        "<Super>9",
        // The retired shell's strip focus keys, and (below) its three-finger workspace slide.
        "<Super>Left",
        "<Super>Right",
        "XF86AudioMute",
        "XF86AudioPlay",
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
fn the_stock_config_claims_the_floating_layouts_move_and_resize() {
    let comp = world(false);
    let (mut shell, _dir) = bare_shell(&comp);
    let loaded = shell.load_default_config().expect("the stock config loads");
    assert!(loaded.is_empty(), "{loaded:?}");
    shell.frame_with_core();
    shell.frame_with_core();
    let claimed = comp.claimed();
    for chord in ["<Super>Button1", "<Super>Button3"] {
        assert!(
            claimed.iter().any(|held| held.contains(chord)),
            "the stock config claims {chord}: {claimed:?}"
        );
    }
}

/// The stock config starts the window-state policy, so a window that asks to be maximised is maximised. Without it the request
/// has no answer: the native library answers window requests only once `WindowState` is started.
#[test]
fn the_stock_config_answers_a_window_that_asks_to_be_maximised() {
    let comp = world(false);
    comp.0.add_node(
        20,
        ROOT,
        "screen",
        vec![
            ("width", MirrorValue::Double(1000.0)),
            ("height", MirrorValue::Double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    let (mut shell, _dir) = bare_shell(&comp);
    let loaded = shell.load_default_config().expect("the stock config loads");
    assert!(loaded.is_empty(), "{loaded:?}");
    for _ in 0..4 {
        shell.frame_with_core();
    }
    comp.0.request(3, "maximize", 0.0, 0.0);
    for _ in 0..6 {
        shell.frame_with_core();
    }
    assert_eq!(
        comp.0.flag(3, "maximised"),
        Some(true),
        "the stock config answers the request"
    );
}

#[test]
fn a_user_init_replaces_the_stock_default_and_claims_only_its_own_keys() {
    let comp = world(false);
    let mut rig = rig(
        &comp,
        "import { gnoblin } from \"gnoblin-core\"; import { Workspace, MonocleLayout } from \"gnoblin-std\";\n\
         gnoblin.workspaces.add(new Workspace(\"a\", { layout: new MonocleLayout() }));\n\
         gnoblin.workspaces.add(new Workspace(\"b\", { layout: new MonocleLayout() }));\n\
         gnoblin.input.primary.bind(\"Super+Return\", () => gnoblin.spawn([\"true\"]));\n\
         gnoblin.input.primary.bind(\"XF86AudioMute\", () => gnoblin.spawn([\"true\"]));\n",
    );
    assert_eq!(
        rig.eval("gnoblin.workspaces.list.map((w) => w.id).join()"),
        "a,b",
        "the user's workspaces are the only ones"
    );
    let claimed = rig.comp.claimed();
    assert!(
        claimed.iter().any(|held| held.contains("<Super>Return")),
        "the user's key is claimed: {claimed:?}"
    );
    assert_eq!(
        claimed.iter().filter(|held| held.contains("XF86AudioMute")).count(),
        1,
        "a key the user binds is claimed once: {claimed:?}"
    );
    assert!(
        !claimed.iter().any(|held| held.contains("XF86AudioPlay")),
        "a stock key the user did not bind is not there: {claimed:?}"
    );
    assert!(
        !claimed.iter().any(|held| held.contains("<Super>1")),
        "nor is the stock Super+1: {claimed:?}"
    );
}

/// Three floating workspaces `a`, `b` and `c` on a 1000 by 800 screen with the stock three-finger slide bound; window 3
/// is on `a` and window 4 on `b`.
fn slide_rig(comp: &Comp) -> Rig {
    add_screen(comp, 1000.0, 800.0);
    let mut rig = rig(
        comp,
        "import { gnoblin } from \"gnoblin-core\"; import { FloatingLayout, Workspace, WorkspaceSlideBinding } from \"gnoblin-std\";\n\
         for (const id of [\"a\", \"b\", \"c\"]) gnoblin.workspaces.add(new Workspace(id, { layout: new FloatingLayout() }));\n\
         new WorkspaceSlideBinding({ trigger: \"Swipe3\" }).install();\n",
    );
    rig.eval("gnoblin.clients.at(1).moveToWorkspace(gnoblin.workspaces.byId('b'))");
    // The window the compositor mapped with no place has none the library would give it (a window that was there when the shell
    // opened is not placed): the test gives it the rest place the slide goes back to.
    for (key, value) in [("x", 20.0), ("y", 40.0), ("width", 300.0), ("height", 200.0)] {
        comp.0.external_set(4, key, MirrorValue::Double(value));
    }
    for _ in 0..3 {
        rig.frame();
    }
    rig
}

#[test]
fn the_stock_swipe_pulls_the_next_workspace_in_under_the_fingers_and_a_long_swipe_slides_on() {
    // The retired shell's three-finger slide (the retired the retired shell-rig test "a three-finger swipe on the input path slides the stock shell to
    // the next workspace"): the fingers move left and the next workspace's windows follow them in before they lift;
    // lifting past a quarter of the travel finishes the slide: `b` is shown, its window slides in over the rest of
    // 0.25 seconds, and the windows of `a` leave and are hidden when that ends.
    let comp = world(false);
    let mut rig = slide_rig(&comp);
    assert_eq!(rig.eval("gnoblin.workspaces.list.filter((w) => w.active)[0].id"), "a");
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
    assert_eq!(rig.eval("gnoblin.workspaces.list.filter((w) => w.active)[0].id"), "a");

    comp.gesture(id, "end", -300.0, 4.0);
    rig.frame();
    assert_eq!(
        rig.eval("gnoblin.workspaces.list.filter((w) => w.active)[0].id"),
        "b",
        "lifting finished the slide"
    );
    rig.frame();
    assert_eq!(
        rig.eval("gnoblin.clients.at(0).visible"),
        "true",
        "the leaving windows stay drawn while they slide"
    );
    rig.advance(120);
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
    rig.advance(280);
    rig.frame();
    rig.frame();
    assert_eq!(rig.eval("gnoblin.workspaces.list.filter((w) => w.active)[0].id"), "a");
    assert_eq!(comp.prop(3, "x"), rest_a);
    assert_eq!(comp.prop(4, "x"), rest_b);
    assert_eq!(
        rig.eval("gnoblin.clients.at(1).visible"),
        "false",
        "the workspace that was pulled in is hidden again"
    );
    assert_eq!(rig.eval("gnoblin.clients.at(0).visible"), "true");
}

// -- a reload hands the claims over --------------------------------------------------------------------------

/// A role over the fake compositor whose entry is `init`, the directory it reads and the directory's guard.
fn role_over(comp: &Comp, init: &str) -> (JsRole, std::path::PathBuf, (TempDir, native::NativeCore)) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), init).expect("shell.ts");
    let mut config = honest_role_config(&root);
    config.watch = false;
    // The shell's library is the std library over a core of its own, as the runtime's is.
    let core = native::NativeCore::new(&comp.0, Arc::clone(&config.shell.clock));
    config.shell.js.natives.push(core.std_native());
    config.shell.js.natives.push(core.embedded().object_native());
    let role = JsRole::over_link(core.link(), JsRoleLinks::default(), config).expect("a role");
    (role, root, (dir, core))
}

/// An entry with one monocle workspace (a layout with no bindings of its own), then `rest`.
fn hand_off_entry(rest: &str) -> String {
    format!(
        "import {{ gnoblin, BindingCollection, PointerDragBinding }} from \"gnoblin-core\"; import {{ FloatingLayout, MonocleLayout, Workspace }} from \"gnoblin-std\";\n\
         class Nop extends PointerDragBinding {{ onStart() {{ return true; }} }}\n\
         const dragLayout = () => {{ const l = new BindingCollection(); l.bindDrag(\"Super+MouseLeft+Drag\", new Nop()); l.bindDrag(\"Super+MouseRight+Drag\", new Nop()); return l; }};\n\
         if (!gnoblin.workspaces.byId(\"main\")) gnoblin.workspaces.add(new Workspace(\"main\", {{ layout: new MonocleLayout() }}));\n\
         const input = gnoblin.input.primary;\n{rest}"
    )
}

/// What the fake compositor holds: the chords, gestures and devices other than pointer chords, sorted, and how many
/// pointer chords there are (their text is the compositor's own spelling of a button).
fn held(comp: &Comp) -> (Vec<String>, usize) {
    let mut all = comp.claimed();
    let pointer = all.iter().filter(|text| text.contains("Button")).count();
    all.retain(|text| !text.contains("Button"));
    all.sort();
    (all, pointer)
}

fn sorted(items: &[&str]) -> Vec<String> {
    let mut items: Vec<String> = items.iter().map(|item| (*item).to_owned()).collect();
    items.sort();
    items
}

/// One frame of the role, then the core's half of it, twice: what a frame reads is delivered to the library's callbacks a
/// dispatch later, so a claim a binding makes is held after the second.
fn pump(role: &mut JsRole, core: &native::NativeCore) {
    role.tick().expect("a frame");
    core.dispatch(role.shell_mut());
    core.dispatch(role.shell_mut());
    role.tick().expect("a frame");
}

/// Reloads the role and checks the swap's own transaction: the compositor holds exactly `expected` once the core has run
/// what the swap queued, none of its claims was refused, and every release came before every claim, which is what keeps a
/// chord from being refused for the old generation still holding it.
#[track_caller]
fn reload_and_check(
    role: &mut JsRole,
    core: &mut native::NativeCore,
    root: &Path,
    comp: &Comp,
    source: &str,
    expected: (Vec<String>, usize),
) {
    let mark = comp.borrow().calls.len();
    fs::write(root.join("shell.ts"), source).expect("shell.ts is rewritten");
    let report = core.reload(role);
    assert!(report.ok, "{:?}", report.errors);
    pump(role, core);
    assert_eq!(
        held(comp),
        expected,
        "the claims the compositor holds once the swap is sent"
    );
    let calls: Vec<String> = comp.borrow().calls[mark..].iter().map(|call| call.0.clone()).collect();
    let last_release = calls.iter().rposition(|method| method == "unclaim");
    let first_claim = calls.iter().position(|method| method != "unclaim");
    assert!(
        last_release
            .zip(first_claim)
            .is_none_or(|(release, claim)| release < claim),
        "a claim was sent before a release: {calls:?}"
    );
    assert!(
        comp.borrow().refused.is_empty(),
        "the compositor refused claims: {:?}",
        comp.borrow().refused
    );
    pump(role, core);
    assert_eq!(held(comp), expected, "the claims after the next frame");
}

#[test]
fn a_reload_releases_the_old_claims_before_it_claims_the_new_ones_so_no_chord_is_refused_or_left_unheld() {
    let comp = world(false);
    let all = "input.bind(\"Super+1\", () => {});\n\
               input.bind(\"Super+2\", () => {});\n\
               input.bind(\"Swipe3\", () => {});\n\
               input.attachLayout(dragLayout(), () => true);\n\
               input.devices[0].bindingOnly = true;\n";
    let (mut role, root, mut guard) = role_over(&comp, &hand_off_entry(all));
    pump(&mut role, &guard.1);
    assert_eq!(
        held(&comp),
        (sorted(&["<Super>1", "<Super>2", "Macro Pad", "swipe"]), 2),
        "the first generation's claims: two chords, the swipe, the device and the layout's move and resize"
    );
    assert!(comp.borrow().refused.is_empty());

    // Two chords stay and one goes; a chord is added; the swipe, the device and the layout's drags stay.
    reload_and_check(
        &mut role,
        &mut guard.1,
        &root,
        &comp,
        &hand_off_entry(
            "input.bind(\"Super+1\", () => {});\n\
             input.bind(\"Super+3\", () => {});\n\
             input.bind(\"Swipe3\", () => {});\n\
             input.attachLayout(dragLayout(), () => true);\n\
             input.devices[0].bindingOnly = true;\n",
        ),
        (sorted(&["<Super>1", "<Super>3", "Macro Pad", "swipe"]), 2),
    );

    // Everything that is not a chord goes: the device, the swipe and the layout are released, not left behind.
    reload_and_check(
        &mut role,
        &mut guard.1,
        &root,
        &comp,
        &hand_off_entry("input.bind(\"Super+1\", () => {});\n"),
        (sorted(&["<Super>1"]), 0),
    );

    // The same file again: every claim is made again behind its own release.
    reload_and_check(
        &mut role,
        &mut guard.1,
        &root,
        &comp,
        &hand_off_entry("input.bind(\"Super+1\", () => {});\n"),
        (sorted(&["<Super>1"]), 0),
    );

    // Five in a row that swap one chord for another.
    for round in 4..9 {
        reload_and_check(
            &mut role,
            &mut guard.1,
            &root,
            &comp,
            &hand_off_entry(&format!(
                "input.bind(\"Super+1\", () => {{}});\ninput.bind(\"Super+{round}\", () => {{}});\n"
            )),
            (sorted(&["<Super>1", &format!("<Super>{round}")]), 0),
        );
    }
}

#[test]
fn a_reload_that_fails_after_it_claimed_leaves_the_claims_as_they_were_and_the_next_reload_claims_the_chord() {
    let comp = world(false);
    let (mut role, root, mut guard) = role_over(&comp, &hand_off_entry("input.bind(\"Super+1\", () => {});\n"));
    pump(&mut role, &guard.1);
    let before = comp.borrow().calls.len();

    // The entry claims `Super+9` and then throws: the stage is dropped, and so is the claim it made.
    fs::write(
        root.join("shell.ts"),
        hand_off_entry("input.bind(\"Super+9\", () => {});\nthrow new Error(\"late\");\n"),
    )
    .expect("shell.ts");
    let report = guard.1.reload(&mut role);
    assert!(!report.ok);
    pump(&mut role, &guard.1);
    assert_eq!(held(&comp), (sorted(&["<Super>1"]), 0));
    assert_eq!(
        comp.borrow().calls.len(),
        before,
        "a failed stage sent no claim and no release: {:?}",
        &comp.borrow().calls[before..]
    );

    // The chord is free to claim: the failed stage left no record of it that would stand in for a claim.
    reload_and_check(
        &mut role,
        &mut guard.1,
        &root,
        &comp,
        &hand_off_entry("input.bind(\"Super+1\", () => {});\ninput.bind(\"Super+9\", () => {});\n"),
        (sorted(&["<Super>1", "<Super>9"]), 0),
    );
}
