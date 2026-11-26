//! `gnoblin.requestTransition` called from the places a shell really calls it (a timer, a listener's `onAdded`), over the real core
//! and the real std library.
//!
//! How the bug could arise, written before the test: `request` (`gnoblin-std-impl/src/animation/objects.rs`) runs inside a call of
//! the object protocol, and runs the shell's callback in the middle of it (begin, `work.call`, commit). The callback writes a window
//! property, which re-enters the library through the same protocol. Anything the outer call (or the turn that delivered it) keeps
//! borrowed across `work.call` and the write touches again is a `RefCell` that is already borrowed. A call made straight from the
//! test's `eval` and one made from a timer or a listener differ in who is up the stack: a timer runs from the core's dispatch, which
//! sets the way a library's callback enters the script (the shell's own entry, which takes the QuickJS context's lock), and the
//! script's call then asks `work` back through that same entry while it already holds the lock. So each place gets a case, and a case
//! has two callbacks in one dispatch, to prove the dispatch's way in is back after a call that used the call's own.
//!
//! The callback may read the library's own objects too, as a shell's callback does (`gnoblin.workspaces`), and that is a read of a getter
//! of the object whose method is running it: `gnoblin-object`'s `tests/protocol.rs` holds the server's side of it, and the last two
//! cases here are the shell's, from a timer and from a binding.

mod common;

use common::{FakeCompositor, LIBRARY_GLOBALS, ROOT, Rig, rig_setup, text};
use gnoblin_tree_client::tree_mirror::MirrorValue;

const SCREEN: u64 = 20;
const SEAT: u64 = 21;
const WINDOW: u64 = 3;
const OTHER: u64 = 4;

fn world(init: &str) -> Rig {
    world_with(init, |_| {})
}

/// [`world`] with `more` added to the compositor before the shell boots.
fn world_with(init: &str, more: impl FnOnce(&FakeCompositor)) -> Rig {
    let mut rig = rig_setup(LIBRARY_GLOBALS, init, |comp| {
        comp.add_node(
            SCREEN,
            ROOT,
            "screen",
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        more(comp);
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// A shell that runs `site` in the `onAdded` of a listener on the clients.
fn animator(site: &str) -> String {
    format!(
        "globalThis.log = [];\n\
         class Animator extends ClientCollectionListener {{\n\
             onAdded(client) {{ {site} }}\n\
         }}\n\
         gnoblin.clients.addListener(new Animator());\n"
    )
}

/// The script that starts a transition of `client`'s opacity and logs how it went.
fn fade(client: &str) -> String {
    format!(
        "try {{ gnoblin.requestTransition(() => {{ {client}.opacity = 0.5; }}, {{ duration: 60000 }}); log.push('ok'); }} \
         catch (e) {{ log.push('failed:' + e.message); }}"
    )
}

/// A window the compositor maps, and the frames that follow it with the shell's timers (1500 ms) coming due.
fn open_window(rig: &mut Rig, id: u64) -> Vec<String> {
    rig.comp.create_window(id, "Terminal", "foot");
    let mut all = rig.settle();
    rig.advance(1500);
    all.extend(rig.settle());
    all
}

fn opacity(rig: &Rig, id: u64) -> f64 {
    match rig.comp.value(id, "opacity") {
        Some(MirrorValue::Double(value)) => value,
        other => panic!("opacity is not a number: {other:?}"),
    }
}

#[test]
fn a_transition_requested_from_a_timer_writes_the_property_and_the_script_hears_no_failure() {
    let init = animator(&format!("gnoblin.timers.after(1000, () => {{ {} }});", fade("client")));
    let mut rig = world(&init);
    let diagnostics = open_window(&mut rig, WINDOW);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("log.join('|')"), "ok");
    assert_eq!(opacity(&rig, WINDOW), 0.5, "the write the callback made landed");
}

#[test]
fn a_transition_requested_inside_on_added_writes_the_property_and_the_script_hears_no_failure() {
    let mut rig = world(&animator(&fade("client")));
    rig.comp.create_window(WINDOW, "Terminal", "foot");
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("log.join('|')"), "ok");
    assert_eq!(opacity(&rig, WINDOW), 0.5, "the write the callback made landed");
}

#[test]
fn a_transition_requested_from_a_timer_the_script_started_itself_writes_the_property() {
    let mut rig = world("globalThis.log = [];\n");
    rig.comp.create_window(WINDOW, "Terminal", "foot");
    rig.settle();
    rig.eval(&format!(
        "globalThis.client = gnoblin.clients.get('3'); gnoblin.timers.after(1000, () => {{ {} }}); 0",
        fade("client")
    ));
    rig.advance(1500);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("log.join('|')"), "ok");
    assert_eq!(opacity(&rig, WINDOW), 0.5, "the write the callback made landed");
}

#[test]
fn two_timers_due_in_one_dispatch_each_request_a_transition() {
    // The first timer's call into the library runs `work` through the call's own way in; the second timer, in the same dispatch,
    // must find the dispatch's way in back, or it has no way into the script at all.
    let mut rig = world("globalThis.log = [];\n");
    rig.comp.create_window(WINDOW, "Terminal", "foot");
    rig.comp.create_window(OTHER, "Files", "nautilus");
    rig.settle();
    rig.eval(&format!(
        "globalThis.first = gnoblin.clients.get('3'); globalThis.second = gnoblin.clients.get('4'); \
         gnoblin.timers.after(1000, () => {{ {} }}); gnoblin.timers.after(1000, () => {{ {} }}); 0",
        fade("first"),
        fade("second")
    ));
    rig.advance(1500);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("log.join('|')"), "ok|ok");
    assert_eq!(opacity(&rig, WINDOW), 0.5);
    assert_eq!(opacity(&rig, OTHER), 0.5);
}

