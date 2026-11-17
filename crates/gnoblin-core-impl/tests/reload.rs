//! Native hot reload (`design/modular-framework.md` sections 8.1 and 12): example 10, example 18, and the lifetime guarantees of the
//! reload protocol (M5.1, M5.3, M5.4).
//!
//! A real shell `.so` is loaded by `NativeShell` into a process that links the installed core (the launcher fixture), over the fake
//! compositor, and replaced while it runs. Example 10 is built twice from one source in a temporary project, once as it is in
//! `examples/10-hot-reload` and once with its two constants edited, which is what a developer's edit-and-rebuild loop produces.
//! Example 18 replaces a running shell with libraries that cannot take over: an `init` that errors, one that panics, a truncated file, a
//! file that is not a library, a missing file, a library for another ABI major and one that needs a symbol the core lacks, and a
//! failed build. Each leaves the old generation serving, leaves nothing of itself registered, names its reason in a text the tests
//! pin, and a good library afterwards still loads.
//!
//! What this does not prove, because the fake compositor has no Wayland behind it: that a real client's connection and surface
//! survive. It proves the property that makes that true, that the compositor-owned nodes (the windows) are neither destroyed nor
//! made again and keep their ids; `design/modular-framework.md` section 20 says what the real-compositor test still needs.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use gnoblin_shell_host::reload::BuildStep;

mod support;
use support::*;

/// The builds the tests share: the core and launcher, the reload fixture in each variant, and the two builds of example 10.
struct Stage {
    dir: PathBuf,
    hot: BTreeMap<&'static str, PathBuf>,
    example_v1: PathBuf,
    example_v2: PathBuf,
    /// A directory with the helper program example 10 starts: it records that it started, then sleeps.
    bin: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("reload-stage");
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
            "--features",
            "gnoblin-core-impl/test-driver",
        ],
        &[],
    )?;
    let debug = target.join("debug");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &dir.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &dir.join(LAUNCHER));

    // One source, a label chosen at build time, and two features for the libraries a loader must refuse.
    let mut hot = BTreeMap::new();
    for (name, label, features) in [
        ("v1", "v1", None),
        ("v2", "v2", None),
        ("bad", "bad", None),
        ("panic", "panic", None),
        ("wrong-abi", "wrong-abi", Some("gnoblin-fixture-shell-hot/wrong-abi")),
        (
            "newer-symbol",
            "newer-symbol",
            Some("gnoblin-fixture-shell-hot/newer-symbol"),
        ),
    ] {
        let mut args = vec!["-p", "gnoblin-fixture-shell-hot"];
        if let Some(features) = features {
            args.extend(["--features", features]);
        }
        cargo_build(&target, &args, &[("FIXTURE_LABEL", label)])?;
        hot.insert(
            name,
            copy_to(
                &debug.join("libshell_hot.so"),
                &dir.join(format!("builds/{name}/libshell_hot.so")),
            ),
        );
    }

    let (example_v1, example_v2) = build_example_10(&base, &dir)?;

    let bin = base.join("bin");
    fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    let helper = bin.join("hot-reload-helper");
    fs::write(
        &helper,
        "#!/bin/sh\necho started >> \"$GNOBLIN_TEST_MARKER\"\nexec sleep 5\n",
    )
    .map_err(|error| error.to_string())?;
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;

    Ok(Stage {
        dir,
        hot,
        example_v1,
        example_v2,
        bin,
    })
}

