//! Every time of the script API is in milliseconds, and the easings are the CSS names.
//!
//! The owner's specification says "milliseconds (the JS API uses ms consistently)" and names its easings the CSS way
//! (`design/js-research/owner-spec.md`, "TransitionOptions"). Two halves hold that:
//!
//! - a scan of everything a script reads (the library sources, the declarations, the shell API pages) that fails when a
//!   time is documented in seconds, or an old easing name comes back, and
//! - runs that give the same number to each API that takes a duration and watch what the compositor and the shell's
//!   clock make of it.
//!
//! The one time in seconds is a drag event's `deltaTime`, which the specification itself gives in seconds so a
//! simulation can integrate it; it is named in `SECONDS_ALLOWED` and in nothing else.

mod common;

use std::path::{Path, PathBuf};

use common::Rig;
use common::policy::{exec, settle, world};
use gnoblin_tree_client::tree_mirror::TxOp;

/// The repository root: this crate is `crates/gnoblin-js`.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// Every file under `dir` with the extension `ext`, sorted.
fn files_under(dir: &Path, ext: &str, into: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files_under(&path, ext, into);
        } else if path.extension().is_some_and(|found| found == ext) {
            into.push(path);
        }
    }
}

/// What a script reads as TypeScript: the library and the starter.
fn typescript_sources() -> Vec<PathBuf> {
    let root = repo();
    let mut files = Vec::new();
    files_under(&root.join("data/js"), "ts", &mut files);
    files
}

/// The pages that describe a time parameter.
fn time_pages() -> Vec<PathBuf> {
    let root = repo();
    [
        "docs/js-transitions.md",
        "docs/guides/animations.md",
        "docs/shell-api/tweens.md",
        "docs/shell-api/errors.md",
        "docs/shell-api/workspaces.md",
        "docs/shell-api/processes.md",
        "docs/shell-api/idle.md",
        "docs/shell-api/launch-feedback.md",
        "docs/shell-api/index.md",
    ]
    .iter()
    .map(|page| root.join(page))
    .collect()
}

/// Lines that may say "seconds" and why: each is a time the owner or the user reads in seconds, not a parameter.
const SECONDS_ALLOWED: &[(&str, &str)] = &[
    (
        "Seconds since the previous event",
        "`deltaTime` is in seconds in the specification (owner-spec.md, \"deltaTime is seconds\")",
    ),
    ("deltaTime", "the pages and comments that name that one exception"),
    (
        "{seconds}",
        "the lock screen's message tells a person the wait in seconds; the placeholder is text, not a parameter",
    ),
    ("getSeconds", "`Date.getSeconds`, the clock widget's seconds field"),
];

/// Whether `line` names a unit of time other than milliseconds: the word `seconds`, an identifier that ends in
/// `Seconds` or `SECONDS`, or `secs`.
fn names_seconds(line: &str) -> bool {
    let bytes = line.as_bytes();
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    for needle in ["seconds", "Seconds", "SECONDS", "secs", "sec"] {
        let mut from = 0;
        while let Some(at) = line[from..].find(needle) {
            let start = from + at;
            let end = start + needle.len();
            from = end;
            let after_ok = end >= bytes.len() || !is_word(bytes[end]);
            // `seconds` and `secs` stand alone as words; `Seconds` and `SECONDS` also end a camelCase or
            // snake_case identifier (`armSeconds`, `SLIDE_SECONDS`).
            let before_ok = start == 0 || !is_word(bytes[start - 1]);
            let ends_identifier = needle == "Seconds" || needle == "SECONDS";
            if after_ok && (before_ok || ends_identifier) {
                return true;
            }
        }
    }
    false
}

