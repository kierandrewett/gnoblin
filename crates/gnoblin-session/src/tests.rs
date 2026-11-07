//! One test per failure mode in the crate documentation.
//!
//! Every test drives the guardian through [`Guardian::apply`] and its queries
//! only: no private field, no helper and no clock. A test here fails when the
//! guardian could start a second instance of a child, restart the compositor,
//! leave a dependant running after its server, spin a restart ladder, or end a
//! session for a child the session can live without.

use crate::recovery::ConfigStatus;
use crate::time::secs;
use crate::{
    Backoff, ChildId, ChildState, EnvTarget, Environment, Event, EventKind, ExitStatus, Guardian, InheritedEnvironment,
    Level, Plan, Recovery, Restart, Rules, SessionState, ShutdownReason, Signal, StopReason, Tick,
};

const COMPOSITOR: ChildId = ChildId::new(1);
const SERVICES: ChildId = ChildId::new(2);
const RUNTIME: ChildId = ChildId::new(3);
const FRONT: ChildId = ChildId::new(4);
const FIRST_APP: ChildId = ChildId::new(5);
const SECOND_APP: ChildId = ChildId::new(6);

const PREFIX: &str = "/opt/gnoblin";

fn inherited() -> InheritedEnvironment<'static> {
    InheritedEnvironment {
        prefix: PREFIX,
        path: "/usr/local/bin:/usr/bin",
        library_path: "/opt/gnoblin/lib64",
        data_dirs: "/usr/local/share:/usr/share",
    }
}

fn plan() -> Plan {
    Plan::standard(true, ["first.desktop".to_owned(), "second.desktop".to_owned()]).expect("valid plan")
}

fn rules() -> Rules {
    Rules::standard(&inherited())
}

fn guardian() -> Guardian {
    Guardian::new(plan(), rules())
}

fn at(now: u64, kind: EventKind) -> Event {
    Event::new(Tick::new(now), kind)
}

fn tick(guardian: &mut Guardian, now: u64) -> Vec<crate::Effect> {
    guardian.apply(at(now, EventKind::Tick))
}

fn display() -> Vec<(String, String)> {
    vec![
        ("WAYLAND_DISPLAY".to_owned(), "wayland-1".to_owned()),
        ("DISPLAY".to_owned(), ":0".to_owned()),
    ]
}

fn generation(guardian: &Guardian, child: ChildId) -> u64 {
    guardian.status(child).expect("planned child").generation
}

fn state(guardian: &Guardian, child: ChildId) -> ChildState {
    guardian.status(child).expect("planned child").state
}

fn ready_with(guardian: &mut Guardian, child: ChildId, instance: u64, now: u64) -> Vec<crate::Effect> {
    guardian.apply(at(
        now,
        EventKind::Ready {
            child,
            generation: instance,
            environment: display(),
        },
    ))
}

fn ready(guardian: &mut Guardian, child: ChildId, now: u64) -> Vec<crate::Effect> {
    ready_with(guardian, child, generation(guardian, child), now)
}

fn exited(guardian: &mut Guardian, child: ChildId, status: ExitStatus, now: u64) -> Vec<crate::Effect> {
    guardian.apply(at(
        now,
        EventKind::Exited {
            child,
            generation: generation(guardian, child),
            status,
        },
    ))
}

/// Drop log lines, so a test states the actions it cares about.
fn actions(effects: Vec<crate::Effect>) -> Vec<crate::Effect> {
    effects
        .into_iter()
        .filter(|effect| !matches!(effect, crate::Effect::Log { .. }))
        .collect()
}

fn signals(effects: Vec<crate::Effect>) -> Vec<ChildId> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            crate::Effect::Signal { child, .. } => Some(child),
            _ => None,
        })
        .collect()
}

fn kills(effects: Vec<crate::Effect>) -> Vec<ChildId> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            crate::Effect::Kill { child } => Some(child),
            _ => None,
        })
        .collect()
}

fn spawns(effects: Vec<crate::Effect>) -> Vec<(ChildId, u64)> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            crate::Effect::Spawn { child, generation, .. } => Some((child, generation)),
            _ => None,
        })
        .collect()
}