/// Builds example 10 twice from its one source in a project of its own: as it is, and with `VERSION` and `STEP` edited. The build is
/// `BuildStep`, the developer loop's own step, so this also proves it finds the library cargo wrote.
fn build_example_10(base: &Path, dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let root = repo_root();
    let project = base.join("example-10-project");
    fs::create_dir_all(project.join("src")).map_err(|error| error.to_string())?;
    let source =
        fs::read_to_string(root.join("examples/10-hot-reload/src/lib.rs")).map_err(|error| error.to_string())?;
    fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\nname = \"hot-reload-project\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
             [lib]\nname = \"hot_reload\"\ncrate-type = [\"cdylib\"]\n\n\
             [dependencies]\ngnoblin-core = {{ path = \"{}\" }}\n\n[workspace]\n",
            root.join("crates/gnoblin-core").display()
        ),
    )
    .map_err(|error| error.to_string())?;
    // The versions the workspace resolved, so an offline build needs nothing it has not already fetched.
    fs::copy(root.join("Cargo.lock"), project.join("Cargo.lock")).map_err(|error| error.to_string())?;
    let target = base.join("target-example-10");
    let target_arg = target.to_string_lossy().into_owned();
    let step = BuildStep::cargo(&project).with_args(["--offline", "--target-dir", target_arg.as_str()]);

    let entry = project.join("src").join("lib.rs");
    fs::write(&entry, &source).map_err(|error| error.to_string())?;
    let first = step.run().map_err(|failure| format!("{failure}\n{}", failure.stderr))?;
    let v1 = copy_to(&first.artifact, &dir.join("builds/example-10/v1/libhot_reload.so"));

    // The edit. Cargo compares modification times, so let a second pass between the two writes.
    std::thread::sleep(Duration::from_millis(1100));
    let edited = source
        .replace("const VERSION: u32 = 1;", "const VERSION: u32 = 2;")
        .replace("const STEP: f64 = 40.0;", "const STEP: f64 = 80.0;");
    assert_ne!(edited, source, "the example's two constants are what the test edits");
    fs::write(&entry, edited).map_err(|error| error.to_string())?;
    let second = step.run().map_err(|failure| format!("{failure}\n{}", failure.stderr))?;
    let v2 = copy_to(&second.artifact, &dir.join("builds/example-10/v2/libhot_reload.so"));
    Ok((v1, v2))
}

fn stage() -> &'static Stage {
    static STAGE: LazyLock<Result<Stage, String>> = LazyLock::new(build_stage);
    match &*STAGE {
        Ok(stage) => stage,
        Err(message) => panic!("cannot build the reload fixtures: {message}"),
    }
}

/// What one scenario printed, cut at its `mark` lines.
struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

impl Run {
    /// The lines between `mark NAME` and the next mark (or the end); the whole output for `None`.
    fn section(&self, name: Option<&str>) -> Vec<&str> {
        let mut lines: Vec<&str> = self.stdout.lines().collect();
        if let Some(name) = name {
            let start = lines
                .iter()
                .position(|line| *line == format!("mark {name}"))
                .unwrap_or_else(|| panic!("no mark {name} in:\n{}", self.stdout));
            lines.drain(..=start);
            if let Some(end) = lines.iter().position(|line| line.starts_with("mark ")) {
                lines.truncate(end);
            }
        }
        lines
    }

    fn lines<'a>(&'a self, section: Option<&str>, prefix: &str) -> Vec<&'a str> {
        self.section(section)
            .into_iter()
            .filter(|line| line.starts_with(prefix))
            .collect()
    }

    fn dump(&self) -> String {
        format!("--- stdout ---\n{}\n--- stderr ---\n{}", self.stdout, self.stderr)
    }
}

/// `value` of `key` on a line, as text.
fn word(line: &str, key: &str) -> String {
    let marker = format!("{key}=");
    line.split_whitespace()
        .find_map(|word| word.strip_prefix(marker.as_str()))
        .unwrap_or_else(|| panic!("no `{key}` in `{line}`"))
        .to_owned()
}

fn number(line: &str, key: &str) -> f64 {
    word(line, key)
        .parse()
        .unwrap_or_else(|error| panic!("`{key}` in `{line}` is not a number: {error}"))
}

/// A fresh place for a shell to be replaced in: its first build is copied there, and the scenario starts it from there.
fn live(stage: &Stage, name: &str, first: &Path) -> PathBuf {
    let path = stage.dir.join(format!("live/{name}/libshell.so"));
    copy_to(first, &path)
}

