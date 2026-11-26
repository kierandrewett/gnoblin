//! What a call the core makes into the script costs the generation, and who stages the generation that replaces it.
//!
//! On the native library a listener is not called by the engine's frame: the core's dispatch calls it, through the runtime's entry into
//! the running generation. A call that never returns is stopped there by the listener budget, and it has to count as a strike like any
//! other call that ran out its budget, so that the role's supervisor restages the generation, or stops restaging it after three.
//!
//! A role built with `external_reload` does not stage a generation in its own turn: the host does, through the core, so that what the
//! staged generation registers is kept apart from the running one until it takes over. The strike is the role's to count and the host's
//! to act on (`JsRole::take_reload_wanted`, `JsRole::reload_now`).

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_js::{JsRole, JsRoleLinks};
use gnoblin_shell_host::supervisor::Health;

mod common;
use common::{FakeCompositor, RUNAWAY_BUDGET, honest_role_config, native};

/// A listener that never returns for a window of the app `spin`.
const SPIN: &str = r#"import { gnoblin } from "gnoblin-core";
gnoblin.clients.addListener({
    onAdded(client: { appId: string }): void {
        if (client.appId === "spin") {
            while (true) {
                // Only the budget stops it.
            }
        }
    },
});
"#;

/// A role over the shared fake compositor whose `shell.ts` is `SPIN`, with a listener budget a runaway runs out quickly.
fn spinning_role() -> (JsRole, FakeCompositor, (tempfile::TempDir, native::NativeCore)) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root: PathBuf = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(root.join("shell.ts"), SPIN).expect("shell.ts");
    let mut config = honest_role_config(&root);
    config.watch = false;
    config.shell.entry_budget = RUNAWAY_BUDGET;
    let comp = FakeCompositor::new();
    let mut core = native::NativeCore::staged(&comp, Arc::clone(&config.shell.clock));
    let role = core.role(config, JsRoleLinks::default());
    (role, comp, (dir, core))
}

/// Runs the role's frame and the core's half of it until a call has been stopped, or fails. Returns as soon as the strike is counted:
/// the role sees it in its next frame.
#[track_caller]
fn until_a_call_is_stopped(role: &mut JsRole, core: &native::NativeCore) {
    for _ in 0..400 {
        role.tick().expect("a frame");
        core.dispatch(role.shell_mut());
        if role.shell_mut().strikes() > 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no call was stopped: {:?}", role.diagnostics());
}

/// How many stopped calls the role has reported.
fn stopped(role: &JsRole) -> usize {
    role.diagnostics()
        .iter()
        .filter(|text| text.contains("a listener call was stopped"))
        .count()
}

#[test]
fn a_call_of_the_core_that_runs_out_its_budget_is_a_strike_and_the_host_stages_the_replacement() {
    let (mut role, comp, (_dir, mut core)) = spinning_role();
    let booted = role.generation_id();
    comp.create_bare_window(5, "Spin", "spin");
    until_a_call_is_stopped(&mut role, &core);
    assert_eq!(
        role.shell_mut().strikes(),
        1,
        "the call that ran out its budget is a strike"
    );

    // The next frame reports it and the supervisor decides: the replacement is the host's to stage, not the tick's.
    role.tick().expect("a frame");
    assert_eq!(
        stopped(&role),
        1,
        "the stopped call is a diagnostic: {:?}",
        role.diagnostics()
    );
    assert!(
        role.diagnostics()
            .iter()
            .any(|text| text.contains("CPU budget and was interrupted")),
        "it says what ran out: {:?}",
        role.diagnostics()
    );
    assert_eq!(
        role.generation_id(),
        booted,
        "the tick did not evaluate the entry against the running generation: no stage of the core was open"
    );
    assert!(
        role.take_reload_wanted(),
        "the supervisor allowed a restage, which the host is asked for"
    );
    assert_eq!(role.health(), Health::Healthy);

    // The host stages it through the core, and the generation that was suspect is gone.
    let report = core.reload(&mut role);
    assert!(report.ok, "{report:?}");
    assert!(role.generation_id() > booted, "a new generation serves");
    assert!(!role.take_reload_wanted(), "one strike asks for one reload");
}

#[test]
fn the_strikes_of_restaged_generations_add_up_and_the_third_stops_the_restaging() {
    let (mut role, comp, (_dir, mut core)) = spinning_role();
    comp.create_bare_window(5, "Spin", "spin");
    // Each restaged generation is told of the window the first generation hung on, and hangs on it too: strike after strike, with a
    // restage after each of the first two.
    let mut restages = 0;
    for _ in 0..8 {
        until_a_call_is_stopped(&mut role, &core);
        role.tick().expect("a frame");
        if !role.take_reload_wanted() {
            break;
        }
        let report = core.reload(&mut role);
        assert!(report.ok, "{report:?}");
        restages += 1;
    }
    assert_eq!(restages, 2, "two strikes are restaged and the third is not");
    assert!(
        matches!(role.health(), Health::Degraded { .. }),
        "three strikes in 30 s latch the supervisor"
    );
    assert!(
        role.diagnostics()
            .iter()
            .any(|text| text.contains("interrupted too many times") && text.contains("stays as the last good one")),
        "the role says why it stopped: {:?}",
        role.diagnostics()
    );
    assert_eq!(
        stopped(&role),
        3,
        "each stopped call is a diagnostic: {:?}",
        role.diagnostics()
    );

    // Latched, a runaway is still stopped and reported, and nothing asks the host for a generation.
    let last = role.generation_id();
    let before = stopped(&role);
    comp.create_bare_window(6, "Spin", "spin");
    for _ in 0..400 {
        role.tick().expect("a frame");
        core.dispatch(role.shell_mut());
        role.tick().expect("a frame");
        if stopped(&role) > before {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        stopped(&role) > before,
        "the runaway was stopped: {:?}",
        role.diagnostics()
    );
    assert!(!role.take_reload_wanted(), "the latched supervisor asks for nothing");
    assert_eq!(role.generation_id(), last, "the last generation stays");
}

#[test]
fn a_reload_the_host_refused_is_not_taken_for_the_restage_of_the_next_strike() {
    let (mut role, comp, (_dir, mut core)) = spinning_role();
    comp.create_bare_window(5, "Spin", "spin");
    // A first strike, which the supervisor allows a restage of.
    until_a_call_is_stopped(&mut role, &core);
    role.tick().expect("a frame");
    assert!(role.take_reload_wanted());
    // The host refuses it, and the user then reloads on purpose: a deliberate retry, which clears the strike. The two runaways that
    // follow (each generation is told of the window again) are strikes one and two, so nothing latches.
    role.reload_refused();
    let report = core.reload(&mut role);
    assert!(report.ok, "{report:?}");
    for _ in 0..2 {
        until_a_call_is_stopped(&mut role, &core);
        role.tick().expect("a frame");
        assert!(role.take_reload_wanted(), "the supervisor still restages");
        let report = core.reload(&mut role);
        assert!(report.ok, "{report:?}");
    }
    assert_eq!(role.health(), Health::Healthy);
}
