//! The per-output frame state machine of the hardware path.
//!
//! An output owns a small swapchain of buffers. This module decides which buffer the next
//! frame is drawn into and when a new flip may be queued. It holds no kernel handles, so the
//! rules that matter for a display that must never wedge are tested without a device. The
//! hardware path reports each kernel call's outcome back here.
//!
//! # Transitions
//!
//! The output is in one of three states. `Clean` has nothing in flight and the screen is
//! current. `Dirty` has content the screen lacks; `failing` is set while the last flip was
//! refused. `InFlight` has a flip queued; `dirty` is set if content changed since it was queued.
//! Every method consumes the state it is in and moves to the next one. A row marked *refused*
//! is a call the frame path must not make: it leaves the state unchanged.
//!
//! | State    | Event               | Next state                      | Returns                       |
//! |----------|---------------------|---------------------------------|-------------------------------|
//! | Clean    | `step`              | Clean                           | `Idle`                        |
//! | Dirty    | `step`              | Dirty                           | `Draw` into a slot not `front` |
//! | InFlight | `step`              | InFlight                        | `Wait`                        |
//! | Clean    | `mark_dirty`        | Dirty, not failing              |                               |
//! | Dirty    | `mark_dirty`        | Dirty                           |                               |
//! | InFlight | `mark_dirty`        | InFlight, dirty                 |                               |
//! | Clean    | `flip_submitted`    | InFlight, clean                 |                               |
//! | Dirty    | `flip_submitted`    | InFlight, clean                 |                               |
//! | InFlight | `flip_submitted`    | *refused*, unchanged            |                               |
//! | Clean    | `flip_failed`       | Dirty, failing                  | `true`                        |
//! | Dirty    | `flip_failed`       | Dirty, failing                  | `true` unless already failing |
//! | InFlight | `flip_failed`       | *refused*, unchanged            | `false`                       |
//! | InFlight | `vblank`, clean     | Clean, front = the flip's slot  | `true`                        |
//! | InFlight | `vblank`, dirty     | Dirty, front = the flip's slot  | `true`                        |
//! | Clean    | `vblank`            | unchanged                       | `false`                       |
//! | Dirty    | `vblank`            | unchanged                       | `false`                       |
//! | any      | `resume`            | Dirty, no front, not failing    |                               |
//!
//! The guarantees this table gives the frame path:
//!
//! 1. **At most one frame is in flight per output.** `step` answers [`FrameStep::Wait`] for
//!    as long as the state is `InFlight`.
//! 2. **A buffer that is on screen is never drawn into.** A `Draw` never names `front`.
//! 3. **A refused flip keeps the output dirty.** The next `step` draws again into the same
//!    slot. The failure is logged once per streak: [`Presenter::flip_failed`] says whether it is
//!    the first.
//! 4. **Resume starts from nothing.** Nothing is known to be on screen afterwards, and a vblank
//!    from before the switch is ignored.

/// The number of buffers an output cycles through. Two is the double buffer a flip needs: one
/// slot is scanned out while the other is drawn into.
pub const SLOTS: usize = 2;

/// What the output should do with the next frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameStep {
    /// Nothing changed since the last frame on screen; draw nothing.
    Idle,
    /// Draw the next frame into `slot`, then report the flip with [`Presenter::flip_submitted`]
    /// or [`Presenter::flip_failed`].
    Draw {
        /// The buffer to draw into.
        slot: usize,
    },
    /// A frame is already in flight; wait for its vblank.
    Wait,
}

/// The state of one output's swapchain. Every variant is a row of the table in the module doc.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PresenterState {
    /// Nothing is in flight and the screen shows the latest content.
    Clean {
        /// The slot scanned out now.
        front: Option<usize>,
    },
    /// The screen lacks content. `failing` is set while the last flip was refused.
    Dirty {
        /// The slot scanned out now.
        front: Option<usize>,
        /// The last flip was refused and no flip has succeeded since.
        failing: bool,
    },
    /// A flip of `slot` is queued and not yet completed.
    InFlight {
        /// The slot scanned out before this flip completes.
        front: Option<usize>,
        /// The slot whose flip is queued.
        slot: usize,
        /// The output has content that changed since the flip was queued.
        dirty: bool,
    },
}

