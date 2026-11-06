//! XML protocol `ext_session_lock_v1` (`<protocol name="ext_session_lock_v1">`); interfaces served: `ext_session_lock_v1`.
//!
//! `ext_session_lock_v1`: the lock request, the cover and the embargo.
//!
//! The protocol is translated here and decided in `gnoblin-lock`: a request becomes
//! [`gnoblin_lock::Event::LockRequested`], and the effects that come back are what
//! this module performs — hold the client's confirmation until the cover is proven,
//! send `locked` once, refuse a second locker, keep the cover when the locker goes
//! away, and take it down only for the owner's own unlock.
//!
//! ## What "covered" means in this slice
//!
//! There is no renderer yet, so nothing is composited: a frame is the whole target
//! presented as damage, and no client content is drawn on it at all. What the lock
//! requires is not a drawn cover but a **presented** one: [`crate::State`] arms a
//! barrier with a generation, and a frame presented on an output echoes that
//! generation back as [`gnoblin_lock::Event::CoverPresented`]. Only when every
//! output of the layout has echoed the current generation may `locked` be sent, so
//! a frame presented before the barrier was armed cannot prove anything. When the
//! renderer lands it fills that frame with the cover instead of with nothing.
//!
//! ## Failure modes first
//!
//! The plan's card, `src/protocols/session-lock/README.md` and `gnoblin-lock`'s own
//! list name these; each is a test in `tests/session_lock.rs` unless it says
//! otherwise.
//!
//! 1. **`locked` is sent before a frame proved the cover.** The confirmation is held
//!    from `lock` until the barrier proves, so withholding the backend's presents
//!    leaves the client unannounced: `locked_waits_for_a_presented_cover`.
//! 2. **A second locker takes the lock from the first, or a refusal cancels a lock
//!    the client already holds.** The first request owns the lock and every later
//!    one is answered with `finished`: `a_second_locker_is_refused_with_finished`.
//!    A refusal answers one request and no more, so a client that asks twice loses
//!    only the second: `a_second_request_from_the_same_client_is_refused_alone`.
//! 3. **A covered client keeps receiving input.** Every input path asks the lock
//!    first, so keys and pointer events reach the owner's lock surface or nothing:
//!    `a_covered_client_receives_no_input` and
//!    `an_unmapped_lock_surface_receives_no_input`.
//! 4. **A locker that dies unlocks the session.** Its disappearance is noticed on
//!    the next loop turn and keeps the cover and the embargo:
//!    `a_locker_crash_keeps_the_cover` and `a_locker_crash_before_locked_keeps_the_cover`.
//! 5. **A replacement locker inherits the old proof.** A takeover restarts the
//!    barrier, so the replacement waits for its own presented cover:
//!    `a_takeover_waits_for_a_fresh_cover`.
//! 6. **Every output removed while locked reveals the desktop.** With no output to
//!    present on, the lock drops to the covered failure state and stays embargoed:
//!    `removing_every_output_keeps_the_cover`.
//! 7. **A hotplugged output shows the desktop, or admits input early.** A layout
//!    change restarts the barrier and reconfigures the owner's surfaces:
//!    `a_hotplugged_output_is_covered_before_it_shows_anything`.
//! 8. **Unlock leaves the session without focus.** The reveal restores the window
//!    the window manager had focused: `unlock_restores_the_previous_focus`.
//!
//! 9. **A locker gives up before `locked` and the session stays coverable.** It
//!    destroys its lock object; the backend destroys the object with the request, so
//!    the next loop turn sees a lock that is gone, keeps the cover and lets a
//!    replacement in: `abandoning_before_locked_keeps_the_cover`.
//!
//! One mode stays with the slice that owns it: a configuration reload that withdraws
//! the global ([`gnoblin_lock::Event::ProtocolDisabled`], config slice). The reason
//! a destroyed lock object records is `LockerDied` rather than `LockerAbandoned`,
//! because a destroyed object and a closed connection are the same thing from here.
//!
//! ## Capture and clipboard
//!
//! No capture or selection global exists in this slice, so the embargo is exposed
//! rather than performed: [`crate::State::capture`] answers what a capture path may
//! return, [`crate::State::capture_allowed`] whether it may run at all, and
//! [`crate::State::selection_allowed`] whether the selection may be reached. The
//! capture lane must call them; a stream that is granted before the lock sees the
//! lock scene once it is presented and no frames before that.

