//! The bytecode cache, end to end: a start that finds what the last one compiled loads it, a file that cannot be
//! trusted is compiled again, and the result is the same shell either way.
//!
//! The shells here run over the fake compositor of `common`, with the stock library embedded as the product embeds
//! it, so `import "gnoblin"` is every stock module: the numbers asserted are how many of them came from bytecode.

use std::fs;
use std::path::{Path, PathBuf};

use gnoblin_js::{JsBytecodeCache, JsShell, JsShellError};

mod common;
use common::*;

const IMPORTS: &str = "import * as lib from \"gnoblin\";\nvoid lib;\n";

/// A shell over a fresh fake compositor, whose config root is `root` and whose modules are cached by `cache`.
fn boot(root: &Path, cache: &JsBytecodeCache) -> Result<JsShell, JsShellError> {
    let mut config = honest_config(root);
    config.js.cache = cache.clone();
    let mut shell = JsShell::new(Box::new(FakeCompositor::new()), config)?;
    shell.load_entry(Path::new("init.ts"))?;
    Ok(shell)
}

/// A config root holding `init.ts`, and a cache directory beside it.
fn world(init: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let base = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let root = base.join("config");
    let cache = base.join("cache");
    fs::create_dir_all(&root).expect("the config root");
    fs::write(root.join("init.ts"), init).expect("init.ts is written");
    (dir, root, cache)
}

fn files_in(cache: &JsBytecodeCache) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(cache.dir().expect("an enabled cache"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
}

fn works(shell: &mut JsShell) {
    let kind = shell.eval("typeof gnoblin.clients").expect("the script runs");
    assert_eq!(kind.as_deref(), Some("object"), "the object model is there");
}

#[test]
fn the_second_start_loads_every_module_from_bytecode() {
    let (_dir, root, cache_dir) = world(IMPORTS);
    let first = JsBytecodeCache::in_dir(&cache_dir);
    let mut shell = boot(&root, &first).expect("the first start");
    works(&mut shell);
    let cold = first.stats();
    assert_eq!(cold.hits, 0, "nothing was cached yet");
    assert!(
        cold.writes >= 40,
        "the stock library and init.ts were written: {cold:?}"
    );
    assert_eq!(cold.misses, cold.writes, "every module that missed was written");
    assert_eq!(files_in(&first).len() as u64, cold.writes);

    let second = JsBytecodeCache::in_dir(&cache_dir);
    let mut shell = boot(&root, &second).expect("the second start");
    works(&mut shell);
    let warm = second.stats();
    assert_eq!(warm.hits, cold.writes, "every module came from bytecode: {warm:?}");
    assert_eq!((warm.misses, warm.writes, warm.rejected), (0, 0, 0), "{warm:?}");
}

#[test]
fn a_cached_start_runs_the_same_script_as_a_compiled_one() {
    let init = format!(
        "{IMPORTS}class Probe extends lib.Layout {{ compute() {{ return []; }} }}\n(globalThis as any).answer = [new Probe() instanceof lib.Layout, typeof lib.gnoblin.clients].join(',');\n"
    );
    let (_dir, root, cache_dir) = world(&init);
    let plain = {
        let mut shell = boot(&root, &JsBytecodeCache::disabled()).expect("a start with no cache");
        shell.eval("answer").expect("the script runs")
    };
    for _ in 0..2 {
        let mut shell = boot(&root, &JsBytecodeCache::in_dir(&cache_dir)).expect("a cached start");
        assert_eq!(shell.eval("answer").expect("the script runs"), plain);
    }
    assert_eq!(plain.as_deref(), Some("true,object"));
}

#[test]
fn a_changed_config_is_compiled_and_the_stock_library_is_still_cached() {
    let (_dir, root, cache_dir) = world(IMPORTS);
    boot(&root, &JsBytecodeCache::in_dir(&cache_dir)).expect("the first start");
    fs::write(root.join("init.ts"), format!("{IMPORTS}// an edit\n")).expect("init.ts is rewritten");
    let cache = JsBytecodeCache::in_dir(&cache_dir);
    boot(&root, &cache).expect("the start after the edit");
    let stats = cache.stats();
    assert_eq!(
        (stats.misses, stats.writes),
        (1, 1),
        "only init.ts was compiled: {stats:?}"
    );
    assert!(
        stats.hits >= 40,
        "the stock library was loaded from bytecode: {stats:?}"
    );
}

#[test]
fn a_file_that_cannot_be_trusted_is_compiled_again_and_replaced() {
    let (_dir, root, cache_dir) = world(IMPORTS);
    let seed = JsBytecodeCache::in_dir(&cache_dir);
    boot(&root, &seed).expect("the first start");
    let files = files_in(&seed);
    let count = files.len() as u64;
    // Four kinds of damage, spread over the files: a flipped bit in the payload, a short file, an empty one, and
    // bytes that are not a cache file at all.
    for (index, file) in files.iter().enumerate() {
        let mut bytes = fs::read(file).expect("a cache file");
        match index % 4 {
            0 => {
                let last = bytes.len() - 1;
                bytes[last] ^= 0x55;
            }
            1 => bytes.truncate(bytes.len() / 2),
            2 => bytes.clear(),
            _ => bytes = b"not bytecode at all".to_vec(),
        }
        fs::write(file, bytes).expect("a damaged file");
    }
    let damaged = JsBytecodeCache::in_dir(&cache_dir);
    let mut shell = boot(&root, &damaged).expect("a start over a damaged cache");
    works(&mut shell);
    let stats = damaged.stats();
    assert_eq!(stats.hits, 0, "nothing damaged was used: {stats:?}");
    assert_eq!(stats.writes, count, "every module was written again: {stats:?}");

    let repaired = JsBytecodeCache::in_dir(&cache_dir);
    boot(&root, &repaired).expect("a start over the repaired cache");
    assert_eq!(
        repaired.stats().hits,
        count,
        "the replacements are good: {:?}",
        repaired.stats()
    );
}

#[test]
fn an_error_in_a_cached_config_is_reported_at_its_ts_line() {
    let (_dir, root, cache_dir) = world("const ready: number = 1;\nvoid ready;\nthrow new Error(\"boom\");\n");
    for start in ["cold", "cached"] {
        let error = match boot(&root, &JsBytecodeCache::in_dir(&cache_dir)) {
            Err(JsShellError::Js(error)) => error,
            other => panic!("{start}: init.ts throws, so the start fails: {:?}", other.map(|_| ())),
        };
        assert_eq!(
            error.location.as_ref().map(|at| at.line),
            Some(3),
            "{start}: the throw is on line 3 of init.ts: {error:?}"
        );
    }
}

#[test]
fn a_disabled_cache_writes_nothing() {
    let (_dir, root, cache_dir) = world(IMPORTS);
    boot(&root, &JsBytecodeCache::disabled()).expect("a start with no cache");
    assert!(!cache_dir.exists());
}

#[test]
fn a_cache_that_cannot_be_written_does_not_stop_the_start() {
    let (_dir, root, cache_dir) = world(IMPORTS);
    // A file where the directory would be: nothing can be made under it.
    fs::write(&cache_dir, "in the way").expect("a file in the way");
    let cache = JsBytecodeCache::in_dir(&cache_dir);
    let mut shell = boot(&root, &cache).expect("the start goes on");
    works(&mut shell);
    assert_eq!(cache.stats().writes, 0);
}
