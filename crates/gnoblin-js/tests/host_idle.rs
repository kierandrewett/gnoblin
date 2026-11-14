//! `gnoblin.idle` (`data/js/gnoblin/idle.ts`, `src/host_idle.rs`) over a fake compositor and a fake idle connection.
//!
//! The cases are those of `docs/shell-api/idle.md` that the compositor's reading can back: the validation of a watch,
//! its states (not idle when it starts, idle once, active again, idle again), a watch of its own time each, the
//! inhibitor that delays idleness, `cancel` and its `onClosed`, the lifetime of the connection (opened when a script
//! wants one, closed when it does not, every hold given back when the generation is retired) and the failures first:
//! no connection, a connection that is lost, a hold the compositor refuses, a handler that throws.
//!
//! The real compositor's `idle` object is read by `crates/gnoblin/tests/js_shell.rs`.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use gnoblin_js::{JsIdleLink, JsIdleReading, JsLinkError, JsShell};

mod common;
use common::{FakeCompositor, honest_config};

#[derive(Default)]
struct Idle {
    /// The reading the next poll returns.
    reading: Option<JsIdleReading>,
    /// Every call the shell made, one line each.
    calls: Vec<String>,
    /// How many connections were opened and how many dropped.
    opened: u32,
    closed: u32,
    fail_poll: Option<String>,
    refuse_inhibit: bool,
}

#[derive(Clone, Default)]
struct FakeIdle(Rc<RefCell<Idle>>);

struct FakeIdleLink(FakeIdle);

impl Drop for FakeIdleLink {
    fn drop(&mut self) {
        self.0.0.borrow_mut().closed += 1;
    }
}

impl JsIdleLink for FakeIdleLink {
    fn poll(&mut self) -> Result<Option<JsIdleReading>, JsLinkError> {
        let mut idle = self.0.0.borrow_mut();
        if let Some(reason) = idle.fail_poll.take() {
            return Err(JsLinkError(reason));
        }
        Ok(idle.reading.take())
    }

    fn inhibit(&mut self, holder: &str) -> Result<(), JsLinkError> {
        let mut idle = self.0.0.borrow_mut();
        idle.calls.push(format!("inhibit {holder}"));
        if idle.refuse_inhibit {
            return Err(JsLinkError("too-many-inhibitors".to_owned()));
        }
        Ok(())
    }

    fn uninhibit(&mut self, holder: &str) -> Result<(), JsLinkError> {
        self.0.0.borrow_mut().calls.push(format!("uninhibit {holder}"));
        Ok(())
    }
}

impl FakeIdle {
    /// What the compositor publishes next.
    fn say(&self, idle_ms: u64, inhibitors: u64) {
        self.0.borrow_mut().reading = Some(JsIdleReading { idle_ms, inhibitors });
    }

    fn calls(&self) -> Vec<String> {
        self.0.borrow().calls.clone()
    }

    fn opened(&self) -> u32 {
        self.0.borrow().opened
    }

    fn closed(&self) -> u32 {
        self.0.borrow().closed
    }
}

struct Rig {
    shell: JsShell,
    idle: FakeIdle,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }

    /// The compositor says `idle_ms` and `inhibitors`, and one frame carries it to the script.
    fn hear(&mut self, idle_ms: u64, inhibitors: u64) -> Vec<String> {
        self.idle.say(idle_ms, inhibitors);
        self.frame()
    }

    fn log(&mut self) -> String {
        self.eval("log.join(',')")
    }
}

/// A shell over a fake compositor and a fake idle connection, with `body` as its `init.ts`. `connected` says whether the
/// shell has a way to open the connection at all.
fn rig_with(body: &str, connected: bool) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        format!("const g = globalThis as any;\ng.g = g;\ng.log = [];\n{body}\n"),
    )
    .expect("init.ts is written");
    let idle = FakeIdle::default();
    let mut shell = JsShell::new(Box::new(FakeCompositor::new()), honest_config(&root)).expect("the shell boots");
    if connected {
        let state = idle.clone();
        shell.set_idle_connector(Box::new(move || {
            state.0.borrow_mut().opened += 1;
            Ok(Box::new(FakeIdleLink(state.clone())) as Box<dyn JsIdleLink>)
        }));
    }
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Rig { shell, idle, _dir: dir }
}

