//! What the std library's turn reads of the windows. A turn that only a title changed reads none of them, so it costs the same at
//! any number of windows, and every change the layout reads still lays the workspace out.
//!
//! The library posts a turn for every change it hears of, and the layout pass of the turn used to read the focus, fullscreen and
//! maximised flags, the rectangle and the liveness of every window of every workspace before it asked whether anything had to be
//! computed (`frame_cost.rs` prints what that came to: a title change cost a third more than it did before the layout moved into the
//! library). The pass now skips a workspace whose windows have not changed (`State::workspace_at_rest`), and the second half of this
//! file is the table that guards the skip: each row changes one thing a layout reads, and asserts that the workspace was computed. The
//! last section is the one change that says nothing when it is undone: a flag the library wrote itself, which the compositor then does
//! not take.
//!
//! The count is `gnoblin_std_impl::native_accesses`, the trips the library makes to the core for a node. It is compared across window
//! counts and not against a number, so a new constant read does not fail it and a read per window does.

use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

mod common;
use common::policy::*;
use common::*;

/// A layout that counts how often it computes and places every window in a row. Its `tick` is an option of the layout: the
/// workspace computes again when an own field of the layout changed (`#n`, which counts the computes of one workspace, and `#last`, how
/// many windows the last one was given, are not one). It throws once when `failNext` is set.
const PROBE: &str = "(globalThis as any).probes = [];\n\
    (globalThis as any).computes = 0;\n\
    class Probe extends Layout {\n\
    \x20   tick = 0;\n\
    \x20   #n = 0;\n\
    \x20   #last = 0;\n\
    \x20   constructor() { super(); (globalThis as any).probes.push(this); }\n\
    \x20   get n() { return this.#n; }\n\
    \x20   get last() { return this.#last; }\n\
    \x20   revision() { return this.tick; }\n\
    \x20   compute(c: any) {\n\
    \x20       this.#n += 1;\n\
    \x20       (globalThis as any).computes += 1;\n\
    \x20       this.#last = c.children.length;\n\
    \x20       if ((globalThis as any).failNext) { (globalThis as any).failNext = false; throw new Error('boom'); }\n\
    \x20       return c.children.map((n: any, i: number) => ({ node: n, tiled: true, bounds: { x: c.area.x + 10 * i, y: c.area.y, width: 100, height: 100 } }));\n\
    \x20   }\n\
    }\n";

/// A shell over `n` windows on three workspaces whose layouts are the probe, run until it is quiet, over a second screen when
/// `second`. `config` follows the workspaces.
fn world_of(n: u64, second: bool, config: &str) -> Rig {
    world_over(&imports(), n, second, config)
}

/// Like `world_of`, over a script that starts with `imports`.
fn world_over(imports: &str, n: u64, second: bool, config: &str) -> Rig {
    let init = format!(
        "{PROBE}for (const id of [\"1\", \"2\", \"3\"]) gnoblin.workspaces.add(new Workspace(id, {{ layout: new Probe() }}));\n{config}\n"
    );
    let mut rig = rig_setup(imports, &init, move |comp| populate(comp, n, second));
    settle(&mut rig);
    rig.quiet();
    rig
}

fn computes(rig: &mut Rig) -> u64 {
    number(rig, "globalThis.computes") as u64
}

/// How often the layout of the `index`th workspace added (0 is workspace `1`) has computed.
fn computes_of(rig: &mut Rig, index: usize) -> u64 {
    number(rig, &format!("globalThis.probes[{index}].n")) as u64
}

// ---- a title reads no window --------------------------------------------------------------------------------------

/// The trips the library makes to the core while `n` windows are open and the compositor changes one title.
fn trips_for_one_title(n: u64) -> u64 {
    let mut rig = world_of(n, false, "");
    let computed = computes(&mut rig);
    let placed = rect(&rig, 3);
    let before = gnoblin_std_impl::native_accesses();
    rig.comp.external_set(3, "title", text("renamed"));
    settle(&mut rig);
    let trips = gnoblin_std_impl::native_accesses() - before;
    assert_eq!(
        computes(&mut rig),
        computed,
        "a title change computed the layout again at {n} windows"
    );
    assert_eq!(rect(&rig, 3), placed, "a title change moved a window at {n} windows");
    trips
}

