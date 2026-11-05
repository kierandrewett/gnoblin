//! One test per failure mode in the crate documentation.
//!
//! Every test drives the machine through [`Lock::apply`] and its queries only:
//! no private field, no helper and no clock. A test here fails when the
//! compositor could show, capture or feed something it must not, which is the
//! property each failure-mode entry names.

use gnoblin_types::{ClientId, OutputId};

use crate::{
    Capture, CaptureStream, CoverFailure, Delivery, Effect, Embargo, Event, InputSource, Lock, Published, RejectReason,
    State, Subject,
};

const LOCKER: ClientId = ClientId::new(1);
const OTHER: ClientId = ClientId::new(2);
const OUTPUT: OutputId = OutputId::new(1);
const SECOND: OutputId = OutputId::new(2);
const SESSION: Subject = Subject::Session;
const OWNER_SUBJECT: Subject = Subject::Client(LOCKER);

/// A lock that knows about one output, which is what a live compositor reports
/// before any request arrives.
fn ready() -> Lock {
    let mut lock = Lock::new();
    assert_eq!(lock.apply(Event::OutputsChanged { outputs: vec![OUTPUT] }), Vec::new());
    lock
}

/// A lock established by [`LOCKER`] on one output, with its surface mapped.
fn locked() -> Lock {
    let mut lock = ready();
    assert_eq!(
        lock.apply(Event::LockRequested { locker: LOCKER }),
        vec![
            Effect::Cover {
                generation: 1,
                subjects: vec![SESSION],
            },
            Effect::AwaitPresentation {
                generation: 1,
                outputs: vec![OUTPUT],
            },
            Effect::Embargo(Embargo::HELD),
            Effect::Publish(Published {
                state: State::Locking,
                active_changed: true,
            }),
        ]
    );
    assert_eq!(
        lock.apply(Event::LockerSurfaceMapped {
            locker: LOCKER,
            output: OUTPUT,
        }),
        Vec::new()
    );
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 1,
            output: OUTPUT,
        }),
        vec![
            Effect::SendLocked { locker: LOCKER },
            Effect::Publish(Published {
                state: State::Locked,
                active_changed: false,
            }),
        ]
    );
    assert_eq!(lock.state(), State::Locked);
    lock
}

#[test]
fn cover_failure_keeps_the_cover() {
    let mut lock = locked();
    assert_eq!(
        lock.apply(Event::CoverFailed {
            generation: 1,
            reason: CoverFailure::RenderFailed,
        }),
        vec![
            Effect::KeepCovered {
                reason: CoverFailure::RenderFailed,
            },
            Effect::Publish(Published {
                state: State::CoverFailed,
                active_changed: false,
            }),
        ]
    );
    assert_eq!(lock.state(), State::CoverFailed);
    assert!(lock.active());
    assert_eq!(lock.capture(CaptureStream::Requested), Capture::Refused);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Refused);
    assert_eq!(lock.gate_input(InputSource::Local, SESSION), Delivery::Discard);
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Discard);
}

#[test]
fn locker_crash_while_locking_keeps_the_cover() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    assert_eq!(
        lock.apply(Event::LockerGone { locker: LOCKER }),
        vec![
            Effect::KeepCovered {
                reason: CoverFailure::LockerDied,
            },
            Effect::Publish(Published {
                state: State::CoverFailed,
                active_changed: false,
            }),
        ]
    );
    assert_eq!(lock.state(), State::CoverFailed);
    assert_eq!(lock.locker(), None);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Refused);
    // The cover outlives the client: the desktop is never revealed by a crash.
    assert!(lock.is_covered(SESSION));
}

#[test]
fn locker_crash_while_locked_keeps_the_cover() {
    let mut lock = locked();
    assert_eq!(
        lock.apply(Event::LockerGone { locker: LOCKER }),
        vec![
            Effect::KeepCovered {
                reason: CoverFailure::LockerDied,
            },
            Effect::Publish(Published {
                state: State::CoverFailed,
                active_changed: false,
            }),
        ]
    );
    assert_eq!(lock.state(), State::CoverFailed);
    assert_eq!(lock.locker(), None);
    assert_eq!(lock.gate_input(InputSource::Local, SESSION), Delivery::Discard);
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Discard);
    assert_eq!(lock.embargo(), Embargo::HELD);
}

#[test]
fn abandoning_before_locked_is_not_an_unlock() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    let effects = lock.apply(Event::LockAbandoned { locker: LOCKER });
    assert_eq!(
        effects,
        vec![
            Effect::KeepCovered {
                reason: CoverFailure::LockerAbandoned,
            },
            Effect::Publish(Published {
                state: State::CoverFailed,
                active_changed: false,
            }),
        ]
    );
    assert!(!effects.contains(&Effect::Reveal));
    assert_eq!(lock.state(), State::CoverFailed);
    assert!(lock.active());
}

