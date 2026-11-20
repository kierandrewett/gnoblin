//! Example 2 and example 17 of `design/modular-framework.md` section 12, as tests (M7.1 to M7.3).
//!
//! The examples are real shells: `cdylib` crates under `examples/` that depend on the SDK crates and nothing else. These tests build
//! them with nested `cargo` runs, stage a prefix the way an install does (the core, the std library under its SONAME, the launcher
//! fixture, the shells), load each shell through the shell host's loader into a process that links the installed core, put the fake
//! compositor behind the tree link, and drive windows, outputs and a chord through it. What the shell did is asserted from what the
//! compositor holds afterwards, and the ELF checks the design names are made on the files: a std shell needs the std library and the
//! core library (std first, then the core it builds on), the std library needs the core and nothing else of ours, the process maps one
//! of each, and a shell that does not use std never loads it, even from a prefix that has it.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[path = "../../gnoblin-core-impl/tests/support/mod.rs"]
mod support;
use support::*;

mod reference;
use reference::Reference;

const STD_FILE: &str = "libgnoblin-std.so.1";

/// Two staged prefixes: one with the std library installed and one without, the same core, launcher and shells in both.
struct Stage {
    /// The prefix with `libgnoblin-std.so.1`.
    with_std: PathBuf,
    /// The prefix without it (example 17).
    without_std: PathBuf,
    std_library: PathBuf,
    std_shell: PathBuf,
    /// Example 6, in the prefix with std.
    rust_layout: PathBuf,
    /// The fixture that switches between the library's tiling layouts, in the prefix with std.
    layouts: PathBuf,
    /// The fixture with the move and resize bindings and a drag binding of its own.
    bindings: PathBuf,
    /// The shell of the specification's section 3.1 (`examples/02-std-prelude/examples/spec_text.rs`), in the prefix with std.
    spec_text: PathBuf,
    core_shell: PathBuf,
    /// The std shell as staged in the prefix without std.
    std_shell_alone: PathBuf,
    core_shell_alone: PathBuf,
    /// A directory with a stand-in `foot`, which creates the file `GNOBLIN_TEST_MARKER` names.
    bin: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("std-examples-stage");
    let _ = fs::remove_dir_all(&base);
    let target = base.join("target");
    cargo_build(
        &target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-std-impl",
            "-p",
            "gnoblin-fixture-launcher",
            "-p",
            "gnoblin-example-std-prelude",
            "-p",
            "gnoblin-example-minimal-core-shell",
            "-p",
            "gnoblin-example-rust-layout",
            "-p",
            "gnoblin-fixture-shell-layouts",
            "-p",
            "gnoblin-fixture-shell-bindings",
            "--features",
            "gnoblin-core-impl/test-driver",
        ],
        &[],
    )?;
    // The spec's shell is an `example` target of the same package, and selecting one target leaves the rest unbuilt, so it is its own
    // build into the same target directory.
    cargo_build(
        &target,
        &["-p", "gnoblin-example-std-prelude", "--example", "spec_text"],
        &[],
    )?;
    let debug = target.join("debug");
    let with_std = base.join("with-std");
    let without_std = base.join("without-std");
    let mut shells = Vec::new();
    for dir in [&with_std, &without_std] {
        copy_to(&debug.join("libgnoblin_core_impl.so"), &dir.join(CORE_FILE));
        copy_to(&debug.join(LAUNCHER), &dir.join(LAUNCHER));
        shells.push((
            copy_to(&debug.join("libstd_prelude.so"), &dir.join("shells/libstd_prelude.so")),
            copy_to(
                &debug.join("libminimal_core_shell.so"),
                &dir.join("shells/libminimal_core_shell.so"),
            ),
        ));
    }
    let rust_layout = copy_to(
        &debug.join("librust_layout.so"),
        &with_std.join("shells/librust_layout.so"),
    );
    let layouts = copy_to(
        &debug.join("libshell_layouts.so"),
        &with_std.join("shells/libshell_layouts.so"),
    );
    let bindings = copy_to(
        &debug.join("libshell_bindings.so"),
        &with_std.join("shells/libshell_bindings.so"),
    );
    let spec_text = copy_to(
        &debug.join("examples/libspec_text.so"),
        &with_std.join("shells/libspec_text.so"),
    );
    let std_library = copy_to(&debug.join("libgnoblin_std_impl.so"), &with_std.join(STD_FILE));
    let bin = base.join("bin");
    fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    let foot = bin.join("foot");
    fs::write(&foot, "#!/bin/sh\n: > \"$GNOBLIN_TEST_MARKER\"\n").map_err(|error| error.to_string())?;
    fs::set_permissions(&foot, fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;
    let (std_shell, core_shell) = shells.remove(0);
    let (std_shell_alone, core_shell_alone) = shells.remove(0);
    Ok(Stage {
        with_std,
        without_std,
        std_library,
        std_shell,
        rust_layout,
        layouts,
        bindings,
        spec_text,
        core_shell,
        std_shell_alone,
        core_shell_alone,
        bin,
    })
}

