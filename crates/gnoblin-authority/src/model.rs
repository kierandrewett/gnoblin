//! Model-based tests over random event sequences.
//!
//! The generator is hand-written, deterministic and dependency-free: a failing
//! seed replays exactly. Identifiers are drawn from three values each, so
//! collisions, replays and stale handles are common rather than rare, and the
//! tick only moves forward, as a monotonic clock does.

use std::collections::BTreeMap;

use gnoblin_types::{ClientId, Tick};

use crate::{
    Answer, AppId, Authority, Claimant, ContextRequest, Decision, Denial, Effect, Event, Gesture, GestureId,
    GestureKind, LaunchOrigin, PortalCapability, RequestId, RequesterId, SessionLeader, SurfaceId, TokenId,
};

/// A 64-bit linear congruential generator: small, hand-written and identical on
/// every platform.
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
        ClientId::new(1 + self.below(3))
    }

    fn leader(&mut self) -> SessionLeader {
        SessionLeader::new(if self.below(2) == 0 { 100 } else { 200 })
    }

    fn surface(&mut self) -> SurfaceId {
        SurfaceId::new(1 + self.below(3))
    }

    fn token(&mut self) -> TokenId {
        TokenId::new(u128::from(1 + self.below(3)))
    }

    fn gesture(&mut self) -> GestureId {
        GestureId::new(1 + self.below(3))
    }

    fn requester(&mut self) -> RequesterId {
        RequesterId::new(1 + self.below(2))
    }

    fn request(&mut self) -> RequestId {
        RequestId::new(1 + self.below(3))
    }

    fn capability(&mut self) -> PortalCapability {
        if self.below(2) == 0 {
            PortalCapability::Capture
        } else {
            PortalCapability::InputInjection
        }
    }

    fn request_kind(&mut self) -> ContextRequest {
        match self.below(4) {
            0 => ContextRequest::Move,
            1 => ContextRequest::Resize,
            2 => ContextRequest::SnapTarget,
            _ => ContextRequest::TextInput,
        }
    }
}

fn generate(rng: &mut Rng) -> Event {
    match rng.below(12) {
        0 => Event::Launch {
            origin: LaunchOrigin {
                leader: rng.leader(),
                app_id: Some(AppId::new("org.example.app")),
            },
            token: rng.token(),
            lifetime: rng.below(6),
        },
        1 | 2 => Event::Activate {
            claim: Claimant {
                client: rng.client(),
                leader: rng.leader(),
                surface: rng.surface(),
                app_id: Some(AppId::new("org.example.app")),
                token: if rng.below(4) == 0 { None } else { Some(rng.token()) },
            },
        },
        3 => Event::GestureBegan {
            gesture: Gesture {
                id: rng.gesture(),
                client: rng.client(),
                surface: rng.surface(),
                kind: GestureKind::Pointer { button: 1 },
            },
            lifetime: rng.below(6),
        },
        4 => Event::GestureEnded { gesture: rng.gesture() },
        5 | 6 => Event::ContextRequest {
            gesture: rng.gesture(),
            client: rng.client(),
            surface: rng.surface(),
            request: rng.request_kind(),
        },
        7 => {
            if rng.below(3) == 0 {
                Event::PortalUiClaimed { client: rng.client() }
            } else {
                Event::PortalUiReleased { client: rng.client() }
            }
        }
        8 | 9 => Event::ConsentRequested {
            request: rng.request(),
            requester: rng.requester(),
            capability: rng.capability(),
            lifetime: rng.below(6),
        },
        10 => Event::ConsentAnswered {
            request: rng.request(),
            from: rng.client(),
            answer: if rng.below(2) == 0 { Answer::Allow } else { Answer::Deny },
        },
        _ => match rng.below(3) {
            0 => Event::RequesterGone {
                requester: rng.requester(),
            },
            1 => Event::GrantUsed {
                requester: rng.requester(),
                capability: rng.capability(),
            },
            _ => Event::LockChanged {
                active: rng.below(2) == 0,
            },
        },
    }
}

/// The token an activation event offered, if it offered one.
fn offered_token(event: &Event) -> Option<TokenId> {
    match event {
        Event::Activate { claim } => claim.token,
        _ => None,
    }
}

#[test]
fn invariants_hold_over_random_sequences() {
    for seed in 1..=200 {
        let mut rng = Rng::new(seed);
        let mut authority = Authority::new();
        let mut now = Tick::new(1_000);
        let mut activations: BTreeMap<TokenId, u32> = BTreeMap::new();

        for step in 0..80 {
            // A monotonic clock never runs backwards.
            now = now.saturating_add(rng.below(3));
            let event = generate(&mut rng);
            let offered = offered_token(&event);
            let effects = authority.apply(now, event);

            for effect in &effects {
                match effect {
                    Effect::Activate { .. } => {
                        assert!(!authority.locked(), "seed {seed} step {step}: activated while covered");
                        if let Some(token) = offered {
                            let count = activations.entry(token).or_default();
                            *count += 1;
                            assert_eq!(
                                *count,
                                1,
                                "seed {seed} step {step}: token {} activated {count} times",
                                token.get()
                            );
                        }
                    }
                    Effect::Authorise { .. } => {
                        assert!(
                            !authority.locked(),
                            "seed {seed} step {step}: authorised a context while covered"
                        );
                    }
                    Effect::Prompt { provider, .. } => {
                        assert_eq!(
                            Some(*provider),
                            authority.provider(),
                            "seed {seed} step {step}: a prompt was offered with no provider"
                        );
                    }
                    _ => {}
                }
            }

            for requester in [RequesterId::new(1), RequesterId::new(2)] {
                for capability in [PortalCapability::Capture, PortalCapability::InputInjection] {
                    let decision = authority.gate(now, requester, capability);
                    if authority.locked() {
                        assert_eq!(
                            decision,
                            Decision::Denied(Denial::Locked),
                            "seed {seed} step {step}: a grant survived the lock"
                        );
                    }
                    assert!(
                        decision.is_allowed() || matches!(decision, Decision::Denied(_)),
                        "seed {seed} step {step}: a gate answered with nothing"
                    );
                }
            }
        }
    }
}

#[test]
fn the_same_sequence_produces_the_same_effects() {
    for seed in 1..=200 {
        let mut rng = Rng::new(seed);
        let mut first = Authority::new();
        let mut second = Authority::new();
        let mut now = Tick::new(1_000);
        for _ in 0..80 {
            now = now.saturating_add(rng.below(3));
            let event = generate(&mut rng);
            assert_eq!(first.apply(now, event.clone()), second.apply(now, event), "seed {seed}");
        }
    }
}

#[test]
fn a_token_never_activates_twice_under_replay() {
    // A single token, hammered: every later redemption must be refused as
    // reused, and exactly one activation may exist.
    let mut rng = Rng::new(41);
    for _ in 0..200 {
        let mut authority = Authority::new();
        let token = rng.token();
        authority.apply(
            Tick::new(0),
            Event::Launch {
                origin: LaunchOrigin {
                    leader: SessionLeader::new(100),
                    app_id: None,
                },
                token,
                lifetime: 100,
            },
        );
        let mut allowed = 0;
        for _ in 0..10 {
            let effects = authority.apply(
                Tick::new(1),
                Event::Activate {
                    claim: Claimant {
                        client: rng.client(),
                        leader: SessionLeader::new(100),
                        surface: rng.surface(),
                        app_id: None,
                        token: Some(token),
                    },
                },
            );
            allowed += effects
                .iter()
                .filter(|effect| matches!(effect, Effect::Activate { .. }))
                .count();
        }
        assert_eq!(allowed, 1);
    }
}