#[test]
fn stale_presentation_never_proves_the_cover() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    // A frame queued before the cover was raised carries the old generation.
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 0,
            output: OUTPUT,
        }),
        Vec::new()
    );
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 7,
            output: OUTPUT,
        }),
        Vec::new()
    );
    assert_eq!(lock.state(), State::Locking);
    assert_eq!(lock.gate_input(InputSource::Remote, OWNER_SUBJECT), Delivery::Discard);
    // The armed generation is the only one that proves anything.
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 1,
            output: OUTPUT,
        }),
        vec![
            Effect::SendLocked { locker: LOCKER },
            Effect::Publish(Published {
                state: State::Locked,
                active_changed: false,
            }),
        ]
    );
}

#[test]
fn hotplug_while_locked_withholds_input_until_reproven() {
    let mut lock = locked();
    assert_eq!(
        lock.apply(Event::OutputsChanged {
            outputs: vec![OUTPUT, SECOND],
        }),
        vec![
            Effect::Cover {
                generation: 2,
                subjects: vec![SESSION],
            },
            Effect::AwaitPresentation {
                generation: 2,
                outputs: vec![OUTPUT, SECOND],
            },
            Effect::ConfigureLockSurfaces {
                locker: LOCKER,
                outputs: vec![OUTPUT, SECOND],
            },
            Effect::Publish(Published {
                state: State::Locking,
                active_changed: false,
            }),
        ]
    );
    // The new output is not proven, so no remote input and no capture yet.
    assert_eq!(lock.gate_input(InputSource::Remote, OWNER_SUBJECT), Delivery::Discard);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Refused);
    // One output proving the cover is not enough for the new layout.
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 2,
            output: OUTPUT,
        }),
        Vec::new()
    );
    assert_eq!(lock.state(), State::Locking);
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 2,
            output: SECOND,
        }),
        vec![Effect::Publish(Published {
            state: State::Locked,
            active_changed: false,
        })]
    );
    assert_eq!(lock.gate_input(InputSource::Remote, OWNER_SUBJECT), Delivery::Deliver);
}

#[test]
fn removing_every_output_never_reaches_locked() {
    let mut lock = locked();
    assert_eq!(
        lock.apply(Event::OutputsChanged { outputs: Vec::new() }),
        vec![
            Effect::Cover {
                generation: 2,
                subjects: vec![SESSION],
            },
            Effect::KeepCovered {
                reason: CoverFailure::NoOutputs,
            },
            Effect::Publish(Published {
                state: State::CoverFailed,
                active_changed: false,
            }),
        ]
    );
    assert_eq!(lock.state(), State::CoverFailed);
    // A frame for an output that no longer exists proves nothing.
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 2,
            output: OUTPUT,
        }),
        Vec::new()
    );
    assert_eq!(lock.state(), State::CoverFailed);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Refused);
}

#[test]
fn runtime_restart_changes_nothing() {
    let mut lock = locked();
    assert_eq!(
        lock.apply(Event::RuntimeRestarted),
        vec![Effect::Publish(Published {
            state: State::Locked,
            active_changed: false,
        })]
    );
    assert_eq!(lock.state(), State::Locked);
    assert_eq!(lock.locker(), Some(LOCKER));
    assert_eq!(lock.embargo(), Embargo::HELD);
    assert_eq!(lock.gate_input(InputSource::Local, SESSION), Delivery::Discard);
}

#[test]
fn disabling_the_protocol_keeps_the_cover() {
    let mut lock = locked();
    let effects = lock.apply(Event::ProtocolDisabled);
    assert_eq!(
        effects,
        vec![Effect::KeepCovered {
            reason: CoverFailure::ProtocolDisabled,
        }]
    );
    assert!(!effects.contains(&Effect::Reveal));
    assert!(lock.active());
    assert_eq!(lock.state(), State::Locked);
    assert_eq!(lock.gate_clipboard(SESSION), Delivery::Discard);
}

#[test]
fn capture_during_locking_is_refused() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    assert_eq!(lock.state(), State::Locking);
    assert_eq!(lock.capture(CaptureStream::Requested), Capture::Refused);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Refused);
}

#[test]
fn granted_stream_sees_only_the_lock_scene() {
    let mut lock = locked();
    assert_eq!(lock.capture(CaptureStream::Requested), Capture::Refused);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::LockScene);
    lock.apply(Event::UnlockRequested { locker: LOCKER });
    assert_eq!(lock.state(), State::Unlocking);
    // The desktop is coming back but is not back: no frames either way.
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Refused);
    lock.apply(Event::RevealComplete);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Desktop);
}

