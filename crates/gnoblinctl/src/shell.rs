//! The `shell` verbs: what the session's shell is, and how to make it reload.
//!
//! `shell list` asks no session: it runs the same discovery the launcher does and prints every shell it can see (the table
//! `gnoblin --list-shells` prints). `shell status`, `shell reload` and `shell watch` ask the compositor, which answers for the
//! shell it runs in its own process (`shell.status`, `shell.reload`): its generation and how its reloads went.
//!
//! A reload is a request the shell's thread takes between its dispatches, so `reload` returns as soon as the request is taken
//! and then reads the status until the reload has run: the exit code is the reload's own, and a failed reload prints the text
//! the shell host logged (`reload of /x/libshell.so failed, still running generation 3: <reason>`) with exit 1, the old shell
//! still serving.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use gnoblin_client::wire::ShellStatus;
use gnoblin_client::{Client, Transport};
use gnoblin_shell_discovery::{Environment, Request, discover, list_shells};
use serde_json::json;

use crate::commands::Failure;

/// How long a reload may take to run before the command gives up waiting for its outcome.
const RELOAD_PATIENCE: Duration = Duration::from_secs(60);

/// How often the outcome is looked for.
const POLL: Duration = Duration::from_millis(40);

/// How often `watch` looks at the file.
const WATCH_POLL: Duration = Duration::from_millis(150);

/// The quiet a rebuilt file must keep before `watch` reloads: a linker writes a library in several steps.
const SETTLE: Duration = Duration::from_millis(250);

/// `shell list`: the discovery table, with no session involved.
pub(crate) fn list(json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let listing = list_shells(&discovery_environment(), &Request::Discover);
    let result = if json {
        writeln!(out, "{}", json!({ "shells": listing.render() }))
    } else {
        writeln!(out, "{}", listing.render())
    };
    result.map_err(Failure::Io)?;
    Ok(0)
}

/// The facts discovery reads, with the directory launcher files' `module` keys are relative to found from where this
/// program is installed: `<prefix>/lib/gnoblin/shells`, or `lib64`.
fn discovery_environment() -> Environment {
    let mut environment = Environment::from_env();
    environment.module_dir = std::env::current_exe().ok().and_then(|executable| {
        let prefix = executable.parent()?.parent()?.to_path_buf();
        ["lib", "lib64"]
            .into_iter()
            .map(|lib| prefix.join(lib).join("gnoblin").join("shells"))
            .find(|directory| directory.is_dir())
    });
    environment
}

/// `shell status`.
pub(crate) fn status<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let status = client.shell_status().map_err(Failure::Client)?;
    print_status(&status, json, out)?;
    Ok(0)
}

fn print_status(status: &ShellStatus, json: bool, out: &mut dyn Write) -> Result<(), Failure> {
    let result = if json {
        writeln!(out, "{}", status_json(status))
    } else {
        write!(out, "{}", status.summary)
    };
    result.map_err(Failure::Io)
}

fn status_json(status: &ShellStatus) -> serde_json::Value {
    json!({
        "generation": status.generation,
        "reloads": status.reloads,
        "failures": status.failures,
        "build_failures": status.build_failures,
        "pending": status.pending,
        "last": status.last.as_ref().map(|outcome| match outcome {
            Ok(generation) => json!({"ok": true, "generation": generation}),
            Err(message) => json!({"ok": false, "error": message}),
        }),
    })
}

/// `shell reload`: asks for the reload, waits until the shell thread has run it, and reports how it went.
pub(crate) fn reload<T: Transport>(client: &mut Client<T>, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    let before = client.shell_status().map_err(Failure::Client)?;
    client.reload_shell().map_err(Failure::Client)?;
    let after = wait_for_reload(client, &before)?;
    report_reload(&after, json, out)
}

/// Reads the status until a reload has run since `before`.
fn wait_for_reload<T: Transport>(client: &mut Client<T>, before: &ShellStatus) -> Result<ShellStatus, Failure> {
    let deadline = Instant::now() + RELOAD_PATIENCE;
    loop {
        let now = client.shell_status().map_err(Failure::Client)?;
        if now.reloads + now.failures > before.reloads + before.failures {
            return Ok(now);
        }
        if Instant::now() >= deadline {
            return Err(Failure::Refusal {
                code: "reload-timeout".to_owned(),
                message:
                    "the shell did not run the reload; it may be stuck in a callback (see `gnoblinctl shell status`)"
                        .to_owned(),
            });
        }
        std::thread::sleep(POLL);
    }
}

fn report_reload(status: &ShellStatus, json: bool, out: &mut dyn Write) -> Result<u8, Failure> {
    match &status.last {
        Some(Ok(generation)) => {
            let result = if json {
                writeln!(out, "{}", json!({"reloaded": true, "generation": generation}))
            } else {
                writeln!(out, "reloaded: generation {generation}")
            };
            result.map_err(Failure::Io)?;
            Ok(0)
        }
        Some(Err(message)) => Err(Failure::Refusal {
            code: "reload-failed".to_owned(),
            message: message.clone(),
        }),
        None => Err(Failure::Refusal {
            code: "reload-unknown".to_owned(),
            message: "the shell counted a reload but says nothing of how it went".to_owned(),
        }),
    }
}

/// The file `shell watch` watches: the one named, or the module the launcher's discovery selects.
fn watched_file(path: Option<&Path>) -> Result<PathBuf, Failure> {
    let request = path.map_or(Request::Discover, |path| Request::Path(path.to_path_buf()));
    discover(&discovery_environment(), &request)
        .map(|selection| selection.path)
        .map_err(|error| Failure::Refusal {
            code: "no-shell".to_owned(),
            message: error.to_string(),
        })
}

/// When the file was last written, or `None` when it is not there (a build that has not finished leaves none).
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// `shell watch [PATH]`: reloads the running shell each time its module is rebuilt, until interrupted (or, with `once`, after
/// the first reload). The developer loop builds the library (`cargo build`, or `cargo xtask watch-shell`, which also keeps
/// the compiler's output of a failed build); this sees the library change and asks the session to load it, so the
/// compositor never runs cargo. A failed reload is printed and the watch goes on: the old shell is still serving.
pub(crate) fn watch<T: Transport>(
    client: &mut Client<T>,
    path: Option<&Path>,
    once: bool,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8, Failure> {
    let file = watched_file(path)?;
    let mut seen = modified(&file);
    let announced = if json {
        writeln!(out, "{}", json!({"watching": file.display().to_string()}))
    } else {
        writeln!(out, "watching {} (a rebuild reloads the shell)", file.display())
    };
    announced.map_err(Failure::Io)?;
    out.flush().map_err(Failure::Io)?;
    loop {
        std::thread::sleep(WATCH_POLL);
        let now = modified(&file);
        if now == seen {
            continue;
        }
        // A rebuild writes in steps: wait until the file has been quiet for a moment before loading what is there.
        std::thread::sleep(SETTLE);
        let settled = modified(&file);
        if settled != now {
            seen = now;
            continue;
        }
        seen = settled;
        if settled.is_none() {
            continue;
        }
        let code = match reload(client, json, out) {
            Ok(code) => code,
            Err(failure) => failure.report(err, json),
        };
        out.flush().map_err(Failure::Io)?;
        if once {
            return Ok(code);
        }
    }
}