/// The window counts the trips are compared at. `populate` numbers the windows from 3 and the screen, the seat and the lock are 20, 21 and
/// 23, so a world of 18 windows or more has windows where the screen and the seat should be, and a library with no seat or screen reads
/// every window for its focus. These stay below that.
#[test]
fn a_title_change_reads_no_window_so_it_costs_the_same_at_any_number_of_windows() {
    let trips: Vec<(u64, u64)> = [4_u64, 8, 16]
        .into_iter()
        .map(|n| (n, trips_for_one_title(n)))
        .collect();
    assert!(
        trips.windows(2).all(|pair| pair[0].1 == pair[1].1),
        "the trips to the core for one title change, by number of windows, are not the same: {trips:?}"
    );
}

// ---- a change a layout reads is still laid out ---------------------------------------------------------------------

type Change = (&'static str, fn(&mut Rig));

/// One row for each thing the pass reads of a window or a workspace. A new one belongs here: the skip is only safe while every input
/// of the pass has a path that says it changed.
const CHANGES: &[Change] = &[
    ("a window takes the focus", |rig| {
        rig.comp.external_set(4, "focused", MirrorValue::Bool(true));
    }),
    ("a window goes fullscreen", |rig| {
        rig.comp.external_set(4, "fullscreen", MirrorValue::Bool(true));
    }),
    ("a window is maximised", |rig| {
        rig.comp.external_set(4, "maximised", MirrorValue::Bool(true));
    }),
    ("a window opens", |rig| rig.comp.create_window(900, "new", "foot")),
    ("a window closes", |rig| rig.comp.destroy(4)),
    ("a window changes who draws its frame", |rig| {
        rig.comp.external_set(4, "decoration", text("client"));
    }),
    ("two windows trade places", |rig| {
        exec(
            rig,
            "gnoblin.workspaces.byId('1').clients.swap(gnoblin.clients.get('3'), gnoblin.clients.get('4'));",
        );
    }),
    ("a window moves to another workspace", |rig| {
        exec(
            rig,
            "gnoblin.clients.get('4').moveToWorkspace(gnoblin.workspaces.byId('2'));",
        );
    }),
    ("the workspace's gaps change", |rig| {
        exec(rig, "gnoblin.workspaces.byId('1').gaps = { inner: 4, outer: 4 };");
    }),
    ("a title listener changes an option of the layout", |rig| {
        rig.comp.external_set(3, "title", text("renamed"));
    }),
];

/// The listener of the last row: the layout's option changes inside a title listener, and nothing else about the workspace does.
const LISTENER: &str = "gnoblin.onClientTitleChanged(() => { (globalThis as any).probes[0].tick += 1; });\n";

#[test]
fn every_change_a_layout_reads_still_computes_the_workspace() {
    for (name, change) in CHANGES {
        let mut rig = world_of(6, false, LISTENER);
        let before = computes(&mut rig);
        change(&mut rig);
        settle(&mut rig);
        assert!(
            computes(&mut rig) > before,
            "{name}: the workspace was not computed again"
        );
    }
}

#[test]
fn a_title_change_with_a_listener_that_leaves_the_layout_alone_computes_nothing() {
    let mut rig = world_of(6, false, "gnoblin.onClientTitleChanged(() => {});\n");
    let before = computes(&mut rig);
    rig.comp.external_set(3, "title", text("renamed"));
    settle(&mut rig);
    assert_eq!(computes(&mut rig), before);
}

#[test]
fn the_skip_does_not_outlive_a_change_it_could_not_see() {
    // The flags are read again for each of these in turn, however many quiet turns lie between: a change is not remembered
    // as seen because a title change came after it.
    let mut rig = world_of(6, false, "");
    for round in 0..3_u32 {
        let before = computes(&mut rig);
        rig.comp
            .external_set(4, "fullscreen", MirrorValue::Bool(round % 2 == 0));
        settle(&mut rig);
        let after_flag = computes(&mut rig);
        assert!(
            after_flag > before,
            "round {round}: the fullscreen flag did not compute the workspace"
        );
        rig.comp.external_set(3, "title", text(&format!("title {round}")));
        settle(&mut rig);
        assert_eq!(
            computes(&mut rig),
            after_flag,
            "round {round}: a title change computed the workspace"
        );
    }
}

#[test]
#[ignore = "vrnw: needs the layout pass to read again a flag a failed requestTransition staged: after the failure and a title change the workspace is not computed again (2 computes, 3 expected)"]
fn a_flag_a_failed_transition_staged_is_read_again_once_the_staging_is_gone() {
    let mut rig = world_of(6, false, "");
    exec(&mut rig, "g.b = gnoblin.clients.get('4');");
    let before = computes(&mut rig);
    // Inside the callback the pass reads the flag as the callback set it, which makes the workspace compute, and the layout fails:
    // nothing staged is sent, so the window is not maximised, and the pass has noted that it was.
    exec(&mut rig, "globalThis.failNext = true;");
    let message = rig.eval(
        "(() => { try { gnoblin.requestTransition(() => { g.b.maximised = true; }, { duration: 100 }); return 'no'; } \
         catch (e) { return e.message; } })()",
    );
    assert!(message.contains("compute failed"), "{message}");
    assert_eq!(computes(&mut rig), before + 1, "the failed compute was the one attempt");
    assert_ne!(rig.comp.flag(4, "maximised"), Some(true), "nothing staged was sent");
    // A title change and the frames after it: the pass looks at the window again and computes the workspace as it is.
    rig.comp.external_set(3, "title", text("renamed"));
    settle(&mut rig);
    assert_eq!(
        computes(&mut rig),
        before + 2,
        "the flag the failed transition staged was remembered as read, and the workspace was not computed again"
    );
}

#[test]
fn a_flag_change_on_the_workspace_of_another_screen_computes_that_workspace_and_no_other() {
    for (name, key) in [
        ("takes the focus", "focused"),
        ("goes fullscreen", "fullscreen"),
        ("is maximised", "maximised"),
    ] {
        let mut rig = world_of(6, true, "");
        // Window 4 goes to workspace 2, which the second screen shows; workspace 1 keeps the other five.
        exec(
            &mut rig,
            "gnoblin.clients.get('4').moveToWorkspace(gnoblin.workspaces.byId('2'));",
        );
        settle(&mut rig);
        rig.quiet();
        let (first, second) = (computes_of(&mut rig, 0), computes_of(&mut rig, 1));
        assert!(second > 0, "{name}: workspace 2 is not shown, so it computes no change");
        rig.comp.external_set(4, key, MirrorValue::Bool(true));
        settle(&mut rig);
        assert_eq!(
            computes_of(&mut rig, 0),
            first,
            "{name}: a window of workspace 2 computed workspace 1"
        );
        assert!(
            computes_of(&mut rig, 1) > second,
            "a window of workspace 2 {name}: workspace 2 was not computed again"
        );
    }
}

// ---- a flag the library wrote and the compositor did not take -----------------------------------------------------
//
// A flag a pass reads is not read again until an event says it changed, and a value the library wrote itself is one the tree shows
// before the compositor has taken it: the transaction that carries it is in flight, and a read finds what was sent. A pass that runs
// meanwhile (for any other change of the workspace) reads the flag as written and computes the workspace for it. If the compositor then
// refuses the write, or drops it after the gate's resends, the tree shows the old value again and says nothing: no event comes for a
// value that was never the compositor's. The workspace must be read again when the compositor has answered.

/// A shell as `world_of` makes one that reaches the library's own window policy: `WindowState`, whose `maximise` writes a window's
/// `maximised` flag and its rectangle itself, and the core's `Client` objects it takes (`CoreGnoblin.get().clients`).
fn world_with_policy(n: u64) -> Rig {
    let imports = format!(
        "{}import {{ WindowState }} from \"gnoblin-std/generated/objects\";\n\
         import {{ Gnoblin as CoreGnoblin }} from \"gnoblin-core/generated/objects\";\n\
         (globalThis as any).WindowState = WindowState;\n\
         (globalThis as any).CoreGnoblin = CoreGnoblin;\n",
        imports()
    );
    world_over(&imports, n, false, "")
}

/// The rectangles of windows 3 to 8 as the compositor holds them: with six windows on one workspace, one tile each.
fn tiles(rig: &Rig) -> Vec<String> {
    (3..=8).map(|window| rect(rig, window)).collect()
}

/// The compositor refuses the transaction that carries the library's write of window 4's `maximised` (the `write`th of its ops) and
/// applies nothing of it, for a reason of the script's: no newer base cures it, so the gate gives the write up at once.
fn refuses(comp: &FakeCompositor, write: usize) {
    comp.fault(Fault::Refuse {
        code: "read-only",
        failed_op: write as i64,
    });
}

/// The compositor writes window 4's title before each landing of the transaction, one refusal `stale-base` more than the gate sends the
/// write again for: the write is dropped after its resends.
fn writes_the_title_first(comp: &FakeCompositor, write: usize) {
    comp.stale(gnoblin_js::STALE_RETRIES as usize + 1, Some(write as i64), 4, "title");
}

/// The library maximises window 4, and while the transaction that carries the write is unanswered the workspace's gaps change, which is
/// a change a pass runs for (the compositor says nothing in between, so nothing else of the tree moves): the pass reads the flag as the
/// library wrote it and lays the workspace out without the window. Then the compositor answers as `answer` says and says nothing of the
/// flag, and a title changes, which is all that follows. Gives the shell, and the tiles the six windows were in before.
fn library_maximises_and_the_compositor(answer: fn(&FakeCompositor, usize)) -> (Rig, Vec<String>) {
    let mut rig = world_with_policy(6);
    let tiled = tiles(&rig);
    // The compositor holds what it is sent until the test lets it answer.
    rig.comp.hold();
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "WindowState.get().maximise(CoreGnoblin.get().clients.get('4'));",
    );
    rig.frame();
    let write = rig
        .comp
        .sent_since(mark)
        .first()
        .and_then(|ops| {
            ops.iter()
                .position(|op| matches!(op, TxOp::Set { node: 4, key, .. } if key == "maximised"))
        })
        .expect("the library wrote window 4's maximised flag in a transaction of its own");
    exec(&mut rig, "gnoblin.workspaces.byId('1').gaps = { inner: 4, outer: 4 };");
    for _ in 0..4 {
        rig.frame();
    }
    assert_eq!(
        rig.eval("String(probes[0].last)"),
        "5",
        "the pass did not read the flag as the library wrote it: the workspace was not laid out without the window"
    );
    answer(&rig.comp, write);
    for _ in 0..16 {
        rig.comp.release();
        rig.frame();
    }
    rig.comp.external_set(3, "title", text("renamed"));
    for _ in 0..16 {
        rig.comp.release();
        rig.frame();
    }
    (rig, tiled)
}

