//! Example 12: a library upgrade without recompiling an ABI-compatible shell (`design/modular-framework.md` 5.8, 12 and 30).
//!
//! The subject is two shell binaries that were built once, against ABI 1.0, and are stored in `tests/modular/golden/bin/` (a core-only
//! shell and a std shell; `golden.lock` records their hashes and `cargo xtask abi-check` guards them). Nothing here rebuilds them.
//! What the tests build is the other side: a copy of the installed prefix whose `libgnoblin-core.so.1` and `libgnoblin-std.so.1` are a
//! later version of themselves, made with the test-only knobs of the ABI crates:
//!
//! - `next-minor`: ABI 1.1 and library patch 1, one function appended to each table, and a different build of both libraries;
//! - `next-major`: a core (and a std library) that answers `gnoblin_abi_version` with major 2 under the file name `.so.1`.
//!
//! Each test starts the shipped `gnoblin` binary on a headless backend from a private prefix (`bin/gnoblin`, `lib/libgnoblin-*.so.*`),
//! so the libraries the process maps are the ones that prefix holds and no others, and reads what the session does: the log of the
//! shell (which says the ABI it was built for and the ABI of the core it finds), the windows the real Wayland client sees placed, and
//! `/proc/PID/maps`. The static half of the dependency validation (`readelf -d`, `nm -D`, `ldd`) is run on the golden shells and on
//! freshly built ones, so the two cannot drift apart.
//!
//! The new libraries are built into a target directory of their own (`CARGO_TARGET_TMPDIR/abi-upgrade-libs`): a build with a test
//! knob must never replace the staged libraries the other suites run. That is a second compile of the core's dependencies, so the
//! first run of this suite on a clean box takes a few minutes and a few GB.
//!
//! Needs `cargo xtask stage` first (the baseline libraries are the staged ones), as the other session suites do.

mod support;
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use support::native_shell::*;
use support::{Mapper, scene};

/// The library versions the tree builds, as `(major, minor, patch)`: the baseline, and the `next-minor` build.
const BASE: (u32, u32, u32) = (1, 0, 0);
const NEXT: (u32, u32, u32) = (1, 1, 1);

// ---- the golden shells ------------------------------------------------------------------------------------------------------

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

fn golden_dir() -> PathBuf {
    root().join("tests/modular/golden")
}

/// The stored golden shells.
struct Golden {
    core: PathBuf,
    std: PathBuf,
}

/// The golden shells, after checking they are the files `golden.lock` records (`sha256sum`, so the check does not share the
/// code that wrote the file).
fn golden() -> &'static Golden {
    static GOLDEN: LazyLock<Golden> = LazyLock::new(|| {
        let dir = golden_dir();
        let lock = fs::read_to_string(dir.join("golden.lock")).expect("tests/modular/golden/golden.lock");
        let mut recorded = Vec::new();
        for line in lock.lines().filter(|line| line.starts_with("shell ")) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 5, "a shell line of golden.lock: {line}");
            recorded.push((
                fields[1].to_owned(),
                fields[2].to_owned(),
                fields[3].to_owned(),
                fields[4].to_owned(),
            ));
        }
        assert_eq!(
            recorded.len(),
            2,
            "golden.lock records a core-only and a std shell: {lock}"
        );
        for (_, path, sha256, bytes) in &recorded {
            let file = dir.join(path);
            let sum = tool("sha256sum", &[], &file);
            assert_eq!(
                sum.split_whitespace().next(),
                Some(sha256.as_str()),
                "{} is not the binary golden.lock records: the golden shells are replaced by an owner with regenerate.sh, never edited",
                file.display()
            );
            assert_eq!(
                fs::metadata(&file).expect("the golden shell").len().to_string(),
                *bytes,
                "{}",
                file.display()
            );
        }
        let of = |kind: &str| {
            let (_, path, _, _) = recorded.iter().find(|row| row.0 == kind).expect("the kind is recorded");
            dir.join(path)
        };
        Golden {
            core: of("core-only"),
            std: of("std"),
        }
    });
    &GOLDEN
}