fn messages(effects: &[crate::Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            crate::Effect::Log { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn level_of(effects: &[crate::Effect], needle: &str) -> Option<Level> {
    effects.iter().find_map(|effect| match effect {
        crate::Effect::Log { level, message } if message.contains(needle) => Some(*level),
        _ => None,
    })
}

/// A guardian whose session is fully up, with every child started and ready.
fn up() -> Guardian {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    ready(&mut guardian, COMPOSITOR, 1);
    ready(&mut guardian, SERVICES, 2);
    ready(&mut guardian, RUNTIME, 3);
    ready(&mut guardian, FRONT, 4);
    assert_eq!(guardian.session(), SessionState::Up);
    guardian
}

/// Stop the session the way a cooperative caller does: report every signalled
/// child's exit, and tick between ranks.
fn stop_all(guardian: &mut Guardian) {
    let mut now = 1000u64;
    guardian.apply(at(
        now,
        EventKind::Shutdown {
            reason: ShutdownReason::Logout,
        },
    ));
    for _ in 0..16 {
        if guardian.session() == SessionState::Ended {
            return;
        }
        let stopping: Vec<ChildId> = guardian
            .statuses()
            .into_iter()
            .filter(|status| status.state == ChildState::Stopping)
            .map(|status| status.id)
            .collect();
        assert!(!stopping.is_empty(), "the stop sequence made no progress");
        for child in stopping {
            now += 100;
            let instance = generation(guardian, child);
            guardian.apply(at(
                now,
                EventKind::Exited {
                    child,
                    generation: instance,
                    status: ExitStatus::Exited(0),
                },
            ));
        }
        now += 100;
        tick(guardian, now);
    }
    panic!("the session did not stop");
}

#[test]
fn a_crash_looping_runtime_stops_and_degrades_the_session() {
    let mut guardian = up();
    let mut now = 100;
    for attempt in 1..=5u32 {
        let effects = exited(&mut guardian, RUNTIME, ExitStatus::Exited(1), now);
        assert_eq!(guardian.status(RUNTIME).expect("runtime").restarts, attempt);
        assert_eq!(state(&guardian, RUNTIME), ChildState::Backoff);
        let message = messages(&effects)
            .into_iter()
            .find(|message| message.contains("restarting runtime"))
            .expect("a restart is announced");
        let expected = [250u64, 500, 1000, 2000, 4000][usize::try_from(attempt - 1).expect("attempt fits")];
        assert!(
            message.contains(&format!("in {expected} ms")),
            "attempt {attempt} should wait {expected} ms: {message}"
        );
        now += 5000;
        let restarted = spawns(actions(tick(&mut guardian, now)));
        assert_eq!(restarted, vec![(RUNTIME, u64::from(attempt + 1))]);
    }

    // The sixth crash is one too many: no spawn, ever again, and the session says
    // it is degraded instead of pretending to be healthy.
    let effects = exited(&mut guardian, RUNTIME, ExitStatus::Exited(1), now);
    assert_eq!(state(&guardian, RUNTIME), ChildState::Failed);
    assert_eq!(guardian.session(), SessionState::Degraded);
    assert_eq!(
        level_of(&effects, "the session is degraded"),
        Some(Level::Error),
        "the circuit breaker reports itself: {}",
        messages(&effects).join(" | ")
    );
    assert!(spawns(actions(effects)).is_empty());

    for _ in 0..20 {
        now += 10_000;
        assert!(
            spawns(actions(tick(&mut guardian, now))).is_empty(),
            "a failed child must never be spawned again"
        );
    }
}

#[test]
fn a_services_crash_does_not_end_the_session() {
    let mut guardian = up();
    let effects = exited(&mut guardian, SERVICES, ExitStatus::Signalled(Signal::Term), 100);
    assert_eq!(
        guardian.session(),
        SessionState::Up,
        "a services crash is not a session failure"
    );
    assert_eq!(state(&guardian, SERVICES), ChildState::Backoff);
    assert_eq!(kills(effects.clone()), Vec::new());
    assert!(
        signals(effects).is_empty(),
        "nothing else is stopped for a restartable child"
    );

    let restarted = spawns(actions(tick(&mut guardian, 350)));
    assert_eq!(restarted, vec![(SERVICES, 2)]);
    assert_eq!(guardian.session(), SessionState::Up);
    assert_eq!(state(&guardian, COMPOSITOR), ChildState::Running);
}

#[test]
fn a_compositor_crash_stops_every_other_child() {
    let mut guardian = up();
    let mut now = 100;
    let effects = exited(&mut guardian, COMPOSITOR, ExitStatus::Exited(1), now);
    assert_eq!(guardian.session(), SessionState::Stopping);
    assert_eq!(guardian.stop_reason(), Some(StopReason::CompositorLost));
    assert_eq!(signals(effects.clone()), vec![SECOND_APP, FIRST_APP]);
    assert!(spawns(actions(effects)).is_empty(), "the compositor is never restarted");

    // Finish the stop and assert nothing was left behind.
    for _ in 0..8 {
        let stopping: Vec<ChildId> = guardian
            .statuses()
            .into_iter()
            .filter(|status| status.state == ChildState::Stopping)
            .map(|status| status.id)
            .collect();
        for child in stopping {
            now += 100;
            let instance = generation(&guardian, child);
            let effects = guardian.apply(at(
                now,
                EventKind::Exited {
                    child,
                    generation: instance,
                    status: ExitStatus::Exited(0),
                },
            ));
            assert!(
                spawns(actions(effects)).is_empty(),
                "no child restarts through a shutdown"
            );
        }
    }
    assert_eq!(guardian.session(), SessionState::Ended);
    assert!(
        guardian.statuses().iter().all(|status| !status.state.is_live()),
        "an orphan is a live child after the session ended"
    );
    assert_eq!(generation(&guardian, COMPOSITOR), 1, "the compositor ran once");
}

#[test]
fn dependants_stop_before_the_compositor() {
    let mut guardian = up();
    let mut now = 100;
    let mut order = signals(guardian.apply(at(
        now,
        EventKind::Shutdown {
            reason: ShutdownReason::Logout,
        },
    )));
    for child in [SECOND_APP, FIRST_APP, FRONT, RUNTIME, SERVICES, COMPOSITOR] {
        now += 100;
        let instance = generation(&guardian, child);
        let effects = guardian.apply(at(
            now,
            EventKind::Exited {
                child,
                generation: instance,
                status: ExitStatus::Exited(0),
            },
        ));
        order.extend(signals(effects));
    }
    assert_eq!(
        order,
        vec![SECOND_APP, FIRST_APP, FRONT, RUNTIME, SERVICES, COMPOSITOR],
        "a client is stopped before the server it talks to"
    );
    assert_eq!(guardian.session(), SessionState::Ended);
    assert_eq!(guardian.stop_reason(), Some(StopReason::Logout));
}

#[test]
fn an_unready_child_is_killed_at_its_deadline() {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    ready(&mut guardian, COMPOSITOR, 1);
    assert_eq!(state(&guardian, SERVICES), ChildState::Starting);

    assert!(
        actions(tick(&mut guardian, 10_000)).is_empty(),
        "the deadline is exclusive"
    );
    let effects = tick(&mut guardian, 10_001);
    assert_eq!(kills(effects.clone()), vec![SERVICES]);
    assert!(
        messages(&effects)
            .iter()
            .any(|message| message.contains("did not report ready")),
        "the deadline is reported: {}",
        messages(&effects).join(" | ")
    );
    assert_eq!(state(&guardian, SERVICES), ChildState::Stopping);

    exited(&mut guardian, SERVICES, ExitStatus::Signalled(Signal::Kill), 10_002);
    assert_eq!(state(&guardian, SERVICES), ChildState::Backoff);
    assert_eq!(guardian.status(SERVICES).expect("services").restarts, 1);
}

#[test]
fn backoff_saturates_instead_of_overflowing() {
    let ladder = Backoff::guardian();
    assert_eq!(ladder.delay(1), Tick::new(250));
    assert_eq!(ladder.delay(2), Tick::new(500));
    assert_eq!(ladder.delay(3), Tick::new(1000));
    assert_eq!(ladder.delay(4), Tick::new(2000));
    assert_eq!(ladder.delay(5), Tick::new(4000));
    assert_eq!(ladder.delay(6), Tick::new(4000));
    assert_eq!(ladder.delay(u32::MAX), Tick::new(4000));

    // A caller-supplied ladder can ask for more than a `u64` holds.
    let huge = Backoff {
        initial: Tick::new(u64::MAX / 3),
        factor: 3,
        max: Tick::new(u64::MAX),
        ..ladder
    };
    assert_eq!(huge.delay(u32::MAX), Tick::new(u64::MAX));
    let still = Backoff {
        initial: Tick::new(0),
        factor: 1,
        ..ladder
    };
    assert_eq!(still.delay(u32::MAX), Tick::new(0));

    // Time itself saturates, and a session that has run for ever still stops.
    assert_eq!(secs(u64::MAX), Tick::new(u64::MAX));
    assert_eq!(Tick::new(u64::MAX).saturating_add(1), Tick::new(u64::MAX));
    let mut guardian = up();
    exited(&mut guardian, RUNTIME, ExitStatus::Exited(1), 1);
    let effects = spawns(actions(tick(&mut guardian, u64::MAX)));
    assert_eq!(effects, vec![(RUNTIME, 2)]);
    assert_eq!(guardian.session(), SessionState::Up);
}

#[test]
fn a_stale_exit_does_not_restart_a_healthy_child() {
    let mut guardian = up();
    exited(&mut guardian, SERVICES, ExitStatus::Exited(1), 100);
    tick(&mut guardian, 350);
    ready(&mut guardian, SERVICES, 351);
    assert_eq!(state(&guardian, SERVICES), ChildState::Running);

    let effects = guardian.apply(at(
        400,
        EventKind::Exited {
            child: SERVICES,
            generation: 1,
            status: ExitStatus::Exited(1),
        },
    ));
    assert!(actions(effects).is_empty(), "an earlier instance's exit is not news");
    assert_eq!(state(&guardian, SERVICES), ChildState::Running);
    assert_eq!(guardian.status(SERVICES).expect("services").restarts, 1);
    assert_eq!(generation(&guardian, SERVICES), 2);
}

#[test]
fn a_child_that_ignores_the_graceful_signal_is_killed() {
    let mut guardian = up();
    let effects = guardian.apply(at(
        100,
        EventKind::Shutdown {
            reason: ShutdownReason::Logout,
        },
    ));
    assert_eq!(signals(effects), vec![SECOND_APP, FIRST_APP]);

    let effects = tick(&mut guardian, 2100);
    assert_eq!(kills(effects), vec![SECOND_APP, FIRST_APP], "the grace period expired");

    // A second grace with no reported exit: the guardian assumes the kill worked
    // and moves on to the next rank instead of hanging the logout.
    let effects = tick(&mut guardian, 4100);
    assert_eq!(signals(effects), vec![FRONT]);
}

#[test]
fn an_unreaped_kill_does_not_hold_the_session() {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    tick(&mut guardian, 10_000);
    assert_eq!(state(&guardian, COMPOSITOR), ChildState::Stopping);

    let effects = tick(&mut guardian, 12_000);
    assert_eq!(
        level_of(&effects, "did not exit after the forced kill"),
        Some(Level::Error)
    );
    assert_eq!(guardian.session(), SessionState::Ended);
    assert_eq!(guardian.stop_reason(), Some(StopReason::StartFailed));
    assert_eq!(guardian.recovery().failed_starts, 1);
    assert!(
        spawns(actions(effects)).is_empty(),
        "nothing starts after a failed start"
    );
}

#[test]
fn a_failed_autostart_entry_does_not_end_the_session() {
    let mut guardian = up();
    let effects = exited(&mut guardian, FIRST_APP, ExitStatus::Exited(1), 100);
    assert_eq!(state(&guardian, FIRST_APP), ChildState::Failed);
    assert_eq!(guardian.session(), SessionState::Up);
    assert_eq!(state(&guardian, SECOND_APP), ChildState::Running);
    assert_eq!(state(&guardian, COMPOSITOR), ChildState::Running);
    assert!(actions(effects).is_empty(), "a dead application is not a session event");
}

#[test]
fn a_front_that_dies_before_the_session_is_up_does_not_block_it() {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    ready(&mut guardian, COMPOSITOR, 1);
    ready(&mut guardian, SERVICES, 2);
    ready(&mut guardian, RUNTIME, 3);
    assert_eq!(state(&guardian, FRONT), ChildState::Starting);

    exited(&mut guardian, FRONT, ExitStatus::Exited(1), 10);
    assert_eq!(state(&guardian, FRONT), ChildState::Failed);
    assert_eq!(guardian.session(), SessionState::Up);
    assert_eq!(state(&guardian, FIRST_APP), ChildState::Running);
    assert_eq!(state(&guardian, SECOND_APP), ChildState::Running);
}

#[test]
fn an_unready_compositor_ends_the_session() {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    let effects = tick(&mut guardian, 10_000);
    assert_eq!(kills(effects), vec![COMPOSITOR]);

    let effects = exited(&mut guardian, COMPOSITOR, ExitStatus::Signalled(Signal::Kill), 10_050);
    assert_eq!(guardian.session(), SessionState::Ended);
    assert_eq!(guardian.stop_reason(), Some(StopReason::StartFailed));
    assert_eq!(guardian.recovery().failed_starts, 1);
    assert!(
        guardian
            .statuses()
            .iter()
            .any(|status| status.id == COMPOSITOR && status.last_exit == Some(ExitStatus::Signalled(Signal::Kill)))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, crate::Effect::SetRecovery(_))),
        "a failed start is persisted for the next login"
    );
    assert!(spawns(effects).is_empty());
}

#[test]
fn a_compositor_without_a_display_environment_ends_the_session() {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    let effects = guardian.apply(at(
        1,
        EventKind::Ready {
            child: COMPOSITOR,
            generation: 1,
            environment: Vec::new(),
        },
    ));
    assert_eq!(
        level_of(&effects, "without WAYLAND_DISPLAY"),
        Some(Level::Error),
        "a session with no display fails closed: {}",
        messages(&effects).join(" | ")
    );
    assert_eq!(guardian.session(), SessionState::Stopping);
    assert_eq!(guardian.stop_reason(), Some(StopReason::StartFailed));
    assert_eq!(signals(effects.clone()), vec![COMPOSITOR]);
    assert!(spawns(actions(effects)).is_empty(), "no child starts without a display");
    assert_eq!(state(&guardian, SERVICES), ChildState::Pending);
}

#[test]
fn safe_mode_is_requested_after_the_configured_failed_starts() {
    let mut guardian = Guardian::with_recovery(
        plan(),
        rules(),
        Recovery {
            failed_starts: 2,
            ..Recovery::default()
        },
    );
    let effects = guardian.apply(at(0, EventKind::Start));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, crate::Effect::SetRecovery(_))),
        "two failed starts is below the threshold"
    );

    tick(&mut guardian, 10_000);
    exited(&mut guardian, COMPOSITOR, ExitStatus::Signalled(Signal::Kill), 10_050);
    assert_eq!(guardian.recovery().failed_starts, 3);
    assert!(!guardian.recovery().safe_mode, "safe mode starts with the next session");

    let next_recovery = guardian.recovery().clone();
    let mut next = Guardian::with_recovery(plan(), rules(), next_recovery);
    let effects = next.apply(at(0, EventKind::Start));
    assert!(next.recovery().safe_mode);
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            crate::Effect::SetRecovery(recovery) if recovery.safe_mode
        )),
        "safe mode is persisted so the next session knows"
    );
    let spawn = actions(effects)
        .into_iter()
        .find_map(|effect| match effect {
            crate::Effect::Spawn { environment, .. } => Some(environment),
            _ => None,
        })
        .expect("the compositor is spawned");
    assert_eq!(spawn.get("GNOBLIN_SAFE_MODE"), Some("1"));

    // A successful start clears the count but keeps safe mode until the user's
    // configuration is accepted again.
    ready(&mut next, COMPOSITOR, 1);
    ready(&mut next, SERVICES, 2);
    ready(&mut next, RUNTIME, 3);
    ready(&mut next, FRONT, 4);
    assert_eq!(next.session(), SessionState::Up);
    assert_eq!(next.recovery().failed_starts, 0);
    assert!(next.recovery().safe_mode);
    next.apply(at(
        10,
        EventKind::ConfigAccepted {
            revision: 4,
            autostart: Vec::new(),
        },
    ));
    assert!(!next.recovery().safe_mode, "an accepted configuration leaves safe mode");
}

