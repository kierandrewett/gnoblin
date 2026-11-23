//! The classes generated for every kind a catalog has, run against a library that serves one of each (`probe`): an enum, a record with
//! defaults and one without, a list, a closure property, an interface the script implements, a static function, a signal and a property
//! listener. The `probe` library is the one the typed Rust API is held to (`crates/gnoblin-object/fixtures/probe`), so a script and a Rust
//! shell are checked against the same behaviour.
//!
//! The module is the one `gnoblin-sdkgen` renders from the probe's catalog, embedded in the shell under the name `probe`, and the library is
//! served to the runtime beside the core and the std library. The library's turn is the test's: it tells what it queued inside the core's
//! dispatch, as the runtime's turn does, so a closure of the script is called from the library and not inside the call that asked for it.

mod common;
use common::Rig;

use gnoblin_fixture_object_probe::{OBJECTS_JS, last_error, live_objects_for, table, turn_for};

const IMPORTS: &str = r#"
import * as probe from "probe";
(globalThis as any).probe = probe;
"#;

/// The library's turn, entered the way the runtime's is.
fn turn(rig: &mut Rig) {
    let core = rig.core.as_ref().expect("a rig over a core");
    let ctx = core.ctx();
    core.embedded().enter(&rig.shell, || turn_for(ctx));
    rig.settle();
}

/// Runs statements, which have no value of their own.
fn run(rig: &mut Rig, source: &str) {
    rig.eval(&format!("{source}; 'done'"));
}

fn thrown(rig: &mut Rig, source: &str) -> String {
    rig.eval(&format!(
        "(() => {{ try {{ {source}; return 'none'; }} catch (e) {{ return e.name + ': ' + e.message + ' (' + e.code + ')'; }} }})()"
    ))
}

/// A rig whose probe library is served before the script runs (the module opens the namespace when it is evaluated).
fn served_rig(body: &str) -> Rig {
    use std::sync::Arc;
    let comp = gnoblin_testkit::compositor::FakeCompositor::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = std::fs::canonicalize(dir.path()).expect("the temp dir exists");
    std::fs::write(root.join("shell.ts"), format!("{IMPORTS}{body}\n")).expect("shell.ts is written");
    let (mut config, clock) = common::timed_config(&root);
    config.js = config.js.clone().embed("probe", OBJECTS_JS);
    let core = common::native::NativeCore::new(&comp, Arc::new(clock.clone()));
    core.embedded().serve_namespace("probe", table(), last_error);
    core.configure(&mut config);
    let mut shell = gnoblin_js::JsShell::new(core.link(), config).expect("the shell boots over the core");
    let load_diagnostics = shell
        .load_entry(std::path::Path::new("shell.ts"))
        .expect("shell.ts loads");
    let mut rig = Rig {
        shell,
        comp,
        load_diagnostics,
        clock,
        core: Some(core),
        _dir: dir,
    };
    rig.settle();
    rig
}

