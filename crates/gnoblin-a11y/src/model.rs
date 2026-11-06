//! Model-based tests over random event sequences.
//!
//! A scenario test proves one transition. These check the invariants that hold
//! in *every* reachable state, over a hand-written deterministic generator: no
//! dependency, no clock and no seed from the environment, so a failure replays
//! from its seed alone.
//!
//! The invariants are the three the plan's card asks for:
//!
//! - no latched or locked modifier survives a session lock;
//! - every grab belongs to a live owner, and a decision never names an owner
//!   that has gone away;
//! - the magnifier's view always lies inside its output.

use std::collections::BTreeSet;

use crate::geometry::{Point, Rect, Scale};
use crate::ids::{KeyCode, Keysym, OutputId, OwnerId};
use crate::{
    A11y, BounceKeysConfig, Config, Effect, Event, Follow, Key, KeyDecision, Keystroke, MagnifierConfig, Mode,
    Modifiers, MouseKeysConfig, Output, SlowKeysConfig, StickyKeysConfig,
};

/// A 64-bit linear congruential generator: small, hand-written and identical on
/// every platform, so a failing seed replays exactly.
struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed)
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

/// The generator's own view of the world: the clock it advances and the layout
/// it last sent, so an invariant can be checked against what the machine was
/// told rather than against what it believes.
struct Model {
    rng: Rng,
    now_ms: u64,
    outputs: Vec<Output>,
}

impl Model {
    fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            now_ms: 0,
            outputs: Vec::new(),
        }
    }

    fn config(&mut self) -> Config {
        Config {
            sticky_keys: StickyKeysConfig {
                enabled: self.rng.below(3) != 0,
                two_key_off: self.rng.below(2) == 0,
                beep_on_modifier: self.rng.below(4) == 0,
            },
            slow_keys: SlowKeysConfig {
                enabled: self.rng.below(2) == 0,
                delay_ms: match self.rng.below(3) {
                    0 => 0,
                    1 => 120,
                    _ => 300,
                },
                beep_on_press: false,
                beep_on_accept: false,
                beep_on_reject: false,
            },
            bounce_keys: BounceKeysConfig {
                enabled: self.rng.below(2) == 0,
                delay_ms: match self.rng.below(3) {
                    0 => 0,
                    1 => 150,
                    _ => 300,
                },
                beep_on_reject: false,
            },
            mouse_keys: MouseKeysConfig {
                enabled: self.rng.below(2) == 0,
                max_speed: 1 + u32::try_from(self.rng.below(2000)).unwrap_or(1),
                acceleration_time_ms: 1 + self.rng.below(400),
                initial_delay_ms: self.rng.below(200),
            },
            magnifier: MagnifierConfig {
                enabled: self.rng.below(2) == 0,
                mode: if self.rng.below(2) == 0 {
                    Mode::Lens
                } else {
                    Mode::FullScreen
                },
                zoom: match self.rng.below(3) {
                    0 => Scale::integral(1),
                    1 => Scale::integral(2),
                    _ => Scale::integral(5),
                },
                follow: match self.rng.below(3) {
                    0 => Follow::Focus,
                    1 => Follow::Caret,
                    _ => Follow::Pointer,
                },
            },
        }
    }

    /// One event, at the frequency an adversarial sequence would produce:
    /// repeated keys, drops of owners that never connected, and layouts that
    /// change mid-chord are all common.
    fn next(&mut self) -> Event {
        match self.rng.below(16) {
            0 | 1 => {
                self.now_ms = self.now_ms.saturating_add(1 + self.rng.below(400));
                Event::Tick { now_ms: self.now_ms }
            }
            2 | 3 => Event::KeyPressed(self.key()),
            4 | 5 => Event::KeyReleased(self.key()),
            6 => Event::SessionLocked,
            7 => Event::SessionUnlocked,
            8 => Event::WatchKeyboard { owner: self.owner() },
            9 => Event::GrabKeyboard { owner: self.owner() },
            10 => {
                if self.rng.below(2) == 0 {
                    Event::UngrabKeyboard { owner: self.owner() }
                } else {
                    Event::UnwatchKeyboard { owner: self.owner() }
                }
            }
            11 => {
                if self.rng.below(2) == 0 {
                    Event::MonitorDropped { owner: self.owner() }
                } else {
                    Event::SetKeyGrabs {
                        owner: self.owner(),
                        modifiers: if self.rng.below(2) == 0 {
                            vec![Keysym::new(0xffe1)]
                        } else {
                            Vec::new()
                        },
                        keystrokes: if self.rng.below(2) == 0 {
                            vec![Keystroke {
                                keysym: Keysym::new(0x61),
                                modifiers: Modifiers::CONTROL,
                            }]
                        } else {
                            Vec::new()
                        },
                    }
                }
            }
            12 => Event::NumLockChanged {
                active: self.rng.below(2) == 0,
            },
            13 => {
                self.outputs = self.layout();
                Event::OutputsChanged {
                    outputs: self.outputs.clone(),
                }
            }
            14 => match self.rng.below(3) {
                0 => Event::PointerMoved { at: self.point() },
                1 => Event::FocusMoved { at: self.point() },
                _ => Event::CaretMoved { at: self.point() },
            },
            _ => Event::Configure(self.config()),
        }
    }

    fn key(&mut self) -> Key {
        let (keycode, keysym, modifier) = match self.rng.below(4) {
            0 => (50, 0xffe1, Some(Modifiers::SHIFT)),
            1 => (37, 0xffe3, Some(Modifiers::CONTROL)),
            2 => (38, 0x61, None),
            _ => (90, 0xffb6, None),
        };
        Key {
            keycode: KeyCode::new(keycode),
            keysym: Keysym::new(keysym),
            modifiers: Modifiers::NONE,
            unichar: 0,
            modifier,
        }
    }

    fn owner(&mut self) -> OwnerId {
        OwnerId::new(1 + self.rng.below(2))
    }

    fn point(&mut self) -> Point {
        match self.rng.below(5) {
            0 => Point::new(0, 0),
            1 => Point::new(1919, 1079),
            2 => Point::new(1919, 0),
            3 => Point::new(2100, 300),
            _ => Point::new(
                i32::try_from(self.rng.below(3840)).unwrap_or(0),
                i32::try_from(self.rng.below(1080)).unwrap_or(0),
            ),
        }
    }

    fn layout(&mut self) -> Vec<Output> {
        let left = Output {
            id: OutputId::new(1),
            bounds: Rect::new(0, 0, 1920, 1080),
        };
        let right = Output {
            id: OutputId::new(2),
            bounds: Rect::new(1920, 0, 1920, 1080),
        };
        match self.rng.below(4) {
            0 => Vec::new(),
            1 => vec![left],
            2 => vec![right],
            _ => vec![left, right],
        }
    }
}

