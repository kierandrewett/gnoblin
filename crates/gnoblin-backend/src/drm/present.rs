//! The per-output frame state machine of the hardware path.
//!
//! An output owns a small swapchain of buffers. This module decides which buffer the next
//! frame is drawn into and when a new flip may be queued. It holds no kernel handles, so the
//! rules that matter for a display that must never wedge are tested without a device. The
//! hardware path reports each kernel call's outcome back here.
//!
//! # Rules
//!
//! 1. **At most one frame is in flight per output.** A flip is queued only when the previous
//!    one has completed: [`Presenter::step`] answers [`FrameStep::Wait`] until [`Presenter::vblank`]
//!    reports it.
//! 2. **A buffer that is on screen is never drawn into.** The frame to draw goes to a slot
//!    other than the one in [`Presenter::front`].
//! 3. **A refused flip keeps the output dirty.** The next call to [`Presenter::step`] draws again
//!    and retries the same slot, so a flip that failed once cannot leave the output frozen. The
//!    failure is logged once per streak: [`Presenter::flip_failed`] says whether this is the first.
//! 4. **Resume starts from nothing.** After the seat returns, nothing is known to be on screen,
//!    so the output is dirty and a vblank from before the switch is ignored.

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

/// The state of one output's swapchain, as far as the frame path is concerned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Presenter {
    /// The slot scanned out now, once a flip has completed.
    front: Option<usize>,
    /// The slot whose flip is queued and not yet completed.
    pending: Option<usize>,
    /// The output has content the screen does not show yet.
    dirty: bool,
    /// The last flip was refused and no flip has succeeded since.
    failing: bool,
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
            front: None,
            pending: None,
            dirty: true,
            failing: false,
        }
    }

    /// The slot on screen, once a flip has completed.
    #[must_use]
    pub fn front(&self) -> Option<usize> {
        self.front
    }

    /// The slot whose flip is in flight, if any.
    #[must_use]
    pub fn pending(&self) -> Option<usize> {
        self.pending
    }

    /// Whether the output has content the screen does not show.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The host changed what the output shows.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// What the next frame needs. A frame in flight always wins: nothing is drawn over a buffer
    /// the kernel is still scanning out or about to.
    #[must_use]
    pub fn step(&self) -> FrameStep {
        if self.pending.is_some() {
            return FrameStep::Wait;
        }
        if !self.dirty {
            return FrameStep::Idle;
        }
        let slot = (0..SLOTS).find(|slot| Some(*slot) != self.front).unwrap_or(0);
        FrameStep::Draw { slot }
    }

    /// The kernel accepted a flip of `slot`. The output is clean until the next change, and the
    /// slot stays in flight until its vblank.
    pub fn flip_submitted(&mut self, slot: usize) {
        self.pending = Some(slot);
        self.dirty = false;
        self.failing = false;
    }

    /// The kernel refused a flip. The output stays dirty so the next [`Presenter::step`] retries.
    ///
    /// Returns `true` for the first refusal of a streak, which is the one the caller logs; the
    /// rest of the streak is retried without another line.
    pub fn flip_failed(&mut self) -> bool {
        let first = !self.failing;
        self.failing = true;
        self.dirty = true;
        first
    }

    /// A vblank arrived. It completes the flip in flight, if there is one; the slot it showed
    /// becomes the front. A vblank with nothing in flight changes nothing.
    ///
    /// Returns whether a flip completed.
    pub fn vblank(&mut self) -> bool {
        match self.pending.take() {
            Some(slot) => {
                self.front = Some(slot);
                true
            }
            None => false,
        }
    }

    /// The seat came back. Nothing is on screen any more as far as this output knows, and any
    /// flip queued before the switch is abandoned, so the output redraws in full.
    pub fn resume(&mut self) {
        self.front = None;
        self.pending = None;
        self.dirty = true;
        self.failing = false;
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
    use super::{FrameStep, OutputPresenter, Presenter, complete_flips};
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
}
