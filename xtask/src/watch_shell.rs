//! `cargo xtask watch-shell PROJECT`: rebuild a native shell when its source changes, so the compositor's `--watch` reloads it.
//!
//! The contract with a running compositor is the library file itself (`design/modular-framework.md` 8.4): the compositor watches the
//! module it loaded (`ModuleWatch`) and reloads when the file changes, so this tool needs no socket and no second trigger. A build
//! that succeeds rewrites the library and the compositor reloads it; a build that fails leaves the library as it was, so the running
//! shell stays, and the compiler's output is printed here and kept in the `ReloadController` status. The compositor never runs cargo.
//!
//! ```text
//! cargo xtask watch-shell PROJECT [--once] [--release] [--debounce MS] [-- CARGO_ARGS...]
//! ```
//!
//! `PROJECT` is a directory with a `Cargo.toml` (or the manifest itself). `--once` builds once and exits with the build's status.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gnoblin_shell_host::reload::{BuildStep, ReloadController};
use gnoblin_shell_host::watch::ConfigWatcher;

const USAGE: &str = "usage: cargo xtask watch-shell PROJECT [--once] [--release] [--debounce MS] [-- CARGO_ARGS...]";

struct WatchOptions {
    project: PathBuf,
    once: bool,
    debounce: Duration,
    cargo_args: Vec<String>,
}