#[test]
fn no_script_facing_time_is_documented_or_named_in_seconds() {
    let mut found = Vec::new();
    for path in typescript_sources().into_iter().chain(time_pages()) {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        for (index, line) in text.lines().enumerate() {
            if names_seconds(line) && !SECONDS_ALLOWED.iter().any(|(allowed, _)| line.contains(allowed)) {
                found.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(repo()).unwrap_or(&path).display(),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "every time is milliseconds (the specification's rule); these lines name seconds:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_scan_knows_a_seconds_line_from_a_milliseconds_one() {
    for flagged in [
        "/** Seconds to fade in. */",
        "armSeconds = 0.5;",
        "export const SLIDE_SECONDS = 0.25;",
        "| `duration` | seconds, 0 or more |",
        "timeout(seconds: number): Operation<T>;",
        "a wait of 3 secs",
    ] {
        assert!(names_seconds(flagged), "{flagged} names seconds");
    }
    for fine in [
        "/** Milliseconds to fade in. */",
        "armMs = 500;",
        "Key repeats per second, from 0.1 to 1000.",
        "the second binding of one trigger",
        "a second call changes nothing",
        "export const SLIDE_DURATION = 250;",
        "timeout(ms: number): Operation<T>;",
        "the second Enter",
    ] {
        assert!(!names_seconds(fine), "{fine} does not name seconds");
    }
}

#[test]
fn the_old_easing_names_and_the_old_timer_are_gone_from_everything_a_script_reads() {
    let mut found = Vec::new();
    let old = [
        "\"out-cubic\"",
        "\"in-cubic\"",
        "\"in-out\"",
        "'out-cubic'",
        "ClientEasing",
        "TransitionEasing",
        "gnoblin.after(",
        "Client.animate",
        "client.animate",
        "client?.animate",
    ];
    let mut paths = typescript_sources();
    paths.extend(time_pages());
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("a readable source");
        for (index, line) in text.lines().enumerate() {
            if old.iter().any(|name| line.contains(name)) {
                found.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "old names are back:\n{}", found.join("\n"));
}

#[test]
fn the_declared_easing_names_are_the_ones_the_runtime_accepts() {
    let source = std::fs::read_to_string(repo().join("data/js/gnoblin/transition/easing.ts")).expect("the library");
    let declared: Vec<String> = source
        .lines()
        .find_map(|line| line.trim().strip_prefix("export type Easing = "))
        .expect("the library names `Easing`")
        .trim_end_matches(';')
        .split('|')
        .map(|name| name.trim().trim_matches('"').to_owned())
        .collect();
    let mut rig = world(1, false, "new Layout()", "");
    let accepted = rig.eval(
        "(() => { try { gnoblin.clients.get('3').set({ x: 1 }, { duration: 1, easing: 'bouncy' }); return 'no'; } \
         catch (e) { return e.message; } })()",
    );
    let listed = accepted
        .rsplit("easing must be one of ")
        .next()
        .expect("the message lists the names")
        .split(", ")
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(declared, listed, "the d.ts and the runtime name the same easings");
    assert_eq!(
        declared,
        ["linear", "ease", "ease-in", "ease-out", "ease-in-out"],
        "and they are the CSS names"
    );
    // Each name is the CSS curve of that name: it runs from 0 to 1 and never backwards, and its middle is where the
    // CSS function has it (`ease-in` starts slowly, `ease-out` fast).
    let mut middles = std::collections::BTreeMap::new();
    for (index, easing) in declared.iter().enumerate() {
        exec(&mut rig, "g.c = gnoblin.clients.get('3');");
        let mark = rig.comp.sent_count();
        exec(
            &mut rig,
            &format!(
                "g.c.set({{ opacity: {} }}, {{ duration: 100, easing: '{easing}' }})",
                0.9 - 0.1 * index as f64
            ),
        );
        settle(&mut rig);
        let samples = rig
            .comp
            .ops_since(mark)
            .into_iter()
            .find_map(|op| match op {
                TxOp::Set {
                    samples: Some(samples), ..
                } => Some(samples),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{easing}: a timed write"));
        assert_eq!(samples.len(), 33, "{easing}: 33 samples");
        assert_eq!((samples[0], samples[32]), (0.0, 1.0), "{easing}: from 0 to 1");
        assert!(
            samples.windows(2).all(|pair| pair[0] <= pair[1]),
            "{easing}: never backwards"
        );
        middles.insert(easing.clone(), samples[16]);
    }
    assert!((middles["linear"] - 0.5).abs() < 1e-9);
    assert!((middles["ease-in-out"] - 0.5).abs() < 1e-4, "ease-in-out is symmetric");
    assert!(middles["ease-in"] < 0.4 && middles["ease-out"] > 0.6, "{middles:?}");
    assert!(
        (middles["ease-out"] - 0.684_643).abs() < 1e-5,
        "cubic-bezier(0, 0, 0.58, 1)"
    );
    assert!(
        (middles["ease-in"] - 0.315_357).abs() < 1e-5,
        "cubic-bezier(0.42, 0, 1, 1)"
    );
    assert!(
        middles["ease"] > 0.7,
        "cubic-bezier(0.25, 0.1, 0.25, 1) is quick at the start"
    );
}

/// The timed writes after `mark`: `(key, seconds)`. The wire counts seconds; the script gave milliseconds.
fn timed_seconds(rig: &Rig, mark: usize) -> Vec<(String, f64)> {
    rig.comp
        .ops_since(mark)
        .into_iter()
        .filter_map(|op| match op {
            TxOp::Set {
                key,
                seconds: Some(seconds),
                ..
            } => Some((key, seconds)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_duration_of_250_is_a_quarter_of_a_second_on_the_wire_for_every_api_that_takes_one() {
    let mut rig = world(1, false, "new Layout()", "");
    exec(&mut rig, "g.c = gnoblin.clients.get('3');");
    settle(&mut rig);
    for (what, call) in [
        ("Client.set", "g.c.set({ opacity: 0.25 }, { duration: 250 })"),
        (
            "gnoblin.requestTransition",
            "gnoblin.requestTransition(() => { g.c.opacity = 0.75; }, { duration: 250 })",
        ),
    ] {
        let mark = rig.comp.sent_count();
        exec(&mut rig, call);
        settle(&mut rig);
        assert_eq!(
            timed_seconds(&rig, mark),
            [("opacity".to_owned(), 0.25)],
            "{what}: a duration of 250 is 250 ms"
        );
    }
}

#[test]
fn client_animate_is_gone_and_set_is_the_one_timed_write_of_a_client() {
    let mut rig = world(1, false, "new Layout()", "");
    assert_eq!(
        rig.eval("typeof gnoblin.clients.get('3').animate"),
        "undefined",
        "the specification has no `animate`: a second engine's entry point"
    );
    assert_eq!(rig.eval("typeof gnoblin.clients.get('3').set"), "function");
    // The transition engine owns every timed write, so a newer write takes the property from an older one and the
    // older transition ends cancelled.
    exec(
        &mut rig,
        "g.c = gnoblin.clients.get('3'); g.first = g.c.set({ opacity: 0.5 }, { duration: 250 });\n\
         g.first.finished.then((r) => { g.cancelled = r.cancelled; });",
    );
    settle(&mut rig);
    exec(&mut rig, "g.c.set({ opacity: 0.25 }, { duration: 250 });");
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(g.cancelled)"),
        "true",
        "the newest transition supersedes the older one"
    );
}

#[test]
fn a_duration_that_looks_like_seconds_is_not_read_as_seconds() {
    let mut rig = world(1, false, "new Layout()", "");
    exec(&mut rig, "g.c = gnoblin.clients.get('3');");
    settle(&mut rig);
    for (what, call) in [
        ("Client.set", "g.c.set({ opacity: 0.25 }, { duration: 0.25 })"),
        (
            "gnoblin.requestTransition",
            "gnoblin.requestTransition(() => { g.c.opacity = 0.75; }, { duration: 0.25 })",
        ),
    ] {
        let mark = rig.comp.sent_count();
        exec(&mut rig, call);
        settle(&mut rig);
        let timed = timed_seconds(&rig, mark);
        assert_eq!(timed.len(), 1, "{what}: one timed write: {timed:?}");
        assert!(
            (timed[0].1 - 0.000_25).abs() < 1e-9,
            "{what}: 0.25 is a quarter of a millisecond, not a quarter of a second: {timed:?}"
        );
    }
}

#[test]
fn a_workspace_slide_lasts_the_milliseconds_it_is_given() {
    for (given, seconds) in [(250.0, 0.25), (0.25, 0.000_25)] {
        let mut rig = world(1, false, "new Layout()", "");
        let mark = rig.comp.sent_count();
        exec(
            &mut rig,
            &format!("gnoblin.workspaces.show('2', {{ duration: {given} }});"),
        );
        settle(&mut rig);
        let slides: Vec<f64> = timed_seconds(&rig, mark)
            .into_iter()
            .filter(|(key, _)| key == "x")
            .map(|(_, seconds)| seconds)
            .collect();
        assert!(!slides.is_empty(), "a slide writes the windows' x over time");
        assert!(
            slides.iter().all(|got| (got - seconds).abs() < 1e-9),
            "show({given}): {slides:?} is not {seconds} seconds"
        );
    }
}

#[test]
fn motion_and_the_timers_count_milliseconds_too() {
    let mut rig = world(1, false, "new Layout()", "");
    assert_eq!(
        rig.eval("String(gnoblin.motion(250))"),
        "250",
        "motion passes milliseconds through"
    );
    assert_eq!(
        rig.eval("typeof gnoblin.after"),
        "undefined",
        "the seconds spelling of a timer is gone: `gnoblin.timers.after` is the one"
    );
    exec(&mut rig, "g.ran = 0; gnoblin.timers.after(250, () => { g.ran += 1; });");
    settle(&mut rig);
    rig.advance(240);
    settle(&mut rig);
    assert_eq!(rig.eval("String(g.ran)"), "0", "250 is not yet 240 ms");
    rig.advance(20);
    settle(&mut rig);
    assert_eq!(rig.eval("String(g.ran)"), "1", "and is over at 260 ms");
}