#[test]
fn a_rejected_configuration_keeps_the_last_good_revision() {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    guardian.apply(at(
        1,
        EventKind::ConfigAccepted {
            revision: 7,
            autostart: Vec::new(),
        },
    ));
    let effects = guardian.apply(at(
        2,
        EventKind::ConfigRejected {
            revision: 8,
            diagnostic: "unknown key input.touchpad.speed".to_owned(),
        },
    ));
    assert_eq!(guardian.recovery().config.last_good, Some(7));
    assert_eq!(
        guardian.recovery().config.status,
        ConfigStatus::Rejected {
            revision: 8,
            diagnostic: "unknown key input.touchpad.speed".to_owned(),
        }
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, crate::Effect::SetRecovery(_)))
    );

    let effects = guardian.apply(at(
        3,
        EventKind::ConfigFallback {
            reason: "saved document is from an older schema".to_owned(),
        },
    ));
    assert_eq!(guardian.recovery().config.last_good, Some(7));
    assert_eq!(
        guardian.recovery().config.status,
        ConfigStatus::Defaults {
            reason: "saved document is from an older schema".to_owned(),
        }
    );
    assert!(
        messages(&effects)
            .iter()
            .any(|message| message.contains("embedded defaults"))
    );
}

#[test]
fn the_compositor_display_environment_reaches_later_children() {
    let mut guardian = guardian();
    let started = actions(guardian.apply(at(0, EventKind::Start)));
    let first_environment = started
        .into_iter()
        .find_map(|effect| match effect {
            crate::Effect::Spawn { environment, .. } => Some(environment),
            _ => None,
        })
        .expect("the compositor is spawned");
    assert_eq!(
        first_environment.get("WAYLAND_DISPLAY"),
        None,
        "no display exists before the compositor makes one"
    );
    assert_eq!(
        first_environment.get("PATH"),
        Some("/opt/gnoblin/bin:/usr/local/bin:/usr/bin")
    );
    assert!(first_environment.unset.contains(&"XAUTHORITY".to_owned()));

    let effects = actions(ready(&mut guardian, COMPOSITOR, 1));
    let published = Environment {
        set: display(),
        unset: Vec::new(),
    };
    assert_eq!(
        effects[0],
        crate::Effect::ExportEnv {
            target: EnvTarget::Session,
            environment: published.clone()
        }
    );
    assert_eq!(
        effects[1],
        crate::Effect::ExportEnv {
            target: EnvTarget::Activation,
            environment: published
        }
    );
    let next_environment = effects
        .into_iter()
        .find_map(|effect| match effect {
            crate::Effect::Spawn { environment, .. } => Some(environment),
            _ => None,
        })
        .expect("services are spawned after the display");
    assert_eq!(next_environment.get("WAYLAND_DISPLAY"), Some("wayland-1"));
    assert_eq!(next_environment.get("DISPLAY"), Some(":0"));
    assert_eq!(guardian.environment().get("WAYLAND_DISPLAY"), Some("wayland-1"));
}

