//! A real `gnoblin` process that runs a native shell in its compositor (`design/modular-framework.md`, M3.3 and M3.5).
//!
//! Nothing here is linked into the test. The session is the shipped binary, which links `libgnoblin-core.so.1` and serves the
//! compositor role from it, started on a headless backend with its sockets in a temporary directory; the shell is a `.so`
//! built from the workspace's own crates (the two examples and the fault fixture) and loaded by the compositor on its shell
//! thread. The test reads what the session holds through observer connections to the tree, and what the process says through
//! its log, which each line of is stamped with the time the test read it, so a duration is measured from outside the shell
//! thread, as design section 19 asks.
//!
//! The binary finds the core through its run path (`$ORIGIN`), so the core must be staged beside it: `cargo xtask stage`, or a
//! workspace build, which stages it (`crates/gnoblin-core-impl/build.rs`).
#![allow(dead_code)]

use std::fs;
use std::io::{BufRead as _, BufReader};
use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// The binary under test, built by cargo for this test.
pub const BIN: &str = env!("CARGO_BIN_EXE_gnoblin");

/// How long a test waits for a state before it fails. A passing run never waits this long.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// How often a wait looks at the state again.
pub const POLL: Duration = Duration::from_millis(25);

// ---- the shells -----------------------------------------------------------------------------------------------------

/// The native shells the tests run, built once per test binary.
pub struct Shells {
    /// Example 1: places windows in a cascade, starts a terminal on `Super+Return`, closes the focused window on `Super+q`.
    pub minimal: PathBuf,
    /// Example 8: tiles the windows master-and-stack; `Super+Return` starts a terminal.
    pub tiler: PathBuf,
    /// The fault fixture: misbehaves when its directory says so.
    pub fault: PathBuf,
    /// The smallest core-only shell: it holds no tree, places nothing and loads no std, so what decides is the compositor's own.
    pub idle: PathBuf,
    /// Calls verbs of the compositor's `gnoblin` namespace and subscribes to the session's bell, saying what came back.
    pub verbs: PathBuf,
    /// Claims `Super+Return` in the plus form and starts a terminal on it.
    pub plus: PathBuf,
    /// Example 21: follows the windows through the object model of the core and logs what it is told.
    pub model: PathBuf,
    /// Example 23: the session services of the core: a program, an idle watch, a launch that times out, the session lock.
    pub services: PathBuf,
}

/// Builds the shells into a target directory of their own, so this does not wait for the lock of the one the test run holds.
/// A shell is the SDK and its own code, so this compiles little.
pub fn shells() -> &'static Shells {
    static SHELLS: LazyLock<Result<Shells, String>> = LazyLock::new(build_shells);
    match &*SHELLS {
        Ok(shells) => shells,
        Err(message) => panic!("cannot build the native shells: {message}"),
    }
}

fn build_shells() -> Result<Shells, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native-shells");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "--locked", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args([
            "-p",
            "gnoblin-example-minimal-core-shell",
            "-p",
            "gnoblin-example-no-workspaces",
            "-p",
            "gnoblin-fixture-shell-fault",
            "-p",
            "gnoblin-fixture-shell-min",
            "-p",
            "gnoblin-fixture-shell-verbs",
            "-p",
            "gnoblin-fixture-shell-plus",
            "-p",
            "gnoblin-example-native-model",
            "-p",
            "gnoblin-example-native-services",
        ])
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build of the shells failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let debug = target.join("debug");
    Ok(Shells {
        minimal: debug.join("libminimal_core_shell.so"),
        tiler: debug.join("libno_workspaces.so"),
        fault: debug.join("libfixture_shell_fault.so"),
        idle: debug.join("libshell_min.so"),
        verbs: debug.join("libshell_verbs.so"),
        plus: debug.join("libshell_plus.so"),
        model: debug.join("libnative_model.so"),
        services: debug.join("libnative_services.so"),
    })
}

// ---- the examples that link std ----------------------------------------------------------------------------------------

