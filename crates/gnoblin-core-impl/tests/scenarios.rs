//! The proof spike's scenarios as tests (`design/modular-framework-spike.md`, results 3 to 6), against the real core.
//!
//! The core is a `cdylib` that a launcher links, so each scenario runs a launcher process that loads fixture shells into it
//! (`gnoblin-core-impl`'s `test-driver` feature) and prints `key=value` lines these tests assert on. The fixtures are real
//! shells built with the SDK; the tests build them with nested `cargo build` invocations into their own target
//! directories, because a test of one package cannot make Cargo build another package's `cdylib`.
//!
//! What is covered: the core's SONAME, `NODELETE` and exported symbols; the launcher's single NEEDED; a shell with the core
//! as its only NEEDED and one `gnoshell_entry_v1` export; exactly one core mapping with a shell loaded, and the
//! negative control that a shell linking the implementation as a Rust library is caught; a panic in a callback and in a
//! core export contained; a reload by unique copy with a staged generation, a retire without `dlclose` and no stale call;
//! an abandoned reload; a shell built for ABI 1.0 on a core of 1.1 and the reverse; thread affinity; and loader errors.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gnoblin_core_abi as abi;

mod support;
use support::*;

/// A directory holding a staged core, the launcher next to it, and the fixture shells.
struct Stage {
    dir: PathBuf,
    shell_min: PathBuf,
    /// The shell with a poll loop of its own on `wake_fd`.
    shell_wake: PathBuf,
    hot_v1: PathBuf,
    hot_v2: PathBuf,
    hot_bad: PathBuf,
    /// The reload shell built so that it reports it is safe to unload (`gnoshell_unload_safe` answers yes).
    hot_safe: PathBuf,
    rlib_control: PathBuf,
    /// A core built as ABI 1.1 and the shell built against it, next to the same launcher binary.
    next_dir: PathBuf,
    next_shell_min: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("abi-stage");
    let _ = fs::remove_dir_all(&base);
    let main_target = base.join("target-main");
    // One target directory for both cores: the compositor is inside the library now, and building its dependencies twice would
    // double the disk the nested builds take. The next-minor build only recompiles the ABI crate and the core.
    let next_target = main_target.clone();
    let core_dir = base.join("core10");
    let next_dir = base.join("core11");

    // The negative control first: a shell that links the implementation as a Rust library. Built without the driver, so it
    // does not export the test driver's symbol; the main build below overwrites the uplifted library.
    cargo_build(
        &main_target,
        &["-p", "gnoblin-core-impl", "--example", "rlib_control"],
        &[],
    )?;
    let rlib_control = copy_to(
        &main_target.join("debug/examples/librlib_control.so"),
        &base.join("control/librlib_control.so"),
    );

