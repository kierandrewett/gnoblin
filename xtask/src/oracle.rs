//! `cargo xtask oracle`: the golden fixtures of the native std port (`design/native-std-port.md`, lane NpOracle; the map is
//! `design/native-std-port-tests.md`).
//!
//! The TypeScript library under `data/js` is the specification of the desktop's toolkit, frame, menus, style, scene and input
//! behaviour until the last lane deletes it. The fixtures hold what it does: the transactions it sends, the order of its listener calls,
//! the bytes of every widget it paints, and the declarations it emits. Each native lane runs against them.
//!
//! ```text
//! cargo xtask oracle record            run the TypeScript library and rewrite every fixture
//! cargo xtask oracle check [--repeat N]  run it again into a scratch directory and fail on any difference from the committed files
//! cargo xtask oracle list              name the fixture directories and what writes each
//! ```
//!
//! `check` records into `target/oracle/run-<n>` and compares the whole tree with the committed one, in both directions, so a missing
//! fixture, a stale one and a changed one each fail. With `--repeat 3` it also compares the runs with each other, which is the proof that a
//! recording does not depend on the run. The tests that write the fixtures check against them as well when they run on their own
//! (`cargo test -p gnoblin-js --test oracle`), so an ordinary test run notices a change to the TypeScript library.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

pub const USAGE: &str = "usage: cargo xtask oracle record | check [--repeat N] | list";

/// A directory of fixtures, relative to the workspace root, and what makes the files in it.
const FIXTURE_DIRS: &[(&str, &str)] = &[
    (
        "crates/gnoblin-std-impl/tests/fixtures/reference",
        "the TypeScript std's writes: tiling, examples, anim_style, and the scenarios of crates/gnoblin-js/tests/oracle",
    ),
    (
        "crates/gnoblin-js/tests/fixtures/paint",
        "paint lists of widgets in each state: crates/gnoblin-js/tests/oracle",
    ),
    (
        "crates/gnoblin-core-impl/tests/fixtures/model",
        "listener order and input matcher traces: crates/gnoblin-js/tests/oracle",
    ),
];

/// What one step of a recording runs: a cargo test run, with the arguments after `cargo`.
struct Producer {
    label: &'static str,
    args: &'static [&'static str],
}

const PRODUCERS: &[Producer] = &[
    Producer {
        label: "gnoblin-js oracle scenarios",
        args: &["test", "--locked", "-p", "gnoblin-js", "--test", "oracle"],
    },
    Producer {
        label: "gnoblin-std-impl tiling reference",
        args: &["test", "--locked", "-p", "gnoblin-std-impl", "--test", "tiling"],
    },
    Producer {
        label: "gnoblin-std-impl anim_style reference",
        args: &["test", "--locked", "-p", "gnoblin-std-impl", "--test", "anim_style"],
    },
    Producer {
        label: "gnoblin-std-impl examples reference",
        args: &[
            "test",
            "--locked",
            "-p",
            "gnoblin-std-impl",
            "--test",
            "examples",
            "--",
            "floating_placement_matches_the_typescript_std",
            "the_native_tiling_layouts_match_the_typescript_std",
        ],
    },
];

#[derive(Debug, PartialEq, Eq)]
enum OracleCommand {
    Record,
    Check { repeat: usize },
    List,
}

