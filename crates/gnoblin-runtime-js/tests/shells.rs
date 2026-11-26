//! The JavaScript runtime library, loaded by the core's loader into a process that links the installed core, serving shells over the
//! fake compositor.
//!
//! Nothing here is the engine's own test harness: the process is the core's test launcher, the shell is a file the loader runs
//! by its extension through `libgnoblin-runtime-js.so`, and what the tests read is the compositor's tree and the log. Examples 4,
//! 5 and 7 are the files of `examples/`, run as they are; examples 11 and 17 are built from them.
//!
//! What this does not prove, because the fake compositor has no Wayland behind it: that a client's connection survives a reload
//! (the property that makes it true is proven: the windows are neither destroyed nor made again), and the session suite of
//! `crates/gnoblin/tests`, which needs `gnoblin ./shell.ts`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[path = "../../gnoblin-core-impl/tests/support/mod.rs"]
mod support;
use support::*;

struct Stage {
    dir: PathBuf,
    runtimes: PathBuf,
    /// The same prefix with `libgnoblin-std.so.1` beside the core: the one a std shell runs in.
    with_std: PathBuf,
    with_std_runtimes: PathBuf,
    cache: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("runtime-js-stage");
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
            "gnoblin-runtime-js",
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
    copy_to(
        &debug.join("libgnoblin_runtime_js.so"),
        &runtimes.join("libgnoblin-runtime-js.so"),
    );
    // The same files with the std library beside the core, where `gnoblin-std/generated/objects` finds it.
    let with_std = base.join("core10-std");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &with_std.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &with_std.join(LAUNCHER));
    let with_std_runtimes = with_std.join("runtimes");
    copy_to(
        &debug.join("libgnoblin_runtime_js.so"),
        &with_std_runtimes.join("libgnoblin-runtime-js.so"),
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
    Ok(Stage {
        cache: base.join("cache"),
        dir,
        runtimes,
        with_std,
        with_std_runtimes,
    })
}