#[test]
fn input_for_a_covered_client_is_consumed() {
    let lock = locked();
    // Keyboard, pointer, touch, tablet and input-method traffic all take this
    // decision, so a subtree, popup or preedit of a covered client cannot
    // appear either: the event never reaches the client that would map it.
    assert_eq!(lock.gate_input(InputSource::Local, SESSION), Delivery::Discard);
    assert_eq!(lock.gate_input(InputSource::Remote, SESSION), Delivery::Discard);
    assert_eq!(
        lock.gate_input(InputSource::Local, Subject::Client(OTHER)),
        Delivery::Discard
    );
    // The owner's own UI still works, otherwise the locker cannot be used.
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Deliver);
}

#[test]
fn clipboard_belongs_to_the_owner_only() {
    let mut lock = locked();
    assert_eq!(lock.gate_clipboard(SESSION), Delivery::Discard);
    assert_eq!(lock.gate_clipboard(Subject::Client(OTHER)), Delivery::Discard);
    assert_eq!(lock.gate_clipboard(OWNER_SUBJECT), Delivery::Deliver);
    lock.apply(Event::LockerGone { locker: LOCKER });
    // A cover with no owner has nobody who may use the selection.
    assert_eq!(lock.gate_clipboard(OWNER_SUBJECT), Delivery::Discard);
}

#[test]
fn remote_input_waits_for_the_presented_scene() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    lock.apply(Event::LockerSurfaceMapped {
        locker: LOCKER,
        output: OUTPUT,
    });
    assert_eq!(lock.gate_input(InputSource::Remote, OWNER_SUBJECT), Delivery::Discard);
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Deliver);
    lock.apply(Event::CoverPresented {
        generation: 1,
        output: OUTPUT,
    });
    assert_eq!(lock.gate_input(InputSource::Remote, OWNER_SUBJECT), Delivery::Deliver);
    lock.apply(Event::CoverFailed {
        generation: 1,
        reason: CoverFailure::RenderFailed,
    });
    assert_eq!(lock.state(), State::CoverFailed);
    assert_eq!(lock.gate_input(InputSource::Remote, OWNER_SUBJECT), Delivery::Discard);
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Discard);
}

#[test]
fn only_the_owner_unlocks() {
    let mut lock = locked();
    assert_eq!(
        lock.apply(Event::UnlockRequested { locker: OTHER }),
        vec![Effect::Reject {
            locker: OTHER,
            reason: RejectReason::NotOwner,
        }]
    );
    assert_eq!(lock.state(), State::Locked);
    assert_eq!(lock.locker(), Some(LOCKER));
}

#[test]
fn unlock_before_locked_is_refused() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    assert_eq!(
        lock.apply(Event::UnlockRequested { locker: LOCKER }),
        vec![Effect::Reject {
            locker: LOCKER,
            reason: RejectReason::NotLocked,
        }]
    );
    assert_eq!(lock.state(), State::Locking);
    assert!(lock.active());
}

#[test]
fn a_stray_reveal_complete_unlocks_nothing() {
    let mut lock = locked();
    assert_eq!(lock.apply(Event::RevealComplete), Vec::new());
    assert_eq!(lock.state(), State::Locked);
    assert_eq!(lock.embargo(), Embargo::HELD);
}

#[test]
fn concurrent_requests_are_finished() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    assert_eq!(
        lock.apply(Event::LockRequested { locker: OTHER }),
        vec![Effect::Reject {
            locker: OTHER,
            reason: RejectReason::AlreadyLocked,
        }]
    );
    assert_eq!(lock.locker(), Some(LOCKER));
    lock.apply(Event::CoverPresented {
        generation: 1,
        output: OUTPUT,
    });
    assert_eq!(
        lock.apply(Event::LockRequested { locker: OTHER }),
        vec![Effect::Reject {
            locker: OTHER,
            reason: RejectReason::AlreadyLocked,
        }]
    );
    // A refusal never arms a second cover.
    assert_eq!(lock.state(), State::Locked);
}

#[test]
fn locking_without_outputs_fails_closed() {
    let mut lock = Lock::new();
    assert_eq!(
        lock.apply(Event::LockRequested { locker: LOCKER }),
        vec![
            Effect::KeepCovered {
                reason: CoverFailure::NoOutputs,
            },
            Effect::Reject {
                locker: LOCKER,
                reason: RejectReason::CoverUnavailable,
            },
            Effect::Embargo(Embargo::HELD),
            Effect::Publish(Published {
                state: State::CoverFailed,
                active_changed: true,
            }),
        ]
    );
    assert_eq!(lock.state(), State::CoverFailed);
    assert_eq!(lock.locker(), None);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Refused);
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Discard);
}

