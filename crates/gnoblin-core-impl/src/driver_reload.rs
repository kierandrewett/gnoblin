//! The reload scenarios of the test driver: a native shell run by `NativeShell` over the fake compositor, driven by a script of steps
//! on the command line (examples 10 and 18).
//!
//! `gnoblin-fixture-launcher reload MODULE STEP...` starts the shell at `MODULE`, then performs each step in order and prints what
//! it saw as `key=value` lines the integration tests (`tests/reload.rs`) assert on. A step is a word, with an argument after a
//! colon when it takes one:
//!
//! | step            | what it does                                                                                       |
//! | --------------- | -------------------------------------------------------------------------------------------------- |
//! | `pump`          | runs the shell's turns until the tree settles                                                      |
//! | `open:ID`       | a client opens window `ID` in the compositor                                                        |
//! | `chord:TEXT`    | the user presses a chord the shell claimed                                                          |
//! | `advance:MS`    | the manual clock moves `MS` milliseconds and the shell's timers that fell due run                    |
//! | `lock`          | publishes `lock.requested`, which the reload fixture shell answers                                  |
//! | `mark:NAME`     | prints `mark NAME`, so a test can cut the output into the part before and the part after            |
//! | `copy:SRC`      | replaces the module file with `SRC`, atomically, as a finished build does                           |
//! | `truncate:N`    | cuts the module file to `N` bytes, as a build still writing it would leave it                       |
//! | `garbage`       | overwrites the module file with text that is not a library (longer than an ELF header)             |
//! | `remove`        | deletes the module file, as a failed or running build leaves it                                     |
//! | `reload`        | `NativeShell::reload` for a user's request, and what came of it                                     |
//! | `request`       | asks the controller for a reload; `serve` then takes it as the shell thread's loop does             |
//! | `build:COMMAND` | runs a build command (`sh -c COMMAND`), reporting it to the controller                              |
//! | `status`        | the controller's status                                                                             |
//! | `dump`          | the tree as the compositor holds it: windows, the shell's nodes, claims, how many nodes were made   |
//! | `limit:N`       | the retire list refuses a load once `N` libraries are kept                                           |
//! | `stats`         | the running generation's and every retired generation's counters, and the core's                    |
//! | `put:DEST=SRC`  | replaces the file `DEST` with `SRC`, atomically: an edit of a file a script imports, not the module  |
//! | `sleep:MS`      | waits `MS` real milliseconds, for a file watcher that runs on the clock and not on the manual one    |
//! | `writes:ID:KEY` | prints how many times a transaction set property `KEY` of node `ID` since the scene started          |
//! | `stop`          | ends the shell                                                                                       |
#![allow(unsafe_code)]

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gnoblin_shell_host::supervisor::ManualClock;
use gnoblin_testkit::compositor::FakeCompositor;
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

use crate::driver_examples::{FIRST, world};
use crate::host::{self, BuildStep, NativeShell, ReloadTrigger, RetirePolicy};

