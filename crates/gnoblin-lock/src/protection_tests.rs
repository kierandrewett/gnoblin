//! One test per failure mode in [`crate::protection`]'s documentation.
//!
//! Every test drives [`Protection::apply`] and its queries only: no private
//! field, no helper and no clock, so a test here fails when the compositor could
//! show, announce or release a hosted session it must not.

use gnoblin_types::{ClientId, OutputId};

use crate::protection::{Protection, ProtectionEffect, ProtectionEvent, ProtectionFailure, ProtectionState};

const SESSION: ClientId = ClientId::new(1);
const OTHER: ClientId = ClientId::new(2);
const OUTPUT: OutputId = OutputId::new(1);
const SECOND: OutputId = OutputId::new(2);

/// A protection machine that knows about one output.
fn ready() -> Protection {
    let mut protection = Protection::new();
    assert_eq!(
        protection.apply(ProtectionEvent::OutputsChanged { outputs: vec![OUTPUT] }),
        Vec::new()
    );
    protection
}

/// The effects asking for [`SESSION`] to be protected on one output.
fn ask() -> Vec<ProtectionEffect> {
    vec![
        ProtectionEffect::Hide {
            client: SESSION,
            generation: 1,
        },
        ProtectionEffect::Ask {
            client: SESSION,
            serial: 1,
        },
        ProtectionEffect::AwaitPresentation {
            generation: 1,
            outputs: vec![OUTPUT],
        },
    ]
}

/// A protection that was asked for and has presented its frame, but which the
/// session has not acknowledged yet.
fn covering() -> Protection {
    let mut protection = ready();
    assert_eq!(protection.apply(ProtectionEvent::Protect { client: SESSION }), ask());
    assert_eq!(
        protection.apply(ProtectionEvent::Presented {
            generation: 1,
            output: OUTPUT
        }),
        Vec::new()
    );
    protection
}

/// A protection that the session acknowledged as well as presented.
fn protected() -> Protection {
    let mut protection = covering();
    assert_eq!(
        protection.apply(ProtectionEvent::Acknowledged {
            client: SESSION,
            serial: 1
        }),
        vec![ProtectionEffect::Protected {
            client: SESSION,
            serial: 1
        }]
    );
    assert_eq!(protection.state(SESSION), ProtectionState::Covered);
    protection
}

#[test]
fn nothing_is_protected_before_a_frame_covers_every_output() {
    let mut protection = covering();
    assert!(
        !protection.is_hidden(OTHER),
        "another session is never touched by this one's cover"
    );
    assert_eq!(
        protection.state(SESSION),
        ProtectionState::Covering,
        "a hidden session is not a protected one"
    );
    assert!(
        protection.protected().next().is_none(),
        "nothing is published before a frame proved it"
    );
    // A second output is added while the cover is being proven: the layout now
    // owes a frame on both.
    assert_eq!(
        protection.apply(ProtectionEvent::OutputsChanged {
            outputs: vec![OUTPUT, SECOND]
        }),
        Vec::new()
    );
    assert_eq!(
        protection.apply(ProtectionEvent::Acknowledged {
            client: SESSION,
            serial: 1
        }),
        Vec::new(),
        "the new output has not presented the cover"
    );
    assert_eq!(
        protection.apply(ProtectionEvent::Presented {
            generation: 1,
            output: SECOND
        }),
        vec![ProtectionEffect::Protected {
            client: SESSION,
            serial: 1
        }],
        "every output presented, so the protection lands"
    );
}

#[test]
fn an_acknowledgement_alone_does_not_protect() {
    let mut protection = ready();
    assert_eq!(protection.apply(ProtectionEvent::Protect { client: SESSION }), ask());
    assert_eq!(
        protection.apply(ProtectionEvent::Acknowledged {
            client: SESSION,
            serial: 1
        }),
        Vec::new(),
        "the session's own cover says nothing about the host's pixels"
    );
    assert_eq!(protection.state(SESSION), ProtectionState::Covering);
}