    // The core with the driver, the launcher, and the minimal shell, all as ABI 1.0.
    cargo_build(
        &main_target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-fixture-launcher",
            "-p",
            "gnoblin-fixture-shell-min",
            "-p",
            "gnoblin-fixture-shell-wake",
            "--features",
            "gnoblin-core-impl/test-driver",
        ],
        &[],
    )?;
    let debug = main_target.join("debug");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &core_dir.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &core_dir.join(LAUNCHER));
    let shell_min = copy_to(&debug.join("libshell_min.so"), &core_dir.join("shells/libshell_min.so"));
    let shell_wake = copy_to(
        &debug.join("libshell_wake.so"),
        &core_dir.join("shells/libshell_wake.so"),
    );

    // The reload generations: one source, three labels.
    let mut hot = Vec::new();
    for label in ["v1", "v2", "bad", "safe"] {
        cargo_build(
            &main_target,
            &["-p", "gnoblin-fixture-shell-hot"],
            &[("FIXTURE_LABEL", label)],
        )?;
        hot.push(copy_to(
            &debug.join("libshell_hot.so"),
            &core_dir.join(format!("shells/{label}/libshell_hot.so")),
        ));
    }

    // A core of ABI 1.1 (one function appended to the table) and a shell built against it. The launcher is the very same
    // file as in `core10`: the upgrade claim is that nothing but the library changed.
    cargo_build(
        &next_target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-fixture-shell-min",
            "--features",
            "gnoblin-core-impl/test-driver,gnoblin-core-impl/next-minor,gnoblin-fixture-shell-min/next-minor",
        ],
        &[],
    )?;
    let next_debug = next_target.join("debug");
    copy_to(&next_debug.join("libgnoblin_core_impl.so"), &next_dir.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &next_dir.join(LAUNCHER));
    let next_shell_min = copy_to(
        &next_debug.join("libshell_min.so"),
        &next_dir.join("shells/libshell_min_next.so"),
    );
    // The shell built against 1.0, unchanged, also sits next to the 1.1 core.
    copy_to(&shell_min, &next_dir.join("shells/libshell_min.so"));

    Ok(Stage {
        dir: core_dir,
        shell_min,
        shell_wake,
        hot_v1: hot[0].clone(),
        hot_v2: hot[1].clone(),
        hot_bad: hot[2].clone(),
        hot_safe: hot[3].clone(),
        rlib_control,
        next_dir,
        next_shell_min,
    })
}

fn stage() -> &'static Stage {
    static STAGE: OnceLock<Result<Stage, String>> = OnceLock::new();
    match STAGE.get_or_init(build_stage) {
        Ok(stage) => stage,
        Err(message) => panic!("cannot build the fixtures: {message}"),
    }
}

// ---- the core library and its callers ---------------------------------------------------------------------------------

#[test]
fn the_core_has_the_soname_nodelete_and_exports_exactly_the_listed_symbols() {
    let stage = stage();
    let core = stage.dir.join(CORE_FILE);
    let dynamic = dynamic(&core);
    assert_eq!(dynamic.soname.as_deref(), Some("libgnoblin-core.so.1"), "{dynamic:?}");
    assert!(dynamic.flags_1.contains("NODELETE"), "FLAGS_1 is {:?}", dynamic.flags_1);
    assert!(
        !dynamic.needed.iter().any(|needed| needed.contains("gnoblin")),
        "the core needs no other gnoblin library: {:?}",
        dynamic.needed
    );

    // The driver adds one export; a shipped build has exactly `exports.txt`.
    let mut expected = exports();
    expected.insert("gnoblin_test_main".to_owned());
    assert_eq!(defined_symbols(&core), expected);
    for symbol in defined_symbols(&core) {
        assert!(
            symbol.starts_with("gnoblin_"),
            "{symbol} is not in the gnoblin_ namespace"
        );
    }
}

#[test]
fn the_launcher_needs_the_core_exactly_once_and_finds_it_beside_itself() {
    let stage = stage();
    let launcher = stage.dir.join(LAUNCHER);
    let dynamic = dynamic(&launcher);
    let count = dynamic
        .needed
        .iter()
        .filter(|needed| needed.as_str() == CORE_FILE)
        .count();
    assert_eq!(count, 1, "NEEDED is {:?}", dynamic.needed);
    assert!(
        dynamic.runpath.as_deref().is_some_and(|path| path.contains("$ORIGIN")),
        "{dynamic:?}"
    );
    assert!(
        !dynamic
            .needed
            .iter()
            .any(|needed| needed.contains("gnoblin_std") || needed.contains("gnoblin-std"))
    );
    let ldd = tool("ldd", &[], &launcher);
    assert!(
        ldd.contains(&format!("{CORE_FILE} => {}", stage.dir.display())),
        "{ldd}"
    );
}

