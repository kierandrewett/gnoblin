//! What starting the shell costs: attach, load the stock library, evaluate a `shell.ts` that imports it.
//!
//! Every start of the runtime role and every reload pays this. `boot_cost` prints the milliseconds of ten boots with no
//! bytecode cache, with an empty one (the first start after an install or an upgrade) and with a filled one (every
//! start after that) (`cargo test --release -p gnoblin-js --test boot_cost -- --ignored --nocapture`); it is ignored
//! because a timing is not a pass or fail. What the cache promises without a clock is asserted by `bytecode_cache.rs`.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use gnoblin_js::{JsBytecodeCache, JsShell};

mod common;
use common::*;

const BOOTS: usize = 10;

/// One boot: attach a shell, load the stock library and evaluate `shell.ts`. The stock `gnoblin-std` calls
/// `libgnoblin-std` through `__gnoblinStd`, so the shell runs over a core generation with the std library loaded, as in the
/// compositor. That core is set up before the clock starts: what is timed is the shell's start, the same span as before
/// the library went native.
fn boot(root: &Path, cache: &JsBytecodeCache) -> f64 {
    let mut config = honest_config(root);
    config.js.cache = cache.clone();
    let core = common::native::NativeCore::new(&FakeCompositor::new(), Arc::clone(&config.clock));
    core.configure(&mut config);
    let started = Instant::now();
    let mut shell = JsShell::new(core.link(), config).expect("the shell boots over the core");
    shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    let took = started.elapsed().as_secs_f64() * 1e3;
    drop(shell);
    drop(core);
    took
}

fn report(label: &str, mut times: Vec<f64>) {
    times.sort_by(f64::total_cmp);
    println!(
        "{label:<22} min {:6.1}  median {:6.1}  max {:6.1}",
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

#[test]
#[ignore = "a benchmark: run with --release --ignored --nocapture"]
fn boot_cost() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let base = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let root = base.join("config");
    fs::create_dir_all(&root).expect("the config root");
    fs::write(root.join("shell.ts"), "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\nvoid lib;\n").expect("shell.ts");

    println!("boot ms ({BOOTS} boots each)");
    report("rig (no cache)", {
        (0..BOOTS)
            .map(|_| {
                let started = Instant::now();
                let rig = rig_with("import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\nvoid lib;\n", "");
                let took = started.elapsed().as_secs_f64() * 1e3;
                drop(rig);
                took
            })
            .collect()
    });
    report(
        "no cache",
        (0..BOOTS).map(|_| boot(&root, &JsBytecodeCache::disabled())).collect(),
    );
    report(
        "cold (empty cache)",
        (0..BOOTS)
            .map(|index| boot(&root, &JsBytecodeCache::in_dir(base.join(format!("cold{index}")))))
            .collect(),
    );
    let warm = base.join("warm");
    boot(&root, &JsBytecodeCache::in_dir(&warm));
    report(
        "warm (filled cache)",
        (0..BOOTS)
            .map(|_| boot(&root, &JsBytecodeCache::in_dir(&warm)))
            .collect(),
    );
}
