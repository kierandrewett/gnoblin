//! What the library has to register so that gating never leaves a change undrawn (`src/shell/gate.rs`).
//!
//! A frame runs when something arrived, script ran, or a source the library registered (`data/js/gnoblin-core/wake.ts`) says
//! it is due. The first two are the shell's and are guarded where the links are (`frame_gating.rs`, `host_api.rs`,
//! `host_idle.rs`, `config_slots.rs`). The third is the library's own, and the way it goes wrong is a part that keeps a
//! clock and forgets to register: its deadline passes with no frame to see it. These tests make that a failure, from three
//! sides, so that a new part has to say what it is:
//!
//! - every field of `HostContext`, the one place the shell's subsystems live, is classified here, and a subsystem that
//!   holds a clock names the source it registers;
//! - the sources the stock shell registers are exactly the ones this file expects, in both directions;
//! - every file of the library that reads a clock registers a source or says here why it needs none.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod common;
use common::policy::*;
use common::*;

/// What a subsystem of the shell is to the gate.
enum Part {
    /// State read and written by code that a frame already runs. It has no clock and writes nothing on its own.
    State,
    /// Has a deadline or draws on every frame while it holds, and registers it under this owner.
    Wakes(&'static str),
    /// Writes the tree or a link, but only in answer to something that arrived or that a source above wakes for. The
    /// reason says what.
    Driven(&'static str),
}

use Part::{Driven, State, Wakes};

/// The starter, `shells/default/shell.ts`: what the packaged default shell is.
const STARTER_CONFIG: &str = include_str!("../../../shells/default/shell.ts");

/// Every field of `HostContext` (`data/js/gnoblin-core/host/context.ts`). A new field fails
/// `every_subsystem_of_the_host_context_is_classified` until it has a row: decide whether it can change the tree or hold
/// a clock on its own, and if it can, register a wake source for it and name the source.
const CONTEXT: &[(&str, Part)] = &[
    ("queue", Wakes("listener queue")),
    ("reports", State),
    ("notices", State),
    ("noticed", State),
    ("verdicts", State),
    ("states", State),
    ("outputSeen", State),
    ("collections", State),
    ("clientsByNode", State),
    ("outputsByNode", State),
    ("preexisting", State),
    ("tracked", State),
    (
        "applications",
        Driven("an output application is answered by the compositor, which is a message on the link"),
    ),
    ("flags", State),
    ("clientRoot", State),
    ("outputRoot", State),
    (
        "input",
        Driven("a drag samples the seat's pointer, which the compositor reports as a message"),
    ),
    ("dialogs", Driven("the offers of the dialog slot, which wake a frame")),
    (
        "idleMonitor",
        Driven("the readings of the idle connection, which wake a frame"),
    ),
    (
        "authentication",
        Driven("a polkit request arrives through the dialog slot"),
    ),
    ("launches", Wakes("launch deadlines")),
    ("hostApi", Wakes("timers")),
];

/// Every field of `StdContext` (`data/js/gnoblin-std/host/state.ts`), classified the same way: the desktop policy the
/// package installs on the object model.
const STD_CONTEXT: &[(&str, Part)] = &[
    ("wsStates", State),
    ("membership", State),
    ("byHandle", State),
    ("closed", State),
    ("frame", State),
    ("insets", State),
    ("insetSent", State),
    ("scopeStyles", State),
    ("styledSelections", State),
    ("clientStyles", State),
    ("focusPolicy", Wakes("focus rests")),
    ("decorations", Wakes("window close fade")),
    ("flags", State),
    (
        "staging",
        Driven("holds the writes of the frame that made them until it sends them"),
    ),
    ("profileRoot", State),
    ("workspaceRoot", State),
    ("inherited", State),
    ("displayRoot", State),
    ("displayManager", State),
    ("gestureTools", State),
    ("xrRuntime", Wakes("xr session")),
    (
        "transitions",
        Driven("a transition ends on the compositor's `track_ended`, which is a message"),
    ),
];

/// The parts that register a source and are not a field of `HostContext`: a module-level singleton or a widget.
const MODULES: &[&str] = &[
    "lock screen",
    "toasts",
    "not-responding alert",
    "window drag limit",
    "clock widgets",
];

/// The stock library as a user's `shell.ts` starts it, with the widgets that are loaded on demand and the two modules the
/// test reads.
const STOCK: &str = "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\n\
    const { gnoblin: _runtime, ...exported } = lib;\n\
    Object.assign(globalThis, exported);\n";

/// The frames a rig runs on boot, and nothing they say.
fn settled(mut rig: Rig) -> Rig {
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// The stock shell over three decorated windows.
fn stock() -> Rig {
    settled(rig_setup(STOCK, STARTER_CONFIG, |comp| populate(comp, 3, false)))
}

/// The stock shell as the runtime role runs it: its shell holds the connection for the configuration document, which a frame
/// reads to see whether a script wrote a device setting.
fn stock_with_document() -> Rig {
    settled(rig_prepared(
        STOCK,
        STARTER_CONFIG,
        |comp| populate(comp, 3, false),
        |_| {},
        |shell| shell.set_config_link(Box::new(AcceptingDocument::default())),
    ))
}

fn names(rig: &mut Rig, expression: &str) -> BTreeSet<String> {
    let listing = rig.eval(&format!("({expression}).join('\\n')"));
    listing.lines().map(str::to_owned).collect()
}

#[test]
#[ignore = "vrnw: TS internal: classifies the fields of the TS library's HostContext and StdContext (`gnoblin-core/host/context`, `gnoblin-std/host/state`), which the generated classes do not have"]
fn every_subsystem_of_the_host_context_is_classified() {
    let mut rig = stock();
    for (context, table, name) in [("ctx", CONTEXT, "HostContext"), ("std", STD_CONTEXT, "StdContext")] {
        let held = names(&mut rig, &format!("Object.keys({context})"));
        let listed: BTreeSet<String> = table.iter().map(|(field, _)| (*field).to_owned()).collect();
        let new: Vec<_> = held.difference(&listed).collect();
        let gone: Vec<_> = listed.difference(&held).collect();
        assert!(
            new.is_empty() && gone.is_empty(),
            "{name} gained {new:?} and lost {gone:?}: give each new field a row in its table (State, Wakes or Driven), \
             and delete the rows of fields that are gone"
        );
    }
}

#[test]
#[ignore = "vrnw: TS internal: lists the wake sources the TS library registers (`wakeOwners()` of `gnoblin-core/wake`); the generated library has no such registry to read"]
fn the_stock_shell_registers_a_source_for_every_part_that_keeps_a_clock_and_for_nothing_else() {
    let mut rig = stock();
    let registered = names(&mut rig, "wakeOwners()");
    let mut expected: BTreeSet<String> = MODULES.iter().map(|owner| (*owner).to_owned()).collect();
    for (field, part) in CONTEXT.iter().chain(STD_CONTEXT) {
        if let Wakes(owner) = part {
            assert!(!owner.is_empty(), "{field}: a part that keeps a clock names its source");
            expected.insert((*owner).to_owned());
        }
    }
    let missing: Vec<_> = expected.difference(&registered).collect();
    let unknown: Vec<_> = registered.difference(&expected).collect();
    assert!(
        missing.is_empty() && unknown.is_empty(),
        "sources the stock shell should register and does not: {missing:?}; sources it registers that this file does not \
         know: {unknown:?} (a new source goes in CONTEXT, when it belongs to a subsystem, or in MODULES)"
    );
    // A subsystem that is driven by something else says what, so that the reason is read when it is reviewed.
    for (field, part) in CONTEXT.iter().chain(STD_CONTEXT) {
        if let Driven(why) = part {
            assert!(!why.is_empty(), "{field}: a driven subsystem says what drives it");
        }
    }
}

#[test]
fn a_source_that_throws_is_reported_and_does_not_end_the_shell() {
    let mut rig = rig_setup(
        "import { onWake } from \"gnoblin-core/wake\";\n",
        "onWake(\"broken part\", () => { throw new Error(\"no deadline today\"); });\n",
        |_| {},
    );
    let diagnostics = rig.settle();
    assert!(
        diagnostics.iter().any(|line| line.contains("wake source broken part")),
        "{diagnostics:?}"
    );
    // The shell is alive: it still answers a script and still goes quiet.
    assert_eq!(rig.eval("String(1 + 1)"), "2");
    rig.quiet();
}

// ---- the library's own clocks -------------------------------------------------------------------------------------

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js")
}

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("the library is readable") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "ts") {
            found.push(path);
        }
    }
}