#[test]
fn no_output_can_prove_a_cover() {
    let mut protection = Protection::new();
    assert_eq!(
        protection.apply(ProtectionEvent::Protect { client: SESSION }),
        vec![
            ProtectionEffect::Hide {
                client: SESSION,
                generation: 1
            },
            ProtectionEffect::Ask {
                client: SESSION,
                serial: 1
            },
            ProtectionEffect::KeepCovered {
                client: SESSION,
                reason: ProtectionFailure::NoOutputs
            },
        ]
    );
    // Even a session that covers itself and a stray presentation cannot make a
    // protection out of nothing.
    assert!(
        protection
            .apply(ProtectionEvent::Presented {
                generation: 1,
                output: OUTPUT
            })
            .is_empty()
    );
    assert!(
        protection
            .apply(ProtectionEvent::Acknowledged {
                client: SESSION,
                serial: 1
            })
            .is_empty()
    );
    assert_eq!(protection.state(SESSION), ProtectionState::Covering);
}

#[test]
fn a_crashed_session_keeps_the_cover() {
    for (name, mut protection) in [("protected", protected()), ("covering", covering())] {
        assert_eq!(
            protection.apply(ProtectionEvent::SessionGone { client: SESSION }),
            vec![ProtectionEffect::KeepCovered {
                client: SESSION,
                reason: ProtectionFailure::SessionDied
            }],
            "{name}: a crash keeps the cover"
        );
        assert!(
            protection.is_hidden(SESSION),
            "{name}: the crashed session is not shown again"
        );
        assert!(
            protection
                .apply(ProtectionEvent::Release { client: SESSION })
                .is_empty(),
            "{name}: releasing a session that is gone reveals nothing"
        );
        assert!(
            protection
                .apply(ProtectionEvent::Presented {
                    generation: protection.generation(),
                    output: OUTPUT
                })
                .is_empty(),
            "{name}: a late frame does not announce a dead session"
        );
    }
}

#[test]
fn an_acknowledgement_of_an_unknown_serial_is_reported() {
    let mut protection = covering();
    let future = protection.generation() + 1;
    assert_eq!(
        protection.apply(ProtectionEvent::Acknowledged {
            client: SESSION,
            serial: future
        }),
        vec![ProtectionEffect::UnknownAck {
            client: SESSION,
            serial: future
        }],
        "a serial the host never sent is a protocol error"
    );
    // A session that was never asked at all is refused the same way.
    let mut other = ready();
    assert_eq!(
        other.apply(ProtectionEvent::Acknowledged {
            client: SESSION,
            serial: 1
        }),
        vec![ProtectionEffect::UnknownAck {
            client: SESSION,
            serial: 1
        }]
    );
}

#[test]
fn a_stale_acknowledgement_changes_nothing() {
    let mut protection = protected();
    assert!(
        protection
            .apply(ProtectionEvent::Acknowledged {
                client: SESSION,
                serial: 0
            })
            .is_empty(),
        "an older protection proves nothing about the current one"
    );
    assert_eq!(protection.state(SESSION), ProtectionState::Covered);
}

#[test]
fn a_new_output_does_not_owe_a_frame() {
    let mut protection = protected();
    assert_eq!(
        protection.apply(ProtectionEvent::OutputsChanged {
            outputs: vec![OUTPUT, SECOND]
        }),
        Vec::new(),
        "the new output never showed the hidden session, so it owes nothing"
    );
    assert_eq!(protection.state(SESSION), ProtectionState::Covered);
    assert!(protection.is_hidden(SESSION));
}

