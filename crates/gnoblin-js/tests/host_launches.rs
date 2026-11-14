//! `gnoblin.launches` (`data/js/gnoblin/launches.ts`) over a fake compositor: the cases of
//! `docs/shell-api/launch-feedback.md`.
//!
//! A launch is pending until a window of its application appears or takes focus (`started`), its deadline passes
//! (`timed_out`) or the shell finishes it (`ended`); it leaves `pending` once. The registry holds 64 records, a token in
//! use is replaced, matching lowercases the application and drops a `.desktop` suffix. The validation of `begin` is
//! the page's table. Failures first: a launch the registry cannot take, a handler that throws, a window that existed
//! before the launch.
//!
//! The real compositor's windows answer a launch in `crates/gnoblin/tests/js_shell.rs`.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use gnoblin_js::{JsShell, JsShellConfig};
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::FakeCompositor;

struct Rig {
    shell: JsShell,
    comp: FakeCompositor,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    /// Frames until the compositor's echo of the last write has been taken in, so a window's events have run.
    fn settle(&mut self) -> Vec<String> {
        let mut all = Vec::new();
        for _ in 0..4 {
            all.extend(self.frame());
        }
        all
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }

    fn log(&mut self) -> String {
        self.eval("log.join(',')")
    }

    /// Frames until `check` holds, within a deadline: a launch's own deadline is real time.
    #[track_caller]
    fn until(&mut self, what: &str, mut check: impl FnMut(&mut Rig) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !check(self) {
            assert!(Instant::now() < deadline, "never reached: {what}");
            self.frame();
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn rig(body: &str, windows: &[(u64, &str)]) -> Rig {
    let comp = FakeCompositor::new();
    for (id, app) in windows {
        comp.add_window(*id, "title", app);
    }
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        format!("const g = globalThis as any;\ng.g = g;\ng.log = [];\n{body}\n"),
    )
    .expect("init.ts is written");
    let mut shell = JsShell::new(Box::new(comp.clone()), JsShellConfig::new(&root)).expect("the shell boots");
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Rig { shell, comp, _dir: dir }
}

const BEGIN: &str = "g.l = gnoblin.launches.begin('t1', 'org.example.App', {\n\
    timeoutMs: 5000,\n\
    onChanged: (l, fields) => g.log.push('changed:' + fields.join('+')),\n\
    onStarted: (l) => g.log.push('started'),\n\
    onTimedOut: (l) => g.log.push('timed-out'),\n\
    onClosed: (l, why) => g.log.push('closed:' + why),\n\
});";

// --- the states of a launch --------------------------------------------------------------------------------------

#[test]
fn a_launch_is_pending_at_once_with_what_was_asked_and_revision_zero() {
    let mut rig = rig(BEGIN, &[]);
    assert_eq!(rig.eval("g.l.token"), "t1");
    assert_eq!(rig.eval("g.l.application"), "org.example.App");
    assert_eq!(rig.eval("g.l.state"), "pending");
    assert_eq!(rig.eval("g.l.alive"), "true");
    assert_eq!(rig.eval("g.l.revision"), "0");
    assert_eq!(rig.eval("g.l.timeoutMs"), "5000");
    assert_eq!(rig.eval("typeof g.l.startedAt"), "number");
    assert_eq!(rig.eval("gnoblin.launches.list().length"), "1");
    assert_eq!(
        rig.eval("gnoblin.launches.list()[0] === g.l"),
        "true",
        "the same object for as long as it exists"
    );
    assert_eq!(
        rig.eval("gnoblin.launches.begin('t2', 'x').timeoutMs"),
        "3000",
        "3000 ms is the default"
    );
}

#[test]
fn a_window_of_the_application_starts_the_launch_and_closed_runs_last() {
    let mut rig = rig(BEGIN, &[]);
    rig.comp.create_window(3, "Hello", "org.example.App");
    rig.settle();
    assert_eq!(rig.log(), "changed:state+alive+revision,started,closed:started");
    assert_eq!(rig.eval("g.l.state"), "started");
    assert_eq!(rig.eval("g.l.alive"), "false");
    assert_eq!(rig.eval("g.l.revision"), "1");
    rig.comp.create_window(4, "Another", "org.example.App");
    rig.settle();
    assert_eq!(rig.eval("g.l.revision"), "1", "a launch never leaves its end state");
}

#[test]
fn matching_lowercases_both_names_and_drops_a_desktop_suffix() {
    let mut rig = rig(
        "g.a = gnoblin.launches.begin('a', 'Org.Example.App.desktop');\n\
         g.b = gnoblin.launches.begin('b', 'org.example.other');\n\
         g.c = gnoblin.launches.begin('c', 'org.example.third.desktop');",
        &[],
    );
    rig.comp.create_window(3, "A", "org.example.app");
    rig.comp.create_window(4, "B", "ORG.EXAMPLE.OTHER.desktop");
    rig.settle();
    assert_eq!(rig.eval("g.a.state"), "started");
    assert_eq!(rig.eval("g.b.state"), "started");
    assert_eq!(rig.eval("g.c.state"), "pending", "no window of that application came");
}

#[test]
fn a_window_that_existed_before_the_launch_does_not_start_it() {
    let mut rig = rig(BEGIN, &[(3, "org.example.App")]);
    rig.settle();
    assert_eq!(
        rig.eval("g.l.state"),
        "pending",
        "a launch waits for a window to appear or take focus"
    );
}

#[test]
fn a_window_that_takes_focus_or_names_its_application_later_starts_the_launch() {
    let mut rig = rig(
        "g.f = gnoblin.launches.begin('focus', 'org.example.Focus');\n\
         g.n = gnoblin.launches.begin('name', 'org.example.Late');",
        &[(3, "org.example.Focus"), (4, "")],
    );
    rig.settle();
    assert_eq!(rig.eval("g.f.state"), "pending");
    rig.comp.external_set(3, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(rig.eval("g.f.state"), "started", "focus on a matching window");
    rig.comp
        .external_set(4, "app_id", MirrorValue::Text("org.example.Late".to_owned()));
    rig.settle();
    assert_eq!(
        rig.eval("g.n.state"),
        "started",
        "a window that names its application after it mapped"
    );
}

#[test]
fn a_window_starts_only_the_oldest_pending_launch_it_matches() {
    let mut rig = rig(
        "g.first = gnoblin.launches.begin('one', 'org.example.App');\n\
         g.second = gnoblin.launches.begin('two', 'org.example.App');",
        &[],
    );
    rig.comp.create_window(3, "A", "org.example.App");
    rig.settle();
    assert_eq!(rig.eval("g.first.state + ',' + g.second.state"), "started,pending");
    rig.comp.create_window(4, "B", "org.example.App");
    rig.settle();
    assert_eq!(rig.eval("g.second.state"), "started");
}

#[test]
fn the_deadline_times_the_launch_out_and_a_later_window_changes_nothing() {
    let mut rig = rig(
        "g.l = gnoblin.launches.begin('t1', 'org.example.App', {\n\
            timeoutMs: 100,\n\
            onTimedOut: () => g.log.push('timed-out'),\n\
            onStarted: () => g.log.push('started'),\n\
            onClosed: (l, why) => g.log.push('closed:' + why),\n\
        });",
        &[],
    );
    rig.until("the launch timed out", |rig| rig.eval("g.l.state") == "timed_out");
    rig.settle();
    assert_eq!(rig.log(), "timed-out,closed:timed_out");
    assert_eq!(rig.eval("g.l.revision"), "1");
    rig.comp.create_window(3, "Late", "org.example.App");
    rig.settle();
    assert_eq!(rig.log(), "timed-out,closed:timed_out", "a launch leaves pending once");
}

#[test]
fn finish_ends_a_pending_launch_once_and_leaves_a_finished_one_alone() {
    let mut rig = rig(BEGIN, &[]);
    rig.eval("g.l.finish(); g.l.finish();");
    rig.settle();
    assert_eq!(rig.log(), "changed:state+alive+revision,closed:ended");
    assert_eq!(rig.eval("g.l.state"), "ended");
    assert_eq!(rig.eval("g.l.revision"), "1");
    rig.comp.create_window(3, "Late", "org.example.App");
    rig.settle();
    assert_eq!(rig.eval("g.l.state"), "ended");
}

#[test]
fn a_finish_after_the_launch_started_is_unchanged() {
    let mut rig = rig(BEGIN, &[]);
    rig.comp.create_window(3, "A", "org.example.App");
    rig.settle();
    rig.eval("g.l.finish()");
    rig.settle();
    assert_eq!(rig.eval("g.l.state"), "started");
    assert_eq!(rig.eval("g.l.revision"), "1");
}

// --- tokens and the registry -------------------------------------------------------------------------------------

#[test]
fn beginning_a_token_in_use_ends_the_earlier_launch_with_the_reason_replaced() {
    let mut rig = rig(
        "g.old = gnoblin.launches.begin('t', 'org.example.App', { onClosed: (l, why) => g.log.push('old:' + why) });\n\
         g.fresh = gnoblin.launches.begin('t', 'org.example.App', { onClosed: (l, why) => g.log.push('new:' + why) });",
        &[],
    );
    rig.settle();
    assert_eq!(rig.log(), "old:replaced");
    assert_eq!(rig.eval("g.old.state"), "ended");
    assert_eq!(rig.eval("g.fresh.state"), "pending");
    assert_eq!(rig.eval("gnoblin.launches.list().length"), "1");
    assert_eq!(rig.eval("gnoblin.launches.list()[0] === g.fresh"), "true");
}

#[test]
fn the_registry_holds_64_launches_drops_the_oldest_finished_and_refuses_when_all_are_pending() {
    let mut rig = rig(
        "for (let i = 0; i < 64; i++) { gnoblin.launches.begin('t' + i, 'app' + i); }\n\
         g.extra = gnoblin.launches.begin('extra', 'app', { onClosed: (l, why) => g.log.push('extra:' + why) });",
        &[],
    );
    rig.settle();
    assert_eq!(
        rig.eval("g.extra.state"),
        "ended",
        "all 64 are pending, so a new launch cannot be taken"
    );
    assert!(
        rig.log().starts_with("extra:the registry holds 64 pending launches"),
        "{}",
        rig.log()
    );
    assert_eq!(rig.eval("gnoblin.launches.list().length"), "64");
    rig.eval("gnoblin.launches.list()[10].finish()");
    rig.eval("g.fits = gnoblin.launches.begin('fits', 'app')");
    assert_eq!(rig.eval("g.fits.state"), "pending", "a finished record makes room");
    assert_eq!(rig.eval("gnoblin.launches.list().length"), "64");
    assert_eq!(
        rig.eval("gnoblin.launches.list().some((l) => l.token === 't10')"),
        "false",
        "the oldest finished one went"
    );
}

// --- validation --------------------------------------------------------------------------------------------------

#[test]
fn begin_takes_a_token_up_to_128_characters_an_application_up_to_512_and_a_timeout_of_100_to_10000() {
    let mut rig = rig(
        "g.try = (a: any, b: any, c?: any) => { try { gnoblin.launches.begin(a, b, c); return 'ok'; } catch (e: any) { return e.name; } };",
        &[],
    );
    assert_eq!(rig.eval("g.try('t', 'app')"), "ok");
    assert_eq!(rig.eval("g.try('', 'app')"), "TypeError");
    assert_eq!(rig.eval("g.try('x'.repeat(129), 'app')"), "TypeError");
    assert_eq!(rig.eval("g.try('x'.repeat(128), 'app')"), "ok");
    assert_eq!(rig.eval("g.try('t3', '')"), "TypeError");
    assert_eq!(rig.eval("g.try('t3', 'x'.repeat(513))"), "TypeError");
    assert_eq!(rig.eval("g.try('t3', 'x'.repeat(512))"), "ok");
    assert_eq!(rig.eval("g.try(5, 'app')"), "TypeError");
    assert_eq!(rig.eval("g.try('t4', 'app', { timeoutMs: 99 })"), "RangeError");
    assert_eq!(rig.eval("g.try('t4', 'app', { timeoutMs: 10001 })"), "RangeError");
    assert_eq!(rig.eval("g.try('t4', 'app', { timeoutMs: 1.5 })"), "RangeError");
    assert_eq!(rig.eval("g.try('t4', 'app', { timeoutMs: 100 })"), "ok");
    assert_eq!(rig.eval("g.try('t5', 'app', { timeoutMs: 10000 })"), "ok");
    assert_eq!(rig.eval("g.try('t6', 'app', { onStarted: 1 })"), "TypeError");
    assert_eq!(rig.eval("g.try('t6', 'app', 5)"), "TypeError");
}

// --- failures in the script --------------------------------------------------------------------------------------

#[test]
fn a_handler_that_throws_is_reported_once_and_the_next_handler_still_runs() {
    let mut rig = rig(
        "g.l = gnoblin.launches.begin('t', 'org.example.App', {\n\
            onStarted: () => { throw new Error('started broke'); },\n\
            onClosed: (l, why) => g.log.push('closed:' + why),\n\
        });",
        &[],
    );
    rig.comp.create_window(3, "A", "org.example.App");
    let diagnostics = rig.settle();
    assert_eq!(
        diagnostics.iter().filter(|line| line.contains("started broke")).count(),
        1,
        "{diagnostics:?}"
    );
    assert_eq!(rig.log(), "closed:started");
}
