//! The classes generated from the catalog of `libgnoblin-std` (`gnoblin-std/generated/objects`), run over the real core and the real std
//! library in this process: a class is made, read, written, called and listened to through `__gnoblinObjects`, and nothing of the classes is
//! written by hand.
//!
//! What it proves is the path from a script's value to the library and back: a record that goes in and one that comes out, an enum by its
//! name, an optional argument left out, a signal told from the library's turn and not inside the call that caused it, a listener that is
//! stopped, an object that has ended, and the exceptions of a value the catalog does not take.

mod common;
use common::{Rig, rig_with};

const IMPORTS: &str = r#"
import * as model from "gnoblin-core/generated/objects";
import { EntryModel, FocusPolicy } from "gnoblin-std/generated/objects";
(globalThis as any).model = model;
(globalThis as any).EntryModel = EntryModel;
(globalThis as any).FocusPolicy = FocusPolicy;
"#;

/// Runs statements, which have no value of their own.
fn run(rig: &mut Rig, source: &str) {
    rig.eval(&format!("{source}; 'done'"));
}

/// What the script's expression `source` throws, as `name: message (code)`, or `none`.
fn thrown(rig: &mut Rig, source: &str) -> String {
    rig.eval(&format!(
        "(() => {{ try {{ {source}; return 'none'; }} catch (e) {{ return e.name + ': ' + e.message + ' (' + e.code + ')'; }} }})()"
    ))
}

