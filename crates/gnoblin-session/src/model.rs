//! Model-based tests over random event sequences.
//!
//! A scenario test proves one transition. These check the invariants that hold in
//! *every* reachable state, over a hand-written deterministic generator: no
//! dependency, no clock and no seed from the environment, so a failure replays by
//! seed alone.

use crate::{
    ChildId, ChildState, Effect, Event, EventKind, ExitStatus, Guardian, InheritedEnvironment, Plan, Rules,
    SessionState, ShutdownReason, Signal, Tick,
};

const COMPOSITOR: ChildId = ChildId::new(1);
const SERVICES: ChildId = ChildId::new(2);
const RUNTIME: ChildId = ChildId::new(3);
const FRONT: ChildId = ChildId::new(4);
const FIRST_APP: ChildId = ChildId::new(5);
const SECOND_APP: ChildId = ChildId::new(6);
const CHILDREN: [ChildId; 6] = [COMPOSITOR, SERVICES, RUNTIME, FRONT, FIRST_APP, SECOND_APP];

/// A 64-bit linear congruential generator: small, hand-written and identical on
/// every platform, so a failing seed replays exactly.
struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn plan() -> Plan {
    Plan::standard(true, ["first.desktop".to_owned(), "second.desktop".to_owned()]).expect("valid plan")
}

fn rules() -> Rules {
    Rules::standard(&InheritedEnvironment {
        prefix: "/opt/gnoblin",
        path: "/usr/bin",
        library_path: "/opt/gnoblin/lib64",
        data_dirs: "/usr/share",
    })
}

fn slot(child: ChildId) -> usize {
    usize::try_from(child.get() - 1).expect("plan ids start at one")
}

/// What the harness has observed in the effects, mirrored per child.
///
/// The guardian is pure, so the *caller's* view of which processes exist has to
/// be tracked here: that is what makes "at most one live instance" and "every
/// spawn is matched" checkable.
#[derive(Default)]
struct Observed {
    generation: [u64; 6],
    live: [bool; 6],
    spawns: [u32; 6],
    exits: [u32; 6],
    kills: [u32; 6],
}

struct Harness {
    observed: Observed,
    events: u64,
    /// Whether the session ever became usable in this sequence.
    reached_up: bool,
}

impl Harness {
    fn new() -> Self {
        Self {
            observed: Observed::default(),
            events: 0,
            reached_up: false,
        }
    }

    /// Apply one event and check everything that must hold afterwards.
    fn apply(&mut self, guardian: &mut Guardian, event: Event, seed: u64, step: usize) {
        self.events += 1;
        let before = guardian.session();
        let exited = match &event.kind {
            EventKind::Exited { child, generation, .. } => Some((*child, *generation)),
            _ => None,
        };
        let effects = guardian.apply(event);
        if guardian.session() == SessionState::Up {
            self.reached_up = true;
        }

        if let Some((child, generation)) = exited {
            let index = slot(child);
            if self.observed.generation[index] == generation && self.observed.live[index] {
                self.observed.live[index] = false;
                self.observed.exits[index] += 1;
            }
        }

        for effect in &effects {
            match effect {
                Effect::Spawn { child, generation, .. } => {
                    let index = slot(*child);
                    assert!(
                        !self.observed.live[index],
                        "seed {seed} step {step}: a second instance of {child:?} was spawned while one is live"
                    );
                    assert_eq!(
                        *generation,
                        self.observed.generation[index] + 1,
                        "seed {seed} step {step}: {child:?} skipped a generation"
                    );
                    assert!(
                        before.accepts_children(),
                        "seed {seed} step {step}: {child:?} was spawned in {before:?}"
                    );
                    self.observed.generation[index] = *generation;
                    self.observed.live[index] = true;
                    self.observed.spawns[index] += 1;
                }
                Effect::Kill { child } => {
                    let index = slot(*child);
                    self.observed.kills[index] += 1;
                    assert!(
                        self.observed.live[index],
                        "seed {seed} step {step}: {child:?} was killed while no instance of it is live"
                    );
                    self.observed.live[index] = false;
                }
                _ => {}
            }
        }

        self.invariants(guardian, seed, step);
    }

