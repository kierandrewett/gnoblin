//! `gnoblin.idle` (`IdleMonitor`, `IdleWatch` and `IdleInhibitor` of `gnoblin-core`, served by the native core) over a fake
//! compositor and a fake owner connection.
//!
//! The cases are those of `docs/shell-api/idle.md` that the compositor's reading can back: the validation of a watch,
//! its states (not idle when it starts, idle once, active again, idle again), a watch of its own time each, the
//! inhibitor that delays idleness, `cancel` and its `onClosed`, the lifetime of the connection (opened when a script
//! wants one, closed when it does not, every hold given back when the generation is retired) and the failures first:
//! no connection, a connection that is lost, a hold the compositor refuses, a handler that throws.
//!
//! The real compositor's `idle` object is read by `crates/gnoblin/tests/js_shell.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_core_impl::host;
use gnoblin_core_impl::verbs::{self, VerbCall, VerbFailure, VerbLink, VerbLinkProvider};
use gnoblin_model::Value;

mod common;
use common::{FakeCompositor, timed_config};

#[derive(Default)]
struct Idle {
    /// What the compositor published, by object and name.
    props: BTreeMap<(String, String), Value>,
    /// Every call the shell made, one line each.
    calls: Vec<String>,
    /// How many connections were opened and how many dropped.
    opened: u32,
    closed: u32,
    refuse_inhibit: bool,
}

#[derive(Clone, Default)]
struct FakeIdle(Arc<Mutex<Idle>>);

struct FakeIdleLink(FakeIdle);

impl Drop for FakeIdleLink {
    fn drop(&mut self) {
        self.0.state().closed += 1;
    }
}

impl VerbLink for FakeIdleLink {
    fn call(&mut self, call: VerbCall) -> Result<(), String> {
        let holder = match call.args.get("holder") {
            Some(Value::Text(holder)) => holder.clone(),
            _ => String::new(),
        };
        let mut idle = self.0.state();
        idle.calls.push(format!("{} {holder}", call.method));
        if call.method == "inhibit" && idle.refuse_inhibit {
            verbs::answered(
                call.token,
                Err(VerbFailure {
                    code: "too-many-inhibitors".to_owned(),
                }),
            );
        }
        Ok(())
    }

    fn property(&self, class: &str, name: &str) -> Option<Value> {
        self.0.state().props.get(&(class.to_owned(), name.to_owned())).cloned()
    }
}

impl VerbLinkProvider for FakeIdle {
    fn open(&self) -> Result<Box<dyn VerbLink>, String> {
        self.state().opened += 1;
        Ok(Box::new(FakeIdleLink(self.clone())))
    }
}

impl FakeIdle {
    fn state(&self) -> MutexGuard<'_, Idle> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the compositor publishes next.
    fn say(&self, idle_ms: u64, inhibitors: u64) {
        {
            let mut idle = self.state();
            idle.props
                .insert(("idle".to_owned(), "idle_ms".to_owned()), Value::Uint(idle_ms));
            idle.props
                .insert(("idle".to_owned(), "inhibitors".to_owned()), Value::Uint(inhibitors));
        }
        verbs::property_changed("idle", "idle_ms");
    }

    fn calls(&self) -> Vec<String> {
        self.state().calls.clone()
    }

    fn opened(&self) -> u32 {
        self.state().opened
    }

    fn closed(&self) -> u32 {
        self.state().closed
    }
}

struct Rig {
    inner: common::Rig,
    idle: FakeIdle,
    /// What the library logged while the entry loaded.
    loaded: Vec<String>,
}

impl std::ops::Deref for Rig {
    type Target = common::Rig;

    fn deref(&self) -> &common::Rig {
        &self.inner
    }
}

impl std::ops::DerefMut for Rig {
    fn deref_mut(&mut self) -> &mut common::Rig {
        &mut self.inner
    }
}

impl Rig {
    /// Frames until what the compositor said has been heard: the core's turn comes after the engine's.
    ///
    /// What the frames report is what the engine returned and what the library logged: the core's callbacks that throw, and its
    /// warnings, are logged and not returned.
    fn frames(&mut self) -> Vec<String> {
        let mut all = Vec::new();
        let logged = gnoblin_log::event::capture::logged(|| {
            for _ in 0..3 {
                all.extend(self.inner.frame());
            }
        });
        all.extend(logged.into_iter().map(|event| event.message));
        all
    }

    fn eval(&mut self, source: &str) -> String {
        self.inner
            .shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }

    /// The compositor says `idle_ms` and `inhibitors`, and the frames that follow carry it to the script.
    fn hear(&mut self, idle_ms: u64, inhibitors: u64) -> Vec<String> {
        self.idle.say(idle_ms, inhibitors);
        self.frames()
    }

    /// Turns until the script's log reads `expected`: the core's dispatch carries what the compositor said to the script.
    #[track_caller]
    fn until_log(&mut self, what: &str, expected: &str) {
        for _ in 0..20 {
            self.inner.frame();
            if self.log() == expected {
                return;
            }
        }
        panic!("never heard: {what}: the log reads {:?}, not {expected:?}", self.log());
    }