/// The state of one output's swapchain, as far as the frame path is concerned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Presenter {
    state: PresenterState,
}

impl Default for Presenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Presenter {
    /// A presenter for an output with nothing on screen: the first frame is always drawn.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: PresenterState::Dirty {
                front: None,
                failing: false,
            },
        }
    }

    /// The slot on screen, once a flip has completed.
    #[must_use]
    pub fn front(&self) -> Option<usize> {
        match self.state {
            PresenterState::Clean { front }
            | PresenterState::Dirty { front, .. }
            | PresenterState::InFlight { front, .. } => front,
        }
    }

    /// The slot whose flip is in flight, if any.
    #[must_use]
    pub fn pending(&self) -> Option<usize> {
        match self.state {
            PresenterState::InFlight { slot, .. } => Some(slot),
            PresenterState::Clean { .. } | PresenterState::Dirty { .. } => None,
        }
    }

    /// Whether the output has content the screen does not show.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        matches!(
            self.state,
            PresenterState::Dirty { .. } | PresenterState::InFlight { dirty: true, .. }
        )
    }

    /// The host changed what the output shows.
    pub fn mark_dirty(&mut self) {
        self.state = match self.state {
            PresenterState::Clean { front } => PresenterState::Dirty { front, failing: false },
            PresenterState::InFlight { front, slot, .. } => PresenterState::InFlight {
                front,
                slot,
                dirty: true,
            },
            dirty @ PresenterState::Dirty { .. } => dirty,
        };
    }

    /// What the next frame needs. A frame in flight always wins: nothing is drawn over a buffer
    /// the kernel is still scanning out or about to.
    #[must_use]
    pub fn step(&self) -> FrameStep {
        match self.state {
            PresenterState::Clean { .. } => FrameStep::Idle,
            PresenterState::InFlight { .. } => FrameStep::Wait,
            PresenterState::Dirty { front, .. } => {
                // Two slots, so the one that is not on screen is always free to draw into.
                // With no front yet (first frame, or after resume) the search returns slot 0.
                let slot = (0..SLOTS).find(|slot| Some(*slot) != front).unwrap_or(0);
                FrameStep::Draw { slot }
            }
        }
    }

    /// The kernel accepted a flip of `slot`. The output is clean until the next change, and the
    /// slot stays in flight until its vblank. A second submit while one is in flight is refused
    /// and changes nothing.
    pub fn flip_submitted(&mut self, slot: usize) {
        // A second flip on one CRTC is not accepted by the kernel until the first completes,
        // so the state refuses it here instead of queuing a flip that would be rejected.
        self.state = match self.state {
            PresenterState::Clean { front } | PresenterState::Dirty { front, .. } => PresenterState::InFlight {
                front,
                slot,
                dirty: false,
            },
            in_flight @ PresenterState::InFlight { .. } => in_flight,
        };
    }

    /// The kernel refused a flip. The output stays dirty so the next [`Presenter::step`] retries.
    ///
    /// Returns `true` for the first refusal of a streak, which is the one the caller logs; the
    /// rest of the streak is retried without another line. A refusal while a flip is in flight is
    /// refused: nothing was drawn for it, so it changes nothing and returns `false`.
    pub fn flip_failed(&mut self) -> bool {
        // A refused flip drew into a slot that is not on screen, so the old front is still
        // valid. The output stays dirty and the next step picks the same slot again.
        match self.state {
            PresenterState::Clean { front } => {
                self.state = PresenterState::Dirty { front, failing: true };
                true
            }
            PresenterState::Dirty { front, failing } => {
                self.state = PresenterState::Dirty { front, failing: true };
                !failing
            }
            PresenterState::InFlight { .. } => false,
        }
    }

    /// A vblank arrived. It completes the flip in flight, if there is one; the slot it showed
    /// becomes the front. A vblank with nothing in flight changes nothing.
    ///
    /// Returns whether a flip completed.
    pub fn vblank(&mut self) -> bool {
        // A vblank with nothing queued is stale: it can arrive after a resume abandoned the
        // flip it belonged to. Moving the front on it would show a buffer nobody drew.
        match self.state {
            PresenterState::InFlight { slot, dirty, .. } => {
                self.state = if dirty {
                    PresenterState::Dirty {
                        front: Some(slot),
                        failing: false,
                    }
                } else {
                    PresenterState::Clean { front: Some(slot) }
                };
                true
            }
            PresenterState::Clean { .. } | PresenterState::Dirty { .. } => false,
        }
    }

    /// The seat came back. Nothing is on screen any more as far as this output knows, and any
    /// flip queued before the switch is abandoned, so the output redraws in full.
    pub fn resume(&mut self) {
        // After a seat switch the kernel may have reset the CRTC, so the presenter cannot know
        // which buffer is scanned out. Forgetting the front forces a full redraw.
        self.state = PresenterState::Dirty {
            front: None,
            failing: false,
        };
    }
}

