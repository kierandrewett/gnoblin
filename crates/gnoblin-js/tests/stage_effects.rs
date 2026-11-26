//! What a stage of a generation leaves behind, and what a reload does with the programs the config's top level spawns.
//!
//! A reload stages the new generation by running the user's entry against the live shell, and decides afterwards whether
//! it takes over. A program the entry spawns at its top level is a side effect of that run: a stage that fails must not
//! have started it, and a reload that starts the same program again must not leave two of it. The programs are told apart
//! by an argument of their own (`sleep 31.<n>`), read from `/proc`, so each test sees only its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gnoblin_js::{JsRole, JsRoleLinks};

mod common;
use common::{FakeCompositor, honest_role_config, native};

/// The pids of the processes running `sleep <tag>`.
fn running(tag: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for entry in fs::read_dir("/proc").expect("/proc") {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(line) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let words: Vec<&[u8]> = line.split(|byte| *byte == 0).filter(|word| !word.is_empty()).collect();
        if words == [b"sleep".as_slice(), tag.as_bytes()] {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    pids
}

/// Waits until `tag` has `count` processes, or fails.
#[track_caller]
fn wait_for_count(tag: &str, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while running(tag).len() != count {
        assert!(
            Instant::now() < deadline,
            "{count} processes of {tag} never ran: {:?}",
            running(tag)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An entry that spawns `sleep <tag>` at its top level and keeps the handle where a test can read it.
fn spawning(tag: &str, rest: &str) -> String {
    format!(
        "import {{ gnoblin }} from \"gnoblin-core\";\n(globalThis as any).job = gnoblin.spawn([\"sleep\", \"{tag}\"]);\n{rest}\n"
    )
}

/// A role over the shared fake compositor, whose `shell.ts` is `init`.
fn role_of(init: &str) -> (JsRole, PathBuf, (tempfile::TempDir, native::NativeCore)) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), init).expect("shell.ts");
    let mut config = honest_role_config(&root);
    config.watch = false;
    // The first generation is staged and takes over once the entry loaded, as the loader starts a shell: the programs the entry
    // spawns are the config's, which a reload can adopt or stop.
    let comp = FakeCompositor::new();
    let mut core = native::NativeCore::staged(&comp, std::sync::Arc::clone(&config.shell.clock));
    let role = core.role(config, JsRoleLinks::default());
    (role, root, (dir, core))
}

fn rewrite(root: &Path, init: &str) {
    fs::write(root.join("shell.ts"), init).expect("shell.ts");
}

#[test]
fn a_stage_that_spawns_at_its_top_level_and_then_throws_starts_nothing() {
    let tag = "31.7001";
    let (mut role, root, mut keep) = role_of("export {};\n");

    rewrite(&root, &spawning(tag, "throw new Error(\"late\");"));
    let report = keep.1.reload(&mut role);
    assert!(!report.ok, "a file that throws is refused: {report:?}");
    assert!(
        running(tag).is_empty(),
        "the refused generation spawned nothing: {:?}",
        running(tag)
    );
}

#[test]
fn a_first_load_that_spawns_at_its_top_level_starts_one_process_and_its_handle_has_the_pid() {
    let tag = "31.7002";
    let (mut role, _root, _dir) = role_of(&spawning(tag, ""));
    wait_for_count(tag, 1);
    let pid = running(tag)[0];
    assert_eq!(
        role.shell_mut()
            .eval("String(globalThis.job.pid)")
            .expect("the script runs")
            .as_deref(),
        Some(pid.to_string().as_str()),
        "a handle made while the config loaded reads the pid of the program that started when it took over"
    );
    assert_eq!(
        role.shell_mut()
            .eval("String(globalThis.job.alive)")
            .expect("the script runs")
            .as_deref(),
        Some("true")
    );
}

#[test]
fn a_reload_whose_top_level_spawns_the_same_program_keeps_the_one_process() {
    let tag = "31.7003";
    let (mut role, root, mut keep) = role_of(&spawning(tag, ""));
    wait_for_count(tag, 1);
    let first = running(tag);

    rewrite(&root, &spawning(tag, "// saved again"));
    let report = keep.1.reload(&mut role);
    assert!(report.ok, "{report:?}");
    // Give a second process every chance to appear before looking.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(running(tag), first, "the program was not started a second time");
    assert_eq!(
        role.shell_mut()
            .eval("String(globalThis.job.pid)")
            .expect("the script runs")
            .as_deref(),
        Some(first[0].to_string().as_str()),
        "the new generation's handle is the process that was already running"
    );
}

#[test]
fn a_reload_that_no_longer_spawns_a_program_stops_it() {
    let tag = "31.7004";
    let (mut role, root, mut keep) = role_of(&spawning(tag, ""));
    wait_for_count(tag, 1);

    rewrite(&root, "export {};\n");
    let report = keep.1.reload(&mut role);
    assert!(report.ok, "{report:?}");
    wait_for_count(tag, 0);
}

#[test]
fn a_reload_that_spawns_another_program_replaces_the_first() {
    let (first, second) = ("31.7005", "31.7006");
    let (mut role, root, mut keep) = role_of(&spawning(first, ""));
    wait_for_count(first, 1);

    rewrite(&root, &spawning(second, ""));
    let report = keep.1.reload(&mut role);
    assert!(report.ok, "{report:?}");
    wait_for_count(first, 0);
    wait_for_count(second, 1);
}

#[test]
fn a_refused_reload_leaves_the_running_config_s_program_alone() {
    let tag = "31.7007";
    let (mut role, root, mut keep) = role_of(&spawning(tag, ""));
    wait_for_count(tag, 1);
    let before = running(tag);

    rewrite(&root, &spawning(tag, "throw new Error(\"late\");"));
    let report = keep.1.reload(&mut role);
    assert!(!report.ok, "{report:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        running(tag),
        before,
        "the refused reload neither doubled nor stopped it"
    );
}