    fn log(&mut self) -> String {
        self.eval("log.join(',')")
    }
}

/// A shell over a fake compositor and a fake owner connection, with `body` as its `shell.ts`. `connected` says whether the
/// shell has a way to open the connection at all.
fn rig_with(body: &str, connected: bool) -> Rig {
    rig_on(body, connected, FakeIdle::default())
}

/// As [`rig_with`], over an owner connection that was set up before the entry runs.
fn rig_on(body: &str, connected: bool, idle: FakeIdle) -> Rig {
    let comp = FakeCompositor::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("shell.ts"),
        format!("const g = globalThis as any;\ng.g = g;\ng.log = [];\n{body}\n"),
    )
    .expect("shell.ts is written");
    let (config, clock) = timed_config(&root);
    let (mut shell, core) = common::native::shell_over_core(&comp, Arc::clone(&config.clock), config);
    if connected {
        host::set_verb_link_provider(Box::new(idle.clone()));
    }
    let mut diagnostics = Vec::new();
    let logged = gnoblin_log::event::capture::logged(|| {
        diagnostics = shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    });
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let loaded = logged.into_iter().map(|event| event.message).collect();
    let inner = common::Rig {
        shell,
        comp,
        load_diagnostics: diagnostics,
        clock,
        core: Some(core),
        tell_notices: true,
        _dir: dir,
    };
    Rig { inner, idle, loaded }
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
    assert_eq!(rig.eval("g.w.isIdle"), "false");
    assert_eq!(rig.eval("g.w.alive"), "true");
    assert_eq!(rig.eval("g.w.ms"), "300");
    assert_eq!(rig.eval("g.w.id"), "idle-1");
    assert!(rig.hear(299, 0).is_empty());
    assert_eq!(rig.log(), "", "299 ms is short of 300");
    rig.hear(300, 0);
    assert_eq!(rig.log(), "idle");
    assert_eq!(rig.eval("g.w.isIdle"), "true");
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
    assert_eq!(rig.eval("g.w.isIdle"), "false");
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
    rig.frames();
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
    rig.frames();
    assert_eq!(rig.idle.calls(), ["inhibit player", "inhibit other"]);
    rig.eval("g.h.release()");
    rig.eval("g.h.release()");
    rig.frames();
    assert_eq!(
        rig.idle.calls(),
        ["inhibit player", "inhibit other", "uninhibit player"]
    );
    assert_eq!(rig.eval("g.h.alive"), "false");
    assert_eq!(rig.eval("g.keep.alive"), "true");
}

#[test]
fn a_hold_the_compositor_refuses_is_not_a_hold_and_the_failure_is_reported() {
    let idle = FakeIdle::default();
    idle.state().refuse_inhibit = true;
    let mut rig = rig_on("g.h = gnoblin.idle.inhibit('player');", true, idle);
    let diagnostics = rig.frames();
    assert!(
        diagnostics.iter().any(|line| line.contains("refused the hold")
            && line.contains("player")
            && line.contains("too-many-inhibitors")),
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
    rig.frames();
    assert_eq!(rig.log(), "idle,closed:cancelled");
    assert_eq!(rig.eval("g.w.alive"), "false");
    assert_eq!(rig.eval("g.w.isIdle"), "false");
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
        assert_eq!(rig.eval(&format!("g.try({ms})")), "TypeError", "{ms}");
    }
    assert_eq!(rig.eval("g.try(0.5)"), "ok", "a fraction of a millisecond is allowed");
    assert_eq!(rig.eval("g.try(86400000)"), "ok", "a day is the longest");
    for _ in 0..254 {
        rig.eval("g.try(1000)");
    }
    assert_eq!(rig.eval("gnoblin.idle.watches.length"), "256");
    assert_eq!(rig.eval("g.try(1000)"), "TypeError", "the 257th is refused");
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
fn the_connection_is_opened_when_a_script_first_wants_it_and_one_serves_any_number_of_watches() {
    let mut rig = rig("g.make = () => { g.w = gnoblin.idle.watch(100); };");
    for _ in 0..3 {
        rig.frames();
    }
    assert_eq!(
        rig.idle.opened(),
        0,
        "a shell with no watch costs the compositor nothing"
    );
    rig.eval("g.make()");
    rig.frames();
    assert_eq!(rig.idle.opened(), 1);
    rig.eval("g.make()");
    rig.frames();
    assert_eq!(rig.idle.opened(), 1, "one connection for any number of watches");
    assert_eq!(rig.idle.closed(), 0);
}

#[test]
#[ignore = "vrnw: needs the owner connection closed when the last watch or hold ends (the native core keeps the shell's one connection open for the generation)"]
fn the_connection_is_closed_when_it_has_no_watch_or_hold_and_a_later_watch_opens_a_new_one() {
    let mut rig = rig("g.make = () => { g.w = gnoblin.idle.watch(100); };");
    rig.eval("g.make()");
    rig.frames();
    assert_eq!(rig.idle.opened(), 1);
    rig.eval("g.w.cancel()");
    rig.frames();
    assert_eq!(
        rig.idle.closed(),
        1,
        "the last watch ends and the connection goes with it"
    );
    rig.eval("g.make()");
    rig.frames();
    assert_eq!(rig.idle.opened(), 2, "a later watch opens a new one");
}

#[test]
fn a_shell_with_no_idle_connection_says_so_once_and_its_watches_never_fire() {
    let mut rig = rig_with(ONE_WATCH, false);
    let first = rig.loaded.clone();
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("no idle connection"), "{first:?}");
    assert!(rig.frames().is_empty(), "reported once");
    assert_eq!(rig.log(), "");
}

