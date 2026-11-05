//! Per-client protection covers for hosted sessions.
//!
//! Design/rust-compositor.md, "Hosting sessions" primitive 3: the session lock
//! covers every surface, and a host compositor that runs nested sessions must
//! also be able to hide **one** client while the rest of the machine keeps
//! working. bsm's rules become this machine's rules:
//!
//! - **Protect before reveal.** A client is never reported protected because
//!   someone asked: the answer waits for a frame that was described *after* its
//!   surfaces left the scene and that reached every output.
//! - **The session says so too.** The host asks the session to cover its own
//!   scene ([`ProtectionEffect::Ask`]) and the session acknowledges after its
//!   own cover frame was presented. Both facts are required, so a session that
//!   is asked and then dies leaves the cover standing and is never announced.
//! - **A crash unlocks nothing.** [`ProtectionEvent::SessionGone`] and
//!   [`ProtectionEvent::CoverFailed`] keep the client hidden: they remove the
//!   chance of a proof, never a cover.
//! - **Only the host lifts a cover.** There is no event a session can send that
//!   shows it again; [`ProtectionEvent::Release`] comes from the session
//!   manager, which is the only party that decides a session may be seen.
//!
//! The subject is a [`gnoblin_types::ClientId`], exactly as the session lock's
//! [`crate::Subject::Client`] is, so the two covers name a session the same way
//! and a consumer never branches on which cover hid it.
//!
//! # Failure modes first
//!
//! Each entry names its test in [`crate::protection_tests`].
//!
//! 1. A protection is announced before a frame without the client reached every
//!    output: `nothing_is_protected_before_a_frame_covers_every_output`.
//! 2. The session acknowledges, and its own pixels are still on screen:
//!    `an_acknowledgement_alone_does_not_protect`.
//! 3. A protection is asked for while no output exists:
//!    `no_output_can_prove_a_cover`.
//! 4. The session dies while protected, or while still covering:
//!    `a_crashed_session_keeps_the_cover`.
//! 5. A session acknowledges a protection it was never sent:
//!    `an_acknowledgement_of_an_unknown_serial_is_reported`.
//! 6. A stale acknowledgement releases the current protection:
//!    `a_stale_acknowledgement_changes_nothing`.
//! 7. An output appears while the client is hidden and the new output shows it:
//!    `a_new_output_does_not_owe_a_frame`.
//! 8. An output that disappears keeps owing a frame:
//!    `a_removed_output_stops_being_owed`.
//! 9. Covering one session hides another:
//!    `only_the_named_session_is_hidden`.
//! 10. A failed frame is announced as a protection:
//!     `a_failed_frame_announces_nothing`.
//! 11. A protection that was already announced is announced twice:
//!     `a_protection_is_announced_once` (and a repeated request answers, which
//!     is the manager's retry path).

use std::collections::{BTreeMap, BTreeSet};

use gnoblin_types::{ClientId, OutputId};

/// Where one client is against the cover.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectionState {
    /// Not covered: its surfaces may be shown and it may be given input.
    Visible,
    /// Hidden, but not proven: a frame has not reached every output, the session
    /// has not acknowledged, or both.
    Covering,
    /// Every output presented a frame without it **and** the session
    /// acknowledged that frame. This is what the manager may act on.
    Covered,
}

impl ProtectionState {
    /// A stable lower-case name for logs, events and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Covering => "covering",
            Self::Covered => "covered",
        }
    }

    /// Whether the client's surfaces must not be composed.
    ///
    /// True from the moment the cover is asked for: the pixels leave the scene
    /// before anything is proven about the frame that carries the change.
    #[must_use]
    pub const fn is_hidden(self) -> bool {
        !matches!(self, Self::Visible)
    }

    /// Whether the protection is proven and may be reported to the manager.
    #[must_use]
    pub const fn is_protected(self) -> bool {
        matches!(self, Self::Covered)
    }
}

/// Why a protection is not proven, or why a hidden client stays hidden.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectionFailure {
    /// No output exists, so no frame can prove anything.
    NoOutputs,
    /// A frame for the cover could not be produced.
    RenderFailed,
    /// The session's connection went away while it was hidden.
    SessionDied,
    /// The session-host protocol was withdrawn while a client was hidden.
    ProtocolDisabled,
}

