//! The Python runtime library, loaded by the core's loader into a process that links the installed core, serving shells over the fake
//! compositor.
//!
//! Nothing here is the runtime's own test harness: the process is the core's test launcher, the shell is a `.py` file the loader runs
//! by its extension through `libgnoblin-runtime-python.so`, and what the tests read is the compositor's tree and the log. Examples 7
//! and 8 are the files of `examples/`, run as they are; examples 11 and 17 are built from them.
//!
//! What this does not prove, because the fake compositor has no Wayland behind it: that a client's connection survives a reload (the
//! property that makes it true is proven: the windows are neither destroyed nor made again).

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

#[path = "../../gnoblin-core-impl/tests/support/mod.rs"]
mod support;
use support::*;

struct Stage {
    dir: PathBuf,
    runtimes: PathBuf,
    /// The same prefix with `libgnoblin-std.so.1` beside the core: the one a std shell runs in.
    with_std: PathBuf,
    with_std_runtimes: PathBuf,
    /// A directory with a stand-in `foot`, which creates the file `GNOBLIN_TEST_MARKER` names.
    bin: PathBuf,
    runtime: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("runtime-python-stage");
    let _ = fs::remove_dir_all(&base);
    let target = base.join("target-main");
    let dir = base.join("core10");
    cargo_build(
        &target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-fixture-launcher",
            "-p",
            "gnoblin-runtime-python",
            "-p",
            "gnoblin-std-impl",
            "-p",
            "gnoblin-fixture-shell-layouts",
            "--features",
            "gnoblin-core-impl/test-driver",
        ],
        &[],
    )?;
    let debug = target.join("debug");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &dir.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &dir.join(LAUNCHER));
    let runtimes = dir.join("runtimes");
    let runtime = copy_to(
        &debug.join("libgnoblin_runtime_python.so"),
        &runtimes.join("libgnoblin-runtime-python.so"),
    );
    // A second prefix of the same files with the std library beside the core, where `gnoblin.std` finds it.
    let with_std = base.join("core10-std");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &with_std.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &with_std.join(LAUNCHER));
    let with_std_runtimes = with_std.join("runtimes");
    copy_to(
        &debug.join("libgnoblin_runtime_python.so"),
        &with_std_runtimes.join("libgnoblin-runtime-python.so"),
    );
    copy_to(
        &debug.join("libgnoblin_std_impl.so"),
        &with_std.join("libgnoblin-std.so.1"),
    );
    // The native Rust shell of the same layouts, which the scripts are held to.
    copy_to(
        &debug.join("libshell_layouts.so"),
        &with_std.join("shells/libshell_layouts.so"),
    );
    let bin = base.join("bin");
    fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    let foot = bin.join("foot");
    fs::write(&foot, "#!/bin/sh\n: > \"$GNOBLIN_TEST_MARKER\"\n").map_err(|error| error.to_string())?;
    fs::set_permissions(&foot, fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;
    Ok(Stage {
        dir,
        runtimes,
        with_std,
        with_std_runtimes,
        bin,
        runtime,
    })
}

fn stage() -> &'static Stage {
    static STAGE: LazyLock<Result<Stage, String>> = LazyLock::new(build_stage);
    match &*STAGE {
        Ok(stage) => stage,
        Err(message) => panic!("cannot build the fixtures: {message}"),
    }
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