fn rig(body: &str) -> Rig {
    rig_with(body, true)
}

const ONE_WATCH: &str = "g.w = gnoblin.idle.watch(300, {\n\
    onIdle: (w) => g.log.push('idle'),\n\
    onResume: (w) => g.log.push('resume'),\n\
    onClosed: (w, why) => g.log.push('closed:' + why),\n\
});";

// --- the states of a watch ---------------------------------------------------------------------------------------

#[test]
fn a_watch_is_not_idle_when_it_starts_and_runs_idle_once_when_the_session_has_been_idle_for_its_time() {
    let mut rig = rig(ONE_WATCH);
    assert_eq!(rig.eval("g.w.idle"), "false");
    assert_eq!(rig.eval("g.w.alive"), "true");
    assert_eq!(rig.eval("g.w.ms"), "300");
    assert_eq!(rig.eval("g.w.id"), "idle-1");
    assert!(rig.hear(299, 0).is_empty());
    assert_eq!(rig.log(), "", "299 ms is short of 300");
    rig.hear(300, 0);
    assert_eq!(rig.log(), "idle");
    assert_eq!(rig.eval("g.w.idle"), "true");
    rig.hear(5_000, 0);
    assert_eq!(
        rig.log(),
        "idle",
        "an idle period runs idle once, however long it lasts"
    );
}

#[test]
fn input_after_an_idle_period_runs_resume_and_the_watch_counts_again() {
    let mut rig = rig(ONE_WATCH);
    rig.hear(400, 0);
    rig.hear(0, 0);
    assert_eq!(rig.log(), "idle,resume");
    assert_eq!(rig.eval("g.w.idle"), "false");
    rig.hear(100, 0);
    assert_eq!(rig.log(), "idle,resume", "the watch is counting again, not idle");
    rig.hear(310, 0);
    assert_eq!(
        rig.log(),
        "idle,resume,idle",
        "every idle period runs idle until the watch is cancelled"
    );
}

#[test]
fn each_watch_has_its_own_time() {
    let mut rig = rig("g.a = gnoblin.idle.watch(100, { onIdle: () => g.log.push('a') });\n\
         g.b = gnoblin.idle.watch(500, { onIdle: () => g.log.push('b'), onResume: () => g.log.push('b-back') });");
    rig.hear(150, 0);
    assert_eq!(rig.log(), "a");
    rig.hear(600, 0);
    assert_eq!(rig.log(), "a,b");
    rig.hear(0, 0);
    assert_eq!(rig.log(), "a,b,b-back");
}

#[test]
fn a_watch_made_while_the_session_is_idle_hears_it_at_once() {
    let mut rig =
        rig("g.later = () => gnoblin.idle.watch(50, { onIdle: () => g.log.push('late') });\ngnoblin.idle.watch(10);");
    rig.hear(1_000, 0);
    rig.eval("g.later()");
    rig.frame();
    assert_eq!(
        rig.log(),
        "late",
        "the reading the shell holds decides the new watch's first state"
    );
}

// --- inhibitors --------------------------------------------------------------------------------------------------

#[test]
fn an_inhibitor_delays_idle_and_one_that_appears_while_idle_resumes_the_watch() {
    let mut rig = rig(ONE_WATCH);
    rig.hear(1_000, 1);
    assert_eq!(
        rig.log(),
        "",
        "a held session is not idle, as it does not blank the screen"
    );
    rig.hear(1_100, 0);
    assert_eq!(rig.log(), "idle", "the hold ending lets the idle time count");
    rig.hear(1_200, 1);
    assert_eq!(rig.log(), "idle,resume");
}

