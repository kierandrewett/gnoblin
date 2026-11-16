//! The shell rig the UI tests share, over the fake compositor of `gnoblin-testkit` (an in-memory `TreeLink` that applies `New`,
//! `Insert`, `Set` and `Destroy` as the compositor does, echoes each as a change and answers a transaction with a reply).
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gnoblin_js::{JsRoleConfig, JsShell, JsShellConfig};
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_tree_client::tree_mirror::MirrorValue;
use tempfile::TempDir;

pub mod policy;

pub use gnoblin_testkit::compositor::*;

// ---- time ---------------------------------------------------------------------------------------------------------
//
// The rule for every test in this crate (design/js-runtime-testing.md, "Unit test timing"): no assertion may rest on how fast
// the machine is. A script that is not meant to run out of time gets `HONEST_BUDGET`, and anything that waits
// (an arming window, a fade, a delay, `gnoblin.timers.after`) waits on the injected clock of `timed_config`, which
// moves only when the test moves it. A test that is about a budget runs a script that never ends, keeps the budget
// short and asserts that it was interrupted, never when.

/// What one honest script of a test may use per call. The product default is 100 ms for a listener call, which a
/// machine that is busy for a tenth of a second would exhaust in a script that is doing nothing wrong.
pub const HONEST_BUDGET: Duration = Duration::from_secs(5);

/// The budget of a test about a script that never ends. Short, so the loop is cheap; no shorter than half a second,
/// because whatever honest script the same budget covers (the shell's own calls in a frame, a listener beside the
/// runaway one) must not be able to trip it. Such a test asserts that the script was interrupted, never when.
pub const RUNAWAY_BUDGET: Duration = Duration::from_millis(500);

/// A shell config for a test whose scripts are honest: `root`, with call and frame budgets a loaded machine cannot
/// exhaust, and the real clock.
pub fn honest_config(root: impl Into<PathBuf>) -> JsShellConfig {
    let mut config = JsShellConfig::new(root);
    config.entry_budget = HONEST_BUDGET;
    config.frame_budget = HONEST_BUDGET * 2;
    config
}

/// Like [`honest_config`], with a clock that stands at zero until the test advances it, and the handle to advance it.
pub fn timed_config(root: impl Into<PathBuf>) -> (JsShellConfig, ManualClock) {
    let clock = ManualClock::default();
    let mut config = honest_config(root);
    config.clock = Arc::new(clock.clone());
    (config, clock)
}

/// A runtime role over `root` for a test whose scripts are honest, with the budgets of [`honest_config`].
pub fn honest_role_config(root: impl Into<PathBuf>) -> JsRoleConfig {
    let mut config = JsRoleConfig::new(root, None);
    config.shell.entry_budget = HONEST_BUDGET;
    config.shell.frame_budget = HONEST_BUDGET * 2;
    config
}

// ---- change gating ------------------------------------------------------------------------------------------------
//
// A turn the shell finds nothing to do on costs nothing, and a change that does not wake it is never drawn
// (`tests/frame_gating.rs`). What each wake test asks of a shell, whatever fake it sits on.

/// Whether the next frame did its work. False when the turn was found quiet.
pub fn frame_runs(shell: &mut JsShell) -> bool {
    let before = shell.frame_stats().run;
    shell.frame().expect("a frame");
    shell.frame_stats().run > before
}

/// Runs frames until the shell is quiet: the frame after the last one that did work finds nothing and stops.
#[track_caller]
pub fn go_quiet(shell: &mut JsShell) {
    for _ in 0..12 {
        if !frame_runs(shell) {
            return;
        }
    }
    panic!("the shell never went quiet: {:?}", shell.frame_stats());
}

// ---- the rig ------------------------------------------------------------------------------------------------------
pub struct Rig {
    pub shell: JsShell,
    pub comp: FakeCompositor,
    pub load_diagnostics: Vec<String>,
    pub clock: ManualClock,
    pub _dir: TempDir,
}

impl Rig {
    /// Moves the shell's clock forward by `ms` milliseconds. The frames that follow see the time pass; no real time
    /// does.
    pub fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    /// Runs frames until the shell is quiet. See [`go_quiet`].
    #[track_caller]
    pub fn quiet(&mut self) {
        go_quiet(&mut self.shell);
    }