#[test]
fn an_enum_a_record_and_a_list_cross_as_a_scripts_own_values() {
    let mut rig = served_rig("(globalThis as any).p = new probe.Probe(3);");
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    // A constructor argument is a property that starts set, and an enum with a default starts at it and is read by its name.
    assert_eq!(rig.eval("p.count"), "3");
    assert_eq!(rig.eval("p.shade"), "light");
    run(&mut rig, "p.shade = 'half-tone'");
    assert_eq!(rig.eval("p.shade"), "half-tone");
    assert!(thrown(&mut rig, "p.shade = 'sideways'").contains("\"sideways\""));

    // A record the library has not been given reads as absent; one it was given comes back as it went in, with the defaults filled.
    assert_eq!(rig.eval("String(p.origin)"), "undefined");
    run(&mut rig, "p.origin = { x: 3, y: 4, label: 'here' }");
    assert_eq!(
        rig.eval("JSON.stringify(p.origin)"),
        r#"{"x":3,"y":4,"label":"here","shade":"dark"}"#
    );
    assert_eq!(rig.eval("p.reach({ x: 3, y: 4 })"), "5");
    assert_eq!(rig.eval("p.reach({})"), "0", "a field left out is its default");
    // A record with fields that are always there asks for them, and an optional one may be left out.
    assert_eq!(rig.eval("p.lengthOf({ start: 1, end: 4 })"), "3");
    assert_eq!(rig.eval("p.lengthOf({ start: 1, end: 4, closed: true })"), "4");
    assert!(thrown(&mut rig, "p.lengthOf({ start: 1 })").contains("Span.end is required"));
    assert_eq!(
        rig.eval("JSON.stringify(p.mark())"),
        r#"{"x":3,"y":1,"label":"mark","shade":"dark"}"#
    );

    // A list of records goes in, a list of text goes in and comes back.
    assert_eq!(
        rig.eval("p.total([{ x: 1.5 }, { x: 2, label: 'b', shade: 'light' }])"),
        "3.5"
    );
    assert_eq!(rig.eval("p.names.length"), "0");
    run(&mut rig, "p.names = ['a', 'b']");
    assert_eq!(rig.eval("p.names.join('+')"), "a+b");

    // An optional argument is left out or given, and a function of the class needs no object.
    assert_eq!(rig.eval("p.greet('ada')"), "hello, ada");
    assert_eq!(rig.eval("p.greet('ada', true)"), "HELLO, ADA!");
    assert_eq!(rig.eval("probe.Probe.clamp(15, 0, 10)"), "10");
    let refused = thrown(&mut rig, "probe.Probe.clamp(1, 5, 2)");
    assert!(
        refused.starts_with("TypeError:") && refused.contains("`low`"),
        "{refused}"
    );
}

#[test]
fn a_closure_a_signal_and_a_property_listener_are_told_from_the_turn_and_a_stopped_one_is_not() {
    let mut rig = served_rig(
        r#"
const p = new probe.Probe(0);
(globalThis as any).p = p;
(globalThis as any).seen = [] as number[];
(globalThis as any).told = 0;
(globalThis as any).counts = [] as number[];
p.onTick = (x: number) => { (globalThis as any).seen.push(x); return x * 10; };
(globalThis as any).ticked = p.on("ticked", (x: number) => { (globalThis as any).told += x; });
(globalThis as any).watching = p.on("countChanged", (count: number) => (globalThis as any).counts.push(count));
"#,
    );
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    run(&mut rig, "p.fire(4)");
    assert_eq!(
        rig.eval("seen.length"),
        "0",
        "the closure never runs inside the call that asked for it"
    );
    run(&mut rig, "p.count = 1; p.count = 2; p.count = 3");
    turn(&mut rig);
    assert_eq!(rig.eval("seen.join(',')"), "4");
    assert_eq!(
        rig.eval("p.last"),
        "40",
        "the closure's answer went back to the library"
    );
    assert_eq!(rig.eval("told"), "4");
    assert_eq!(
        rig.eval("counts.join(',')"),
        "3",
        "the changes of one turn are told once, with the last value"
    );

    run(&mut rig, "ticked.stop(); watching.stop(); p.fire(1); p.count = 9");
    turn(&mut rig);
    assert_eq!(rig.eval("told"), "4", "a stopped listener hears no more");
    assert_eq!(rig.eval("counts.join(',')"), "3");
    assert_eq!(rig.eval("seen.join(',')"), "4,1", "the closure property stays set");
}