fn stage() -> &'static Stage {
    static STAGE: LazyLock<Result<Stage, String>> = LazyLock::new(build_stage);
    match &*STAGE {
        Ok(stage) => stage,
        Err(message) => panic!("cannot build the examples: {message}"),
    }
}

/// Runs scenario `name` over `shell` in `dir`, with the stand-in terminal on the `PATH`. The shell finds `libgnoblin-std.so.1` through
/// `LD_LIBRARY_PATH` when the prefix has one: a private prefix is not on the loader's search path, and the shell carries no run path.
fn scenario_of(stage: &Stage, dir: &Path, name: &str, shell: &Path) -> (String, String, i32) {
    let marker = dir.join(format!(
        "marker-{name}-{}",
        shell.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_file(&marker);
    let path = format!(
        "{}:{}",
        stage.bin.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned())
    );
    run_with_env(
        dir,
        &[name],
        &[shell],
        &[
            ("PATH", Path::new(&path)),
            ("GNOBLIN_TEST_MARKER", marker.as_path()),
            ("LD_LIBRARY_PATH", dir),
        ],
    )
}

/// The rectangles printed under `step=<name>`: the window id and its rectangle (`None` for a window that is gone).
fn step(stdout: &str, name: &str) -> Rects {
    let marker = format!("step={name}");
    let mut inside = false;
    let mut found = Vec::new();
    for line in stdout.lines() {
        if line.starts_with("step=") {
            inside = line == marker;
        } else if inside && let Some(rest) = line.strip_prefix("window id=") {
            let mut parts = rest.split_whitespace();
            let id = parts.next().unwrap().parse().unwrap();
            let numbers: Vec<f64> = parts
                .filter_map(|part| part.split_once('=').and_then(|(_, value)| value.parse().ok()))
                .collect();
            found.push((id, <[f64; 4]>::try_from(numbers).ok()));
        }
    }
    found
}

/// The window ids and rectangles of one step.
type Rects = Vec<(u64, Option<[f64; 4]>)>;

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn inside(rect: [f64; 4], area: [f64; 4]) -> bool {
    rect[0] >= area[0]
        && rect[1] >= area[1]
        && rect[0] + rect[2] <= area[0] + area[2]
        && rect[1] + rect[3] <= area[1] + area[3]
}

// ---- the files --------------------------------------------------------------------------------------------------------------

#[test]
fn the_std_library_has_its_soname_its_exports_and_needs_the_core_and_nothing_else_of_ours() {
    let stage = stage();
    let dynamic = dynamic(&stage.std_library);
    assert_eq!(dynamic.soname.as_deref(), Some(STD_FILE));
    assert!(
        dynamic.flags_1.contains("NODELETE"),
        "retired generations' registrations point into the library: {dynamic:?}"
    );
    let ours: Vec<&String> = dynamic
        .needed
        .iter()
        .filter(|needed| needed.starts_with("libgnoblin"))
        .collect();
    assert_eq!(
        ours,
        [CORE_FILE],
        "the std library needs the core through the core's ABI and no other library of ours: {dynamic:?}"
    );
    let expected: BTreeSet<String> = gnoblin_std_abi::export_names().map(str::to_owned).collect();
    assert_eq!(
        defined_symbols(&stage.std_library),
        expected,
        "the library defines the std exports and nothing else, no copy of a core symbol"
    );
    // Every core symbol it calls is one the core exports.
    let wanted: BTreeSet<String> = undefined_symbols(&stage.std_library)
        .into_iter()
        .filter(|symbol| symbol.starts_with("gnoblin_"))
        .collect();
    assert!(!wanted.is_empty(), "the library calls the core");
    assert!(wanted.is_subset(&exports()), "{wanted:?}");
}

#[test]
fn example_2_needs_std_then_the_core_and_defines_one_entry_symbol() {
    let stage = stage();
    let dynamic = dynamic(&stage.std_shell);
    let ours: Vec<&str> = dynamic
        .needed
        .iter()
        .filter(|needed| needed.starts_with("libgnoblin"))
        .map(String::as_str)
        .collect();
    assert_eq!(ours.len(), 2, "{dynamic:?}");
    assert!(ours.contains(&STD_FILE) && ours.contains(&CORE_FILE), "{dynamic:?}");
    assert_eq!(
        defined_symbols(&stage.std_shell),
        BTreeSet::from(["gnoshell_entry_v1".to_owned()]),
        "the shell defines no gnoblin_* symbol: the libraries' are the only ones in the process"
    );
    let wanted: BTreeSet<String> = undefined_symbols(&stage.std_shell)
        .into_iter()
        .filter(|symbol| symbol.starts_with("gnoblin_"))
        .collect();
    let std_exports: BTreeSet<String> = gnoblin_std_abi::export_names().map(str::to_owned).collect();
    let allowed: BTreeSet<String> = exports().union(&std_exports).cloned().collect();
    assert!(
        wanted.iter().any(|symbol| std_exports.contains(symbol)),
        "the shell calls the std library: {wanted:?}"
    );
    assert!(wanted.is_subset(&allowed), "{wanted:?}");
}

// ---- example 2 --------------------------------------------------------------------------------------------------------------

#[test]
fn example_2_gives_every_window_a_place_on_its_outputs_workspace_and_starts_a_terminal() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.with_std, "example02", &stage.std_shell);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("loaded label=gnoblin-example-std-prelude"), "{stdout}");
    let screen_one = [0.0, 0.0, 1000.0, 800.0];
    let screen_two = [1000.0, 0.0, 800.0, 600.0];

    // Three windows on the one output: every one inside it, none on top of another.
    let three = step(&stdout, "three");
    assert_eq!(three.len(), 3, "{stdout}");
    let rects: Vec<[f64; 4]> = three.iter().map(|(_, rect)| rect.expect("the window exists")).collect();
    for rect in &rects {
        assert!(inside(*rect, screen_one), "{rect:?} is off the screen: {stdout}");
    }
    for (index, a) in rects.iter().enumerate() {
        for b in &rects[index + 1..] {
            assert!(!overlaps(*a, *b), "{a:?} and {b:?} overlap: {stdout}");
        }
    }

    // The window that opened with the pointer on the second output joined that output's workspace.
    let second = step(&stdout, "second_output");
    let fourth = second
        .iter()
        .find(|(id, _)| *id == 43)
        .and_then(|(_, rect)| *rect)
        .expect("window 43");
    assert!(
        inside(fourth, screen_two),
        "{fourth:?} is not on the second output: {stdout}"
    );
    // The first output's windows stayed where they were.
    assert_eq!(&second[..3], &three[..], "{stdout}");

    // A window that closed is gone and the others did not move.
    let closed = step(&stdout, "closed");
    assert_eq!(closed[1], (41, None), "{stdout}");
    assert_eq!(closed[0], three[0], "{stdout}");
    assert_eq!(closed[2], three[2], "{stdout}");

    assert!(stdout.contains("terminal_started=true"), "{stdout}");
    assert!(stderr.contains("std prelude shell is running"), "{stderr}");
    assert!(stderr.contains("client 40 is on workspace"), "{stderr}");
    assert!(stderr.contains("client 43 is on workspace"), "{stderr}");
    assert!(!stderr.contains("cannot"), "{stderr}");

    // The chain shell -> std -> core, one of each in the process.
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "shell_mappings"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "std_mappings"), 1, "{stdout}");
    assert_eq!(num(&stdout, "core_stats", "panics"), 0, "{stdout}");
}