/// The properties that must hold in every state a sequence reaches.
fn invariants(
    machine: &A11y,
    outputs: &[Output],
    effects: &[Effect],
    locked: bool,
    step: usize,
    seed: u64,
    checks: &mut usize,
) {
    if locked {
        *checks += 2;
        assert!(
            machine.sticky().latched.is_empty(),
            "seed {seed} step {step}: a latched modifier survived the lock"
        );
        assert!(
            machine.sticky().locked.is_empty(),
            "seed {seed} step {step}: a locked modifier survived the lock"
        );
    }

    *checks += 1;
    if let Some(view) = machine.view() {
        let Some(output) = outputs.iter().find(|output| output.id == view.output) else {
            panic!("seed {seed} step {step}: the view names an output that is not in the layout");
        };
        *checks += 1;
        assert!(
            view.source.is_inside(output.bounds),
            "seed {seed} step {step}: the view {}x{} at {},{} is outside {}x{} at {},{}",
            view.source.width,
            view.source.height,
            view.source.x,
            view.source.y,
            output.bounds.width,
            output.bounds.height,
            output.bounds.x,
            output.bounds.y
        );
    }

    let live: BTreeSet<OwnerId> = machine.monitors().collect();
    for effect in effects {
        let Effect::Key(routing) = effect else {
            continue;
        };
        *checks += 1;
        let consistent = match routing.decision {
            KeyDecision::PassThrough => routing.monitors.is_empty(),
            KeyDecision::Both | KeyDecision::Consumed => !routing.monitors.is_empty(),
        };
        assert!(
            consistent,
            "seed {seed} step {step}: {:?} with {} monitors",
            routing.decision,
            routing.monitors.len()
        );
        for owner in &routing.monitors {
            *checks += 1;
            assert!(
                live.contains(owner),
                "seed {seed} step {step}: {owner:?} receives a key but owns no watch or grab"
            );
        }
    }
}

#[test]
fn invariants_hold_over_random_sequences() {
    let mut checks = 0usize;
    for seed in 1..=200 {
        let mut model = Model::new(seed);
        let mut machine = A11y::with_config(model.config());
        for step in 0..60 {
            let event = model.next();
            let locked = matches!(event, Event::SessionLocked);
            let effects = machine.apply(event);
            invariants(&machine, &model.outputs, &effects, locked, step, seed, &mut checks);
        }
    }
    println!("gnoblin-a11y model invariants: {checks} checks");
    assert!(checks > 0);
}

#[test]
fn the_same_sequence_produces_the_same_effects() {
    for seed in 1..=200 {
        let mut left = Model::new(seed);
        let mut right = Model::new(seed);
        let mut machine = A11y::with_config(left.config());
        let mut twin = A11y::with_config(right.config());
        for step in 0..60 {
            let event = left.next();
            let same = right.next();
            assert_eq!(
                event, same,
                "seed {seed} step {step}: the generator is not deterministic"
            );
            assert_eq!(
                machine.apply(event),
                twin.apply(same),
                "seed {seed} step {step}: the machine is not deterministic"
            );
            assert_eq!(machine.published(), twin.published(), "seed {seed} step {step}");
        }
    }
}
