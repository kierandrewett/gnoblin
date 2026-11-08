//! Model-based test: random sequences over every state machine in the crate.
//!
//! The generator is a small hand-written xorshift, so the suite needs no dependency and a
//! failure is reproducible from its seed. Each sequence runs twice: the recorded decision
//! stream must be identical, which is the determinism the parity harness relies on.
//!
//! Invariants:
//!
//! - every key and button press is delivered or consumed exactly once: the router reports
//!   exactly one target per edge, and the counted edges equal the edges fed;
//! - a release is routed to the same target as its press, whatever the priority order or
//!   the binding table says by then, and an unpaired release is counted instead of being
//!   silently matched;
//! - the set of held edges the router remembers is exactly the set of presses it has
//!   routed without a release;
//! - at most one gesture binding reserves a movement, and a reservation ends at the end
//!   or cancel of its gesture;
//! - every queued policy request is decided, expired or retired exactly once, and
//!   retirement is in submission order;
//! - the idle clock never moves backwards, and activity counts every event.

mod common;

use std::collections::BTreeMap;

use common::{CODE_Q, CODE_W, Q, W, button_event, chord, name, pointer_chord};
use gnoblin_input::{
    Activity, ButtonState, Contact, ContactPhase, Decision, Event, GestureAction, GestureConfig, GestureEvent,
    GestureKind, GestureLock, GesturePhase, GestureRoute, GestureRouter, GestureSpec, GrabId, KeyCode, KeyState,
    Keysym, Modifiers, PinchDirection, PolicyQueue, Recogniser, Reply, RequestId, Routed, Router, Scope,
    ShortcutInhibit, Submitted, Target, Tick, Trigger,
};

/// Sequences per test run.
const SEQUENCES: u64 = 64;

/// Steps per sequence.
const STEPS_PER_SEQUENCE: u32 = 320;

/// A failing run must report at least this many invariant checks, so a suite that quietly
/// stopped exercising a state machine is visible.
const MIN_CHECKS: usize = 18_000;

/// A deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }
}

/// Everything under test, plus what a compositor would have to remember.
struct Model {
    router: Router,
    gestures: GestureRouter,
    recogniser: Recogniser,
    policy: PolicyQueue,
    activity: Activity,
    held: BTreeMap<KeyCode, Target>,
    held_buttons: BTreeMap<u8, Target>,
    grabs: Vec<GrabId>,
    waiting: Vec<u64>,
    tick: u64,
    touch: u8,
    presses: u64,
    releases: u64,
    orphans: u64,
    checks: usize,
    digest: Vec<u64>,
}

