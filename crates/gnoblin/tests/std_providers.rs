//! What a session does with `libgnoblin-std` and without it: the providers std
//! registers through `provider_register`, and the minimal behaviour the core keeps when nothing does.
//!
//! The session is the shipped binary on a headless backend. Windows are mapped by a real `wayland-client` connection and what
//! each test asserts is where the compositor says it placed them (`gnoblinctl window get`), so it is what a user would see:
//!
//! - **Example 17.** A core-only native shell that holds no tree, in a session where no std is loaded: window management is the
//!   core's `NullWm`, which places every window at the origin and has no cascade.
//! - **The stock session.** The compositor role with no shell opens `libgnoblin-std.so.1` itself and std registers its window
//!   manager: the second window cascades off the first, as the default desktop always did.
//! - **A stock session on a prefix with no std.** The same binary and core copied beside no std library: the session starts,
//!   says std is not loaded, and runs `NullWm`.
//!
//! The stock session needs the std library staged beside the core (`cargo xtask stage`, which `cargo test --workspace` does not
//! replace); a test that needs it says so when it is missing.

mod support;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use support::Mapper;
use support::native_shell::*;

/// The directory of the binary under test, where `cargo xtask stage` puts the core and std libraries.
fn stage_dir() -> PathBuf {
    Path::new(BIN)
        .parent()
        .expect("the binary has a directory")
        .to_path_buf()
}

/// The staged std library, which the stock session finds beside the core.
fn staged_std() -> PathBuf {
    let path = stage_dir().join("libgnoblin-std.so.1");
    assert!(
        path.exists(),
        "{} is missing: run `cargo xtask stage` before the suites that run the stock session",
        path.display()
    );
    path
}

/// One `gnoblinctl` command against the session; `None` while it does not answer.
fn ctl(session: &Gnoblin, arguments: &[&str]) -> Option<String> {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = gnoblinctl::run(
        arguments.iter().map(std::ffi::OsString::from),
        gnoblinctl::SocketConnection::at(session.ipc()),
        &mut out,
        &mut err,
    );
    (code == 0).then(|| String::from_utf8_lossy(&out).trim().to_owned())
}

/// Where the compositor holds the window of `app_id` once it has a size: the rectangle its window management decided, read
/// through `gnoblinctl window get`, which is the session's own account of it.
fn position_of(session: &Gnoblin, app_id: &str) -> Option<(i64, i64)> {
    let listed = ctl(session, &["window", "list"])?;
    for id in listed.lines().filter_map(|line| line.split_whitespace().next()) {
        let get = |key: &str| ctl(session, &["window", "get", id, key]);
        if get("app_id")?.trim_matches('"') != app_id {
            continue;
        }
        let number = |key: &str| get(key)?.trim().parse::<f64>().ok().map(|value| value as i64);
        return (number("width")? > 0).then_some((number("x")?, number("y")?));
    }
    None
}

/// Waits (up to [`PATIENCE`]) for the window of `app_id` to be placed, and returns where.
#[track_caller]
fn placed(session: &mut Gnoblin, app_id: &str) -> (i64, i64) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(position) = position_of(session, app_id) {
            return position;
        }
        if let Some(status) = session.exited() {
            panic!(
                "gnoblin ended ({status}) before {app_id} was placed\n{}",
                session.log_text()
            );
        }
        assert!(
            Instant::now() < deadline,
            "{app_id} was never placed\n{}",
            session.log_text()
        );
        std::thread::sleep(POLL);
    }
}

/// Maps two windows and returns where the compositor places each.
fn two_windows(session: &mut Gnoblin) -> ((i64, i64), (i64, i64)) {
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("first", "First");
    let first = placed(session, "first");
    mapper.map_window("second", "Second");
    let second = placed(session, "second");
    ((first.0, first.1), (second.0, second.1))
}

fn is_std(path: &str) -> bool {
    path.contains("gnoblin-std") || path.contains("gnoblin_std")
}

#[test]
fn example_17_a_core_only_shell_with_no_std_loaded_runs_nullwm() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().idle, &[], &[]);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });

    let (first, second) = two_windows(&mut session);
    assert_eq!(first, (0, 0), "NullWm places a window at the origin");
    assert_eq!(second, (0, 0), "and the next one there too: it has no cascade");
    assert!(
        !session.mapped_files().iter().any(|path| is_std(path)),
        "no std library is mapped: {:#?}",
        session.mapped_files()
    );
    assert!(
        session.exited().is_none(),
        "the session runs on:\n{}",
        session.log_text()
    );
}

#[test]
fn the_stock_session_opens_std_and_its_window_manager_cascades_windows() {
    staged_std();
    let mut session = Gnoblin::launch(Home::new(), Launch::compositor(&[]));
    session.wait_serving();
    session.wait_log("std registered its providers", |line| {
        line.contains("the stock desktop's policy is std's")
    });

    let (first, second) = two_windows(&mut session);
    assert_ne!(
        first,
        (0, 0),
        "the stock window manager places the first window off the origin (the second is at {second:?})\n{}",
        session.log_text()
    );
    assert_ne!(second, first, "and cascades the second off the first");
    assert!(
        session.mapped_files().iter().any(|path| is_std(path)),
        "the compositor process maps std: {:#?}",
        session.mapped_files()
    );
    assert!(
        session.exited().is_none(),
        "the session runs on:\n{}",
        session.log_text()
    );
}

#[test]
fn a_stock_session_on_a_prefix_with_no_std_says_so_and_runs_nullwm() {
    // The binary and the core, copied beside no std library; the loader's search path is empty, so nothing else has one.
    let prefix = tempfile::tempdir().expect("a prefix");
    let bin = prefix.path().join("gnoblin");
    fs::copy(BIN, &bin).expect("the binary copies");
    fs::copy(
        stage_dir().join("libgnoblin-core.so.1"),
        prefix.path().join("libgnoblin-core.so.1"),
    )
    .expect("the core copies (run `cargo xtask stage` first)");
    assert!(!prefix.path().join("libgnoblin-std.so.1").exists());

    let mut session = Gnoblin::launch(Home::new(), Launch::compositor(&[]).with_binary(bin.clone()));
    session.wait_serving();
    let said = session.wait_log("the session says std is not loaded", |line| {
        line.contains("libgnoblin-std is not loaded")
    });
    assert!(
        said.text.contains("NullWm") && said.text.contains("no-ui"),
        "it names what runs instead: {}",
        said.text
    );

    let (first, second) = two_windows(&mut session);
    assert_eq!(first, (0, 0), "NullWm places a window at the origin");
    assert_eq!(second, (0, 0), "and the next one there too");
    assert!(
        !session.mapped_files().iter().any(|path| is_std(path)),
        "no std library is mapped: {:#?}",
        session.mapped_files()
    );
}