// ---- a callback that reads the library's workspaces ------------------------------------------------------------------------------
//
// How the bug could arise, written before the test: `gnoblin.requestTransition` is a method of the object that stands for the library's
// members of `gnoblin` (`gnoblin_augment`), and so is `gnoblin.workspaces`. The server held that object's native state for the whole
// call of `request_transition`, so the callback's read of `gnoblin.workspaces` found the object in use (`E_BUSY`: "the object is in use by
// a call that is running") and the callback failed before it wrote anything, from a timer and from a binding alike. The callback of a
// transition reads the desktop it is about to change (the layout of a workspace, the area it has), so it is not a rare thing for it to do.

/// A transition of `client`'s opacity whose callback first reads the library's workspaces, and logs how it went.
fn fade_reading_workspaces(client: &str) -> String {
    format!(
        "try {{ gnoblin.requestTransition(() => {{ \
             log.push(gnoblin.workspaces.length === expected ? 'workspaces' : 'workspaces:' + gnoblin.workspaces.length); \
             {client}.opacity = 0.5; }}, {{ duration: 60000 }}); log.push('ok'); }} \
         catch (e) {{ log.push('failed:' + e.message); }}"
    )
}

/// The shell of these cases: a log.
const READS_WORKSPACES: &str = "globalThis.log = [];\n";

/// What the shell outside the callback sees of the workspaces once the world has settled, which the callback must see too. It is read
/// after the settle and not at load: the count at load is not the count after the library's first turn.
const WORKSPACES_NOW: &str = "globalThis.expected = gnoblin.workspaces.length; 0";

#[test]
fn a_transition_whose_callback_reads_the_workspaces_runs_from_a_timer() {
    let mut rig = world(READS_WORKSPACES);
    rig.comp.create_window(WINDOW, "Terminal", "foot");
    rig.settle();
    rig.eval(WORKSPACES_NOW);
    rig.eval(&format!(
        "globalThis.client = gnoblin.clients.get('3'); gnoblin.timers.after(1000, () => {{ {} }}); 0",
        fade_reading_workspaces("client")
    ));
    rig.advance(1500);
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("log.join('|')"), "workspaces|ok");
    assert_eq!(opacity(&rig, WINDOW), 0.5, "the write the callback made landed");
}

#[test]
fn a_transition_whose_callback_reads_the_workspaces_runs_from_a_binding() {
    let init = format!(
        "{READS_WORKSPACES}gnoblin.input.primary.bind('Super+t', () => {{ {} }});\n",
        fade_reading_workspaces("gnoblin.clients.get('3')")
    );
    let mut rig = world_with(&init, |comp| {
        comp.add_node(
            SEAT,
            ROOT,
            "seat",
            vec![
                ("name", text("seat0")),
                ("pointer_x", MirrorValue::Double(0.0)),
                ("pointer_y", MirrorValue::Double(0.0)),
            ],
        );
    });
    rig.comp.create_window(WINDOW, "Terminal", "foot");
    rig.settle();
    rig.eval(WORKSPACES_NOW);
    let claim = rig.comp.claim_id("<Super>t");
    rig.comp.chord(claim, "<Super>t", "press");
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(rig.eval("log.join('|')"), "workspaces|ok");
    assert_eq!(opacity(&rig, WINDOW), 0.5, "the write the callback made landed");
}
