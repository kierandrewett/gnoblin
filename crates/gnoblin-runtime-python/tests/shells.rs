//! The Python runtime library, loaded by the core's loader into a process that links the installed core, serving shells over the fake
//! compositor (`design/modular-framework.md` sections 8.2, 10.1 and 12; M6.5).
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
    let bin = base.join("bin");
    fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    let foot = bin.join("foot");
    fs::write(&foot, "#!/bin/sh\n: > \"$GNOBLIN_TEST_MARKER\"\n").map_err(|error| error.to_string())?;
    fs::set_permissions(&foot, fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;
    Ok(Stage {
        dir,
        runtimes,
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
    let stage = stage();
    let mut env = vec![
        ("GNOBLIN_RUNTIME_DIR", stage.runtimes.clone()),
        ("GNOBLIN_PYTHON_DIR", repo_root().join("data/python")),
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
    let marker = stage().dir.join(format!("marker-{name}"));
    let _ = fs::remove_file(&marker);
    let env = environment(&[("GNOBLIN_TEST_MARKER", marker)]);
    let extra: Vec<(&str, &Path)> = env.iter().map(|(key, value)| (*key, value.as_path())).collect();
    let name_path = PathBuf::from(name);
    finish(scenario_with_env(&stage().dir, &[name_path.as_path(), script], &extra))
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
    let run = scenario_of("example08", &example("08-no-workspaces-py/shell.py"));
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
    assert!(run.stdout.contains("order=[42, 20, 21, 40]"), "{}", run.dump());
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
            "chord:Super+Right",
            "mark:right",
            "dump",
            "chord:Super+Down",
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
        tree.contains("Super+Right") && tree.contains("Super+Down"),
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
            "mark:timers",
            "dump",
            "reload",
            "pump",
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

#[test]
fn the_stub_file_declares_every_public_name_of_the_package() {
    let script = repo_root().join("crates/gnoblin-runtime-python/tests/check_stubs.py");
    let output = std::process::Command::new("python3")
        .arg(&script)
        .arg(repo_root().join("data/python"))
        .output()
        .expect("python3 runs");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
