//! The host API that is not about windows: `gnoblin.session`, `gnoblin.runtime`, `gnoblin.process`, `gnoblin.timers`,
//! `gnoblin.on`, `gnoblin.find`, `gnoblin.focus` and `gnoblin.version` (`data/js/gnoblin/{operation,process,session}.ts`
//! over `crates/gnoblin-js/src/host_{api,process,session}.rs`).
//!
//! The cases are the Lua host's: `crates/gnoblin-lua/tests/failure_modes.rs` (operations, timers, events, find,
//! process, version, session). A case keeps the Lua case's inputs and expected values; where the JavaScript API changed
//! a word (`timers.after` takes milliseconds and returns a timer, `by_id` is `byId`, an event field is camelCase, the
//! `Operation` is plain TypeScript and its callbacks are queued calls), the case says so.
//!
//! The session connection is a fake here: a [`JsSessionLink`] that records the verbs the script began and plays back
//! the events a compositor would send. Processes are real children of the test. What the real compositor does with a
//! session call is `crates/gnoblin/tests/js_shell.rs`.

mod common;

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use common::{FakeCompositor, Rig, rig_setup};
use gnoblin_js::{
    JsLinkError, JsOperationError, JsSessionEvent, JsSessionLink, JsSessionStatus, JsSessionVerb, JsShell,
    JsShellConfig, strip_types,
};
use gnoblin_tree_client::tree_mirror::MirrorValue;

const PATIENCE: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Fake {
    status: JsSessionStatus,
    began: Vec<JsSessionVerb>,
    inbox: Vec<JsSessionEvent>,
    next: u64,
    refuse: Option<String>,
}

#[derive(Clone, Default)]
struct FakeSession(Rc<RefCell<Fake>>);

impl JsSessionLink for FakeSession {
    fn status(&self) -> JsSessionStatus {
        self.0.borrow().status.clone()
    }

    fn begin(&mut self, verb: JsSessionVerb) -> Result<u64, JsLinkError> {
        let mut fake = self.0.borrow_mut();
        if let Some(reason) = &fake.refuse {
            return Err(JsLinkError(reason.clone()));
        }
        fake.began.push(verb);
        fake.next += 1;
        Ok(fake.next)
    }

    fn poll(&mut self) -> Vec<JsSessionEvent> {
        std::mem::take(&mut self.0.borrow_mut().inbox)
    }
}

impl FakeSession {
    fn finish(&self, ticket: u64, outcome: Result<serde_json::Value, (&str, &str)>) {
        self.0.borrow_mut().inbox.push(JsSessionEvent::Finished {
            ticket,
            outcome: outcome.map_err(|(code, message)| JsOperationError {
                code: code.to_owned(),
                message: message.to_owned(),
            }),
        });
    }

    fn signal(&self, name: &str, object: u64, fields: serde_json::Value) {
        let serde_json::Value::Object(fields) = fields else {
            panic!("a signal's fields are an object");
        };
        self.0.borrow_mut().inbox.push(JsSessionEvent::Signal {
            name: name.to_owned(),
            object,
            fields,
        });
    }
}

const PRELUDE: &str =
    "import { ProcessListener } from \"gnoblin\";\nconst g = globalThis as any;\ng.g = g;\ng.log = [] as string[];\n";

/// `Rig::eval` runs plain JavaScript; the cases are written in TypeScript like a user's config.
trait TsEval {
    fn ts(&mut self, source: &str) -> String;
}

impl TsEval for Rig {
    fn ts(&mut self, source: &str) -> String {
        let stripped = strip_types(Path::new("case.ts"), source).expect("the case is valid TypeScript");
        self.shell
            .eval(&stripped.code)
            .expect("the case runs")
            .unwrap_or_default()
    }
}

/// A shell over a fake compositor with two windows, `init_body` as its entry, and a fake session connection the
/// entry already has: a config that calls the session while it loads sees it.
fn rig(init_body: &str) -> (Rig, FakeSession) {
    let comp = FakeCompositor::new();
    comp.add_window(3, "one", "foot");
    comp.add_window(5, "two", "org.gnome.Nautilus");
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("init.ts"), format!("{PRELUDE}{init_body}\n")).expect("init.ts is written");
    let mut shell = JsShell::new(Box::new(comp.clone()), JsShellConfig::new(&root)).expect("the shell boots");
    let session = FakeSession::default();
    shell.set_session_link(Box::new(session.clone()));
    let load_diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    assert!(load_diagnostics.is_empty(), "{load_diagnostics:?}");
    let mut rig = Rig {
        shell,
        comp,
        load_diagnostics,
        _dir: dir,
    };
    rig.settle();
    (rig, session)
}

/// What a throw of `expression` says: `Name: message`.
fn thrown(rig: &mut Rig, expression: &str) -> String {
    rig.ts(&format!(
        "(() => {{ try {{ {expression}; return 'did not throw'; }} catch (e) {{ return e.name + ': ' + e.message; }} }})()"
    ))
}