fn parse(args: &[String]) -> Result<WatchOptions, String> {
    let mut project = None;
    let mut once = false;
    let mut debounce = ConfigWatcher::DEFAULT_DEBOUNCE;
    let mut cargo_args = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--once" => once = true,
            "--release" => cargo_args.push("--release".to_owned()),
            "--debounce" => {
                let ms = rest.next().ok_or("--debounce needs a number of milliseconds")?;
                debounce = Duration::from_millis(
                    ms.parse()
                        .map_err(|_| format!("`{ms}` is not a number of milliseconds"))?,
                );
            }
            "--" => {
                cargo_args.extend(rest.by_ref().cloned());
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            path if project.is_none() => project = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    let project = project.ok_or("no project given")?;
    let project = if project.file_name().is_some_and(|name| name == "Cargo.toml") {
        project.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    } else {
        project
    };
    Ok(WatchOptions {
        project,
        once,
        debounce,
        cargo_args,
    })
}

pub fn run(args: &[String]) -> ExitCode {
    let options = match parse(args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("watch-shell: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let step = BuildStep::cargo(&options.project).with_args(&options.cargo_args);
    let controller = ReloadController::new();
    if options.once {
        return if build(&step, &controller) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    println!("watch-shell: watching {} (Ctrl-C to stop)", options.project.display());
    let never = AtomicBool::new(false);
    match watch(&step, options.debounce, &controller, &never) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("watch-shell: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Whether a changed path can change the build: Rust source and the manifests.
pub fn source_relevant(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
        || path
            .file_name()
            .is_some_and(|name| name == "Cargo.toml" || name == "Cargo.lock")
}

/// The project directory and every directory under it that holds source: not `target`, not `.git`, not a hidden directory.
fn source_dirs(project: &Path) -> Vec<PathBuf> {
    let mut found = vec![project.to_path_buf()];
    let mut next = 0;
    while let Some(dir) = found.get(next).cloned() {
        next += 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                found.push(entry.path());
            }
        }
    }
    found
}

/// Builds once and says how it went. A failure's output is printed and kept in `controller`; a success names the library the
/// compositor will reload.
fn build(step: &BuildStep, controller: &ReloadController) -> bool {
    match step.run_reporting(controller) {
        Ok(built) => {
            println!(
                "watch-shell: built {} in {:.1}s; a compositor watching it reloads it",
                built.artifact.display(),
                built.took.as_secs_f64()
            );
            true
        }
        Err(failure) => {
            eprintln!("watch-shell: {failure}");
            eprint!("{}", failure.stderr);
            eprintln!("watch-shell: the library is unchanged, so the running shell stays");
            false
        }
    }
}

/// Builds, then rebuilds whenever a source file changes, until `stop` is set.
///
/// # Errors
///
/// The file watcher cannot be started.
pub fn watch(
    step: &BuildStep,
    debounce: Duration,
    controller: &ReloadController,
    stop: &AtomicBool,
) -> Result<(), String> {
    let project = step.project().to_path_buf();
    let mut watcher = ConfigWatcher::new(&project, debounce, source_relevant).map_err(|error| error.to_string())?;
    let rearm = |watcher: &mut ConfigWatcher| {
        let dirs: Vec<PathBuf> = source_dirs(&project).iter().map(|dir| dir.join("x")).collect();
        if let Err(error) = watcher.watch_modules(&dirs) {
            eprintln!("watch-shell: {error}");
        }
    };
    rearm(&mut watcher);
    build(step, controller);
    while !stop.load(Ordering::Acquire) {
        let Some(request) = watcher.next_request(Duration::from_millis(100)) else {
            continue;
        };
        if request.paths.is_empty() {
            println!("watch-shell: the watcher may have lost events; rebuilding");
        }
        for path in &request.paths {
            println!("watch-shell: changed {}", path.display());
        }
        build(step, controller);
        rearm(&mut watcher);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;

    #[test]
    fn the_options_name_a_project_a_mode_and_cargo_arguments() {
        let words = |list: &[&str]| list.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
        let options = parse(&words(&[
            "shells/mine/Cargo.toml",
            "--once",
            "--release",
            "--",
            "--features",
            "x",
        ]))
        .expect("parses");
        assert_eq!(options.project, PathBuf::from("shells/mine"));
        assert!(options.once);
        assert_eq!(options.cargo_args, ["--release", "--features", "x"]);
        assert_eq!(parse(&words(&[])).err().as_deref(), Some("no project given"));
        assert_eq!(
            parse(&words(&["a", "--bogus"])).err().as_deref(),
            Some("unknown option --bogus")
        );
    }

    #[test]
    fn only_source_and_manifests_count_and_target_is_not_walked() {
        for yes in ["/p/code/lib.rs", "/p/build.rs", "/p/Cargo.toml", "/p/Cargo.lock"] {
            assert!(source_relevant(Path::new(yes)), "{yes}");
        }
        for no in ["/p/notes.md", "/p/target/debug/libx.so", "/p/code/data.json"] {
            assert!(!source_relevant(Path::new(no)), "{no}");
        }
        let dir = tempfile::tempdir().expect("a temp dir");
        for sub in ["code/deep", "target/debug", ".git/objects"] {
            fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
        }
        let found: Vec<String> = source_dirs(dir.path())
            .iter()
            .map(|path| {
                path.strip_prefix(dir.path())
                    .expect("under the project")
                    .display()
                    .to_string()
            })
            .collect();
        assert!(
            found.contains(&"code/deep".to_owned())
                && !found
                    .iter()
                    .any(|dir| dir.starts_with("target") || dir.starts_with(".git")),
            "{found:?}"
        );
    }

    #[test]
    fn an_edit_rebuilds_a_broken_edit_leaves_the_library_and_reports_and_a_fix_rebuilds() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let project = fs::canonicalize(dir.path()).expect("exists");
        fs::create_dir(project.join("code")).expect("mkdir");
        fs::write(project.join("code/shell.rs"), "one").expect("write");
        let artifact = project.join("libshell.so");
        let script = "if grep -q BROKEN code/shell.rs; then echo 'error: broken source' >&2; exit 101; fi; cp code/shell.rs libshell.so";
        let step = BuildStep::command(&project, "sh", ["-c", script]).with_artifact(&artifact);
        let controller = ReloadController::new();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (step, controller, stop) = (step.clone(), controller.clone(), Arc::clone(&stop));
            std::thread::spawn(move || watch(&step, Duration::from_millis(200), &controller, &stop))
        };
        let wait_for = |what: &str, done: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done() {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let content = || fs::read_to_string(&artifact).unwrap_or_default();
        wait_for("the first build", &|| content() == "one");

        fs::write(project.join("code/shell.rs"), "two").expect("write");
        wait_for("the rebuild", &|| content() == "two");

        fs::write(project.join("code/shell.rs"), "BROKEN").expect("write");
        wait_for("the failure to be reported", &|| {
            controller.status().build_failures == 1
        });
        assert_eq!(
            content(),
            "two",
            "a failed build leaves the library, so the running shell stays"
        );
        assert!(
            controller
                .status()
                .last_build_failure
                .is_some_and(|failure| failure.stderr.contains("broken source")),
            "the compiler's output is kept"
        );
        assert!(
            !controller.has_request(),
            "no reload is asked for: the compositor reloads on the library's change"
        );

        fs::write(project.join("code/shell.rs"), "three").expect("write");
        wait_for("the fixed build", &|| content() == "three");
        wait_for("the failure to clear", &|| {
            controller.status().last_build_failure.is_none()
        });

        stop.store(true, Ordering::Release);
        worker.join().expect("the watcher thread ends").expect("the watch ran");
    }
}
