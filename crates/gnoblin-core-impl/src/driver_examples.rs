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

const SCREEN: u64 = 20;
const SEAT: u64 = 21;
const FIRST: u64 = 40;
const SECOND: u64 = 41;
const THIRD: u64 = 42;

struct Provider(FakeCompositor);

impl TreeLinkProvider for Provider {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        Ok(Box::new(self.0.clone()))
    }
}

/// A compositor with a 1000 by 800 screen, a seat and one window already open, and the shell loaded into the core over it.
fn start(shell: &Path) -> Result<(FakeCompositor, Module), i32> {
    out!("core_abi={}.{}", abi::ABI_MAJOR, abi::ABI_MINOR);
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