impl Model {
    fn new() -> Self {
        let mut router = Router::new();
        router
            .bind(&name("super_q"), chord("<Super>q"), Trigger::Press, false)
            .expect("the first binding takes its chord");
        router
            .bind(&name("plain_q"), chord("q"), Trigger::Press, true)
            .expect("a different chord");
        router
            .bind(&name("shift_w"), chord("<Shift>w"), Trigger::Release, false)
            .expect("a different chord");
        router
            .bind(&name("resize"), pointer_chord("<Super>Button3"), Trigger::Press, false)
            .expect("a pointer chord is its own chord");
        let gestures = GestureRouter::new(vec![
            GestureSpec {
                name: "two_finger_right".to_owned(),
                gesture: Some(GestureKind::Swipe),
                fingers: Some(2),
                path: Some(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]),
                action: Some(GestureAction::Command(vec!["kgx".to_owned()])),
                ..GestureSpec::default()
            }
            .validate()
            .expect("a valid swipe"),
            GestureSpec {
                name: "two_finger_out".to_owned(),
                gesture: Some(GestureKind::Pinch),
                fingers: Some(2),
                direction: Some(PinchDirection::Out),
                action: Some(GestureAction::Action("workspace.next".to_owned())),
                ..GestureSpec::default()
            }
            .validate()
            .expect("a valid pinch"),
        ])
        .expect("a swipe and a pinch may share a finger count");
        Self {
            router,
            gestures,
            recogniser: Recogniser::new(GestureConfig::default()),
            policy: PolicyQueue::new(8),
            activity: Activity::new(),
            held: BTreeMap::new(),
            held_buttons: BTreeMap::new(),
            grabs: Vec::new(),
            waiting: Vec::new(),
            tick: 1,
            touch: 0,
            presses: 0,
            releases: 0,
            orphans: 0,
            checks: 0,
            digest: Vec::new(),
        }
    }

    fn step(&mut self, rng: &mut Rng) {
        self.tick += 1;
        let tick = Tick::new(self.tick);
        match rng.below(14) {
            0..=2 => {
                let (code, symbol) = if rng.chance(2) { (CODE_Q, Q) } else { (CODE_W, W) };
                let modifiers = if rng.chance(2) {
                    Modifiers::SUPER
                } else {
                    Modifiers::NONE
                };
                let inhibit = if rng.chance(3) {
                    ShortcutInhibit::Inhibited
                } else {
                    ShortcutInhibit::Clear
                };
                self.press_key(code, symbol, modifiers, inhibit);
            }
            3..=4 => {
                let (code, symbol) = if rng.chance(2) { (CODE_Q, Q) } else { (CODE_W, W) };
                let modifiers = if rng.chance(2) {
                    Modifiers::SUPER
                } else {
                    Modifiers::NONE
                };
                let inhibit = if rng.chance(3) {
                    ShortcutInhibit::Inhibited
                } else {
                    ShortcutInhibit::Clear
                };
                self.release_key(code, symbol, modifiers, inhibit);
            }
            5 => {
                let button = 1 + (rng.below(3) as u8);
                let modifiers = if rng.chance(2) {
                    Modifiers::SUPER
                } else {
                    Modifiers::NONE
                };
                let inhibit = if rng.chance(3) {
                    ShortcutInhibit::Inhibited
                } else {
                    ShortcutInhibit::Clear
                };
                self.press_button(button, modifiers, inhibit);
            }
            6 => {
                let button = 1 + (rng.below(3) as u8);
                let modifiers = if rng.chance(2) {
                    Modifiers::SUPER
                } else {
                    Modifiers::NONE
                };
                let inhibit = if rng.chance(3) {
                    ShortcutInhibit::Inhibited
                } else {
                    ShortcutInhibit::Clear
                };
                self.release_button(button, modifiers, inhibit);
            }
            7 => self.grab_step(rng),
            8 => self.reload(),
            9..=10 => self.policy_step(rng, tick),
            11..=12 => {
                if !rng.chance(4) {
                    self.contact_frame(tick);
                }
                self.checks += 1;
            }
            _ => {
                if let Some(event) = self.recogniser.expire(tick) {
                    self.observe_gesture(event);
                }
                self.activity.record(tick);
                assert_eq!(self.activity.last(), Some(tick));
                assert_eq!(self.activity.idle_ticks(tick), 0);
                assert_eq!(
                    self.activity.idle_ticks(Tick::new(self.tick.saturating_sub(1))),
                    0,
                    "idle time never wraps to a huge value"
                );
                self.checks += 1;
            }
        }
    }

    fn press_key(&mut self, code: KeyCode, symbol: Keysym, modifiers: Modifiers, inhibit: ShortcutInhibit) {
        let routed = self.router.route(
            Event::Key {
                key_code: code,
                symbol,
                modifiers,
                state: KeyState::Pressed,
                repeat: false,
            },
            inhibit,
        );
        self.presses += 1;
        self.held.insert(code, routed.target);
        self.record_routed(routed);
        assert_eq!(
            self.router.held(),
            self.held.len() + self.held_buttons.len(),
            "the router holds exactly the routed presses"
        );
        assert_eq!(self.router.stats().presses, self.presses);
        assert_eq!(self.router.stats().orphan_releases, self.orphans);
    }

    fn release_key(&mut self, code: KeyCode, symbol: Keysym, modifiers: Modifiers, inhibit: ShortcutInhibit) {
        let routed = self.router.route(
            Event::Key {
                key_code: code,
                symbol,
                modifiers,
                state: KeyState::Released,
                repeat: false,
            },
            inhibit,
        );
        self.releases += 1;
        match self.held.remove(&code) {
            Some(expected) => {
                assert_eq!(
                    routed.target, expected,
                    "a release is routed to the same target as its press"
                );
                assert!(routed.followed_press);
            }
            None => {
                self.orphans += 1;
                assert!(
                    !routed.followed_press,
                    "an unpaired release cannot claim to follow a press"
                );
            }
        }
        self.record_routed(routed);
        assert_eq!(self.router.held(), self.held.len() + self.held_buttons.len());
        assert_eq!(self.router.stats().releases, self.releases);
        assert_eq!(self.router.stats().orphan_releases, self.orphans);
    }

    fn press_button(&mut self, button: u8, modifiers: Modifiers, inhibit: ShortcutInhibit) {
        let routed = self
            .router
            .route(button_event(button, modifiers, ButtonState::Pressed), inhibit);
        self.presses += 1;
        self.held_buttons.insert(button, routed.target);
        if routed.fired.is_some() {
            assert_ne!(routed.target, Target::Client, "a binding that fires owns the edge");
        }
        self.record_routed(routed);
        assert_eq!(self.router.stats().presses, self.presses);
        assert_eq!(self.router.held(), self.held.len() + self.held_buttons.len());
    }

    fn release_button(&mut self, button: u8, modifiers: Modifiers, inhibit: ShortcutInhibit) {
        let routed = self
            .router
            .route(button_event(button, modifiers, ButtonState::Released), inhibit);
        self.releases += 1;
        match self.held_buttons.remove(&button) {
            Some(expected) => {
                assert_eq!(routed.target, expected, "a button release follows its press");
                assert!(routed.followed_press);
            }
            None => {
                self.orphans += 1;
                assert!(!routed.followed_press);
            }
        }
        self.record_routed(routed);
        assert_eq!(self.router.stats().releases, self.releases);
        assert_eq!(self.router.stats().orphan_releases, self.orphans);
        assert_eq!(self.router.held(), self.held.len() + self.held_buttons.len());
    }

    fn grab_step(&mut self, rng: &mut Rng) {
        if self.grabs.is_empty() || rng.chance(2) {
            let scope = match rng.below(3) {
                0 => Scope::Keyboard,
                1 => Scope::Pointer,
                _ => Scope::Both,
            };
            self.grabs.push(self.router.begin_grab(scope));
        } else {
            let index = rng.below(self.grabs.len() as u64) as usize;
            let id = self.grabs.remove(index);
            assert!(self.router.end_grab(id));
        }
        assert_eq!(self.router.grabs().count(), self.grabs.len());
        self.checks += 1;
    }

    fn reload(&mut self) {
        let table = self.router.bindings_mut();
        table.remove_name(&name("plain_q"));
        table.remove_name(&name("shift_w"));
        // The same names come back, so their ids survive a reload and a press held across
        // it still names a live binding.
        table
            .insert(&name("plain_q"), chord("q"), Trigger::Press, true)
            .expect("the chord is free after the removal");
        table
            .insert(&name("shift_w"), chord("<Shift>w"), Trigger::Release, false)
            .expect("the chord is free after the removal");
        self.checks += 1;
    }

    fn policy_step(&mut self, rng: &mut Rng, tick: Tick) {
        match rng.below(4) {
            0 => {
                if let Submitted::Queued { id, deadline } = self.policy.submit(tick, 1 + rng.below(5)) {
                    assert!(deadline >= tick);
                    self.waiting.push(id.get());
                }
            }
            1 => {
                for expired in self.policy.expire(tick) {
                    let position = self
                        .waiting
                        .iter()
                        .position(|raw| *raw == expired.id.get())
                        .expect("an expired request was waiting");
                    assert_eq!(position, 0, "retirement starts with the oldest request");
                    self.waiting.remove(0);
                }
            }
            2 => {
                if let Some(raw) = self.waiting.first().copied() {
                    let id = RequestId::new(raw);
                    let reply = self.policy.reply(id, Decision::Consume);
                    assert_eq!(
                        reply,
                        Reply::Applied {
                            id,
                            decision: Decision::Consume
                        }
                    );
                    self.waiting.retain(|other| *other != raw);
                }
            }
            _ => {
                let retired = self.policy.retire_all();
                let order: Vec<u64> = retired.iter().map(|id| id.get()).collect();
                assert_eq!(order, self.waiting, "a reload retires in submission order");
                self.waiting.clear();
                // A retired request cannot be decided again.
                for raw in &order {
                    let id = RequestId::new(*raw);
                    assert_eq!(self.policy.reply(id, Decision::Forward), Reply::Late { id });
                }
            }
        }
        let stats = self.policy.stats();
        let settled = stats.applied + stats.expired + stats.retired;
        assert_eq!(
            stats.queued,
            settled + self.waiting.len() as u64,
            "every queued request is decided, expired or retired exactly once"
        );
        self.checks += 1;
    }

    fn contact_frame(&mut self, tick: Tick) {
        let frame: Vec<Contact> = match self.touch % 6 {
            0 => vec![
                contact(0, ContactPhase::Begin, 100.0, 100.0),
                contact(1, ContactPhase::Begin, 120.0, 100.0),
            ],
            1 => vec![
                contact(0, ContactPhase::Update, 100.0, 150.0),
                contact(1, ContactPhase::Update, 120.0, 150.0),
            ],
            2 => vec![
                contact(0, ContactPhase::Update, 60.0, 200.0),
                contact(1, ContactPhase::Update, 160.0, 200.0),
            ],
            3 => vec![
                contact(0, ContactPhase::End, 60.0, 200.0),
                contact(1, ContactPhase::End, 160.0, 200.0),
            ],
            4 => vec![contact(0, ContactPhase::Begin, 300.0, 300.0)],
            _ => vec![contact(0, ContactPhase::End, 300.0, 300.0)],
        };
        self.touch = self.touch.wrapping_add(1);
        if let Some(event) = self.recogniser.feed(tick, &frame) {
            self.observe_gesture(event);
        }
        if !self.recogniser.is_active() {
            assert_eq!(
                self.gestures.reserved(),
                None,
                "a finished gesture holds no reservation"
            );
        }
        self.checks += 1;
    }

    fn observe_gesture(&mut self, event: GestureEvent) {
        let route = self.gestures.observe(&event, GestureLock::Unlocked);
        let reserved = self.gestures.reserved();
        match (event.phase(), route) {
            (GesturePhase::Begin, GestureRoute::Reserved { binding }) => assert_eq!(reserved, Some(binding)),
            (GesturePhase::Begin, GestureRoute::Unclaimed) => assert_eq!(reserved, None),
            (GesturePhase::Update, GestureRoute::Reserved { binding }) => assert_eq!(reserved, Some(binding)),
            (GesturePhase::Update, GestureRoute::Unclaimed) => assert_eq!(reserved, None),
            (GesturePhase::End, GestureRoute::Fired { .. } | GestureRoute::MatchedNothing { .. }) => {
                assert_eq!(reserved, None, "the reservation ends with the gesture")
            }
            (GesturePhase::End, GestureRoute::Unclaimed) => assert_eq!(reserved, None),
            (GesturePhase::Cancel, GestureRoute::Cancelled { .. }) => assert_eq!(reserved, None),
            (phase, route) => panic!("unexpected {phase:?} -> {route:?}"),
        }
        // A fired binding is one of the declared ones and fits the gesture it fired for.
        if let GestureRoute::Fired { binding } = route {
            let binding = self.gestures.binding(binding).expect("a fired binding exists");
            assert_eq!(binding.kind(), event.kind());
            assert_eq!(binding.fingers(), event.fingers());
        }
        self.record(route);
    }

    fn record_routed(&mut self, routed: Routed) {
        let (kind, id) = match routed.target {
            Target::Grab(id) => (0, id.raw()),
            Target::Binding(id) => (1, u64::from(id.raw())),
            Target::Client => (2, 0),
        };
        self.digest.push(kind);
        self.digest.push(id);
        self.digest
            .push(u64::from(routed.fired.map_or(u32::MAX, |binding| binding.raw())));
        self.digest.push(u64::from(routed.followed_press));
        self.checks += 1;
    }

    fn record(&mut self, route: GestureRoute) {
        let (kind, id) = match route {
            GestureRoute::Unclaimed => (0, 0),
            GestureRoute::Reserved { binding } => (1, binding as u64),
            GestureRoute::Fired { binding } => (2, binding as u64),
            GestureRoute::MatchedNothing { binding } => (3, binding as u64),
            GestureRoute::Cancelled { binding } => (4, binding.map_or(u64::MAX, |binding| binding as u64)),
        };
        self.digest.push(kind);
        self.digest.push(id);
        self.checks += 1;
    }

    /// The invariants that hold once a sequence is over.
    fn finish(&mut self) {
        let stats = self.router.stats();
        assert_eq!(stats.presses, self.presses, "every press is routed exactly once");
        assert_eq!(stats.releases, self.releases, "every release is routed exactly once");
        assert_eq!(stats.orphan_releases, self.orphans);
        assert_eq!(
            stats.grab_deliveries + stats.binding_deliveries + stats.client_deliveries,
            stats.presses + stats.releases + stats.repeats,
            "each edge has exactly one delivery"
        );
        assert_eq!(self.router.held(), self.held.len() + self.held_buttons.len());
        if !self.recogniser.is_active() {
            assert_eq!(
                self.gestures.reserved(),
                None,
                "a gesture that is over holds no reservation"
            );
        }
        assert_eq!(
            self.policy.stats().queued,
            self.policy.stats().applied
                + self.policy.stats().expired
                + self.policy.stats().retired
                + self.waiting.len() as u64
        );
    }
}

fn contact(slot: u8, phase: ContactPhase, x: f64, y: f64) -> Contact {
    Contact { slot, phase, x, y }
}

/// Run one sequence and return its decision stream.
fn run(seed: u64) -> (Vec<u64>, usize) {
    let mut rng = Rng::new(seed);
    let mut model = Model::new();
    for _ in 0..STEPS_PER_SEQUENCE {
        model.step(&mut rng);
    }
    model.finish();
    (model.digest, model.checks)
}

#[test]
fn random_sequences_keep_every_invariant() {
    let mut checks = 0;
    for seed in 0..SEQUENCES {
        let (digest, sequence_checks) = run(seed);
        assert!(!digest.is_empty(), "seed {seed} exercised nothing");
        checks += sequence_checks;
    }
    assert!(checks >= MIN_CHECKS, "only {checks} invariant checks ran");
}

#[test]
fn the_same_sequence_decides_the_same_way_twice() {
    for seed in [1, 7, 99] {
        let (first, _) = run(seed);
        let (second, _) = run(seed);
        assert_eq!(first, second, "seed {seed} was not deterministic");
    }
}