/// One output's presenter, with the CRTC that scans it out and the output's identity.
///
/// A device reports a completed flip by CRTC, so the presenter is found from that handle. `C` is
/// the CRTC type and `I` the output identity: the hardware path instantiates them with the kernel
/// handle and [`gnoblin_types::OutputId`], and the tests use integers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputPresenter<C, I> {
    /// The output this presenter draws for.
    pub id: I,
    /// The CRTC that scans the output out; a flip completion names it.
    pub crtc: C,
    /// The output's swapchain state.
    pub presenter: Presenter,
}

/// Delivers the CRTCs whose flips completed, and returns the outputs whose flip completed with them.
///
/// A completion for a CRTC no channel drives is ignored, as is one for a channel with nothing in
/// flight. A second completion for the same CRTC in one call finds nothing in flight and changes
/// nothing.
pub fn complete_flips<C: PartialEq, I: Copy>(channels: &mut [OutputPresenter<C, I>], crtcs: &[C]) -> Vec<I> {
    let mut completed = Vec::new();
    for crtc in crtcs {
        let Some(channel) = channels.iter_mut().find(|channel| channel.crtc == *crtc) else {
            continue;
        };
        if channel.presenter.vblank() {
            completed.push(channel.id);
        }
    }
    completed
}

#[cfg(test)]
mod tests {
    use super::{FrameStep, OutputPresenter, Presenter, PresenterState, complete_flips};
    /// A channel driving `crtc` for output `id`, with nothing in flight.
    fn channel(id: u32, crtc: u32) -> OutputPresenter<u32, u32> {
        OutputPresenter {
            id,
            crtc,
            presenter: Presenter::new(),
        }
    }

    #[test]
    fn a_completed_flip_is_reported_for_the_output_it_drives() {
        let mut channels = [channel(1, 10), channel(2, 20)];
        channels[1].presenter.flip_submitted(0);
        assert_eq!(complete_flips(&mut channels, &[20]), vec![2]);
        assert_eq!(channels[1].presenter.front(), Some(0));
        assert_eq!(channels[0].presenter.front(), None, "the other output is untouched");
    }

    #[test]
    fn a_completion_for_a_crtc_no_output_drives_changes_nothing() {
        let mut channels = [channel(1, 10)];
        channels[0].presenter.flip_submitted(0);
        assert!(complete_flips(&mut channels, &[99]).is_empty());
        assert_eq!(channels[0].presenter.pending(), Some(0), "the flip is still in flight");
    }

    #[test]
    fn a_completion_with_no_flip_in_flight_is_not_reported() {
        let mut channels = [channel(1, 10)];
        assert!(complete_flips(&mut channels, &[10]).is_empty());
        assert_eq!(channels[0].presenter.front(), None);
    }

    #[test]
    fn two_completions_for_one_crtc_in_one_call_advance_the_output_once() {
        let mut channels = [channel(1, 10)];
        channels[0].presenter.flip_submitted(0);
        assert_eq!(complete_flips(&mut channels, &[10, 10]), vec![1]);
        assert_eq!(channels[0].presenter.front(), Some(0));
    }