// ---- reading binaries --------------------------------------------------------------------------------------------------------

/// Runs a binutils (or coreutils) tool on `path` and returns its standard output. `LD_LIBRARY_PATH` is cleared so `ldd` shows what
/// the run path alone resolves.
fn tool(name: &str, args: &[&str], path: &Path) -> String {
    tool_with(name, args, path, &[])
}

fn tool_with(name: &str, args: &[&str], path: &Path, env: &[(&str, &Path)]) -> String {
    let mut command = Command::new(name);
    command.args(args).arg(path).env_remove("LD_LIBRARY_PATH");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("`{name}` is needed for these tests (binutils): {error}"));
    assert!(
        output.status.success(),
        "{name} failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// What `readelf -d` says of a file.
#[derive(Debug, Default)]
struct Dynamic {
    needed: Vec<String>,
    soname: Option<String>,
    runpath: Option<String>,
}

fn bracketed(line: &str) -> Option<String> {
    let start = line.find('[')?;
    let end = line.rfind(']')?;
    Some(line[start + 1..end].to_owned())
}

fn dynamic(path: &Path) -> Dynamic {
    let mut dynamic = Dynamic::default();
    for line in tool("readelf", &["-d", "-W"], path).lines() {
        if line.contains("(NEEDED)") {
            dynamic.needed.extend(bracketed(line));
        } else if line.contains("(SONAME)") {
            dynamic.soname = bracketed(line);
        } else if line.contains("(RUNPATH)") {
            dynamic.runpath = bracketed(line);
        }
    }
    dynamic
}

fn symbols(path: &Path, which: &str) -> BTreeSet<String> {
    tool("nm", &["-D", which], path)
        .lines()
        .filter_map(|line| line.split_whitespace().last().map(str::to_owned))
        .collect()
}

fn is_core(name: &str) -> bool {
    name == "libgnoblin-core.so.1"
}

fn is_std(name: &str) -> bool {
    name == "libgnoblin-std.so.1"
}

/// The dependency facts of a shell, core-only or std (`design/modular-framework.md` 11): it links the installed core, and std when it
/// uses std, and no other library of ours; it defines nothing but `gnoshell_entry_v1` (no copy of the compositor, the core or std);
/// every `gnoblin_` symbol it needs is one the core (or std) exports.
#[track_caller]
fn assert_shell_dependencies(shell: &Path, uses_std: bool) {
    let facts = dynamic(shell);
    assert_eq!(
        facts.needed.iter().filter(|name| is_core(name)).count(),
        1,
        "{} names the core exactly once: {facts:?}",
        shell.display()
    );
    assert_eq!(
        facts.needed.iter().filter(|name| is_std(name)).count(),
        usize::from(uses_std),
        "{} names std exactly when it uses std: {facts:?}",
        shell.display()
    );
    let ours: Vec<&String> = facts.needed.iter().filter(|name| name.contains("gnoblin")).collect();
    assert_eq!(
        ours.len(),
        1 + usize::from(uses_std),
        "no library of ours but the core (and std): {facts:?}"
    );
    assert_eq!(
        facts.runpath, None,
        "a shell carries no run path: the prefix finds its libraries"
    );
    assert_eq!(
        symbols(shell, "--defined-only"),
        BTreeSet::from(["gnoshell_entry_v1".to_owned()]),
        "{} defines only the shell entry",
        shell.display()
    );
    let core_exports: BTreeSet<String> = fs::read_to_string(root().join("crates/gnoblin-core-abi/exports.txt"))
        .expect("the core's export list")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    for name in symbols(shell, "--undefined-only")
        .into_iter()
        .filter(|name| name.starts_with("gnoblin_"))
    {
        assert!(
            core_exports.contains(&name) || (uses_std && name.starts_with("gnoblin_std_")),
            "{} needs `{name}`, which neither the core nor std exports",
            shell.display()
        );
    }
}

/// Where `ldd` says `name` resolves, with the path made canonical (`$ORIGIN/../lib` is printed as written).
fn resolved(ldd: &str, name: &str) -> Option<PathBuf> {
    ldd.lines().find_map(|line| {
        let (library, rest) = line.trim().split_once(" => ")?;
        (library == name)
            .then(|| rest.rsplit_once(" (").map_or(rest, |(path, _)| path))
            .and_then(|path| fs::canonicalize(path).ok())
    })
}

// ---- the libraries -------------------------------------------------------------------------------------------------------------

/// A built library of the next version, kept where the tests copy it from.
struct NextLibs {
    /// Core and std of ABI 1.1, patch 1.
    minor_core: PathBuf,
    minor_std: PathBuf,
    /// A core that reports major 2, and a std library that does.
    major_core: PathBuf,
    major_std: PathBuf,
    /// The core-only fixture shell of the old tests, built against ABI 1.1: it calls the function the 1.1 table appended.
    shell_next: PathBuf,
    /// The same shell built against ABI 1.0.
    shell_base: PathBuf,
    /// A freshly built core-only shell and std shell (the examples), for the dependency checks.
    fresh_core: PathBuf,
    fresh_std: PathBuf,
}

fn libs() -> &'static NextLibs {
    static LIBS: LazyLock<Result<NextLibs, String>> = LazyLock::new(build_libs);
    match &*LIBS {
        Ok(libs) => libs,
        Err(message) => panic!("cannot build the libraries of the next version: {message}"),
    }
}