/// The calls that read a clock.
const CLOCK_READS: &[&str] = &[".now()", "uiNow(", "Date.now(", "new Date("];

/// Files that read a clock and need no source, and why. A file that registers one is not listed.
const NO_SOURCE_NEEDED: &[(&str, &str)] = &[
    (
        "gnoblin-std/ui/session.ts",
        "defines `uiNow`, which the parts below read; it keeps no deadline",
    ),
    ("gnoblin-std/host/install.ts", "hands the toolkit the seat's clock"),
    (
        "gnoblin-std/host/policy.ts",
        "hands the focus and window-state policies the seat's clock",
    ),
    (
        "gnoblin-core/host/services.ts",
        "hands the launch registry the seat's clock",
    ),
    (
        "gnoblin-core/host/session_bridge.ts",
        "hands the host API the seat's clock",
    ),
    ("gnoblin-std/host/xr_bridge.ts", "hands the XR runtime the seat's clock"),
    (
        "gnoblin-core/session/host_api.ts",
        "hands the timers the host's clock; `operation.ts` registers them",
    ),
    (
        "gnoblin-std/frame/drag.ts",
        "stamps a press with the time it was handled, to tell a double click; the close fade's clock is `frame/actions.ts`'s",
    ),
    (
        "gnoblin-core/input/gesture.ts",
        "stamps a gesture's samples with the time they were handled",
    ),
    (
        "gnoblin-core/input/hooks.ts",
        "stamps a binding's event with the time it was handled",
    ),
    (
        "gnoblin-core/input/keys.ts",
        "stamps a key event with the time it was handled",
    ),
    (
        "gnoblin-core/input/pointer.ts",
        "stamps a drag's samples with the time they were handled",
    ),
];

