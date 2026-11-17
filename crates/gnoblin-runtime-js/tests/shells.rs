//! The JavaScript runtime library, loaded by the core's loader into a process that links the installed core, serving shells over the
//! fake compositor (`design/modular-framework.md` sections 8.2, 10.2 and 12; M6.1, M6.2, M6.4).
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
    Ok(Stage {
        cache: base.join("cache"),
        dir,
        runtimes,
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
    let stage = stage();
    let mut all: Vec<PathBuf> = vec![PathBuf::from("reload"), script.to_owned()];
    all.extend(steps.iter().map(PathBuf::from));
    let refs: Vec<&Path> = all.iter().map(PathBuf::as_path).collect();
    let output = scenario_with_env(
        &stage.dir,
        &refs,
        &[
            ("GNOBLIN_RUNTIME_DIR", stage.runtimes.as_path()),
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
    let run = drive(
        &example("04-js-esm/shell.js"),
        &[
            "pump", "mark:one", "dump", "open:50", "mark:two", "dump", "stats", "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(run.stdout.contains("started generation=1"), "{}", run.dump());
    // The context is a JavaScript one, and the engine is in the runtime library: the one extra mapping is the runtime.
    assert!(
        run.stdout
            .contains("maps core_copies=1 shell_mappings=1 std_mappings=0"),
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
        r#"import {{ CollectionListener, gnoblin }} from "gnoblin-core";
import type {{ Client }} from "gnoblin-core";

import {{ place }} from "./place.ts";

// Every window the listener hears of adds a line to the marker file, so a listener that outlives its generation shows as a
// second line for one window.
class Tiler extends CollectionListener<Client> {{
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
    const windows = [...gnoblin.clients];
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
    const windows = [...gnoblin.clients];
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
            "stop",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
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

// ---- example 17 (JavaScript): a shell that imports only gnoblin-core -----------------------------------------------------------

#[test]
fn example_17_a_js_shell_that_imports_only_gnoblin_core_runs_through_the_runtime() {
    let dir = project("core-only");
    fs::write(
        dir.join("shell.js"),
        r#"import { CollectionListener, gnoblin } from "gnoblin-core";

class Tiler extends CollectionListener {
    onAdded() {
        tile();
    }
    onRemoved() {
        tile();
    }
}

function tile() {
    const output = gnoblin.outputs.at(0);
    const windows = [...gnoblin.clients];
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