#[test]
fn the_privileged_display_reaches_later_children() {
    // The privileged socket is how a user-declared tool gets screencopy. The compositor names
    // it in its ready report, and the guardian must hand that name to every later child, or
    // the tool has no way to find the socket.
    let mut guardian = guardian();
    let _ = actions(guardian.apply(at(0, EventKind::Start)));
    let mut environment = display();
    environment.push((
        "GNOBLIN_PRIVILEGED_DISPLAY".to_owned(),
        "gnoblin-wayland-privileged.sock".to_owned(),
    ));
    let effects = actions(guardian.apply(at(
        1,
        EventKind::Ready {
            child: COMPOSITOR,
            generation: generation(&guardian, COMPOSITOR),
            environment,
        },
    )));
    let next_environment = effects
        .into_iter()
        .find_map(|effect| match effect {
            crate::Effect::Spawn { environment, .. } => Some(environment),
            _ => None,
        })
        .expect("services are spawned after the display");
    assert_eq!(
        next_environment.get("GNOBLIN_PRIVILEGED_DISPLAY"),
        Some("gnoblin-wayland-privileged.sock")
    );
    assert_eq!(
        guardian.environment().get("GNOBLIN_PRIVILEGED_DISPLAY"),
        Some("gnoblin-wayland-privileged.sock")
    );
}

