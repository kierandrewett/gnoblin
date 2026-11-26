//! A native shell started from an install prefix the dynamic loader does not search (`/usr/local`, `/opt`, a home directory).
//!
//! The launcher finds the core through its run path (`$ORIGIN/../lib`), but a shell is a library the core `dlopen`s, and the loader
//! does not use the launcher's run path for the dependencies of a `dlopen`ed library. A shell that links std records
//! `NEEDED libgnoblin-std.so.1` and no run path, so before the host opened std beside the core for it, such a shell was refused with
//! `libgnoblin-std.so.1: cannot open shared object file` unless `LD_LIBRARY_PATH` named the prefix. Each test lays out a private
//! prefix as an install does (`bin/gnoblin`, `lib/libgnoblin-core.so.1`, and `lib/libgnoblin-std.so.1` when std is installed),
//! starts the copy of the launcher in it with no `LD_LIBRARY_PATH` (the `Home` of these tests clears it), and asserts what the
//! session does and what the process maps:
//!
//! - a std shell runs, on the std library of the prefix;
//! - a shell that needs the core only runs and maps no std, even when std is installed;
//! - a std shell on a prefix with no std is refused with a message that says std is missing and where it looked, and the
//!   compositor runs on.
//!
//! Needs `cargo xtask stage` first (the libraries copied are the staged ones), as the other session suites do.

mod support;
use std::fs;
use std::path::{Path, PathBuf};

use support::native_shell::*;

/// A prefix laid out as an install is, with real files in `lib` (not links into the build tree, so nothing resolves outside it).
struct Install {
    directory: tempfile::TempDir,
}

impl Install {
    fn new(with_std: bool) -> Self {
        let directory = tempfile::Builder::new()
            .prefix("gnoblin-shell-prefix-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).expect("the temporary directory resolves"))
            .expect("a temporary directory");
        let root = directory.path();
        fs::create_dir_all(root.join("bin")).expect("bin");
        fs::create_dir_all(root.join("lib")).expect("lib");
        fs::copy(BIN, root.join("bin/gnoblin")).expect("a copy of the binary");
        let built = Path::new(BIN).parent().expect("the binary's directory");
        let mut names = vec!["libgnoblin-core.so.1"];
        if with_std {
            names.push("libgnoblin-std.so.1");
        }
        for name in names {
            fs::copy(built.join(name), root.join("lib").join(name))
                .unwrap_or_else(|error| panic!("{name}: {error} (run `cargo xtask stage` first)"));
        }
        Self { directory }
    }

    fn lib(&self) -> PathBuf {
        self.directory.path().join("lib")
    }

    /// Starts the compositor role of the installed launcher with `shell`, which lives outside the prefix as a user's does
    /// (`~/.config/gnoblin/shell.so`). Nothing names the prefix to the loader.
    fn start(&self, shell: &Path) -> Gnoblin {
        let launch = Launch::compositor(&[])
            .with_arg(shell)
            .with_binary(self.directory.path().join("bin/gnoblin"));
        Gnoblin::launch(Home::new(), launch)
    }
}

/// The files of ours (`libgnoblin-*`) the process maps, by path.
fn ours(session: &Gnoblin) -> Vec<String> {
    session
        .mapped_files()
        .into_iter()
        .filter(|path| path.rsplit('/').next().unwrap_or(path).starts_with("libgnoblin"))
        .collect()
}

fn is_std(path: &str) -> bool {
    path.rsplit('/').next().unwrap_or(path).starts_with("libgnoblin-std")
}

fn wait_serving_shell(session: &mut Gnoblin) {
    session.wait_serving();
    session.wait_log("the shell serves", |line| {
        line.contains("the shell ") && line.contains(" serves ")
    });
}

/// Whether the dynamic loader of this machine finds std on its own, as it does on a machine with gnoblin-std installed under
/// `/usr`: the case of a prefix with no std cannot be made there.
fn loader_finds_std() -> bool {
    std::process::Command::new("ldconfig")
        .arg("-p")
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains("libgnoblin-std.so.1"))
}

#[test]
fn a_std_shell_runs_from_a_prefix_the_loader_does_not_search_on_the_std_beside_the_core() {
    let install = Install::new(true);
    let mut session = install.start(&std_shells().prelude);
    wait_serving_shell(&mut session);

    let mapped = ours(&session);
    let stds: Vec<&String> = mapped.iter().filter(|path| is_std(path)).collect();
    assert_eq!(stds.len(), 1, "one std mapping: {mapped:#?}");
    for path in &mapped {
        assert!(
            Path::new(path).starts_with(install.lib()),
            "{path} is mapped from outside the prefix's lib ({})",
            install.lib().display()
        );
    }
    assert!(session.exited().is_none(), "{}", session.log_text());
    assert!(
        session.log_text().contains("opened for a shell that links std"),
        "the host says it opened std for the shell:\n{}",
        session.log_text()
    );
}

#[test]
fn a_core_only_shell_runs_from_the_same_prefix_and_maps_no_std_though_std_is_installed() {
    let install = Install::new(true);
    assert!(install.lib().join("libgnoblin-std.so.1").exists());
    let mut session = install.start(&shells().minimal);
    wait_serving_shell(&mut session);

    let mapped = ours(&session);
    assert!(!mapped.is_empty(), "the process maps the core");
    assert!(
        !mapped.iter().any(|path| is_std(path)),
        "a core-only shell maps no std: {mapped:#?}"
    );
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn a_std_shell_on_a_prefix_with_no_std_is_refused_by_name_and_the_compositor_runs_on() {
    if loader_finds_std() {
        eprintln!(
            "skipped: this machine's loader finds libgnoblin-std.so.1 itself, so there is no way to leave std out"
        );
        return;
    }
    let install = Install::new(false);
    let mut session = install.start(&std_shells().prelude);
    session.wait_serving();
    let refusal = session.wait_log("the host refuses the shell and says std is missing", |line| {
        line.contains("this shell uses gnoblin-std")
    });
    assert!(
        refusal.text.contains("`libgnoblin-std.so.1` is not installed")
            && refusal.text.contains(&install.lib().display().to_string())
            && refusal.text.contains("Install gnoblin-std"),
        "the refusal names the library, where it looked and what to do: {}",
        refusal.text
    );
    assert!(
        !refusal.text.contains("cannot open shared object file"),
        "the loader's words are replaced by an explanation: {}",
        refusal.text
    );
    assert!(
        !ours(&session).iter().any(|path| is_std(path)),
        "no std is mapped: {:#?}",
        session.mapped_files()
    );
    assert!(
        session.exited().is_none(),
        "the session runs on without the shell:\n{}",
        session.log_text()
    );
}