fn parse(args: &[String]) -> Result<OracleCommand, String> {
    match args {
        [word] if word == "record" => Ok(OracleCommand::Record),
        [word] if word == "list" => Ok(OracleCommand::List),
        [word, rest @ ..] if word == "check" => match rest {
            [] => Ok(OracleCommand::Check { repeat: 1 }),
            [flag, count] if flag == "--repeat" => {
                let repeat: usize = count.parse().map_err(|_| format!("`{count}` is not a count"))?;
                if repeat == 0 {
                    return Err("`--repeat` is at least 1".to_owned());
                }
                Ok(OracleCommand::Check { repeat })
            }
            _ => Err("`check` takes only `--repeat N`".to_owned()),
        },
        _ => Err("expected `record`, `check` or `list`".to_owned()),
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let command = match parse(args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("[oracle] {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let root = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(error) => {
            eprintln!("[oracle] no working directory: {error}");
            return ExitCode::from(2);
        }
    };
    match command {
        OracleCommand::List => {
            for (dir, maker) in FIXTURE_DIRS {
                println!("{dir}\n    {maker}");
            }
            ExitCode::SUCCESS
        }
        OracleCommand::Record => record(&root),
        OracleCommand::Check { repeat } => check(&root, repeat),
    }
}

/// Runs every producer with `GNOBLIN_ORACLE=record`, writing under `out` when it is given.
fn produce(root: &Path, out: Option<&Path>) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    for producer in PRODUCERS {
        eprintln!("[oracle] {}", producer.label);
        let mut command = Command::new(&cargo);
        command
            .current_dir(root)
            .args(producer.args)
            .env("GNOBLIN_ORACLE", "record")
            .env("TZ", "UTC")
            .env_remove("GNOBLIN_FAKE_REFUSE_EVERY");
        match out {
            Some(out) => command.env("GNOBLIN_ORACLE_OUT", out),
            None => command.env_remove("GNOBLIN_ORACLE_OUT"),
        };
        let status = command
            .status()
            .map_err(|error| format!("cannot run `{cargo}`: {error}"))?;
        if !status.success() {
            return Err(format!("{} failed ({status})", producer.label));
        }
    }
    Ok(())
}

fn record(root: &Path) -> ExitCode {
    for (dir, _) in FIXTURE_DIRS {
        // Rewritten from nothing, so a fixture no scenario makes any more does not stay behind.
        if let Err(error) = clear(&root.join(dir)) {
            eprintln!("[oracle] {error}");
            return ExitCode::from(2);
        }
    }
    match produce(root, None) {
        Ok(()) => {
            let count: usize = FIXTURE_DIRS.iter().map(|(dir, _)| files(&root.join(dir)).len()).sum();
            eprintln!("[oracle] recorded {count} fixture files");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("[oracle] {message}");
            ExitCode::FAILURE
        }
    }
}

fn check(root: &Path, repeat: usize) -> ExitCode {
    let scratch = root.join("target/oracle");
    let mut runs = Vec::new();
    for n in 0..repeat {
        let out = scratch.join(format!("run-{n}"));
        let _ = fs::remove_dir_all(&out);
        if let Err(message) = produce(root, Some(&out)) {
            eprintln!("[oracle] {message}");
            return ExitCode::FAILURE;
        }
        runs.push(out);
    }
    let mut problems = Vec::new();
    for (dir, _) in FIXTURE_DIRS {
        for (n, out) in runs.iter().enumerate() {
            for line in compare(&root.join(dir), &out.join(dir)) {
                problems.push(format!("run {n}: {dir}: {line}"));
            }
        }
        for (n, out) in runs.iter().enumerate().skip(1) {
            for line in compare(&runs[0].join(dir), &out.join(dir)) {
                problems.push(format!("run 0 against run {n}: {dir}: {line}"));
            }
        }
    }
    if problems.is_empty() {
        let count: usize = FIXTURE_DIRS.iter().map(|(dir, _)| files(&root.join(dir)).len()).sum();
        eprintln!("[oracle] {count} fixture files equal what the TypeScript library does, over {repeat} run(s)");
        return ExitCode::SUCCESS;
    }
    for line in &problems {
        eprintln!("[oracle] {line}");
    }
    eprintln!(
        "[oracle] {} difference(s); `cargo xtask oracle record` rewrites the fixtures if the TypeScript change is meant",
        problems.len()
    );
    ExitCode::FAILURE
}