fn cargo_build(target: &Path, args: &[&str]) -> Result<(), String> {
    let root = root();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "--locked", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args(args)
        .env("CARGO_TARGET_DIR", target)
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo build {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn keep(from: &Path, to: &Path) -> PathBuf {
    fs::create_dir_all(to.parent().expect("a parent")).expect("the directory");
    fs::copy(from, to).unwrap_or_else(|error| panic!("{}: {error}", from.display()));
    to.to_path_buf()
}

fn build_libs() -> Result<NextLibs, String> {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let target = tmp.join("abi-upgrade-libs");
    let out = tmp.join("abi-upgrade-out");
    let _ = fs::remove_dir_all(&out);
    let debug = target.join("debug");

    // ABI 1.1, patch 1: both libraries, and the fixture shell that uses the added function. The ABI crates' knobs are features of
    // the libraries and the SDK, never defaults.
    cargo_build(
        &target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-std-impl",
            "-p",
            "gnoblin-fixture-shell-min",
            "--features",
            "gnoblin-core-impl/next-minor,gnoblin-std-impl/next-minor,gnoblin-fixture-shell-min/next-minor",
        ],
    )?;
    let minor_core = keep(
        &debug.join("libgnoblin_core_impl.so"),
        &out.join("minor/libgnoblin_core_impl.so"),
    );
    let minor_std = keep(
        &debug.join("libgnoblin_std_impl.so"),
        &out.join("minor/libgnoblin_std_impl.so"),
    );
    let shell_next = keep(&debug.join("libshell_min.so"), &out.join("minor/libshell_min_next.so"));

    // A core that says it is major 2. Built alone: the std library must keep asking its core for major 1.
    cargo_build(
        &target,
        &["-p", "gnoblin-core-impl", "--features", "gnoblin-core-impl/next-major"],
    )?;
    let major_core = keep(
        &debug.join("libgnoblin_core_impl.so"),
        &out.join("major/libgnoblin_core_impl.so"),
    );
    // A std library that says it is major 2, over the unchanged core ABI.
    cargo_build(
        &target,
        &["-p", "gnoblin-std-impl", "--features", "gnoblin-std-impl/next-major"],
    )?;
    let major_std = keep(
        &debug.join("libgnoblin_std_impl.so"),
        &out.join("major/libgnoblin_std_impl.so"),
    );

    // Shells built against 1.0: the fixture again (the SDK of this tree, no knob) and the two examples for the dependency checks.
    let shells = tmp.join("native-shells");
    cargo_build(
        &shells,
        &[
            "-p",
            "gnoblin-fixture-shell-min",
            "-p",
            "gnoblin-example-minimal-core-shell",
            "-p",
            "gnoblin-example-std-prelude",
        ],
    )?;
    let built = shells.join("debug");
    Ok(NextLibs {
        minor_core,
        minor_std,
        major_core,
        major_std,
        shell_next,
        shell_base: keep(&built.join("libshell_min.so"), &out.join("base/libshell_min_base.so")),
        fresh_core: keep(
            &built.join("libminimal_core_shell.so"),
            &out.join("base/libminimal_core_shell.so"),
        ),
        fresh_std: keep(&built.join("libstd_prelude.so"), &out.join("base/libstd_prelude.so")),
    })
}

/// The libraries the tree stages, which are the baseline of every comparison.
fn staged(name: &str) -> PathBuf {
    let path = Path::new(BIN).parent().expect("the binary's directory").join(name);
    fs::canonicalize(&path).unwrap_or_else(|error| {
        panic!(
            "{} is missing ({error}): run `cargo xtask stage` before the suites that run the libraries",
            path.display()
        )
    })
}

/// A private prefix laid out as an install is: `bin/gnoblin` (the binary under test, copied), and the libraries under their real
/// versioned names with the SONAME link and the linker name beside them. The binary finds `../lib` through its run path, so this
/// tree is the only place its libraries can come from.
struct Install {
    directory: tempfile::TempDir,
}

impl Install {
    /// `core` and `std` are `(file, version)`; a prefix with no std is a supported one.
    fn new(core: (&Path, (u32, u32, u32)), std: Option<(&Path, (u32, u32, u32))>) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("gnoblin-abi-prefix-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).expect("the temporary directory resolves"))
            .expect("a temporary directory");
        let root = directory.path();
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::create_dir_all(root.join("lib")).expect("lib");
        fs::copy(BIN, root.join("bin/gnoblin")).expect("a copy of the binary");
        let install = |library: &str, from: &Path, version: (u32, u32, u32)| {
            let real = format!("lib{library}.so.{}.{}.{}", version.0, version.1, version.2);
            fs::copy(from, root.join("lib").join(&real)).expect("the library");
            // The SONAME link is `.so.1` whatever the file's own version says: that is the name shells record.
            symlink(&real, root.join(format!("lib/lib{library}.so.1"))).expect("the SONAME link");
            symlink(format!("lib{library}.so.1"), root.join(format!("lib/lib{library}.so"))).expect("the linker name");
        };
        install("gnoblin-core", core.0, core.1);
        if let Some((from, version)) = std {
            install("gnoblin-std", from, version);
        }
        Self { directory }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    fn binary(&self) -> PathBuf {
        self.root().join("bin/gnoblin")
    }

    fn lib(&self) -> PathBuf {
        self.root().join("lib")
    }

    /// Starts the compositor role with `shell`. The std shells carry no run path (a shell is found by its package, and an install
    /// under `/usr` has the libraries on the loader's own path), so a private prefix is put on `LD_LIBRARY_PATH` for them.
    fn start(&self, shell: &Path) -> Gnoblin {
        let launch = Launch::compositor(&[])
            .with_arg(shell)
            .with_binary(self.binary())
            .with_env("LD_LIBRARY_PATH", &self.lib().display().to_string());
        Gnoblin::launch(Home::new(), launch)
    }
}

/// The baseline prefix: the libraries this tree stages.
fn base_install() -> Install {
    Install::new(
        (&staged("libgnoblin-core.so.1"), BASE),
        Some((&staged("libgnoblin-std.so.1"), BASE)),
    )
}

/// The next-minor prefix: the same binary and nothing else the same.
fn next_install() -> Install {
    let libs = libs();
    Install::new((&libs.minor_core, NEXT), Some((&libs.minor_std, NEXT)))
}

// ---- reading a session ---------------------------------------------------------------------------------------------------------

/// The files of `kind` (`core` or `std`) the process maps, by path.
fn mapped(session: &Gnoblin, kind: &str) -> Vec<String> {
    session
        .mapped_files()
        .into_iter()
        .filter(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            name.starts_with(&format!("libgnoblin-{kind}.")) || name.starts_with(&format!("libgnoblin_{kind}_"))
        })
        .collect()
}

/// Every library of ours the process maps lives in the prefix's `lib`, and the core was not found anywhere else: the proof that the
/// run path resolved to the installed prefix and to no other copy.
#[track_caller]
fn assert_libraries_come_from(session: &Gnoblin, install: &Install) {
    let lib = install.lib();
    let ours: Vec<String> = session
        .mapped_files()
        .into_iter()
        .filter(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            name.starts_with("libgnoblin")
        })
        .collect();
    assert!(!ours.is_empty(), "the process maps the core");
    for path in ours {
        assert!(
            Path::new(&path).starts_with(&lib),
            "{path} is mapped from outside the prefix's lib ({})",
            lib.display()
        );
    }
}