/// Runs frames until `check` holds, or fails after the patience: a state that never came, not a slow turn. Returns
/// the diagnostics of the frames it ran.
#[track_caller]
fn until(rig: &mut Rig, what: &str, mut check: impl FnMut(&mut Rig) -> bool) -> Vec<String> {
    let deadline = Instant::now() + PATIENCE;
    let mut diagnostics = Vec::new();
    loop {
        diagnostics.extend(rig.frame());
        if check(rig) {
            return diagnostics;
        }
        assert!(Instant::now() < deadline, "never reached: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---- gnoblin.session ----------------------------------------------------------------------------------------------

#[test]
fn session_status_omits_the_lock_state_until_the_session_publishes_one() {
    // Lua: `session_status_omits_the_lock_state_until_the_session_publishes_one`.
    let (mut rig, _session) = rig("export {};");
    assert_eq!(
        rig.ts("JSON.stringify(gnoblin.session.status())"),
        r#"{"state":"running","lockAvailable":false,"revision":0}"#
    );
}

#[test]
fn session_status_reports_the_published_lock_state_and_revision() {
    // Lua: `session_status_reports_the_published_lock_state_and_revision`; the status is read on each call.
    let (mut rig, session) = rig("export {};");
    session.0.borrow_mut().status = JsSessionStatus {
        lock_available: true,
        revision: 7,
        lock_state: Some("locked".to_owned()),
    };
    assert_eq!(
        rig.ts("JSON.stringify(gnoblin.session.status())"),
        r#"{"state":"running","lockAvailable":true,"lockState":"locked","revision":7}"#
    );
    session.0.borrow_mut().status.lock_available = false;
    assert_eq!(
        rig.ts("'lockState' in gnoblin.session.status()"),
        "false",
        "unavailable does not mean unlocked: the field is absent"
    );
}

#[test]
fn a_shell_with_no_session_connection_reports_unavailable_and_refuses_every_verb() {
    let mut rig = rig_setup(PRELUDE, "export {};", |_| {});
    assert_eq!(
        rig.ts("JSON.stringify(gnoblin.session.status())"),
        r#"{"state":"unavailable","lockAvailable":false,"revision":0}"#
    );
    for verb in ["logout", "poweroff", "reboot", "suspend", "hibernate", "lock"] {
        assert_eq!(
            rig.ts(&format!(
                "(() => {{ const op = gnoblin.session.{verb}(); return [op.status, op.error.code, op.method].join('|'); }})()"
            )),
            format!("failed|unavailable|session.{verb}")
        );
    }
}

#[test]
fn each_verb_calls_the_session_and_its_operation_settles_with_the_compositors_answer() {
    // Lua: `an_operation_completes_its_callback_once`, over the five session methods.
    let (mut rig, session) = rig("g.ops = {
            logout: gnoblin.session.logout(),
            poweroff: gnoblin.session.poweroff(),
            reboot: gnoblin.session.reboot(),
            suspend: gnoblin.session.suspend(),
            hibernate: gnoblin.session.hibernate(),
            lock: gnoblin.session.lock(),
        };
        for (const [name, op] of Object.entries(g.ops) as [string, any][]) {
            op.onComplete((value: unknown, error: unknown) => g.log.push(`${name}:${JSON.stringify(value)}:${error}`));
        }");
    assert_eq!(
        session.0.borrow().began,
        [
            JsSessionVerb::Logout,
            JsSessionVerb::Poweroff,
            JsSessionVerb::Reboot,
            JsSessionVerb::Suspend,
            JsSessionVerb::Hibernate,
            JsSessionVerb::Lock
        ]
    );
    assert_eq!(
        rig.ts("g.ops.logout.status"),
        "pending",
        "nothing settles before the compositor answers"
    );
    for ticket in 1..=5 {
        session.finish(ticket, Ok(serde_json::json!({ "accepted": true })));
    }
    session.finish(6, Ok(serde_json::json!({ "dispatched": true })));
    rig.settle();
    assert_eq!(
        rig.ts("g.log.join('\\n')"),
        [
            r#"logout:{"accepted":true}:undefined"#,
            r#"poweroff:{"accepted":true}:undefined"#,
            r#"reboot:{"accepted":true}:undefined"#,
            r#"suspend:{"accepted":true}:undefined"#,
            r#"hibernate:{"accepted":true}:undefined"#,
            r#"lock:{"dispatched":true}:undefined"#,
        ]
        .join("\n")
    );
    rig.settle();
    assert_eq!(rig.ts("g.log.length"), "6", "each callback runs once");
}

#[test]
fn a_refused_verb_fails_the_operation_with_the_typed_error_and_the_message() {
    // Lua docs `session-runtime.md`: `denied`, `unsupported`, `busy`, `unavailable`, `timed_out`, `internal`.
    let (mut rig, session) = rig(
        "g.ops = [gnoblin.session.suspend(), gnoblin.session.hibernate(), gnoblin.session.reboot()];
         g.ops.forEach((op: any) => op.onComplete((_v: unknown, e: any) => g.log.push(`${e.code}: ${e.message}`)));",
    );
    session.finish(
        1,
        Err((
            "denied",
            "logind could not suspend: org.freedesktop.DBus.Error.AccessDenied: no",
        )),
    );
    session.finish(2, Err(("unsupported", "this machine cannot hibernate")));
    session.finish(3, Err(("timed_out", "logind did not answer in 30 s")));
    rig.settle();
    assert_eq!(
        rig.ts("g.log.join('|')"),
        "denied: logind could not suspend: org.freedesktop.DBus.Error.AccessDenied: no|\
         unsupported: this machine cannot hibernate|timed_out: logind did not answer in 30 s"
    );
    assert_eq!(
        rig.ts("[g.ops[0].status, g.ops[0].value, g.ops[0].error.code].join(',')"),
        "failed,,denied",
        "a failure has an error and no value"
    );
}

#[test]
fn a_link_that_refuses_to_begin_fails_the_operation_unavailable_at_once() {
    let (mut rig, session) = rig("export {};");
    session.0.borrow_mut().refuse = Some("the session connection is gone".to_owned());
    assert_eq!(
        rig.ts("(() => { const op = gnoblin.session.logout(); return op.status + ':' + op.error.code + ':' + op.error.message; })()"),
        "failed:unavailable:the session connection is gone"
    );
}

#[test]
fn a_lost_connection_fails_every_call_still_waiting() {
    let (mut rig, session) = rig("g.a = gnoblin.session.poweroff(); g.b = gnoblin.session.suspend();
         g.a.onComplete((_v: unknown, e: any) => g.log.push(e.code));");
    session
        .0
        .borrow_mut()
        .inbox
        .push(JsSessionEvent::Lost("the compositor closed the connection".to_owned()));
    rig.settle();
    assert_eq!(
        rig.ts("[g.a.error.code, g.b.error.code, g.log.join()].join('|')"),
        "unavailable|unavailable|unavailable"
    );
}

#[test]
fn reload_config_begins_the_reload_verb_and_is_refused_while_the_configuration_loads() {
    // Lua: `a_script_cannot_ask_for_a_reload_while_the_configuration_loads`, and the two reload answers.
    let (mut rig, session) = rig("export {};");
    // The entry has run and the shell's first frame is over: a reload may be asked for now.
    rig.ts("g.ok = gnoblin.runtime.reloadConfig();
         g.ok.onComplete((v: any) => g.log.push('rev ' + v.settingsRevision + ' ' + v.notApplied.join()));");
    assert_eq!(session.0.borrow().began, [JsSessionVerb::ReloadConfig]);
    rig.ts("g.bad = gnoblin.runtime.reloadConfig();
         g.bad.onComplete((_v: unknown, e: any) => g.log.push(e.code + ' ' + e.message));");
    session.finish(
        1,
        Ok(serde_json::json!({"ok": true, "action": "config reload", "settingsRevision": 4, "warnings": [], "notApplied": ["a: restart required"]})),
    );
    session.finish(2, Err(("config_rejected", "init.ts:3:1: unexpected token")));
    rig.settle();
    assert_eq!(
        rig.ts("g.log.join('|')"),
        "rev 4 a: restart required|config_rejected init.ts:3:1: unexpected token"
    );

    let mut loading = rig_setup(
        PRELUDE,
        "try { gnoblin.runtime.reloadConfig(); g.log.push('no'); } catch (e) { g.log.push(e.name + ': ' + e.message); }",
        |_| {},
    );
    assert_eq!(
        loading.ts("g.log.join()"),
        "RangeError: gnoblin.runtime.reloadConfig: the configuration is loading, and each reload would ask for another"
    );
}

// ---- operations ---------------------------------------------------------------------------------------------------

/// A pending operation a script can settle: the session's `lock`, settled by ticket 1 of the fake.
const PENDING: &str = "g.op = gnoblin.session.lock();\n";

#[test]
fn an_operation_calls_each_callback_once_and_a_late_callback_runs_after_the_current_call() {
    // Lua: `an_operation_completes_its_callback_once`.
    let (mut rig, session) = rig(PENDING);
    rig.ts("g.op.onComplete((v: any) => g.log.push('first ' + v.dispatched)); g.op.onComplete(() => { throw new Error('boom'); }); g.op.onComplete(() => g.log.push('third'))");
    session.finish(1, Ok(serde_json::json!({ "dispatched": true })));
    let diagnostics = rig.settle();
    assert_eq!(
        rig.ts("g.log.join('|')"),
        "first true|third",
        "a throwing callback does not stop the others"
    );
    assert!(
        diagnostics.iter().any(|line| line.contains("boom")),
        "the throw is reported: {diagnostics:?}"
    );
    assert_eq!(
        rig.ts("(() => { g.op.onComplete(() => g.log.push('late')); return g.log.length; })()"),
        "2",
        "a callback added after the operation settled does not run inside the adding call"
    );
    rig.settle();
    assert_eq!(rig.ts("g.log.join('|')"), "first true|third|late");
}

#[test]
fn a_callback_that_was_unsubscribed_never_runs() {
    let (mut rig, session) = rig(PENDING);
    rig.ts("g.op.onComplete(() => g.log.push('kept')); g.op.onComplete(() => g.log.push('dropped')).unsubscribe();");
    session.finish(1, Ok(serde_json::json!({ "dispatched": true })));
    rig.settle();
    assert_eq!(rig.ts("g.log.join('|')"), "kept");
}

#[test]
fn map_transforms_the_value_and_a_throw_in_it_fails_the_new_operation_internal() {
    // Lua: `an_operation_map_transforms_its_completion_value`, `an_operation_map_error_fails_the_derived_operation`,
    // `an_operation_map_propagates_a_source_failure_without_calling_the_mapper`.
    let (mut rig, session) = rig("const first = gnoblin.session.lock();
         const second = gnoblin.session.lock();
         const third = gnoblin.session.lock();
         g.mapped = first.map((v: any) => v.dispatched ? 'yes' : 'no');
         g.thrower = second.map(() => { throw new Error('mapper broke'); });
         g.untouched = third.map(() => { g.log.push('mapper ran'); return 1; });");
    session.finish(1, Ok(serde_json::json!({ "dispatched": true })));
    session.finish(2, Ok(serde_json::json!({ "dispatched": true })));
    session.finish(3, Err(("busy", "another action runs")));
    rig.settle();
    assert_eq!(rig.ts("[g.mapped.status, g.mapped.value].join()"), "succeeded,yes");
    assert_eq!(
        rig.ts("[g.thrower.status, g.thrower.error.code, g.thrower.error.message].join()"),
        "failed,internal,mapper broke"
    );
    assert_eq!(
        rig.ts("[g.untouched.status, g.untouched.error.code].join()"),
        "failed,busy"
    );
    assert_eq!(
        rig.ts("g.log.length"),
        "0",
        "the mapper does not run for a failed source"
    );
}

#[test]
fn timeout_fails_timed_out_after_its_deadline_and_is_cleared_when_the_source_finishes() {
    // Lua: `an_operation_timeout_fails_after_its_deadline`, `an_operation_timeout_is_cleared_when_its_source_finishes`.
    let (mut rig, session) = rig("g.slow = gnoblin.session.lock().timeout(0.05);
         g.quick = gnoblin.session.lock();
         g.limited = g.quick.timeout(0.05);");
    session.finish(2, Ok(serde_json::json!({ "dispatched": true })));
    until(&mut rig, "the slow operation times out", |rig| {
        rig.ts("g.slow.status") == "failed"
    });
    assert_eq!(rig.ts("g.slow.error.code"), "timed_out");
    assert_eq!(rig.ts("g.limited.status"), "succeeded", "the source finished first");
    assert_eq!(
        rig.ts("gnoblin.timers.size"),
        "0",
        "a settled source clears its deadline's timer"
    );
    assert_eq!(
        rig.ts("g.quick.status"),
        "succeeded",
        "timeout does not cancel the operation it waits on"
    );
}

#[test]
fn cancel_fails_cancelled_and_ignores_the_late_completion_and_a_settled_operation_cannot_be_cancelled() {
    // Lua: `cancelling_an_operation_ignores_its_late_completion`, `cancelling_a_completed_operation_is_a_noop`.
    let (mut rig, session) = rig("g.cancelled = gnoblin.session.lock();
         g.cancelled.onComplete((_v: unknown, e: any) => g.log.push(e.code));
         g.done = gnoblin.session.lock();");
    assert_eq!(rig.ts("g.cancelled.cancel()"), "true");
    session.finish(1, Ok(serde_json::json!({ "dispatched": true })));
    session.finish(2, Ok(serde_json::json!({ "dispatched": true })));
    rig.settle();
    assert_eq!(rig.ts("g.log.join()"), "cancelled");
    assert_eq!(rig.ts("g.cancelled.status"), "failed", "the late completion is ignored");
    assert_eq!(rig.ts("g.done.cancel()"), "false");
    assert_eq!(rig.ts("g.done.status"), "succeeded");
}

// ---- gnoblin.timers -----------------------------------------------------------------------------------------------

#[test]
fn a_timer_runs_once_at_the_first_frame_past_its_time_and_never_after_cancel() {
    // Lua: `a_timer_fires_on_its_tick_and_never_after_cancel`; the JavaScript timer counts milliseconds.
    let (mut rig, _session) = rig("g.fired = gnoblin.timers.after(30, () => g.log.push('fired'));
         g.cancelled = gnoblin.timers.after(30, () => g.log.push('cancelled'));
         g.cancelled.cancel();
         g.late = gnoblin.timers.after(10_000, () => g.log.push('late'));");
    assert_eq!(
        rig.ts("[g.fired.pending, g.cancelled.pending, g.cancelled.cancel()].join()"),
        "true,false,false"
    );
    until(&mut rig, "the timer ran", |rig| rig.ts("g.log.length") != "0");
    rig.settle();
    assert_eq!(
        rig.ts("g.log.join()"),
        "fired",
        "once, and never the cancelled one or the far one"
    );
    assert_eq!(
        rig.ts("[g.fired.pending, g.fired.cancel(), gnoblin.timers.size].join()"),
        "false,false,1"
    );
}

#[test]
fn timers_now_is_monotonic_milliseconds_and_gnoblin_after_is_the_seconds_spelling() {
    let (mut rig, _session) =
        rig("g.t0 = gnoblin.timers.now(); g.alias = gnoblin.after(0.02, () => g.log.push('after'));");
    assert_eq!(rig.ts("gnoblin.timers.now() >= g.t0"), "true");
    assert_eq!(
        rig.ts("g.alias.constructor.name"),
        "Timer",
        "the alias returns the timer"
    );
    until(&mut rig, "gnoblin.after ran", |rig| rig.ts("g.log.join()") == "after");
}

#[test]
fn a_timer_delay_outside_one_day_or_not_a_number_is_refused_and_the_pending_count_is_capped() {
    // Lua: `a_timer_delay_outside_one_day_or_not_a_number_is_refused`, `the_pending_timer_count_is_capped_at_1024`.
    let (mut rig, _session) = rig("export {};");
    for bad in ["-1", "86_400_001", "NaN", "Infinity", "'5'"] {
        assert!(
            thrown(&mut rig, &format!("gnoblin.timers.after({bad}, () => {{}})"))
                .starts_with("TypeError: gnoblin.timers.after: ms must be"),
            "{bad}"
        );
    }
    assert!(
        thrown(&mut rig, "gnoblin.timers.after(1, 5 as any)")
            .starts_with("TypeError: gnoblin.timers.after: needs a function")
    );
    assert_eq!(
        rig.ts("(() => { gnoblin.timers.after(86_400_000, () => {}); return 'a day is allowed'; })()"),
        "a day is allowed"
    );
    rig.ts("for (let i = 0; i < 1023; i++) gnoblin.timers.after(60_000, () => {});");
    assert_eq!(rig.ts("gnoblin.timers.size"), "1024");
    assert_eq!(
        thrown(&mut rig, "gnoblin.timers.after(1, () => {})"),
        "RangeError: gnoblin.timers.after: 1024 timers are already pending"
    );
}

#[test]
fn a_timer_that_throws_is_reported_and_the_next_timer_still_runs() {
    let (mut rig, _session) = rig("gnoblin.timers.after(30, () => { throw new Error('timer broke'); });
         gnoblin.timers.after(30, () => g.log.push('next'));");
    let diagnostics = until(&mut rig, "both timers ran", |rig| rig.ts("g.log.length") == "1");
    assert!(
        diagnostics.iter().any(|line| line.contains("timer broke")),
        "{diagnostics:?}"
    );
}

#[test]
fn retiring_the_generation_cancels_its_timers_pending_operations_and_handlers() {
    // Lua: `a_reload_cancels_the_timers_of_the_previous_run`, `reloading_clears_global_signal_handlers`.
    let (mut rig, session) = rig("g.timer = gnoblin.timers.after(50, () => g.log.push('timer'));
         gnoblin.on('gnoblin.session.bell', () => g.log.push('bell'));
         g.op = gnoblin.session.lock();");
    rig.shell.retire().expect("the generation retires");
    session.signal("gnoblin.session.bell", 1, serde_json::json!({ "count": 1 }));
    session.finish(1, Ok(serde_json::json!({ "dispatched": true })));
    rig.settle();
    std::thread::sleep(Duration::from_millis(80));
    rig.settle();
    assert_eq!(
        rig.ts("[g.timer.pending, g.log.length, g.op.status].join()"),
        "false,0,pending"
    );
}

// ---- gnoblin.on ---------------------------------------------------------------------------------------------------

#[test]
fn handlers_run_in_order_one_that_throws_does_not_stop_the_rest_and_once_runs_once() {
    // Lua: `global_signal_handlers_are_ordered_isolated_and_one_shot`; the event's fields are camelCase.
    let (mut rig, session) = rig("gnoblin.on('gnoblin.session.bell', (e) => g.log.push('a:' + e.count));
         gnoblin.on('gnoblin.session.bell', () => { throw new Error('handler broke'); });
         gnoblin.on('gnoblin.session.bell', (e) => g.log.push('c:' + e.name + ':' + e.object));
         gnoblin.once('gnoblin.session.bell', (e) => g.log.push('once:' + e.count));");
    session.signal("gnoblin.session.bell", 9, serde_json::json!({ "count": 1 }));
    session.signal("gnoblin.session.bell", 9, serde_json::json!({ "count": 2 }));
    let diagnostics = rig.settle();
    assert_eq!(
        rig.ts("g.log.join('|')"),
        "a:1|c:gnoblin.session.bell:9|once:1|a:2|c:gnoblin.session.bell:9"
    );
    assert!(
        diagnostics.iter().any(|line| line.contains("handler broke")),
        "{diagnostics:?}"
    );
}

#[test]
fn off_detaches_by_subscription_and_by_name_and_handler() {
    let (mut rig, session) = rig("const keep = () => g.log.push('keep');
         const drop = () => g.log.push('drop');
         const sub = gnoblin.on('gnoblin.session.bell', drop);
         gnoblin.on('gnoblin.session.bell', keep);
         gnoblin.on('gnoblin.session.bell', drop);
         gnoblin.off(sub);
         gnoblin.off('gnoblin.session.bell', drop);");
    session.signal("gnoblin.session.bell", 1, serde_json::json!({ "count": 1 }));
    rig.settle();
    assert_eq!(rig.ts("g.log.join()"), "keep");
    assert!(thrown(&mut rig, "gnoblin.off(5 as any)").starts_with("TypeError: gnoblin.off: expects a subscription"));
}

#[test]
fn hyphen_spellings_name_the_underscored_signal() {
    // Lua: `hyphenated_event_spellings_subscribe_to_the_underscored_signal`.
    let (mut rig, session) =
        rig("gnoblin.on('gnoblin.session.reload-requested', (e) => g.log.push(e.name + ':' + e.generation));");
    session.signal(
        "gnoblin.session.reload_requested",
        1,
        serde_json::json!({ "generation": 3 }),
    );
    rig.settle();
    assert_eq!(rig.ts("g.log.join()"), "gnoblin.session.reload_requested:3");
}

#[test]
fn an_unknown_event_name_is_an_error_that_lists_the_near_matches() {
    // Lua: `global_event_registration_rejects_unknown_names_and_excess_handlers`,
    // `an_unknown_hyphenated_event_is_refused_with_the_underscored_name`.
    let (mut rig, _session) = rig("export {};");
    assert_eq!(
        thrown(&mut rig, "gnoblin.on('gnoblin.session.bel', () => {})"),
        "TypeError: gnoblin.on: no schema signal named `gnoblin.session.bel`; expected `namespace.object.signal`; \
         near matches: gnoblin.session.bell"
    );
    let hyphenated = thrown(&mut rig, "gnoblin.once('gnoblin.session.lock-reqested', () => {})");
    assert!(
        hyphenated.contains("`gnoblin.session.lock_reqested`"),
        "the message names the underscored spelling: {hyphenated}"
    );
    assert!(
        thrown(&mut rig, "gnoblin.on('nonsense', () => {})")
            .starts_with("TypeError: gnoblin.on: no schema signal named `nonsense`")
    );
    assert!(
        thrown(&mut rig, "gnoblin.on(5 as any, () => {})")
            .starts_with("TypeError: gnoblin.on: the event name must be a string")
    );
    assert!(
        thrown(&mut rig, "gnoblin.on('gnoblin.session.bell', 5 as any)")
            .starts_with("TypeError: gnoblin.on: needs a function")
    );
}

#[test]
fn the_handler_count_is_capped_at_1024() {
    let (mut rig, _session) = rig("for (let i = 0; i < 1024; i++) gnoblin.on('gnoblin.session.bell', () => {});");
    assert_eq!(
        thrown(&mut rig, "gnoblin.on('gnoblin.session.bell', () => {})"),
        "RangeError: gnoblin.on: 1024 event handlers are already attached"
    );
}

// ---- gnoblin.find and gnoblin.focus -------------------------------------------------------------------------------

#[test]
fn find_filters_windows_by_fields_and_by_predicate_in_list_order() {
    // Lua: `find_returns_only_live_windows_and_alive_turns_false_on_close`; `app_id` and `appId` are one field.
    let (mut rig, _session) = rig("export {};");
    assert_eq!(rig.ts("gnoblin.find('Window').map((w: any) => w.id).join()"), "3,5");
    assert_eq!(
        rig.ts("gnoblin.find('Window', { appId: 'foot' }).map((w: any) => w.title).join()"),
        "one"
    );
    assert_eq!(
        rig.ts("gnoblin.find('Window', { app_id: 'org.gnome.Nautilus', title: 'two' }).length"),
        "1"
    );
    assert_eq!(
        rig.ts("gnoblin.find('Window', { appId: 'foot', title: 'two' }).length"),
        "0",
        "every field must match"
    );
    assert_eq!(
        rig.ts("gnoblin.find('Window', (w: any) => w.title.includes('w')).map((w: any) => w.id).join()"),
        "5"
    );
    assert_eq!(rig.ts("gnoblin.find('Workspace').length > 0"), "true");
    rig.comp.destroy(3);
    rig.settle();
    assert_eq!(
        rig.ts("gnoblin.find('Window').map((w: any) => w.id).join()"),
        "5",
        "a closed window leaves the answer"
    );
}

#[test]
fn find_refuses_an_unknown_field_kind_or_value_and_names_what_works() {
    // Lua: `find_refuses_a_table_field_the_kind_does_not_declare_by_name`,
    // `find_refuses_an_unknown_kind_and_names_the_kinds_that_work`, `a_predicate_error_is_an_error_of_the_find_call`.
    let (mut rig, _session) = rig("export {};");
    let field = thrown(&mut rig, "gnoblin.find('Window', { colour: 'red' })");
    assert!(
        field.starts_with("TypeError: gnoblin.find: Window has no scalar field \"colour\"; the fields are id, appId"),
        "{field}"
    );
    assert_eq!(
        thrown(&mut rig, "gnoblin.find('Widget' as any)"),
        "TypeError: gnoblin.find: no kind named \"Widget\"; the kinds are Window, Workspace, Output"
    );
    assert!(
        thrown(&mut rig, "gnoblin.find('Window', { title: {} as any })")
            .contains("must be a string, a number or a boolean")
    );
    assert!(thrown(&mut rig, "gnoblin.find('Window', 5 as any)").contains("a table of field values or a predicate"));
    assert_eq!(
        thrown(
            &mut rig,
            "gnoblin.find('Window', () => { throw new Error('predicate broke'); })"
        ),
        "Error: predicate broke",
        "an error inside the predicate is an error of the find call"
    );
}

#[test]
fn focus_history_lists_windows_by_recent_focus_and_filters_by_workspace_and_limit() {
    let (mut rig, _session) = rig("export {};");
    assert_eq!(
        rig.ts("gnoblin.focus.history().map((w: any) => w.id).join()"),
        "3,5",
        "never focused: list order"
    );
    rig.comp.external_set(5, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(
        rig.ts("gnoblin.focus.history().map((w: any) => w.id).join()"),
        "5,3",
        "the focused window is first"
    );
    rig.comp.external_set(5, "focused", MirrorValue::Bool(false));
    rig.comp.external_set(3, "focused", MirrorValue::Bool(true));
    rig.settle();
    assert_eq!(rig.ts("gnoblin.focus.history().map((w: any) => w.id).join()"), "3,5");
    rig.comp.external_set(3, "focused", MirrorValue::Bool(false));
    rig.settle();
    assert_eq!(
        rig.ts("gnoblin.focus.history().map((w: any) => w.id).join()"),
        "3,5",
        "nothing focused: the order seen so far stays"
    );
    assert_eq!(
        rig.ts("gnoblin.focus.history({ limit: 1 }).map((w: any) => w.id).join()"),
        "3"
    );
    assert_eq!(
        rig.ts("gnoblin.focus.history({ workspaceId: gnoblin.workspaces.at(0)!.id }).length"),
        "2",
        "both windows are on the first workspace"
    );
    assert_eq!(rig.ts("gnoblin.focus.history({ workspace_id: 'nowhere' }).length"), "0");
    for bad in ["{ limit: 0 }", "{ limit: 257 }", "{ limit: 1.5 }", "{ colour: 1 }"] {
        assert!(
            thrown(&mut rig, &format!("gnoblin.focus.history({bad} as any)"))
                .starts_with("TypeError: gnoblin.focus.history"),
            "{bad}"
        );
    }
}

// ---- gnoblin.process ----------------------------------------------------------------------------------------------

#[test]
fn a_spawned_process_has_a_handle_and_its_end_reaches_wait_and_the_listeners() {
    // Lua: `a_process_reports_its_fields_and_how_it_ended`; `exit_code` is `exitCode`, `closed` is `ProcessListener.onExit`.
    let (mut rig, _session) = rig("class Watch extends ProcessListener {
             onExit(process: any, code: number | null) { g.log.push(`exit ${process.id} ${code} ${process.alive}`); }
         }
         gnoblin.process.addListener(new Watch());
         g.job = gnoblin.process.spawn(['sh', '-c', 'printf out; exit 3'], { stdout: 'capture' });
         g.job.wait().onComplete((exit: any) => g.log.push(`wait ${exit.exitCode} ${exit.stdout} ${exit.timedOut}`));");
    assert_eq!(
        rig.ts("[g.job.id, g.job.pid > 0, g.job.alive, g.job.status, g.job.exitCode, g.job.argv.join(' ')].join('|')"),
        "1|true|true|running||sh -c printf out; exit 3"
    );
    until(&mut rig, "the process ended", |rig| rig.ts("g.job.alive") == "false");
    rig.settle();
    assert_eq!(
        rig.ts("[g.job.status, g.job.exitCode, g.job.signal, g.job.timedOut].join('|')"),
        "exited|3||false"
    );
    let mut lines: Vec<String> = rig.ts("g.log.join('\\n')").lines().map(str::to_owned).collect();
    lines.sort();
    assert_eq!(
        lines,
        ["exit 1 3 false", "wait 3 out false"],
        "both the listener and the wait operation hear the end"
    );
    assert_eq!(
        rig.ts("g.job.wait() === g.job.wait()"),
        "true",
        "every call returns the same operation"
    );
    assert_eq!(rig.ts("gnoblin.process.list().length"), "0");
}

#[test]
fn kill_ends_the_group_names_the_signal_and_fails_not_found_once_it_has_ended() {
    // Lua: `changed_names_the_fields_that_moved_and_a_killed_process_names_its_signal`,
    // `killing_an_ended_process_fails_with_not_found`.
    let (mut rig, _session) = rig(
        "class Watch extends ProcessListener { onExit(process: any, code: number | null) { g.log.push(`${code}:${process.signal}`); } }
         gnoblin.process.addListener(new Watch());
         g.job = gnoblin.process.spawn(['sh', '-c', 'sleep 60 & wait']);
         g.kill = g.job.kill('kill');",
    );
    assert_eq!(rig.ts("g.kill.status"), "succeeded");
    until(&mut rig, "the killed process ended", |rig| {
        rig.ts("g.job.alive") == "false"
    });
    rig.settle();
    assert_eq!(rig.ts("g.log.join()"), "null:kill", "a signal's end has no exit code");
    assert_eq!(rig.ts("g.job.kill().error.code"), "not_found");
    assert!(
        thrown(&mut rig, "g.job.kill('nope')").starts_with("TypeError: process.kill: the signal is one of term, kill")
    );
}

#[test]
fn list_holds_the_running_processes_oldest_first_and_by_id_finds_them_again() {
    // Lua: `the_list_holds_running_processes_and_by_id_finds_them_again`.
    let (mut rig, _session) = rig("g.a = gnoblin.process.spawn(['sleep', '60']);
         g.b = gnoblin.spawn('sleep 60');");
    assert_eq!(rig.ts("gnoblin.process.list().map((p: any) => p.id).join()"), "1,2");
    assert_eq!(
        rig.ts("gnoblin.process.byId(2) === g.b"),
        "true",
        "one handle for one id"
    );
    assert_eq!(rig.ts("gnoblin.process.byId(99)"), "null");
    rig.ts("g.a.kill('kill'); g.b.kill('kill');");
    until(&mut rig, "both ended", |rig| {
        rig.ts("gnoblin.process.list().length") == "0"
    });
    assert_eq!(
        rig.ts("gnoblin.process.byId(1)"),
        "null",
        "an ended process is known only to whoever holds it"
    );
}

#[test]
fn a_timeout_ends_the_process_and_reports_it() {
    // Lua: `a_timeout_ends_the_process_and_reports_it_as_timed_out`.
    let (mut rig, _session) = rig("g.job = gnoblin.process.spawn(['sleep', '60'], { timeout: 0.05 });");
    until(&mut rig, "the timeout ended it", |rig| rig.ts("g.job.alive") == "false");
    assert_eq!(rig.ts("[g.job.timedOut, g.job.signal].join()"), "true,term");
}

#[test]
fn spawn_options_reach_the_program_and_a_bad_option_is_a_type_error_naming_it() {
    let (mut rig, _session) = rig(
        "g.job = gnoblin.process.spawn(['sh', '-c', 'printf \"%s|%s|\" \"$GNOBLIN_X\" \"$(pwd)\"; cat'],
             { env: { GNOBLIN_X: 'set' }, cwd: '/', stdin: 'fed', stdout: 'capture' });",
    );
    until(&mut rig, "the program ended", |rig| rig.ts("g.job.alive") == "false");
    assert_eq!(rig.ts("g.job.exit.stdout"), "set|/|fed");
    for (bad, needle) in [
        ("{ cwd: 'relative' }", "gnoblin.spawn: cwd must be an absolute path"),
        ("{ timeout: 0 }", "gnoblin.spawn: timeout is more than 0"),
        ("{ outputLimit: 99999999 }", "gnoblin.spawn: outputLimit is at most"),
        (
            "{ stdout: 'tty' }",
            "gnoblin.process.spawn: stdout must be inherit, discard, capture",
        ),
        ("{ nonsense: 1 }", "gnoblin.process.spawn: no option named nonsense"),
        (
            "{ env: { A: 1 } }",
            "gnoblin.process.spawn: env must be an object of strings",
        ),
    ] {
        let message = thrown(&mut rig, &format!("gnoblin.process.spawn(['true'], {bad} as any)"));
        assert!(
            message.starts_with("TypeError: ") && message.contains(needle),
            "{bad}: {message}"
        );
    }
}

#[test]
fn spawn_throws_the_spawn_failed_error_for_a_program_that_cannot_start_and_a_type_error_for_a_bad_command() {
    // The behaviour of the old `gnoblin.spawn`, kept: a refusal is a `GnoblinError` with the code `spawn-failed`.
    let (mut rig, _session) = rig("export {};");
    assert_eq!(
        rig.ts("(() => { try { gnoblin.spawn(['/nonexistent/gnoblin-test']); } catch (e) { return [e.name, e.code, e.message].join('|'); } })()")
            .split_once("cannot start")
            .map(|(head, _)| head.to_owned()),
        Some("GnoblinError|spawn-failed|gnoblin.spawn: ".to_owned())
    );
    for bad in ["''", "[]", "42", "['a', 1]", "undefined"] {
        let outcome = thrown(&mut rig, &format!("gnoblin.spawn({bad} as any)"));
        assert!(
            outcome.starts_with("TypeError: gnoblin.spawn: expects a command string"),
            "{bad}: {outcome}"
        );
    }
    assert_eq!(
        rig.ts("gnoblin.process.list().length"),
        "0",
        "a bad argument starts nothing"
    );
}

// ---- gnoblin.version ----------------------------------------------------------------------------------------------

#[test]
fn version_is_a_frozen_record_of_the_build_identity_with_unknown_for_what_the_build_lacks() {
    // Lua: `version_is_an_immutable_record_of_the_build_identity`.
    let (mut rig, _session) = rig("export {};");
    assert_eq!(
        rig.ts("Object.keys(gnoblin.version()).sort().join()"),
        "api,buildId,gitRemote,gitSha,gnoblin,gnome,js,mutter"
    );
    assert_eq!(rig.ts("gnoblin.version().js"), "quickjs-ng");
    assert_eq!(rig.ts("/^\\d+\\.\\d+\\.\\d+$/.test(gnoblin.version().api)"), "true");
    assert_eq!(rig.ts("Object.isFrozen(gnoblin.version())"), "true");
    assert_eq!(
        rig.ts("(() => { 'use strict'; try { (gnoblin.version() as any).api = 'x'; return 'wrote'; } catch (e) { return e.name; } })()"),
        "TypeError",
        "a write to the immutable record is refused"
    );
}