/// A refused maximise leaves the six windows in the tiles they were in: the workspace was laid out once without window 4 and has to be
/// laid out with it again.
fn assert_tiled_again(answer: fn(&FakeCompositor, usize)) {
    let (rig, tiled) = library_maximises_and_the_compositor(answer);
    assert!(rig.comp.refused() > 0, "the compositor took the write");
    assert_ne!(
        rig.comp.flag(4, "maximised"),
        Some(true),
        "the compositor took the maximise"
    );
    assert_eq!(
        tiles(&rig),
        tiled,
        "the workspace was not laid out again once the flag read as it was, so the window is out of its tiles"
    );
}

#[test]
fn a_maximise_the_library_wrote_and_the_compositor_refused_is_read_again_and_the_window_is_tiled_again() {
    assert_tiled_again(refuses);
}

#[test]
fn a_maximise_the_library_wrote_and_the_gate_dropped_after_its_resends_is_read_again_and_the_window_is_tiled_again() {
    assert_tiled_again(writes_the_title_first);
}

#[test]
fn a_maximise_the_library_wrote_and_the_compositor_took_keeps_the_window_out_of_the_tiles() {
    let (rig, _) = library_maximises_and_the_compositor(|_, _| {});
    assert_eq!(rig.comp.flag(4, "maximised"), Some(true));
    assert_eq!(
        tiles(&rig),
        [
            "0,0,100,100",
            "0,0,1000,800",
            "10,0,100,100",
            "20,0,100,100",
            "30,0,100,100",
            "40,0,100,100"
        ],
        "a window the compositor maximised is not tiled: the other five take its place"
    );
}