/// Removes the fixture files of `dir` and leaves the rest (a `README.md` is the reader's).
fn clear(dir: &Path) -> Result<(), String> {
    for (name, _) in files(dir) {
        let path = dir.join(&name);
        fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

/// Every fixture file under `dir`, by path relative to it, with its bytes' length. A missing directory has none.
fn files(dir: &Path) -> BTreeMap<String, u64> {
    let mut found = BTreeMap::new();
    walk(dir, dir, &mut found);
    found
}

fn walk(root: &Path, dir: &Path, found: &mut BTreeMap<String, u64>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "json" || ext == "bin") {
            let name = path.strip_prefix(root).unwrap_or(&path).display().to_string();
            found.insert(name, entry.metadata().map_or(0, |meta| meta.len()));
        }
    }
}

/// What differs between two fixture directories: a file only one has, and a file whose bytes differ.
fn compare(left: &Path, right: &Path) -> Vec<String> {
    let (a, b) = (files(left), files(right));
    let mut out = Vec::new();
    for name in a.keys() {
        if !b.contains_key(name) {
            out.push(format!("{name} is committed but nothing records it"));
        }
    }
    for name in b.keys() {
        if !a.contains_key(name) {
            out.push(format!("{name} is recorded but not committed"));
        }
    }
    for name in a.keys().filter(|name| b.contains_key(*name)) {
        let (x, y) = (fs::read(left.join(name)), fs::read(right.join(name)));
        if x.ok() != y.ok() {
            out.push(format!("{name} differs"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::scratch;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_command_line_is_record_check_or_list() {
        assert_eq!(parse(&strings(&["record"])), Ok(OracleCommand::Record));
        assert_eq!(parse(&strings(&["list"])), Ok(OracleCommand::List));
        assert_eq!(parse(&strings(&["check"])), Ok(OracleCommand::Check { repeat: 1 }));
        assert_eq!(
            parse(&strings(&["check", "--repeat", "3"])),
            Ok(OracleCommand::Check { repeat: 3 })
        );
        assert!(parse(&strings(&["check", "--repeat", "0"])).is_err());
        assert!(parse(&strings(&["check", "--repeat", "x"])).is_err());
        assert!(parse(&strings(&["check", "again"])).is_err());
        assert!(parse(&strings(&[])).is_err());
        assert!(parse(&strings(&["bless"])).is_err());
    }

    #[test]
    fn a_missing_a_stale_and_a_changed_fixture_each_fail_the_comparison() {
        let base = scratch("oracle-compare");
        let (committed, recorded) = (base.join("committed"), base.join("recorded"));
        for dir in [&committed, &recorded] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(committed.join("same.json"), "{}\n").unwrap();
        fs::write(recorded.join("same.json"), "{}\n").unwrap();
        fs::write(committed.join("changed.bin"), [1, 2]).unwrap();
        fs::write(recorded.join("changed.bin"), [1, 3]).unwrap();
        fs::write(committed.join("stale.json"), "{}\n").unwrap();
        fs::write(recorded.join("new.json"), "{}\n").unwrap();
        fs::write(committed.join("README.md"), "not a fixture\n").unwrap();
        let problems = compare(&committed, &recorded);
        assert_eq!(
            problems,
            strings(&[
                "stale.json is committed but nothing records it",
                "new.json is recorded but not committed",
                "changed.bin differs",
            ])
        );
        assert!(compare(&committed, &committed).is_empty());
    }

    #[test]
    fn clearing_leaves_what_is_not_a_fixture() {
        let base = scratch("oracle-clear");
        fs::create_dir_all(base.join("deep")).unwrap();
        fs::write(base.join("a.json"), "{}\n").unwrap();
        fs::write(base.join("deep/b.bin"), [0]).unwrap();
        fs::write(base.join("README.md"), "text\n").unwrap();
        clear(&base).unwrap();
        assert!(files(&base).is_empty());
        assert!(base.join("README.md").exists());
    }
}
