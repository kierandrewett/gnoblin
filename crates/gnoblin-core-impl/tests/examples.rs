//! Examples 1 and 8 of `design/modular-framework.md` section 12, as tests (M3.4).
//!
//! The examples are real shells: cargo `cdylib` crates under `examples/` that depend on `gnoblin-core` and nothing else. These tests
//! build them, load each `.so` through the shell host's loader into a process that links the installed core (the launcher fixture),
//! put the fake compositor behind the tree link, and drive windows, a closing window and key chords through it. The shell's
//! behaviour is asserted from what the compositor holds afterwards, and the ELF checks the design names are made on the files:
//! `NEEDED` is the core library only (no `gnoblin-std`), the shell defines no `gnoblin_*` symbol, and the process maps one core and
//! no std. The ABI-upgrade test runs the very same compiled example against a core rebuilt with a function added to the table.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

mod support;
use support::*;

/// A staged core, the launcher beside it, the two example shells, and a core of ABI 1.1 that the same shells run against.
struct Stage {
    dir: PathBuf,
    minimal: PathBuf,
    tiler: PathBuf,
    next_dir: PathBuf,
    /// A directory with a stand-in `foot`, which creates the file `GNOBLIN_TEST_MARKER` names.
    bin: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("examples-stage");
    let _ = fs::remove_dir_all(&base);
    let main_target = base.join("target-main");
    let next_target = base.join("target-next");
    let core_dir = base.join("core10");
    let next_dir = base.join("core11");

    cargo_build(
        &main_target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-fixture-launcher",
            "-p",
            "gnoblin-example-minimal-core-shell",
            "-p",
            "gnoblin-example-no-workspaces",
            "--features",
            "gnoblin-core-impl/test-driver",
        ],
        &[],
    )?;
    let debug = main_target.join("debug");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &core_dir.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &core_dir.join(LAUNCHER));
    let minimal = copy_to(
        &debug.join("libminimal_core_shell.so"),
        &core_dir.join("shells/libminimal_core_shell.so"),
    );
    let tiler = copy_to(
        &debug.join("libno_workspaces.so"),
        &core_dir.join("shells/libno_workspaces.so"),
    );

    // A core of ABI 1.1 (one function appended to the table), next to the same launcher binary and the same compiled shells: the
    // examples were built above, against the 1.0 table, and are not rebuilt.
    cargo_build(
        &next_target,
        &[
            "-p",
            "gnoblin-core-impl",
            "--features",
            "gnoblin-core-impl/test-driver,gnoblin-core-impl/next-minor",
        ],
        &[],
    )?;
    copy_to(
        &next_target.join("debug/libgnoblin_core_impl.so"),
        &next_dir.join(CORE_FILE),
    );
    copy_to(&debug.join(LAUNCHER), &next_dir.join(LAUNCHER));
    copy_to(&minimal, &next_dir.join("shells/libminimal_core_shell.so"));
    copy_to(&tiler, &next_dir.join("shells/libno_workspaces.so"));

    let bin = base.join("bin");
    fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    let foot = bin.join("foot");
    fs::write(&foot, "#!/bin/sh\n: > \"$GNOBLIN_TEST_MARKER\"\n").map_err(|error| error.to_string())?;
    fs::set_permissions(&foot, fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;
    Ok(Stage {
        dir: core_dir,
        minimal,
        tiler,
        next_dir,
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

/// Runs scenario `name` over `shell` in `dir`, with the stand-in terminal on the `PATH`, and returns stdout, stderr and the exit code.
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
        &[("PATH", Path::new(&path)), ("GNOBLIN_TEST_MARKER", marker.as_path())],
    )
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

// ---- the files --------------------------------------------------------------------------------------------------------

/// What example 1 and example 8 share on disk: the core library is the only library of ours a shell needs, and the shell defines the
/// one entry symbol and nothing in the core's namespace.
fn assert_core_only(shell: &Path) {
    let dynamic = dynamic(shell);
    assert!(dynamic.needed.iter().any(|needed| needed == CORE_FILE), "{dynamic:?}");
    assert!(
        !dynamic
            .needed
            .iter()
            .any(|needed| needed.contains("gnoblin") && needed != CORE_FILE),
        "a core-only shell loads nothing else of ours (no gnoblin-std): {:?}",
        dynamic.needed
    );
    assert_eq!(
        defined_symbols(shell),
        BTreeSet::from(["gnoshell_entry_v1".to_owned()]),
        "the shell defines no gnoblin_* symbol: the core's are the only ones in the process"
    );
    let wanted: BTreeSet<String> = undefined_symbols(shell)
        .into_iter()
        .filter(|symbol| symbol.starts_with("gnoblin_"))
        .collect();
    assert!(!wanted.is_empty(), "the shell calls into the core");
    assert!(
        wanted.is_subset(&exports()),
        "the shell references a gnoblin_ symbol the core does not export: {wanted:?}"
    );
}

#[test]
fn example_1_needs_the_core_only_and_defines_one_entry_symbol() {
    assert_core_only(&stage().minimal);
}

#[test]
fn example_8_needs_the_core_only_and_defines_one_entry_symbol() {
    assert_core_only(&stage().tiler);
}

// ---- example 1 --------------------------------------------------------------------------------------------------------------

#[test]
fn example_1_places_every_window_claims_two_chords_and_closes_the_focused_window() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.dir, "example01", &stage.minimal);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(
        stdout.contains("loaded label=gnoblin-example-minimal-core-shell"),
        "{stdout}"
    );

    // The window that was open before the shell started is placed in the first slot at once; the two that open later take the next
    // slots of the cascade. The shell wrote them as the compositor's own windows: the first is where it was put at the end too.
    let placed = windows(&stdout);
    assert_eq!(
        placed,
        [
            (40, Some([60.0, 60.0, 800.0, 600.0])),
            (41, Some([100.0, 100.0, 800.0, 600.0])),
            (42, Some([140.0, 140.0, 800.0, 600.0])),
            (40, Some([60.0, 60.0, 800.0, 600.0])),
        ],
        "{stdout}"
    );

    assert!(stdout.contains("claims=[\"Super+Return\", \"Super+q\"]"), "{stdout}");
    assert!(
        stdout.contains("terminal_started=true"),
        "the chord started the terminal: {stdout}"
    );
    assert!(
        stdout.contains("call node=41 method=close"),
        "the chord closed the window that had the keyboard: {stdout}"
    );
    assert!(!stdout.contains("method=kill"), "{stdout}");
    assert_eq!(
        num(&stdout, "new_nodes", "new_nodes"),
        0,
        "the shell made no node of its own: {stdout}"
    );

    // One core, one shell, no std in the process, and the shell's log line went through the core.
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "shell_mappings"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "std_mappings"), 0, "{stdout}");
    assert!(stderr.contains("minimal core shell is running"), "{stderr}");
    assert_eq!(num(&stdout, "core_stats", "panics"), 0, "{stdout}");
}