#[test]
fn events_after_the_session_ended_change_nothing() {
    let mut guardian = up();
    stop_all(&mut guardian);
    assert_eq!(guardian.session(), SessionState::Ended);

    let mut now = 5000;
    for kind in [
        EventKind::Tick,
        EventKind::Start,
        EventKind::Shutdown {
            reason: ShutdownReason::Signal,
        },
        EventKind::Ready {
            child: COMPOSITOR,
            generation: 1,
            environment: display(),
        },
        EventKind::Exited {
            child: COMPOSITOR,
            generation: 1,
            status: ExitStatus::Exited(0),
        },
    ] {
        now += 1000;
        let effects = guardian.apply(at(now, kind.clone()));
        assert!(
            actions(effects).is_empty(),
            "{kind:?} after the session ended produced effects"
        );
    }
    assert_eq!(guardian.session(), SessionState::Ended);
    assert_eq!(
        guardian
            .statuses()
            .iter()
            .filter(|status| status.state.is_live())
            .count(),
        0
    );
}

#[test]
fn a_stable_child_gets_its_restart_budget_back() {
    let mut guardian = up();
    exited(&mut guardian, RUNTIME, ExitStatus::Exited(1), 100);
    tick(&mut guardian, 350);
    ready(&mut guardian, RUNTIME, 400);
    assert_eq!(guardian.status(RUNTIME).expect("runtime").restarts, 1);

    tick(&mut guardian, 60_400);
    assert_eq!(
        guardian.status(RUNTIME).expect("runtime").restarts,
        0,
        "a minute of uptime resets the ladder"
    );

    let effects = exited(&mut guardian, RUNTIME, ExitStatus::Exited(1), 60_500);
    assert_eq!(guardian.status(RUNTIME).expect("runtime").restarts, 1);
    assert!(
        messages(&effects)
            .iter()
            .any(|message| message.contains("restarting runtime in 250 ms")),
        "the ladder starts at its first rung again: {}",
        messages(&effects).join(" | ")
    );
}