// ---- the specification's section 3.1 -------------------------------------------------------------------------------------------

#[test]
fn example_2_spec_text_shell_places_windows_on_its_workspace_and_a_plus_form_chord_starts_a_terminal() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.with_std, "example02_spec", &stage.spec_text);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("loaded label=gnoblin-example-std-prelude"), "{stdout}");
    let screen = [0.0, 0.0, 1000.0, 800.0];

    // The window that was open before the shell started is not the workspace's; the two that opened afterwards are, each placed
    // inside the screen and clear of the other.
    let three = step(&stdout, "three");
    assert_eq!(three.len(), 3, "{stdout}");
    let rects: Vec<[f64; 4]> = three[1..]
        .iter()
        .map(|(_, rect)| rect.expect("the window exists"))
        .collect();
    for rect in &rects {
        assert!(inside(*rect, screen), "{rect:?} is off the screen: {stdout}");
    }
    assert!(!overlaps(rects[0], rects[1]), "{rects:?} overlap: {stdout}");
    assert_eq!(
        three[0],
        step(&stdout, "one")[0],
        "the first window did not move: {stdout}"
    );

    // `Super+Return` is the spec's spelling, and the SDK claims it as written: the compositor's parser reads the plus form.
    assert!(
        stdout.lines().any(|line| line == "claims=Super+Return"),
        "the claim is the text the shell wrote: {stdout}"
    );
    assert!(stdout.contains("terminal_started=true"), "{stdout}");
    assert!(!stderr.contains("callback failed"), "{stderr}");
    assert_eq!(num(&stdout, "core_stats", "panics"), 0, "{stdout}");
}