#[test]
fn an_entry_model_is_made_read_edited_and_its_changes_are_heard_from_the_next_turn() {
    let mut rig = rig_with(
        IMPORTS,
        r#"
const entry = new EntryModel("héllo");
(globalThis as any).entry = entry;
(globalThis as any).heard = [] as string[];
entry.on("changed", () => (globalThis as any).heard.push("changed"));
entry.on("textChanged", (text: string) => (globalThis as any).heard.push("text " + text));
"#,
    );
    rig.settle();
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    // Made with its text: positions count characters, the caret starts at the end, and a selection that is empty reads as absent.
    assert_eq!(rig.eval("entry.length"), "5");
    assert_eq!(rig.eval("entry.caret"), "5");
    assert_eq!(rig.eval("String(entry.selection)"), "undefined");
    assert_eq!(rig.eval("heard.length"), "0");

    // A call changes the entry at once and the library tells the signal from its next turn, never inside the call.
    run(&mut rig, "entry.insert(', world')");
    assert_eq!(rig.eval("entry.text"), "héllo, world");
    assert_eq!(
        rig.eval("heard.length"),
        "0",
        "the signal is not told inside the call that caused it"
    );
    rig.settle();
    assert_eq!(rig.eval("heard.filter((line) => line === 'changed').length"), "1");
    assert_eq!(rig.eval("heard.includes('text héllo, world')"), "true");

    // A record comes back as an object with the catalog's fields; an optional argument is left out.
    run(&mut rig, "entry.selectAll()");
    assert_eq!(rig.eval("JSON.stringify(entry.selection)"), r#"{"start":0,"end":12}"#);
    assert_eq!(rig.eval("entry.selectedText"), "héllo, world");
    run(&mut rig, "entry.end()");
    assert_eq!(rig.eval("String(entry.selection)"), "undefined");
    run(&mut rig, "entry.home(true)");
    assert_eq!(rig.eval("JSON.stringify(entry.selection)"), r#"{"start":0,"end":12}"#);
    assert_eq!(rig.eval("entry.before(5)"), "héllo");

    // A property is written like any other, and the new text moves the caret to its end.
    run(&mut rig, "entry.text = 'abc'");
    assert_eq!(rig.eval("entry.caret"), "3");

    // A listener that is stopped hears no more, one that is not goes on.
    run(
        &mut rig,
        "globalThis.stopped = entry.on('changed', () => heard.push('stopped'))",
    );
    rig.settle();
    run(&mut rig, "heard.length = 0; entry.insert('d')");
    rig.settle();
    assert_eq!(rig.eval("heard.filter((line) => line === 'stopped').length"), "1");
    run(
        &mut rig,
        "stopped.stop(); stopped.stop(); heard.length = 0; entry.insert('e')",
    );
    rig.settle();
    assert_eq!(rig.eval("heard.filter((line) => line === 'stopped').length"), "0");
    assert_eq!(rig.eval("heard.filter((line) => line === 'changed').length"), "1");
    assert_eq!(rig.comp.refused(), 0);
}

#[test]
fn the_core_serves_its_own_namespace_through_its_own_module() {
    let mut rig = rig_with(IMPORTS, "");
    rig.settle();
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    // The classes of `gnoblin.model` are the core's, in a module of their own: the module opened the core's table and has its classes.
    assert_eq!(rig.eval("typeof model.Client"), "function");
    assert_eq!(rig.eval("typeof model.ClientCollection"), "function");
}

#[test]
fn the_focus_policy_takes_records_and_enums_by_name_and_answers_a_record() {
    let mut rig = rig_with(IMPORTS, "");
    rig.settle();
    // Two records and an enum by its name go in; a record comes out, and a field it leaves out is absent.
    assert_eq!(
        rig.eval("JSON.stringify(FocusPolicy.focusDecision({ mode: 'sloppy' }, { kind: 'enter', held: false }))"),
        r#"{"focus":true,"clear":false,"raise":false}"#
    );
    // A field the caller leaves out has the catalog's default: a new window from a launcher is not focused unless the policy allows it.
    assert_eq!(rig.eval("FocusPolicy.opensFocused({}, true)"), "false");
    assert_eq!(
        rig.eval("FocusPolicy.opensFocused({ newWindows: 'allow' }, true)"),
        "true"
    );
    assert_eq!(
        rig.eval("FocusPolicy.hoverFocus({ mode: 'follow-mouse' }, true, false)"),
        "false"
    );
    assert_eq!(
        rig.eval("FocusPolicy.hoverFocus({ mode: 'follow-mouse' }, true, true)"),
        "true"
    );
    assert_eq!(
        rig.eval("FocusPolicy.restFocus({ mode: 'sloppy', changeOnPointerRest: true })"),
        "true"
    );
}

#[test]
fn a_value_the_catalog_does_not_take_is_an_exception_that_names_it() {
    let mut rig = rig_with(IMPORTS, "(globalThis as any).entry = new EntryModel('abc');");
    rig.settle();
    let wrong_enum = thrown(&mut rig, "FocusPolicy.restFocus({ mode: 'sideways' })");
    assert!(
        wrong_enum.starts_with("TypeError:") && wrong_enum.contains("\"sideways\""),
        "{wrong_enum}"
    );
    assert!(
        wrong_enum.contains("click, sloppy, follow-mouse"),
        "the values it does take are named: {wrong_enum}"
    );
    let missing = thrown(&mut rig, "FocusPolicy.focusDecision({}, {})");
    assert!(
        missing.starts_with("TypeError:") && missing.contains("FocusEvent.kind is required"),
        "{missing}"
    );
    let not_a_record = thrown(&mut rig, "FocusPolicy.restFocus(3)");
    assert!(
        not_a_record.starts_with("TypeError:") && not_a_record.contains("FocusSettings"),
        "{not_a_record}"
    );
    // The library's own refusal: a text where a number is wanted comes back with its code.
    let refused = thrown(&mut rig, "entry.move('far')");
    assert!(
        refused.starts_with("TypeError:") && refused.contains("`at`"),
        "{refused}"
    );
    // An unknown event names the ones there are.
    let event = thrown(&mut rig, "entry.on('nothing', () => {})");
    assert!(event.contains("\"nothing\"") && event.contains("changed"), "{event}");
    // An object that has ended answers a GnoblinError with the library's code (a word, `not_found`) and not a crash.
    run(&mut rig, "entry.destroy()");
    let gone = thrown(&mut rig, "entry.text");
    assert!(
        gone.starts_with("GnoblinError:") && gone.ends_with("(not_found)"),
        "{gone}"
    );
    assert_eq!(rig.comp.refused(), 0);
}

#[test]
fn a_failure_of_a_generated_class_is_the_gnoblin_error_of_the_package_too() {
    // A shell that imports the package entry and the generated classes runs on the TypeScript library, with both modules. The
    // failure of a generated member is one `GnoblinError`, whichever module the shell took the class from.
    let mut rig = rig_with(
        r#"
import { GnoblinError as PackageError } from "gnoblin-core";
import { GnoblinError as ObjectsError } from "gnoblin-std/generated/objects";
import { EntryModel } from "gnoblin-std/generated/objects";
(globalThis as any).PackageError = PackageError;
(globalThis as any).ObjectsError = ObjectsError;
(globalThis as any).entry = new EntryModel("abc");
"#,
        "",
    );
    rig.settle();
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    run(&mut rig, "entry.destroy()");
    let caught = rig.eval(
        "(() => { try { entry.text; return 'none'; } catch (e) { \
         return [e instanceof PackageError, e instanceof ObjectsError, e.code].join(' '); } })()",
    );
    assert_eq!(caught, "true true not_found");
    assert_eq!(rig.eval("PackageError === ObjectsError"), "true");
}

#[test]
fn a_listener_that_raises_does_not_stop_the_listeners_after_it() {
    let mut rig = rig_with(
        IMPORTS,
        r#"
const entry = new EntryModel("a");
(globalThis as any).entry = entry;
(globalThis as any).ran = 0;
entry.on("changed", () => { throw new Error("a listener that fails"); });
entry.on("changed", () => { (globalThis as any).ran += 1; });
"#,
    );
    rig.settle();
    run(&mut rig, "entry.insert('b')");
    rig.settle();
    assert_eq!(rig.eval("ran"), "1", "the second listener ran after the first raised");
    run(&mut rig, "entry.insert('c')");
    rig.settle();
    assert_eq!(rig.eval("ran"), "2", "and the library still tells the signal");
}

#[test]
fn the_library_object_of_a_released_slot_is_no_longer_reachable_from_the_script() {
    let mut rig = rig_with(IMPORTS, "(globalThis as any).entry = new EntryModel('abc');");
    rig.settle();
    // The generated base releases a slot when its object is collected; a released slot is gone from the generation's table.
    run(
        &mut rig,
        "globalThis.slot = entry[Object.getOwnPropertySymbols(entry)[0]]; __gnoblinObjects.release(slot)",
    );
    let after = thrown(&mut rig, "__gnoblinObjects.get(slot, 1)");
    assert!(after.contains("released or ended"), "{after}");
}

/// What a call from a script costs, in nanoseconds, for the calls the object protocol times from Rust. The script's own loop is
/// timed and taken off. A measurement and not a check, so it is ignored: run it in a release build,
/// `cargo test -p gnoblin-js --release --test object_classes -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "a measurement: run it in a release build with --ignored --nocapture"]
fn what_a_call_from_a_script_costs() {
    const ROUNDS: u32 = 200_000;
    let mut rig = rig_with(
        IMPORTS,
        "(globalThis as any).entry = new EntryModel('hello, world, and more text');",
    );
    rig.settle();
    let time = |rig: &mut Rig, body: &str| -> f64 {
        let script = format!("(() => {{ for (let i = 0; i < {ROUNDS}; i++) {{ {body}; }} return 'done'; }})()");
        let start = std::time::Instant::now();
        rig.eval(&script);
        start.elapsed().as_nanos() as f64 / f64::from(ROUNDS)
    };
    let empty = time(&mut rig, "void 0");
    println!("the script's own loop: {empty:.0} ns");
    for (what, body) in [
        ("get of a number", "entry.length"),
        ("get of text", "entry.text"),
        ("get of a record (absent)", "entry.selection"),
        ("set of text", "entry.text = 'hello, world, and more text'"),
        ("a method with no arguments that changes nothing", "entry.home()"),
        (
            "two calls that each change the entry (insert and backspace)",
            "entry.insert('a'); entry.backspace()",
        ),
        (
            "a static function with a record and a boolean",
            "FocusPolicy.opensFocused({}, true)",
        ),
        (
            "a static function with two records, answering a record",
            "FocusPolicy.focusDecision({ mode: 'sloppy' }, { kind: 'enter', held: false })",
        ),
    ] {
        let cost = time(&mut rig, body) - empty;
        println!("{what}: {cost:.0} ns");
    }
}