fn reads_a_clock(text: &str) -> bool {
    text.lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with("//") && !line.starts_with('*') && !line.starts_with("/*"))
        .any(|line| CLOCK_READS.iter().any(|read| line.contains(read)))
}

#[test]
fn every_file_of_the_library_that_reads_a_clock_registers_a_source_or_says_why_not() {
    let root = library();
    let mut files = Vec::new();
    for package in ["gnoblin-core", "gnoblin-std"] {
        sources(&root.join(package), &mut files);
    }
    assert!(files.len() > 50, "the library was not found at {}", root.display());
    let mut unaccounted = Vec::new();
    let mut stale = Vec::new();
    for (name, _) in NO_SOURCE_NEEDED {
        let text = fs::read_to_string(root.join(name)).unwrap_or_default();
        if !reads_a_clock(&text) || text.contains("onWake(") {
            stale.push(*name);
        }
    }
    for file in files {
        let relative = file.strip_prefix(&root).expect("under the library");
        let relative = relative.to_str().expect("a UTF-8 name");
        let text = fs::read_to_string(&file).expect("a readable source");
        if reads_a_clock(&text)
            && !text.contains("onWake(")
            && !NO_SOURCE_NEEDED.iter().any(|(name, _)| *name == relative)
        {
            unaccounted.push(relative.to_owned());
        }
    }
    assert!(
        unaccounted.is_empty(),
        "{unaccounted:?} read a clock and register no wake source: a deadline or a time-driven look that no frame is \
         woken for is never drawn. Register one (`onWake` of `gnoblin-core/wake`), or add the file to \
         NO_SOURCE_NEEDED with the reason it keeps no deadline"
    );
    assert!(
        stale.is_empty(),
        "{stale:?} are listed in NO_SOURCE_NEEDED but no longer read a clock, or now register a source: remove them"
    );
}

// ---- the stock shell at rest --------------------------------------------------------------------------------------

/// Lets the shell come to rest as it would in time: frames run until none is due, and the clock moves to each deadline that
/// is left (the one look the alert owes for the boot's frames, which came inside its first interval).
#[track_caller]
fn rest(rig: &mut Rig) {
    for _ in 0..4 {
        rig.quiet();
        match rig.shell.next_wake() {
            None => return,
            Some(due) => rig.advance((due.as_secs_f64() * 1000.0).ceil() as u64),
        }
    }
    panic!("the shell never came to rest: {:?}", rig.shell.next_wake());
}