// ---- example 17 -------------------------------------------------------------------------------------------------------------

#[test]
fn example_17_a_core_only_shell_runs_without_the_std_library_installed() {
    let stage = stage();
    assert!(!stage.without_std.join(STD_FILE).exists());
    let (stdout, stderr, code) = scenario_of(stage, &stage.without_std, "example01", &stage.core_shell_alone);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("terminal_started=true"), "{stdout}");
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "std_mappings"), 0, "{stdout}");
}

#[test]
fn example_17_a_std_shell_without_the_library_fails_naming_it() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.without_std, "example02", &stage.std_shell_alone);
    assert_ne!(code, 0, "{stdout}\n{stderr}");
    assert!(
        stdout.contains("load_failed") && stdout.contains(STD_FILE),
        "the failure names the library the shell needs: {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("cannot open shared object file"),
        "and says why, in the loader's words: {stdout}"
    );
}

#[test]
fn a_core_only_shell_never_loads_the_std_library_even_from_a_prefix_that_has_it() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.with_std, "example01", &stage.core_shell);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(
        num(&stdout, "maps", "std_mappings"),
        0,
        "the library is on the search path and the shell does not name it: {stdout}"
    );
    let needed = dynamic(&stage.core_shell).needed;
    assert!(!needed.iter().any(|needed| needed.contains("std")), "{needed:?}");
}

// ---- parity with the reference ----------------------------------------------------------------------------------------------
//
// The TypeScript std (`data/js/gnoblin-std`) is the reference for what the native one does. The same script of the compositor, run
// once over the TypeScript std in this process and once over the native shell in the launcher, must leave the windows where the other
// left them.

use std::time::Duration;

use gnoblin_js::JsShellConfig;
use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
use gnoblin_testkit::oracle::{self, Value, json};
use gnoblin_tree_client::tree_mirror::MirrorValue;