#[test]
fn takeover_requires_a_fresh_cover() {
    let mut lock = locked();
    lock.apply(Event::LockerGone { locker: LOCKER });
    assert_eq!(
        lock.apply(Event::LockRequested { locker: OTHER }),
        vec![
            Effect::Cover {
                generation: 2,
                subjects: vec![SESSION],
            },
            Effect::AwaitPresentation {
                generation: 2,
                outputs: vec![OUTPUT],
            },
            Effect::Publish(Published {
                state: State::Locking,
                active_changed: false,
            }),
        ]
    );
    // The replacement may not inherit the old owner's proof.
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 1,
            output: OUTPUT,
        }),
        Vec::new()
    );
    assert_eq!(lock.state(), State::Locking);
    assert_eq!(
        lock.apply(Event::CoverPresented {
            generation: 2,
            output: OUTPUT,
        }),
        vec![
            Effect::SendLocked { locker: OTHER },
            Effect::Publish(Published {
                state: State::Locked,
                active_changed: false,
            }),
        ]
    );
}

#[test]
fn locked_is_announced_once_across_hotplug() {
    let mut lock = locked();
    lock.apply(Event::OutputsChanged {
        outputs: vec![OUTPUT, SECOND],
    });
    lock.apply(Event::CoverPresented {
        generation: 2,
        output: OUTPUT,
    });
    let effects = lock.apply(Event::CoverPresented {
        generation: 2,
        output: SECOND,
    });
    assert_eq!(
        effects,
        vec![Effect::Publish(Published {
            state: State::Locked,
            active_changed: false,
        })]
    );
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::SendLocked { .. })));
}

#[test]
fn a_new_subject_is_covered_before_it_is_shown() {
    let mut lock = locked();
    assert_eq!(
        lock.apply(Event::SubjectAdded {
            subject: Subject::Client(OTHER),
        }),
        vec![
            Effect::Cover {
                generation: 2,
                subjects: vec![SESSION, Subject::Client(OTHER)],
            },
            Effect::AwaitPresentation {
                generation: 2,
                outputs: vec![OUTPUT],
            },
            Effect::Publish(Published {
                state: State::Locking,
                active_changed: false,
            }),
        ]
    );
    assert!(lock.is_covered(Subject::Client(OTHER)));
    assert_eq!(
        lock.gate_input(InputSource::Local, Subject::Client(OTHER)),
        Delivery::Discard
    );
    assert_eq!(
        lock.apply(Event::SubjectRemoved {
            subject: Subject::Client(OTHER),
        }),
        vec![Effect::Cover {
            generation: 2,
            subjects: vec![SESSION],
        }]
    );
    assert!(!lock.is_covered(Subject::Client(OTHER)));
}

#[test]
fn an_unmapped_lock_surface_receives_no_input() {
    let mut lock = ready();
    lock.apply(Event::LockRequested { locker: LOCKER });
    lock.apply(Event::CoverPresented {
        generation: 1,
        output: OUTPUT,
    });
    assert_eq!(lock.state(), State::Locked);
    // `locked` follows the cover, not the lock surface, so the owner can still
    // be without a usable surface: a miss must be consumed.
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Discard);
    lock.apply(Event::LockerSurfaceMapped {
        locker: LOCKER,
        output: OUTPUT,
    });
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Deliver);
    lock.apply(Event::LockerSurfaceUnmapped {
        locker: LOCKER,
        output: OUTPUT,
    });
    assert_eq!(lock.gate_input(InputSource::Local, OWNER_SUBJECT), Delivery::Discard);
}

#[test]
fn a_crash_during_the_reveal_completes_it() {
    let mut lock = locked();
    lock.apply(Event::UnlockRequested { locker: LOCKER });
    assert_eq!(lock.state(), State::Unlocking);
    // The owner authenticated before it went away, so the authorised reveal
    // finishes; it is not a new decision.
    assert_eq!(lock.apply(Event::LockerGone { locker: LOCKER }), Vec::new());
    assert_eq!(lock.state(), State::Unlocking);
    assert_eq!(
        lock.apply(Event::RevealComplete),
        vec![
            Effect::Embargo(Embargo::RELEASED),
            Effect::Publish(Published {
                state: State::Unlocked,
                active_changed: true,
            }),
        ]
    );
    assert_eq!(lock.state(), State::Unlocked);
    assert_eq!(lock.capture(CaptureStream::Granted), Capture::Desktop);
}