    fn invariants(&self, guardian: &Guardian, seed: u64, step: usize) {
        // The compositor runs once and is never restarted: its exit ends the
        // session, and a restart would leave every client talking to a dead
        // server's replacement.
        assert!(
            self.observed.spawns[slot(COMPOSITOR)] <= 1,
            "seed {seed} step {step}: the compositor was spawned more than once"
        );

        // A child the harness believes is live is a child the guardian agrees is
        // live: the guardian never forgets a process it started.
        for child in CHILDREN {
            let index = slot(child);
            if self.observed.live[index] {
                let state = guardian.status(child).expect("planned child").state;
                assert!(
                    state.is_live() || state == ChildState::Stopping,
                    "seed {seed} step {step}: {child:?} is live in the caller's view but {} here",
                    state.as_str()
                );
            }
        }

        // An ended session owns no process and accepts no spawn.
        if guardian.session() == SessionState::Ended {
            for status in guardian.statuses() {
                assert!(
                    !status.state.is_live(),
                    "seed {seed} step {step}: {:?} is {} after the session ended",
                    status.id,
                    status.state.as_str()
                );
            }
        }
    }

    /// Start the session and bring every child up, so the random phase explores
    /// restarts and stops instead of spending its steps failing to start.
    fn bring_up(&mut self, guardian: &mut Guardian, seed: u64, now: &mut u64) {
        self.apply(guardian, Event::new(Tick::new(*now), EventKind::Start), seed, 0);
        for child in [COMPOSITOR, SERVICES, RUNTIME, FRONT] {
            *now += 10;
            let generation = guardian.status(child).expect("planned child").generation;
            self.apply(
                guardian,
                Event::new(
                    Tick::new(*now),
                    EventKind::Ready {
                        child,
                        generation,
                        environment: display_environment(),
                    },
                ),
                seed,
                0,
            );
        }
    }

    /// Stop the session and drive it to `Ended`, reporting every exit the way a
    /// cooperative caller does. The force path is exercised by ticking past the
    /// grace period before reporting, on alternate seeds.
    fn stop(&mut self, guardian: &mut Guardian, seed: u64, force: bool) {
        if guardian.session() != SessionState::Ended {
            let now = guardian.now().saturating_add(100);
            self.apply(
                guardian,
                Event::new(
                    now,
                    EventKind::Shutdown {
                        reason: ShutdownReason::Logout,
                    },
                ),
                seed,
                0,
            );
        }
        for round in 0..64 {
            if guardian.session() == SessionState::Ended {
                assert!(
                    guardian.statuses().iter().all(|status| !status.state.is_live()),
                    "seed {seed}: the session ended with a live child"
                );
                return;
            }
            let now = guardian.now().saturating_add(if force { 3000 } else { 500 });
            self.apply(guardian, Event::new(now, EventKind::Tick), seed, round);
            let live: Vec<(ChildId, u64)> = CHILDREN
                .iter()
                .filter(|child| self.observed.live[slot(**child)])
                .map(|child| (*child, self.observed.generation[slot(*child)]))
                .collect();
            for (child, generation) in live {
                // A shut-down child is not restarted, so leaving it live is the
                // one way this loop could spin.
                self.apply(
                    guardian,
                    Event::new(
                        now,
                        EventKind::Exited {
                            child,
                            generation,
                            status: ExitStatus::Exited(0),
                        },
                    ),
                    seed,
                    round,
                );
            }
        }
        panic!("seed {seed}: the session never stopped");
    }
}

/// Pick one child out of a candidate list, or nothing when the list is empty.
fn pick(rng: &mut Rng, from: &[ChildId]) -> Option<ChildId> {
    if from.is_empty() {
        return None;
    }
    let index = usize::try_from(rng.below(u64::try_from(from.len()).expect("list length fits"))).expect("fits");
    from.get(index).copied()
}

