//! Model-based tests over random event sequences.
//!
//! A scenario test proves one transition. These check the invariants that hold
//! in *every* reachable state, over a hand-written deterministic generator: no
//! dependency, no clock and no seed from the environment, so a failure is
//! reproducible by seed alone.

use gnoblin_types::{ClientId, OutputId};

use crate::{Capture, CaptureStream, CoverFailure, Delivery, Effect, Event, InputSource, Lock, State, Subject};

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

    fn client(&mut self) -> ClientId {
        ClientId::new(1 + self.below(2))
    }

    fn output(&mut self) -> OutputId {
        OutputId::new(1 + self.below(2))
    }

    fn layout(&mut self) -> Vec<OutputId> {
        let first = OutputId::new(1);
        let second = OutputId::new(2);
        match self.below(3) {
            0 => Vec::new(),
            1 => vec![first],
            _ => vec![first, second],
        }
    }
}

/// Every event a compositor can send, at the frequency an adversarial sequence
/// would: repeats, stale generations and out-of-order requests are all common.
fn generate(rng: &mut Rng) -> Event {
    match rng.below(14) {
        0 => Event::LockRequested { locker: rng.client() },
        1 | 2 => Event::CoverPresented {
            generation: rng.below(4),
            output: rng.output(),
        },
        3 => Event::CoverFailed {
            generation: rng.below(4),
            reason: CoverFailure::RenderFailed,
        },
        4 => Event::OutputsChanged { outputs: rng.layout() },
        5 => Event::LockerSurfaceMapped {
            locker: rng.client(),
            output: rng.output(),
        },
        6 => Event::LockerSurfaceUnmapped {
            locker: rng.client(),
            output: rng.output(),
        },
        7 => Event::LockerGone { locker: rng.client() },
        8 => Event::LockAbandoned { locker: rng.client() },
        9 => Event::UnlockRequested { locker: rng.client() },
        10 => Event::RevealComplete,
        11 => Event::SubjectAdded {
            subject: Subject::Client(rng.client()),
        },
        12 => Event::SubjectRemoved {
            subject: Subject::Client(rng.client()),
        },
        _ => {
            if rng.below(2) == 0 {
                Event::ProtocolDisabled
            } else {
                Event::RuntimeRestarted
            }
        }
    }
}

/// The properties that must hold in every state a sequence reaches.
fn invariants(lock: &Lock, step: usize, seed: u64) {
    let state = lock.state();
    let owner = lock.locker();
    let probes = [
        Subject::Session,
        Subject::Client(ClientId::new(1)),
        Subject::Client(ClientId::new(2)),
    ];
    for subject in probes {
        for source in [InputSource::Local, InputSource::Remote] {
            let delivery = lock.gate_input(source, subject);
            if delivery == Delivery::Deliver {
                let authorised = state == State::Unlocked || owner == Some(ClientId::new(subject_client(subject)));
                assert!(
                    authorised,
                    "seed {seed} step {step}: {source:?} input reached {subject:?} in {state:?} with owner {owner:?}"
                );
            }
        }
        if lock.gate_clipboard(subject) == Delivery::Deliver {
            let authorised = state == State::Unlocked || owner == Some(ClientId::new(subject_client(subject)));
            assert!(
                authorised,
                "seed {seed} step {step}: clipboard reached {subject:?} in {state:?} with owner {owner:?}"
            );
        }
    }

    // Capture never returns desktop content while anything is covered. This is
    // the invariant the whole crate exists for: no reachable state leaks a
    // covered frame.
    if state != State::Unlocked {
        assert_eq!(
            lock.capture(CaptureStream::Requested),
            Capture::Refused,
            "seed {seed} step {step}: a capture request was answered in {state:?}"
        );
        assert_ne!(
            lock.capture(CaptureStream::Granted),
            Capture::Desktop,
            "seed {seed} step {step}: a covered desktop was captured in {state:?}"
        );
        assert!(lock.active());
    } else {
        assert_eq!(lock.capture(CaptureStream::Granted), Capture::Desktop);
        assert!(!lock.active());
    }

    // Being covered means being covered: the active lock always answers with
    // the full embargo.
    assert_eq!(lock.embargo().holds(), state != State::Unlocked);
}

/// Used only to compare a probe subject with the owner client.
const fn subject_client(subject: Subject) -> u64 {
    match subject {
        Subject::Session => 0,
        Subject::Client(client) => client.get(),
    }
}

#[test]
fn invariants_hold_over_random_sequences() {
    for seed in 1..=200 {
        let mut rng = Rng::new(seed);
        let mut lock = Lock::new();
        let mut announced: Option<ClientId> = None;
        for step in 0..60 {
            let event = generate(&mut rng);
            let presenting = matches!(&event, Event::CoverPresented { .. });
            let before = lock.state();
            let effects = lock.apply(event);
            invariants(&lock, step, seed);

            // A lock is only ever reported after a protected frame reached every
            // output, so the only event that can enter `Locked` is a
            // presentation.
            if before != State::Locked && lock.state() == State::Locked {
                assert!(
                    presenting,
                    "seed {seed} step {step}: entered Locked without a presentation"
                );
            }

            // `locked` reaches an owner at most once. Re-announcing it after a
            // hotplug would tell a locker that has already authenticated that it
            // is locked again.
            for effect in &effects {
                if let Effect::SendLocked { locker } = effect {
                    assert_ne!(
                        announced,
                        Some(*locker),
                        "seed {seed} step {step}: {locker:?} was told it is locked twice"
                    );
                    announced = Some(*locker);
                }
            }
            if lock.state() == State::Unlocked {
                announced = None;
            }
        }
    }
}

#[test]
fn the_same_sequence_produces_the_same_effects() {
    for seed in 1..=200 {
        let mut rng = Rng::new(seed);
        let mut first = Lock::new();
        let mut second = Lock::new();
        for _ in 0..60 {
            let event = generate(&mut rng);
            assert_eq!(first.apply(event.clone()), second.apply(event), "seed {seed}");
            assert_eq!(first.state(), second.state(), "seed {seed}");
        }
    }
}

#[test]
fn an_announced_lock_never_reaches_a_new_owner() {
    // A takeover after a crash must not inherit the previous owner's `locked`.
    let mut rng = Rng::new(9);
    for _ in 0..200 {
        let mut lock = Lock::new();
        lock.apply(Event::OutputsChanged {
            outputs: vec![OutputId::new(1)],
        });
        let first = rng.client();
        lock.apply(Event::LockRequested { locker: first });
        lock.apply(Event::CoverPresented {
            generation: 1,
            output: OutputId::new(1),
        });
        lock.apply(Event::LockerGone { locker: first });
        let second = rng.client();
        let effects = lock.apply(Event::LockRequested { locker: second });
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::SendLocked { .. })));
        assert_eq!(lock.state(), State::Locking);
    }
}