#[test]
fn a_hold_is_sent_once_per_holder_and_given_back_with_release() {
    let mut rig = rig(
        "g.h = gnoblin.idle.inhibit('player');\ng.same = gnoblin.idle.inhibit('player');\ng.keep = gnoblin.idle.inhibit('other');",
    );
    assert_eq!(
        rig.eval("g.h === g.same"),
        "true",
        "holding again under a name returns the hold"
    );
    assert_eq!(rig.eval("g.h.alive"), "true");
    assert_eq!(rig.eval("g.h.holder"), "player");
    rig.frame();
    assert_eq!(rig.idle.calls(), ["inhibit player", "inhibit other"]);
    rig.eval("g.h.release()");
    rig.eval("g.h.release()");
    rig.frame();
    assert_eq!(
        rig.idle.calls(),
        ["inhibit player", "inhibit other", "uninhibit player"]
    );
    assert_eq!(rig.eval("g.h.alive"), "false");
    assert_eq!(rig.eval("g.keep.alive"), "true");
}

#[test]
fn a_hold_the_compositor_refuses_is_not_a_hold_and_the_failure_is_reported() {
    let mut rig = rig("g.h = gnoblin.idle.inhibit('player');");
    rig.idle.0.borrow_mut().refuse_inhibit = true;
    let diagnostics = rig.frame();
    assert!(
        diagnostics
            .iter()
            .any(|line| line.contains("idle.inhibit(player)") && line.contains("too-many-inhibitors")),
        "{diagnostics:?}"
    );
    assert_eq!(rig.eval("g.h.alive"), "false");
}

// --- cancel and validation ---------------------------------------------------------------------------------------

#[test]
fn cancel_ends_the_watch_runs_closed_once_and_the_watch_hears_nothing_more() {
    let mut rig = rig(ONE_WATCH);
    rig.hear(400, 0);
    rig.eval("g.w.cancel()");
    rig.eval("g.w.cancel()");
    rig.frame();
    assert_eq!(rig.log(), "idle,closed:cancelled");
    assert_eq!(rig.eval("g.w.alive"), "false");
    assert_eq!(rig.eval("g.w.idle"), "false");
    rig.hear(0, 0);
    rig.hear(900, 0);
    assert_eq!(rig.log(), "idle,closed:cancelled");
}

#[test]
fn a_watch_may_cancel_itself_in_its_own_handler() {
    let mut rig = rig("g.w = gnoblin.idle.watch(100, { onIdle: (w) => { g.log.push('idle'); w.cancel(); } });");
    rig.hear(200, 0);
    rig.hear(0, 0);
    rig.hear(200, 0);
    assert_eq!(rig.log(), "idle");
    assert_eq!(rig.eval("gnoblin.idle.watches.length"), "0");
}

#[test]
fn the_time_is_a_number_above_zero_and_at_most_a_day_and_a_runtime_has_256_watches() {
    let mut rig =
        rig("g.try = (ms: any) => { try { gnoblin.idle.watch(ms); return 'ok'; } catch (e: any) { return e.name; } };");
    for ms in ["0", "-1", "NaN", "Infinity", "86400001", "'5'", "null", "undefined"] {
        assert_eq!(rig.eval(&format!("g.try({ms})")), "RangeError", "{ms}");
    }
    assert_eq!(rig.eval("g.try(0.5)"), "ok", "a fraction of a millisecond is allowed");
    assert_eq!(rig.eval("g.try(86400000)"), "ok", "a day is the longest");
    for _ in 0..254 {
        rig.eval("g.try(1000)");
    }
    assert_eq!(rig.eval("gnoblin.idle.watches.length"), "256");
    assert_eq!(rig.eval("g.try(1000)"), "RangeError", "the 257th is refused");
    rig.eval("gnoblin.idle.watches[0].cancel()");
    assert_eq!(rig.eval("g.try(1000)"), "ok", "a cancelled watch frees its place");
    assert_eq!(
        rig.eval(
            "(() => { try { gnoblin.idle.watch(10, { onIdle: 5 }); return 'ok'; } catch (e) { return e.name; } })()"
        ),
        "TypeError"
    );
    assert_eq!(
        rig.eval("(() => { try { gnoblin.idle.inhibit(''); return 'ok'; } catch (e) { return e.name; } })()"),
        "TypeError"
    );
}

// --- the connection ----------------------------------------------------------------------------------------------