#[test]
fn an_interface_the_script_extends_runs_only_the_methods_it_defined() {
    let mut rig = served_rig(
        r#"
class Plain extends probe.Paint { colour() { return "teal"; } }
class Sized extends probe.Paint {
    colour() { return "red"; }
    size() { return 3; }
    scaled(factor: number) { return factor * 2; }
}
const p = new probe.Probe(0);
(globalThis as any).p = p;
(globalThis as any).Plain = Plain;
(globalThis as any).Sized = Sized;
"#,
    );
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    run(&mut rig, "p.painter = new Plain(); p.paint()");
    turn(&mut rig);
    assert_eq!(
        rig.eval("p.lastColour"),
        "teal",
        "an absent optional method is not called"
    );
    run(&mut rig, "p.painter = new Sized(); p.paint()");
    turn(&mut rig);
    assert_eq!(
        rig.eval("p.lastColour"),
        "red 3",
        "a method the script defined is called"
    );
    assert_eq!(
        rig.eval("p.painter instanceof probe.Paint"),
        "true",
        "the painter reads back as an object of the interface"
    );
    // A script object that implements nothing the interface requires is refused by the library, naming the method.
    let missing = thrown(&mut rig, "p.painter = new probe.Paint()");
    assert!(missing.contains("colour"), "{missing}");
}

#[test]
fn an_object_ends_with_destroy_and_is_released_when_the_script_lets_go() {
    let mut rig = served_rig("(globalThis as any).p = new probe.Probe(0);");
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    run(&mut rig, "p.destroy()");
    let gone = thrown(&mut rig, "p.count");
    assert!(
        gone.starts_with("Error:") && gone.contains("(-6)"),
        "a destroyed object is stale: {gone}"
    );
}

/// The options of a constructor: a field is a property that starts set, in the call that makes the object. A field that is `undefined` or
/// left out is not sent, so the property keeps the library's default; a value the library refuses throws from the constructor, with the
/// library's words, and leaves no object behind; the constructor without options is the one that was there before.
#[test]
fn the_options_of_a_constructor_are_set_in_the_call_that_makes_the_object() {
    let mut rig = served_rig(
        r#"
(globalThis as any).seen = [] as number[];
(globalThis as any).p = new probe.Probe(2, {
    shade: "half-tone",
    origin: { x: 3, y: 4 },
    names: ["a", "b"],
    onTick: (x: number) => { (globalThis as any).seen.push(x); return x * 10; },
});
(globalThis as any).bare = new probe.Probe(1, {});
(globalThis as any).named = new probe.Probe(1, { names: ["z"], shade: undefined });
(globalThis as any).plain = new probe.Probe(5);
"#,
    );
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    assert_eq!(rig.eval("p.count"), "2", "the positional property is unchanged");
    assert_eq!(rig.eval("p.shade"), "half-tone");
    assert_eq!(rig.eval("p.origin.x + ',' + p.origin.y"), "3,4");
    assert_eq!(rig.eval("p.names.join('+')"), "a+b");
    run(&mut rig, "p.fire(4)");
    turn(&mut rig);
    assert_eq!(rig.eval("seen.join(',')"), "4", "the closure the options gave runs");
    assert_eq!(rig.eval("p.last"), "40");

    // A field left out, or undefined, keeps the library's default.
    assert_eq!(
        rig.eval("bare.count + ',' + bare.shade + ',' + String(bare.origin) + ',' + bare.names.length"),
        "1,light,undefined,0"
    );
    assert_eq!(rig.eval("named.shade + ',' + named.names.join('+')"), "light,z");
    assert_eq!(rig.eval("plain.count"), "5");

    // A value the library refuses throws, and no object is left.
    let ctx = rig.core.as_ref().expect("a rig over a core").ctx();
    let before = live_objects_for(ctx);
    let empty = thrown(&mut rig, "new probe.Probe(1, { shade: 'dark', names: [''] })");
    assert!(empty.contains("must not hold an empty name"), "{empty}");
    let sideways = thrown(&mut rig, "new probe.Probe(1, { shade: 'sideways' })");
    assert!(sideways.contains("\"sideways\""), "{sideways}");
    assert_eq!(
        live_objects_for(ctx),
        before,
        "a refused create leaves no half-made object"
    );
}