/// Starts nothing; waits for the golden shell to say it runs.
#[track_caller]
fn wait_running(session: &mut Gnoblin, label: &str) -> String {
    session.wait_serving();
    let line = session.wait_log(&format!("{label} logs its versions"), |line| {
        line.contains(&format!("{label}: built for"))
    });
    session.wait_log(&format!("{label} runs"), |line| {
        line.contains(&format!("{label}: running"))
    });
    line.text
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.5
}

/// Runs the core-only golden shell in `install` and returns the line it logged about versions. It places the windows of a
/// cascade, which is what shows it runs and not only that it loaded.
#[track_caller]
fn run_core_golden(install: &Install) -> String {
    let mut session = install.start(&golden().core);
    let line = wait_running(&mut session, "golden-core-shell");
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("first", "First");
    wait_for_window(&mut session, "the shell places the first window", "first", |window| {
        near(window.x, 60.0) && near(window.y, 60.0) && near(window.width, 800.0) && near(window.height, 600.0)
    });
    mapper.map_window("second", "Second");
    wait_for_window(
        &mut session,
        "the shell places the second window a step down the cascade",
        "second",
        |window| near(window.x, 100.0) && near(window.y, 100.0),
    );

    // One core, no std, and every library from the prefix.
    assert_eq!(
        mapped(&session, "core").len(),
        1,
        "one core mapping: {:#?}",
        session.mapped_files()
    );
    assert!(
        mapped(&session, "std").is_empty(),
        "a core-only shell maps no std: {:#?}",
        session.mapped_files()
    );
    assert_libraries_come_from(&session, install);
    assert!(session.exited().is_none(), "{}", session.log_text());
    line
}