use std::collections::{HashMap, VecDeque};

use smithay::output::Output as SmithayOutput;
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId as WaylandClientId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Size};
use smithay::wayland::session_lock::{
    LockSurface, LockSurfaceConfigure, SessionLockHandler, SessionLockManagerState, SessionLocker,
};
use tracing::{debug, info, warn};

use gnoblin_backend::render::{Color, OutputFrame};
use gnoblin_lock::{
    Capture, CaptureStream, Delivery, Effect as LockEffect, Event as LockEvent, InputSource, Lock, RejectReason,
    State as LockState, Subject,
};
use gnoblin_types::{ClientId, OutputId, Rect};

use crate::State;

/// The colour of the cover: opaque black, so nothing under it can be read through.
const COVER: Color = Color::BLACK;

/// One lock request that is waiting for the cover to be proven.
///
/// The confirmation is what Smithay holds the request open with: dropping it sends
/// `finished`, and [`SessionLocker::lock`] sends `locked`. Holding it here is what
/// keeps the two events in the order the protocol requires.
struct ExtSessionLockV1PendingLock {
    /// The requesting connection, as the lock state machine names it.
    locker: ClientId,
    /// The lock object, kept to notice the connection going away.
    object: ExtSessionLockV1,
    /// Smithay's confirmation, dropped on a refusal and consumed by `locked`.
    confirmation: SessionLocker,
}

/// One lock surface the owner created for an output.
struct ExtSessionLockV1LockSurfaceEntry {
    /// The connection that owns the lock.
    locker: ClientId,
    /// The output it covers.
    output: OutputId,
    /// The protocol object, which is also what a configure is sent through.
    surface: LockSurface,
    /// Whether the client has committed a buffer, which is what makes the surface
    /// a destination for input.
    mapped: bool,
}

/// The presentation barrier: the generation a covering frame must carry, and the
/// outputs that have not presented it yet.
struct ExtSessionLockV1Barrier {
    /// The generation [`LockEvent::CoverPresented`] must echo.
    generation: u64,
    /// Outputs still owing a frame for it.
    pending: Vec<OutputId>,
}

/// The locker the compositor's own `lock` method takes. Protocol identities count up
/// from zero, so the largest value is never one of them.
fn internal_locker() -> ClientId {
    ClientId::new(u64::MAX)
}

/// The per-lock bookkeeping: held confirmations, lock surfaces, the barrier and the
/// client identities this session handed out.
///
/// It is one value so the compositor's own fields stay a short list, and so a
/// reader can see the whole of the protocol's state in one place.
#[derive(Default)]
pub(crate) struct ExtSessionLockV1Tables {
    /// Requests waiting for the barrier, in arrival order.
    pending: Vec<ExtSessionLockV1PendingLock>,
    /// The lock object of the owner, kept to notice its connection going away.
    owner: Option<ExtSessionLockV1>,
    /// Every lock surface, one entry per owner and output.
    surfaces: Vec<ExtSessionLockV1LockSurfaceEntry>,
    /// The armed barrier, while some output still owes a covering frame.
    barrier: Option<ExtSessionLockV1Barrier>,
    /// The generation of the cover the scene is showing.
    cover_generation: u64,
    /// The last cover generation each output was painted for, so an idle covered
    /// session draws nothing and a new generation repaints every output once.
    painted: HashMap<OutputId, u64>,
    /// The identity this session gave each connection, so one locker is one
    /// subject across every request it makes.
    clients: HashMap<WaylandClientId, ClientId>,
    /// The next connection identity.
    next_client: u64,
    /// The replies owed to the compositor's `lock` method, sent once the lock settles.
    lock_waiters: Vec<std::sync::mpsc::SyncSender<bool>>,
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // A request can arrive in the same dispatch as the previous owner's death. The
        // cover is decided after that death is noticed, so a takeover is not refused
        // for an owner that is already gone.
        self.poll_lock();
        let Some(locker) = self.lock_client(confirmation.ext_session_lock()) else {
            // The connection is gone already, so the only true answer is `finished`,
            // which dropping the confirmation sends.
            drop(confirmation);
            return;
        };
        let object = confirmation.ext_session_lock().clone();
        self.lock_tables.pending.push(ExtSessionLockV1PendingLock {
            locker,
            object,
            confirmation,
        });
        let effects = self.lock.apply(LockEvent::LockRequested { locker });
        self.apply_lock(effects);
    }

    fn unlock(&mut self) {
        // Smithay has already checked that this request came from the owner's lock
        // object and that the lock is `Locked`; the state machine answers with the
        // reveal or with nothing.
        let Some(locker) = self.lock.locker() else {
            warn!("[lock] an unlock arrived with no owner; the cover stays");
            return;
        };
        let effects = self.lock.apply(LockEvent::UnlockRequested { locker });
        self.apply_lock(effects);
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(locker) = self.lock_client(surface.ext_session_lock()) else {
            return;
        };
        let Some(target) = self.lock_output(&output) else {
            // A lock surface for an output this session does not drive cannot be
            // configured, so it is not tracked and never receives input. The cover
            // stays up regardless, so nothing is exposed by ignoring it.
            warn!("[lock] a lock surface names an output that is not connected");
            return;
        };
        let Some(size) = self.output_size(target) else {
            return;
        };
        // The configure is what the client acknowledges before its first buffer, so
        // it carries the output's whole logical size.
        surface.with_pending_state(|state| state.size = Some(size));
        surface.send_configure();
        debug!(?locker, %target, "[lock] lock surface created");
        self.lock_tables.surfaces.push(ExtSessionLockV1LockSurfaceEntry {
            locker,
            output: target,
            surface,
            mapped: false,
        });
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: LockSurfaceConfigure) {
        // Nothing waits on the acknowledgement: the client's commit is what says the
        // surface has content, and that is what moves input onto it.
        debug!(
            serial = ?configure.serial,
            surface = ?surface.id(),
            "[lock] configure acknowledged"
        );
    }
}