/// The world of the launcher's scenarios (`driver_examples::world`): a 1000 by 800 screen, a seat and the window 40, with both spellings
/// of the screen's geometry because the two libraries read the rectangle by different keys.
fn reference_world() -> FakeCompositor {
    let comp = FakeCompositor::new();
    let double = MirrorValue::Double;
    comp.add_node(
        20,
        ROOT,
        "screen",
        vec![
            ("screen_x", double(0.0)),
            ("screen_y", double(0.0)),
            ("screen_width", double(1000.0)),
            ("screen_height", double(800.0)),
            ("x", double(0.0)),
            ("y", double(0.0)),
            ("width", double(1000.0)),
            ("height", double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
    comp.add_window(40, "Terminal", "foot");
    comp
}

/// Runs the TypeScript shell `source` over `comp`: the frames before and after each step of `steps`, which may create windows.
fn reference_run(comp: &FakeCompositor, source: &str, mut steps: impl FnMut(&FakeCompositor, &mut Reference)) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), source).expect("shell.ts is written");
    let clock = ManualClock::default();
    let mut config =
        JsShellConfig::new(&root).with_stock(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js"));
    config.entry_budget = Duration::from_secs(5);
    config.frame_budget = Duration::from_secs(10);
    config.clock = std::sync::Arc::new(clock);
    let mut shell = Reference::new(comp, config);
    let diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for _ in 0..6 {
        shell.frame().expect("a frame");
    }
    steps(comp, &mut shell);
}

fn settle(shell: &mut Reference) {
    for _ in 0..6 {
        let diagnostics = shell.frame().expect("a frame").diagnostics;
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

fn rect_of(comp: &FakeCompositor, id: u64) -> Option<[f64; 4]> {
    comp.exists(id).then(|| {
        [
            comp.number(id, "x"),
            comp.number(id, "y"),
            comp.number(id, "width"),
            comp.number(id, "height"),
        ]
    })
}

/// What a step shows, as a fixture holds it.
fn rects_json(rects: &[(u64, Option<[f64; 4]>)]) -> Value {
    Value::Array(
        rects
            .iter()
            .map(|(id, rect)| json!({ "id": id, "rect": rect }))
            .collect(),
    )
}

#[test]
fn floating_placement_matches_the_typescript_std() {
    let comp = reference_world();
    let seen = std::cell::RefCell::new(Vec::new());
    reference_run(&comp, "import \"gnoblin-std\";\nexport {};\n", |comp, shell| {
        settle(shell);
        comp.create_window(41, "Files", "nautilus");
        settle(shell);
        comp.create_window(42, "Editor", "code");
        settle(shell);
        *seen.borrow_mut() = [40, 41, 42].map(|id| (id, rect_of(comp, id))).to_vec();
    });
    let reference = seen.into_inner();
    reference::fixtures().json(
        "examples_floating_placement",
        &json!({ "steps": [{ "step": "three", "windows": rects_json(&reference) }] }),
    );
    if oracle::recording() {
        return;
    }

    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.with_std, "example02", &stage.std_shell);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    let native = step(&stdout, "three");
    assert_eq!(
        native, reference,
        "the native std places the first three windows as the TypeScript std does\nnative: {stdout}"
    );
}

// ---- example 6 --------------------------------------------------------------------------------------------------------------

#[test]
fn example_6_arranges_the_clients_of_a_workspace_in_columns_as_they_open_and_close() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.with_std, "example06", &stage.rust_layout);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stderr.contains("rust layout shell is running"), "{stderr}");

    // The workspace fills the 1000 by 800 screen less the outer gap of 20 on each side, and the columns are 10 apart. Each edge is
    // floored on its own: three columns of 313.33 start at 20, 343.33 and 666.67.
    assert_eq!(
        step(&stdout, "one"),
        [(40, Some([20.0, 20.0, 960.0, 760.0]))],
        "{stdout}"
    );
    assert_eq!(
        step(&stdout, "two"),
        [
            (40, Some([20.0, 20.0, 475.0, 760.0])),
            (41, Some([505.0, 20.0, 475.0, 760.0])),
        ],
        "{stdout}"
    );
    assert_eq!(
        step(&stdout, "three"),
        [
            (40, Some([20.0, 20.0, 313.0, 760.0])),
            (41, Some([343.0, 20.0, 313.0, 760.0])),
            (42, Some([666.0, 20.0, 314.0, 760.0])),
        ],
        "{stdout}"
    );
    // `<Super>Tab` swapped the focused third window with the first: the layout was not told how, it read the new order.
    assert_eq!(
        step(&stdout, "swapped"),
        [
            (40, Some([666.0, 20.0, 314.0, 760.0])),
            (41, Some([343.0, 20.0, 313.0, 760.0])),
            (42, Some([20.0, 20.0, 313.0, 760.0])),
        ],
        "{stdout}"
    );
    // A window that closes leaves its column and the others share the room.
    assert_eq!(
        step(&stdout, "closed"),
        [
            (40, Some([505.0, 20.0, 475.0, 760.0])),
            (41, None),
            (42, Some([20.0, 20.0, 475.0, 760.0])),
        ],
        "{stdout}"
    );
    assert!(!stderr.contains("cannot"), "{stderr}");
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "std_mappings"), 1, "{stdout}");
    assert_eq!(num(&stdout, "core_stats", "panics"), 0, "{stdout}");
}

// ---- the tiling layouts against the reference ------------------------------------------------------------------------------

