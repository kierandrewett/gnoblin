//! What the product's call budgets guard, and the numbers they were set from.
//!
//! The budgets are wall-clock limits on one call into a script: they
//! bound a script that hangs the shell, and nothing else. A limit on wall time also trips when the machine does not
//! schedule the thread, so what matters is how far the work a call really does is from the limit.
//!
//! `the_budgets_are_the_documented_constants` pins the values and the one place each lives (`src/budget.rs`, which
//! also carries the numbers they were set from). The two measurements are
//! ignored by default because they report time and assert none of it (no assertion in this crate may rest on how fast
//! the machine is): run them in release and in debug and read the numbers.
//!
//! ```text
//! cargo test --release -p gnoblin-js --test budgets -- --ignored --nocapture --test-threads=1
//! ```

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gnoblin_js::{
    ENTRY_BUDGET, FRAME_BUDGET, JsConfig, JsErrorKind, JsShell, JsShellConfig, JsShellError, STAGE_BUDGET,
};

mod common;
use common::*;

/// The starter, `shells/default/shell.ts`: what the packaged default shell is.
const STARTER_CONFIG: &str = include_str!("../../../shells/default/shell.ts");

#[test]
fn the_budgets_are_the_documented_constants() {
    assert_eq!(STAGE_BUDGET, Duration::from_secs(30));
    assert_eq!(ENTRY_BUDGET, Duration::from_secs(1));
    assert_eq!(FRAME_BUDGET, Duration::from_secs(5));
    let config = JsShellConfig::new("/nonexistent");
    assert_eq!(config.js.stage_budget, STAGE_BUDGET, "the load of a module");
    assert_eq!(config.entry_budget, ENTRY_BUDGET, "one listener call");
    assert_eq!(config.frame_budget, FRAME_BUDGET, "one frame's dispatch");
    assert_eq!(JsConfig::new("/nonexistent").stage_budget, STAGE_BUDGET);
}

#[test]
fn the_stock_default_config_is_a_load_and_runs_under_the_stage_budget() {
    // Loading the default config evaluates the whole starter, as loading a user's `shell.ts` does, and under load it
    // took as long (about 1.8 s with two busy threads per CPU, against 10 ms idle). It is a load, so it has the
    // stage budget, and not the 100-millisecond-class budget of one listener call. A stage budget too short for it
    // must therefore be what interrupts it.
    let comp = FakeCompositor::new();
    populate(&comp, 3);
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut config = JsShellConfig::new(fs::canonicalize(dir.path()).expect("the temp dir exists"))
        .with_stock(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js"));
    config.js.stage_budget = Duration::from_micros(1);
    let mut shell = JsShell::new(Box::new(comp), config).expect("the library installs under its own floor");
    let error = shell
        .load_default_config()
        .expect_err("a stage budget of a microsecond cannot hold the starter");
    assert!(
        matches!(&error, JsShellError::Js(js) if js.kind == JsErrorKind::Budget),
        "the load ran out of its stage budget: {error:?}"
    );
}

/// A screen, a seat, a lock and `windows` windows, as a busy desktop would have.
fn populate(comp: &FakeCompositor, windows: u64) {
    comp.add_desktop(1920.0, 1080.0);
    for index in 0..windows {
        comp.add_window(100 + index, &format!("window {index}"), "foot");
    }
}

/// Busy threads that keep every CPU contended until they are dropped, the load the budgets have to survive.
struct Load {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Load {
    fn per_cpu(factor: usize) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let count = std::thread::available_parallelism().map_or(1, usize::from) * factor;
        let threads = (0..count)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut value = 1_u64;
                    while !stop.load(Ordering::Relaxed) {
                        value = std::hint::black_box(value.wrapping_mul(6364136223846793005).wrapping_add(1));
                    }
                })
            })
            .collect();
        Self { stop, threads }
    }
}

impl Drop for Load {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn percentile(sorted: &[Duration], per_cent: usize) -> Duration {
    sorted[(sorted.len() * per_cent / 100).min(sorted.len() - 1)]
}

/// Boots the stock desktop under the product's own budgets and reports how long each guarded call took.
fn measure(label: &str, load_factor: usize) {
    let _load = (load_factor > 0).then(|| Load::per_cpu(load_factor));
    let comp = FakeCompositor::new();
    populate(&comp, 40);
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), STARTER_CONFIG).expect("shell.ts is written");

    // The budgets are the product's: `JsShellConfig::new`, the real clock, nothing loosened.
    let config = JsShellConfig::new(&root).with_stock(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js"));
    let started = Instant::now();
    let mut shell = JsShell::new(Box::new(comp.clone()), config).expect("the shell boots under the product budgets");
    let boot = started.elapsed();

    let started = Instant::now();
    let diagnostics = shell
        .load_entry(Path::new("shell.ts"))
        .expect("the starter config loads under the stage budget");
    let load = started.elapsed();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let mut bare = JsShell::new(
        Box::new(comp.clone()),
        JsShellConfig::new(&root).with_stock(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/js")),
    )
    .expect("a second shell boots");
    let started = Instant::now();
    bare.load_default_config()
        .expect("the stock default loads under the stage budget");
    let default = started.elapsed();
    drop(bare);

    // A busy minute of a desktop in miniature: titles change every frame, a window opens every tenth.
    let mut frames = Vec::new();
    let mut fired: Vec<String> = Vec::new();
    for round in 0..300_u64 {
        comp.external_set(100 + round % 40, "title", text(&format!("title {round}")));
        if round % 10 == 0 {
            comp.create_window(1000 + round, "new", "foot");
        }
        let started = Instant::now();
        let frame = shell.frame().expect("a frame");
        frames.push(started.elapsed());
        fired.extend(
            frame
                .diagnostics
                .into_iter()
                .filter(|line| line.contains("budget") || line.contains("stopped")),
        );
    }
    frames.sort();
    eprintln!(
        "[{label}] boot (library install) {boot:?}; load shell.ts (stage budget) {load:?}; \
         load the stock default (stage budget) {default:?}; \
         frame p50 {:?} p99 {:?} max {:?} (frame budget {FRAME_BUDGET:?}, listener call {ENTRY_BUDGET:?}); \
         strikes {}, budget diagnostics {}: {:?}",
        percentile(&frames, 50),
        percentile(&frames, 99),
        frames[frames.len() - 1],
        shell.strikes(),
        fired.len(),
        fired.first(),
    );
}

#[test]
#[ignore = "a measurement: it reports time and asserts none"]
fn measure_the_budgets_on_an_idle_machine() {
    measure("idle", 0);
}

#[test]
#[ignore = "a measurement: it reports time and asserts none"]
fn measure_the_budgets_with_two_busy_threads_per_cpu() {
    measure("2 busy threads per CPU", 2);
}

#[test]
#[ignore = "a measurement: it reports time and asserts none"]
fn measure_the_budgets_with_four_busy_threads_per_cpu() {
    measure("4 busy threads per CPU", 4);
}
