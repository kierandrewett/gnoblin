//! The state machine itself.

use std::collections::BTreeSet;

use gnoblin_types::{ClientId, OutputId};

use crate::effect::{Effect, Published, RejectReason};
use crate::event::{CoverFailure, Event};
use crate::ids::Subject;
use crate::state::{Capture, CaptureStream, Delivery, Embargo, InputSource, State};

/// The active owner and what it has proven so far.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Owner {
    /// The owning connection.
    client: ClientId,
    /// Outputs whose lock surface is mapped and usable.
    surfaces: BTreeSet<OutputId>,
    /// Whether `locked` was already sent. Output hotplug resets presentation,
    /// never this: the owner heard about the lock exactly once.
    announced: bool,
}

/// A snapshot of the two things an event can change, for the change report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Observed {
    state: State,
    embargo: Embargo,
}

/// Session lock and protection cover state machine.
///
/// Construct one with [`Lock::new`] and feed it every event in
/// [`crate::Event`]. It holds the covered subject set, the output layout, the
/// presentation barrier and the active owner; it holds no actor, no stage, no
/// seat and no protocol object, so it is testable without a compositor.
#[derive(Clone, Debug)]
pub struct Lock {
    state: State,
    owner: Option<Owner>,
    subjects: BTreeSet<Subject>,
    outputs: BTreeSet<OutputId>,
    presented: BTreeSet<OutputId>,
    generation: u64,
}

impl Default for Lock {
    fn default() -> Self {
        Self::new()
    }
}