impl ProtectionFailure {
    /// A stable lower-case name for logs, events and tests.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoOutputs => "no-outputs",
            Self::RenderFailed => "render-failed",
            Self::SessionDied => "session-died",
            Self::ProtocolDisabled => "protocol-disabled",
        }
    }
}

/// Everything that can change a protection cover.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtectionEvent {
    /// The session manager asked for this client to be protected.
    Protect {
        /// The client to hide.
        client: ClientId,
    },
    /// The session answered `protected` with this serial.
    Acknowledged {
        /// The session that answered.
        client: ClientId,
        /// The protection serial it answered, which is a generation.
        serial: u64,
    },
    /// A frame described after `generation` was reached on `output`.
    Presented {
        /// The protection generation the frame was described under.
        generation: u64,
        /// The output it reached.
        output: OutputId,
    },
    /// The output layout is now exactly this set.
    OutputsChanged {
        /// Every connected output.
        outputs: Vec<OutputId>,
    },
    /// The manager lifted the cover: the client may be shown again.
    Release {
        /// The client to show.
        client: ClientId,
    },
    /// The client's connection went away.
    SessionGone {
        /// The session that died.
        client: ClientId,
    },
    /// A frame that would have proven a cover could not be drawn.
    CoverFailed {
        /// The protection generation that was being proven.
        generation: u64,
        /// Why it failed.
        reason: ProtectionFailure,
    },
}

/// What the compositor must do, in the order it must do it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtectionEffect {
    /// Stop composing this client's surfaces. The cover is what the scene draws
    /// in their place, so nothing else is needed for the pixels.
    Hide {
        /// The client whose surfaces leave the scene.
        client: ClientId,
        /// The protection this belongs to.
        generation: u64,
    },
    /// Ask the session to cover its own scene, with this serial.
    Ask {
        /// The session to ask.
        client: ClientId,
        /// The serial it must answer with, which is also the generation.
        serial: u64,
    },
    /// Arm the barrier: every listed output must present a frame described
    /// after this generation.
    AwaitPresentation {
        /// The protection being proven.
        generation: u64,
        /// The outputs that must present it.
        outputs: Vec<OutputId>,
    },
    /// The protection is proven and may be reported to the manager.
    Protected {
        /// The client that is now protected.
        client: ClientId,
        /// The protection serial the answer belongs to.
        serial: u64,
    },
    /// Compose this client's surfaces again.
    Show {
        /// The client that may be shown.
        client: ClientId,
    },
    /// Keep the client hidden although nothing can prove the cover.
    KeepCovered {
        /// The client that stays hidden.
        client: ClientId,
        /// Why no proof will arrive.
        reason: ProtectionFailure,
    },
    /// The session answered a serial the host never sent. The protocol layer
    /// raises a protocol error on that connection; the cover stays.
    UnknownAck {
        /// The session that answered.
        client: ClientId,
        /// The serial it answered with.
        serial: u64,
    },
}

/// What one client has proven so far.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    /// Hidden, proven, or neither.
    state: ProtectionState,
    /// The generation this protection was asked for with.
    generation: u64,
    /// Outputs that presented a frame described after the hide.
    presented: BTreeSet<OutputId>,
    /// Whether the session answered `protected` for this generation.
    acknowledged: bool,
    /// The connection is gone, so no acknowledgement can arrive.
    gone: bool,
}

/// Protection covers, one entry per hidden client.
///
/// Holds no compositor: the surface flags, the frames and the protocol objects
/// are the caller's, and every answer here is a value a test can assert on.
#[derive(Clone, Debug)]
pub struct Protection {
    /// The last generation handed to a protection.
    generation: u64,
    /// The current output layout.
    outputs: BTreeSet<OutputId>,
    /// Every client that is not visible, in identifier order.
    clients: BTreeMap<ClientId, Entry>,
}

impl Default for Protection {
    fn default() -> Self {
        Self::new()
    }
}