    #[test]
    fn a_resume_discards_the_flip_in_flight_so_its_completion_is_ignored() {
        let mut channels = [channel(1, 10)];
        channels[0].presenter.flip_submitted(0);
        channels[0].presenter.resume();
        assert!(complete_flips(&mut channels, &[10]).is_empty());
        assert_eq!(channels[0].presenter.front(), None);
    }

    /// Draws and submits one frame, asserting the slot it was drawn into.
    fn present(presenter: &mut Presenter, expected: usize) {
        assert_eq!(presenter.step(), FrameStep::Draw { slot: expected });
        presenter.flip_submitted(expected);
    }

    #[test]
    fn a_fresh_output_draws_its_first_frame_into_slot_zero() {
        let presenter = Presenter::new();
        assert!(presenter.is_dirty());
        assert_eq!(presenter.step(), FrameStep::Draw { slot: 0 });
    }

    #[test]
    fn at_most_one_frame_is_in_flight_however_often_the_host_changes() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.mark_dirty();
        presenter.mark_dirty();
        assert_eq!(presenter.step(), FrameStep::Wait);
        assert_eq!(presenter.pending(), Some(0));
    }

    #[test]
    fn a_completed_flip_makes_its_slot_the_front_and_the_next_frame_uses_the_other() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        assert!(presenter.vblank());
        assert_eq!(presenter.front(), Some(0));
        assert_eq!(presenter.step(), FrameStep::Idle, "a clean output draws nothing");
        presenter.mark_dirty();
        present(&mut presenter, 1);
        assert!(presenter.vblank());
        presenter.mark_dirty();
        present(&mut presenter, 0);
    }

    #[test]
    fn the_slot_on_screen_is_never_drawn_into() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.vblank();
        for _ in 0..5 {
            presenter.mark_dirty();
            let FrameStep::Draw { slot } = presenter.step() else {
                panic!("a dirty idle output draws");
            };
            assert_ne!(Some(slot), presenter.front());
            presenter.flip_submitted(slot);
            presenter.vblank();
        }
    }

    #[test]
    fn a_vblank_with_nothing_in_flight_changes_nothing() {
        let mut presenter = Presenter::new();
        assert!(!presenter.vblank());
        assert_eq!(presenter.front(), None);
        present(&mut presenter, 0);
        presenter.vblank();
        assert!(!presenter.vblank(), "the second vblank has no flip to complete");
        assert_eq!(presenter.front(), Some(0));
    }

    #[test]
    fn a_refused_flip_stays_dirty_and_is_retried_into_the_same_slot() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.vblank();
        presenter.mark_dirty();
        assert_eq!(presenter.step(), FrameStep::Draw { slot: 1 });
        assert!(presenter.flip_failed(), "the first refusal is logged");
        assert_eq!(presenter.pending(), None, "a refused flip is not in flight");
        assert_eq!(presenter.front(), Some(0), "the screen still shows the old frame");
        assert_eq!(presenter.step(), FrameStep::Draw { slot: 1 }, "retried, not wedged");
    }

    #[test]
    fn a_streak_of_refusals_is_logged_once_and_a_success_ends_it() {
        let mut presenter = Presenter::new();
        assert!(presenter.flip_failed());
        assert!(!presenter.flip_failed());
        assert!(!presenter.flip_failed());
        present(&mut presenter, 0);
        presenter.vblank();
        presenter.mark_dirty();
        assert!(presenter.flip_failed(), "a new streak after a success is logged again");
    }

    #[test]
    fn a_resume_redraws_in_full_and_forgets_the_flip_in_flight() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.vblank();
        presenter.mark_dirty();
        present(&mut presenter, 1);
        presenter.resume();
        assert_eq!(presenter.front(), None);
        assert_eq!(presenter.pending(), None);
        assert!(presenter.is_dirty());
        assert_eq!(presenter.step(), FrameStep::Draw { slot: 0 });
    }

    #[test]
    fn a_vblank_from_before_a_resume_does_not_advance_the_front() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.resume();
        assert!(!presenter.vblank(), "the stale vblank completes nothing");
        assert_eq!(presenter.front(), None);
    }

    #[test]
    fn a_resume_clears_a_refusal_streak() {
        let mut presenter = Presenter::new();
        assert!(presenter.flip_failed());
        presenter.resume();
        assert!(presenter.flip_failed(), "a streak that began before the resume is over");
    }

    #[test]
    fn a_clean_output_steps_to_idle_and_stays_clean() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.vblank();
        assert_eq!(presenter.step(), FrameStep::Idle);
        assert_eq!(presenter.state, PresenterState::Clean { front: Some(0) });
    }

    #[test]
    fn marking_a_clean_output_dirty_makes_it_dirty_and_not_failing() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.vblank();
        presenter.mark_dirty();
        assert_eq!(
            presenter.state,
            PresenterState::Dirty {
                front: Some(0),
                failing: false
            }
        );
    }

    #[test]
    fn marking_an_in_flight_output_dirty_keeps_the_flip_and_records_the_change() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.mark_dirty();
        assert_eq!(
            presenter.state,
            PresenterState::InFlight {
                front: None,
                slot: 0,
                dirty: true
            }
        );
        assert_eq!(presenter.step(), FrameStep::Wait);
    }

    #[test]
    fn a_submitted_flip_from_dirty_leaves_the_output_in_flight_and_clean() {
        let mut presenter = Presenter::new();
        presenter.flip_submitted(1);
        assert_eq!(
            presenter.state,
            PresenterState::InFlight {
                front: None,
                slot: 1,
                dirty: false
            }
        );
    }

    #[test]
    fn a_vblank_with_no_change_since_the_flip_makes_the_output_clean() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        assert!(presenter.vblank());
        assert_eq!(presenter.state, PresenterState::Clean { front: Some(0) });
    }

    #[test]
    fn a_vblank_after_a_change_during_the_flip_leaves_the_output_dirty() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.mark_dirty();
        assert!(presenter.vblank());
        assert_eq!(
            presenter.state,
            PresenterState::Dirty {
                front: Some(0),
                failing: false
            }
        );
        assert_eq!(presenter.step(), FrameStep::Draw { slot: 1 });
    }

    #[test]
    fn a_refused_flip_from_a_clean_output_makes_it_failing_and_dirty() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        presenter.vblank();
        assert!(presenter.flip_failed());
        assert_eq!(
            presenter.state,
            PresenterState::Dirty {
                front: Some(0),
                failing: true
            }
        );
    }

    #[test]
    fn a_refused_flip_while_in_flight_is_refused_and_changes_nothing() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        let before = presenter.clone();
        assert!(!presenter.flip_failed());
        assert_eq!(presenter, before);
    }

    #[test]
    fn a_second_submit_while_in_flight_is_refused_and_changes_nothing() {
        let mut presenter = Presenter::new();
        present(&mut presenter, 0);
        let before = presenter.clone();
        presenter.flip_submitted(1);
        assert_eq!(presenter, before);
        assert_eq!(presenter.pending(), Some(0));
    }

    #[test]
    fn a_vblank_on_an_output_with_nothing_in_flight_is_refused_and_changes_nothing() {
        let mut presenter = Presenter::new();
        let before = presenter.clone();
        assert!(!presenter.vblank());
        assert_eq!(presenter, before);
        presenter.vblank();
        presenter.mark_dirty();
        let clean_dirty = presenter.clone();
        assert!(!presenter.vblank());
        assert_eq!(presenter, clean_dirty);
    }

    #[test]
    fn resume_from_every_state_makes_the_output_dirty_with_nothing_on_screen() {
        let expected = PresenterState::Dirty {
            front: None,
            failing: false,
        };
        let mut clean = Presenter::new();
        present(&mut clean, 0);
        clean.vblank();
        clean.resume();
        assert_eq!(clean.state, expected);

        let mut dirty = Presenter::new();
        dirty.flip_failed();
        dirty.resume();
        assert_eq!(dirty.state, expected);

        let mut in_flight = Presenter::new();
        present(&mut in_flight, 0);
        in_flight.mark_dirty();
        in_flight.resume();
        assert_eq!(in_flight.state, expected);
    }
}