#[test]
fn restart_policies_match_the_plan() {
    let plan = plan();
    assert_eq!(plan.spec(COMPOSITOR).expect("compositor").restart, Restart::EndsSession);
    assert_eq!(
        plan.spec(SERVICES).expect("services").restart,
        Restart::Backoff(Backoff::guardian())
    );
    assert_eq!(
        plan.spec(RUNTIME).expect("runtime").restart,
        Restart::Backoff(Backoff::guardian())
    );
    assert_eq!(plan.spec(FRONT).expect("front").restart, Restart::Never);
    assert_eq!(plan.spec(FIRST_APP).expect("autostart").restart, Restart::Never);
    assert_eq!(plan.spec(FIRST_APP).expect("autostart").ready_within, None);

    // The plan refuses the orders that would leave a child without its display.
    let no_compositor = Plan::new(vec![crate::ChildSpec::new(
        ChildId::new(1),
        crate::ChildKind::Runtime,
        "runtime",
    )]);
    assert_eq!(no_compositor, Err(crate::PlanError::MissingCompositor));
    let late_compositor = Plan::new(vec![
        crate::ChildSpec::new(ChildId::new(1), crate::ChildKind::Runtime, "runtime"),
        crate::ChildSpec::new(ChildId::new(2), crate::ChildKind::Compositor, "compositor"),
    ]);
    assert_eq!(late_compositor, Err(crate::PlanError::CompositorNotFirst));
    let duplicate = Plan::new(vec![
        crate::ChildSpec::new(ChildId::new(1), crate::ChildKind::Compositor, "compositor"),
        crate::ChildSpec::new(ChildId::new(1), crate::ChildKind::Runtime, "runtime"),
    ]);
    assert_eq!(duplicate, Err(crate::PlanError::DuplicateId(ChildId::new(1))));
}