impl Protection {
    /// No cover and no outputs yet.
    ///
    /// The compositor reports the layout with [`ProtectionEvent::OutputsChanged`]
    /// before the first protection; with no output a protection is asked for but
    /// never proven, which is the safe direction.
    #[must_use]
    pub fn new() -> Self {
        Self {
            generation: 0,
            outputs: BTreeSet::new(),
            clients: BTreeMap::new(),
        }
    }

    /// The generation the next frame must be described under.
    ///
    /// A frame the compositor starts describing reads this once and reports it
    /// with [`ProtectionEvent::Presented`], so a protection that arrives while a
    /// frame is being described cannot be proven by that frame.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Where `client` is against the cover.
    #[must_use]
    pub fn state(&self, client: ClientId) -> ProtectionState {
        self.clients
            .get(&client)
            .map_or(ProtectionState::Visible, |entry| entry.state)
    }

    /// Whether `client`'s surfaces must not be composed.
    #[must_use]
    pub fn is_hidden(&self, client: ClientId) -> bool {
        self.state(client).is_hidden()
    }

    /// Every client whose surfaces must not be composed, in identifier order.
    pub fn hidden(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients
            .iter()
            .filter(|(_, entry)| entry.state.is_hidden())
            .map(|(client, _)| *client)
    }

    /// Every proven protection, in identifier order. This is what a capture
    /// path, the object graph and the manager read.
    pub fn protected(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients
            .iter()
            .filter(|(_, entry)| entry.state.is_protected())
            .map(|(client, _)| *client)
    }

    /// Whether any cover is still waiting for a proof.
    ///
    /// The compositor uses it to describe a covered frame again every turn: a frame
    /// that could not be produced takes its damage with it, and a cover that is
    /// merely owed must never look settled.
    #[must_use]
    pub fn is_proving(&self) -> bool {
        self.clients
            .values()
            .any(|entry| entry.state == ProtectionState::Covering)
    }

    /// Apply one event and return the effects in the order they must happen.
    pub fn apply(&mut self, event: ProtectionEvent) -> Vec<ProtectionEffect> {
        match event {
            ProtectionEvent::Protect { client } => self.protect(client),
            ProtectionEvent::Acknowledged { client, serial } => self.acknowledge(client, serial),
            ProtectionEvent::Presented { generation, output } => self.presented(generation, output),
            ProtectionEvent::OutputsChanged { outputs } => self.outputs_changed(&outputs),
            ProtectionEvent::Release { client } => self.release(client),
            ProtectionEvent::SessionGone { client } => self.session_gone(client),
            ProtectionEvent::CoverFailed { generation, reason } => self.cover_failed(generation, reason),
        }
    }

    /// Hides the client at once and starts a fresh barrier.
    ///
    /// A repeated request is answered with the state the client is in: a manager
    /// that timed out may ask again for a protection it already holds, and the
    /// answer must not be silence.
    fn protect(&mut self, client: ClientId) -> Vec<ProtectionEffect> {
        if let Some(entry) = self.clients.get(&client) {
            return if entry.state.is_protected() {
                vec![ProtectionEffect::Protected {
                    client,
                    serial: entry.generation,
                }]
            } else {
                Vec::new()
            };
        }
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        self.clients.insert(
            client,
            Entry {
                state: ProtectionState::Covering,
                generation,
                presented: BTreeSet::new(),
                acknowledged: false,
                gone: false,
            },
        );
        let mut effects = vec![
            ProtectionEffect::Hide { client, generation },
            ProtectionEffect::Ask {
                client,
                serial: generation,
            },
        ];
        if self.outputs.is_empty() {
            // Nothing can present, so nothing will ever be announced. The client
            // stays hidden, which is the only safe answer with no screen.
            effects.push(ProtectionEffect::KeepCovered {
                client,
                reason: ProtectionFailure::NoOutputs,
            });
        } else {
            effects.push(ProtectionEffect::AwaitPresentation {
                generation,
                outputs: self.output_list(),
            });
        }
        effects
    }

