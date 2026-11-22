//! Examples 2, 6 and 9 (`examples/README.md`), run for real: the shipped binary on a headless backend serving a native
//! shell that links `libgnoblin-std.so.1` (which links the core), with a real Wayland client that maps windows and a virtual keyboard that
//! types the chords. Until these tests existed the three were proved only against the fake compositor of the std crate's driver, which is
//! a proof of what the shell asks for and not of what a compositor does with it.
//!
//! What each asserts is what the compositor's tree holds, what the process maps and what it logs:
//!
//! - **Example 2** (`examples/02-std-prelude`): std gives every window a place on the workspace of its output (a core-only session leaves
//!   them all at the origin), a chord the shell claimed starts a terminal, and the process maps one core and one std library.
//! - **Example 6** (`examples/06-rust-layout`): a `Layout` written in Rust arranges the real windows in columns as they open and close,
//!   and a chord swaps two of them; the layout is called by the library and the shell never positions a window.
//! - **Example 9** (`examples/09-native-linked`): the shell is linked to the libraries of a prefix and nothing else. `readelf -d` and
//!   `nm -D` on the built file, `ldd` with the prefix on the library path, and `/proc/PID/maps` of the session agree: one core, one std,
//!   both files of the prefix, and the shell reports the ABI of the library it found.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use crate::support::elf::{assert_shell_dependencies, dynamic, is_core, is_std, resolved, symbols, tool_with};
use crate::support::native_shell::{
    BIN, Gnoblin, Home, KEY_ENTER, KEY_TAB, Launch, Typist, screen_size, staged_library_path, std_shells,
    wait_for_window,
};
use crate::support::{Mapper, scene};

/// A session whose compositor serves the std shell at `shell`. The shell records no run path, so the staged libraries are on the
/// loader's path.
pub(crate) fn start_std(shell: &Path) -> Gnoblin {
    let launch = Launch::compositor(&[])
        .with_arg(shell)
        .with_env("LD_LIBRARY_PATH", &staged_library_path());
    let mut session = Gnoblin::launch(Home::new(), launch);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    session
}

fn core_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with("libgnoblin_core") || name.starts_with("libgnoblin-core")
}

fn std_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with("libgnoblin_std") || name.starts_with("libgnoblin-std")
}

/// One mapping of the core and one of std, and the shell as one more.
#[track_caller]
pub(crate) fn assert_one_core_one_std(session: &Gnoblin) {
    let files = session.mapped_files();
    assert_eq!(
        files.iter().filter(|path| core_file(path)).count(),
        1,
        "one core mapping: {files:#?}"
    );
    assert_eq!(
        files.iter().filter(|path| std_file(path)).count(),
        1,
        "one std mapping: {files:#?}"
    );
}

/// Waits until the compositor holds the windows `one` and `two` at different places: the library placed the second off the first, which
/// the compositor's own window management would not (a core-only session leaves both at the origin).
#[track_caller]
fn placed_apart(session: &mut Gnoblin) -> (crate::support::Window, crate::support::Window) {
    let ipc = session.ipc();
    let mut found = None;
    session.wait_for("std places the two windows apart", |_| {
        let windows = scene(&ipc);
        let find = |id: &str| windows.iter().find(|window| window.app_id == id).cloned();
        if let (Some(one), Some(two)) = (find("one"), find("two"))
            && ((one.x - two.x).abs() > 0.5 || (one.y - two.y).abs() > 0.5)
        {
            found = Some((one, two));
        }
        found.is_some()
    });
    found.expect("the wait ended with a pair")
}

// ---- example 2 ---------------------------------------------------------------------------------------------------------------