fn display_environment() -> Vec<(String, String)> {
    vec![
        ("WAYLAND_DISPLAY".to_owned(), "wayland-0".to_owned()),
        ("DISPLAY".to_owned(), ":0".to_owned()),
    ]
}

/// Every event a caller can send, at the frequency an adversarial sequence would:
/// repeats, stale generations, exits for children that never ran, and a shutdown
/// in the middle of a restart are all common.
///
/// Readiness is sent often enough that sequences reach a usable session, because
/// the restart, stop-order and degraded paths only exist once something has
/// started; the crashes, stale generations and shutdowns are what then break it.
fn generate(rng: &mut Rng, guardian: &Guardian, now: &mut u64) -> Event {
    *now += rng.below(700);
    let moment = Tick::new(*now);
    let statuses = guardian.statuses();
    let starting: Vec<ChildId> = statuses
        .iter()
        .filter(|status| status.state == ChildState::Starting)
        .map(|status| status.id)
        .collect();
    let live: Vec<ChildId> = statuses
        .iter()
        .filter(|status| status.state.is_live())
        .map(|status| status.id)
        .collect();
    let generation = |child: ChildId| guardian.status(child).expect("planned child").generation;
    let kind = match rng.below(22) {
        0 => EventKind::Start,
        // Move the start sequence along, so the session reaches `Up`.
        1..=6 => match pick(rng, &starting) {
            Some(child) => EventKind::Ready {
                child,
                generation: generation(child),
                environment: display_environment(),
            },
            None => EventKind::Tick,
        },
        7 | 8 => EventKind::Tick,
        // A readiness report that is a duplicate, stale, or from a child that is
        // not starting at all.
        9 | 10 => {
            let child = pick(rng, &live).unwrap_or(COMPOSITOR);
            EventKind::Ready {
                child,
                generation: generation(child).saturating_sub(1),
                environment: display_environment(),
            }
        }
        // The interesting failures: a live child ends.
        11..=16 => match pick(rng, &live) {
            Some(child) => EventKind::Exited {
                child,
                generation: generation(child),
                status: if rng.below(3) == 0 {
                    ExitStatus::Exited(0)
                } else {
                    ExitStatus::Exited(1)
                },
            },
            None => EventKind::Tick,
        },
        // A late exit from an earlier instance.
        17 | 18 => {
            let child = CHILDREN[usize::try_from(rng.below(6)).expect("six children")];
            EventKind::Exited {
                child,
                generation: generation(child).saturating_sub(1),
                status: ExitStatus::Signalled(Signal::Term),
            }
        }
        19 => EventKind::Shutdown {
            reason: if rng.below(2) == 0 {
                ShutdownReason::Logout
            } else {
                ShutdownReason::Signal
            },
        },
        _ => {
            if rng.below(2) == 0 {
                EventKind::ConfigAccepted {
                    revision: rng.below(9) + 1,
                }
            } else {
                EventKind::ConfigRejected {
                    revision: rng.below(9) + 1,
                    diagnostic: "rejected by the corpus".to_owned(),
                }
            }
        }
    };
    Event::new(moment, kind)
}