/// Runs the std golden shell in `install`; returns the versions line. The windows are placed by std's library, so two windows opening
/// do not land on one spot.
#[track_caller]
fn run_std_golden(install: &Install) -> String {
    let mut session = install.start(&golden().std);
    let line = wait_running(&mut session, "golden-std-shell");
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("one", "One");
    mapper.map_window("two", "Two");
    session.wait_log("std reports both clients", |line| {
        line.contains("golden-std-shell: client") && line.contains("is on workspace")
    });
    let ipc = session.ipc();
    let deadline = Instant::now() + PATIENCE;
    loop {
        let windows = scene(&ipc);
        let find = |id: &str| windows.iter().find(|window| window.app_id == id).cloned();
        if let (Some(one), Some(two)) = (find("one"), find("two"))
            && (!near(one.x, two.x) || !near(one.y, two.y))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "std never placed the two windows apart; the tree held {windows:?}\n{}",
            session.log_text()
        );
        assert!(session.exited().is_none(), "{}", session.log_text());
        std::thread::sleep(POLL);
    }

    // The chain shell, std, core: one of each. The shell is one more mapping.
    assert_eq!(
        mapped(&session, "core").len(),
        1,
        "one core mapping: {:#?}",
        session.mapped_files()
    );
    assert_eq!(
        mapped(&session, "std").len(),
        1,
        "one std mapping: {:#?}",
        session.mapped_files()
    );
    assert_libraries_come_from(&session, install);
    assert!(session.exited().is_none(), "{}", session.log_text());
    line
}