#[test]
fn a_shell_needs_the_core_only_and_exports_one_entry_symbol() {
    let stage = stage();
    let dynamic = dynamic(&stage.shell_min);
    assert!(dynamic.needed.iter().any(|needed| needed == CORE_FILE), "{dynamic:?}");
    assert!(
        !dynamic
            .needed
            .iter()
            .any(|needed| needed.contains("gnoblin") && needed != CORE_FILE),
        "a core-only shell must not load anything else of ours: {:?}",
        dynamic.needed
    );
    assert_eq!(
        defined_symbols(&stage.shell_min),
        BTreeSet::from(["gnoshell_entry_v1".to_owned()])
    );
    let undefined = undefined_symbols(&stage.shell_min);
    let wanted: BTreeSet<String> = undefined
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
fn a_shell_that_links_the_implementation_as_a_rust_library_is_caught() {
    let stage = stage();
    let defined: BTreeSet<String> = defined_symbols(&stage.rlib_control)
        .into_iter()
        .filter(|symbol| symbol.starts_with("gnoblin_"))
        .collect();
    assert!(
        exports().is_subset(&defined),
        "the control must carry its own copy of the core's exports so the check has something to catch: {defined:?}"
    );
    assert!(
        !dynamic(&stage.rlib_control)
            .needed
            .iter()
            .any(|needed| needed == CORE_FILE),
        "the control does not link the installed core: it carries a copy"
    );
}

#[test]
fn one_core_mapping_with_a_shell_loaded() {
    let stage = stage();
    let (stdout, stderr, code) = run(&stage.dir, &["run"], &[&stage.shell_min]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(
        stdout.contains("loaded label=gnoblin-fixture-shell-min generation=1"),
        "{stdout}"
    );
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "shell_mappings"), 1, "{stdout}");
    assert!(
        stdout.contains("[shell-min] lock.requested"),
        "the shell's callback ran: {stdout}"
    );
    assert_eq!(num(&stdout, "shell_stats", "registered"), 2);
    assert_eq!(num(&stdout, "shell_stats", "calls"), 1);
    assert_eq!(num(&stdout, "shell_stats", "drops"), 0);
    assert!(
        stderr.contains("shell-min init (core only)"),
        "the shell's log line went through the core: {stderr}"
    );
}

// ---- panics -----------------------------------------------------------------------------------------------------------

#[test]
fn a_panic_in_a_callback_and_in_a_core_export_is_contained_and_logged() {
    let stage = stage();
    let (stdout, stderr, code) = run(&stage.dir, &["panic"], &[&stage.shell_min]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("selftest_panic rc=-2"), "{stdout}");
    assert!(
        stdout.contains("last_error=\"panic in core export selftest_panic: selftest panic inside the core export\""),
        "{stdout}"
    );
    // The panic hook the SDK installed routed the shell's panic to the core's log, with the generation.
    assert!(
        stderr.contains("PANIC in shell generation 1: boom from the shell callback"),
        "{stderr}"
    );
    assert!(
        stderr.contains("panicked; contained, the next event is still delivered"),
        "{stderr}"
    );
    // The event published after the panic was still delivered.
    assert!(stdout.contains("[shell-min] lock.requested"), "{stdout}");
    assert!(num(&stdout, "core_stats", "panics") >= 2, "{stdout}");
    assert!(stdout.contains("still_alive=true"));
}

// ---- reload -----------------------------------------------------------------------------------------------------------

