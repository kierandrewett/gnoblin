//! The example shells of `examples/`, loaded into the core and driven over the fake compositor (`design/modular-framework.md`
//! section 12, examples 1 and 8).
//!
//! Each scenario is one process, started by the launcher fixture: the core is the library the launcher links, the example is the
//! `.so` it loads, and the compositor is the fake of `gnoblin-testkit` behind the tree link, so the process maps exactly what a
//! real session does (the core and one shell) and nothing else. The scenario drives windows, keys and a closing window through the
//! fake, and prints what the shell did to the tree as `key=value` lines the tests assert on.
#![allow(unsafe_code)]

use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
use gnoblin_tree_client::tree_mirror::{LinkError, MirrorValue, TreeLink, TxOp};

use gnoblin_core_abi as abi;

use crate::host::{self, AttachMode, Copying, Module, TreeLinkProvider};

macro_rules! out {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

pub(crate) const SCREEN: u64 = 20;
pub(crate) const SEAT: u64 = 21;
pub(crate) const FIRST: u64 = 40;
const SECOND: u64 = 41;
const THIRD: u64 = 42;
const FOURTH: u64 = 43;
const SECOND_SCREEN: u64 = 22;

struct Provider(FakeCompositor);

impl TreeLinkProvider for Provider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        Ok(Box::new(self.0.clone()))
    }
}

