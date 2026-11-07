//! The render-then-flip step of one output, with the kernel and the renderer behind a trait.
//!
//! [`present`] asks the [`Presenter`] what the next frame needs and, when the answer is a
//! draw, draws the frame into the slot it chose and queues that slot's flip. Only a flip the
//! kernel accepted is reported to the presenter as in flight; a draw or a flip the kernel refused
//! leaves the output dirty, so the next vblank retries the same slot. The ordering and the
//! failure rules live here, where a fake can check them without a GPU.

use super::present::{FrameStep, Presenter};

/// What one output's scan-out needs from the device: draw a frame into a slot, then queue that
/// slot's flip.
pub(crate) trait Scanout {
    /// The frame the host produced for this output.
    type Frame;

    /// Draws `frame` into buffer `slot`.
    ///
    /// # Errors
    ///
    /// The reason the frame could not be drawn, for the log.
    fn draw(&mut self, slot: usize, frame: &Self::Frame) -> Result<(), String>;

    /// Queues a flip that scans `slot` out.
    ///
    /// # Errors
    ///
    /// The reason the kernel refused the flip, for the log.
    fn flip(&mut self, slot: usize) -> Result<(), String>;
}

/// What [`present`] did for one output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Presented {
    /// Nothing changed since the last frame on screen.
    Idle,
    /// A frame is in flight; the output waits for its vblank.
    Waiting,
    /// The frame was drawn into `slot` and its flip accepted; it is in flight now.
    Queued {
        /// The buffer scanned out next.
        slot: usize,
    },
    /// The draw or the flip failed. The output stays dirty and the next call retries.
    Refused {
        /// The buffer that was being drawn.
        slot: usize,
        /// Whether this is the first refusal of a streak; the caller logs only that one.
        first: bool,
        /// The reason, for the log.
        error: String,
    },
}

/// Draws `frame` into the buffer the presenter chooses and queues its flip, once.
///
/// Draws before it flips, and never draws while a flip is in flight.
pub(crate) fn present<S: Scanout>(presenter: &mut Presenter, scanout: &mut S, frame: &S::Frame) -> Presented {
    match presenter.step() {
        FrameStep::Idle => Presented::Idle,
        FrameStep::Wait => Presented::Waiting,
        FrameStep::Draw { slot } => match scanout.draw(slot, frame).and_then(|()| scanout.flip(slot)) {
            Ok(()) => {
                presenter.flip_submitted(slot);
                Presented::Queued { slot }
            }
            Err(error) => {
                let first = presenter.flip_failed();
                Presented::Refused { slot, first, error }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Presented, Scanout, present};
    use crate::present::Presenter;

    /// Records every call in order, and refuses the calls listed in `fail`.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        /// Each entry fails one call: `"draw"` or `"flip"`, consumed in order.
        fail: Vec<&'static str>,
    }

    impl Fake {
        fn refuse(&mut self, what: &'static str) -> Result<(), String> {
            if self.fail.first() == Some(&what) {
                self.fail.remove(0);
                Err(format!("{what} refused"))
            } else {
                Ok(())
            }
        }
    }

    impl Scanout for Fake {
        type Frame = u32;

        fn draw(&mut self, slot: usize, frame: &u32) -> Result<(), String> {
            self.calls.push(format!("draw {slot} {frame}"));
            self.refuse("draw")
        }

        fn flip(&mut self, slot: usize) -> Result<(), String> {
            self.calls.push(format!("flip {slot}"));
            self.refuse("flip")
        }
    }

    #[test]
    fn a_frame_is_drawn_before_its_flip_is_queued() {
        let mut presenter = Presenter::new();
        let mut fake = Fake::default();
        assert_eq!(present(&mut presenter, &mut fake, &1), Presented::Queued { slot: 0 });
        assert_eq!(fake.calls, ["draw 0 1", "flip 0"]);
        assert_eq!(presenter.pending(), Some(0));
    }

    #[test]
    fn nothing_is_drawn_while_a_flip_is_in_flight() {
        let mut presenter = Presenter::new();
        let mut fake = Fake::default();
        present(&mut presenter, &mut fake, &1);
        presenter.mark_dirty();
        assert_eq!(present(&mut presenter, &mut fake, &2), Presented::Waiting);
        assert_eq!(
            fake.calls,
            ["draw 0 1", "flip 0"],
            "the second frame waits for the vblank"
        );
        presenter.vblank();
        assert_eq!(present(&mut presenter, &mut fake, &2), Presented::Queued { slot: 1 });
        assert_eq!(fake.calls.last().map(String::as_str), Some("flip 1"));
    }

    #[test]
    fn an_unchanged_output_draws_nothing() {
        let mut presenter = Presenter::new();
        let mut fake = Fake::default();
        present(&mut presenter, &mut fake, &1);
        presenter.vblank();
        assert_eq!(present(&mut presenter, &mut fake, &1), Presented::Idle);
        assert_eq!(fake.calls.len(), 2);
    }

    #[test]
    fn a_refused_flip_is_retried_on_the_same_slot_and_logged_once() {
        let mut presenter = Presenter::new();
        let mut fake = Fake {
            fail: vec!["flip", "flip"],
            ..Fake::default()
        };
        assert_eq!(
            present(&mut presenter, &mut fake, &1),
            Presented::Refused {
                slot: 0,
                first: true,
                error: "flip refused".to_owned(),
            }
        );
        assert!(presenter.is_dirty(), "a refused flip keeps the output dirty");
        assert_eq!(
            present(&mut presenter, &mut fake, &1),
            Presented::Refused {
                slot: 0,
                first: false,
                error: "flip refused".to_owned(),
            }
        );
        assert_eq!(present(&mut presenter, &mut fake, &1), Presented::Queued { slot: 0 });
        assert_eq!(
            fake.calls,
            ["draw 0 1", "flip 0", "draw 0 1", "flip 0", "draw 0 1", "flip 0"]
        );
    }

    #[test]
    fn a_refused_draw_queues_no_flip() {
        let mut presenter = Presenter::new();
        let mut fake = Fake {
            fail: vec!["draw"],
            ..Fake::default()
        };
        assert!(matches!(
            present(&mut presenter, &mut fake, &1),
            Presented::Refused {
                slot: 0,
                first: true,
                ..
            }
        ));
        assert_eq!(
            fake.calls,
            ["draw 0 1"],
            "no flip is queued for a frame that was not drawn"
        );
        assert_eq!(presenter.pending(), None);
    }

    #[test]
    fn a_resume_redraws_the_output_in_full() {
        let mut presenter = Presenter::new();
        let mut fake = Fake::default();
        present(&mut presenter, &mut fake, &1);
        presenter.vblank();
        presenter.resume();
        assert_eq!(present(&mut presenter, &mut fake, &1), Presented::Queued { slot: 0 });
        assert_eq!(fake.calls.len(), 4, "the frame is drawn again after the switch");
    }
}