/// The native examples that use `gnoblin-std`: each needs `libgnoblin-std.so.1` where the loader finds it, so a session runs
/// with the staged libraries' directory on `LD_LIBRARY_PATH` (see [`staged_library_path`]).
pub struct StdShells {
    /// Example 2: one floating workspace per output, chords that walk the workspaces.
    pub prelude: PathBuf,
    /// Example 6: a `Layout` written in Rust, equal columns.
    pub layout: PathBuf,
    /// Example 9: logs the libraries its process maps and the ABIs it was built for and finds installed.
    pub linked: PathBuf,
    /// Example 19: a custom shader on the style of every window; `<Super>w` logs the style warnings of the windows.
    pub shader_tint: PathBuf,
    /// Example 20: the typed object API of std: an entry model edited and listened to, and a focus decision; `<Super>e` types into it.
    pub object: PathBuf,
    /// Example 22: the toolkit: a text and a focused entry over the windows; Return logs what was typed.
    pub ui: PathBuf,
    /// Example 24: a view and a camera placed in the scene; the placement is read back and logged.
    pub scene: PathBuf,
    /// Example 25: a style on every window, and a transition the shell aborts; the library's rollback is read back and logged.
    pub style: PathBuf,
    /// Example 26: a workspace with the library's layout, then one of the shell's own, then the library's again with its divider moved.
    pub objects: PathBuf,
    pub frame: PathBuf,
}

/// Builds the std examples into the shells' target directory. Built once per test binary.
pub fn std_shells() -> &'static StdShells {
    static SHELLS: LazyLock<Result<StdShells, String>> = LazyLock::new(build_std_shells);
    match &*SHELLS {
        Ok(shells) => shells,
        Err(message) => panic!("cannot build the std examples: {message}"),
    }
}

fn build_std_shells() -> Result<StdShells, String> {
    let root = repo_root();
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native-shells");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "--locked", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args([
            "-p",
            "gnoblin-example-std-prelude",
            "-p",
            "gnoblin-example-rust-layout",
            "-p",
            "gnoblin-example-native-linked",
            "-p",
            "gnoblin-example-shader-tint",
            "-p",
            "gnoblin-example-native-object",
            "-p",
            "gnoblin-example-native-ui",
            "-p",
            "gnoblin-example-native-scene",
            "-p",
            "gnoblin-example-native-style",
            "-p",
            "gnoblin-example-native-objects",
            "-p",
            "gnoblin-example-native-frame",
        ])
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build of the std examples failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let debug = target.join("debug");
    Ok(StdShells {
        prelude: debug.join("libstd_prelude.so"),
        layout: debug.join("librust_layout.so"),
        linked: debug.join("libnative_linked.so"),
        shader_tint: debug.join("libshader_tint.so"),
        object: debug.join("libnative_object.so"),
        ui: debug.join("libnative_ui.so"),
        scene: debug.join("libnative_scene.so"),
        style: debug.join("libnative_style.so"),
        objects: debug.join("libnative_objects.so"),
        frame: debug.join("libnative_frame.so"),
    })
}

/// The directory of the staged libraries (`cargo xtask stage` puts `libgnoblin-core.so.1` and `libgnoblin-std.so.1` beside the
/// binary): what a std shell needs on `LD_LIBRARY_PATH`, because a shell records no run path.
pub fn staged_library_path() -> String {
    let directory = Path::new(BIN).parent().expect("the binary's directory");
    assert!(
        directory.join("libgnoblin-std.so.1").exists(),
        "{} has no libgnoblin-std.so.1: run `cargo xtask stage` before the suites that run a std shell",
        directory.display()
    );
    directory.display().to_string()
}

// ---- example 10, built as a developer builds it -----------------------------------------------------------------------

/// Example 10 (`examples/10-hot-reload`) as the libraries a reload test swaps in. One source, edited and rebuilt in a project of
/// its own, exactly as an edit-and-rebuild loop does.
pub struct Hot {
    /// As it is: `VERSION` 1, a 40 pixel cascade, a toast it makes and does not keep.
    pub v1: PathBuf,
    /// The two constants edited: `VERSION` 2 and an 80 pixel cascade.
    pub v2: PathBuf,
    /// A replacement whose `init` returns an error after it has registered a listener.
    pub bad: PathBuf,
    /// A replacement whose `init` panics.
    pub panics: PathBuf,
}

/// Builds example 10 four times from its one source. Built once per test binary.
pub fn hot() -> &'static Hot {
    static HOT: LazyLock<Result<Hot, String>> = LazyLock::new(build_hot);
    match &*HOT {
        Ok(hot) => hot,
        Err(message) => panic!("cannot build example 10: {message}"),
    }
}

