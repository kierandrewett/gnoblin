//! The developer loop's build step: `cargo build` for a shell project, and what to do with its failure.
//!
//! The compositor never runs a build. A developer who wants a shell rebuilt when its source changes runs this beside it
//! (`cargo xtask watch-shell`): the build writes the `.so`, and the compositor's own watcher reloads it. A failed build
//! leaves the artifact as it was, so the running shell stays; [`BuildStep::run`] says what went wrong, and
//! [`ReloadController::report_build_failure`](super::ReloadController::report_build_failure) puts the compiler's output in
//! the log and in `gnoblinctl shell status`.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A build that did not produce a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFailure {
    /// The project directory.
    pub project: PathBuf,
    /// The command line that ran, as typed.
    pub command: String,
    /// The exit code, or `None` when the build was killed by a signal or could not start.
    pub status: Option<i32>,
    /// What the build wrote to its standard error: the compiler's diagnostics, or why nothing ran.
    pub stderr: String,
}

impl BuildFailure {
    /// One line: the command, how it ended, and the first line of the compiler's complaint.
    pub fn summary(&self) -> String {
        let how = match self.status {
            Some(code) => format!("exited with {code}"),
            None => "did not finish".to_owned(),
        };
        let first = self
            .stderr
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("error"))
            .or_else(|| self.stderr.lines().map(str::trim).find(|line| !line.is_empty()))
            .unwrap_or("no output");
        format!("`{}` {how}: {first}", self.command)
    }
}

impl fmt::Display for BuildFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "build of {} failed: {}", self.project.display(), self.summary())
    }
}

impl std::error::Error for BuildFailure {}

/// A build that produced a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The `.so` the build wrote.
    pub artifact: PathBuf,
    /// How long it took.
    pub took: Duration,
}

/// One way to build a shell project.
#[derive(Clone, Debug)]
pub struct BuildStep {
    project: PathBuf,
    program: OsString,
    args: Vec<OsString>,
    artifact: Option<PathBuf>,
}

impl BuildStep {
    /// `cargo build` in `project` (a directory with a `Cargo.toml`). The artifact is the `cdylib` cargo reports.
    pub fn cargo(project: impl Into<PathBuf>) -> Self {
        Self {
            project: project.into(),
            program: "cargo".into(),
            args: vec!["build".into(), "--message-format=json-render-diagnostics".into()],
            artifact: None,
        }
    }

    /// Adds arguments to `cargo build` (`--release`, `-p name`, `--features x`).
    #[must_use]
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Any command that builds the shell in `project`, for a build that is not cargo. Its artifact is named with
    /// [`with_artifact`](Self::with_artifact).
    pub fn command<I, S>(project: impl Into<PathBuf>, program: impl Into<OsString>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            project: project.into(),
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            artifact: None,
        }
    }

    /// Names the file the build writes, for a build that does not report it.
    #[must_use]
    pub fn with_artifact(mut self, artifact: impl Into<PathBuf>) -> Self {
        self.artifact = Some(artifact.into());
        self
    }

    /// The project directory.
    pub fn project(&self) -> &Path {
        &self.project
    }

    fn command_line(&self) -> String {
        let mut line = self.program.to_string_lossy().into_owned();
        for arg in &self.args {
            line.push(' ');
            line.push_str(&arg.to_string_lossy());
        }
        line
    }

    /// Runs the build and waits for it.
    ///
    /// # Errors
    ///
    /// [`BuildFailure`]: the build could not start, exited with a failure, or finished without a shell library.
    pub fn run(&self) -> Result<Built, BuildFailure> {
        let started = Instant::now();
        let failure = |status: Option<i32>, stderr: String| BuildFailure {
            project: self.project.clone(),
            command: self.command_line(),
            status,
            stderr,
        };
        let output = Command::new(&self.program)
            .args(&self.args)
            .current_dir(&self.project)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                failure(
                    None,
                    format!("cannot run {}: {error}\n", self.program.to_string_lossy()),
                )
            })?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(failure(output.status.code(), stderr));
        }
        let artifact = match &self.artifact {
            Some(artifact) => artifact.clone(),
            None => cdylib_of(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
                failure(
                    output.status.code(),
                    format!(
                        "{stderr}the build finished but produced no shared library: does {}/Cargo.toml set \
                         `crate-type = [\"cdylib\"]` under [lib]?\n",
                        self.project.display()
                    ),
                )
            })?,
        };
        if !artifact.exists() {
            return Err(failure(
                output.status.code(),
                format!("{stderr}the build finished but {} does not exist\n", artifact.display()),
            ));
        }
        Ok(Built {
            artifact,
            took: started.elapsed(),
        })
    }

    /// Runs the build and tells `controller` how it went: a failure goes to the log and the status, a success clears the
    /// failure the status held. Asks for no reload: the compositor's watcher sees the library change.
    ///
    /// # Errors
    ///
    /// The [`BuildFailure`] that was reported.
    pub fn run_reporting(&self, controller: &super::ReloadController) -> Result<Built, BuildFailure> {
        match self.run() {
            Ok(built) => {
                controller.report_build_success();
                Ok(built)
            }
            Err(failure) => {
                controller.report_build_failure(failure.clone());
                Err(failure)
            }
        }
    }
}