#[test]
fn invariants_hold_over_random_sequences() {
    let sequences = 200u64;
    let steps = 80;
    let mut totals = Observed::default();
    let mut events = 0u64;
    let mut stops = 0u64;
    let mut reached_up = 0u64;
    let mut compositor_lost = 0u64;
    let mut logouts = 0u64;

    for seed in 1..=sequences {
        let mut rng = Rng::new(seed);
        let mut guardian = Guardian::new(plan(), rules());
        let mut harness = Harness::new();
        let mut now = 0u64;
        harness.bring_up(&mut guardian, seed, &mut now);
        assert_eq!(
            guardian.session(),
            SessionState::Up,
            "seed {seed}: the session did not come up"
        );
        for step in 0..steps {
            let event = generate(&mut rng, &guardian, &mut now);
            harness.apply(&mut guardian, event, seed, usize::try_from(step).expect("step fits"));
        }
        harness.stop(&mut guardian, seed, seed % 2 == 0);
        stops += 1;
        match guardian.stop_reason() {
            Some(crate::StopReason::CompositorLost) => compositor_lost += 1,
            Some(crate::StopReason::Logout) => logouts += 1,
            _ => {}
        }

        // Every spawned instance ended by an exit the caller reported or by a
        // kill: nothing is left running once the session has stopped.
        for child in CHILDREN {
            let index = slot(child);
            assert_eq!(
                harness.observed.spawns[index],
                harness.observed.exits[index] + harness.observed.kills[index],
                "seed {seed}: {child:?} has {} spawns, {} exits and {} kills",
                harness.observed.spawns[index],
                harness.observed.exits[index],
                harness.observed.kills[index]
            );
        }
        // A child never spins: the budget bounds every restartable one.
        for child in [SERVICES, RUNTIME] {
            assert!(
                harness.observed.spawns[slot(child)] <= 1 + crate::Backoff::guardian().max_restarts,
                "seed {seed}: {child:?} was spawned {} times",
                harness.observed.spawns[slot(child)]
            );
        }
        for child in [FRONT, FIRST_APP, SECOND_APP] {
            assert!(
                harness.observed.spawns[slot(child)] <= 1,
                "seed {seed}: {child:?} is not restartable"
            );
        }
        assert!(
            harness.observed.spawns[slot(COMPOSITOR)] <= 1,
            "seed {seed}: the compositor is never restarted"
        );

        events += harness.events;
        if harness.reached_up {
            reached_up += 1;
        }
        for index in 0..6 {
            totals.spawns[index] += harness.observed.spawns[index];
            totals.exits[index] += harness.observed.exits[index];
            totals.kills[index] += harness.observed.kills[index];
        }
    }

    // The generator must actually reach the interesting paths: a session that
    // came up, a restart ladder, and the circuit breaker. Otherwise these tests
    // prove only that idle sequences are quiet.
    assert!(
        reached_up * 4 >= sequences,
        "only {reached_up} of {sequences} sequences reached a usable session"
    );
    assert!(
        compositor_lost > 0,
        "no sequence lost its compositor, so the orphan-stopping path was never taken"
    );
    assert!(logouts > 0, "no sequence logged out cleanly");
    let restartable_spawns: u32 = totals.spawns[slot(SERVICES)] + totals.spawns[slot(RUNTIME)];
    assert!(
        restartable_spawns > u32::try_from(sequences).expect("fits"),
        "the restart path was never taken: {restartable_spawns} spawns of the restartable children"
    );

    let spawns: u32 = totals.spawns.iter().sum();
    let exits: u32 = totals.exits.iter().sum();
    let kills: u32 = totals.kills.iter().sum();
    println!(
        "guardian model: {sequences} sequences, {steps} steps each, {events} applied events, \
         {stops} full stops ({reached_up} after a usable start, {logouts} logouts, \
         {compositor_lost} lost compositors), \
         {spawns} spawns ({exits} answered by exits, {kills} by kills), 0 invariant violations"
    );
    assert_eq!(spawns, exits + kills);
    assert!(events >= sequences * u64::try_from(steps).expect("steps fit"));
}

/// The model must also hold when the caller only ever ticks: restart ladders and
/// deadlines move, and nothing is ever reported ready.
#[test]
fn deadlines_hold_without_readiness_or_exits() {
    let mut guardian = Guardian::new(plan(), rules());
    let mut harness = Harness::new();
    harness.apply(&mut guardian, Event::new(Tick::ZERO, EventKind::Start), 0, 0);
    let mut now = 0u64;
    for step in 0..400 {
        now += 1000;
        harness.apply(&mut guardian, Event::new(Tick::new(now), EventKind::Tick), 0, step);
    }
    // The compositor missed its deadline, so the session was stopped, and its
    // start counted as a failure.
    assert_eq!(guardian.session(), SessionState::Ended);
    assert_eq!(guardian.recovery().failed_starts, 1);
    assert_eq!(harness.observed.spawns[slot(COMPOSITOR)], 1);
    assert!(harness.observed.spawns[slot(SERVICES)] == 0);
}
