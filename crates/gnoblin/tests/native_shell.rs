//! `gnoblin ./libshell.so`: a native shell run by the compositor, in its process (`design/modular-framework.md`, M3.3 and M3.4).
//!
//! The session is the shipped binary on a headless backend, started with the path of a shell module. The compositor role is
//! `libgnoblin-core.so.1`'s (`gnoblin_main`), which loads the module on the in-process shell thread. Windows are mapped by a real
//! `wayland-client` connection, keys are typed through `zwp_virtual_keyboard_v1`, and what each test asserts is what the
//! compositor's tree holds (read through observer connections) and what the process maps and logs, so it is what a user of
//! the session would see and not what the shell believes.
//!
//! The two examples of section 12 run for real here: example 1 places the windows of a cascade and starts a terminal on a chord
//! it claimed, example 8 tiles the windows master-and-stack and keeps tiling as they come and go. The process holds one core
//! mapping, which is what the in-process default promises.

mod support;
use support::native_shell::*;
use support::{Mapper, Watcher};

/// Whether `path` names the core library file (the build tree's `libgnoblin_core_impl.so`, or an installed
/// `libgnoblin-core.so.1`).
fn is_core(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with("libgnoblin_core") || name.starts_with("libgnoblin-core")
}

#[test]
fn a_native_shell_runs_in_the_compositor_process_with_one_core_mapping_and_places_real_windows() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().minimal, &[], &[]);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });

    // One core, and the shell as one more mapping: the shell links the very core the compositor runs in, so the process holds
    // exactly one copy of it. No std library is mapped (the shell is core-only), and the launcher maps no second compositor.
    let files = session.mapped_files();
    let cores: Vec<&String> = files.iter().filter(|path| is_core(path)).collect();
    assert_eq!(cores.len(), 1, "one mapping of the core, found {cores:?} in {files:#?}");
    let shells: Vec<&String> = files.iter().filter(|path| path.contains("/gen-")).collect();
    assert_eq!(shells.len(), 1, "one mapping of the shell, found {shells:?}");
    assert!(
        !files
            .iter()
            .any(|path| path.contains("gnoblin-std") || path.contains("gnoblin_std")),
        "a core-only shell maps no std library: {files:#?}"
    );

    // The windows are placed by the shell, in the cascade of example 1: it is the shell that decides here, not the compositor's
    // own window management.
    let _watcher = Watcher::attach(&session.ipc());
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("first", "First");
    let first = wait_for_window(&mut session, "the shell places the first window", "first", |window| {
        (window.x - 60.0).abs() < 0.5 && (window.y - 60.0).abs() < 0.5
    });
    assert!(
        (first.width - 800.0).abs() < 0.5 && (first.height - 600.0).abs() < 0.5,
        "{first:?}"
    );
    mapper.map_window("second", "Second");
    wait_for_window(
        &mut session,
        "the shell places the second window a step down the cascade",
        "second",
        |window| (window.x - 100.0).abs() < 0.5 && (window.y - 100.0).abs() < 0.5,
    );
    assert!(
        session.exited().is_none(),
        "the session runs on:\n{}",
        session.log_text()
    );
}

#[test]
fn a_chord_the_shell_claimed_reaches_it_through_the_in_process_link() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().minimal, &[], &[]);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("anchor", "Anchor");
    wait_for_window(
        &mut session,
        "the shell has seen a window, so it holds the tree",
        "anchor",
        |_| true,
    );

    // The shell claims `Super+Return` through the core (a `seat.claim` call over the link); the compositor then withholds the
    // chord from clients and sends it to the shell, which starts a terminal. The terminal is a stand-in `foot` on the PATH of
    // the session that leaves a file behind.
    let mut typist = Typist::connect(&session.wayland());
    assert!(session.noted("terminal").is_none());
    typist.super_plus(KEY_ENTER);
    session.wait_noted("terminal");
}

#[test]
fn example_eight_tiles_real_windows_and_tiles_again_as_they_come_and_go() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().tiler, &[], &[]);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    let (width, height) = screen_size(&session.ipc());
    let gap = 10.0;
    let inner_width = width - 2.0 * gap;
    let inner_height = height - 2.0 * gap;
    let near = |a: f64, b: f64| (a - b).abs() < 0.5;

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    // One window fills the screen less the gaps.
    wait_for_window(&mut session, "one window fills the screen", "a", |window| {
        near(window.x, gap)
            && near(window.y, gap)
            && near(window.width, inner_width)
            && near(window.height, inner_height)
    });

    // Two: the master takes the left half and the other the right half.
    mapper.map_window("b", "B");
    let half = (inner_width - gap) / 2.0;
    wait_for_window(&mut session, "the master takes the left half", "a", |window| {
        near(window.x, gap) && near(window.width, half) && near(window.height, inner_height)
    });
    wait_for_window(&mut session, "the other window takes the right half", "b", |window| {
        near(window.x, gap + half + gap)
            && near(window.y, gap)
            && near(window.width, half)
            && near(window.height, inner_height)
    });

    // The master closes: the one left fills the screen again, in the same way.
    mapper.close_window(0);
    wait_for_window(&mut session, "the window left fills the screen again", "b", |window| {
        near(window.x, gap) && near(window.width, inner_width) && near(window.height, inner_height)
    });
}

#[test]
fn the_guardian_given_a_path_starts_no_runtime_and_hears_the_shell_accepted_and_ready() {
    let mut session = Gnoblin::start(Start::Guardian, &shells().minimal, &[], &[]);
    session.wait_log("the session is ready", |line| line.contains("[guardian] session ready"));
    session.wait_log("the shell accepted its (empty) configuration", |line| {
        line.contains("[guardian] configuration revision 0 accepted")
    });
    let log = session.log_text();
    // The compositor is started with the shell and loads it; the services role starts as it always did; there is no runtime
    // child, which would find the tree held and a shell that is not its own.
    assert!(log.contains("started compositor ("), "{log}");
    assert!(log.contains("started services ("), "{log}");
    assert!(
        !log.contains("started runtime ("),
        "a native shell has no runtime child:\n{log}"
    );
    assert!(log.contains("serves in process"), "{log}");

    // The shell is the one that holds the tree: a window is placed by it.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("first", "First");
    wait_for_window(&mut session, "the shell places the window", "first", |window| {
        (window.x - 60.0).abs() < 0.5 && (window.y - 60.0).abs() < 0.5
    });
}

#[test]
fn a_path_that_is_not_a_file_ends_the_start_with_the_reason() {
    let missing = std::path::Path::new("/nonexistent/libshell.so");
    let output = std::process::Command::new(BIN)
        .arg(missing)
        .output()
        .expect("the binary runs");
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("cannot find the shell module"), "{message}");
    assert!(message.contains("/nonexistent/libshell.so"), "{message}");
}

#[test]
fn a_shell_path_belongs_to_the_guardian_and_the_compositor_only() {
    let shell = &shells().minimal;
    let output = std::process::Command::new(BIN)
        .args(["--role", "services"])
        .arg(shell)
        .output()
        .expect("the binary runs");
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("a shell path applies to the guardian and the compositor"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