fn scenario_run(stage: &Stage, module: &Path, steps: &[&str], extra: &[(&str, &Path)]) -> Run {
    let path = format!(
        "{}:{}",
        stage.bin.display(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned())
    );
    let mut all: Vec<PathBuf> = vec![PathBuf::from("reload"), module.to_owned()];
    all.extend(steps.iter().map(PathBuf::from));
    let refs: Vec<&Path> = all.iter().map(PathBuf::as_path).collect();
    let mut env: Vec<(&str, &Path)> = vec![("PATH", Path::new(&path))];
    env.extend(extra.iter().copied());
    let output = scenario_with_env(&stage.dir, &refs, &env);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

// ---- example 10 -------------------------------------------------------------------------------------------------------------------

#[test]
fn example_10_reloads_three_times_without_touching_a_client_and_keeps_its_state_and_nothing_doubles() {
    let stage = stage();
    let module = live(stage, "example-10", &stage.example_v1);
    let marker = stage.dir.join("live/example-10/helper-started");
    let _ = fs::remove_file(&marker);
    let v2 = stage.example_v2.to_string_lossy().into_owned();
    let run = scenario_run(
        stage,
        &module,
        &[
            // Version 1 runs: two clients open, the chord counts a press, the timer ticks.
            "open:50",
            "open:51",
            "chord:Super+n",
            "advance:50",
            "mark:one",
            "dump",
            "stats",
            // The first reload, by a request to the controller as `gnoblinctl shell reload` makes it.
            &format!("copy:{v2}"),
            "mark:reload-one",
            "reload",
            "advance:50",
            "dump",
            "stats",
            // A client opens after the reload: the new behaviour (the step is 80, not 40) and the kept count (the fourth window).
            "mark:after-one",
            "open:52",
            "chord:Super+n",
            "advance:50",
            "dump",
            "stats",
            // The second and third reloads are asked for through the controller and served by the shell thread's loop.
            "mark:reload-two",
            "request",
            "serve",
            "mark:reload-three",
            "request",
            "status",
            "serve",
            "chord:Super+n",
            "advance:50",
            "dump",
            "stats",
            "status",
        ],
        &[("GNOBLIN_TEST_MARKER", marker.as_path())],
    );
    assert_eq!(run.code, 0, "{}", run.dump());

    // Before: generation 1 placed the window that was open (cascade slot 0) and the two that opened (slots 1 and 2, step 40).
    let windows = |section: &str| -> Vec<&str> { run.lines(Some(section), "window ") };
    let dump_one = windows("one");
    let geometry = |lines: &[&str], id: u64| -> (f64, f64, f64, f64) {
        let line = lines
            .iter()
            .find(|line| word(line, "id") == id.to_string())
            .unwrap_or_else(|| panic!("no window {id}\n{}", run.dump()));
        (
            number(line, "x"),
            number(line, "y"),
            number(line, "w"),
            number(line, "h"),
        )
    };
    assert_eq!(geometry(&dump_one, 40), (60.0, 60.0, 640.0, 480.0), "{}", run.dump());
    assert_eq!(geometry(&dump_one, 50), (100.0, 100.0, 640.0, 480.0), "{}", run.dump());
    assert_eq!(geometry(&dump_one, 51), (140.0, 140.0, 640.0, 480.0), "{}", run.dump());
    let canvases_one = run.lines(Some("one"), "canvas ");
    let badge = |lines: &[&str]| -> String {
        lines
            .iter()
            .find(|line| word(line, "name") == "reload-badge")
            .unwrap_or_else(|| panic!("no badge in {lines:?}\n{}", run.dump()))
            .to_string()
    };
    let badge_one = badge(&canvases_one);
    assert_eq!(
        (
            number(&badge_one, "x"),
            number(&badge_one, "y"),
            number(&badge_one, "z")
        ),
        (10.0, 1.0, 1.0)
    );
    assert!(
        canvases_one.iter().any(|line| word(line, "name") == "reload-toast"),
        "version 1 makes the ephemeral toast: {canvases_one:?}"
    );

    // The first reload: generation 2 took over from generation 1, which retired with its registrations removed.
    let first = run.lines(Some("reload-one"), "reload ");
    assert_eq!(first.len(), 1, "{}", run.dump());
    assert!(
        first[0].starts_with("reload ok generation=2 previous=1 "),
        "{}",
        first[0]
    );
    assert_eq!(
        word(first[0], "retained"),
        "1",
        "the old library is kept mapped, counted and never unloaded"
    );
    assert_eq!(word(first[0], "stale_calls"), "0");
    assert!(
        number(first[0], "removed") >= 4.0,
        "the old generation's listener, chords and timer were removed: {}",
        first[0]
    );

    // What the clients saw: nothing. Same windows, same ids, same places; no window was made or destroyed.
    let dump_after = windows("reload-one");
    for id in [40, 50, 51] {
        assert_eq!(
            geometry(&dump_after, id),
            geometry(&dump_one, id),
            "window {id} was not touched: {}",
            run.dump()
        );
    }
    let tree_after = run.lines(Some("reload-one"), "tree ")[0].to_owned();
    assert_eq!(word(&tree_after, "made_windows"), "0");
    assert_eq!(
        run.lines(Some("reload-one"), "first_window_destroys=")[0],
        "first_window_destroys=0",
        "the compositor's window was not destroyed"
    );
    // The nodes the shell owns: the badge is the same node (same id), one badge, version 2's `x`; the ephemeral toast is gone.
    let canvases_after = run.lines(Some("reload-one"), "canvas ");
    let badge_after = badge(&canvases_after);
    assert_eq!(
        word(&badge_after, "id"),
        word(&badge_one, "id"),
        "adopted by name: the same node, not a second one"
    );
    assert_eq!(number(&badge_after, "x"), 20.0, "the new generation's own write");
    assert_eq!(
        number(&badge_after, "z"),
        1.0,
        "the press count survived in the kept state and the node"
    );
    assert_eq!(
        canvases_after
            .iter()
            .filter(|line| word(line, "name") == "reload-badge")
            .count(),
        1
    );
    assert!(
        !canvases_after.iter().any(|line| word(line, "name") == "reload-toast"),
        "version 2 did not adopt the ephemeral toast, so the takeover destroyed it: {canvases_after:?}"
    );
    assert_eq!(
        word(&tree_after, "made_canvases"),
        "2",
        "the badge and the toast, made once each by generation 1"
    );

    // The claims: the chords are held once each, not twice (released and made again behind the release).
    assert!(
        tree_after.contains(r#"claims=["Super+n", "Super+Shift+r"]"#),
        "each chord held once after a reload: {tree_after}"
    );

    // After the reload a window opens: version 2's step (80) and the count the first generation kept (3 placed, so slot 3).
    let opened_windows = windows("after-one");
    assert_eq!(
        geometry(&opened_windows, 52),
        (60.0 + 80.0 * 3.0, 60.0 + 80.0 * 3.0, 640.0, 480.0),
        "new behaviour, continuing the old count: {}",
        run.dump()
    );
    let badge_two = badge(&run.lines(Some("after-one"), "canvas "));
    assert_eq!(
        number(&badge_two, "z"),
        2.0,
        "one press counted once, continuing the count: no second binding"
    );

    // The two requests reload twice more: generation 3 and generation 4.
    let two = run.lines(Some("reload-two"), "reload ");
    assert!(two[0].starts_with("reload ok generation=3 previous=2 "), "{}", two[0]);
    assert_eq!(word(two[0], "retained"), "2");
    let three = run.lines(Some("reload-three"), "reload ");
    assert!(!three.is_empty(), "{}", run.dump());
    assert!(
        three[0].starts_with("reload ok generation=4 previous=3 "),
        "{}",
        three[0]
    );
    assert_eq!(word(three[0], "retained"), "3");
    let status_waiting: Vec<&str> = run.lines(Some("reload-three"), "status a reload is waiting");
    assert_eq!(
        status_waiting,
        ["status a reload is waiting (it was requested)"],
        "the request reached the controller before the thread served it"
    );

    // The helper program was started by generation 1 and adopted by every later generation with the same arguments.
    let started = fs::read_to_string(&marker).unwrap_or_default();
    assert_eq!(
        started.lines().count(),
        1,
        "one helper, however many generations asked for it: {started:?}"
    );

    // The callback canary. Every retired generation's counters stood still after it retired, and each dropped exactly the
    // registrations it made. The core holds only the running generation's.
    let final_stats = run.section(Some("reload-three"));
    let retired: Vec<&str> = final_stats
        .iter()
        .copied()
        .filter(|line| line.starts_with("retired "))
        .collect();
    assert_eq!(retired.len(), 3, "generations 1, 2 and 3 retired: {}", run.dump());
    let first_snapshot = *run
        .section(Some("reload-one"))
        .iter()
        .find(|line| line.starts_with("retired generation=1 "))
        .unwrap_or_else(|| panic!("generation 1 not in the stats after the first reload\n{}", run.dump()));
    let last_snapshot = retired
        .iter()
        .find(|line| line.starts_with("retired generation=1 "))
        .expect("generation 1 stays on the retired list");
    assert_eq!(
        first_snapshot, *last_snapshot,
        "generation 1's counters did not move after it retired (no stale call reached it)"
    );
    for line in &retired {
        assert_eq!(
            word(line, "registered"),
            word(line, "drops"),
            "every registration released exactly once: {line}"
        );
    }
    let current = final_stats
        .iter()
        .find(|line| line.starts_with("current "))
        .expect("the running generation reports");
    let core = final_stats
        .iter()
        .find(|line| line.starts_with("core "))
        .expect("the core reports");
    assert_eq!(
        number(core, "registrations"),
        number(current, "registered") - number(current, "drops"),
        "the core holds exactly the running generation's registrations: {core} / {current}"
    );
    assert_eq!(
        number(core, "stale"),
        0.0,
        "no call from a retired generation reached the core"
    );
    // One private copy of the library is mapped per generation that ever loaded, plus the core once.
    let maps = final_stats.iter().find(|line| line.starts_with("maps ")).expect("maps");
    assert_eq!(word(maps, "core_copies"), "1");
    assert_eq!(
        word(maps, "shell_mappings"),
        "4",
        "generations 1 to 4, each in a copy of its own: {maps}"
    );
    assert_eq!(word(maps, "std_mappings"), "0");
}

#[test]
fn example_10_reloaded_with_the_same_build_still_reloads_and_keeps_counting() {
    // A build that changes nothing is a reload like any other: the file is replaced as a build replaces it (written beside, then
    // renamed over), the running shell is a private copy and is unaffected until the reload, and the kept count goes on.
    let stage = stage();
    let module = live(stage, "example-10-twice", &stage.example_v1);
    let marker = stage.dir.join("live/example-10-twice/helper-started");
    let _ = fs::remove_file(&marker);
    let v1 = stage.example_v1.to_string_lossy().into_owned();
    let run = scenario_run(
        stage,
        &module,
        &[
            "open:50",
            &format!("copy:{v1}"),
            "reload",
            "open:51",
            "reload",
            "open:52",
            "dump",
            "stats",
        ],
        &[("GNOBLIN_TEST_MARKER", marker.as_path())],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let reloads = run.lines(None, "reload ");
    assert_eq!(reloads.len(), 2, "{}", run.dump());
    assert!(
        reloads[0].starts_with("reload ok generation=2 previous=1 "),
        "{}",
        reloads[0]
    );
    assert!(
        reloads[1].starts_with("reload ok generation=3 previous=2 "),
        "{}",
        reloads[1]
    );
    // The windows were placed in order across the generations: the count was kept, so no slot repeats.
    let windows = run.lines(None, "window ");
    let places: Vec<(f64, f64)> = windows
        .iter()
        .map(|line| (number(line, "x"), number(line, "y")))
        .collect();
    assert_eq!(
        places,
        [(60.0, 60.0), (100.0, 100.0), (140.0, 140.0), (180.0, 180.0)],
        "{}",
        run.dump()
    );
}

// ---- example 18 -------------------------------------------------------------------------------------------------------------------

/// The text of a failed reload of `module` while generation `running` serves.
fn refused(module: &Path, running: u64, reason: &str) -> String {
    format!(
        "reload failed reload of {} failed, still running generation {running}: {reason}",
        module.display()
    )
}

#[test]
fn example_18_a_replacement_that_cannot_take_over_leaves_the_old_generation_serving_and_names_why() {
    let stage = stage();
    let module = live(stage, "example-18", &stage.hot["v1"]);
    let p = module.display().to_string();
    let hot = |name: &str| stage.hot[name].to_string_lossy().into_owned();
    let run = scenario_run(
        stage,
        &module,
        &[
            "lock",
            "stats",
            // 1. An `init` that returns an error after registering a callback.
            &format!("copy:{}", hot("bad")),
            "mark:bad",
            "reload",
            "lock",
            "stats",
            // 2. An `init` that panics after registering a callback.
            &format!("copy:{}", hot("panic")),
            "mark:panic",
            "reload",
            "lock",
            "stats",
            // 3. A file a build is still writing: cut inside the ELF header.
            &format!("copy:{}", hot("v2")),
            "truncate:40",
            "mark:truncated",
            "reload",
            "lock",
            // 4. A file that is not a library at all.
            "garbage",
            "mark:garbage",
            "reload",
            // 5. No file: the build failed and left none.
            "remove",
            "mark:missing",
            "reload",
            "lock",
            // 6. A library built for another ABI major.
            &format!("copy:{}", hot("wrong-abi")),
            "mark:wrong-abi",
            "reload",
            // 7. A library that needs a symbol the core does not export.
            &format!("copy:{}", hot("newer-symbol")),
            "mark:newer-symbol",
            "reload",
            "lock",
            "stats",
            // 8. A failed build: the build step reports its stderr; no library appears, no reload is asked for, the shell stays.
            "mark:build",
            "build:echo 'error[E0425]: cannot find value `x` in this scope' >&2; exit 101",
            "status",
            "lock",
            // And after all of that a good library takes over, and the status says what happened.
            &format!("copy:{}", hot("v2")),
            "mark:recovered",
            "reload",
            "lock",
            "stats",
            "status",
        ],
        &[],
    );
    assert_eq!(run.code, 0, "{}", run.dump());

    let expect_reason = |mark: &str, reason: &str, running: u64| {
        let lines = run.lines(Some(mark), "reload ");
        assert_eq!(lines, [refused(&module, running, reason)], "{}", run.dump());
    };
    expect_reason(
        "bad",
        &format!("{p}: init failed (E_LOAD): simulated init failure (bad shell)"),
        1,
    );
    // The panic text carries the place the shell panicked, which is source-line dependent: the rest is exact.
    let panic = run.lines(Some("panic"), "reload ");
    let panic_prefix = refused(
        &module,
        1,
        &format!("{p}: init failed (E_PANIC): simulated panic in init at "),
    );
    assert!(
        panic[0].starts_with(&panic_prefix),
        "{}\nwanted a start of: {panic_prefix}",
        panic[0]
    );
    assert!(panic[0].contains("fixtures/shell-hot/src/lib.rs:"), "{}", panic[0]);
    expect_reason(
        "truncated",
        &format!("{p}: cannot load the shell library: file too short"),
        1,
    );
    expect_reason(
        "garbage",
        &format!("{p}: cannot load the shell library: invalid ELF header"),
        1,
    );
    expect_reason(
        "missing",
        &format!("{p}: the shell library does not exist (a build that failed or has not finished leaves no file)"),
        1,
    );
    expect_reason(
        "wrong-abi",
        &format!("{p}: incompatible ABI: this shell was built for gnoblin ABI 2.3; the installed core is 1.0"),
        1,
    );
    expect_reason(
        "newer-symbol",
        &format!(
            "{p}: incompatible ABI: the shell needs the symbol `gnoblin_future_call`, which the installed core (ABI 1.0) does not \
             provide; it was built against a newer libgnoblin-core, so install that one or rebuild the shell"
        ),
        1,
    );

    // Through every failure the first generation answered, and nothing of a failed generation did: its callback was registered
    // before it failed and never ran, and the core holds exactly one registration.
    let lock_lines: Vec<&str> = run.lines(None, "[shell-hot ").into_iter().collect();
    let v1_answers = lock_lines
        .iter()
        .filter(|line| **line == "[shell-hot v1] lock.requested")
        .count();
    assert_eq!(
        v1_answers, 7,
        "generation 1 answered every `lock`, between the failures: {lock_lines:?}"
    );
    assert!(
        !lock_lines
            .iter()
            .any(|line| line.contains("bad") || line.contains("panic")),
        "a failed generation's callback never ran: {lock_lines:?}"
    );
    for mark in ["bad", "panic"] {
        let core = run.lines(Some(mark), "core ");
        assert_eq!(
            number(core[0], "registrations"),
            1.0,
            "only the running generation's registration is left: {}",
            core[0]
        );
    }
    let bad_current = run.lines(Some("bad"), "current ")[0];
    assert_eq!(
        number(bad_current, "registered"),
        number(bad_current, "drops") + 1.0,
        "{bad_current}"
    );

    // A library that was mapped and refused is counted and kept (never unloaded); one the dynamic loader refused maps nothing.
    // Mapped and refused: bad, panic, wrong-abi: three. Everything else failed before a mapping existed.
    let retained = run.lines(Some("newer-symbol"), "retained=");
    assert_eq!(retained, ["retained=3"], "{}", run.dump());

    // The failed build: the step's stderr is in the status and the log; no reload was asked for; the shell stayed.
    let build = run.lines(Some("build"), "build ");
    assert_eq!(build.len(), 1, "{}", run.dump());
    assert!(
        build[0].starts_with("build failed `sh -c echo 'error[E0425]: cannot find value `x` in this scope' >&2; exit 101` exited with 101: error[E0425]: cannot find value `x` in this scope"),
        "{}",
        build[0]
    );
    let status = run.lines(Some("build"), "status ");
    assert!(
        status.contains(&"status the last build failed (1 since the last good one): `sh -c echo 'error[E0425]: cannot find value `x` in this scope' >&2; exit 101` exited with 101: error[E0425]: cannot find value `x` in this scope"),
        "{status:?}"
    );
    assert!(
        run.stderr.contains("build of ") && run.stderr.contains("failed, still running generation 1:"),
        "the compiler's output is in the log:\n{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("error[E0425]: cannot find value `x` in this scope"),
        "{}",
        run.stderr
    );

    // A good library afterwards takes over. A generation number is spent by each failed load that got as far as a generation
    // (the `init` that errored and the one that panicked; the library of another ABI major was refused before it had one), so the
    // new one is generation 4, and it answers.
    let recovered = run.lines(Some("recovered"), "reload ");
    assert!(
        recovered[0].starts_with("reload ok generation=4 previous=1 "),
        "{}",
        recovered[0]
    );
    assert!(
        run.section(Some("recovered"))
            .contains(&"[shell-hot v2] lock.requested"),
        "the new generation answers: {}",
        run.dump()
    );
    let final_status = run.lines(Some("recovered"), "status ");
    assert!(
        final_status.contains(&"status generation 4 is running"),
        "{final_status:?}"
    );
    assert!(
        final_status.contains(&"status 1 reload(s) committed, 7 failed"),
        "{final_status:?}"
    );
    // A good reload does not clear a failed build, only a good build does: the build that failed is still the last build.
    assert!(
        final_status
            .iter()
            .any(|line| line.starts_with("status the last build failed (1 since the last good one): ")),
        "{final_status:?}"
    );
}

#[test]
fn example_18_the_retire_limit_refuses_the_reload_before_anything_is_copied_and_the_shell_serves_on() {
    let stage = stage();
    let module = live(stage, "example-18-limit", &stage.hot["v1"]);
    let v2 = stage.hot["v2"].to_string_lossy().into_owned();
    let run = scenario_run(
        stage,
        &module,
        &[
            "limit:2",
            &format!("copy:{v2}"),
            "reload",
            "reload",
            "mark:full",
            "reload",
            "lock",
            "status",
            "stats",
        ],
        &[],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let reloads = run.lines(None, "reload ");
    assert!(
        reloads[0].starts_with("reload ok generation=2 previous=1 "),
        "{}",
        reloads[0]
    );
    assert!(
        reloads[1].starts_with("reload ok generation=3 previous=2 "),
        "{}",
        reloads[1]
    );
    assert_eq!(
        run.lines(Some("full"), "reload "),
        [refused(
            &module,
            3,
            "2 retired shell libraries are still mapped (the limit is 2). Retired native code is never unloaded, so restart \
             gnoblin to reload the shell again"
        )],
        "{}",
        run.dump()
    );
    assert!(
        run.section(Some("full")).contains(&"[shell-hot v2] lock.requested"),
        "the running generation serves on: {}",
        run.dump()
    );
    let status = run.lines(Some("full"), "status ");
    assert!(status.contains(&"status generation 3 is running"), "{status:?}");
    // The refusal was made before the library was copied: the map shows three copies (generations 1 to 3), not four.
    let maps = run.lines(Some("full"), "maps ")[0];
    assert_eq!(word(maps, "shell_mappings"), "3", "{maps}");
}