/// The TypeScript std on the same four windows and the same six layouts, switched the way the fixture's chords switch them.
fn reference_layouts() -> Vec<(String, Rects)> {
    let comp = reference_world();
    let seen = std::cell::RefCell::new(Vec::new());
    let source = "import \"gnoblin-std\";\n\
        import { Workspace, MasterStackLayout, GridLayout, MonocleLayout } from \"gnoblin-std\";\n\
        const g = globalThis as any;\n\
        g.g = g;\n\
        g.main = new Workspace({ id: \"main\", layout: new MasterStackLayout(), gaps: { inner: 6, outer: 12 } });\n\
        gnoblin.workspaces.add(g.main);\n\
        g.MasterStackLayout = MasterStackLayout; g.GridLayout = GridLayout; g.MonocleLayout = MonocleLayout;\n";
    reference_run(&comp, source, |comp, shell| {
        settle(shell);
        for (id, title) in [(41, "Files"), (42, "Editor"), (43, "Browser")] {
            comp.create_window(id, title, "app");
            settle(shell);
        }
        comp.external_set(42, "focused", MirrorValue::Bool(true));
        settle(shell);
        let layouts = [
            ("Super+1", "new MasterStackLayout()"),
            (
                "Super+2",
                "new MasterStackLayout({ ratio: 0.6, masterCount: 2, gap: 4, orientation: 'right' })",
            ),
            ("Super+3", "new GridLayout()"),
            ("Super+4", "new GridLayout({ columns: 3, fill: 'column' })"),
            ("Super+5", "new MonocleLayout()"),
            ("Super+6", "new MasterStackLayout({ orientation: 'top' })"),
        ];
        for (chord, layout) in layouts {
            shell
                .eval(&format!("g.main.layout = {layout};"))
                .expect("the layout is set");
            settle(shell);
            seen.borrow_mut().push((
                chord.to_owned(),
                [40, 41, 42, 43].map(|id| (id, rect_of(comp, id))).to_vec(),
            ));
        }
        shell
            .eval("g.main.layout = new MasterStackLayout();")
            .expect("the layout is set");
        settle(shell);
        comp.destroy(41);
        settle(shell);
        seen.borrow_mut().push((
            "closed".to_owned(),
            [40, 41, 42, 43].map(|id| (id, rect_of(comp, id))).to_vec(),
        ));
    });
    seen.into_inner()
}

#[test]
fn the_native_tiling_layouts_match_the_typescript_std() {
    let reference = reference_layouts();
    reference::fixtures().json(
        "examples_tiling_layouts",
        &json!({ "steps": reference.iter().map(|(name, rects)| json!({
            "step": name,
            "windows": rects_json(rects),
        })).collect::<Vec<_>>() }),
    );
    if oracle::recording() {
        return;
    }
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.with_std, "std_layouts", &stage.layouts);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stderr.contains("layouts shell is running"), "{stderr}");
    assert!(!stderr.contains("cannot"), "{stderr}");
    for (name, expected) in &reference {
        assert_eq!(
            &step(&stdout, name),
            expected,
            "layout step {name}\nnative output: {stdout}"
        );
    }
    assert_eq!(reference.len(), 7);

    // The swap binding: after the close the tiles are 40 (master), 42 and 43; dragging the bottom tile onto the master trades their
    // places in the workspace's order, and a drop on nothing changes nothing.
    assert_eq!(
        step(&stdout, "swapped"),
        [
            (40, Some([503.0, 403.0, 485.0, 385.0])),
            (42, Some([503.0, 12.0, 485.0, 385.0])),
            (43, Some([12.0, 12.0, 485.0, 776.0])),
        ],
        "{stdout}"
    );
    assert_eq!(step(&stdout, "dropped_nowhere"), step(&stdout, "swapped"), "{stdout}");
    assert_eq!(num(&stdout, "core_stats", "panics"), 0, "{stdout}");
}

/// `["Super+Button1", "<Alt>Button3"]` with every chord in the compositor's spelling (`<Super>Button1`, `<Alt>Button3`).
fn canonical_chords(list: &str) -> Vec<String> {
    list.trim_matches(|c| c == '[' || c == ']')
        .split(", ")
        .map(|chord| {
            let chord = chord.trim_matches('"');
            match chord.rsplit_once('+') {
                Some((modifiers, key)) if !chord.contains('<') => {
                    let mut spelled: String = modifiers.split('+').map(|modifier| format!("<{modifier}>")).collect();
                    spelled.push_str(key);
                    spelled
                }
                _ => chord.to_owned(),
            }
        })
        .collect()
}

// ---- pointer bindings (M7.5) ------------------------------------------------------------------------------------------------