/// A compositor with a 1000 by 800 screen, a seat and one window already open, behind the tree link the core's shell reaches. The
/// shell instance is reset first, so a scenario starts from nothing.
pub(crate) fn world() -> FakeCompositor {
    host::reset_session();
    let comp = FakeCompositor::new();
    let double = MirrorValue::Double;
    comp.add_node(
        SCREEN,
        ROOT,
        "screen",
        vec![
            ("screen_x", double(0.0)),
            ("screen_y", double(0.0)),
            ("screen_width", double(1000.0)),
            ("screen_height", double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(SEAT, ROOT, "seat", vec![("name", text("seat0"))]);
    comp.add_window(FIRST, "Terminal", "foot");
    host::set_tree_link_provider(Box::new(Provider(comp.clone())));
    comp
}

/// [`world`], with the shell loaded into the core over it.
fn start(shell: &Path) -> Result<(FakeCompositor, Module), i32> {
    out!("core_abi={}.{}", abi::ABI_MAJOR, abi::ABI_MINOR);
    let comp = world();
    let module = match host::load(shell, Copying::Unique, 0) {
        Ok(module) => module,
        Err(error) => {
            out!("load_failed {error}");
            return Err(1);
        }
    };
    out!("loaded label={} generation={}", module.label(), module.generation());
    if let Err(message) = module.activate() {
        out!("activate_failed {message}");
        return Err(1);
    }
    Ok((comp, module))
}

/// Runs the shell's turns until the tree has settled: callbacks, the transaction they write, and the compositor's answer.
fn pump(module: &Module) {
    for _ in 0..4 {
        module.dispatch(0);
    }
}

fn geometry(comp: &FakeCompositor, id: u64) {
    if comp.exists(id) {
        out!(
            "window id={id} x={} y={} w={} h={}",
            comp.number(id, "x"),
            comp.number(id, "y"),
            comp.number(id, "width"),
            comp.number(id, "height")
        );
    } else {
        out!("window id={id} gone");
    }
}

/// The chord claim whose text is `chord`, pressed.
fn press(comp: &FakeCompositor, module: &Module, chord: &str) {
    let claim = comp.claim_id(chord);
    comp.chord(claim, chord, "press");
    pump(module);
}

/// Waits (up to five seconds) for the program a chord started to leave its mark: the test puts a stand-in terminal on the `PATH`
/// that creates the file `GNOBLIN_TEST_MARKER` names.
fn terminal_started() -> bool {
    let Some(marker) = std::env::var_os("GNOBLIN_TEST_MARKER") else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if Path::new(&marker).exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn new_nodes(comp: &FakeCompositor) -> usize {
    comp.ops().iter().filter(|op| matches!(op, TxOp::New { .. })).count()
}

fn finish(comp: &FakeCompositor, module: &Module) {
    out!("new_nodes={}", new_nodes(comp));
    out!("claims={:?}", comp.claimed());
    crate::driver::maps_line();
    crate::driver::shell_stats_line("shell_stats", module);
    crate::driver::core_stats_line("core_stats", module.ctx());
}

/// Example 1: the minimal core-only shell. A window that was open is placed at once, one that opens is placed in the next slot of
/// the cascade, a chord starts a terminal, and another closes the window that has the keyboard.
pub(crate) fn minimal(shell: &Path) -> i32 {
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    pump(&module);
    geometry(&comp, FIRST);
    comp.create_window(SECOND, "Files", "nautilus");
    pump(&module);
    geometry(&comp, SECOND);
    comp.create_window(THIRD, "Editor", "code");
    pump(&module);
    geometry(&comp, THIRD);
    // The shell's own writes did not move a window the compositor still owns: the first one is where it was put.
    geometry(&comp, FIRST);

    press(&comp, &module, "Super+Return");
    out!("terminal_started={}", terminal_started());

    comp.external_set(SECOND, "focused", MirrorValue::Bool(true));
    pump(&module);
    press(&comp, &module, "Super+q");
    for (node, method) in comp.calls() {
        out!("call node={node} method={method}");
    }
    finish(&comp, &module);
    0
}

/// Example 8: the tiler with no workspaces and no scene graph. Windows are tiled master and stack as they open and close, `Super+m`
/// promotes the focused window, and the shell makes no node of its own.
pub(crate) fn tiler(shell: &Path) -> i32 {
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    pump(&module);
    out!("step=one");
    geometry(&comp, FIRST);
    comp.create_window(SECOND, "Files", "nautilus");
    pump(&module);
    out!("step=two");
    geometry(&comp, FIRST);
    geometry(&comp, SECOND);
    comp.create_window(THIRD, "Editor", "code");
    pump(&module);
    out!("step=three");
    for id in [FIRST, SECOND, THIRD] {
        geometry(&comp, id);
    }
    comp.destroy(SECOND);
    pump(&module);
    out!("step=closed");
    for id in [FIRST, SECOND, THIRD] {
        geometry(&comp, id);
    }
    comp.external_set(THIRD, "focused", MirrorValue::Bool(true));
    pump(&module);
    press(&comp, &module, "Super+m");
    out!("step=promoted");
    for id in [FIRST, THIRD] {
        geometry(&comp, id);
    }
    out!("order={:?}", comp.children(ROOT));
    press(&comp, &module, "Super+Return");
    out!("terminal_started={}", terminal_started());
    finish(&comp, &module);
    0
}

/// Example 2: a desktop shell on the std library. Windows join the floating workspace of their output and are placed by it, a second
/// output gets a workspace of its own, and a chord starts a terminal.
pub(crate) fn std_prelude(shell: &Path) -> i32 {
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    pump(&module);
    out!("step=one");
    geometry(&comp, FIRST);
    comp.create_window(SECOND, "Files", "nautilus");
    pump(&module);
    comp.create_window(THIRD, "Editor", "code");
    pump(&module);
    out!("step=three");
    for id in [FIRST, SECOND, THIRD] {
        geometry(&comp, id);
    }
    // A second output appears with the pointer on it: the window that opens next joins the workspace of that output.
    comp.create_screen(SECOND_SCREEN, "HDMI-2", (1000.0, 0.0, 800.0, 600.0), false);
    pump(&module);
    comp.set_pointer(SEAT, 1400.0, 300.0);
    pump(&module);
    comp.create_window(FOURTH, "Browser", "firefox");
    pump(&module);
    out!("step=second_output");
    for id in [FIRST, SECOND, THIRD, FOURTH] {
        geometry(&comp, id);
    }
    // A window that closes leaves the others where they were.
    comp.destroy(SECOND);
    pump(&module);
    out!("step=closed");
    for id in [FIRST, SECOND, THIRD, FOURTH] {
        geometry(&comp, id);
    }
    press(&comp, &module, "Super+Return");
    out!("terminal_started={}", terminal_started());
    finish(&comp, &module);
    0
}

/// Example 6: a tiling layout written in Rust. Windows open and close and the layout arranges what is left; a chord swaps two.
pub(crate) fn rust_layout(shell: &Path) -> i32 {
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    pump(&module);
    out!("step=one");
    geometry(&comp, FIRST);
    comp.create_window(SECOND, "Files", "nautilus");
    pump(&module);
    out!("step=two");
    for id in [FIRST, SECOND] {
        geometry(&comp, id);
    }
    comp.create_window(THIRD, "Editor", "code");
    pump(&module);
    out!("step=three");
    for id in [FIRST, SECOND, THIRD] {
        geometry(&comp, id);
    }
    // The window that has the keyboard swaps places with the first.
    comp.external_set(THIRD, "focused", MirrorValue::Bool(true));
    pump(&module);
    press(&comp, &module, "Super+Tab");
    out!("step=swapped");
    for id in [FIRST, SECOND, THIRD] {
        geometry(&comp, id);
    }
    comp.destroy(SECOND);
    pump(&module);
    out!("step=closed");
    for id in [FIRST, SECOND, THIRD] {
        geometry(&comp, id);
    }
    finish(&comp, &module);
    0
}

/// The layouts fixture: four windows, then each chord's layout, then a window closing under the last one.
pub(crate) fn std_layouts(shell: &Path) -> i32 {
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    pump(&module);
    for (id, title) in [(SECOND, "Files"), (THIRD, "Editor"), (FOURTH, "Browser")] {
        comp.create_window(id, title, "app");
        pump(&module);
    }
    comp.external_set(THIRD, "focused", MirrorValue::Bool(true));
    pump(&module);
    for chord in ["Super+1", "Super+2", "Super+3", "Super+4", "Super+5", "Super+6"] {
        press(&comp, &module, chord);
        out!("step={chord}");
        for id in [FIRST, SECOND, THIRD, FOURTH] {
            geometry(&comp, id);
        }
    }
    press(&comp, &module, "Super+1");
    comp.destroy(SECOND);
    pump(&module);
    out!("step=closed");
    for id in [FIRST, SECOND, THIRD, FOURTH] {
        geometry(&comp, id);
    }
    // A tile dragged onto another swaps places with it: the bottom tile of the stack onto the master.
    drag(
        &comp,
        &module,
        "Super+Button1",
        (600.0, 600.0),
        &[(400.0, 400.0), (200.0, 300.0)],
        "end",
    );
    out!("step=swapped");
    for id in [FIRST, THIRD, FOURTH] {
        geometry(&comp, id);
    }
    // Dropped on nothing, it stays where it is.
    drag(
        &comp,
        &module,
        "Super+Button1",
        (600.0, 600.0),
        &[(5000.0, 5000.0)],
        "end",
    );
    out!("step=dropped_nowhere");
    for id in [FIRST, THIRD, FOURTH] {
        geometry(&comp, id);
    }
    finish(&comp, &module);
    0
}

/// One whole pointer drag of the claimed chord `chord`: the press at `from`, the motion through `path`, and the release at the last
/// point (or `phase` as the last step instead, `cancel` to have the compositor end it).
fn drag(comp: &FakeCompositor, module: &Module, chord: &str, from: (f64, f64), path: &[(f64, f64)], last: &str) {
    let claim = comp.claim_id(chord);
    comp.pointer_chord(claim, chord, "start", from, from);
    pump(module);
    let mut at = from;
    for point in path {
        at = *point;
        comp.pointer_chord(claim, chord, "update", from, at);
        pump(module);
    }
    comp.pointer_chord(claim, chord, last, from, at);
    pump(module);
}

fn place(comp: &FakeCompositor, module: &Module, id: u64, rect: (f64, f64, f64, f64)) {
    for (key, value) in [("x", rect.0), ("y", rect.1), ("width", rect.2), ("height", rect.3)] {
        comp.external_set(id, key, MirrorValue::Double(value));
    }
    pump(module);
}

/// The bindings fixture: drags that move and resize a window, one that is held inside the work area, one the compositor cancels, one
/// that lands on nothing, one that stays under the threshold, and a binding of the shell's own.
pub(crate) fn std_bindings(shell: &Path) -> i32 {
    let (comp, module) = match start(shell) {
        Ok(started) => started,
        Err(code) => return code,
    };
    pump(&module);
    place(&comp, &module, FIRST, (100.0, 100.0, 300.0, 200.0));
    comp.create_window(SECOND, "Files", "nautilus");
    pump(&module);
    place(&comp, &module, SECOND, (500.0, 400.0, 200.0, 150.0));

    // Under the threshold nothing moves; past it the window follows the pointer from where the press was.
    drag(
        &comp,
        &module,
        "Super+Button1",
        (150.0, 150.0),
        &[(152.0, 151.0)],
        "end",
    );
    out!("step=below_threshold");
    geometry(&comp, FIRST);
    drag(
        &comp,
        &module,
        "Super+Button1",
        (150.0, 150.0),
        &[(152.0, 151.0), (250.0, 200.0), (260.0, 210.0)],
        "end",
    );
    out!("step=moved");
    geometry(&comp, FIRST);
    // The work area is the 1000 by 800 screen: a window dragged far off it stops at the edge.
    drag(
        &comp,
        &module,
        "Super+Button1",
        (300.0, 250.0),
        &[(2000.0, 250.0), (2000.0, -500.0)],
        "end",
    );
    out!("step=held_in");
    geometry(&comp, FIRST);
    // A drag the compositor cancels puts the window back.
    drag(
        &comp,
        &module,
        "Super+Button1",
        (600.0, 450.0),
        &[(450.0, 550.0)],
        "cancel",
    );
    out!("step=cancelled");
    geometry(&comp, SECOND);
    // A press on nothing starts nothing.
    drag(&comp, &module, "Super+Button1", (20.0, 700.0), &[(300.0, 700.0)], "end");
    out!("step=nothing");
    for id in [FIRST, SECOND] {
        geometry(&comp, id);
    }

    // Resize by the nearest side: the press near the right edge of the second window at (500,400,200,150).
    drag(&comp, &module, "Alt+Button3", (698.0, 470.0), &[(740.0, 470.0)], "end");
    out!("step=resized_right");
    geometry(&comp, SECOND);
    // Near the top edge: the top moves with the pointer, the bottom stays.
    drag(&comp, &module, "Alt+Button3", (600.0, 402.0), &[(600.0, 380.0)], "end");
    out!("step=resized_top");
    geometry(&comp, SECOND);
    // Below the smallest size the opposite side stays put: the left edge dragged far to the right.
    drag(&comp, &module, "Alt+Button3", (502.0, 450.0), &[(900.0, 450.0)], "end");
    out!("step=resized_min");
    geometry(&comp, SECOND);

    // The shell's own binding: raises the window it landed on, once the pointer is ten pixels away.
    out!("order_before={:?}", comp.children(ROOT));
    drag(
        &comp,
        &module,
        "Super+Button2",
        (800.0, 100.0),
        &[(804.0, 102.0), (810.0, 112.0)],
        "end",
    );
    out!("order_after={:?}", comp.children(ROOT));
    for (node, method) in comp.calls() {
        out!("call node={node} method={method}");
    }
    finish(&comp, &module);
    0
}