    fn acknowledge(&mut self, client: ClientId, serial: u64) -> Vec<ProtectionEffect> {
        let Some(entry) = self.clients.get_mut(&client) else {
            return vec![ProtectionEffect::UnknownAck { client, serial }];
        };
        if serial < entry.generation {
            // An answer to a cover the session has already replaced. A cover
            // raised for an older protection proves nothing about this one.
            return Vec::new();
        }
        if serial > entry.generation {
            // The session cannot know a protection that was never sent, so this
            // is a client that is not following the protocol.
            return vec![ProtectionEffect::UnknownAck { client, serial }];
        }
        entry.acknowledged = true;
        self.complete(client)
    }

    /// Marks the frame's output as proven for every protection that existed
    /// before the frame was described.
    fn presented(&mut self, generation: u64, output: OutputId) -> Vec<ProtectionEffect> {
        if !self.outputs.contains(&output) {
            // An output this session does not drive, or one that was removed
            // while the frame was in flight, proves nothing.
            return Vec::new();
        }
        let covering: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.state == ProtectionState::Covering && entry.generation <= generation)
            .map(|(client, _)| *client)
            .collect();
        let mut effects = Vec::new();
        for client in covering {
            let Some(entry) = self.clients.get_mut(&client) else {
                continue;
            };
            // A frame reaches an output once, so a second report for the same
            // output changes nothing.
            if !entry.presented.insert(output) {
                continue;
            }
            effects.extend(self.complete(client));
        }
        effects
    }

    /// Announces a protection once both proofs are in.
    fn complete(&mut self, client: ClientId) -> Vec<ProtectionEffect> {
        let Some(entry) = self.clients.get(&client) else {
            return Vec::new();
        };
        if entry.state != ProtectionState::Covering
            || !entry.acknowledged
            || entry.gone
            || self.outputs.is_empty()
            || !self.outputs.iter().all(|output| entry.presented.contains(output))
        {
            return Vec::new();
        }
        let serial = entry.generation;
        if let Some(entry) = self.clients.get_mut(&client) {
            entry.state = ProtectionState::Covered;
        }
        vec![ProtectionEffect::Protected { client, serial }]
    }

    /// An output that appears while a client is hidden never showed that client,
    /// because the hide happened first; one that disappears cannot present
    /// anything again. Neither owes a frame.
    fn outputs_changed(&mut self, outputs: &[OutputId]) -> Vec<ProtectionEffect> {
        let next: BTreeSet<OutputId> = outputs.iter().copied().collect();
        if next == self.outputs {
            return Vec::new();
        }
        self.outputs = next;
        let covering: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.state == ProtectionState::Covering)
            .map(|(client, _)| *client)
            .collect();
        let layout = self.outputs.clone();
        let mut effects = Vec::new();
        for client in covering {
            if let Some(entry) = self.clients.get_mut(&client) {
                entry.presented.retain(|output| layout.contains(output));
            }
            effects.extend(self.complete(client));
        }
        effects
    }

    fn release(&mut self, client: ClientId) -> Vec<ProtectionEffect> {
        match self.clients.remove(&client) {
            // A session whose connection is gone has no surfaces to show, so a
            // release only forgets it. The cover was kept for as long as
            // anything could have been on screen.
            Some(entry) if entry.state.is_hidden() && !entry.gone => {
                vec![ProtectionEffect::Show { client }]
            }
            // A release for a client that is not hidden changes nothing.
            Some(_) | None => Vec::new(),
        }
    }

    /// Keeps the cover. A session that dies while hidden is never announced and
    /// never shown again through this machine.
    fn session_gone(&mut self, client: ClientId) -> Vec<ProtectionEffect> {
        let Some(entry) = self.clients.get_mut(&client) else {
            return Vec::new();
        };
        entry.gone = true;
        if !entry.state.is_hidden() {
            return Vec::new();
        }
        vec![ProtectionEffect::KeepCovered {
            client,
            reason: ProtectionFailure::SessionDied,
        }]
    }

    fn cover_failed(&mut self, generation: u64, reason: ProtectionFailure) -> Vec<ProtectionEffect> {
        let affected: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.state.is_hidden() && entry.generation <= generation)
            .map(|(client, _)| *client)
            .collect();
        affected
            .into_iter()
            .map(|client| ProtectionEffect::KeepCovered { client, reason })
            .collect()
    }

    fn output_list(&self) -> Vec<OutputId> {
        self.outputs.iter().copied().collect()
    }
}
