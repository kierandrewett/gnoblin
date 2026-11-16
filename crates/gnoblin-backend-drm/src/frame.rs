//! The render-then-flip step of one output, with the kernel and the renderer behind a trait.
//!
//! [`present`] asks the [`Presenter`] what the next frame needs and, when the answer is a
//! draw, draws the frame into the slot it chose and queues that slot's flip. Only a flip the
//! kernel accepted is reported to the presenter as in flight; a draw or a flip the kernel refused
//! leaves the output dirty, so the next vblank retries the same slot. The ordering and the
//! failure rules live here, where a fake can check them without a GPU.

use gnoblin_render_frame::frame::OutputFrame;

use super::present::{FrameStep, Presenter};

/// The most damage rectangles a frame that waits keeps; past it they are one bounding rectangle.
const MOST_WAITING_RECTS: usize = 32;

/// The frame an output was last described and has not yet put on screen.
///
/// The host takes its damage once per iteration and forgets it, so a description the output cannot draw at once
/// (a flip is in flight, or the kernel refused the last one) is the only record of what changed. It is kept until it
/// is drawn: the next iteration's description, when there is one, replaces it and carries its damage; when the host
/// has nothing new to say the output draws the kept one. Without this a retry draws a frame described as empty, which
/// is a black screen, and a supersampled output repaints less than changed.
#[derive(Debug)]
pub(crate) struct UndrawnFrame {
    /// What to draw next, with the damage of every earlier description that was never drawn.
    frame: OutputFrame,
    /// Whether `frame` holds a description that has not been drawn.
    undrawn: bool,
    /// Where the host describes an iteration, so a description that is not made leaves `frame` alone.
    scratch: OutputFrame,
}

impl UndrawnFrame {
    pub(crate) fn new() -> Self {
        Self {
            frame: OutputFrame::new(),
            undrawn: false,
            scratch: OutputFrame::new(),
        }
    }

    /// Asks `describe` for this iteration's frame. Returns whether it described one.
    pub(crate) fn describe(&mut self, describe: impl FnOnce(&mut OutputFrame) -> bool) -> bool {
        if !describe(&mut self.scratch) {
            return false;
        }
        if self.undrawn {
            for rect in self.frame.damage() {
                self.scratch.push_damage(*rect);
            }
            self.scratch.bound_damage(MOST_WAITING_RECTS);
        }
        std::mem::swap(&mut self.frame, &mut self.scratch);
        self.undrawn = true;
        true
    }

    /// The frame to draw.
    pub(crate) fn frame(&self) -> &OutputFrame {
        &self.frame
    }