/// The `cdylib` a `--message-format=json` build reported last: the `compiler-artifact` messages name each target's files.
fn cdylib_of(stdout: &str) -> Option<PathBuf> {
    let mut found = None;
    for line in stdout.lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if message.get("reason").and_then(serde_json::Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        let is_cdylib = message
            .pointer("/target/crate_types")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("cdylib")));
        if !is_cdylib {
            continue;
        }
        let file = message
            .get("filenames")
            .and_then(serde_json::Value::as_array)
            .and_then(|files| {
                files
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .find(|f| f.ends_with(".so"))
            });
        if let Some(file) = file {
            found = Some(PathBuf::from(file));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temp dir")
    }

    #[test]
    fn a_failed_build_carries_the_compilers_stderr_and_a_one_line_summary() {
        let dir = project();
        let step = BuildStep::command(
            dir.path(),
            "sh",
            [
                "-c",
                "echo 'warning: noise' >&2; echo 'error[E0425]: cannot find value `x` in this scope' >&2; exit 101",
            ],
        );
        let failure = step.run().expect_err("the build fails");
        assert_eq!(failure.status, Some(101));
        assert!(failure.stderr.contains("cannot find value `x`"), "{}", failure.stderr);
        assert_eq!(
            failure.summary(),
            "`sh -c echo 'warning: noise' >&2; echo 'error[E0425]: cannot find value `x` in this scope' >&2; exit 101` \
             exited with 101: error[E0425]: cannot find value `x` in this scope"
        );
        assert_eq!(
            failure.to_string(),
            format!("build of {} failed: {}", dir.path().display(), failure.summary())
        );
    }

    #[test]
    fn a_build_that_cannot_start_says_so() {
        let dir = project();
        let failure = BuildStep::command(dir.path(), "/nonexistent/gnoblin-build-tool", ["x"])
            .run()
            .expect_err("nothing runs");
        assert_eq!(failure.status, None);
        assert!(
            failure
                .stderr
                .starts_with("cannot run /nonexistent/gnoblin-build-tool: No such file or directory"),
            "{}",
            failure.stderr
        );
        assert!(failure.summary().contains("did not finish"), "{}", failure.summary());
    }

    #[test]
    fn a_successful_build_names_its_artifact_and_a_missing_one_is_a_failure() {
        let dir = project();
        let artifact = dir.path().join("libshell.so");
        let step = BuildStep::command(dir.path(), "sh", ["-c", "echo built > libshell.so"]).with_artifact(&artifact);
        let built = step.run().expect("the build succeeds");
        assert_eq!(built.artifact, artifact);

        let forgot =
            BuildStep::command(dir.path(), "true", Vec::<String>::new()).with_artifact(dir.path().join("none.so"));
        let failure = forgot.run().expect_err("the file is not there");
        assert!(failure.stderr.contains("none.so does not exist"), "{}", failure.stderr);
    }

    #[test]
    fn the_cdylib_is_read_from_cargos_artifact_messages() {
        let stdout = concat!(
            r#"{"reason":"compiler-artifact","target":{"crate_types":["lib"]},"filenames":["/t/libdep.rlib"]}"#,
            "\n",
            r#"{"reason":"compiler-artifact","target":{"crate_types":["cdylib"]},"filenames":["/t/debug/libmine.so"]}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
            "\nnot json at all\n",
        );
        assert_eq!(cdylib_of(stdout), Some(PathBuf::from("/t/debug/libmine.so")));
        assert_eq!(cdylib_of(r#"{"reason":"build-finished","success":true}"#), None);
    }

    #[test]
    fn a_reported_failure_reaches_the_controller_and_a_success_clears_it() {
        let dir = project();
        let controller = super::super::ReloadController::new();
        let failing = BuildStep::command(dir.path(), "sh", ["-c", "echo 'error: boom' >&2; exit 1"]);
        failing.run_reporting(&controller).expect_err("fails");
        assert_eq!(controller.status().build_failures, 1);
        assert!(!controller.has_request(), "a failed build asks for no reload");

        let artifact = dir.path().join("libok.so");
        let good = BuildStep::command(dir.path(), "sh", ["-c", "echo ok > libok.so"]).with_artifact(&artifact);
        good.run_reporting(&controller).expect("builds");
        assert_eq!(controller.status().last_build_failure, None);
    }
}