#[test]
fn a_configuration_that_keeps_an_entry_leaves_its_application_running() {
    let mut guardian = up();
    let effects = guardian.apply(at(
        100,
        EventKind::ConfigAccepted {
            revision: 2,
            autostart: vec![
                crate::AutostartEntry {
                    name: "first.desktop".to_owned(),
                    command: Vec::new(),
                },
                crate::AutostartEntry {
                    name: "second.desktop".to_owned(),
                    command: Vec::new(),
                },
            ],
        },
    ));
    assert!(signals(effects).is_empty(), "a kept entry is not signalled");
    assert_eq!(state(&guardian, FIRST_APP), ChildState::Running);
    assert_eq!(state(&guardian, SECOND_APP), ChildState::Running);
}

#[test]
fn a_running_application_the_configuration_drops_is_asked_to_exit_and_never_restarts() {
    let mut guardian = up();
    let effects = guardian.apply(at(
        100,
        EventKind::ConfigAccepted {
            revision: 2,
            autostart: Vec::new(),
        },
    ));
    assert_eq!(signals(effects), vec![FIRST_APP, SECOND_APP]);
    assert_eq!(state(&guardian, FIRST_APP), ChildState::Stopping);

    let effects = exited(&mut guardian, FIRST_APP, ExitStatus::Exited(0), 110);
    assert_eq!(state(&guardian, FIRST_APP), ChildState::Stopped);
    assert!(spawns(effects).is_empty(), "a retired application is not restarted");
    assert_eq!(
        guardian.session(),
        SessionState::Up,
        "retiring one application ends nothing"
    );
}