macro_rules! out {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

struct ReloadScene {
    comp: FakeCompositor,
    clock: ManualClock,
    shell: NativeShell,
    module: PathBuf,
    /// The windows that were open at some point, to say which are gone.
    windows: Vec<u64>,
}

/// Runs the scenario: `args` are the module path and the steps. Returns the process exit code.
pub(crate) fn run(args: &[String]) -> i32 {
    let Some(module) = args.first().map(PathBuf::from) else {
        out!("usage: gnoblin-fixture-launcher reload MODULE STEP...");
        return 2;
    };
    let comp = world();
    let clock = ManualClock::default();
    host::set_clock(Arc::new(clock.clone()));
    let mut scene = ReloadScene {
        comp,
        clock,
        shell: host::native_shell(&module),
        module,
        windows: vec![FIRST],
    };
    match scene.shell.start() {
        Ok(generation) => out!("started generation={generation}"),
        Err(error) => {
            out!("start_failed {error}");
            return 1;
        }
    }
    scene.pump();
    for step in &args[1..] {
        if let Err(code) = scene.step(step) {
            return code;
        }
    }
    0
}

impl ReloadScene {
    fn pump(&self) {
        for _ in 0..4 {
            self.shell.dispatch(0);
        }
    }

    fn step(&mut self, step: &str) -> Result<(), i32> {
        let (name, argument) = step.split_once(':').map_or((step, ""), |(name, rest)| (name, rest));
        match name {
            "pump" => self.pump(),
            "open" => {
                let id = number(argument)?;
                self.comp.create_window(id, &format!("Window {id}"), "app");
                self.windows.push(id);
                self.pump();
            }
            "chord" => {
                let claim = self.comp.claim_id(argument);
                self.comp.chord(claim, argument, "press");
                self.pump();
            }
            "advance" => {
                self.clock.advance(Duration::from_millis(number(argument)?));
                self.pump();
            }
            "lock" => {
                if let Err(message) = host::publish_named("lock", "requested", 0, Vec::new()) {
                    out!("publish_failed {message}");
                }
                self.pump();
            }
            "copy" => self.replace_module(Path::new(argument))?,
            "put" => {
                let (dest, source) = argument.split_once('=').ok_or_else(|| fail("put takes DEST=SRC"))?;
                replace_file(Path::new(dest), Path::new(source))?;
            }
            "truncate" => {
                let len = number(argument)?;
                let file = fs::OpenOptions::new()
                    .write(true)
                    .open(&self.module)
                    .map_err(|error| fail(&format!("truncate: {error}")))?;
                file.set_len(len).map_err(|error| fail(&format!("truncate: {error}")))?;
            }
            "garbage" => fs::write(&self.module, "this is not a shared library, it is text\n".repeat(3))
                .map_err(|error| fail(&format!("garbage: {error}")))?,
            "remove" => fs::remove_file(&self.module).map_err(|error| fail(&format!("remove: {error}")))?,
            "mark" => out!("mark {argument}"),
            "reload" => self.reload(ReloadTrigger::User),
            "request" => {
                let already = host::reload_controller().request(ReloadTrigger::User);
                out!("request already_waiting={already}");
            }
            "serve" => match self.shell.serve_requests() {
                Some(outcome) => {
                    self.print_reload(&outcome);
                    // The new generation's claims go out with its first transaction, as after `reload`.
                    self.pump();
                }
                None => out!("serve nothing_waiting=true"),
            },
            "build" => self.build(argument),
            "status" => {
                for line in host::reload_controller().status().to_string().lines() {
                    out!("status {line}");
                }
            }
            "dump" => self.dump(),
            "limit" => {
                host::set_retire_policy(RetirePolicy {
                    max_retained: usize::try_from(number(argument)?).unwrap_or(usize::MAX),
                    allow_unload: false,
                });
                out!("limit {}", host::retired_libraries());
            }
            "stats" => self.stats(),
            "sleep" => std::thread::sleep(Duration::from_millis(number(argument)?)),
            "writes" => {
                let (id, key) = argument.split_once(':').ok_or_else(|| fail("writes takes ID:KEY"))?;
                let id = number(id)?;
                let count = self
                    .comp
                    .ops()
                    .iter()
                    .filter(|op| matches!(op, TxOp::Set { node, key: set, .. } if *node == id && set == key))
                    .count();
                out!("writes id={id} key={key} count={count}");
            }
            "stop" => {
                let removed = self.shell.stop();
                out!("stopped removed={removed}");
            }
            other => {
                out!("unknown_step {other}");
                return Err(2);
            }
        }
        Ok(())
    }

    /// What a finished build does to the library: a new file is written beside it and renamed over it.
    fn replace_module(&self, source: &Path) -> Result<(), i32> {
        replace_file(&self.module, source)
    }

    fn reload(&mut self, trigger: ReloadTrigger) {
        let outcome = self.shell.reload(trigger);
        self.print_reload(&outcome);
        self.pump();
    }

    fn print_reload(&self, outcome: &Result<host::Reloaded, host::ReloadFailure>) {
        match outcome {
            Ok(done) => out!(
                "reload ok generation={} previous={} removed={} retired_generations={} stale_calls={} retained={}",
                done.generation,
                done.previous.map_or_else(|| "none".to_owned(), |id| id.to_string()),
                done.removed,
                done.retired_generations,
                done.stale_calls,
                done.retained.count
            ),
            Err(failure) => out!("reload failed {failure}"),
        }
    }

    fn build(&self, command: &str) {
        let controller = host::reload_controller();
        let step = BuildStep::command(std::env::temp_dir(), "sh", ["-c", command]).with_artifact(&self.module);
        match step.run_reporting(&controller) {
            Ok(built) => out!("build ok artifact={}", built.artifact.display()),
            Err(failure) => out!("build failed {}", failure.summary()),
        }
    }

    fn dump(&self) {
        for id in &self.windows {
            if self.comp.exists(*id) {
                out!(
                    "window id={id} x={} y={} w={} h={}",
                    self.comp.number(*id, "x"),
                    self.comp.number(*id, "y"),
                    self.comp.number(*id, "width"),
                    self.comp.number(*id, "height")
                );
            } else {
                out!("window id={id} gone");
            }
        }
        for id in self.comp.ids_of_class("canvas") {
            let name = match self.comp.value(id, "name") {
                Some(MirrorValue::Text(name)) => name,
                _ => String::new(),
            };
            out!(
                "canvas id={id} name={name} x={} y={} z={}",
                self.comp.number(id, "x"),
                self.comp.number(id, "y"),
                self.comp.number(id, "z")
            );
        }
        let ops = self.comp.ops();
        let made = |class: &str| {
            ops.iter()
                .filter(|op| matches!(op, TxOp::New { class: made, .. } if made.eq_ignore_ascii_case(class)))
                .count()
        };
        let destroyed = ops.iter().filter(|op| matches!(op, TxOp::Destroy { .. })).count();
        out!(
            "tree made_canvases={} made_windows={} destroys={destroyed} claims={:?}",
            made("canvas"),
            made("window"),
            self.comp.claimed()
        );
        out!("first_window_destroys={}", self.comp.destroys_of(FIRST));
    }

    fn stats(&self) {
        if let Some(module) = self.shell.current() {
            crate::driver::shell_stats_line("current", module);
            crate::driver::core_stats_line("core", module.ctx());
        }
        for module in self.shell.retired() {
            match module.shell_stats() {
                Some(stats) => out!(
                    "retired generation={} registered={} calls={} drops={}",
                    module.generation(),
                    stats.registered,
                    stats.calls,
                    stats.drops
                ),
                None => out!("retired generation={} unavailable", module.generation()),
            }
        }
        out!("retained={}", host::retired_libraries());
        crate::driver::maps_line();
    }
}

/// Replaces `dest` with a copy of `source` the way an editor or a build does: the new file is written beside it and renamed over it.
fn replace_file(dest: &Path, source: &Path) -> Result<(), i32> {
    let staging = dest.with_extension("new");
    fs::copy(source, &staging).map_err(|error| fail(&format!("copy {}: {error}", source.display())))?;
    fs::rename(&staging, dest).map_err(|error| fail(&format!("rename: {error}")))
}

fn number(text: &str) -> Result<u64, i32> {
    text.parse().map_err(|_| fail(&format!("`{text}` is not a number")))
}

fn fail(message: &str) -> i32 {
    out!("step_failed {message}");
    2
}