#[test]
fn the_connection_is_opened_when_a_script_first_wants_it_and_closed_when_it_has_no_watch_or_hold() {
    let mut rig = rig("g.make = () => { g.w = gnoblin.idle.watch(100); };");
    for _ in 0..3 {
        rig.frame();
    }
    assert_eq!(
        rig.idle.opened(),
        0,
        "a shell with no watch costs the compositor nothing"
    );
    rig.eval("g.make()");
    rig.frame();
    assert_eq!(rig.idle.opened(), 1);
    rig.frame();
    assert_eq!(rig.idle.opened(), 1, "one connection for any number of watches");
    assert_eq!(rig.idle.closed(), 0);
    rig.eval("g.w.cancel()");
    rig.frame();
    assert_eq!(
        rig.idle.closed(),
        1,
        "the last watch ends and the connection goes with it"
    );
    rig.eval("g.make()");
    rig.frame();
    assert_eq!(rig.idle.opened(), 2, "a later watch opens a new one");
}

#[test]
fn a_shell_with_no_idle_connection_says_so_once_and_its_watches_never_fire() {
    let mut rig = rig_with(ONE_WATCH, false);
    let first = rig.frame();
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("no idle connection"), "{first:?}");
    assert!(rig.frame().is_empty(), "reported once");
    assert_eq!(rig.log(), "");
}

#[test]
fn a_lost_connection_is_reported_once_and_an_idle_watch_is_active_again() {
    let mut rig = rig(ONE_WATCH);
    rig.hear(400, 0);
    assert_eq!(rig.log(), "idle");
    rig.idle.0.borrow_mut().fail_poll = Some("the socket closed".to_owned());
    let diagnostics = rig.frame();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("the socket closed"), "{diagnostics:?}");
    assert_eq!(rig.log(), "idle,resume", "nothing says the session is idle any more");
    assert!(
        rig.frame().is_empty(),
        "reported once, and not opened again behind the script's back"
    );
    assert_eq!(rig.idle.opened(), 1);
}

#[test]
fn a_lost_connection_ends_the_scripts_holds_because_the_compositor_gave_them_back() {
    // The compositor keeps a hold for the connection that made it, so a connection that ends takes its holds along; a
    // script must not keep believing in one.
    let mut rig = rig("g.h = gnoblin.idle.inhibit('player');\ng.w = gnoblin.idle.watch(100);");
    rig.frame();
    assert_eq!(rig.eval("g.h.alive"), "true");
    rig.idle.0.borrow_mut().fail_poll = Some("the socket closed".to_owned());
    let diagnostics = rig.frame();
    assert!(diagnostics[0].contains("given back its holds"), "{diagnostics:?}");
    assert_eq!(rig.eval("g.h.alive"), "false");
    rig.eval("g.h.release()");
    rig.shell.retire().expect("a retire");
    assert_eq!(
        rig.idle.calls(),
        ["inhibit player"],
        "nothing is given back for a hold that is already gone"
    );
}

#[test]
fn retiring_gives_every_hold_back_and_closes_the_connection() {
    let mut rig =
        rig("g.h = gnoblin.idle.inhibit('player');\ngnoblin.idle.inhibit('other');\ng.w = gnoblin.idle.watch(100);");
    rig.frame();
    assert_eq!(rig.idle.opened(), 1);
    rig.shell.retire().expect("a retire");
    let mut calls = rig.idle.calls();
    let back: Vec<String> = calls.split_off(2);
    assert_eq!(calls, ["inhibit player", "inhibit other"]);
    assert_eq!(
        back,
        ["uninhibit player", "uninhibit other"],
        "a retired generation gives its holds back"
    );
    assert_eq!(rig.idle.closed(), 1);
}

// --- failures in the script --------------------------------------------------------------------------------------

#[test]
fn a_handler_that_throws_is_reported_once_and_the_next_watch_still_runs() {
    let mut rig = rig(
        "gnoblin.idle.watch(100, { onIdle: () => { throw new Error('first broke'); } });\n\
         gnoblin.idle.watch(100, { onIdle: () => g.log.push('second') });",
    );
    let diagnostics = rig.hear(200, 0);
    assert!(
        diagnostics.iter().any(|line| line.contains("first broke")),
        "{diagnostics:?}"
    );
    assert_eq!(rig.log(), "second");
    rig.hear(0, 0);
    let again = rig.hear(300, 0);
    assert!(again.is_empty(), "the failure site is reported once: {again:?}");
}