/// Rests `rig`, and proves that it then asks for nothing and does nothing, however long it is left.
#[track_caller]
fn assert_rests(mut rig: Rig) {
    rest(&mut rig);
    assert_eq!(
        rig.shell.next_wake(),
        None,
        "something the stock shell started is waiting on the clock"
    );
    let before = rig.shell.frame_stats();
    for _ in 0..100 {
        rig.frame();
    }
    assert_eq!(
        rig.shell.frame_stats().run,
        before.run,
        "the stock shell ran a frame with nothing to do"
    );
    // Even a long time on its own does not make it do anything.
    rig.advance(3_600_000);
    assert!(!rig.next_frame_runs(), "an hour passing woke a frame");
}

#[test]
fn the_stock_shell_rests_when_nothing_happens() {
    // Titlebars on every window, the window menu, the not-responding alert and the stock keys are all running.
    assert_rests(stock());
}

/// The turn does not leave work for itself: a frame of a shell that holds the configuration document reads the document's revision
/// (`commit_document_changes`), and that read must leave nothing behind that asks for another frame. It did: each read made a script
/// object for the input manager and dropped it, the engine queued the cleanup of that object as a promise job, and a job left at the
/// end of a frame is a frame due at once (`plan_next_frame`), so the shell ran a frame on every turn for as long as it lived.
#[test]
fn the_stock_shell_that_holds_the_configuration_document_rests_when_nothing_happens() {
    assert_rests(stock_with_document());
}

// ---- the clock widget ---------------------------------------------------------------------------------------------

/// A clock widget over a frozen wall clock the test moves: `fake` is its milliseconds since the epoch.
const FAKE_TIME: &str = "globalThis.fake = 1700000000000;\n\
    globalThis.clock = new Clock({ format: FORMAT, now: () => globalThis.fake });\n\
    const holder = new Box({ width: 200, height: 40 });\n\
    holder.add(globalThis.clock);\n\
    holder.mountTop();\n";

fn clock_world(format: &str) -> Rig {
    world(
        1,
        false,
        "new Layout()",
        &FAKE_TIME.replace("FORMAT", &format!("{format:?}")),
    )
}

#[test]
#[ignore = "vrnw: needs Clock to take `now: () => number` (milliseconds): `new Clock({ format, now })` fails with `Clock.new: the callback failed`"]
fn a_clock_widget_asks_for_a_frame_when_its_minute_ends_and_draws_the_new_time_in_it() {
    let mut rig = clock_world("%H:%M");
    let node = rig
        .eval("String(globalThis.clock.node)")
        .parse::<u64>()
        .expect("a node id");
    rig.quiet();
    let before = rig.comp.text_of(node);
    assert_eq!(before.len(), 1, "the clock draws one text: {before:?}");
    // The milliseconds left of the minute are the system clock's.
    let left = 60_000 - (1_700_000_000_000_u64 % 60_000);
    assert_eq!(rig.shell.next_wake(), Some(Duration::from_millis(left)));
    rig.advance(left - 1);
    assert!(!rig.next_frame_runs(), "the minute has not ended");
    rig.eval("globalThis.fake += 60000; String(globalThis.fake)");
    rig.advance(1);
    assert!(rig.next_frame_runs(), "the end of the minute woke no frame");
    let after = rig.comp.text_of(node);
    assert_ne!(after, before, "the frame it woke drew the new minute");
    rig.quiet();
}

#[test]
#[ignore = "vrnw: needs Clock to take `now: () => number` (milliseconds): `new Clock({ format, now })` fails with `Clock.new: the callback failed`"]
fn a_clock_widget_that_shows_seconds_asks_for_a_frame_each_second() {
    let mut rig = clock_world("%H:%M:%S");
    rig.quiet();
    // The frozen time sits on a whole second, so a whole second is left.
    assert_eq!(rig.shell.next_wake(), Some(Duration::from_millis(1000)));
}

#[test]
#[ignore = "vrnw: needs Clock to take `now: () => number` (milliseconds): `new Clock({ format, now })` fails with `Clock.new: the callback failed`"]
fn a_clock_widget_that_is_destroyed_stops_asking() {
    let mut rig = clock_world("%H:%M");
    rig.quiet();
    assert!(rig.shell.next_wake().is_some());
    rig.eval("globalThis.clock.destroy(); 0");
    rig.quiet();
    assert_eq!(rig.shell.next_wake(), None, "a clock that is gone has nothing to show");
}