// ---- example 8 --------------------------------------------------------------------------------------------------------------

#[test]
fn example_8_tiles_raw_window_nodes_and_makes_no_node_of_its_own() {
    let stage = stage();
    let (stdout, stderr, code) = scenario_of(stage, &stage.dir, "example08", &stage.tiler);
    assert_eq!(code, 0, "{stdout}\n{stderr}");

    // One window fills the screen less the gap; two are master and stack; three split the stack; a window closing tiles the rest
    // again. The screen is 1000 by 800 and the gap 10.
    assert_eq!(
        step(&stdout, "one"),
        [(40, Some([10.0, 10.0, 980.0, 780.0]))],
        "{stdout}"
    );
    assert_eq!(
        step(&stdout, "two"),
        [
            (40, Some([10.0, 10.0, 485.0, 780.0])),
            (41, Some([505.0, 10.0, 485.0, 780.0])),
        ],
        "{stdout}"
    );
    assert_eq!(
        step(&stdout, "three"),
        [
            (40, Some([10.0, 10.0, 485.0, 780.0])),
            (41, Some([505.0, 10.0, 485.0, 385.0])),
            (42, Some([505.0, 405.0, 485.0, 385.0])),
        ],
        "{stdout}"
    );
    assert_eq!(
        step(&stdout, "closed"),
        [
            (40, Some([10.0, 10.0, 485.0, 780.0])),
            (41, None),
            (42, Some([505.0, 10.0, 485.0, 780.0])),
        ],
        "{stdout}"
    );
    // `Super+m` makes the focused window the master: it goes first among the root's children and the rest follow.
    assert_eq!(
        step(&stdout, "promoted"),
        [
            (40, Some([505.0, 10.0, 485.0, 780.0])),
            (42, Some([10.0, 10.0, 485.0, 780.0])),
        ],
        "{stdout}"
    );
    assert!(stdout.contains("order=[42, 20, 21, 40]"), "{stdout}");
    assert!(stdout.contains("terminal_started=true"), "{stdout}");

    // No scene graph: the shell created no node at all (asserted from the ops the compositor received), and no std is mapped.
    assert_eq!(num(&stdout, "new_nodes", "new_nodes"), 0, "{stdout}");
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "std_mappings"), 0, "{stdout}");
    assert_eq!(num(&stdout, "core_stats", "panics"), 0, "{stdout}");
}

// ---- the upgrade ------------------------------------------------------------------------------------------------------------

#[test]
fn the_examples_compiled_against_abi_1_0_run_unchanged_on_a_core_with_a_function_added() {
    let stage = stage();
    for (scenario, shell) in [("example01", &stage.minimal), ("example08", &stage.tiler)] {
        let name = shell.file_name().unwrap();
        let (base_out, base_err, base_code) = scenario_of(stage, &stage.dir, scenario, shell);
        let next_shell = stage.next_dir.join("shells").join(name);
        let (next_out, next_err, next_code) = scenario_of(stage, &stage.next_dir, scenario, &next_shell);
        assert_eq!(
            (base_code, next_code),
            (0, 0),
            "{base_out}\n{base_err}\n--\n{next_out}\n{next_err}"
        );
        assert_eq!(
            field(&base_out, "core_abi", "core_abi").as_deref(),
            Some("1.0"),
            "{base_out}"
        );
        assert_eq!(
            field(&next_out, "core_abi", "core_abi").as_deref(),
            Some("1.1"),
            "{next_out}"
        );
        // The binary of the shell is byte for byte the one built against 1.0, and behaves the same on the newer core.
        assert_eq!(fs::read(shell).unwrap(), fs::read(&next_shell).unwrap());
        let behaviour = |stdout: &str| {
            stdout
                .lines()
                .filter(|line| {
                    !line.starts_with("core_abi=")
                        && !line.starts_with("maps core_path=")
                        && !line.starts_with("loaded ")
                        && !line.starts_with("core_stats")
                })
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            behaviour(&base_out),
            behaviour(&next_out),
            "{scenario} behaves the same on the newer core"
        );
    }
}