impl Lock {
    /// An unlocked session covering the whole desktop and nothing else.
    ///
    /// The compositor reports the output layout with
    /// [`Event::OutputsChanged`] before the first lock request; with no outputs
    /// a lock request fails closed rather than waiting for a frame that cannot
    /// arrive.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Unlocked,
            owner: None,
            subjects: BTreeSet::from([Subject::Session]),
            outputs: BTreeSet::new(),
            presented: BTreeSet::new(),
            generation: 0,
        }
    }

    /// Where the session lock is.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Whether anything is hidden. This is `ScreenSaver.GetActive`.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.state.is_active()
    }

    /// The active owner, if one exists. `CoverFailed` has none.
    #[must_use]
    pub fn locker(&self) -> Option<ClientId> {
        self.owner.as_ref().map(|owner| owner.client)
    }

    /// Every subject that must be hidden, in identifier order.
    pub fn covered(&self) -> impl Iterator<Item = Subject> + '_ {
        self.subjects.iter().copied()
    }

    /// Whether `subject` is under the cover.
    #[must_use]
    pub fn is_covered(&self, subject: Subject) -> bool {
        self.subjects.contains(&subject)
    }

    /// The current output layout, in identifier order.
    pub fn outputs(&self) -> impl Iterator<Item = OutputId> + '_ {
        self.outputs.iter().copied()
    }

    /// What the lock takes away right now.
    #[must_use]
    pub const fn embargo(&self) -> Embargo {
        if self.state.is_active() {
            Embargo::HELD
        } else {
            Embargo::RELEASED
        }
    }

    /// Where an input event goes.
    ///
    /// One table covers every device class, exactly as the C filter does: keys,
    /// pointer, touch, tablet, input-method commits and the mapping of an input
    /// method's popup all follow this answer, because a miss is consumed before
    /// any of them run.
    ///
    /// | state                     | ordinary subject | owner, no mapped surface | owner, mapped |
    /// | ------------------------- | ---------------- | ------------------------ | ------------- |
    /// | `Unlocked`                | deliver          | deliver                  | deliver       |
    /// | `Locking`                 | discard          | discard                  | deliver, local only |
    /// | `Locked`                  | discard          | discard                  | deliver       |
    /// | `Unlocking`               | discard          | discard                  | discard       |
    /// | `CoverFailed`             | discard          | discard                  | discard       |
    ///
    /// Remote input waits for `Locked`: the lock scene must be presented before
    /// a remote session may type into it.
    #[must_use]
    pub fn gate_input(&self, source: InputSource, subject: Subject) -> Delivery {
        if self.state == State::Unlocked {
            return Delivery::Deliver;
        }
        let Some(owner) = &self.owner else {
            return Delivery::Discard;
        };
        if subject != Subject::Client(owner.client) || owner.surfaces.is_empty() {
            // A miss is consumed: no focus, no grab, no binding and no gesture.
            return Delivery::Discard;
        }
        match (source, self.state) {
            // The user may type into the lock UI as soon as it is mapped, even
            // while another output is still proving its frame.
            (InputSource::Local, State::Locking | State::Locked) => Delivery::Deliver,
            (InputSource::Remote, State::Locked) => Delivery::Deliver,
            _ => Delivery::Discard,
        }
    }

    /// Whether `subject` may use the selection, a drag source or data-control.
    ///
    /// While covered only the owner may, so a copy from a covered client cannot
    /// be pasted into the lock screen and a drag cannot leave one.
    #[must_use]
    pub fn gate_clipboard(&self, subject: Subject) -> Delivery {
        if self.state == State::Unlocked {
            return Delivery::Deliver;
        }
        match &self.owner {
            Some(owner) if subject == Subject::Client(owner.client) && self.state != State::CoverFailed => {
                Delivery::Deliver
            }
            _ => Delivery::Discard,
        }
    }

    /// What a capture path may return.
    ///
    /// A request that holds no grant is refused throughout a lock; a stream that
    /// was granted before the lock sees the lock scene only once it is presented.
    /// Before that, and in `CoverFailed` and `Unlocking`, it gets no frames:
    /// returning desktop content there is the leak this crate exists to prevent.
    #[must_use]
    pub const fn capture(&self, stream: CaptureStream) -> Capture {
        match (self.state, stream) {
            (State::Unlocked, _) => Capture::Desktop,
            (State::Locked, CaptureStream::Granted) => Capture::LockScene,
            _ => Capture::Refused,
        }
    }

    /// Apply one event and return the effects in the order they must happen.
    ///
    /// Deterministic: the same event sequence produces the same effects. An
    /// event that does not apply is ignored and produces none, which is how a
    /// late frame, a duplicate presentation or a stray `unlock` after the
    /// reveal are all harmless.
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        let before = Observed {
            state: self.state,
            embargo: self.embargo(),
        };
        // A restarted consumer holds no state, so it is republished even though
        // the lock itself is unchanged.
        let republish = matches!(&event, Event::RuntimeRestarted);
        let mut effects = self.transition(event);
        let after = Observed {
            state: self.state,
            embargo: self.embargo(),
        };
        if after.embargo != before.embargo {
            effects.push(Effect::Embargo(after.embargo));
        }
        if after.state != before.state || republish {
            effects.push(Effect::Publish(Published {
                state: after.state,
                active_changed: after.state.is_active() != before.state.is_active(),
            }));
        }
        effects
    }

    fn transition(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::LockRequested { locker } => self.lock_requested(locker),
            Event::CoverPresented { generation, output } => self.cover_presented(generation, output),
            Event::CoverFailed { generation, reason } => self.cover_failed(generation, reason),
            Event::OutputsChanged { outputs } => self.outputs_changed(&outputs),
            Event::LockerSurfaceMapped { locker, output } => self.surface(locker, output, true),
            Event::LockerSurfaceUnmapped { locker, output } => self.surface(locker, output, false),
            Event::LockerGone { locker } => self.owner_lost(locker, CoverFailure::LockerDied),
            Event::LockAbandoned { locker } => self.owner_lost(locker, CoverFailure::LockerAbandoned),
            Event::UnlockRequested { locker } => self.unlock_requested(locker),
            Event::RevealComplete => self.reveal_complete(),
            Event::SubjectAdded { subject } => self.subject_added(subject),
            Event::SubjectRemoved { subject } => self.subject_removed(subject),
            Event::ProtocolDisabled => self.protocol_disabled(),
            Event::RuntimeRestarted => Vec::new(),
        }
    }

    /// The first request while unlocked owns the lock; a request while any other
    /// lock exists receives `finished`, and a request in `CoverFailed` is a
    /// takeover that must prove a fresh cover.
    fn lock_requested(&mut self, locker: ClientId) -> Vec<Effect> {
        match self.state {
            State::Unlocked => {
                if self.outputs.is_empty() {
                    // Nothing could ever present, so entering `Locking` would
                    // wait for a frame forever. Cover nothing, unlock nothing.
                    self.state = State::CoverFailed;
                    return vec![
                        Effect::KeepCovered {
                            reason: CoverFailure::NoOutputs,
                        },
                        Effect::Reject {
                            locker,
                            reason: RejectReason::CoverUnavailable,
                        },
                    ];
                }
                self.owner = Some(Owner {
                    client: locker,
                    surfaces: BTreeSet::new(),
                    announced: false,
                });
                self.state = State::Locking;
                self.restart_barrier()
            }
            State::CoverFailed => {
                // Take over the existing cover: the old owner is gone and the
                // desktop is still hidden, so nothing may be revealed and the
                // replacement must present a fresh covered frame.
                self.owner = Some(Owner {
                    client: locker,
                    surfaces: BTreeSet::new(),
                    announced: false,
                });
                self.state = State::Locking;
                self.restart_barrier()
            }
            State::Locking | State::Locked | State::Unlocking => vec![Effect::Reject {
                locker,
                reason: RejectReason::AlreadyLocked,
            }],
        }
    }

    fn cover_presented(&mut self, generation: u64, output: OutputId) -> Vec<Effect> {
        if generation != self.generation
            || matches!(self.state, State::Unlocked | State::Unlocking)
            || !self.outputs.contains(&output)
        {
            // A frame from an older barrier, a frame for an output that no
            // longer exists, or a frame while nothing is covered proves
            // nothing. This is the whole defence against a queued pre-cover
            // frame.
            return Vec::new();
        }
        self.presented.insert(output);
        if self.presented != self.outputs {
            return Vec::new();
        }
        let Some(owner) = self.owner.as_mut() else {
            // The cover landed after the owner went away: keep it, announce
            // nothing. A replacement must present its own generation.
            return Vec::new();
        };
        self.state = State::Locked;
        if owner.announced {
            // Hotplug re-proved an already announced lock; the owner heard
            // `locked` once and must not hear it again.
            return Vec::new();
        }
        owner.announced = true;
        vec![Effect::SendLocked { locker: owner.client }]
    }

    fn cover_failed(&mut self, generation: u64, reason: CoverFailure) -> Vec<Effect> {
        if generation != self.generation || matches!(self.state, State::Unlocked | State::Unlocking) {
            return Vec::new();
        }
        self.state = State::CoverFailed;
        // The owner stays: a transient render failure may still present, and if
        // it cannot, it times out and `LockerGone` lets a replacement in.
        vec![Effect::KeepCovered { reason }]
    }

    fn outputs_changed(&mut self, outputs: &[OutputId]) -> Vec<Effect> {
        let next: BTreeSet<OutputId> = outputs.iter().copied().collect();
        if next == self.outputs {
            return Vec::new();
        }
        self.outputs = next;
        // While unlocking the reveal is already committed; while unlocked no
        // barrier was armed. Only a covered session has a proof to invalidate.
        if matches!(self.state, State::Unlocked | State::Unlocking) {
            return Vec::new();
        }
        let mut effects = self.restart_barrier();
        // With no output left there is no surface to configure either.
        if let Some(owner) = &self.owner
            && !self.outputs.is_empty()
        {
            effects.push(Effect::ConfigureLockSurfaces {
                locker: owner.client,
                outputs: self.output_list(),
            });
        }
        effects
    }

    fn surface(&mut self, locker: ClientId, output: OutputId, mapped: bool) -> Vec<Effect> {
        if matches!(self.state, State::Unlocked | State::Unlocking) {
            return Vec::new();
        }
        let Some(owner) = self.owner.as_mut() else {
            return Vec::new();
        };
        if owner.client != locker {
            // Another client's surface is never a lock surface, whoever it is.
            return Vec::new();
        }
        if mapped {
            owner.surfaces.insert(output);
        } else {
            owner.surfaces.remove(&output);
        }
        Vec::new()
    }

    /// A locker that crashed or abandoned its request never unlocks: the cover
    /// and the embargo stay, and a replacement may take over.
    fn owner_lost(&mut self, locker: ClientId, reason: CoverFailure) -> Vec<Effect> {
        if !self.owner.as_ref().is_some_and(|owner| owner.client == locker) {
            return Vec::new();
        }
        self.owner = None;
        if self.state == State::Unlocking {
            // The owner authenticated before it went away, so the reveal is
            // committed. Keep `Unlocking`, drop the owner: no further request
            // can arrive and `RevealComplete` finishes what was authorised.
            return Vec::new();
        }
        self.state = State::CoverFailed;
        vec![Effect::KeepCovered { reason }]
    }

    fn unlock_requested(&mut self, locker: ClientId) -> Vec<Effect> {
        if self.state != State::Locked {
            // Before `locked` a client may only destroy its lock object, so an
            // unlock here is a protocol error and changes nothing.
            return vec![Effect::Reject {
                locker,
                reason: RejectReason::NotLocked,
            }];
        }
        if !self.owner.as_ref().is_some_and(|owner| owner.client == locker) {
            return vec![Effect::Reject {
                locker,
                reason: RejectReason::NotOwner,
            }];
        }
        self.state = State::Unlocking;
        vec![Effect::Reveal]
    }

    fn reveal_complete(&mut self) -> Vec<Effect> {
        if self.state != State::Unlocking {
            // A stray completion while covered must never unlock anything.
            return Vec::new();
        }
        self.state = State::Unlocked;
        self.owner = None;
        self.presented.clear();
        // The next lock starts from a generation nobody has presented.
        self.generation += 1;
        Vec::new()
    }

    fn subject_added(&mut self, subject: Subject) -> Vec<Effect> {
        if !self.subjects.insert(subject) {
            return Vec::new();
        }
        if self.state == State::Unlocked {
            return Vec::new();
        }
        if self.state == State::Unlocking {
            // The reveal restores every subject, so the newcomer is covered for
            // the rest of the teardown and no barrier is needed.
            return vec![Effect::Cover {
                generation: self.generation,
                subjects: self.subject_list(),
            }];
        }
        // A newly covered subject is only proven covered by a fresh frame, so
        // the barrier restarts exactly as it does for an output change.
        self.restart_barrier()
    }

    fn subject_removed(&mut self, subject: Subject) -> Vec<Effect> {
        if !self.subjects.remove(&subject) {
            return Vec::new();
        }
        if self.state == State::Unlocked {
            return Vec::new();
        }
        // Removing a subject reveals nothing new, so no presentation is owed.
        vec![Effect::Cover {
            generation: self.generation,
            subjects: self.subject_list(),
        }]
    }

    fn protocol_disabled(&mut self) -> Vec<Effect> {
        if self.state == State::Unlocked {
            return Vec::new();
        }
        // Withdrawing the protocol must never take the cover with it, and a
        // reload can never unlock: the current owner keeps its lock until it
        // unlocks normally or dies.
        vec![Effect::KeepCovered {
            reason: CoverFailure::ProtocolDisabled,
        }]
    }

    /// Bump the generation, ask for a fresh cover and require every output to
    /// present it again.
    fn restart_barrier(&mut self) -> Vec<Effect> {
        self.generation += 1;
        self.presented.clear();
        let mut effects = vec![Effect::Cover {
            generation: self.generation,
            subjects: self.subject_list(),
        }];
        if self.outputs.is_empty() {
            // No output can present, so no proof can arrive. An owned session
            // drops to `CoverFailed` rather than waiting forever in `Locking`.
            if self.state.is_owned() {
                self.state = State::CoverFailed;
            }
            effects.push(Effect::KeepCovered {
                reason: CoverFailure::NoOutputs,
            });
            return effects;
        }
        if self.state == State::Locked {
            self.state = State::Locking;
        }
        effects.push(Effect::AwaitPresentation {
            generation: self.generation,
            outputs: self.output_list(),
        });
        effects
    }

    fn subject_list(&self) -> Vec<Subject> {
        self.subjects.iter().copied().collect()
    }

    fn output_list(&self) -> Vec<OutputId> {
        self.outputs.iter().copied().collect()
    }
}