impl State {
    /// The session lock policy, for a caller that reads what it guarantees.
    #[must_use]
    pub fn lock(&self) -> &Lock {
        &self.lock
    }

    /// Where the session lock is.
    #[must_use]
    pub fn lock_state(&self) -> LockState {
        self.lock.state()
    }

    /// What a capture path may return right now.
    ///
    /// A path holding no grant is refused throughout a lock; a granted stream sees
    /// the lock scene only from `Locked`, and the lock scene is never the desktop
    /// underneath it.
    #[must_use]
    pub fn capture(&self, stream: CaptureStream) -> Capture {
        self.lock.capture(stream)
    }

    /// Whether a capture path may produce frames at all.
    ///
    /// This is the embargo the lock announced, not a second opinion: a capture path
    /// that cannot see the difference between the lock scene and the desktop must
    /// refuse while this is false.
    #[must_use]
    pub fn capture_allowed(&self) -> bool {
        !self.lock_embargo.capture
    }

    /// Whether the selection may be read or written, or a drag source used.
    #[must_use]
    pub fn selection_allowed(&self) -> bool {
        !self.lock_embargo.clipboard
    }

    /// Applies the effects the lock returned, in the order it returned them.
    pub(crate) fn apply_lock(&mut self, effects: Vec<LockEffect>) {
        let mut queue: VecDeque<LockEffect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            match effect {
                LockEffect::Cover { generation, subjects } => {
                    debug!(generation, ?subjects, "[lock] the cover is up");
                    self.lock_tables.cover_generation = generation;
                    // Nothing a client captured survives the cover: a pointer lock and a
                    // shortcut inhibitor are both claims on the session's input, and the
                    // lock screen owns that input from here.
                    self.cover_raised();
                }
                LockEffect::AwaitPresentation { generation, outputs } => {
                    debug!(generation, ?outputs, "[lock] waiting for a covering frame");
                    self.lock_tables.barrier = Some(ExtSessionLockV1Barrier {
                        generation,
                        pending: outputs,
                    });
                }
                LockEffect::SendLocked { locker } => self.send_locked(locker),
                LockEffect::Reject { locker, reason } => self.refuse(locker, reason),
                LockEffect::ConfigureLockSurfaces { locker, outputs } => {
                    self.configure_lock_surfaces(locker, &outputs);
                }
                LockEffect::Reveal => {
                    // Nothing else draws the desktop, so the reveal completes at once
                    // rather than waiting for a frame: the cover is gone from the next
                    // frame on. The embargo is released, not before, by the completion
                    // the state machine returns for it.
                    self.reveal();
                    queue.extend(self.lock.apply(LockEvent::RevealComplete));
                }
                LockEffect::KeepCovered { reason } => {
                    info!(
                        reason = reason.as_str(),
                        "[lock] the cover stays and nothing is revealed"
                    );
                }
                LockEffect::Embargo(embargo) => {
                    debug!(?embargo, "[lock] embargo changed");
                    self.lock_embargo = embargo;
                }
                LockEffect::Publish(published) => {
                    debug!(?published, "[lock] published");
                }
            }
        }
        self.refresh_lock_focus();
        self.settle_lock_waiters();
        self.publish();
    }

    /// Takes the session lock for the compositor's `lock` method and answers `reply`
    /// once the lock settles: `true` only when the cover has proven the session `locked`.
    pub(crate) fn request_lock(&mut self, reply: std::sync::mpsc::SyncSender<bool>) {
        self.lock_tables.lock_waiters.push(reply);
        let effects = self.lock.apply(LockEvent::LockRequested {
            locker: internal_locker(),
        });
        self.apply_lock(effects);
    }

    /// Answers every waiting `lock` call once the lock has a final state. `Locking`
    /// waits for a covering frame on every output, so the answer waits for that too.
    fn settle_lock_waiters(&mut self) {
        let answer = match self.lock.state() {
            LockState::Locked => true,
            LockState::Unlocked | LockState::CoverFailed => false,
            LockState::Locking | LockState::Unlocking => return,
        };
        for reply in self.lock_tables.lock_waiters.drain(..) {
            let _ = reply.send(answer);
        }
    }

    /// Notices everything that happened to a connection without a protocol request
    /// telling us, and keeps the cover for all of it.
    ///
    /// Called once per loop turn, because a client that dies sends no request: its
    /// resources go with the socket and the compositor has to look.
    pub(crate) fn poll_lock(&mut self) {
        let dead: Vec<ClientId> = self
            .lock_tables
            .pending
            .iter()
            .filter(|pending| !pending.object.is_alive())
            .map(|pending| pending.locker)
            .collect();
        for locker in dead {
            self.lock_tables.pending.retain(|pending| {
                if pending.locker == locker && !pending.object.is_alive() {
                    // Dropping the confirmation tells a connection that is already
                    // gone; it is the same refusal it would have received.
                    return false;
                }
                true
            });
            let effects = self.lock.apply(LockEvent::LockerGone { locker });
            self.apply_lock(effects);
        }

        let owner_gone = self.lock_tables.owner.as_ref().is_some_and(|object| !object.is_alive());
        if owner_gone {
            self.lock_tables.owner = None;
            if let Some(locker) = self.lock.locker() {
                let effects = self.lock.apply(LockEvent::LockerGone { locker });
                self.apply_lock(effects);
            }
        }

        self.forget_dead_lock_surfaces();
    }

    /// Describes the opaque cover for `output`, instead of composing the desktop.
    ///
    /// One frame per output and cover generation: the generation is recorded as painted
    /// only when a frame carrying it reached the output, so a frame that could not be
    /// drawn is described again on the next turn instead of leaving the cover unproven.
    /// Once it is painted, an idle covered session draws nothing.
    /// The cover is the frame's background and nothing on top of it, so a covered
    /// window is not drawn even in the frame that proves the cover.
    pub(crate) fn cover_frame(&mut self, output: OutputId, frame: &mut OutputFrame) -> bool {
        let generation = self.lock_tables.cover_generation;
        if self.lock_tables.painted.get(&output) == Some(&generation) {
            return false;
        }
        let Some(rect) = self.output_rect(output) else {
            return false;
        };
        frame.clear();
        frame.set_background(COVER);
        frame.push_damage(Rect::new(0, 0, rect.width, rect.height));
        true
    }

    /// Echoes a presented frame back to the barrier.
    ///
    /// The generation is the one the barrier was armed with, read at the moment the
    /// frame is presented, so a frame that was queued before the cover was raised
    /// carries an older generation and proves nothing.
    pub(crate) fn cover_presented(&mut self, output: OutputId) {
        // The protection covers hear the same frame and answer their own barrier.
        // This runs first because the lock's own barrier is absent while the session
        // is unlocked, and returning early there would hide every presented frame
        // from a protection cover.
        self.protection_presented(output);
        if self.lock.state() != LockState::Unlocked {
            // The frame that just reached this output is the cover for the current
            // generation, so this output is painted until the generation changes.
            let generation = self.lock_tables.cover_generation;
            self.lock_tables.painted.insert(output, generation);
        }
        let Some(barrier) = self.lock_tables.barrier.as_mut() else {
            return;
        };
        let Some(index) = barrier.pending.iter().position(|owed| *owed == output) else {
            return;
        };
        barrier.pending.remove(index);
        let generation = barrier.generation;
        let proved = barrier.pending.is_empty();
        if proved {
            self.lock_tables.barrier = None;
        }
        let effects = self.lock.apply(LockEvent::CoverPresented { generation, output });
        self.apply_lock(effects);
    }

    /// Tells the lock that the layout is exactly this set.
    pub(crate) fn report_outputs(&mut self) {
        let outputs = self.output_ids();
        // The protection covers hear the same layout for the same reason: a frame on
        // every output is what proves a cover, so a layout change is a different set
        // of frames to wait for.
        self.report_hosted_outputs();
        let effects = self.lock.apply(LockEvent::OutputsChanged { outputs });
        self.apply_lock(effects);
    }

    /// A surface committed: a lock surface with a buffer is a destination for input.
    pub(crate) fn lock_surface_committed(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .lock_tables
            .surfaces
            .iter()
            .position(|entry| entry.surface.wl_surface() == surface)
        else {
            return;
        };
        let mapped = crate::state::has_buffer(surface);
        let Some(entry) = self.lock_tables.surfaces.get_mut(index) else {
            return;
        };
        if entry.mapped == mapped {
            return;
        }
        entry.mapped = mapped;
        let (locker, output) = (entry.locker, entry.output);
        let event = if mapped {
            LockEvent::LockerSurfaceMapped { locker, output }
        } else {
            LockEvent::LockerSurfaceUnmapped { locker, output }
        };
        debug!(?locker, %output, mapped, "[lock] lock surface commit");
        let effects = self.lock.apply(event);
        self.apply_lock(effects);
    }

    /// The lock surface the pointer is over, if any may receive it.
    pub(crate) fn lock_surface_under(&self, position: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let owner = self.lock.locker()?;
        if self.lock.gate_input(InputSource::Local, Subject::Client(owner)) != Delivery::Deliver {
            return None;
        }
        let output = self.output_under(position)?;
        let surface = self
            .lock_tables
            .surfaces
            .iter()
            .find(|entry| entry.locker == owner && entry.output == output && entry.mapped)?;
        // The second element is the surface's origin in global coordinates, not the
        // position inside it: every `PointerTarget` subtracts it from the global
        // position, so returning the output's origin is what makes the locker see
        // surface-local coordinates.
        let rect = self.output_rect(output)?;
        let origin = Point::from((f64::from(rect.x), f64::from(rect.y)));
        Some((surface.surface.wl_surface().clone(), origin))
    }

    /// Moves keyboard focus to where the lock allows it, or to nothing.
    fn refresh_lock_focus(&mut self) {
        let target = if self.lock.state() == LockState::Unlocked {
            self.focused_window_surface()
        } else {
            self.lock_input_surface()
        };
        self.set_keyboard_focus(target);
        if self.lock.state() != LockState::Unlocked {
            self.clear_pointer_focus();
        }
    }

    /// The owner's first mapped lock surface, when the gate delivers input to it.
    fn lock_input_surface(&self) -> Option<WlSurface> {
        let owner = self.lock.locker()?;
        if self.lock.gate_input(InputSource::Local, Subject::Client(owner)) != Delivery::Deliver {
            return None;
        }
        self.lock_tables
            .surfaces
            .iter()
            .find(|entry| entry.locker == owner && entry.mapped)
            .map(|entry| entry.surface.wl_surface().clone())
    }

    /// Sends `locked` and keeps the owner's object for liveness checks.
    fn send_locked(&mut self, locker: ClientId) {
        if locker == internal_locker() {
            // The compositor's own lock has no protocol confirmation to send; its
            // waiters are answered by `settle_lock_waiters`.
            return;
        }
        let Some(index) = self
            .lock_tables
            .pending
            .iter()
            .position(|pending| pending.locker == locker)
        else {
            warn!(?locker, "[lock] a lock was granted with no request to confirm");
            return;
        };
        let pending = self.lock_tables.pending.remove(index);
        self.lock_tables.owner = Some(pending.object);
        pending.confirmation.lock();
        info!(?locker, "[lock] the cover is presented; the session is locked");
    }

    /// Refuses a request: a dropped confirmation is what the client sees as
    /// `finished`.
    fn refuse(&mut self, locker: ClientId, reason: RejectReason) {
        if matches!(reason, RejectReason::AlreadyLocked | RejectReason::CoverUnavailable) {
            // Only the request this refusal answers is dropped. The newest one is that
            // request, and dropping every one of the client's would refuse a lock it
            // already holds — leaving the session covered with no locker to unlock it.
            if let Some(index) = self
                .lock_tables
                .pending
                .iter()
                .rposition(|pending| pending.locker == locker)
            {
                let refused = self.lock_tables.pending.remove(index);
                // Dropping sends `finished`, which is the only correct answer for a
                // lock that was not granted.
                drop(refused);
            }
            info!(?locker, reason = reason.as_str(), "[lock] the request was refused");
            return;
        }
        // The remaining reasons are protocol errors the client caused, and Smithay
        // has already posted the error that ends its connection.
        debug!(?locker, reason = reason.as_str(), "[lock] refused");
    }

    /// Sends a fresh configure for the current layout to the owner's surfaces.
    fn configure_lock_surfaces(&mut self, locker: ClientId, outputs: &[OutputId]) {
        let sizes: Vec<(LockSurface, Option<Size<u32, Logical>>)> = self
            .lock_tables
            .surfaces
            .iter()
            .filter(|entry| entry.locker == locker && outputs.contains(&entry.output))
            .map(|entry| (entry.surface.clone(), self.output_size(entry.output)))
            .collect();
        for (surface, size) in sizes {
            let Some(size) = size else {
                continue;
            };
            surface.with_pending_state(|state| state.size = Some(size));
            surface.send_configure();
        }
    }

    /// Takes the lock scene down; the barrier and the surfaces go with it.
    fn reveal(&mut self) {
        self.lock_tables.barrier = None;
        self.lock_tables.owner = None;
        self.lock_tables.surfaces.clear();
        self.lock_tables.painted.clear();
        // The composer drew nothing while the session was covered, so the desktop has
        // to be painted again rather than waiting for a window to move.
        self.damage_outputs();
        debug!("[lock] the reveal is complete");
    }

    /// Forgets lock surfaces whose surface or connection is gone.
    fn forget_dead_lock_surfaces(&mut self) {
        let dead: Vec<(ClientId, OutputId, bool)> = self
            .lock_tables
            .surfaces
            .iter()
            .filter(|entry| !entry.surface.alive())
            .map(|entry| (entry.locker, entry.output, entry.mapped))
            .collect();
        if dead.is_empty() {
            return;
        }
        self.lock_tables.surfaces.retain(|entry| entry.surface.alive());
        for (locker, output, mapped) in dead {
            if !mapped {
                continue;
            }
            let effects = self.lock.apply(LockEvent::LockerSurfaceUnmapped { locker, output });
            self.apply_lock(effects);
        }
    }

    /// The identity this session uses for a lock object's connection.
    fn lock_client(&mut self, lock: &ExtSessionLockV1) -> Option<ClientId> {
        let client = lock.client()?;
        self.client_identity(&client)
    }

    /// The identity this session uses for a connection, allocated once.
    ///
    /// The lock and the protection covers name a connection the same way, so a
    /// consumer never has to know which of the two hid a client.
    pub(crate) fn client_identity(&mut self, client: &smithay::reexports::wayland_server::Client) -> Option<ClientId> {
        let key = client.id();
        if let Some(token) = self.lock_tables.clients.get(&key) {
            return Some(*token);
        }
        self.lock_tables.next_client = self.lock_tables.next_client.saturating_add(1);
        let token = ClientId::new(self.lock_tables.next_client);
        self.lock_tables.clients.insert(key, token);
        Some(token)
    }

    /// The identity of the output a client named, if this session drives it.
    fn lock_output(&self, output: &WlOutput) -> Option<OutputId> {
        let output = SmithayOutput::from_resource(output)?;
        self.output_id_of(&output)
    }

    /// The logical size a lock surface for this output is configured to.
    fn output_size(&self, output: OutputId) -> Option<Size<u32, Logical>> {
        let rect = self.output_rect(output)?;
        Some(Size::from((rect.width, rect.height)))
    }
}