impl Run {
    fn dump(&self) -> String {
        format!(
            "exit {}\n--- stdout\n{}\n--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }

    fn lines_with(&self, needle: &str) -> Vec<&str> {
        self.stdout.lines().filter(|line| line.contains(needle)).collect()
    }

    /// The lines after the line `mark NAME` and before the next mark.
    fn section(&self, name: &str) -> Vec<&str> {
        let marker = format!("mark {name}");
        self.stdout
            .lines()
            .skip_while(|line| *line != marker)
            .skip(1)
            .take_while(|line| !line.starts_with("mark "))
            .collect()
    }

    /// The `window` line of `id` in `lines`, as its numbers.
    fn window(&self, lines: &[&str], id: u64) -> (f64, f64, f64, f64) {
        let line = lines
            .iter()
            .find(|line| line.starts_with(&format!("window id={id} ")))
            .unwrap_or_else(|| panic!("no window {id} in\n{}", self.dump()));
        let field = |key: &str| -> f64 {
            line.split_whitespace()
                .find_map(|word| word.strip_prefix(&format!("{key}=")))
                .and_then(|value| value.parse().ok())
                .unwrap_or_else(|| panic!("no `{key}` in `{line}`"))
        };
        (field("x"), field("y"), field("w"), field("h"))
    }

    fn claims<'a>(&'a self, lines: &[&'a str]) -> &'a str {
        lines
            .iter()
            .find(|line| line.starts_with("tree "))
            .unwrap_or_else(|| panic!("no tree line in\n{}", self.dump()))
    }
}

/// The environment every run gets: the runtime library's directory, the Python package of this source tree, no bytecode written.
fn environment(extra: &[(&'static str, PathBuf)]) -> Vec<(&'static str, PathBuf)> {
    environment_in(&stage().runtimes, extra)
}

/// [`environment`] for the prefix whose runtime library is in `runtimes`.
fn environment_in(runtimes: &Path, extra: &[(&'static str, PathBuf)]) -> Vec<(&'static str, PathBuf)> {
    let stage = stage();
    let mut env = vec![
        ("GNOBLIN_RUNTIME_DIR", runtimes.to_owned()),
        ("PYTHONDONTWRITEBYTECODE", PathBuf::from("1")),
        (
            "PATH",
            PathBuf::from(format!(
                "{}:{}",
                stage.bin.display(),
                std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned())
            )),
        ),
    ];
    env.extend(extra.iter().cloned());
    env
}

fn finish(output: std::process::Output) -> Run {
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

/// Runs the reload scenario of the launcher over `script`: the shell starts, and each step is performed in order.
fn drive(script: &Path, steps: &[&str]) -> Run {
    let mut all: Vec<PathBuf> = vec![PathBuf::from("reload"), script.to_owned()];
    all.extend(steps.iter().map(PathBuf::from));
    let refs: Vec<&Path> = all.iter().map(PathBuf::as_path).collect();
    let env = environment(&[]);
    let extra: Vec<(&str, &Path)> = env.iter().map(|(key, value)| (*key, value.as_path())).collect();
    finish(scenario_with_env(&stage().dir, &refs, &extra))
}

/// Runs a named scenario of the launcher (`example08`) over `script`, with the stand-in terminal's marker named after the scenario.
fn scenario_of(name: &str, script: &Path) -> Run {
    scenario_in(&stage().dir, &stage().runtimes, name, script)
}

/// [`scenario_of`] in the prefix `dir`, whose runtime library is in `runtimes`.
fn scenario_in(dir: &Path, runtimes: &Path, name: &str, script: &Path) -> Run {
    let marker = dir.join(format!("marker-{name}"));
    let _ = fs::remove_file(&marker);
    let env = environment_in(runtimes, &[("GNOBLIN_TEST_MARKER", marker)]);
    let extra: Vec<(&str, &Path)> = env.iter().map(|(key, value)| (*key, value.as_path())).collect();
    let name_path = PathBuf::from(name);
    finish(scenario_with_env(dir, &[name_path.as_path(), script], &extra))
}

fn example(name: &str) -> PathBuf {
    repo_root().join("examples").join(name)
}

/// A scratch directory for a project the test edits.
fn project(name: &str) -> PathBuf {
    let dir = stage().dir.join("projects").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The `window` lines of a scenario's output, in order: the id, and the rectangle (`None` for a window that is gone).
fn windows(stdout: &str) -> Vec<(u64, Option<[f64; 4]>)> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("window id="))
        .map(|rest| {
            let mut parts = rest.split_whitespace();
            let id = parts.next().unwrap().parse().unwrap();
            let numbers: Vec<f64> = parts
                .filter_map(|part| part.split_once('=').and_then(|(_, value)| value.parse().ok()))
                .collect();
            (id, <[f64; 4]>::try_from(numbers).ok())
        })
        .collect()
}

/// The rectangles printed under `step=<name>`, by window id.
fn step(stdout: &str, name: &str) -> Vec<(u64, Option<[f64; 4]>)> {
    let marker = format!("step={name}");
    let mut inside = false;
    let mut section = String::new();
    for line in stdout.lines() {
        if line.starts_with("step=") {
            inside = line == marker;
        } else if inside {
            section.push_str(line);
            section.push('\n');
        }
    }
    windows(&section)
}

/// The lines of the marker file a shell's programs appended to, once `expected` lines are there (programs run on their own clock).
fn marker_lines(marker: &Path, expected: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let lines: Vec<String> = fs::read_to_string(marker)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        if lines.len() >= expected || Instant::now() > deadline {
            return lines;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn count(lines: &[String], wanted: &str) -> usize {
    lines.iter().filter(|line| *line == wanted).count()
}

// ---- the library ------------------------------------------------------------------------------------------------------------

#[test]
fn the_runtime_library_exports_its_entry_and_needs_the_core_and_libpython() {
    let runtime = &stage().runtime;
    let dynamic = dynamic(runtime);
    assert!(dynamic.needed.iter().any(|needed| needed == CORE_FILE), "{dynamic:?}");
    assert!(
        dynamic.needed.iter().any(|needed| needed.starts_with("libpython3")),
        "the runtime links the system libpython: {:?}",
        dynamic.needed
    );
    assert!(
        !dynamic
            .needed
            .iter()
            .any(|needed| needed.contains("gnoblin") && needed != CORE_FILE),
        "the runtime loads no other library of ours: {:?}",
        dynamic.needed
    );
    let defined = defined_symbols(runtime);
    assert!(defined.contains("gnoruntime_entry_v1"), "{defined:?}");
    assert_eq!(
        defined.len(),
        1,
        "the runtime exports its entry and nothing else (a version script hides PyO3's `PyInit__gnoblin_core`): {defined:?}"
    );
    let own: Vec<&String> = defined
        .iter()
        .filter(|symbol| symbol.starts_with("gnoblin_") || symbol.starts_with("gnoshell_"))
        .collect();
    assert!(
        own.is_empty(),
        "the runtime defines nothing in the core's namespace: {own:?}"
    );
}

// ---- the runtime serves a shell -------------------------------------------------------------------------------------------------

#[test]
fn example_8_a_python_tiler_places_raw_window_nodes_and_makes_no_node_of_its_own() {
    let run = scenario_of("example08", &example("08-no-workspaces/shell.py"));
    assert_eq!(run.code, 0, "{}", run.dump());
    // The same numbers as the Rust example 8: the screen is 1000 by 800 and the gap 10.
    assert_eq!(
        step(&run.stdout, "one"),
        [(40, Some([10.0, 10.0, 980.0, 780.0]))],
        "{}",
        run.dump()
    );
    assert_eq!(
        step(&run.stdout, "two"),
        [
            (40, Some([10.0, 10.0, 485.0, 780.0])),
            (41, Some([505.0, 10.0, 485.0, 780.0])),
        ],
        "{}",
        run.dump()
    );
    assert_eq!(
        step(&run.stdout, "three"),
        [
            (40, Some([10.0, 10.0, 485.0, 780.0])),
            (41, Some([505.0, 10.0, 485.0, 385.0])),
            (42, Some([505.0, 405.0, 485.0, 385.0])),
        ],
        "{}",
        run.dump()
    );
    assert_eq!(
        step(&run.stdout, "closed"),
        [
            (40, Some([10.0, 10.0, 485.0, 780.0])),
            (41, None),
            (42, Some([505.0, 10.0, 485.0, 780.0])),
        ],
        "{}",
        run.dump()
    );
    assert_eq!(
        step(&run.stdout, "promoted"),
        [
            (40, Some([505.0, 10.0, 485.0, 780.0])),
            (42, Some([10.0, 10.0, 485.0, 780.0])),
        ],
        "{}",
        run.dump()
    );
    assert!(run.stdout.contains("terminal_started=true"), "{}", run.dump());
    // No scene graph, no std: the one extra mapping of the process is the runtime, and the interpreter is its dependency.
    assert_eq!(num(&run.stdout, "new_nodes", "new_nodes"), 0, "{}", run.dump());
    assert_eq!(num(&run.stdout, "maps", "core_copies"), 1, "{}", run.dump());
    assert_eq!(num(&run.stdout, "maps", "std_mappings"), 0, "{}", run.dump());
    assert_eq!(num(&run.stdout, "core_stats", "panics"), 0, "{}", run.dump());
}

#[test]
fn example_7_a_python_binding_moves_a_window_when_its_chord_is_pressed() {
    let run = drive(
        &example("07-script-binding/shell.py"),
        &[
            "pump",
            "mark:before",
            "dump",
            "chord:<Super>Right",
            "mark:right",
            "dump",
            "chord:<Super>Down",
            "mark:down",
            "dump",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let before = run.window(&run.section("before"), 40);
    let right = run.window(&run.section("right"), 40);
    let down = run.window(&run.section("down"), 40);
    assert_eq!((right.0, right.1), (before.0 + 100.0, before.1), "{}", run.dump());
    assert_eq!((down.0, down.1), (right.0, right.1 + 100.0), "{}", run.dump());
    assert_eq!(
        (down.2, down.3),
        (before.2, before.3),
        "a nudge keeps the size\n{}",
        run.dump()
    );
    let tree = run.claims(&run.section("before"));
    assert!(
        tree.contains("<Super>Right") && tree.contains("<Super>Down"),
        "{}",
        run.dump()
    );
}

// ---- hot reload: example 11 (Python) ---------------------------------------------------------------------------------------

/// The shell of the reload project. `tag` says which edit it is, and the marker file records which listener ran for which window, so
/// a listener that outlives its generation shows as a line of the old tag for a window the old shell never saw.
fn reload_shell(tag: &str, marker: &Path) -> String {
    format!(
        r#"from gnoblin import gnoblin

import place


def record(window):
    gnoblin.spawn(["sh", "-c", "echo {tag}:%d >> {marker}" % window.node_id])


@gnoblin.on_client_created
def added(window):
    record(window)
    place.place()


for window in gnoblin.windows:
    record(window)
place.place()

gnoblin.bind("Super+Return", lambda: None)
"#,
        marker = marker.display()
    )
}

/// The module the shell imports: columns with no gap, or (the edit) rows 24 apart.
fn place_module(edited: bool) -> &'static str {
    if edited {
        r#"from gnoblin import Screen, gnoblin


def place():
    screen = next(node for node in gnoblin.root.children() if isinstance(node, Screen))
    windows = gnoblin.windows
    gap = 24
    height = (screen.screen_height - gap * (len(windows) - 1)) / len(windows)
    for at, window in enumerate(windows):
        window.x = screen.screen_x
        window.y = screen.screen_y + at * (height + gap)
        window.width = screen.screen_width
        window.height = height
"#
    } else {
        r#"from gnoblin import Screen, gnoblin


def place():
    screen = next(node for node in gnoblin.root.children() if isinstance(node, Screen))
    windows = gnoblin.windows
    width = screen.screen_width / len(windows)
    for at, window in enumerate(windows):
        window.x = screen.screen_x + at * width
        window.y = screen.screen_y
        window.width = width
        window.height = screen.screen_height
"#
    }
}

#[test]
fn example_11_editing_the_shell_and_a_module_it_imports_reloads_it_in_place_with_no_listener_twice() {
    let dir = project("reload-edit");
    let marker = dir.join("marker");
    let edits = dir.join("edits");
    fs::create_dir_all(&edits).unwrap();
    fs::write(dir.join("shell.py"), reload_shell("first", &marker)).unwrap();
    fs::write(dir.join("place.py"), place_module(false)).unwrap();
    fs::write(edits.join("shell.py"), reload_shell("second", &marker)).unwrap();
    // The edit keeps the module's size: a stale `.pyc`, which Python keys on the source's size and its time in seconds, would run the
    // old code.
    let edited = place_module(true);
    assert_ne!(edited, place_module(false));
    fs::write(edits.join("place.py"), edited).unwrap();
    let put = |name: &str| format!("put:{}={}", dir.join(name).display(), edits.join(name).display());
    let run = drive(
        &dir.join("shell.py"),
        &[
            "pump",
            "open:50",
            "mark:one",
            "dump",
            "stats",
            // Both files change, as when one save writes the module and another the entry.
            &put("place.py"),
            &put("shell.py"),
            "sleep:500",
            // The runtime sees the change and asks the host; the host serves the request as it serves every reload.
            "pump",
            "serve",
            "mark:two",
            "dump",
            "open:51",
            "mark:three",
            "dump",
            "stats",
            // The programs the listeners started run on their own clock: give them time before the shell ends.
            "sleep:300",
            "pump",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    // Before: columns with no gap, 500 wide.
    let one = run.section("one");
    assert_eq!(run.window(&one, 40), (0.0, 0.0, 500.0, 800.0), "{}", run.dump());
    assert_eq!(run.window(&one, 50), (500.0, 0.0, 500.0, 800.0), "{}", run.dump());
    // The reload committed as generation 2 in place of 1, through the loader, and kept nothing mapped.
    let reloaded = run.lines_with("reload ok");
    assert_eq!(reloaded.len(), 1, "{}", run.dump());
    assert!(reloaded[0].contains("generation=2 previous=1"), "{}", run.dump());
    assert!(reloaded[0].contains("retained=0"), "{}", run.dump());
    // After: the imported module's edit is in force (rows 24 apart), so the module graph was reloaded and not served from a stale
    // cache; the windows are the same windows, never destroyed.
    let two = run.section("two");
    assert_eq!(run.window(&two, 40), (0.0, 0.0, 1000.0, 388.0), "{}", run.dump());
    assert_eq!(run.window(&two, 50), (0.0, 412.0, 1000.0, 388.0), "{}", run.dump());
    assert!(run.stdout.contains("first_window_destroys=0"), "{}", run.dump());
    // A window that opens after the reload is placed by the new code (three rows: (800 - 48) / 3).
    let three = run.section("three");
    let third = run.window(&three, 51);
    assert!((third.3 - 752.0 / 3.0).abs() < 1e-6 && third.0 == 0.0, "{}", run.dump());
    // No listener twice: the old generation's listener ran for the window it was told of (50) and never again; the new one ran for 51
    // alone. Each shell recorded the windows already open when it started: 40 and 50 for the second.
    let lines = marker_lines(&marker, 5);
    for (wanted, times) in [
        ("first:40", 1),
        ("first:50", 1),
        ("first:51", 0),
        ("second:40", 1),
        ("second:50", 1),
        ("second:51", 1),
    ] {
        assert_eq!(count(&lines, wanted), times, "{wanted} in {lines:?}\n{}", run.dump());
    }
    // No claim twice, and the reload mapped nothing: one runtime library before and after.
    assert_eq!(run.claims(&three).matches("Super+Return").count(), 1, "{}", run.dump());
    let maps: Vec<&str> = run.lines_with("maps core_copies");
    assert_eq!(maps.len(), 2, "{}", run.dump());
    assert!(
        maps.iter().all(|line| line.contains("core_copies=1 shell_mappings=1")),
        "{}",
        run.dump()
    );
}

#[test]
fn a_broken_edit_is_reported_with_its_traceback_keeps_the_running_shell_and_a_fix_loads() {
    let dir = project("reload-broken");
    let marker = dir.join("marker");
    let edits = dir.join("edits");
    fs::create_dir_all(&edits).unwrap();
    fs::write(dir.join("shell.py"), reload_shell("first", &marker)).unwrap();
    fs::write(dir.join("place.py"), place_module(false)).unwrap();
    // A syntax error in the shell, then (after the fix) a module that raises when it is imported.
    let broken = format!("{}\nbroken = (\n", reload_shell("broken", &marker));
    let broken_line = reload_shell("broken", &marker).lines().count() + 2;
    fs::write(edits.join("broken.py"), &broken).unwrap();
    fs::write(edits.join("raising_shell.py"), reload_shell("raising", &marker)).unwrap();
    let raising = format!(
        "{}\nraise RuntimeError(\"place.py is not ready\")\n",
        place_module(false)
    );
    let raising_line = place_module(false).lines().count() + 2;
    fs::write(edits.join("raising_place.py"), &raising).unwrap();
    fs::write(edits.join("fixed.py"), reload_shell("fixed", &marker)).unwrap();
    fs::write(edits.join("fixed_place.py"), place_module(false)).unwrap();
    let put = |to: &str, from: &str| format!("put:{}={}", dir.join(to).display(), edits.join(from).display());
    let run = drive(
        &dir.join("shell.py"),
        &[
            "pump",
            &put("shell.py", "broken.py"),
            "sleep:500",
            "pump",
            "serve",
            // The running generation still serves: a window that opens is placed by it.
            "open:50",
            "mark:after-syntax",
            "dump",
            // The shell is fixed but the module it imports now raises when it is imported.
            &put("shell.py", "raising_shell.py"),
            &put("place.py", "raising_place.py"),
            "sleep:500",
            "pump",
            "serve",
            "open:51",
            "mark:after-import",
            "dump",
            // Both are fixed: the module as it was, the shell as the fix.
            &put("place.py", "fixed_place.py"),
            &put("shell.py", "fixed.py"),
            "sleep:500",
            "pump",
            "serve",
            "open:52",
            "sleep:300",
            "pump",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let failed = run.lines_with("reload failed");
    assert_eq!(failed.len(), 2, "{}", run.dump());
    assert!(failed[0].contains("still running generation 1"), "{}", run.dump());
    assert!(
        failed[0].contains(&format!("shell.py:{broken_line}:")) && failed[0].contains("SyntaxError"),
        "{}\n(line {broken_line})",
        run.dump()
    );
    assert!(failed[1].contains("still running generation 1"), "{}", run.dump());
    assert!(
        failed[1].contains(&format!("place.py:{raising_line}:")) && failed[1].contains("RuntimeError"),
        "{}\n(line {raising_line})",
        run.dump()
    );
    // The full traceback is in the log, with the file and the line of each frame.
    assert!(
        run.stderr.contains("Traceback (most recent call last)")
            && run.stderr.contains(&format!("line {raising_line}, in <module>"))
            && run.stderr.contains("RuntimeError: place.py is not ready"),
        "{}",
        run.dump()
    );
    // The generation that served went on serving, with its own modules, through both failures.
    for (name, id) in [("after-syntax", 50), ("after-import", 51)] {
        let section = run.section(name);
        let window = run.window(&section, id);
        assert_eq!(
            window.1,
            0.0,
            "window {id} was placed as a column by generation 1\n{}",
            run.dump()
        );
        assert_eq!(window.3, 800.0, "{}", run.dump());
    }
    let lines = marker_lines(&marker, 6);
    assert_eq!(count(&lines, "first:50"), 1, "{lines:?}\n{}", run.dump());
    assert_eq!(count(&lines, "first:51"), 1, "{lines:?}\n{}", run.dump());
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("broken:") && !line.starts_with("raising:")),
        "a failed generation ran nothing: {lines:?}"
    );
    // The fix loads as the next generation: the failed ones used numbers 2 and 3.
    assert!(
        run.stdout.contains("reload ok generation=4 previous=1"),
        "{}",
        run.dump()
    );
    assert_eq!(count(&lines, "fixed:52"), 1, "{lines:?}\n{}", run.dump());
    assert_eq!(count(&lines, "first:52"), 0, "{lines:?}\n{}", run.dump());
}

// ---- callbacks, services ------------------------------------------------------------------------------------------------------

#[test]
fn an_exception_in_a_callback_is_logged_with_its_traceback_and_the_next_event_is_delivered() {
    let dir = project("callback-raises");
    let marker = dir.join("marker");
    let source = format!(
        r#"from gnoblin import gnoblin


@gnoblin.on_client_created
def fails(window):
    raise ValueError("no taste for window %d" % window.node_id)


@gnoblin.on_client_created
def hears(window):
    gnoblin.spawn(["sh", "-c", "echo heard:%d >> {marker}" % window.node_id])
"#,
        marker = marker.display()
    );
    fs::write(dir.join("shell.py"), &source).unwrap();
    let line = source
        .lines()
        .position(|line| line.contains("raise ValueError"))
        .unwrap()
        + 1;
    let run = drive(
        &dir.join("shell.py"),
        &["pump", "open:50", "open:51", "stats", "sleep:300", "pump", "stop"],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(
        run.stderr.contains("Traceback (most recent call last)")
            && run.stderr.contains(&format!("shell.py\", line {line}, in fails"))
            && run.stderr.contains("ValueError: no taste for window 50")
            && run.stderr.contains("ValueError: no taste for window 51"),
        "{}",
        run.dump()
    );
    // The failing listener did not keep the next one, or the next event, from running, and the core counted the failures.
    let lines = marker_lines(&marker, 2);
    assert_eq!(count(&lines, "heard:50"), 1, "{lines:?}");
    assert_eq!(count(&lines, "heard:51"), 1, "{lines:?}");
    assert!(num(&run.stdout, "core", "panics") >= 2, "{}", run.dump());
}

#[test]
fn the_tree_timers_and_state_are_reached_by_name_from_a_python_shell() {
    let dir = project("services");
    let marker = dir.join("marker");
    let source = format!(
        r#"from gnoblin import MemberError, Screen, Window, gnoblin


def note(text):
    gnoblin.spawn(["sh", "-c", "echo '%s' >> {marker}" % text])


# State kept across a reload: the next generation reads what this one wrote.
kept = gnoblin.kept_state("runs")
runs = int(kept[0]) + 1 if kept else 1
gnoblin.keep_state("runs", str(runs).encode())
note("runs=%d reloads=%d" % (runs, gnoblin.info["reload_count"]))

# Typed nodes from the schema, by name.
window = gnoblin.windows[0]
screen = next(node for node in gnoblin.root.children() if isinstance(node, Screen))
assert isinstance(window, Window) and window.class_name() == "window"
note("window title=%s app_id=%s focused=%s" % (window.title, window.app_id, window.focused))
note("screen %sx%s" % (screen.screen_width, screen.screen_height))
window.width = 640  # an int into a double property
window.maximised = True
try:
    window.title = "no"
except AttributeError as error:
    note("readonly: %s" % error)
try:
    window.get("no_such_property")
except MemberError:
    note("member error")


@gnoblin.on_title_changed
def titled(window, title):
    note("title %d=%s" % (window.node_id, title))


gnoblin.after(1000, lambda: note("after"))
gnoblin.every(400, lambda: note("every"))
"#,
        marker = marker.display()
    );
    fs::write(dir.join("shell.py"), &source).unwrap();
    let run = drive(
        &dir.join("shell.py"),
        &[
            "pump",
            "advance:500",
            "advance:500",
            "advance:500",
            // The notes are `sh` processes the shell spawned: the clock is the test's own, so they need real time to write their
            // lines before the reload and the stop end the generation that started them.
            "sleep:300",
            "mark:timers",
            "dump",
            "reload",
            "pump",
            "sleep:300",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let lines = marker_lines(&marker, 12);
    let has = |wanted: &str| lines.iter().any(|line| line == wanted);
    assert!(has("runs=1 reloads=0"), "{lines:?}\n{}", run.dump());
    assert!(
        has("window title=Terminal app_id=foot focused=False"),
        "{lines:?}\n{}",
        run.dump()
    );
    assert!(has("screen 1000.0x800.0"), "{lines:?}\n{}", run.dump());
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("readonly: ") && line.contains("read-only")),
        "{lines:?}"
    );
    assert!(has("member error"), "{lines:?}\n{}", run.dump());
    assert_eq!(count(&lines, "after"), 1, "{lines:?}\n{}", run.dump());
    assert!(count(&lines, "every") >= 3, "{lines:?}\n{}", run.dump());
    // The writes reached the compositor: the integer became a double, the boolean a boolean.
    let section = run.section("timers");
    assert_eq!(run.window(&section, 40).2, 640.0, "{}", run.dump());
    // The reload ran the script again with what the first run kept.
    assert!(has("runs=2 reloads=1"), "{lines:?}\n{}", run.dump());
}

// ---- example 17 (Python): a shell that needs only the core --------------------------------------------------------------------------

#[test]
fn example_17_a_python_shell_without_gnoblin_std_runs_and_importing_std_says_why() {
    let dir = project("no-std");
    let marker = dir.join("marker");
    fs::write(
        dir.join("shell.py"),
        format!(
            r#"from gnoblin import gnoblin

try:
    from gnoblin.std import Workspace
except ImportError as error:
    gnoblin.spawn(["sh", "-c", "echo '%s' >> {marker}" % error])


@gnoblin.on_client_created
def place(window):
    window.width = 321
"#,
            marker = marker.display()
        ),
    )
    .unwrap();
    let run = drive(
        &dir.join("shell.py"),
        &["pump", "open:50", "mark:open", "dump", "sleep:300", "pump", "stop"],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    // The core-only part of the shell ran.
    assert_eq!(run.window(&run.section("open"), 50).2, 321.0, "{}", run.dump());
    // Importing std fails with the text every runtime gives when `libgnoblin-std.so.1` is not installed, and says where it looked.
    let lines = marker_lines(&marker, 1);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("gnoblin-std is not installed (looked for libgnoblin-std.so.1 in ")),
        "{lines:?}\n{}",
        run.dump()
    );
    // No std was mapped and none made a provider.
    assert!(!run.stdout.contains("std_mappings=1"), "{}", run.dump());
}

// ---- the package and its stubs ------------------------------------------------------------------------------------------------------

/// A shell that logs, for every public name of the hand-written package and of `_gnoblin_core`, what the interpreter reflects: the members
/// of a class (an instance's, for `Gnoblin`, whose events are attributes of the object), or that a name is a function or a value.
const REFLECT_PACKAGE: &str = r#"
import importlib
import inspect

import _gnoblin_core
from gnoblin import gnoblin

skip = set(dir(object)) | set(dir(Exception))


def describe(label, value, instance=None):
    if inspect.isclass(value):
        names = sorted(n for n in dir(instance if instance is not None else value) if not n.startswith("_") and n not in skip)
        gnoblin.log("reflect " + label + " " + " ".join(names))
    elif callable(value):
        gnoblin.log("reflect " + label + " <function>")
    else:
        gnoblin.log("reflect " + label + " <variable>")


for module_name in ("gnoblin._base", "gnoblin.api", "gnoblin.errors", "gnoblin.events", "_gnoblin_core"):
    module = importlib.import_module(module_name)
    for name, value in sorted(vars(module).items()):
        if name.startswith("_"):
            continue
        describe(module_name + "." + name, value, gnoblin if name == "Gnoblin" else None)

package = importlib.import_module("gnoblin")
generated = set(importlib.import_module("gnoblin._generated.classes").__all__)
gnoblin.log("reflect gnoblin " + " ".join(sorted(n for n in package.__all__ if n not in generated)))
gnoblin.log("reflect done")
"#;

/// What the stubs declare, in the lines [`REFLECT_PACKAGE`] logs: `NAME.stub LABEL` pairs on the command line.
const STUB_PACKAGE: &str = r#"
import ast
import sys

for path, label in zip(sys.argv[1::2], sys.argv[2::2]):
    tree = ast.parse(open(path, encoding="utf-8").read())
    if label == "gnoblin":
        names = sorted(a.asname for n in tree.body if isinstance(n, ast.ImportFrom) for a in n.names if a.name != "*" and a.asname)
        print("reflect gnoblin " + " ".join(names))
        continue
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and not node.name.startswith("_"):
            names = {n.name for n in node.body if isinstance(n, ast.FunctionDef) and not n.name.startswith("_")}
            names |= {n.target.id for n in node.body if isinstance(n, ast.AnnAssign) and not n.target.id.startswith("_")}
            print(("reflect " + label + "." + node.name + " " + " ".join(sorted(names))).rstrip())
        elif isinstance(node, ast.FunctionDef) and not node.name.startswith("_"):
            print("reflect " + label + "." + node.name + " <function>")
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and not node.target.id.startswith("_"):
            print("reflect " + label + "." + node.target.id + " <variable>")
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and not target.id.startswith("_"):
                    print("reflect " + label + "." + target.id + " <variable>")
"#;

/// The `reflect` lines of a run's log, sorted.
fn reflected_lines(run: &Run) -> Vec<String> {
    let mut lines: Vec<String> = run
        .stderr
        .lines()
        .filter_map(|line| {
            line.find("reflect ").map(|at| {
                line[at..]
                    .split("  generation=")
                    .next()
                    .unwrap_or_default()
                    .trim_end()
                    .to_owned()
            })
        })
        .filter(|line| line != "reflect done")
        .collect();
    lines.sort();
    lines
}

#[test]
fn the_stubs_of_the_package_agree_with_what_the_runtime_reflects() {
    let dir = project("package-stub");
    let shell = dir.join("shell.py");
    fs::write(&shell, REFLECT_PACKAGE).unwrap();
    let run = start_in_std_prefix(&shell);
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(!run.stderr.contains("Traceback"), "{}", run.dump());
    assert!(run.stderr.contains("reflect done"), "{}", run.dump());
    let reflected = reflected_lines(&run);
    // The stubs, read with `ast`: nothing of them is imported.
    let script = dir.join("stub_package.py");
    fs::write(&script, STUB_PACKAGE).unwrap();
    let stubs = repo_root().join("crates/gnoblin-runtime-python/stubs");
    let mut command = std::process::Command::new("python3");
    command.arg(&script);
    for (file, label) in [
        ("gnoblin/__init__.pyi", "gnoblin"),
        ("gnoblin/_base.pyi", "gnoblin._base"),
        ("gnoblin/api.pyi", "gnoblin.api"),
        ("gnoblin/errors.pyi", "gnoblin.errors"),
        ("gnoblin/events.pyi", "gnoblin.events"),
        ("_gnoblin_core.pyi", "_gnoblin_core"),
    ] {
        command.arg(stubs.join(file)).arg(label);
    }
    let output = command.output().expect("python3 runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut declared: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    declared.sort();
    assert!(
        declared
            .iter()
            .any(|line| line.starts_with("reflect gnoblin.api.Gnoblin ")),
        "the stub has the classes: {declared:?}"
    );
    assert_eq!(
        reflected,
        declared,
        "the stubs and the runtime disagree\n{}",
        run.dump()
    );
    // The marker file the wheel type checkers look for is installed with them.
    assert!(stubs.join("gnoblin/py.typed").is_file());
}

/// What a shell that imports the whole package says of where it came from.
fn served_shell(data: &Path) -> String {
    format!(
        r#"
import sys

from gnoblin import gnoblin
from gnoblin import _base, api, errors, events, std, verbs, objects
import importlib

importlib.import_module("gnoblin.objects.std")
from gnoblin.objects import model
from gnoblin.std import Collection, Workspace, clients

for name in sorted(sys.modules):
    if name == "gnoblin" or name.startswith("gnoblin."):
        module = sys.modules[name]
        gnoblin.log("served " + name + " file=" + str(getattr(module, "__file__", None)) + " origin=" + str(module.__spec__.origin))
data = {data:?}
for name, module in sorted(sys.modules.items()):
    file = getattr(module, "__file__", None)
    if file and file.startswith(data):
        gnoblin.log("served-from-data " + name + " " + file)
for entry in sys.path:
    if entry.startswith(data) or entry.endswith("share/gnoblin/python"):
        gnoblin.log("served-path " + entry)
gnoblin.log("served done")
"#,
        data = data.display().to_string()
    )
}

#[test]
fn the_package_is_served_by_the_runtime_and_no_file_of_it_is_read() {
    let dir = project("package-embedded");
    let shell = dir.join("shell.py");
    fs::write(&shell, served_shell(&repo_root().join("data"))).unwrap();
    // A directory that holds a `gnoblin` package which raises when it is imported, named where the old runtime looked: nothing may read it.
    let poisoned = dir.join("poisoned");
    fs::create_dir_all(poisoned.join("gnoblin")).unwrap();
    fs::write(
        poisoned.join("gnoblin/__init__.py"),
        "raise RuntimeError(\"read from disk\")\n",
    )
    .unwrap();
    let stage = stage();
    let mut env = environment_in(&stage.with_std_runtimes, &[]);
    env.push(("GNOBLIN_PYTHON_DIR", poisoned));
    let extra: Vec<(&str, &Path)> = env.iter().map(|(key, value)| (*key, value.as_path())).collect();
    let name = PathBuf::from("reload");
    let run = finish(scenario_with_env(&stage.with_std, &[name.as_path(), &shell], &extra));
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(
        !run.stderr.contains("Traceback") && !run.stderr.contains("read from disk"),
        "{}",
        run.dump()
    );
    assert!(run.stderr.contains("served done"), "{}", run.dump());
    let served: Vec<&str> = run
        .stderr
        .lines()
        .filter(|line| line.contains("served gnoblin"))
        .collect();
    for module in [
        "gnoblin",
        "gnoblin._base",
        "gnoblin.api",
        "gnoblin.errors",
        "gnoblin.events",
        "gnoblin._std_base",
        "gnoblin._std_sugar",
        "gnoblin.std",
        "gnoblin._generated.classes",
        "gnoblin._generated.verbs",
        "gnoblin.objects.std",
    ] {
        let line = served
            .iter()
            .find(|line| line.contains(&format!("served {module} ")))
            .unwrap_or_else(|| panic!("{module} was not imported\n{}", run.dump()));
        assert!(line.contains("file=None"), "{module} has a file: {line}");
        assert!(
            line.contains("origin=<native:") || line.contains("origin=<generated:"),
            "{module} was not served by the runtime: {line}"
        );
    }
    assert!(!run.stderr.contains("served-from-data"), "{}", run.dump());
    assert!(!run.stderr.contains("served-path"), "{}", run.dump());
}

// ---- the golden shell -------------------------------------------------------------------------------------------------------------

fn golden_shell() -> PathBuf {
    repo_root().join("crates/gnoblin-runtime-python/tests/golden/shell.py")
}

#[test]
fn the_golden_shell_runs_over_the_embedded_package_and_every_registration_works() {
    let run = drive(
        &golden_shell(),
        &["pump", "open:50", "mark:open", "dump", "sleep:300", "pump", "stop"],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(!run.stderr.contains("Traceback"), "{}", run.dump());
    let log = &run.stderr;
    for wanted in [
        "golden runs 1",
        "golden window 40 Terminal",
        "golden node gone",
        // `off` removed the one registration made for the function, and only that.
        "golden off 1",
        "golden info ['generation', 'isolate', 'module_path', 'reload_count', 'state_dir']",
        // The decorator registration placed the window that opened, as the window's own typed object.
        "golden placed 50 ",
        // The program the shell started ended and its exit hook heard the record.
        "golden exit 0 True",
    ] {
        assert!(log.contains(wanted), "no `{wanted}` in\n{}", run.dump());
    }
    let section = run.section("open");
    assert_eq!(run.window(&section, 50).0, 40.0, "{}", run.dump());
    assert_eq!(run.window(&section, 50).1, 40.0, "{}", run.dump());
}

/// A type checker, when this machine has one: `$GNOBLIN_PYRIGHT`, or `pyright` on the path. A `$GNOBLIN_PYRIGHT` that names nothing is an
/// error, not a skip: whoever sets it wants the check to run.
fn pyright() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GNOBLIN_PYRIGHT").filter(|value| !value.is_empty()) {
        let explicit = PathBuf::from(explicit);
        assert!(
            explicit.is_file(),
            "GNOBLIN_PYRIGHT names {}, which is not a file",
            explicit.display()
        );
        return Some(explicit);
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join("pyright"))
        .find(|candidate| candidate.is_file())
}

/// The stub package `cargo xtask dist` installs under `share/gnoblin/types/python/`: the crate's own stubs, then the generated ones, where a
/// module that has a stub is the stub only.
fn installed_stubs(into: &Path) {
    let _ = fs::remove_dir_all(into);
    let mut copy = vec![repo_root().join("crates/gnoblin-runtime-python/stubs")];
    while let Some(from) = copy.pop() {
        for entry in fs::read_dir(&from).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                copy.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(repo_root().join("crates/gnoblin-runtime-python/stubs"))
                .unwrap();
            fs::create_dir_all(into.join(relative).parent().unwrap()).unwrap();
            fs::copy(&path, into.join(relative)).unwrap();
        }
    }
    let spec = gnoblin_sdkgen::SdkSpec::load(&repo_root()).expect("the catalog and the ABIs load");
    for target in ["python", "object-types"] {
        let rendered = gnoblin_sdkgen::render(&spec, Some(target)).expect("the target renders");
        let files = gnoblin_sdkgen::files_of(&rendered);
        let stubbed: Vec<String> = files
            .iter()
            .filter_map(|(path, _)| path.strip_suffix(".pyi").map(str::to_owned))
            .collect();
        for (path, text) in files {
            if path
                .strip_suffix(".py")
                .is_some_and(|module| stubbed.iter().any(|stub| stub == module))
            {
                continue;
            }
            let Some(relative) = path.strip_prefix("python/") else {
                continue;
            };
            fs::create_dir_all(into.join(relative).parent().unwrap()).unwrap();
            fs::write(into.join(relative), text).unwrap();
        }
    }
}

/// Type-checks each shell directory `(name, directory)` with the installed stubs, each in its own environment so a module beside the
/// shell resolves as it does at run time. Every error fails the test, with what pyright said.
fn type_check(project: &Path, pyright: &Path, shells: &[(String, PathBuf)]) {
    let stubs = project.join("types/python");
    installed_stubs(&stubs);
    let mut environments = Vec::new();
    for (name, from) in shells {
        let to = project.join("shells").join(name);
        fs::create_dir_all(&to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
            }
        }
        environments.push(format!(
            r#"{{ "root": {:?}, "extraPaths": [{:?}] }}"#,
            format!("shells/{name}"),
            stubs.display().to_string()
        ));
    }
    fs::write(
        project.join("pyrightconfig.json"),
        format!(
            r#"{{ "include": ["shells"], "executionEnvironments": [{}], "pythonVersion": "3.11", "typeCheckingMode": "standard", "reportMissingModuleSource": "none" }}"#,
            environments.join(", ")
        ),
    )
    .unwrap();
    let output = std::process::Command::new(pyright)
        .arg("-p")
        .arg(project)
        .output()
        .expect("pyright runs");
    assert!(
        output.status.success(),
        "pyright found problems against the installed stubs:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // A run that checked nothing is not a pass.
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("0 errors"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn the_golden_shell_type_checks_clean_against_the_installed_stubs() {
    let Some(pyright) = pyright() else {
        eprintln!("no pyright (GNOBLIN_PYRIGHT, or pyright on the path): the check is skipped");
        return;
    };
    let golden = golden_shell().parent().unwrap().to_owned();
    type_check(&project("golden-pyright"), &pyright, &[("golden".to_owned(), golden)]);
}

/// The directories of `examples/` that hold a Python shell: every Python example, whatever its number.
fn python_examples() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir(repo_root().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|dir| dir.join("shell.py").is_file())
        .map(|dir| (dir.file_name().unwrap().to_string_lossy().into_owned(), dir))
        .collect();
    found.sort();
    found
}

#[test]
fn every_python_example_type_checks_clean_against_the_installed_stubs() {
    let examples = python_examples();
    assert!(examples.len() >= 9, "the Python examples are found: {examples:?}");
    let Some(pyright) = pyright() else {
        eprintln!("no pyright (GNOBLIN_PYRIGHT, or pyright on the path): the check is skipped");
        return;
    };
    type_check(&project("examples-pyright"), &pyright, &examples);
}

// ---- example 3: a Python shell on the generated std objects -----------------------------------------------------------------
//
// Example 3 is written on the generated object API (`gnoblin.objects.std`). `gnoblin.std`, the typed-table binding over `gnoblin._std_base`,
// still ships in this runtime, so each proof runs a second time on the shells of `tests/fixtures/typed_table`: example 3 as it was before
// it moved. They stay for as long as that binding does; each test of them says `typed_table` in its name.

/// A shell of `tests/fixtures/typed_table`: the typed-table binding's own proof.
fn typed_table(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/typed_table")
        .join(name)
}

/// The scenario of example 6 over the Python shell `script`: the same windows, the same chord, the same rectangles the native Rust
/// shell of example 6 produces (`gnoblin-std-impl/tests/examples.rs`).
fn std_scenario(script: &Path) -> Run {
    let stage = stage();
    scenario_in(&stage.with_std, &stage.with_std_runtimes, "example06", script)
}

#[test]
fn example_3_a_python_shell_arranges_the_clients_of_a_workspace_in_columns_as_they_open_and_close() {
    assert_columns(&std_scenario(&example("03-python-std/shell.py")));
}

#[test]
fn typed_table_a_python_shell_on_gnoblin_std_arranges_the_clients_of_a_workspace_in_columns_as_they_open_and_close() {
    assert_columns(&std_scenario(&typed_table("shell.py")));
}

fn assert_columns(run: &Run) {
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(run.stderr.contains("python std shell is running"), "{}", run.dump());

    // The workspace fills the 1000 by 800 screen less the outer gap of 20 on each side, and the columns are 10 apart. Each edge is
    // floored on its own: three columns of 313.33 start at 20, 343.33 and 666.67.
    assert_eq!(
        step(&run.stdout, "one"),
        [(40, Some([20.0, 20.0, 960.0, 760.0]))],
        "{}",
        run.dump()
    );
    assert_eq!(
        step(&run.stdout, "two"),
        [
            (40, Some([20.0, 20.0, 475.0, 760.0])),
            (41, Some([505.0, 20.0, 475.0, 760.0]))
        ],
        "{}",
        run.dump()
    );
    assert_eq!(
        step(&run.stdout, "three"),
        [
            (40, Some([20.0, 20.0, 313.0, 760.0])),
            (41, Some([343.0, 20.0, 313.0, 760.0])),
            (42, Some([666.0, 20.0, 314.0, 760.0])),
        ],
        "{}",
        run.dump()
    );
    // `Super+Tab` swapped the focused third window with the first: the layout was not told how, it read the new order.
    assert_eq!(
        step(&run.stdout, "swapped"),
        [
            (40, Some([666.0, 20.0, 314.0, 760.0])),
            (41, Some([343.0, 20.0, 313.0, 760.0])),
            (42, Some([20.0, 20.0, 313.0, 760.0])),
        ],
        "{}",
        run.dump()
    );
    assert_eq!(
        step(&run.stdout, "closed"),
        [
            (40, Some([505.0, 20.0, 475.0, 760.0])),
            (41, None),
            (42, Some([20.0, 20.0, 475.0, 760.0])),
        ],
        "{}",
        run.dump()
    );
    // The listener on the collection of clients heard each window that opened after it (window 40 was there already).
    for id in [41, 42] {
        assert!(
            run.stderr.contains(&format!("client {id} opened on main")),
            "no listener line for {id}: {}",
            run.dump()
        );
    }
    assert!(!run.stderr.contains("Traceback"), "{}", run.dump());
    // The process maps one core and one std library; the runtime is the extra mapping.
    assert_eq!(num(&run.stdout, "maps", "core_copies"), 1, "{}", run.dump());
    assert_eq!(num(&run.stdout, "maps", "std_mappings"), 1, "{}", run.dump());
    assert_eq!(num(&run.stdout, "core_stats", "panics"), 0, "{}", run.dump());
}

/// The native Rust shell of the layouts (`gnoblin-std-impl/fixtures/shell-layouts`) over the same scenario: the reference.
fn native_layouts() -> Run {
    let stage = stage();
    let shell = stage.with_std.join("shells/libshell_layouts.so");
    let name = PathBuf::from("std_layouts");
    finish(scenario_with_env(
        &stage.with_std,
        &[name.as_path(), shell.as_path()],
        &[("LD_LIBRARY_PATH", stage.with_std.as_path())],
    ))
}

/// Runs the layouts shell `layouts` and holds every step of it to the native shell's.
fn assert_layouts_match_the_native_shell(layouts: &Path) {
    let stage = stage();
    let native = native_layouts();
    assert_eq!(native.code, 0, "{}", native.dump());
    let script = scenario_in(&stage.with_std, &stage.with_std_runtimes, "std_layouts", layouts);
    assert_eq!(script.code, 0, "{}", script.dump());
    assert!(script.stderr.contains("layouts shell is running"), "{}", script.dump());
    for name in [
        "Super+1",
        "Super+2",
        "Super+3",
        "Super+4",
        "Super+5",
        "Super+6",
        "closed",
        "swapped",
        "dropped_nowhere",
    ] {
        let expected = step(&native.stdout, name);
        assert!(
            !expected.is_empty(),
            "the native run has no step {name}: {}",
            native.dump()
        );
        assert_eq!(step(&script.stdout, name), expected, "step {name}\n{}", script.dump());
    }
    assert!(!script.stderr.contains("Traceback"), "{}", script.dump());
    assert_eq!(num(&script.stdout, "core_stats", "panics"), 0, "{}", script.dump());
}

#[test]
fn the_built_in_layouts_from_python_give_the_rectangles_of_the_native_shell() {
    assert_layouts_match_the_native_shell(&example("03-python-std/layouts.py"));
}

#[test]
fn typed_table_the_built_in_layouts_from_gnoblin_std_give_the_rectangles_of_the_native_shell() {
    assert_layouts_match_the_native_shell(&typed_table("layouts.py"));
}

/// Starts the shell `script` in the prefix that has the std library beside the core, and ends it when it has evaluated: the scenario of a
/// reload with no step, which needs nothing of the compositor.
fn start_in_std_prefix(script: &Path) -> Run {
    let stage = stage();
    let env = environment_in(&stage.with_std_runtimes, &[]);
    let extra: Vec<(&str, &Path)> = env.iter().map(|(key, value)| (*key, value.as_path())).collect();
    let name = PathBuf::from("reload");
    finish(scenario_with_env(&stage.with_std, &[name.as_path(), script], &extra))
}

// ---- the generated object classes ------------------------------------------------------------------------------------------

/// A shell that logs, for every class of `gnoblin.objects.std`, the public names the interpreter reflects on it: a record's fields, an enum's
/// members, and otherwise the attributes a script can reach (a property, a method, a function). Run in the Python runtime, so the classes
/// are the ones the runtime carries, written from the catalog the std library serves.
const REFLECT_SHELL: &str = r#"
import dataclasses
import enum
import inspect

from gnoblin import gnoblin
from gnoblin.objects import model, std
from gnoblin.objects._base import Implementor

for module in (model, std):
    # `model` is the core's namespace, `std` the library's: each is written from its own catalog.
    prefix = module.__name__.rsplit(".", 1)[1]
    for name, value in sorted(vars(module).items()):
        if not inspect.isclass(value) or value.__module__ != module.__name__:
            continue
        if issubclass(value, enum.Enum):
            names = list(value.__members__)
        elif issubclass(value, Implementor):
            # An interface is a base the script subclasses: it has no body for the methods the stub declares.
            names = ["<interface>"]
        elif dataclasses.is_dataclass(value):
            names = [field.name for field in dataclasses.fields(value)]
        else:
            names = [member for member in dir(value) if not member.startswith("_")]
        gnoblin.log("reflect " + prefix + "." + name + " " + " ".join(sorted(names)))
gnoblin.log("reflect done")
"#;

/// What the stub declares of each class, as `reflect NAME names` lines: the same three shapes the shell above reflects.
const STUB_NAMES: &str = r#"
import ast
import sys

tree = ast.parse(open(sys.argv[1], encoding="utf-8").read())


def decorated_as(node, name):
    return any(
        (isinstance(d, ast.Attribute) and d.attr == name) or (isinstance(d, ast.Name) and d.id == name)
        or (isinstance(d, ast.Call) and ((isinstance(d.func, ast.Attribute) and d.func.attr == name) or (isinstance(d.func, ast.Name) and d.func.id == name)))
        for d in node.decorator_list
    )


for node in tree.body:
    if not isinstance(node, ast.ClassDef):
        continue
    if any(isinstance(base, ast.Attribute) and base.attr in ("IntEnum", "Enum") for base in node.bases):
        names = [t.id for n in node.body if isinstance(n, ast.Assign) for t in n.targets if isinstance(t, ast.Name)]
    elif (ast.get_docstring(node) or "").startswith("The interface"):
        names = ["<interface>"]
    elif decorated_as(node, "dataclass"):
        names = [n.target.id for n in node.body if isinstance(n, ast.AnnAssign)]
    else:
        names = {n.name for n in node.body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and not n.name.startswith("_")}
        names |= {n.target.id for n in node.body if isinstance(n, ast.AnnAssign) and not n.target.id.startswith("_")}
    print("reflect " + sys.argv[2] + "." + node.name + " " + " ".join(sorted(names)))
"#;

#[test]
fn the_stub_of_the_generated_classes_agrees_with_what_the_runtime_reflects() {
    // The stub, from the generator.
    let spec = gnoblin_sdkgen::SdkSpec::load(&repo_root()).expect("the catalog and the ABIs load");
    let rendered = gnoblin_sdkgen::render(&spec, Some("object-types")).expect("the declarations render");
    let files = gnoblin_sdkgen::files_of(&rendered);
    let dir = project("object-stub");
    let names = dir.join("stub_names.py");
    fs::write(&names, STUB_NAMES).unwrap();
    let mut declared: Vec<String> = Vec::new();
    for module in ["model", "std"] {
        let stub_text = files
            .iter()
            .find(|(path, _)| *path == format!("python/gnoblin/objects/{module}.pyi"))
            .map(|(_, text)| text.clone())
            .expect("the stub is rendered");
        let stub = dir.join(format!("{module}.pyi"));
        fs::write(&stub, stub_text).unwrap();
        let output = std::process::Command::new("python3")
            .arg(&names)
            .arg(&stub)
            .arg(module)
            .output()
            .expect("python3 runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        declared.extend(String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned));
    }

    // The runtime's reflection of the classes it carries.
    let shell = dir.join("shell.py");
    fs::write(&shell, REFLECT_SHELL).unwrap();
    let run = start_in_std_prefix(&shell);
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(!run.stderr.contains("Traceback"), "{}", run.dump());
    let mut reflected: Vec<String> = run
        .stderr
        .lines()
        .filter_map(|line| {
            line.find("reflect ").map(|at| {
                line[at..]
                    .split("  generation=")
                    .next()
                    .unwrap_or_default()
                    .trim_end()
                    .to_owned()
            })
        })
        .filter(|line| line != "reflect done")
        .collect();
    // The std library adds its augment members to the core's classes when `gnoblin.objects.std` is imported, which the reflection did. A
    // stub cannot add a member to a class of another file, so it declares them on the std classes that hold them (`GnoblinAugment`,
    // `OutputCollectionAugment`): the core's declared names are completed with those.
    let augments: Vec<(String, Vec<String>)> = declared
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("reflect std.")?;
            let (class, names) = rest.split_once(' ').unwrap_or((rest, ""));
            let members = names
                .split(' ')
                .filter(|name| !matches!(*name, "" | "get" | "on" | "destroy"))
                .map(str::to_owned)
                .collect();
            Some((class.strip_suffix("Augment")?.to_owned(), members))
        })
        .collect();
    assert!(
        !augments.is_empty(),
        "the std stub holds the augment members: {declared:?}"
    );
    for (target, members) in augments {
        let prefix = format!("reflect model.{target} ");
        for line in declared.iter_mut().filter(|line| line.starts_with(&prefix)) {
            let mut names: Vec<String> = line[prefix.len()..].split(' ').map(str::to_owned).collect();
            names.extend(members.iter().cloned());
            names.sort();
            *line = format!("{prefix}{}", names.join(" "));
        }
    }
    reflected.sort();
    declared.sort();
    assert!(run.stderr.contains("reflect done"), "{}", run.dump());
    assert!(
        declared.iter().any(|line| line.starts_with("reflect model.")),
        "the core's module has classes: {declared:?}"
    );
    assert!(
        declared.iter().any(|line| line.starts_with("reflect std.EntryModel ")),
        "the stub has the classes: {declared:?}"
    );
    assert_eq!(reflected, declared, "the stub and the runtime disagree\n{}", run.dump());
}

/// What a call from a Python script costs, in nanoseconds, for the calls the object protocol times from Rust. A measurement
/// and not a check, so it is ignored: run it with the fixtures built in release (`cargo test -p gnoblin-runtime-python --release --test
/// shells what_a_call_from_python_costs -- --ignored --nocapture`).
#[test]
#[ignore = "a measurement: run it in a release build with --ignored --nocapture"]
fn what_a_call_from_python_costs() {
    let dir = project("object-cost");
    let shell = dir.join("shell.py");
    fs::write(
        &shell,
        r#"
import time

from gnoblin import gnoblin
from gnoblin.objects.std import EntryModel, FocusEvent, FocusEventKind, FocusMode, FocusPolicy, FocusSettings

ROUNDS = 200000
entry = EntryModel("hello, world, and more text")
settings = FocusSettings(mode=FocusMode.SLOPPY)
event = FocusEvent(kind=FocusEventKind.ENTER, held=False)
plain = FocusSettings()


def cost(label, body):
    start = time.perf_counter_ns()
    body()
    gnoblin.log(f"cost {label}: {(time.perf_counter_ns() - start) / ROUNDS}")


def empty():
    for _ in range(ROUNDS):
        pass


def get_number():
    for _ in range(ROUNDS):
        entry.length


def get_text():
    for _ in range(ROUNDS):
        entry.text


def get_record():
    for _ in range(ROUNDS):
        entry.selection


def set_text():
    for _ in range(ROUNDS):
        entry.text = "hello, world, and more text"


def method():
    for _ in range(ROUNDS):
        entry.home()


def two_changes():
    for _ in range(ROUNDS):
        entry.insert("a")
        entry.backspace()


def static_boolean():
    for _ in range(ROUNDS):
        FocusPolicy.opens_focused(plain, True)


def static_record():
    for _ in range(ROUNDS):
        FocusPolicy.focus_decision(settings, event)


for label, body in [
    ("the script's own loop", empty),
    ("get of a number", get_number),
    ("get of text", get_text),
    ("get of a record (absent)", get_record),
    ("set of text", set_text),
    ("a method with no arguments that changes nothing", method),
    ("two calls that each change the entry (insert and backspace)", two_changes),
    ("a static function with a record and a boolean", static_boolean),
    ("a static function with two records, answering a record", static_record),
]:
    cost(label, body)
gnoblin.log("cost done")
"#,
    )
    .unwrap();
    let run = start_in_std_prefix(&shell);
    assert_eq!(run.code, 0, "{}", run.dump());
    for line in run.stderr.lines().filter(|line| line.contains("cost ")) {
        println!("{}", &line[line.find("cost ").unwrap_or(0)..]);
    }
    assert!(run.stderr.contains("cost done"), "{}", run.dump());
}