fn build_hot() -> Result<Hot, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("hot-{}", std::process::id()));
    let project = scratch.join("project");
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
    fs::copy(root.join("Cargo.lock"), project.join("Cargo.lock")).map_err(|error| error.to_string())?;
    // The shells' own target directory: the SDK is compiled once for all of them.
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native-shells");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = scratch.join("out");
    fs::create_dir_all(&out).map_err(|error| error.to_string())?;

    let build = |name: &str, edited: &str| -> Result<PathBuf, String> {
        // Cargo compares modification times, so let a second pass between two writes of the one file.
        std::thread::sleep(Duration::from_millis(1100));
        fs::write(project.join("src/lib.rs"), edited).map_err(|error| error.to_string())?;
        let output = Command::new(&cargo)
            .current_dir(&project)
            .args(["build", "--offline"])
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .map_err(|error| format!("cannot run cargo: {error}"))?;
        if !output.status.success() {
            return Err(format!("{name}: {}", String::from_utf8_lossy(&output.stderr)));
        }
        let built = out.join(format!("{name}.so"));
        fs::copy(target.join("debug/libhot_reload.so"), &built).map_err(|error| error.to_string())?;
        Ok(built)
    };
    let v1 = build("v1", &source)?;
    let v2_source = source
        .replace("const VERSION: u32 = 1;", "const VERSION: u32 = 2;")
        .replace("const STEP: f64 = 40.0;", "const STEP: f64 = 80.0;");
    if v2_source == source {
        return Err("example 10 no longer has the two constants this test edits".to_owned());
    }
    let v2 = build("v2", &v2_source)?;
    let entry = "fn main(g: &Gnoblin) -> gnoblin::Result<()> {\n";
    if !source.contains(entry) {
        return Err("example 10's entry function changed shape".to_owned());
    }
    // Registers a listener first, as a real replacement would, and then fails: nothing of it may be left behind.
    let bad_source = v2_source.replace(
        entry,
        &format!("{entry}    g.bind(\"Super+x\", || {{}})?;\n    g.adopt_node(\"no-such-class\", \"x\")?;\n"),
    );
    let bad = build("bad", &bad_source)?;
    let panic_source = v2_source.replace(
        entry,
        &format!("{entry}    panic!(\"simulated panic in init (example 18)\");\n"),
    );
    let panics = build("panics", &panic_source)?;
    Ok(Hot { v1, v2, bad, panics })
}

// ---- the language runtimes ----------------------------------------------------------------------------------------------

/// A directory of the runtime libraries the loader opens by name (`libgnoblin-runtime-js.so`, `libgnoblin-runtime-python.so`),
/// for `GNOBLIN_RUNTIME_DIR`: what an installed prefix has in `<libdir>/gnoblin/`. Built once per test binary.
pub fn runtimes() -> &'static Path {
    static RUNTIMES: LazyLock<Result<PathBuf, String>> = LazyLock::new(build_runtimes);
    match &*RUNTIMES {
        Ok(directory) => directory,
        Err(message) => panic!("cannot build the language runtimes: {message}"),
    }
}