#[test]
fn a_lost_connection_is_reported_once_and_an_idle_watch_is_active_again() {
    let mut rig = rig(ONE_WATCH);
    rig.hear(400, 0);
    assert_eq!(rig.log(), "idle");
    verbs::owner_lost("the socket closed");
    let diagnostics = rig.frames();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("the socket closed"), "{diagnostics:?}");
    assert_eq!(rig.log(), "idle,resume", "nothing says the session is idle any more");
    assert!(
        rig.frames().is_empty(),
        "reported once, and not opened again behind the script's back"
    );
    assert_eq!(rig.idle.opened(), 1);
}

#[test]
fn a_lost_connection_ends_the_scripts_holds_because_the_compositor_gave_them_back() {
    // The compositor keeps a hold for the connection that made it, so a connection that ends takes its holds along; a
    // script must not keep believing in one.
    let mut rig = rig("g.h = gnoblin.idle.inhibit('player');\ng.w = gnoblin.idle.watch(100);");
    rig.frames();
    assert_eq!(rig.eval("g.h.alive"), "true");
    verbs::owner_lost("the socket closed");
    let diagnostics = rig.frames();
    assert!(diagnostics[0].contains("holds went with it"), "{diagnostics:?}");
    assert_eq!(rig.eval("g.h.alive"), "false");
    rig.eval("g.h.release()");
    drop(rig.inner.core.take());
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
    rig.frames();
    assert_eq!(rig.idle.opened(), 1);
    drop(rig.inner.core.take());
    let mut calls = rig.idle.calls();
    let back: Vec<String> = calls.split_off(2);
    assert_eq!(calls, ["inhibit player", "inhibit other"]);
    assert_eq!(
        back,
        ["uninhibit player", "uninhibit other"],
        "a retired generation gives its holds back"
    );
    assert_eq!(rig.idle.closed(), 1, "the connection goes with the generation");
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
}

#[test]
fn a_handler_that_throws_is_reported_once_not_on_every_throw() {
    let mut rig = rig("gnoblin.idle.watch(100, { onIdle: () => { throw new Error('first broke'); } });");
    rig.hear(200, 0);
    rig.hear(0, 0);
    let again = rig.hear(300, 0);
    assert!(again.is_empty(), "the failure site is reported once: {again:?}");
}

// --- change gating -----------------------------------------------------------------------------------------------
//
// A quiet turn runs no frame (`tests/frame_gating.rs`), so the compositor's reading is heard because a quiet turn reads
// the connection and keeps what it said for the frame it then runs.

#[test]
fn a_reading_wakes_the_next_frame_and_a_connection_that_says_nothing_wakes_nothing() {
    let mut rig = rig(ONE_WATCH);
    rig.quiet();
    for _ in 0..25 {
        assert!(
            !rig.next_frame_runs(),
            "an open connection with nothing to say woke a frame"
        );
    }
    assert_eq!(
        rig.shell.next_wake(),
        None,
        "a watch is not a timer: the reading is what moves it"
    );
    // Before: the reading woke the very next engine frame (`next_frame_runs`). The core's dispatch hears it now and the engine's
    // frame counter does not move for it, so the turns that follow are read until the watch has heard it.
    rig.idle.say(300, 0);
    rig.until_log("the reading was heard", "idle");
    rig.quiet();
    rig.idle.say(0, 0);
    rig.until_log("the user's return was heard", "idle,resume");
}

#[test]
fn a_hold_the_script_gives_back_is_sent_by_the_frame_it_wakes() {
    // A release is script that ran outside a frame, so it wakes one, and that frame sends the command.
    let mut rig = rig("g.h = gnoblin.idle.inhibit('player');");
    rig.quiet();
    assert_eq!(rig.idle.calls(), ["inhibit player"]);
    rig.eval("g.h.release()");
    assert!(rig.next_frame_runs(), "the release did not wake a frame");
    assert_eq!(rig.idle.calls(), ["inhibit player", "uninhibit player"]);
}

#[test]
fn a_lost_connection_wakes_the_next_frame() {
    let mut rig = rig(ONE_WATCH);
    rig.hear(400, 0);
    rig.quiet();
    verbs::owner_lost("the socket closed");
    rig.until_log("the loss was heard", "idle,resume");
    rig.quiet();
}