// ---- the tests -------------------------------------------------------------------------------------------------------------------

#[test]
fn the_golden_shells_and_fresh_shells_depend_on_the_installed_libraries_and_nothing_else() {
    let golden = golden();
    let libs = libs();
    assert_shell_dependencies(&golden.core, false);
    assert_shell_dependencies(&golden.std, true);
    assert_shell_dependencies(&libs.fresh_core, false);
    assert_shell_dependencies(&libs.fresh_std, true);

    // The libraries themselves: the core needs nothing of ours, std needs the core and finds it beside itself, and both are
    // `.so.1` however their files are named, in the build that ships and in the one with a test knob.
    for (path, soname) in [
        (staged("libgnoblin-core.so.1"), "libgnoblin-core.so.1"),
        (libs.minor_core.clone(), "libgnoblin-core.so.1"),
        (libs.major_core.clone(), "libgnoblin-core.so.1"),
        (staged("libgnoblin-std.so.1"), "libgnoblin-std.so.1"),
        (libs.minor_std.clone(), "libgnoblin-std.so.1"),
        (libs.major_std.clone(), "libgnoblin-std.so.1"),
    ] {
        let facts = dynamic(&path);
        assert_eq!(facts.soname.as_deref(), Some(soname), "{}: {facts:?}", path.display());
        let ours: Vec<&String> = facts.needed.iter().filter(|name| name.contains("gnoblin")).collect();
        if soname.contains("core") {
            assert!(ours.is_empty(), "the core needs no other library of ours: {facts:?}");
        } else {
            assert_eq!(
                ours,
                [&"libgnoblin-core.so.1".to_owned()],
                "std needs the core and nothing else of ours"
            );
            assert_eq!(
                facts.runpath.as_deref(),
                Some("$ORIGIN"),
                "std finds the core beside itself"
            );
        }
    }

    // The new core exports exactly what the old one does: the added function is a table field, not an export.
    assert_eq!(
        symbols(&libs.minor_core, "--defined-only"),
        symbols(&staged("libgnoblin-core.so.1"), "--defined-only"),
        "an ABI minor adds no exported symbol"
    );
    assert_eq!(
        symbols(&libs.minor_std, "--defined-only"),
        symbols(&staged("libgnoblin-std.so.1"), "--defined-only"),
    );
}

#[test]
fn the_installed_prefix_resolves_its_libraries_through_its_own_run_path_and_no_other() {
    let base = base_install();
    let lib = base.lib();

    // The binary finds the core through its run path alone, in the prefix.
    let facts = dynamic(&base.binary());
    assert!(
        facts
            .runpath
            .as_deref()
            .is_some_and(|path| path.contains("$ORIGIN/../lib")),
        "{facts:?}"
    );
    let ldd = tool("ldd", &[], &base.binary());
    let lib = fs::canonicalize(&lib).expect("the prefix's lib");
    assert_eq!(
        resolved(&ldd, "libgnoblin-core.so.1"),
        Some(lib.join("libgnoblin-core.so.1.0.0")),
        "{ldd}"
    );
    assert!(!ldd.contains("not found"), "{ldd}");
    assert!(!ldd.contains("libgnoblin-std"), "the launcher does not link std: {ldd}");

    // Std finds the core beside itself; the std shell finds std on the prefix's library path and so the core in the prefix too.
    let ldd = tool("ldd", &[], &lib.join("libgnoblin-std.so.1"));
    assert_eq!(
        resolved(&ldd, "libgnoblin-core.so.1"),
        Some(lib.join("libgnoblin-core.so.1.0.0")),
        "{ldd}"
    );
    let ldd = tool_with("ldd", &[], &golden().std, &[("LD_LIBRARY_PATH", lib.as_path())]);
    assert_eq!(
        resolved(&ldd, "libgnoblin-core.so.1"),
        Some(lib.join("libgnoblin-core.so.1.0.0")),
        "{ldd}"
    );
    assert_eq!(
        resolved(&ldd, "libgnoblin-std.so.1"),
        Some(lib.join("libgnoblin-std.so.1.0.0")),
        "{ldd}"
    );
    assert!(!ldd.contains("not found"), "{ldd}");
    let ldd = tool_with("ldd", &[], &golden().core, &[("LD_LIBRARY_PATH", lib.as_path())]);
    assert_eq!(
        resolved(&ldd, "libgnoblin-core.so.1"),
        Some(lib.join("libgnoblin-core.so.1.0.0")),
        "{ldd}"
    );
    assert!(!ldd.contains("libgnoblin-std"), "{ldd}");
}