#[test]
fn example_2_a_std_shell_places_real_windows_on_workspaces_and_starts_a_terminal_with_one_core_and_one_std_mapped() {
    let mut session = start_std(&std_shells().prelude);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("one", "One");
    mapper.map_window("two", "Two");

    // std's workspace of the output places each window the moment it joins; the compositor's own window management would leave both at
    // the origin (example 17's session shows exactly that).
    let (one, two) = placed_apart(&mut session);
    assert_ne!(one.id, two.id);
    let said = session.wait_log("the shell's watcher saw a client on a workspace", |line| {
        line.contains("is on workspace") && !line.contains("is on workspace none")
    });
    assert!(said.text.contains("client "), "{}", said.text);

    // `<Super>Return` was claimed by the shell: the compositor sends it there, and the shell starts the stand-in terminal.
    let mut typist = Typist::connect(&session.wayland());
    assert!(session.noted("terminal").is_none());
    typist.super_plus(KEY_ENTER);
    session.wait_noted("terminal");

    assert_one_core_one_std(&session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

// ---- example 6 ---------------------------------------------------------------------------------------------------------------

/// One `gnoblinctl` command against the session: its exit code, standard output and standard error.
fn ctl(session: &Gnoblin, arguments: &[&str]) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = gnoblinctl::run(
        arguments.iter().map(std::ffi::OsString::from),
        gnoblinctl::SocketConnection::at(session.ipc()),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8_lossy(&out).trim().to_owned(),
        String::from_utf8_lossy(&err).trim().to_owned(),
    )
}

/// The compositor's own identity of the window of `app_id` (the one `gnoblinctl window` takes), which is not the node id of the tree.
fn compositor_id(session: &Gnoblin, app_id: &str) -> Option<String> {
    let (code, listed, _) = ctl(session, &["window", "list"]);
    if code != 0 {
        return None;
    }
    listed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .find(|id| {
            let (code, value, _) = ctl(session, &["window", "get", id, "app_id"]);
            code == 0 && value.trim_matches('"') == app_id
        })
        .map(str::to_owned)
}

/// Gives the keyboard to the window of `app_id` (`gnoblinctl window call ID focus`) and waits until the compositor says it has it.
#[track_caller]
fn focus(session: &mut Gnoblin, app_id: &str) {
    let deadline = std::time::Instant::now() + crate::support::native_shell::PATIENCE;
    loop {
        let mut report = format!("no window of `{app_id}` is listed");
        if let Some(id) = compositor_id(session, app_id) {
            let asked = ctl(session, &["window", "call", &id, "focus"]);
            let held = ctl(session, &["window", "get", &id, "focused"]);
            if held.0 == 0 && held.1 == "true" {
                return;
            }
            report = format!("focus asked: {asked:?}; focused read: {held:?}");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the window never took the keyboard ({report})\n{}",
            session.log_text()
        );
        std::thread::sleep(crate::support::native_shell::POLL);
    }
}

/// The rectangle of the window `app_id`, as `(x, y, width, height)`.
fn rect_of(session: &Gnoblin, app_id: &str) -> Option<(f64, f64, f64, f64)> {
    scene(&session.ipc())
        .into_iter()
        .find(|window| window.app_id == app_id)
        .map(|window| (window.x, window.y, window.width, window.height))
}

/// The columns of `count` windows on a screen of `(width, height)` under the example's workspace: an outer gap of 20, an inner gap of 10,
/// and each edge floored on its own, as the library writes them.
fn columns(screen: (f64, f64), count: usize) -> Vec<(f64, f64, f64, f64)> {
    let (outer, inner) = (20.0, 10.0);
    let area_width = screen.0 - 2.0 * outer;
    let height = screen.1 - 2.0 * outer;
    let column = (area_width - inner * (count - 1) as f64) / count as f64;
    (0..count)
        .map(|index| {
            let left = outer + index as f64 * (column + inner);
            let right = left + column;
            (left.floor(), outer, right.floor() - left.floor(), height)
        })
        .collect()
}

fn within(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1.0
}

fn same(rect: (f64, f64, f64, f64), expected: (f64, f64, f64, f64)) -> bool {
    within(rect.0, expected.0) && within(rect.1, expected.1) && within(rect.2, expected.2) && within(rect.3, expected.3)
}

#[test]
fn example_6_a_rust_layout_arranges_real_windows_in_columns_as_they_open_and_close_and_a_chord_swaps_two() {
    let mut session = start_std(&std_shells().layout);
    let screen = screen_size(&session.ipc());
    let mut mapper = Mapper::connect(&session.wayland());

    mapper.map_window("a", "A");
    wait_for_window(&mut session, "one window fills the work area", "a", |window| {
        same((window.x, window.y, window.width, window.height), columns(screen, 1)[0])
    });

    mapper.map_window("b", "B");
    let expected = columns(screen, 2);
    wait_for_window(&mut session, "the first window keeps the left column", "a", |window| {
        same((window.x, window.y, window.width, window.height), expected[0])
    });
    wait_for_window(&mut session, "the second takes the right column", "b", |window| {
        same((window.x, window.y, window.width, window.height), expected[1])
    });

    mapper.map_window("c", "C");
    let expected = columns(screen, 3);
    for (app, rect) in ["a", "b", "c"].into_iter().zip(&expected) {
        wait_for_window(
            &mut session,
            &format!("{app} has its third of the screen"),
            app,
            |window| same((window.x, window.y, window.width, window.height), *rect),
        );
    }

    // `<Super>Tab` swaps the window that has the keyboard (here the last, given it) with the first, which moves both columns without the
    // layout being told how: it reads the new order of the workspace's clients.
    focus(&mut session, "c");
    let mut typist = Typist::connect(&session.wayland());
    typist.super_plus(KEY_TAB);
    wait_for_window(&mut session, "c has the first column", "c", |window| {
        same((window.x, window.y, window.width, window.height), expected[0])
    });
    wait_for_window(&mut session, "a has the last column", "a", |window| {
        same((window.x, window.y, window.width, window.height), expected[2])
    });
    wait_for_window(&mut session, "b is where it was", "b", |window| {
        same((window.x, window.y, window.width, window.height), expected[1])
    });

    // A window that closes leaves its column and the rest share the room. `b` closes (the second window the client mapped).
    mapper.close_window(1);
    let expected = columns(screen, 2);
    wait_for_window(&mut session, "c keeps the first column of two", "c", |window| {
        same((window.x, window.y, window.width, window.height), expected[0])
    });
    wait_for_window(&mut session, "a takes the last column of two", "a", |window| {
        same((window.x, window.y, window.width, window.height), expected[1])
    });
    assert!(
        rect_of(&session, "b").is_none(),
        "the closed window is gone from the tree"
    );
    assert_one_core_one_std(&session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

// ---- example 9 ---------------------------------------------------------------------------------------------------------------

/// A private prefix laid out as an install is: `bin/gnoblin`, and the libraries under their real versioned names with the SONAME link
/// and the linker name beside them. A std shell has no run path, so this tree on `LD_LIBRARY_PATH` is the only place its libraries
/// can come from.
struct Prefix {
    directory: tempfile::TempDir,
}

impl Prefix {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("gnoblin-linked-prefix-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).expect("the temporary directory resolves"))
            .expect("a prefix");
        let root = directory.path();
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::create_dir_all(root.join("lib")).expect("lib");
        fs::copy(BIN, root.join("bin/gnoblin")).expect("a copy of the binary");
        let staged = PathBuf::from(staged_library_path());
        for library in ["gnoblin-core", "gnoblin-std"] {
            let real = format!("lib{library}.so.1.0.0");
            fs::copy(staged.join(format!("lib{library}.so.1")), root.join("lib").join(&real))
                .expect("the staged library");
            symlink(&real, root.join(format!("lib/lib{library}.so.1"))).expect("the SONAME link");
            symlink(format!("lib{library}.so.1"), root.join(format!("lib/lib{library}.so"))).expect("the linker name");
        }
        Self { directory }
    }

    fn lib(&self) -> PathBuf {
        self.directory.path().join("lib")
    }
}

#[test]
fn example_9_a_native_shell_links_the_installed_libraries_and_nothing_else_and_the_session_maps_one_of_each() {
    let shell = &std_shells().linked;

    // Statically: the shell needs std and the core, defines one symbol, and carries no run path.
    assert_shell_dependencies(shell, true);
    let facts = dynamic(shell);
    assert!(facts.needed.iter().any(|name| is_std(name)), "{facts:?}");
    assert!(facts.needed.iter().any(|name| is_core(name)), "{facts:?}");
    assert_eq!(
        symbols(shell, "--defined-only").into_iter().collect::<Vec<_>>(),
        ["gnoshell_entry_v1"]
    );

    // The loader's own answer: with the prefix as the only place to look, both libraries resolve to the prefix's files, and without it
    // the loader says what is missing.
    let prefix = Prefix::new();
    let lib = fs::canonicalize(prefix.lib()).expect("the prefix's lib");
    let ldd = tool_with("ldd", &[], shell, &[("LD_LIBRARY_PATH", &lib)]);
    assert!(!ldd.contains("not found"), "{ldd}");
    for name in ["libgnoblin-std.so.1", "libgnoblin-core.so.1"] {
        let found =
            resolved(&ldd, name).unwrap_or_else(|| panic!("`{name}` is not in the shell's dependencies:\n{ldd}"));
        assert!(
            found.starts_with(&lib),
            "`{name}` resolves to {} and not into the prefix {}",
            found.display(),
            lib.display()
        );
    }
    let bare = tool_with("ldd", &[], shell, &[]);
    assert!(
        bare.lines()
            .any(|line| line.contains("libgnoblin-std.so.1") && line.contains("not found")),
        "without the prefix the std library is what is missing:\n{bare}"
    );

    // At run time: the prefix's binary, with the prefix on the library path.
    let launch = Launch::compositor(&[])
        .with_arg(shell)
        .with_binary(prefix.directory.path().join("bin/gnoblin"))
        .with_env("LD_LIBRARY_PATH", &lib.display().to_string());
    let mut session = Gnoblin::launch(Home::new(), launch);
    session.wait_serving();
    let versions = session
        .wait_log("the shell reports the libraries it found", |line| {
            line.contains("linked: built for core ABI")
        })
        .text;
    session.wait_log("the shell is running", |line| line.contains("linked: running"));
    // The ABI the shell was built for and the ABI it found: the same major, and the library's own patch level.
    assert!(
        versions.contains("installed core ABI 1.") && versions.contains("installed std ABI 1."),
        "{versions}"
    );

    // What the process maps, from outside, and what the shell saw of its own maps, from inside: both name one core and one std, in the
    // prefix.
    assert_one_core_one_std(&session);
    let files = session.mapped_files();
    for path in files.iter().filter(|path| core_file(path) || std_file(path)) {
        assert!(
            Path::new(path).starts_with(&lib),
            "{path} is not a file of the prefix {}",
            lib.display()
        );
    }
    let inside: Vec<String> = session
        .log()
        .into_iter()
        .filter_map(|line| line.text.split_once("linked: maps ").map(|(_, path)| path.to_owned()))
        .collect();
    assert_eq!(
        inside.iter().filter(|path| core_file(path)).count(),
        1,
        "the shell sees one core in its own maps: {inside:?}"
    );
    assert_eq!(
        inside.iter().filter(|path| std_file(path)).count(),
        1,
        "and one std: {inside:?}"
    );
    assert_eq!(
        inside.iter().filter(|path| path.contains("native_linked")).count(),
        1,
        "and itself once: {inside:?}"
    );

    // Windows are placed by the library the shell links.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("one", "One");
    mapper.map_window("two", "Two");
    placed_apart(&mut session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}