    /// Whether the next frame did its work. See [`frame_runs`].
    pub fn next_frame_runs(&mut self) -> bool {
        frame_runs(&mut self.shell)
    }

    pub fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// Runs frames until the shell and the compositor have nothing left to say, and returns every diagnostic.
    pub fn settle(&mut self) -> Vec<String> {
        let mut all = Vec::new();
        for _ in 0..6 {
            all.extend(self.frame());
        }
        all
    }

    pub fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .expect("the script has a value")
    }

    /// The node ids of the canvases `ids` holds in the script, in order.
    pub fn ids(&mut self, expression: &str) -> Vec<u64> {
        self.eval(&format!("{expression}.join(',')"))
            .split(',')
            .map(|id| id.parse().expect("a node id"))
            .collect()
    }
}

/// The stock titlebar of window `window`: the node in the frame's top outlet, which is the window's one child.
pub fn titlebar(rig: &Rig, window: u64) -> u64 {
    let outlet = rig.comp.children(window)[0];
    rig.comp.children(outlet)[0]
}

/// A shell over a fresh fake compositor whose `shell.ts` is `imports` followed by `init_body`.
pub fn rig_with(imports: &str, init_body: &str) -> Rig {
    rig_setup(imports, init_body, |_| {})
}

/// Like [`rig_with`], with `setup` run on the compositor before the shell attaches: the nodes that are there at boot.
pub fn rig_setup(imports: &str, init_body: &str, setup: impl FnOnce(&FakeCompositor)) -> Rig {
    rig_configured(imports, init_body, setup, |_| {})
}

/// Like [`rig_setup`], with `configure` run on the shell's config before the shell boots: a bytecode cache, a stock
/// directory.
pub fn rig_configured(
    imports: &str,
    init_body: &str,
    setup: impl FnOnce(&FakeCompositor),
    configure: impl FnOnce(&mut JsShellConfig),
) -> Rig {
    let comp = FakeCompositor::new();
    setup(&comp);
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), format!("{imports}{init_body}\n")).expect("shell.ts is written");
    let (mut config, clock) = timed_config(&root);
    configure(&mut config);
    let mut shell = JsShell::new(Box::new(comp.clone()), config).expect("the shell boots");
    let load_diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    Rig {
        shell,
        comp,
        load_diagnostics,
        clock,
        _dir: dir,
    }
}

/// A module prelude that puts the library's exports where `rig.eval` can reach them as globals (`FloatingLayout`,
/// `startDecorations`...). The `gnoblin` global is the runtime's own and is left out.
pub const LIBRARY_GLOBALS: &str = "import * as gCore from \"gnoblin-core\"; import * as gStd from \"gnoblin-std\"; const g = { ...gCore, ...gStd };\nconst { gnoblin: _runtime, ...exported } = g;\nObject.assign(globalThis, exported);\n";

/// What the toolkit's tests read of a canvas: the text of its paint list. It is a trait, not a method of the fake, because the
/// fake knows nothing of the painter.
pub trait CanvasText {
    /// The text of every `text` op of the canvas `node`'s paint list.
    fn text_of(&self, node: u64) -> Vec<String>;

    /// Every canvas whose paint list holds `text`, in id order.
    fn canvases_showing(&self, text: &str) -> Vec<u64>;
}

impl CanvasText for FakeCompositor {
    fn text_of(&self, node: u64) -> Vec<String> {
        self.value(node, "content")
            .map_or_else(Vec::new, |content| match content {
                MirrorValue::Bytes(bytes) => gnoblin_paint::decode(&bytes).map_or_else(
                    |_| Vec::new(),
                    |list| {
                        list.ops
                            .iter()
                            .filter_map(|op| match op {
                                gnoblin_paint::PaintOp::Text { text, .. } => Some(text.clone()),
                                _ => None,
                            })
                            .collect()
                    },
                ),
                _ => Vec::new(),
            })
    }

    fn canvases_showing(&self, text: &str) -> Vec<u64> {
        self.canvases()
            .into_iter()
            .filter(|id| self.text_of(*id).iter().any(|t| t == text))
            .collect()
    }
}
