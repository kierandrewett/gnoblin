//! The scenario driver behind `gnoblin_test_main`, compiled only with the `test-driver` feature.
//!
//! The integration tests (`tests/scenarios.rs`) need to load real shell modules into a real process and look at the result:
//! one core mapping, a contained panic, a swap without stale calls, an upgrade of the core under a shell that was not
//! rebuilt. The core is a `cdylib` that a launcher links, so a test cannot call into it as Rust; instead it runs the
//! `gnoblin-fixture-launcher` binary, which links the stub and calls this one extra export. Each scenario is one process
//! so its statics start fresh, and each prints `key=value` lines the tests assert on.
//!
//! This is the proof spike's scenario set (`design/modular-framework-spike.md`, results 3 to 6) as product code.
#![allow(unsafe_code)]

use core::ffi::{CStr, c_char, c_int, c_void};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use gnoblin_core_abi::{self as abi, GnoblinStats};
use gnoblin_log::ProcessRole;
use gnoblin_log::init::{Console, LogOptions, LogSettings, init};

use crate::guard::{guard, last_error};
use crate::host::{self, Copying, Module, OwnedValue, RetirePolicy};
use crate::table::API;

macro_rules! out {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

/// Runs one scenario. `argv[1]` names it and the rest are its arguments; returns the process exit code.
///
/// # Safety
///
/// `argv` must point at `argc` NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_test_main(argc: c_int, argv: *const *const c_char) -> c_int {
    guard(101, "gnoblin_test_main", || {
        let args: Vec<String> = (0..usize::try_from(argc).unwrap_or(0))
            // SAFETY: the caller guarantees `argv` holds `argc` NUL-terminated strings.
            .map(|index| {
                unsafe { CStr::from_ptr(*argv.add(index)) }
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let settings = LogSettings {
            console: Console::Plain,
            level: "info".to_owned(),
        };
        let _log = init(ProcessRole::Compositor, &LogOptions::from_environment(&settings));
        if args.get(1).map(String::as_str) == Some("reload") {
            return crate::driver_reload::run(&args[2..]);
        }
        let arg = |index: usize| args.get(index).map(PathBuf::from);
        match (args.get(1).map(String::as_str), arg(2), arg(3), arg(4)) {
            (Some("run"), Some(shell), _, _) => run(&shell),
            (Some("panic"), Some(shell), _, _) => panic_scenario(&shell),
            (Some("hot"), Some(v1), Some(v2), bad) => hot(&v1, &v2, bad.as_deref()),
            (Some("abandon"), Some(v1), Some(v2), _) => abandon(&v1, &v2),
            (Some("dup"), Some(shell), _, _) => dup(&shell),
            (Some("upgrade"), Some(shell), _, _) => upgrade(&shell),
            (Some("thread"), Some(shell), _, _) => threads(&shell),
            (Some("load"), Some(shell), _, _) => load_only(&shell),
            (Some("wake"), Some(shell), _, _) => wake(&shell),
            (Some("retain"), Some(shell), limit, _) => retain(&shell, limit.as_deref()),
            (Some("unload"), Some(safe), Some(unsure), Some(plain)) => unload(&safe, &unsure, &plain),
            (Some("example01"), Some(shell), _, _) => crate::driver_examples::minimal(&shell),
            (Some("example02"), Some(shell), _, _) => crate::driver_examples::std_prelude(&shell),
            (Some("example02_spec"), Some(shell), _, _) => crate::driver_examples::spec_text(&shell),
            (Some("example06"), Some(shell), _, _) => crate::driver_examples::rust_layout(&shell),
            (Some("example08"), Some(shell), _, _) => crate::driver_examples::tiler(&shell),
            (Some("std_layouts"), Some(shell), _, _) => crate::driver_examples::std_layouts(&shell),
            (Some("std_bindings"), Some(shell), _, _) => crate::driver_examples::std_bindings(&shell),
            (Some("std_script"), Some(shell), _, _) => crate::driver_script::run(&shell),
            (Some("std_anim"), Some(shell), _, _) => crate::driver_anim::std_anim(&shell),
            _ => {
                out!(
                    "usage: gnoblin-fixture-launcher <run|panic|hot|abandon|dup|upgrade|thread|load|retain|unload|reload|example01|example02|example06|example08|std_layouts|std_bindings> MODULE..."
                );
                2
            }
        }
    })
}

fn pump(module: &Module, rounds: u32) {
    for _ in 0..rounds {
        module.dispatch(20);
    }
}

fn publish(class: &str, signal: &str, values: Vec<OwnedValue>) {
    if let Err(message) = host::publish_named(class, signal, 0, values) {
        out!("publish failed: {message}");
    }
}

pub(crate) fn shell_stats_line(prefix: &str, module: &Module) {
    match module.shell_stats() {
        Some(stats) => out!(
            "{prefix} registered={} calls={} drops={}",
            stats.registered,
            stats.calls,
            stats.drops
        ),
        None => out!("{prefix} unavailable"),
    }
}

pub(crate) fn core_stats_line(prefix: &str, ctx: abi::GnoblinCtx) {
    let mut stats = GnoblinStats {
        size: size_of::<GnoblinStats>() as u32,
        reserved: 0,
        registrations: 0,
        callbacks: 0,
        panics: 0,
        retired_generations: 0,
        stale_calls: 0,
    };
    // SAFETY: `stats` fills the struct it is given, a live local.
    let rc = unsafe { (API.stats)(ctx, &raw mut stats) };
    out!(
        "{prefix} rc={rc} registrations={} callbacks={} panics={} retired={} stale={}",
        stats.registrations,
        stats.callbacks,
        stats.panics,
        stats.retired_generations,
        stats.stale_calls
    );
}

/// The core and shell mappings of this process, read from `/proc/self/maps`.
pub(crate) fn maps_line() {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
    let mut cores = std::collections::BTreeSet::new();
    let mut shells = std::collections::BTreeSet::new();
    for line in maps.lines() {
        let Some(path) = line.split_whitespace().nth(5) else {
            continue;
        };
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.starts_with("libgnoblin-core") || name.starts_with("libgnoblin_core") {
            cores.insert(path.to_owned());
        }
        if path.contains("/gen-") || name.starts_with("gen-") {
            shells.insert(path.to_owned());
        }
    }
    // One library maps in several segments, one line each: count the files.
    let std_copies: std::collections::BTreeSet<&str> = maps
        .lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .filter(|path| {
            path.rsplit('/')
                .next()
                .is_some_and(|name| name.contains("gnoblin-std") || name.contains("gnoblin_std"))
        })
        .collect();
    out!(
        "maps core_copies={} shell_mappings={} std_mappings={}",
        cores.len(),
        shells.len(),
        std_copies.len()
    );
    for core in &cores {
        out!("maps core_path={core}");
    }
}

fn load_or_report(path: &std::path::Path, copying: Copying, reload_count: u32) -> Result<Module, i32> {
    match host::load(path, copying, reload_count) {
        Ok(module) => {
            out!(
                "loaded label={} generation={} built_minor={}",
                module.label(),
                module.generation(),
                module.built_minor()
            );
            Ok(module)
        }
        Err(error) => {
            out!("load_failed {error}");
            Err(1)
        }
    }
}

/// Spike 3: load a shell, deliver an event, and show that there is one core mapping.
fn run(shell: &std::path::Path) -> i32 {
    let module = match load_or_report(shell, Copying::Unique, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    if let Err(message) = module.activate() {
        out!("activate_failed {message}");
        return 1;
    }
    publish("lock", "requested", vec![]);
    pump(&module, 3);
    maps_line();
    shell_stats_line("shell_stats", &module);
    core_stats_line("core_stats", module.ctx());
    0
}

/// Load only: print what loading says. For a module that is expected to fail.
fn load_only(shell: &std::path::Path) -> i32 {
    match load_or_report(shell, Copying::Unique, 0) {
        Ok(module) => {
            module.retire();
            0
        }
        Err(code) => code,
    }
}

/// A shell with an event loop of its own (`wake_fd`): load it (its `init` runs the loop), activate it, retire it, and give the
/// threads it left behind time to find out that their generation is gone.
fn wake(shell: &std::path::Path) -> i32 {
    let module = match load_or_report(shell, Copying::Unique, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    if let Err(message) = module.activate() {
        out!("activate_failed {message}");
        return 1;
    }
    pump(&module, 3);
    std::thread::sleep(std::time::Duration::from_millis(50));
    module.retire();
    std::thread::sleep(std::time::Duration::from_millis(300));
    out!("wake_done");
    0
}

/// Spike 4: a panic in a shell callback and in a core export is contained, logged, and the next event is delivered.
fn panic_scenario(shell: &std::path::Path) -> i32 {
    let module = match load_or_report(shell, Copying::Unique, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    if let Err(message) = module.activate() {
        out!("activate_failed {message}");
        return 1;
    }
    publish("tree", "attached", vec![OwnedValue::Uint(1), OwnedValue::Node(2)]);
    publish("lock", "requested", vec![]);
    pump(&module, 3);
    let rc = guard(abi::E_PANIC, "selftest_panic", || {
        panic!("selftest panic inside the core export")
    });
    out!("selftest_panic rc={rc} last_error={:?}", last_error());
    shell_stats_line("shell_stats", &module);
    core_stats_line("core_stats", module.ctx());
    out!("still_alive=true");
    0
}

/// Spike 5: a reload: stage the new generation, swap, retire the old without `dlclose`, no stale calls, a failed reload
/// keeps the running generation.
fn hot(v1: &std::path::Path, v2: &std::path::Path, bad: Option<&std::path::Path>) -> i32 {
    let old = match load_or_report(v1, Copying::Unique, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    if old.activate().is_err() {
        return 1;
    }
    publish("lock", "requested", vec![]);
    pump(&old, 2);
    shell_stats_line("old_before_swap", &old);

    let new = match load_or_report(v2, Copying::Unique, 1) {
        Ok(module) => module,
        Err(code) => {
            out!("reload_failed_keeping_old");
            return code;
        }
    };
    out!("new_state={:?}", new.state());
    // A staged generation receives nothing, while the old one still serves.
    publish("lock", "requested", vec![]);
    pump(&old, 2);
    shell_stats_line("new_while_staged", &new);
    shell_stats_line("old_while_new_staged", &old);

    // Commit: activate the new generation and retire the old in one step, with no dispatch between them.
    if new.activate().is_err() {
        return 1;
    }
    let removed = old.retire();
    out!(
        "retired old generation={} removed={removed} state={:?}",
        old.generation(),
        old.state()
    );
    let at_retire = old.shell_stats().map_or(0, |stats| stats.calls);
    shell_stats_line("old_after_retire", &old);

    for _ in 0..5 {
        publish("lock", "requested", vec![]);
    }
    pump(&new, 5);
    let after = old.shell_stats().map_or(0, |stats| stats.calls);
    out!("stale_calls_into_retired={}", after - at_retire);
    shell_stats_line("new_active", &new);

    // A call from the retired generation's code is refused and does nothing.
    let Some((class_id, signal_id)) = catalog_ids("lock", "requested") else {
        return 1;
    };
    unsafe extern "C" fn ignore(_user: *mut c_void, _event: *const abi::GnoblinEvent) -> i32 {
        abi::E_OK
    }
    // SAFETY: `on_signal` is a function of the core's own table; the callback ignores its arguments.
    let reg = unsafe { (API.on_signal)(old.ctx(), 0, class_id, signal_id, ignore, core::ptr::null_mut(), None) };
    out!("call_from_retired reg={reg} last_error={:?}", last_error());
    core_stats_line("core_stats", new.ctx());
    maps_line();
    out!("retired_libraries={}", host::retired_libraries());
    pump(&new, 1);

    if let Some(bad) = bad {
        match host::load(bad, Copying::Unique, 2) {
            Ok(module) => {
                out!("UNEXPECTED bad module loaded");
                module.retire();
            }
            Err(error) => out!("reload_failed kept_generation={} reason={error}", new.generation()),
        }
        publish("lock", "requested", vec![]);
        pump(&new, 2);
        shell_stats_line("after_failed_reload", &new);
        core_stats_line("core_after_failed_reload", new.ctx());
        out!("retired_libraries={}", host::retired_libraries());
    }
    0
}

fn catalog_ids(class: &str, signal: &str) -> Option<(u32, u32)> {
    let class = crate::catalog::class_id(class)?;
    Some((class, crate::catalog::signal_id(class, signal)?))
}

/// An abandoned reload: the staged generation's registrations are dropped without a callback, the old one keeps serving.
fn abandon(v1: &std::path::Path, v2: &std::path::Path) -> i32 {
    let old = match load_or_report(v1, Copying::Unique, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    if old.activate().is_err() {
        return 1;
    }
    let new = match load_or_report(v2, Copying::Unique, 1) {
        Ok(module) => module,
        Err(code) => return code,
    };
    publish("lock", "requested", vec![]);
    pump(&old, 2);
    shell_stats_line("new_before_abandon", &new);
    let removed = new.abandon();
    out!("abandoned removed={removed} state={:?}", new.state());
    shell_stats_line("new_after_abandon", &new);
    publish("lock", "requested", vec![]);
    pump(&old, 2);
    shell_stats_line("old_still_serving", &old);
    shell_stats_line("new_after_more_events", &new);
    0
}

/// Spike 5a: opening the same path twice returns one handle; unique copies return distinct ones.
fn dup(shell: &std::path::Path) -> i32 {
    let a = host::load(shell, Copying::SamePath, 0);
    let b = host::load(shell, Copying::SamePath, 0);
    let (Ok(a), Ok(b)) = (a, b) else {
        out!("load_failed");
        return 1;
    };
    out!("same_path handles_equal={}", a.library_handle() == b.library_handle());
    let c = host::load(shell, Copying::Unique, 0);
    let d = host::load(shell, Copying::Unique, 0);
    let (Ok(c), Ok(d)) = (c, d) else {
        out!("load_failed");
        return 1;
    };
    out!(
        "unique_copies handles_distinct={}",
        c.library_handle() != d.library_handle() && c.library_handle() != a.library_handle()
    );
    maps_line();
    0
}

/// Spike 6: the core's ABI and table size, and a shell built against some minor running on it.
fn upgrade(shell: &std::path::Path) -> i32 {
    out!("core_abi={}.{} table_size={}", abi::ABI_MAJOR, abi::ABI_MINOR, API.size);
    let module = match load_or_report(shell, Copying::Unique, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    if module.activate().is_err() {
        return 1;
    }
    publish("lock", "requested", vec![]);
    pump(&module, 2);
    shell_stats_line("shell_stats", &module);
    0
}

/// How many module copies of this process are mapped now (the loader's copies are named `gen-N-...`).
fn shell_mappings() -> usize {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
    maps.lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .filter(|path| path.contains("/gen-"))
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// Retention is bounded and visible: reload over and over under a small limit. Each reload retires the previous
/// generation, which stays mapped and counted; the load past the limit is refused with the restart text, and the running
/// generation keeps serving.
fn retain(shell: &std::path::Path, limit: Option<&std::path::Path>) -> i32 {
    let max = limit
        .and_then(|limit| limit.to_str())
        .and_then(|text| text.parse::<usize>().ok())
        .unwrap_or(3);
    host::set_retire_policy(RetirePolicy {
        max_retained: max,
        allow_unload: false,
    });
    let policy = host::retire_policy();
    out!(
        "policy max_retained={} allow_unload={}",
        policy.max_retained,
        policy.allow_unload
    );
    let mut running: Option<Module> = None;
    let rounds = u32::try_from(max).unwrap_or(u32::MAX).saturating_add(3);
    for round in 0..rounds {
        match host::load(shell, Copying::Unique, round) {
            Ok(module) => {
                if let Err(message) = module.activate() {
                    out!("activate_failed {message}");
                    return 1;
                }
                if let Some(old) = running.take() {
                    old.retire();
                }
                let stats = host::retained_stats();
                out!(
                    "round={round} loaded generation={} retained={} mapped_bytes={} max={} status={stats}",
                    module.generation(),
                    stats.count,
                    stats.mapped_bytes,
                    stats.max_retained
                );
                running = Some(module);
            }
            Err(error) => {
                out!("round={round} refused {error}");
                if let Some(module) = &running {
                    publish("lock", "requested", vec![]);
                    pump(module, 2);
                    shell_stats_line("serving_after_refusal", module);
                }
                out!("retained_after_refusal={}", host::retired_libraries());
                out!("shell_mappings={}", shell_mappings());
                return 0;
            }
        }
    }
    out!("UNEXPECTED the limit was never reached");
    1
}

/// Unloading is opt-in and proven: a retired generation that reports (`gnoshell_unload_safe`) it has nothing running is
/// unmapped, and every other case keeps its library mapped with the reason.
fn unload(safe: &std::path::Path, unsure: &std::path::Path, plain: &std::path::Path) -> i32 {
    let load = |path: &std::path::Path, count: u32| match load_or_report(path, Copying::Unique, count) {
        Ok(module) => {
            if module.activate().is_err() {
                return Err(1);
            }
            Ok(module)
        }
        Err(code) => Err(code),
    };
    // Off by default: a module that is safe and retired is still kept.
    let first = match load(safe, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    first.retire();
    let first = match first.try_unload() {
        Ok(_) => {
            out!("UNEXPECTED unloaded with the default policy");
            return 1;
        }
        Err((module, why)) => {
            out!("default_policy refused={why:?}");
            module
        }
    };
    out!(
        "after_default retained={} mappings={}",
        host::retired_libraries(),
        shell_mappings()
    );

    host::set_retire_policy(RetirePolicy {
        allow_unload: true,
        ..host::retire_policy()
    });
    // A generation that has not retired is never unloaded.
    let running = match load(safe, 1) {
        Ok(module) => module,
        Err(code) => return code,
    };
    let running = match running.try_unload() {
        Ok(_) => {
            out!("UNEXPECTED unloaded a running generation");
            return 1;
        }
        Err((module, why)) => {
            out!("running refused={why:?}");
            module
        }
    };
    // A module that does not say it is safe is kept, whether it answers no or has no such export.
    for (name, path) in [("unsure", unsure), ("plain", plain)] {
        let module = match load(path, 2) {
            Ok(module) => module,
            Err(code) => return code,
        };
        module.retire();
        match module.try_unload() {
            Ok(_) => {
                out!("UNEXPECTED unloaded {name}");
                return 1;
            }
            Err((_module, why)) => out!("{name} refused={why:?}"),
        }
    }
    out!(
        "before_unload retained={} mappings={}",
        host::retired_libraries(),
        shell_mappings()
    );

    // The one that proves it is unmapped, and the list gives its entry back.
    match first.try_unload() {
        Ok(stats) => out!(
            "unloaded retained={} unloaded_total={} mappings={}",
            stats.count,
            stats.unloaded,
            shell_mappings()
        ),
        Err((_module, why)) => {
            out!("UNEXPECTED refused {why:?}");
            return 1;
        }
    }
    // The core serves on: the running generation still gets its events.
    publish("lock", "requested", vec![]);
    pump(&running, 2);
    shell_stats_line("running_after_unload", &running);
    core_stats_line("core_after_unload", running.ctx());
    0
}

static POSTED_RAN: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn mark_posted(_user: *mut c_void) {
    POSTED_RAN.store(true, Ordering::SeqCst);
}

/// Thread affinity: shell-thread functions refuse another thread, any-thread functions work, and posted work runs on the
/// shell thread.
fn threads(shell: &std::path::Path) -> i32 {
    let module = match load_or_report(shell, Copying::Unique, 0) {
        Ok(module) => module,
        Err(code) => return code,
    };
    if module.activate().is_err() {
        return 1;
    }
    let ctx = module.ctx();
    let results = thread::spawn(move || {
        // SAFETY: functions of the core's own table, called with a live context.
        unsafe {
            let dispatch = (API.dispatch)(ctx, 0);
            let log = (API.log)(ctx, abi::LOG_INFO, b"hello from another thread".as_ptr(), 25);
            let post = (API.post)(ctx, mark_posted, core::ptr::null_mut(), None);
            (dispatch, log, post)
        }
    })
    .join();
    match results {
        Ok((dispatch, log, post)) => out!("other_thread dispatch={dispatch} log={log} post={post}"),
        Err(_) => {
            out!("other_thread panicked");
            return 1;
        }
    }
    out!("posted_ran_before_dispatch={}", POSTED_RAN.load(Ordering::SeqCst));
    let ran = module.dispatch(100);
    out!(
        "dispatch_returned={ran} posted_ran={}",
        POSTED_RAN.load(Ordering::SeqCst)
    );
    0
}