#[test]
fn the_golden_shells_run_on_the_libraries_of_the_tree_they_were_built_with() {
    let base = base_install();
    let line = run_core_golden(&base);
    assert!(
        line.contains("built for ABI 1.0; installed core ABI 1.0.0"),
        "the baseline reports 1.0.0: {line}"
    );
    let line = run_std_golden(&base);
    assert!(
        line.contains("built for core ABI 1.0 and std ABI 1.0; installed core ABI 1.0.0, installed std ABI 1.0.0"),
        "{line}"
    );
}

#[test]
fn example_twelve_golden_shells_built_for_abi_1_0_run_unchanged_on_libraries_of_abi_1_1() {
    let next = next_install();
    let base = base_install();

    // The new libraries really are different files: another build, installed under the version it reports.
    let digest = |path: &Path| {
        tool("sha256sum", &[], path)
            .split_whitespace()
            .next()
            .map(str::to_owned)
    };
    for name in ["libgnoblin-core.so.1", "libgnoblin-std.so.1"] {
        assert_ne!(digest(&next.lib().join(name)), digest(&base.lib().join(name)), "{name}");
    }
    assert!(next.lib().join("libgnoblin-core.so.1.1.1").exists());
    assert!(next.lib().join("libgnoblin-std.so.1.1.1").exists());
    assert!(base.lib().join("libgnoblin-core.so.1.0.0").exists());

    // The golden core-only shell: it loads, places windows, and says the core it found is 1.1.1.
    let line = run_core_golden(&next);
    assert!(
        line.contains("golden-core-shell: built for ABI 1.0; installed core ABI 1.1.1"),
        "the shell shows the marker of the new core: {line}"
    );
    // The golden std shell: the same against the new core and the new std library.
    let line = run_std_golden(&next);
    assert!(
        line.contains("built for core ABI 1.0 and std ABI 1.0; installed core ABI 1.1.1, installed std ABI 1.1.1"),
        "the shell shows the marker of the new core and the new std: {line}"
    );

    // The maps name the new files: the process runs the libraries of the new prefix, and no file of the old version is in it.
    let mut session = next.start(&golden().std);
    session.wait_serving();
    session.wait_log("the shell runs", |line| line.contains("golden-std-shell: running"));
    let cores = mapped(&session, "core");
    let stds = mapped(&session, "std");
    assert_eq!(cores.len(), 1, "{cores:?}");
    assert_eq!(stds.len(), 1, "{stds:?}");
    assert!(cores[0].ends_with("libgnoblin-core.so.1.1.1"), "{cores:?}");
    assert!(stds[0].ends_with("libgnoblin-std.so.1.1.1"), "{stds:?}");
    assert_libraries_come_from(&session, &next);
}