fn stage() -> &'static Stage {
    static STAGE: OnceLock<Result<Stage, String>> = OnceLock::new();
    match STAGE.get_or_init(build_stage) {
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

fn drive(script: &Path, steps: &[&str]) -> Run {
    drive_in(script, steps, &repo_root().join("data/js"))
}

/// Like [`drive`], with the stock library read from `stock`: an installed prefix's `share/gnoblin/js`, which a test may make hold
/// less than the source tree's.
fn drive_in(script: &Path, steps: &[&str], stock: &Path) -> Run {
    let stage = stage();
    drive_over(&stage.dir, &stage.runtimes, script, steps, stock)
}

/// Like [`drive`], in a shell that uses the std library: the prefix with the library beside the core.
fn drive_std(script: &Path, steps: &[&str]) -> Run {
    let stage = stage();
    drive_over(
        &stage.with_std,
        &stage.with_std_runtimes,
        script,
        steps,
        &repo_root().join("data/js"),
    )
}

fn drive_over(dir: &Path, runtimes: &Path, script: &Path, steps: &[&str], stock: &Path) -> Run {
    let stage = stage();
    let mut all: Vec<PathBuf> = vec![PathBuf::from("reload"), script.to_owned()];
    all.extend(steps.iter().map(PathBuf::from));
    let refs: Vec<&Path> = all.iter().map(PathBuf::as_path).collect();
    let output = scenario_with_env(
        dir,
        &refs,
        &[
            ("GNOBLIN_RUNTIME_DIR", runtimes),
            ("GNOBLIN_JS_STOCK_DIR", stock),
            ("XDG_CACHE_HOME", stage.cache.as_path()),
            ("GNOBLIN_JS_CONSOLE", Path::new("0")),
        ],
    );
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

/// Runs a named scenario of the launcher over `script` in the prefix `dir`, whose runtime library is in `runtimes`.
fn scenario_in(dir: &Path, runtimes: &Path, name: &str, script: &Path) -> Run {
    let stage = stage();
    let name_path = PathBuf::from(name);
    let output = scenario_with_env(
        dir,
        &[name_path.as_path(), script],
        &[
            ("GNOBLIN_RUNTIME_DIR", runtimes),
            ("GNOBLIN_JS_STOCK_DIR", repo_root().join("data/js").as_path()),
            ("XDG_CACHE_HOME", stage.cache.as_path()),
            ("GNOBLIN_JS_CONSOLE", Path::new("0")),
        ],
    );
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

/// The rectangles printed under `step=<name>`: the window id and its rectangle (`None` for a window that is gone).
fn step(stdout: &str, name: &str) -> Vec<(u64, Option<[f64; 4]>)> {
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

// ---- the runtime serves a shell -----------------------------------------------------------------------------------------------

#[test]
fn example_4_a_js_shell_of_es_modules_runs_in_process_and_its_std_workspace_tiles_the_windows() {
    let run = drive_std(
        &example("04-js-esm/shell.js"),
        &[
            "pump", "mark:one", "dump", "open:50", "mark:two", "dump", "stats", "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(run.stdout.contains("started generation=1"), "{}", run.dump());
    // The context is a JavaScript one, and the engine is in the runtime library: the one extra mapping is the runtime. The shell
    // imports `gnoblin-std`, whose stock layouts are the library's, so the library is mapped too.
    assert!(
        run.stdout
            .contains("maps core_copies=1 shell_mappings=1 std_mappings=1"),
        "{}",
        run.dump()
    );
    // The relative import ran: both launcher keys are claimed by the module `keys.js`.
    let one = run.section("one");
    let tree = run.claims(&one);
    assert!(
        tree.contains("<Super>Return") && tree.contains("<Super>d"),
        "{}",
        run.dump()
    );
    // The std master-stack layout of the workspace arranged the one window, and then the two.
    let (x, y, w, h) = run.window(&one, 40);
    assert_eq!((x, y, w, h), (0.0, 0.0, 1000.0, 800.0), "{}", run.dump());
    let two = run.section("two");
    let master = run.window(&two, 40);
    let stacked = run.window(&two, 50);
    assert_eq!(master.0, 0.0, "{}", run.dump());
    assert!(
        master.2 < 1000.0 && stacked.0 >= master.2 - 1e-6,
        "master and stack side by side\n{}",
        run.dump()
    );
    assert_eq!(master.3, 800.0, "{}", run.dump());
    assert_eq!(stacked.3, 800.0, "{}", run.dump());
}

#[test]
fn example_5_a_ts_shell_of_three_files_erases_its_types_and_reports_an_error_at_the_ts_line() {
    let geometry = fs::read_to_string(example("05-ts-types/geometry.ts")).unwrap();
    let throw_line = geometry
        .lines()
        .position(|line| line.contains("throw new Error(reason)"))
        .map(|index| index + 1)
        .expect("the example throws in geometry.ts");
    let run = drive(
        &example("05-ts-types/shell.ts"),
        &[
            "pump",
            "open:50",
            "mark:columns",
            "dump",
            "chord:<Super>t",
            "mark:rows",
            "dump",
            "chord:<Super>x",
            "pump",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    // Two windows in two columns of the 1000 by 800 output, `Gap.Large` (24) apart: (1000 - 24) / 2 each.
    let columns = run.section("columns");
    let (first, second) = (run.window(&columns, 40), run.window(&columns, 50));
    assert_eq!(first, (0.0, 0.0, 488.0, 800.0), "{}", run.dump());
    assert_eq!(second, (512.0, 0.0, 488.0, 800.0), "{}", run.dump());
    // Super+t switches to rows: (800 - 24) / 2 each, the second below the first.
    let rows = run.section("rows");
    assert_eq!(run.window(&rows, 40), (0.0, 0.0, 1000.0, 388.0), "{}", run.dump());
    assert_eq!(run.window(&rows, 50), (0.0, 412.0, 1000.0, 388.0), "{}", run.dump());
    // Super+x throws in `geometry.ts`; the report names that file and the line of the `throw`, not a line of stripped JavaScript.
    assert!(
        run.stderr.contains(&format!("geometry.ts:{throw_line}:")),
        "the error is reported at geometry.ts:{throw_line}\n{}",
        run.dump()
    );
    assert!(run.stderr.contains("example 5 stops here"), "{}", run.dump());
}

#[test]
fn example_7_a_binding_class_written_in_ts_moves_a_window_when_its_chord_is_pressed() {
    let run = drive(
        &example("07-script-binding/shell.ts"),
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

// ---- hot reload: example 11 (TypeScript) ---------------------------------------------------------------------------------------

/// The shell of the reload project. `tag` says which edit it is, and the marker file counts how often a listener ran.
fn reload_shell(tag: &str, marker: &Path) -> String {
    format!(
        r#"import {{ ClientCollectionListener, gnoblin }} from "gnoblin-core";
import type {{ Client }} from "gnoblin-core";

import {{ place }} from "./place.ts";

// Every window the listener hears of adds a line to the marker file, so a listener that outlives its generation shows as a
// second line for one window.
class Tiler extends ClientCollectionListener {{
    onAdded(_client: Client): void {{
        gnoblin.spawn(["sh", "-c", "echo {tag} >> {marker}"]);
        place();
    }}
}}
gnoblin.clients.addListener(new Tiler());

gnoblin.input.primary.bind("Super+Return", () => {{}});
"#,
        marker = marker.display()
    )
}

/// The module the shell imports: columns with no gap, or (the edit) rows 24 apart.
fn place_module(edited: bool) -> &'static str {
    if edited {
        r#"import { gnoblin } from "gnoblin-core";

export function place(): void {
    const output = gnoblin.outputs.at(0);
    if (output === undefined) {
        return;
    }
    const windows = gnoblin.clients.toList();
    const gap = 24;
    const height = (output.height - gap * (windows.length - 1)) / windows.length;
    windows.forEach((client, at) => {
        client.bounds = { x: output.x, y: output.y + at * (height + gap), width: output.width, height };
    });
}
"#
    } else {
        r#"import { gnoblin } from "gnoblin-core";

export function place(): void {
    const output = gnoblin.outputs.at(0);
    if (output === undefined) {
        return;
    }
    const windows = gnoblin.clients.toList();
    const width = output.width / windows.length;
    windows.forEach((client, at) => {
        client.bounds = { x: output.x + at * width, y: output.y, width, height: output.height };
    });
}
"#
    }
}

fn lines_of(marker: &Path, tag: &str) -> usize {
    fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == tag)
        .count()
}

/// The shell's `sh` processes (each one writes a line of the marker) ended before `stop`. `stop` takes the programs a shell has
/// running down with it, so one that had not written yet by then would never write: the step `wait_programs` before `stop` makes
/// every line that will ever be written be in the marker when the test reads it.
fn assert_programs_ended(run: &Run) {
    assert!(
        run.stdout.contains("programs running=0"),
        "a program the shell started was still running at `stop`\n{}",
        run.dump()
    );
}

#[test]
fn example_11_editing_the_shell_and_a_module_it_imports_reloads_it_in_place_with_no_listener_twice() {
    let dir = project("reload-edit");
    let marker = dir.join("marker");
    let edits = dir.join("edits");
    fs::create_dir_all(&edits).unwrap();
    fs::write(dir.join("shell.ts"), reload_shell("first", &marker)).unwrap();
    fs::write(dir.join("place.ts"), place_module(false)).unwrap();
    fs::write(edits.join("shell.ts"), reload_shell("second", &marker)).unwrap();
    fs::write(edits.join("place.ts"), place_module(true)).unwrap();
    let put = |name: &str| format!("put:{}={}", dir.join(name).display(), edits.join(name).display());
    let run = drive(
        &dir.join("shell.ts"),
        &[
            "pump",
            "open:50",
            "mark:one",
            "dump",
            "stats",
            // Both files change, as when one save writes the module and another the entry.
            &put("place.ts"),
            &put("shell.ts"),
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
            "wait_programs",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    assert_programs_ended(&run);
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
    // No listener twice, and no claim twice: the old generation's listener ran for the two windows it saw (40 as the shell
    // started, 50 as it opened) and never again; the new one ran for the two it was told of on attach and for 51.
    assert_eq!(lines_of(&marker, "first"), 2, "{}", run.dump());
    assert_eq!(lines_of(&marker, "second"), 3, "{}", run.dump());
    assert_eq!(run.claims(&three).matches("<Super>Return").count(), 1, "{}", run.dump());
    // The reload mapped nothing: the one mapping beyond the core is the runtime, before and after.
    let maps: Vec<&str> = run.lines_with("maps core_copies");
    assert_eq!(maps.len(), 2, "{}", run.dump());
    assert!(
        maps.iter().all(|line| line.contains("core_copies=1 shell_mappings=1")),
        "{}",
        run.dump()
    );
}

#[test]
fn a_broken_edit_is_reported_at_its_ts_line_keeps_the_running_shell_and_a_fix_loads() {
    let dir = project("reload-broken");
    let marker = dir.join("marker");
    let edits = dir.join("edits");
    fs::create_dir_all(&edits).unwrap();
    fs::write(dir.join("shell.ts"), reload_shell("first", &marker)).unwrap();
    fs::write(dir.join("place.ts"), place_module(false)).unwrap();
    fs::write(
        edits.join("broken.ts"),
        format!("{}\nexport const broken: number = ;\n", reload_shell("broken", &marker)),
    )
    .unwrap();
    fs::write(edits.join("fixed.ts"), reload_shell("fixed", &marker)).unwrap();
    let broken_line = reload_shell("broken", &marker).lines().count() + 2;
    let run = drive(
        &dir.join("shell.ts"),
        &[
            "pump",
            &format!(
                "put:{}={}",
                dir.join("shell.ts").display(),
                edits.join("broken.ts").display()
            ),
            "sleep:500",
            "pump",
            "serve",
            // The running generation still serves: a window that opens is placed by it.
            "open:50",
            "mark:after-failure",
            "dump",
            &format!(
                "put:{}={}",
                dir.join("shell.ts").display(),
                edits.join("fixed.ts").display()
            ),
            "sleep:500",
            "pump",
            "serve",
            "open:51",
            "wait_programs",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    assert_programs_ended(&run);
    let failed = run.lines_with("reload failed");
    assert_eq!(failed.len(), 1, "{}", run.dump());
    assert!(failed[0].contains("still running generation 1"), "{}", run.dump());
    assert!(
        failed[0].contains(&format!("shell.ts:{broken_line}:")),
        "{}\n(line {broken_line})",
        run.dump()
    );
    let window = run.window(&run.section("after-failure"), 50);
    assert_eq!(
        window,
        (500.0, 0.0, 500.0, 800.0),
        "the running generation placed it\n{}",
        run.dump()
    );
    assert_eq!(lines_of(&marker, "first"), 2, "{}", run.dump());
    assert_eq!(lines_of(&marker, "broken"), 0, "{}", run.dump());
    // The fix loads as the next generation: the failed one used number 2.
    assert!(
        run.stdout.contains("reload ok generation=3 previous=1"),
        "{}",
        run.dump()
    );
    assert_eq!(lines_of(&marker, "fixed"), 3, "{}", run.dump());
}

// ---- hot reload: the programs the config's top level starts -------------------------------------------------------------------

/// A shell whose top level starts `sleep <program>` (when it names one) and then runs `rest`. The program's argument tells it from
/// any other, and `sleep` ends only when something stops it.
fn spawning_shell(program: Option<&str>, rest: &str) -> String {
    let spawn = program.map_or_else(String::new, |tag| format!("gnoblin.spawn([\"sleep\", \"{tag}\"]);\n"));
    format!("import {{ gnoblin }} from \"gnoblin-core\";\n{spawn}{rest}\n")
}

/// The pids of each `programs` step of `run`, in order.
fn programs_seen(run: &Run) -> Vec<Vec<u32>> {
    run.lines_with("programs running=")
        .iter()
        .map(|line| {
            line.split_once("pids=")
                .map_or("", |(_, pids)| pids)
                .split(',')
                .filter_map(|pid| pid.parse().ok())
                .collect()
        })
        .collect()
}

#[test]
fn a_reload_keeps_the_program_the_config_starts_again_stops_one_it_no_longer_starts_and_a_refused_stage_starts_nothing()
{
    let dir = project("reload-programs");
    let edits = dir.join("edits");
    fs::create_dir_all(&edits).unwrap();
    fs::write(dir.join("shell.ts"), spawning_shell(Some("31.6101"), "")).unwrap();
    fs::write(
        edits.join("again.ts"),
        spawning_shell(Some("31.6101"), "// saved again"),
    )
    .unwrap();
    fs::write(
        edits.join("throws.ts"),
        spawning_shell(Some("31.6102"), "throw new Error(\"late\");"),
    )
    .unwrap();
    fs::write(edits.join("other.ts"), spawning_shell(Some("31.6103"), "")).unwrap();
    fs::write(edits.join("none.ts"), spawning_shell(None, "")).unwrap();
    let put = |name: &str| format!("put:{}={}", dir.join("shell.ts").display(), edits.join(name).display());
    let run = drive(
        &dir.join("shell.ts"),
        &[
            "pump",
            // The program the entry started when it took over.
            "programs",
            &put("again.ts"),
            "sleep:500",
            "pump",
            "serve",
            // The same program again: the generation that took over keeps the process it has.
            "programs",
            &put("throws.ts"),
            "sleep:500",
            "pump",
            "serve",
            // A stage that starts a program and then fails starts nothing, and the running one is left alone.
            "programs",
            &put("other.ts"),
            "sleep:500",
            "pump",
            "serve",
            "sleep:300",
            // Another program: the first one is stopped and the other runs.
            "programs",
            &put("none.ts"),
            "sleep:500",
            "pump",
            "serve",
            "sleep:300",
            // No program: the one the config started is stopped.
            "programs",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let seen = programs_seen(&run);
    assert_eq!(seen.len(), 5, "{}", run.dump());
    let first = &seen[0];
    assert_eq!(first.len(), 1, "the entry started one program\n{}", run.dump());
    assert_eq!(
        &seen[1],
        first,
        "the same program again is the process that ran\n{}",
        run.dump()
    );
    assert_eq!(&seen[2], first, "a refused stage changed nothing\n{}", run.dump());
    assert_eq!(
        seen[3].len(),
        1,
        "one program runs after the replacement\n{}",
        run.dump()
    );
    assert_ne!(&seen[3], first, "the replacement is another process\n{}", run.dump());
    assert!(
        seen[4].is_empty(),
        "a config that starts none leaves none running\n{}",
        run.dump()
    );
    let outcomes: Vec<&str> = run.stdout.lines().filter(|line| line.starts_with("reload ")).collect();
    assert_eq!(outcomes.len(), 4, "{}", run.dump());
    assert!(outcomes[1].starts_with("reload failed"), "{}", run.dump());
}

/// How many times the compositor was told to set a property of a node, by each `writes` step of `run`, in order.
fn writes_seen(run: &Run) -> Vec<usize> {
    run.lines_with("writes id=")
        .iter()
        .filter_map(|line| line.split_once("count=").and_then(|(_, count)| count.parse().ok()))
        .collect()
}

#[test]
fn a_stage_that_writes_at_its_top_level_and_then_throws_leaves_no_write_behind_and_one_that_loads_sends_it() {
    let dir = project("reload-writes");
    let edits = dir.join("edits");
    fs::create_dir_all(&edits).unwrap();
    let entry = |rest: &str| format!("import {{ gnoblin }} from \"gnoblin-core\";\n{rest}\n");
    fs::write(dir.join("shell.ts"), entry("")).unwrap();
    fs::write(
        edits.join("throws.ts"),
        entry("gnoblin.clients.at(0).x = 999;\nthrow new Error(\"late\");"),
    )
    .unwrap();
    fs::write(edits.join("writes.ts"), entry("gnoblin.clients.at(0).x = 5;")).unwrap();
    let put = |name: &str| format!("put:{}={}", dir.join("shell.ts").display(), edits.join(name).display());
    let run = drive(
        &dir.join("shell.ts"),
        &[
            "pump",
            "writes:40:x",
            &put("throws.ts"),
            "sleep:500",
            // The runtime sees the change and asks the host, which serves the request through the loader: the stage is refused.
            "pump",
            "serve",
            "pump",
            "writes:40:x",
            &put("writes.ts"),
            "sleep:500",
            "pump",
            "serve",
            "pump",
            "writes:40:x",
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let outcomes: Vec<&str> = run.stdout.lines().filter(|line| line.starts_with("reload ")).collect();
    assert_eq!(outcomes.len(), 2, "{}", run.dump());
    assert!(outcomes[0].starts_with("reload failed"), "{}", run.dump());
    assert!(outcomes[1].starts_with("reload ok"), "{}", run.dump());
    assert_eq!(
        writes_seen(&run),
        vec![0, 0, 1],
        "the refused stage's write never went out; the one that loaded sent its own\n{}",
        run.dump()
    );
}

// ---- example 17 (JavaScript): a shell that imports only gnoblin-core -----------------------------------------------------------

#[test]
fn example_17_a_js_shell_that_imports_only_gnoblin_core_runs_through_the_runtime() {
    let dir = project("core-only");
    fs::write(
        dir.join("shell.js"),
        r#"import { ClientCollectionListener, gnoblin } from "gnoblin-core";

class Tiler extends ClientCollectionListener {
    onAdded() {
        tile();
    }
    onRemoved() {
        tile();
    }
}

function tile() {
    const output = gnoblin.outputs.at(0);
    const windows = gnoblin.clients.toList();
    const width = output.width / windows.length;
    windows.forEach((client, at) => {
        client.bounds = { x: output.x + at * width, y: output.y, width, height: output.height };
    });
}
gnoblin.clients.addListener(new Tiler());
"#,
    )
    .unwrap();
    let run = drive(
        &dir.join("shell.js"),
        &["pump", "open:50", "open:51", "mark:three", "dump", "stop"],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let three = run.section("three");
    let width = 1000.0 / 3.0;
    for (at, id) in [40_u64, 50, 51].into_iter().enumerate() {
        let (x, y, w, h) = run.window(&three, id);
        assert!(
            (x - at as f64 * width).abs() < 1e-6 && y == 0.0 && (w - width).abs() < 1e-6 && h == 800.0,
            "window {id}\n{}",
            run.dump()
        );
    }
    // The shell is core only: it made no node of its own, so none of std's scene or canvases exist.
    assert!(run.stdout.contains("made_canvases=0"), "{}", run.dump());
}

// ---- example 3 in TypeScript: a shell on the generated std objects ---------------------------------------------------------------
//
// Example 3 is written on the generated object API (`gnoblin-std/generated/objects`). `gnoblin-std/native`, the typed-table binding over
// `__gnoblinStd`, still ships with the TypeScript library, so each proof runs a second time on the shells of `tests/fixtures/typed_table`:
// example 3 as it was before it moved. They stay for as long as that binding does; each test of them says `typed_table` in its name.

/// A shell of `tests/fixtures/typed_table`: the typed-table binding's own proof.
fn typed_table(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/typed_table")
        .join(name)
}

/// The scenario of example 6 over `script`, in the prefix that has the std library: what a shell that arranges the clients of one
/// workspace in columns, and swaps the focused window with the first on `Super+Tab`, must do.
fn columns_run(script: &Path) -> Run {
    let stage = stage();
    scenario_in(&stage.with_std, &stage.with_std_runtimes, "example06", script)
}

fn assert_columns(run: &Run) {
    assert_eq!(run.code, 0, "{}", run.dump());
    // The same rectangles the native Rust shell of example 6 and the Python shell of example 3 produce.
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
    assert_eq!(num(&run.stdout, "maps", "core_copies"), 1, "{}", run.dump());
    assert_eq!(num(&run.stdout, "maps", "std_mappings"), 1, "{}", run.dump());
    assert_eq!(num(&run.stdout, "core_stats", "panics"), 0, "{}", run.dump());
}

#[test]
fn example_3_a_typescript_shell_on_the_generated_std_objects_arranges_the_clients_of_a_workspace_in_columns() {
    assert_columns(&columns_run(&example("03-typescript-std/shell.ts")));
}

#[test]
fn typed_table_a_typescript_shell_on_gnoblin_std_native_arranges_the_clients_of_a_workspace_in_columns() {
    assert_columns(&columns_run(&typed_table("shell.ts")));
}

/// Runs `script` in the prefix that has no std library: it must fail, and say which library it looked for.
fn assert_fails_naming_the_library(script: &Path) {
    let stage = stage();
    let run = scenario_in(&stage.dir, &stage.runtimes, "example06", script);
    assert_ne!(run.code, 0, "{}", run.dump());
    let text = format!("{}{}", run.stdout, run.stderr);
    assert!(
        text.contains("gnoblin-std is not installed (looked for libgnoblin-std.so.1 in "),
        "{}",
        run.dump()
    );
}

#[test]
fn example_17_a_typescript_shell_that_uses_the_std_objects_without_the_library_fails_naming_it() {
    assert_fails_naming_the_library(&example("03-typescript-std/shell.ts"));
}

#[test]
fn typed_table_example_17_a_typescript_shell_that_imports_std_native_without_the_library_fails_naming_it() {
    assert_fails_naming_the_library(&typed_table("shell.ts"));
}

/// The native Rust shell of the layouts (`gnoblin-std-impl/fixtures/shell-layouts`) over the same scenario: the reference.
fn native_layouts() -> Run {
    let stage = stage();
    let shell = stage.with_std.join("shells/libshell_layouts.so");
    let name = PathBuf::from("std_layouts");
    let output = scenario_with_env(
        &stage.with_std,
        &[name.as_path(), shell.as_path()],
        &[("LD_LIBRARY_PATH", stage.with_std.as_path())],
    );
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

/// Runs the layouts shell `layouts` and holds every step of it to the native shell's.
fn assert_layouts_match_the_native_shell(layouts: &Path) {
    let stage = stage();
    let native = native_layouts();
    assert_eq!(native.code, 0, "{}", native.dump());
    let script = scenario_in(&stage.with_std, &stage.with_std_runtimes, "std_layouts", layouts);
    assert_eq!(script.code, 0, "{}", script.dump());
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
    assert_eq!(num(&script.stdout, "core_stats", "panics"), 0, "{}", script.dump());
}

#[test]
fn the_built_in_layouts_from_typescript_give_the_rectangles_of_the_native_shell() {
    assert_layouts_match_the_native_shell(&example("03-typescript-std/layouts.ts"));
}

#[test]
fn typed_table_the_built_in_layouts_from_gnoblin_std_native_give_the_rectangles_of_the_native_shell() {
    assert_layouts_match_the_native_shell(&typed_table("layouts.ts"));
}

/// The engine and the library write through one connection: nothing is refused as busy, and no second attach is made.
fn assert_one_tree_nothing_refused(script: &Path) {
    let run = columns_run(script);
    assert_eq!(run.code, 0, "{}", run.dump());
    let text = format!("{}{}", run.stdout, run.stderr);
    assert!(!text.contains("E_BUSY"), "{}", run.dump());
    assert!(!text.contains("tree_attach"), "{}", run.dump());
}

#[test]
fn a_typescript_shell_on_the_std_objects_and_the_engine_share_one_tree_and_nothing_is_refused() {
    assert_one_tree_nothing_refused(&example("03-typescript-std/shell.ts"));
}

#[test]
fn typed_table_a_typescript_shell_on_gnoblin_std_native_and_the_engine_share_one_tree_and_nothing_is_refused() {
    assert_one_tree_nothing_refused(&typed_table("shell.ts"));
}