#[test]
fn a_removed_output_stops_being_owed() {
    let mut protection = ready();
    assert_eq!(
        protection.apply(ProtectionEvent::OutputsChanged {
            outputs: vec![OUTPUT, SECOND]
        }),
        Vec::new()
    );
    // Asked for on both outputs, one of which goes away before it presents.
    assert_eq!(
        protection.apply(ProtectionEvent::Protect { client: SESSION }),
        vec![
            ProtectionEffect::Hide {
                client: SESSION,
                generation: 1
            },
            ProtectionEffect::Ask {
                client: SESSION,
                serial: 1
            },
            ProtectionEffect::AwaitPresentation {
                generation: 1,
                outputs: vec![OUTPUT, SECOND]
            },
        ]
    );
    assert_eq!(
        protection.apply(ProtectionEvent::Presented {
            generation: 1,
            output: OUTPUT
        }),
        Vec::new()
    );
    assert_eq!(
        protection.apply(ProtectionEvent::OutputsChanged { outputs: vec![OUTPUT] }),
        Vec::new(),
        "the removed output stops being owed"
    );
    assert_eq!(
        protection.apply(ProtectionEvent::Acknowledged {
            client: SESSION,
            serial: 1
        }),
        vec![ProtectionEffect::Protected {
            client: SESSION,
            serial: 1
        }]
    );
}

#[test]
fn only_the_named_session_is_hidden() {
    let mut protection = protected();
    assert!(protection.is_hidden(SESSION));
    assert_eq!(protection.state(OTHER), ProtectionState::Visible);
    assert_eq!(protection.hidden().collect::<Vec<_>>(), vec![SESSION]);
    assert_eq!(protection.protected().collect::<Vec<_>>(), vec![SESSION]);

    // Releasing one session shows exactly that one.
    assert_eq!(
        protection.apply(ProtectionEvent::Release { client: SESSION }),
        vec![ProtectionEffect::Show { client: SESSION }]
    );
    assert_eq!(protection.state(SESSION), ProtectionState::Visible);
    assert!(protection.protected().next().is_none());
    assert!(
        protection
            .apply(ProtectionEvent::Release { client: SESSION })
            .is_empty(),
        "releasing twice shows nothing twice"
    );
}

#[test]
fn a_failed_frame_announces_nothing() {
    let mut protection = ready();
    assert_eq!(protection.apply(ProtectionEvent::Protect { client: SESSION }), ask());
    assert_eq!(
        protection.apply(ProtectionEvent::CoverFailed {
            generation: 1,
            reason: ProtectionFailure::RenderFailed
        }),
        vec![ProtectionEffect::KeepCovered {
            client: SESSION,
            reason: ProtectionFailure::RenderFailed
        }]
    );
    assert_eq!(
        protection.apply(ProtectionEvent::Acknowledged {
            client: SESSION,
            serial: 1
        }),
        Vec::new(),
        "a frame that could not be drawn does not prove the cover"
    );
    assert_eq!(protection.state(SESSION), ProtectionState::Covering);
    assert!(protection.is_hidden(SESSION));
    // The output that failed presents a later frame, which does prove it.
    assert_eq!(
        protection.apply(ProtectionEvent::Presented {
            generation: 1,
            output: OUTPUT
        }),
        vec![ProtectionEffect::Protected {
            client: SESSION,
            serial: 1
        }]
    );
}

#[test]
fn a_protection_is_announced_once() {
    let mut protection = protected();
    assert!(
        protection
            .apply(ProtectionEvent::Acknowledged {
                client: SESSION,
                serial: protection.generation()
            })
            .is_empty(),
        "the same acknowledgement twice announces once"
    );
    assert_eq!(
        protection.apply(ProtectionEvent::Protect { client: SESSION }),
        vec![ProtectionEffect::Protected {
            client: SESSION,
            serial: protection.generation()
        }],
        "a manager that asks again for a protection it holds hears the state, not silence"
    );
    assert!(
        protection
            .apply(ProtectionEvent::Presented {
                generation: protection.generation(),
                output: OUTPUT
            })
            .is_empty(),
        "a frame after the announcement changes nothing"
    );
}