    /// The frame was drawn and its flip queued, so it is not needed again and its layers are let go.
    pub(crate) fn drawn(&mut self) {
        self.frame.clear();
        self.undrawn = false;
    }
}

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
    use super::{MOST_WAITING_RECTS, Presented, Scanout, UndrawnFrame, present};
    use crate::present::Presenter;
    use gnoblin_render_frame::frame::OutputFrame;
    use gnoblin_types::{Color, Rect};

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
    /// What a scan-out of [`OutputFrame`]s was asked to draw: the slot, the background and the damage of each frame.
    #[derive(Default)]
    struct Screen {
        drawn: Vec<(usize, Color, Vec<Rect>)>,
        /// Flips to refuse before accepting one.
        refuse_flips: usize,
    }

    impl Scanout for Screen {
        type Frame = OutputFrame;

        fn draw(&mut self, slot: usize, frame: &OutputFrame) -> Result<(), String> {
            self.drawn.push((slot, frame.background(), frame.damage().to_vec()));
            Ok(())
        }

        fn flip(&mut self, _slot: usize) -> Result<(), String> {
            if self.refuse_flips > 0 {
                self.refuse_flips -= 1;
                return Err("flip refused".to_owned());
            }
            Ok(())
        }
    }

    const RED: Color = Color::new(1.0, 0.0, 0.0, 1.0);
    const GREEN: Color = Color::new(0.0, 1.0, 0.0, 1.0);

    /// One iteration of `Drm::render` for an output whose host describes `frame` (or nothing, for `None`).
    fn iteration(
        presenter: &mut Presenter,
        waiting: &mut UndrawnFrame,
        screen: &mut Screen,
        described: Option<(Color, Rect)>,
    ) -> Presented {
        let host_described = waiting.describe(|frame| {
            let Some((colour, damage)) = described else {
                return false;
            };
            frame.clear();
            frame.set_background(colour);
            frame.push_damage(damage);
            true
        });
        if host_described {
            presenter.mark_dirty();
        }
        let outcome = present(presenter, screen, waiting.frame());
        if matches!(outcome, Presented::Queued { .. }) {
            waiting.drawn();
        }
        outcome
    }

    #[test]
    fn a_change_described_while_a_flip_is_in_flight_is_drawn_when_the_flip_completes() {
        let mut presenter = Presenter::new();
        let mut waiting = UndrawnFrame::new();
        let mut screen = Screen::default();
        let moved = Rect::new(10, 10, 8, 8);
        let moved_again = Rect::new(40, 40, 8, 8);
        assert_eq!(
            iteration(&mut presenter, &mut waiting, &mut screen, Some((RED, moved))),
            Presented::Queued { slot: 0 }
        );
        // The pointer moves again before the first flip is on screen: the output cannot draw it yet.
        assert_eq!(
            iteration(&mut presenter, &mut waiting, &mut screen, Some((GREEN, moved_again))),
            Presented::Waiting
        );
        presenter.vblank();
        // The host has nothing more to say, because it said it already. The output must not draw an empty frame,
        // which is a black screen: it draws the change it was told of.
        assert_eq!(
            iteration(&mut presenter, &mut waiting, &mut screen, None),
            Presented::Queued { slot: 1 }
        );
        assert_eq!(
            screen.drawn.last(),
            Some(&(1, GREEN, vec![moved_again])),
            "the frame the host described while the output was busy is the one drawn"
        );
    }

    #[test]
    fn the_damage_of_a_frame_that_was_never_drawn_is_carried_into_the_next() {
        let mut presenter = Presenter::new();
        let mut waiting = UndrawnFrame::new();
        let mut screen = Screen::default();
        let first = Rect::new(0, 0, 4, 4);
        let second = Rect::new(20, 0, 4, 4);
        let third = Rect::new(40, 0, 4, 4);
        iteration(&mut presenter, &mut waiting, &mut screen, Some((RED, first)));
        // Two descriptions while the flip is in flight: the second must still repaint the first's place.
        iteration(&mut presenter, &mut waiting, &mut screen, Some((GREEN, second)));
        iteration(&mut presenter, &mut waiting, &mut screen, Some((GREEN, third)));
        presenter.vblank();
        iteration(&mut presenter, &mut waiting, &mut screen, None);
        let (_, _, damage) = screen.drawn.last().expect("a second frame");
        assert!(
            damage.contains(&second) && damage.contains(&third),
            "both changes are repainted: {damage:?}"
        );
    }

    #[test]
    fn a_refused_flip_retries_the_frame_that_was_described_not_an_empty_one() {
        let mut presenter = Presenter::new();
        let mut waiting = UndrawnFrame::new();
        let mut screen = Screen {
            refuse_flips: 1,
            ..Screen::default()
        };
        let area = Rect::new(5, 5, 8, 8);
        assert!(matches!(
            iteration(&mut presenter, &mut waiting, &mut screen, Some((RED, area))),
            Presented::Refused { .. }
        ));
        assert_eq!(
            iteration(&mut presenter, &mut waiting, &mut screen, None),
            Presented::Queued { slot: 0 }
        );
        assert_eq!(screen.drawn.last(), Some(&(0, RED, vec![area])));
    }

    #[test]
    fn a_frame_that_was_drawn_is_let_go() {
        let mut presenter = Presenter::new();
        let mut waiting = UndrawnFrame::new();
        let mut screen = Screen::default();
        iteration(
            &mut presenter,
            &mut waiting,
            &mut screen,
            Some((RED, Rect::new(0, 0, 4, 4))),
        );
        assert!(
            waiting.frame().damage().is_empty() && waiting.frame().layers().is_empty(),
            "the description of a frame that is on screen holds no layers and no damage"
        );
    }

    #[test]
    fn the_damage_a_waiting_frame_carries_is_bounded() {
        let mut presenter = Presenter::new();
        let mut waiting = UndrawnFrame::new();
        let mut screen = Screen::default();
        iteration(
            &mut presenter,
            &mut waiting,
            &mut screen,
            Some((RED, Rect::new(0, 0, 4, 4))),
        );
        for index in 0..(MOST_WAITING_RECTS as i32 * 3) {
            iteration(
                &mut presenter,
                &mut waiting,
                &mut screen,
                Some((GREEN, Rect::new(index * 5, 0, 4, 4))),
            );
        }
        let damage = waiting.frame().damage();
        assert!(
            damage.len() <= MOST_WAITING_RECTS,
            "the list stays within its bound: {}",
            damage.len()
        );
        for index in 0..(MOST_WAITING_RECTS as i32 * 3) {
            let change = Rect::new(index * 5, 0, 4, 4);
            assert!(
                damage.iter().any(|rect| rect.union(change) == *rect),
                "the bound still covers every place that changed: {change:?} in {damage:?}"
            );
        }
    }
}