fn build_runtimes() -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native-shells");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(&root)
        .args(["build", "--locked", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args(["-p", "gnoblin-runtime-js", "-p", "gnoblin-runtime-python"])
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build of the runtimes failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("runtimes");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    for language in ["js", "python"] {
        let link = directory.join(format!("libgnoblin-runtime-{language}.so"));
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(target.join(format!("debug/libgnoblin_runtime_{language}.so")), &link)
            .map_err(|error| error.to_string())?;
    }
    Ok(directory)
}

/// The environment a script shell's session needs beyond the harness's own: where the loader finds the runtimes, where the JavaScript
/// runtime reads the stock library, and no bytecode cache, so a test reads what its own files say. The Python runtime needs nothing: its
/// package is embedded. The repository's trees stand for an installed prefix's `share/gnoblin/`.
pub fn script_environment() -> Vec<(&'static str, String)> {
    let root = repo_root();
    vec![
        ("GNOBLIN_RUNTIME_DIR", runtimes().display().to_string()),
        ("GNOBLIN_JS_STOCK_DIR", root.join("data/js").display().to_string()),
        ("GNOBLIN_JS_CACHE", "0".to_owned()),
    ]
}

/// A session that runs the script shell at `shell` in the compositor's process, as `gnoblin ./shell.ts` does. `overrides` replace
/// or add to the script environment (a stock library with no std in it, a runtime directory with no Python in it).
pub fn start_script(start: Start, shell: &Path, overrides: &[(&str, &str)]) -> Gnoblin {
    start_script_with(None, start, shell, overrides)
}

/// [`start_script`] with the launcher at `binary` when one is given.
pub fn start_script_with(binary: Option<&Path>, start: Start, shell: &Path, overrides: &[(&str, &str)]) -> Gnoblin {
    let mut launch = match start {
        Start::Compositor => Launch::compositor(&[]),
        Start::Guardian => Launch::guardian(&[]),
    };
    launch.binary = binary.map(Path::to_path_buf);
    launch.args.push(shell.into());
    launch.env = script_environment()
        .into_iter()
        .filter(|(name, _)| !overrides.iter().any(|(over, _)| over == name))
        .map(|(name, value)| (name.to_owned(), value))
        .chain(
            overrides
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        )
        .collect();
    Gnoblin::launch(Home::new(), launch)
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

// ---- the process -----------------------------------------------------------------------------------------------------

/// One line the process wrote, and when this test read it.
#[derive(Clone, Debug)]
pub struct Line {
    pub at: Instant,
    pub text: String,
}

/// What a session is started as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// `gnoblin --role compositor PATH`: the compositor alone, with the shell in its process.
    Compositor,
    /// `gnoblin PATH`: the guardian, which starts the compositor with the path and the other children without a runtime.
    Guardian,
}

/// A private message bus for a guardian session, killed on drop (as `tests/guardian.rs` does).
struct Bus {
    child: Child,
    address: String,
}

impl Bus {
    fn start() -> Bus {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon is required by the guardian tests");
        let stdout = child.stdout.take().expect("stdout is piped");
        let address = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .find(|line| line.starts_with("unix:"))
            .expect("dbus-daemon prints the address of its bus");
        Bus { child, address }
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The private directory a session runs in, and the places discovery looks in, all under it so a test never reads the
/// developer's own.
///
/// ```text
/// <root>/config   $XDG_CONFIG_HOME   (the `gnoblin` directory is `config/gnoblin`)
/// <root>/data     $XDG_DATA_HOME     (installed shells: `data/gnoblin/shells`)
/// <root>/share    $XDG_DATA_DIRS     (system shells: `share/gnoblin/shells`)
/// <root>/home     $HOME
/// <root>/fixture  $GNOBLIN_FIXTURE_DIR
/// <root>/bin      a stand-in terminal, first on the PATH
/// ```
pub struct Home {
    directory: tempfile::TempDir,
}

impl Home {
    pub fn new() -> Self {
        // Canonical, so the paths a test expects are the paths the launcher prints.
        let directory = tempfile::Builder::new()
            .prefix("gnoblin-home-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).expect("the temporary directory resolves"))
            .expect("a temporary directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).expect("the directory mode");
        for name in ["fixture", "bin", "config", "data", "share", "home"] {
            fs::create_dir(directory.path().join(name)).expect("a session directory");
        }
        Self { directory }
    }

    pub fn root(&self) -> &Path {
        self.directory.path()
    }

    /// `$XDG_CONFIG_HOME/gnoblin`.
    pub fn config(&self) -> PathBuf {
        self.root().join("config/gnoblin")
    }

    /// `$XDG_DATA_HOME/gnoblin/shells`.
    pub fn user_shells(&self) -> PathBuf {
        self.root().join("data/gnoblin/shells")
    }

    /// The one `$XDG_DATA_DIRS` entry's `gnoblin/shells`.
    pub fn system_shells(&self) -> PathBuf {
        self.root().join("share/gnoblin/shells")
    }

    /// Writes `text` to `relative` under the root, making the directories, and returns the path.
    pub fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("the directories");
        fs::write(&path, text).expect("the file");
        path
    }

    /// Copies `from` to `relative` under the root and returns the path.
    pub fn copy(&self, from: &Path, relative: &str) -> PathBuf {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("the directories");
        fs::copy(from, &path).expect("the copy");
        path
    }

    /// The environment every `gnoblin` of this home runs under: the places it looks in are the home's, the backend is
    /// headless however the machine is set up, and nothing of the developer's shell choice reaches it.
    pub fn environment(&self) -> Vec<(&'static str, PathBuf)> {
        let root = self.root();
        vec![
            ("XDG_RUNTIME_DIR", root.to_path_buf()),
            ("XDG_STATE_HOME", root.join("state")),
            ("XDG_CONFIG_HOME", root.join("config")),
            ("XDG_DATA_HOME", root.join("data")),
            ("XDG_DATA_DIRS", root.join("share")),
            ("XDG_CACHE_HOME", root.join("cache")),
            ("HOME", root.join("home")),
            ("GNOBLIN_IPC_SOCKET", root.join("ipc.sock")),
            ("GNOBLIN_FIXTURE_DIR", root.join("fixture")),
        ]
    }

    /// A `gnoblin` command for this home, ready to run to completion for its exit code and text (`--list-shells`, a refused
    /// start). Not a session: nothing waits for it to serve.
    pub fn command(&self, binary: &Path) -> Command {
        let mut command = Command::new(binary);
        for (key, value) in self.environment() {
            command.env(key, value);
        }
        command
            .env("GNOBLIN_BACKEND", "headless")
            .env("GNOBLIN_LOG", "info")
            .env("GNOBLIN_LOG_FORMAT", "plain")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("GNOBLIN_SHELL")
            .stdin(Stdio::null());
        command
    }
}

impl Default for Home {
    fn default() -> Self {
        Self::new()
    }
}

/// An installed layout of the launcher in a private prefix: `<prefix>/bin/gnoblin` (a copy of the one under test), the core
/// library linked in `<prefix>/lib`, and the places a shell can be installed: `lib/gnoblin/shells` (where a launcher's `module`
/// is relative to) and `share/gnoblin/default-shell` (the packaged default, tier 7).
///
/// The launcher finds the packaged default from where it is installed, so a test that needs to see it, or to prove there is
/// none, runs this copy instead of the binary in `target/`.
pub struct Prefix {
    directory: tempfile::TempDir,
}

impl Prefix {
    pub fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("gnoblin-prefix-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).expect("the temporary directory resolves"))
            .expect("a temporary directory");
        let root = directory.path();
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::create_dir_all(root.join("lib")).expect("lib");
        let built = Path::new(BIN).parent().expect("the binary's directory");
        fs::copy(BIN, root.join("bin/gnoblin")).expect("a copy of the binary");
        std::os::unix::fs::symlink(
            built.join("libgnoblin-core.so.1"),
            root.join("lib/libgnoblin-core.so.1"),
        )
        .expect("the core, linked: `cargo xtask stage` puts it beside the binary");
        Self { directory }
    }

    pub fn root(&self) -> &Path {
        self.directory.path()
    }

    /// The installed launcher.
    pub fn binary(&self) -> PathBuf {
        self.root().join("bin/gnoblin")
    }

    /// `<prefix>/lib/gnoblin/shells`: where a launcher file's `module` key is relative to.
    pub fn module_dir(&self) -> PathBuf {
        self.root().join("lib/gnoblin/shells")
    }

    /// `<prefix>/share/gnoblin/default-shell`: the packaged default's directory.
    pub fn default_shell_dir(&self) -> PathBuf {
        self.root().join("share/gnoblin/default-shell")
    }

    /// Installs `from` under `directory` as `name`.
    pub fn install(&self, from: &Path, directory: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(directory).expect("the install directory");
        let path = directory.join(name);
        fs::copy(from, &path).expect("the installed file");
        path
    }
}

impl Default for Prefix {
    fn default() -> Self {
        Self::new()
    }
}

/// How a session is started.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    /// The launcher to run; the one under test when unset.
    pub binary: Option<PathBuf>,
    /// Its arguments.
    pub args: Vec<std::ffi::OsString>,
    /// Whether it is the guardian (it needs a private bus and the fast numbers).
    pub guardian: bool,
    /// More environment.
    pub env: Vec<(String, String)>,
    /// Files (and what is in them) in the fixture directory before the shell loads.
    pub prepared: Vec<(String, String)>,
}

impl Launch {
    pub fn guardian(args: &[&str]) -> Self {
        Self {
            args: args.iter().map(std::ffi::OsString::from).collect(),
            guardian: true,
            ..Self::default()
        }
    }

    pub fn compositor(args: &[&str]) -> Self {
        let mut launch = Self::guardian(args);
        launch.guardian = false;
        launch
            .args
            .splice(0..0, ["--role", "compositor"].map(std::ffi::OsString::from));
        launch
    }

    #[must_use]
    pub fn with_binary(mut self, binary: PathBuf) -> Self {
        self.binary = Some(binary);
        self
    }

    #[must_use]
    pub fn with_arg(mut self, arg: impl Into<std::ffi::OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    #[must_use]
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }
}

/// A running `gnoblin`, its log and the directories it owns. Killed on drop.
pub struct Gnoblin {
    child: Option<Child>,
    home: Option<Home>,
    fixture: PathBuf,
    lines: Arc<Mutex<Vec<Line>>>,
    _bus: Option<Bus>,
}

impl Gnoblin {
    /// Starts `gnoblin` with `shell` as `start` says. `extra` is added to the environment, and `prepared` names files (and what
    /// is in them) that are in the fixture directory before the shell loads, for a command it must find at its start.
    pub fn start(start: Start, shell: &Path, extra: &[(&str, &str)], prepared: &[(&str, &str)]) -> Self {
        let mut launch = match start {
            Start::Compositor => Launch::compositor(&[]),
            Start::Guardian => Launch::guardian(&[]),
        };
        launch.args.push(shell.into());
        launch.env = extra
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        launch.prepared = prepared
            .iter()
            .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
            .collect();
        Self::launch(Home::new(), launch)
    }

    /// Starts `gnoblin` as `launch` says, in `home`.
    pub fn launch(home: Home, launch: Launch) -> Self {
        let fixture = home.root().join("fixture");
        for (name, text) in &launch.prepared {
            fs::write(fixture.join(name), text).expect("a prepared command");
        }
        // A stand-in terminal: the shells start `foot`, and the test finds out that they did.
        let foot = home.root().join("bin/foot");
        fs::write(&foot, "#!/bin/sh\n: > \"$GNOBLIN_FIXTURE_DIR/terminal\"\n").expect("the stand-in terminal");
        fs::set_permissions(&foot, fs::Permissions::from_mode(0o755)).expect("the stand-in's mode");
        let path = format!(
            "{}:{}",
            home.root().join("bin").display(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned())
        );
        let binary = launch.binary.clone().unwrap_or_else(|| PathBuf::from(BIN));
        let mut command = home.command(&binary);
        command.args(&launch.args);
        let bus = launch.guardian.then(Bus::start);
        command.env("PATH", path).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(bus) = &bus {
            command
                .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
                .env("GNOBLIN_TEST_FAST", "1");
        }
        for (key, value) in &launch.env {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("gnoblin starts");
        let lines: Arc<Mutex<Vec<Line>>> = Arc::default();
        for stream in [
            child
                .stdout
                .take()
                .map(|out| Box::new(out) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|err| Box::new(err) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let lines = Arc::clone(&lines);
            std::thread::Builder::new()
                .name("gnoblin-log".to_owned())
                .spawn(move || {
                    for text in BufReader::new(stream).lines().map_while(Result::ok) {
                        lines
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(Line {
                                at: Instant::now(),
                                text,
                            });
                    }
                })
                .expect("the log reader");
        }
        Self {
            child: Some(child),
            home: Some(home),
            fixture,
            lines,
            _bus: bus,
        }
    }

    /// The directory the session runs in.
    pub fn home(&self) -> &Home {
        self.home
            .as_ref()
            .expect("the home is held until the session is taken apart")
    }

    /// The process id of the process `gnoblin` was started as.
    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("the process is held").id()
    }

    /// The tree's socket.
    pub fn ipc(&self) -> PathBuf {
        self.home().root().join("ipc.sock")
    }

    /// The Wayland socket clients connect to.
    pub fn wayland(&self) -> PathBuf {
        self.home().root().join(gnoblin_wayland::DEFAULT_SOCKET_NAME)
    }

    /// The directory the test and the fixture shell command each other through.
    pub fn fixture(&self) -> &Path {
        &self.fixture
    }

    /// Everything the process has written so far.
    pub fn log(&self) -> Vec<Line> {
        self.lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The log as text, for a failure message.
    pub fn log_text(&self) -> String {
        self.log()
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The first line that satisfies `matches`, and when it was read.
    pub fn find(&self, matches: impl Fn(&str) -> bool) -> Option<Line> {
        self.log().into_iter().find(|line| matches(&line.text))
    }

    /// Whether the process has ended, and how.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.as_mut().and_then(|child| child.try_wait().ok().flatten())
    }

    /// Waits for the process to end, within `patience`.
    pub fn wait_exit(&mut self, patience: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + patience;
        loop {
            if let Some(status) = self.exited() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Waits until `check` holds, failing with the log if the process ends or the patience runs out.
    #[track_caller]
    pub fn wait_for(&mut self, what: &str, mut check: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if check(self) {
                return;
            }
            if let Some(status) = self.exited() {
                panic!("gnoblin ended ({status}) before: {what}\n{}", self.log_text());
            }
            assert!(Instant::now() < deadline, "never reached: {what}\n{}", self.log_text());
            std::thread::sleep(POLL);
        }
    }

    /// Waits until a line of the log satisfies `matches`, and returns it.
    #[track_caller]
    pub fn wait_log(&mut self, what: &str, matches: impl Fn(&str) -> bool) -> Line {
        self.wait_for(what, |session| session.find(&matches).is_some());
        self.find(matches).expect("the line was there")
    }

    /// Waits until the tree's socket accepts a connection, which is when the compositor serves.
    #[track_caller]
    pub fn wait_serving(&mut self) {
        let ipc = self.ipc();
        self.wait_for("the compositor serves its tree", |_| UnixStream::connect(&ipc).is_ok());
        let wayland = self.wayland();
        self.wait_for("the compositor serves Wayland", |_| wayland.exists());
    }

    /// What the fixture shell wrote to `name`, trimmed, if it has.
    pub fn noted(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.fixture.join(name))
            .ok()
            .map(|text| text.trim().to_owned())
    }

    /// Waits for the fixture shell to write `name`, and returns what it wrote.
    #[track_caller]
    pub fn wait_noted(&mut self, name: &str) -> String {
        self.wait_for(&format!("the fixture shell wrote `{name}`"), |session| {
            session.noted(name).is_some()
        });
        self.noted(name).expect("the note was there")
    }

    /// Tells the fixture shell to do `command` (`hang`, `panic`, `exit`, `fail-init`), with `text` in the file.
    pub fn command(&self, command: &str, text: &str) {
        fs::write(self.fixture.join(command), text).expect("the command file");
    }

    /// The maps of the process, as `/proc/<pid>/maps` has them: the paths of the files mapped.
    pub fn mapped_files(&self) -> Vec<String> {
        let maps = fs::read_to_string(format!("/proc/{}/maps", self.pid())).unwrap_or_default();
        let mut files: Vec<String> = maps
            .lines()
            .filter_map(|line| line.split_whitespace().nth(5))
            .filter(|path| path.starts_with('/'))
            .map(str::to_owned)
            .collect();
        files.sort();
        files.dedup();
        files
    }
}

impl Gnoblin {
    /// Ends the session, and gives back its home: the directories and files it ran in, for a session that starts over them (a
    /// setting the first login wrote and only the next one reads).
    pub fn into_home(mut self) -> Home {
        self.stop();
        self.home
            .take()
            .expect("the home is held until the session is taken apart")
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // A guardian's children are in their own process groups; ending the guardian ends them in its stop order.
            let _ = rustix::process::kill_process(
                rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap_or(i32::MAX)).expect("a pid"),
                rustix::process::Signal::TERM,
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(POLL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Gnoblin {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- reading the scene ---------------------------------------------------------------------------------------------------

/// The size of the first screen of the tree.
pub fn screen_size(ipc: &Path) -> (f64, f64) {
    let nodes = super::snapshot(ipc);
    let screen = nodes
        .iter()
        .find(|node| node.class == "screen")
        .expect("the tree has a screen");
    let value = |key: &str| {
        screen
            .props
            .iter()
            .find(|prop| prop.key == key)
            .map_or(0.0, |prop| prop.value_double)
    };
    (value("width"), value("height"))
}

/// Waits (up to [`PATIENCE`]) until the window of `app_id` is placed by `places`: a predicate over its rectangle. A failure
/// says what the tree held, which is what the shell had done by then.
#[track_caller]
pub fn wait_for_window(
    gnoblin: &mut Gnoblin,
    what: &str,
    app_id: &str,
    places: impl Fn(&super::Window) -> bool,
) -> super::Window {
    let ipc = gnoblin.ipc();
    let deadline = Instant::now() + PATIENCE;
    loop {
        let windows = super::scene(&ipc);
        if let Some(window) = windows.iter().find(|window| window.app_id == app_id && places(window)) {
            return window.clone();
        }
        if let Some(status) = gnoblin.exited() {
            panic!("gnoblin ended ({status}) before: {what}\n{}", gnoblin.log_text());
        }
        assert!(
            Instant::now() < deadline,
            "never reached: {what}\nthe tree held {windows:?} on a screen of {:?}\nnodes (id, class, parent): {:?}\n{}",
            screen_size(&ipc),
            super::snapshot(&ipc)
                .iter()
                .map(|node| (node.id, node.class.clone(), node.parent))
                .collect::<Vec<_>>(),
            gnoblin.log_text()
        );
        std::thread::sleep(POLL);
    }
}

/// Waits (up to [`PATIENCE`]) until `held` is true of the nodes of the tree, and returns the nodes it was true of. For the nodes a shell
/// made: they reach the tree after the lines the shell logs about them. The shell sends what its start wrote from its first dispatch after
/// it serves, and the compositor applies it on its own thread, so a snapshot read once after those lines misses the nodes some of the time.
/// A failure says what the tree held.
#[track_caller]
pub fn wait_for_nodes(
    gnoblin: &mut Gnoblin,
    what: &str,
    held: impl Fn(&[gnoblin_proto::tree::snapshot_node::Properties]) -> bool,
) -> Vec<gnoblin_proto::tree::snapshot_node::Properties> {
    let ipc = gnoblin.ipc();
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(status) = gnoblin.exited() {
            panic!("gnoblin ended ({status}) before: {what}\n{}", gnoblin.log_text());
        }
        let nodes = super::snapshot(&ipc);
        if held(&nodes) {
            return nodes;
        }
        assert!(
            Instant::now() < deadline,
            "never reached: {what}\nnodes (id, class, parent): {:?}\n{}",
            nodes
                .iter()
                .map(|node| (node.id, node.class.clone(), node.parent))
                .collect::<Vec<_>>(),
            gnoblin.log_text()
        );
        std::thread::sleep(POLL);
    }
}

/// The ids of the windows of the tree by their application id, for a test that asks whether they are the same windows.
pub fn scene_ids(ipc: &Path) -> std::collections::BTreeMap<String, u64> {
    super::ids(&super::scene(ipc))
}

// ---- typing --------------------------------------------------------------------------------------------------------------

/// A keymap that compiles against the system's xkb data.
const KEYMAP: &str = "xkb_keymap {\n\
    xkb_keycodes { include \"evdev+aliases(qwerty)\" };\n\
    xkb_types { include \"complete\" };\n\
    xkb_compat { include \"complete\" };\n\
    xkb_symbols { include \"pc+us+inet(evdev)\" };\n\
    xkb_geometry { include \"pc(pc105)\" };\n\
};\n";

/// evdev codes of the keys the tests press.
pub const KEY_LEFTMETA: u32 = 125;
pub const KEY_ENTER: u32 = 28;
pub const KEY_1: u32 = 2;
pub const KEY_3: u32 = 4;
pub const KEY_5: u32 = 6;
pub const KEY_Q: u32 = 16;
pub const KEY_M: u32 = 50;
pub const KEY_N: u32 = 49;
pub const KEY_R: u32 = 19;
pub const KEY_T: u32 = 20;
pub const KEY_X: u32 = 45;
pub const KEY_RIGHT: u32 = 106;
pub const KEY_DOWN: u32 = 108;
pub const KEY_E: u32 = 18;
pub const KEY_TAB: u32 = 15;
pub const KEY_A: u32 = 30;
pub const KEY_B: u32 = 48;

/// The `Mod4` bit of an xkb modifier mask, which `pc+us` gives the Super keys.
const MOD_SUPER: u32 = 1 << 6;

struct Typing;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Typing {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(Typing: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Typing: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(Typing: ignore ZwpVirtualKeyboardV1);

/// A virtual keyboard on the session's own socket: the way an on-screen keyboard types, and the one door into a session
/// that runs as a process, which has no input tap.
pub struct Typist {
    _connection: Connection,
    queue: EventQueue<Typing>,
    keyboard: ZwpVirtualKeyboardV1,
    clock: u32,
}

impl Typist {
    pub fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<Typing>(&connection).expect("the global list");
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_seat");
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises zwp_virtual_keyboard_manager_v1");
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        let mut keymap = tempfile::tempfile().expect("a file for the keymap");
        std::io::Write::write_all(&mut keymap, KEYMAP.as_bytes()).expect("the keymap is written");
        keyboard.keymap(1, keymap.as_fd(), u32::try_from(KEYMAP.len()).expect("a small keymap"));
        queue.roundtrip(&mut Typing).expect("the keymap is delivered");
        Self {
            _connection: connection,
            queue,
            keyboard,
            clock: 1000,
        }
    }

    fn key(&mut self, code: u32, pressed: bool) {
        self.clock += 10;
        self.keyboard.key(self.clock, code, u32::from(pressed));
    }

    /// Presses `key` and lets it go.
    pub fn tap(&mut self, key: u32) {
        self.key(key, true);
        self.key(key, false);
        self.queue.roundtrip(&mut Typing).expect("the keys are delivered");
    }

    /// Presses `Super` and `key` together and lets both go.
    pub fn super_plus(&mut self, key: u32) {
        self.key(KEY_LEFTMETA, true);
        self.keyboard.modifiers(MOD_SUPER, 0, 0, 0);
        self.key(key, true);
        self.key(key, false);
        self.keyboard.modifiers(0, 0, 0, 0);
        self.key(KEY_LEFTMETA, false);
        self.queue.roundtrip(&mut Typing).expect("the keys are delivered");
    }
}