#[test]
fn a_reload_by_unique_copy_swaps_generations_with_no_stale_call_and_no_dlclose() {
    let stage = stage();
    let (stdout, stderr, code) = run(&stage.dir, &["hot"], &[&stage.hot_v1, &stage.hot_v2, &stage.hot_bad]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");

    assert_eq!(num(&stdout, "old_before_swap", "registered"), 1);
    assert_eq!(num(&stdout, "old_before_swap", "calls"), 1);
    assert!(stdout.contains("new_state=Staged"), "{stdout}");
    // The staged generation received nothing while the old one kept serving.
    assert_eq!(num(&stdout, "new_while_staged", "registered"), 1);
    assert_eq!(num(&stdout, "new_while_staged", "calls"), 0, "{stdout}");
    assert_eq!(num(&stdout, "old_while_new_staged", "calls"), 2);

    assert!(
        stdout.contains("retired old generation=1 removed=1 state=Retired"),
        "{stdout}"
    );
    assert_eq!(num(&stdout, "old_after_retire", "registered"), 1);
    assert_eq!(
        num(&stdout, "old_after_retire", "drops"),
        1,
        "drop_user ran for the one registration: {stdout}"
    );
    assert_eq!(
        num(&stdout, "stale_calls_into_retired", "stale_calls_into_retired"),
        0,
        "{stdout}"
    );
    assert_eq!(num(&stdout, "new_active", "calls"), 5);

    // A call into the core from the retired generation is refused and does nothing.
    assert!(stdout.contains("call_from_retired reg=0"), "{stdout}");
    assert!(
        stdout.contains("E_RETIRED: generation 1 has been retired; on_signal did nothing"),
        "{stdout}"
    );
    assert_eq!(num(&stdout, "core_stats", "stale"), 1);
    assert_eq!(num(&stdout, "core_stats", "retired"), 1);

    // Two shell copies and one core mapped; the retired library is kept, never unloaded.
    assert_eq!(num(&stdout, "maps", "core_copies"), 1, "{stdout}");
    assert_eq!(num(&stdout, "maps", "shell_mappings"), 2, "{stdout}");
    assert!(stdout.lines().any(|line| line == "retired_libraries=1"), "{stdout}");

    // The failed reload keeps the running generation and names the reason; its half-made registration is gone.
    assert!(stdout.contains("reload_failed kept_generation=2"), "{stdout}");
    assert!(
        stdout.contains("init failed (E_LOAD): simulated init failure (bad shell)"),
        "{stdout}"
    );
    assert_eq!(num(&stdout, "after_failed_reload", "calls"), 6, "{stdout}");
    assert_eq!(
        num(&stdout, "core_after_failed_reload", "registrations"),
        1,
        "only the running generation's registration is left: {stdout}"
    );
    assert!(stdout.lines().any(|line| line == "retired_libraries=2"), "{stdout}");
}

#[test]
fn an_abandoned_reload_drops_its_registrations_without_calling_them() {
    let stage = stage();
    let (stdout, stderr, code) = run(&stage.dir, &["abandon"], &[&stage.hot_v1, &stage.hot_v2]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert_eq!(num(&stdout, "new_before_abandon", "calls"), 0);
    assert_eq!(num(&stdout, "new_before_abandon", "drops"), 0);
    assert!(stdout.contains("abandoned removed=1 state=Retired"), "{stdout}");
    assert_eq!(num(&stdout, "new_after_abandon", "calls"), 0);
    assert_eq!(
        num(&stdout, "new_after_abandon", "drops"),
        1,
        "the closure was freed: {stdout}"
    );
    assert_eq!(num(&stdout, "old_still_serving", "calls"), 2);
    assert_eq!(num(&stdout, "new_after_more_events", "calls"), 0);
}

#[test]
fn retired_generations_stay_mapped_but_the_count_is_bounded_visible_and_a_reload_past_the_limit_is_refused() {
    let stage = stage();
    // The limit is the last word of the command line, after the module.
    let output = scenario(&stage.dir, &[Path::new("retain"), &stage.hot_v1, Path::new("3")]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "{stdout}\n{stderr}");
    assert!(stdout.contains("policy max_retained=3 allow_unload=false"), "{stdout}");

    // Each reload retires the previous generation and keeps its library: the count grows by one a round, and so does the
    // size of what is mapped.
    let mut previous_bytes = 0;
    for (round, retained) in [(0, 0), (1, 1), (2, 2), (3, 3)] {
        let prefix = format!("round={round} loaded");
        assert_eq!(num(&stdout, &prefix, "retained"), retained, "{stdout}");
        assert_eq!(num(&stdout, &prefix, "max"), 3);
        let bytes = num(&stdout, &prefix, "mapped_bytes");
        assert!(bytes >= previous_bytes, "the mapped size never shrinks: {stdout}");
        if retained > 0 {
            assert!(bytes > 0, "a retained library has a size: {stdout}");
        }
        previous_bytes = bytes;
    }
    assert!(
        stdout.contains("status=3 of 3 retired shell libraries retained ("),
        "the status line names the count and the limit: {stdout}"
    );

    // The next reload is refused with a text that tells the user what to do, and the running generation serves on.
    let refused = stdout
        .lines()
        .find(|line| line.starts_with("round=4 refused"))
        .unwrap_or_else(|| panic!("round 4 was not refused:\n{stdout}"));
    assert!(
        refused.contains("3 retired shell libraries are still mapped"),
        "{refused}"
    );
    assert!(refused.contains("the limit is 3"), "{refused}");
    assert!(refused.contains("restart gnoblin"), "{refused}");
    assert_eq!(
        num(&stdout, "serving_after_refusal", "calls"),
        1,
        "the running generation still gets its events: {stdout}"
    );
    assert!(stdout.contains("retained_after_refusal=3"), "{stdout}");
    assert_eq!(
        num(&stdout, "shell_mappings", "shell_mappings"),
        4,
        "the three retired copies and the running one are all still mapped: {stdout}"
    );
}

#[test]
fn unloading_a_retired_generation_is_opt_in_and_only_when_the_module_proves_it_is_unreachable() {
    let stage = stage();
    let output = scenario(
        &stage.dir,
        &[Path::new("unload"), &stage.hot_safe, &stage.hot_v1, &stage.shell_min],
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "{stdout}\n{stderr}");

    // Off by default, even for a module that says it is safe.
    assert!(stdout.contains("default_policy refused=NotAllowed"), "{stdout}");
    assert_eq!(num(&stdout, "after_default", "retained"), 1, "{stdout}");
    // A generation that has not retired is never unloaded.
    assert!(stdout.contains("running refused=NotRetired"), "{stdout}");
    // A module that answers no, and one with no such export, are kept.
    assert!(stdout.contains("unsure refused=NotProven"), "{stdout}");
    assert!(stdout.contains("plain refused=NotProven"), "{stdout}");
    let before = num(&stdout, "before_unload", "mappings");
    assert_eq!(num(&stdout, "before_unload", "retained"), 3, "{stdout}");

    // The one that proved it is unmapped, and the list gave its entry back.
    assert_eq!(num(&stdout, "unloaded", "retained"), 2, "{stdout}");
    assert_eq!(num(&stdout, "unloaded", "unloaded_total"), 1, "{stdout}");
    assert_eq!(
        num(&stdout, "unloaded", "mappings"),
        before - 1,
        "the library really left the address space: {stdout}"
    );
    // The core and the running generation are untouched by it.
    assert_eq!(num(&stdout, "running_after_unload", "calls"), 1, "{stdout}");
    assert!(stdout.contains("core_after_unload rc=0"), "{stdout}");
}

#[test]
fn the_same_path_shares_a_mapping_and_unique_copies_do_not() {
    let stage = stage();
    let (stdout, stderr, code) = run(&stage.dir, &["dup"], &[&stage.hot_v1]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("same_path handles_equal=true"), "{stdout}");
    assert!(stdout.contains("unique_copies handles_distinct=true"), "{stdout}");
    assert_eq!(num(&stdout, "maps", "core_copies"), 1);
}

// ---- upgrade ----------------------------------------------------------------------------------------------------------

#[test]
fn a_shell_built_for_abi_1_0_runs_on_a_core_of_1_1_and_a_shell_of_1_1_degrades_on_a_core_of_1_0() {
    let stage = stage();
    let table_10 = std::mem::size_of::<abi::GnoblinApi>() as i64;
    // This test binary links the ABI crate without `next-minor`, so its table is the 1.0 table.
    assert_eq!(abi::ABI_MINOR, 0);
    let shell_10 = stage.dir.join("shells/libshell_min.so");

    let (on_10, stderr_10, code) = run(&stage.dir, &["upgrade"], &[&shell_10]);
    assert_eq!(code, 0, "{on_10}\n{stderr_10}");
    assert!(on_10.contains("core_abi=1.0"), "{on_10}");
    assert_eq!(num(&on_10, "core_abi", "table_size"), table_10);
    assert_eq!(num(&on_10, "loaded", "built_minor"), 0);
    assert_eq!(num(&on_10, "shell_stats", "calls"), 1);

    // The same shell file, untouched, on the core that has one more function.
    let shell_on_11 = stage.next_dir.join("shells/libshell_min.so");
    assert_eq!(
        fs::read(&shell_10).unwrap(),
        fs::read(&shell_on_11).unwrap(),
        "the shell must not be rebuilt for the upgrade"
    );
    assert_eq!(
        fs::read(stage.dir.join(LAUNCHER)).unwrap(),
        fs::read(stage.next_dir.join(LAUNCHER)).unwrap(),
        "nor the launcher"
    );
    let (on_11, stderr_11, code) = run(&stage.next_dir, &["upgrade"], &[&shell_on_11]);
    assert_eq!(code, 0, "{on_11}\n{stderr_11}");
    assert!(on_11.contains("core_abi=1.1"), "{on_11}");
    assert_eq!(
        num(&on_11, "core_abi", "table_size"),
        table_10 + std::mem::size_of::<usize>() as i64,
        "the table grew by one function"
    );
    assert_eq!(num(&on_11, "loaded", "built_minor"), 0);
    assert_eq!(
        num(&on_11, "shell_stats", "calls"),
        1,
        "the old shell still works: {on_11}"
    );

    // A shell built against 1.1 on a core of 1.1 sees the new function; on a core of 1.0 it degrades to `None`.
    let (built_11_on_11, _, code) = run(&stage.next_dir, &["upgrade"], &[&stage.next_shell_min]);
    assert_eq!(code, 0);
    assert_eq!(num(&built_11_on_11, "loaded", "built_minor"), 1);
    assert!(
        built_11_on_11.contains("[shell-min] uptime_ms=Some("),
        "{built_11_on_11}"
    );
    let shell_11_on_10 = stage.dir.join("shells/libshell_min_next.so");
    copy_to(&stage.next_shell_min, &shell_11_on_10);
    let (built_11_on_10, stderr, code) = run(&stage.dir, &["upgrade"], &[&shell_11_on_10]);
    assert_eq!(code, 0, "{built_11_on_10}\n{stderr}");
    assert_eq!(num(&built_11_on_10, "loaded", "built_minor"), 1);
    assert!(
        built_11_on_10.contains("[shell-min] uptime_ms=None"),
        "degrades without crashing: {built_11_on_10}"
    );
    assert_eq!(num(&built_11_on_10, "shell_stats", "calls"), 1);
}

// ---- threads ----------------------------------------------------------------------------------------------------------

#[test]
fn shell_thread_functions_refuse_another_thread_and_posted_work_runs_on_the_shell_thread() {
    let stage = stage();
    let (stdout, stderr, code) = run(&stage.dir, &["thread"], &[&stage.shell_min]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert_eq!(
        num(&stdout, "other_thread", "dispatch"),
        i64::from(abi::E_THREAD),
        "{stdout}"
    );
    assert_eq!(
        num(&stdout, "other_thread", "log"),
        0,
        "log is callable from any thread"
    );
    assert_eq!(
        num(&stdout, "other_thread", "post"),
        0,
        "post is callable from any thread"
    );
    assert!(stdout.contains("posted_ran_before_dispatch=false"), "{stdout}");
    assert_eq!(num(&stdout, "dispatch_returned", "dispatch_returned"), 1);
    assert!(stdout.contains("posted_ran=true"), "{stdout}");
    assert!(stderr.contains("hello from another thread"), "{stderr}");
}

#[test]
fn a_shell_with_a_poll_loop_of_its_own_serves_posts_from_threads_on_the_wake_descriptor_and_a_retired_poster_is_refused()
 {
    let stage = stage();
    let (stdout, stderr, code) = run(&stage.dir, &["wake"], &[&stage.shell_wake]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    // Two threads posted 40 closures; the loop slept in `poll(2)` on the descriptor, called `dispatch(0)` only when it was
    // readable, ran every one of them, and found the descriptor quiet once they had run.
    assert_eq!(num(&stdout, "[shell-wake]", "ran"), 40, "{stdout}");
    assert_eq!(num(&stdout, "[shell-wake]", "callbacks"), 40, "{stdout}");
    assert_eq!(
        field(&stdout, "[shell-wake]", "quiet").as_deref(),
        Some("true"),
        "{stdout}"
    );
    let wakes = num(&stdout, "[shell-wake]", "wakes");
    assert!(
        (1..=41).contains(&wakes),
        "a wake does not repeat once it is drained: {wakes} wakes for 40 posts\n{stdout}"
    );
    // The poster outlived the generation that made it: while the generation lived any thread could ask for a descriptor, and
    // after it retired the core refused, with the kind the SDK names.
    assert!(
        stdout.contains("[shell-wake] stale poster refused kind=Retired"),
        "{stdout}\n{stderr}"
    );
    assert!(stdout.contains("wake_done"), "{stdout}");
}

// ---- loader errors ----------------------------------------------------------------------------------------------------

#[test]
fn a_failed_init_names_the_reason() {
    let stage = stage();
    let (stdout, _, code) = run(&stage.dir, &["load"], &[&stage.hot_bad]);
    assert_eq!(code, 1);
    assert!(
        stdout.contains("init failed (E_LOAD): simulated init failure (bad shell)"),
        "{stdout}"
    );
}

#[test]
fn a_library_that_is_not_a_shell_and_a_missing_file_and_a_non_library_are_refused_with_a_reason() {
    let stage = stage();
    let core = stage.dir.join(CORE_FILE);
    let (stdout, _, code) = run(&stage.dir, &["load"], &[&core]);
    assert_eq!(code, 1);
    assert!(stdout.contains("not a gnoblin shell"), "{stdout}");
    assert!(stdout.contains("gnoshell_entry_v1"), "{stdout}");

    let missing = stage.dir.join("shells/does-not-exist.so");
    let (stdout, _, code) = run(&stage.dir, &["load"], &[&missing]);
    assert_eq!(code, 1);
    assert!(
        stdout.contains("the shell library does not exist") && stdout.contains("does-not-exist.so"),
        "{stdout}"
    );

    let text = stage.dir.join("shells/notes.txt");
    fs::write(&text, "this is not a library\n").unwrap();
    let (stdout, _, code) = run(&stage.dir, &["load"], &[&text]);
    assert_eq!(code, 1);
    assert!(
        stdout.contains("notes.txt"),
        "the loader's own text names the file: {stdout}"
    );
}

// ---- the SDK build ----------------------------------------------------------------------------------------------------

#[test]
fn the_sdk_refuses_a_build_with_panic_abort() {
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("abi-abort-target");
    let error = cargo_build(
        &target,
        &["-p", "gnoblin-fixture-shell-min"],
        &[("CARGO_PROFILE_DEV_PANIC", "abort")],
    )
    .expect_err("a shell built with panic = abort must not build");
    assert!(
        error.contains("need panic = \"unwind\""),
        "the build script says why: {error}"
    );
}