#[test]
fn a_shell_built_for_the_next_minor_degrades_on_the_old_core_and_uses_the_added_function_on_the_new_one() {
    let libs = libs();
    // The reverse of the upgrade: a shell built against ABI 1.1 on a core of 1.0 loads, and the function the core lacks says so.
    let base = base_install();
    let mut session = base.start(&libs.shell_next);
    session.wait_serving();
    session.wait_log("the 1.1 shell degrades on the 1.0 core", |line| {
        line.contains("[shell-min] uptime_ms=None")
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
    drop(session);

    // On the core that has it, the same file calls it.
    let next = next_install();
    let mut session = next.start(&libs.shell_next);
    session.wait_serving();
    session.wait_log("the 1.1 shell calls the added function on the 1.1 core", |line| {
        line.contains("[shell-min] uptime_ms=Some(")
    });

    // And the 1.0 build of that shell, which knows nothing of the function, runs on the 1.1 core as it did on the 1.0 one.
    drop(session);
    let mut session = next.start(&libs.shell_base);
    session.wait_serving();
    session.wait_log("the 1.0 shell loads on the 1.1 core", |line| {
        line.contains("shell-min init (core only)")
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}

/// What a refused shell leaves: the compositor runs on without it (the core's own window management places windows at the origin),
/// and a signal ends it with an exit code, not a crash.
#[track_caller]
fn assert_refused_and_alive(session: &mut Gnoblin, refusal: &str) {
    session.wait_serving();
    let line = session.wait_log("the loader refuses the shell", |line| line.contains(refusal));
    assert!(line.text.contains("incompatible ABI"), "{}", line.text);
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("alive", "Still served");
    let ipc = session.ipc();
    session.wait_for("the compositor serves a client the shell did not place", |_| {
        scene(&ipc).iter().any(|window| window.app_id == "alive")
    });
    assert!(
        session.exited().is_none(),
        "the session runs on:\n{}",
        session.log_text()
    );
    assert!(
        !session.log_text().contains("panicked"),
        "the refusal is an error, not a panic:\n{}",
        session.log_text()
    );
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(i32::try_from(session.pid()).expect("a pid")).expect("a pid"),
        rustix::process::Signal::TERM,
    )
    .expect("the signal is sent");
    let status = session
        .wait_exit(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("the compositor ends on SIGTERM:\n{}", session.log_text()));
    // The compositor has no handler for the signal in this state, so it ends by the signal that was sent. A crash would be another
    // signal (SEGV, ABRT), or the session would have been gone before the window above was served.
    assert!(
        status.code().is_some() || status.signal() == Some(15),
        "the compositor ended other than by its own exit or the SIGTERM sent: {status:?}\n{}",
        session.log_text()
    );
}

#[test]
fn a_core_of_another_major_refuses_the_golden_shells_naming_both_versions_and_does_not_crash() {
    let libs = libs();
    // A core that answers major 2, under the name the shells record. The std library of the baseline stays beside it.
    let install = Install::new((&libs.major_core, (2, 0, 0)), None);

    let mut session = install.start(&golden().core);
    assert_refused_and_alive(
        &mut session,
        "this shell was built for gnoblin ABI 1.0; the installed core is 2.0",
    );
    drop(session);

    // The std golden shell is refused by the same core before it can reach std.
    let install = Install::new(
        (&libs.major_core, (2, 0, 0)),
        Some((&staged("libgnoblin-std.so.1"), BASE)),
    );
    let mut session = install.start(&golden().std);
    assert_refused_and_alive(
        &mut session,
        "this shell was built for gnoblin ABI 1.0; the installed core is 2.0",
    );
}

#[test]
fn a_std_library_of_another_major_refuses_the_std_golden_shell_naming_both_versions_and_does_not_crash() {
    let libs = libs();
    let install = Install::new(
        (&staged("libgnoblin-core.so.1"), BASE),
        Some((&libs.major_std, (2, 0, 0))),
    );
    let mut session = install.start(&golden().std);
    assert_refused_and_alive(
        &mut session,
        "this shell was built for gnoblin std ABI 1.0; the installed libgnoblin-std is 2.0",
    );
}