#[test]
fn a_retired_application_that_ignores_its_grace_period_is_killed() {
    let mut guardian = up();
    guardian.apply(at(
        100,
        EventKind::ConfigAccepted {
            revision: 2,
            autostart: Vec::new(),
        },
    ));
    let effects = tick(&mut guardian, 10_000);
    assert!(kills(effects).contains(&FIRST_APP));
}

#[test]
fn a_pending_application_the_configuration_drops_is_forgotten() {
    let mut guardian = guardian();
    guardian.apply(at(0, EventKind::Start));
    ready(&mut guardian, COMPOSITOR, 1);
    assert_eq!(state(&guardian, FIRST_APP), ChildState::Pending);
    let effects = guardian.apply(at(
        2,
        EventKind::ConfigAccepted {
            revision: 2,
            autostart: Vec::new(),
        },
    ));
    assert!(signals(effects).is_empty(), "nothing to signal before a spawn");
    assert_eq!(state(&guardian, FIRST_APP), ChildState::Stopped);
}

#[test]
fn a_new_autostart_entry_gets_a_fresh_child_while_the_session_is_up() {
    let mut guardian = up();
    let effects = guardian.apply(at(
        100,
        EventKind::ConfigAccepted {
            revision: 2,
            autostart: vec![crate::AutostartEntry {
                name: "third.desktop".to_owned(),
                command: vec!["third".to_owned()],
            }],
        },
    ));
    let added = ChildId::new(7);
    assert_eq!(guardian.status(added).expect("added child").name, "third.desktop");
    assert!(spawns(effects).iter().any(|(child, _)| *child == added));
}

#[test]
fn a_configuration_accepted_during_shutdown_starts_nothing() {
    let mut guardian = up();
    guardian.apply(at(
        100,
        EventKind::Shutdown {
            reason: crate::ShutdownReason::Logout,
        },
    ));
    let effects = guardian.apply(at(
        101,
        EventKind::ConfigAccepted {
            revision: 2,
            autostart: vec![crate::AutostartEntry {
                name: "late.desktop".to_owned(),
                command: vec!["late".to_owned()],
            }],
        },
    ));
    assert!(spawns(effects).is_empty());
    assert!(
        guardian.status(ChildId::new(7)).is_none(),
        "no child is added while stopping"
    );
}