#[test]
fn pointer_drags_move_and_resize_windows_and_a_binding_of_the_shells_own_runs() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.with_std, "std_bindings", &stage.bindings);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stderr.contains("bindings shell is running"), "{stderr}");
    assert!(!stderr.contains("cannot"), "{stderr}");
    // The chords are claimed in either spelling (the SDK rewrites the plus form for a shell's own drag, the library's go as written),
    // so they are compared in the compositor's.
    let claims = stdout
        .lines()
        .find_map(|line| line.strip_prefix("claims="))
        .unwrap_or_default();
    assert_eq!(
        canonical_chords(claims),
        canonical_chords("[\"Super+Button1\", \"Alt+Button3\", \"Super+Button2\"]"),
        "{stdout}"
    );
    let first = |name: &str| {
        step(&stdout, name)
            .into_iter()
            .find(|(id, _)| *id == 40)
            .and_then(|(_, rect)| rect)
    };
    let second = |name: &str| {
        step(&stdout, name)
            .into_iter()
            .find(|(id, _)| *id == 41)
            .and_then(|(_, rect)| rect)
    };

    // A pointer that stays under the threshold (4 pixels) moves nothing.
    assert_eq!(first("below_threshold"), Some([100.0, 100.0, 300.0, 200.0]), "{stdout}");
    // Past it the window follows the pointer, measured from where the press was: (110, 60) from (150, 150) to (260, 210).
    assert_eq!(first("moved"), Some([210.0, 160.0, 300.0, 200.0]), "{stdout}");
    // The work area is the screen: a window dragged far past its right and top edges stops at them.
    assert_eq!(first("held_in"), Some([700.0, 0.0, 300.0, 200.0]), "{stdout}");
    // A drag the compositor cancels puts the window back where the press found it.
    assert_eq!(second("cancelled"), Some([500.0, 400.0, 200.0, 150.0]), "{stdout}");
    // A press on no window starts nothing, and no window moved.
    assert_eq!(first("nothing"), Some([700.0, 0.0, 300.0, 200.0]), "{stdout}");
    assert_eq!(second("nothing"), Some([500.0, 400.0, 200.0, 150.0]), "{stdout}");

    // Resize by the nearest side: the right edge follows the pointer, then the top edge, then the left edge is stopped at the
    // smallest width and the right edge stays where it was.
    assert_eq!(second("resized_right"), Some([500.0, 400.0, 242.0, 150.0]), "{stdout}");
    assert_eq!(second("resized_top"), Some([500.0, 378.0, 242.0, 172.0]), "{stdout}");
    assert_eq!(second("resized_min"), Some([642.0, 378.0, 100.0, 172.0]), "{stdout}");

    // The shell's own binding: asked to start only past ten pixels, told where the press was, and the window was raised through the
    // core.
    assert!(
        stderr.contains("raiser start 40 at 800,100 local Some((100.0, 100.0))"),
        "{stderr}"
    );
    assert!(stderr.contains("raiser update 810,112"), "{stderr}");
    assert!(stderr.contains("raiser end"), "{stderr}");
    assert!(!stderr.contains("raiser update 804"), "{stderr}");
    assert!(stdout.contains("order_before=[20, 21, 40, 41]"), "{stdout}");
    assert!(stdout.contains("order_after=[20, 21, 41, 40]"), "{stdout}");
    assert_eq!(num(&stdout, "core_stats", "panics"), 0, "{stdout}");
}

// ---- example 9 --------------------------------------------------------------------------------------------------------------

#[test]
fn example_9_a_std_shell_links_the_installed_libraries_and_ldd_is_clean() {
    let stage = stage();
    // The loader's own answer, with the prefix as the only place to look: both libraries of ours resolve to the files of the prefix, and
    // nothing is missing.
    let output = std::process::Command::new("ldd")
        .arg(&stage.std_shell)
        .env("LD_LIBRARY_PATH", &stage.with_std)
        .output()
        .expect("ldd is needed for this test");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!text.contains("not found"), "{text}");
    for library in [STD_FILE, CORE_FILE] {
        let line = text
            .lines()
            .find(|line| line.trim_start().starts_with(library))
            .unwrap_or_else(|| panic!("`{library}` is not in the shell's dependencies:\n{text}"));
        assert!(line.contains(&stage.with_std.display().to_string()), "{line}");
    }
    // Without the prefix on the path the loader says what is missing, and it is the std library.
    let output = std::process::Command::new("ldd")
        .arg(&stage.std_shell)
        .env_remove("LD_LIBRARY_PATH")
        .output()
        .expect("ldd is needed for this test");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.lines()
            .any(|line| line.contains(STD_FILE) && line.contains("not found")),
        "{text}"
    );
}
