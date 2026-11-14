//! Changing the mode or the scale of one live output, with the kernel behind a trait.
//!
//! [`configure`] is the order of operations of a mode change and the rule for every way it can fail. It reads
//! no device: [`ModesetKernel`] is what it asks of the kernel, so a fake checks the ordering and the rollback
//! without a GPU, and `scanout::LiveModeset` is the one implementation that talks to smithay.
//!
//! # Order
//!
//! A scale-only change commits nothing: the CRTC keeps its mode blob, the output composes at the new scale
//! from its next frame, and it is marked dirty so that frame repaints everything. A mode change is an atomic
//! modeset, and every step that can fail comes before the step that cannot be undone:
//!
//! 1. the pure decision ([`crate::plan::modeset`]): no pixels, not driven, dark, or a mode the connector does not list
//!    are refused before anything is allocated;
//! 2. buffers for the new size are allocated beside the old ones, which stay in use;
//! 3. an atomic commit with `TEST_ONLY` over the new buffers;
//! 4. the real commit, which scans the new buffers' [`COMMITTED_SLOT`] out;
//! 5. only now the output's buffers are replaced (the old ones are dropped after the commit that stopped
//!    scanning them), its swapchain is reset onto the committed slot, and the gamma table the output had is
//!    written again.
//!
//! # Failure modes first
//!
//! 1. **The kernel refuses the test.** Nothing was committed and nothing was replaced: the new buffers are
//!    dropped, the old ones, the swapchain and any flip in flight are untouched:
//!    `a_kernel_refusal_in_the_test_leaves_the_output_exactly_as_it_was`.
//! 2. **The kernel refuses the commit after the test passed.** The staged mode is put back before the error
//!    is returned, and the output keeps its buffers and its swapchain:
//!    `a_refusal_at_the_commit_restores_the_previous_mode_and_buffers`. A restore that is itself refused is
//!    named in the error rather than hidden.
//! 3. **A flip is in flight.** The blocking commit is ordered after it by the kernel, so the old buffers are
//!    dropped only after the commit returned, and the swapchain abandons that flip and ignores its one
//!    completion report: `a_modeset_over_a_flip_in_flight_abandons_it_and_drops_the_old_buffers_after_the_commit`.
//! 4. **A modeset loses the gamma table.** The table the output had is written again after the commit, and a
//!    refusal to do so is reported with the success, because the mode did change:
//!    `the_gamma_table_is_written_again_after_the_commit`.

use gnoblin_output::DisplayMode;
use gnoblin_render_frame::error::RenderError;
use gnoblin_types::{Scale, Size};

use crate::plan::{Connector, ModesetRefusal, ModesetStep, OutputState, modeset};
use crate::present::Presenter;

/// The slot of a fresh set of buffers that a modeset commit scans out.
///
/// It is the first, because the swapchain's next frame is drawn into the other slot, which is never the one on screen.
pub(crate) const COMMITTED_SLOT: usize = 0;

/// What a mode change asks of the kernel and the buffer allocator.
pub(crate) trait ModesetKernel {
    /// The buffers of one output.
    type Buffers;

    /// Allocates buffers for an output of `size` physical pixels, with [`COMMITTED_SLOT`] cleared so the first frame on
    /// the new mode is black and not whatever memory held.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the buffers cannot be made. The output is untouched.
    fn allocate(&mut self, size: Size) -> Result<Self::Buffers, RenderError>;

    /// An atomic commit with `TEST_ONLY` of `mode` over `buffers`. It stages nothing: on `Ok` and on `Err` the
    /// kernel and the surface are as they were.
    ///
    /// # Errors
    ///
    /// The reason the kernel refused.
    fn test(&mut self, mode: DisplayMode, buffers: &Self::Buffers) -> Result<(), String>;

    /// The real atomic commit: `mode` with [`COMMITTED_SLOT`] of `buffers` scanned out.
    ///
    /// # Errors
    ///
    /// The reason the kernel refused. The mode may still be staged on the surface; [`ModesetKernel::unstage`]
    /// puts it back.
    fn commit(&mut self, mode: DisplayMode, buffers: &Self::Buffers) -> Result<(), String>;

    /// Puts the surface's staged mode back to the mode that was committed last.
    ///
    /// # Errors
    ///
    /// The reason the kernel refused, in which case the surface still holds the mode that failed.
    fn unstage(&mut self) -> Result<(), String>;

    /// Writes the gamma table the output had, which a modeset can lose. An output that never had one is not an error.
    ///
    /// # Errors
    ///
    /// The reason the kernel refused the table.
    fn reapply_gamma(&mut self) -> Result<(), String>;
}

/// What [`configure`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ModesetKind {
    /// The output already drove that mode and scale; nothing was asked of the kernel.
    Unchanged,
    /// Only the scale changed; nothing was committed.
    Scale,
    /// The mode changed through an atomic modeset.
    Mode,
}

/// A mode change that was made.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct ModesetOutcome {
    /// The output as it is now.
    pub(crate) state: OutputState,
    /// What was done to get there.
    pub(crate) kind: ModesetKind,
    /// Why the gamma table could not be written again after the modeset, if it could not. The mode did change.
    pub(crate) gamma_failure: Option<String>,
}

/// Why a mode change was not made. In every case the output is as it was.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum ModesetError {
    /// The change was refused before the kernel was asked.
    Refused(ModesetRefusal),
    /// The buffers for the new size could not be made.
    Buffers(RenderError),
    /// The kernel refused the test-only commit.
    Test(String),
    /// The kernel refused the real commit after the test passed.
    Commit {
        /// What the kernel said.
        reason: String,
        /// Why the previous mode could not be staged again, if it could not. The caller must not trust the surface.
        rollback_failure: Option<String>,
    },
}

impl std::fmt::Display for ModesetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(formatter, "{refusal}"),
            Self::Buffers(error) => write!(formatter, "the buffers for the new mode could not be made: {error}"),
            Self::Test(reason) => write!(formatter, "the kernel refused the test commit: {reason}"),
            Self::Commit {
                reason,
                rollback_failure: None,
            } => write!(formatter, "the kernel refused the commit: {reason}"),
            Self::Commit {
                reason,
                rollback_failure: Some(rollback),
            } => write!(
                formatter,
                "the kernel refused the commit: {reason}; the previous mode could not be staged again: {rollback}"
            ),
        }
    }
}

impl std::error::Error for ModesetError {}

/// Changes one live output from `current` to `mode` and `scale`.
///
/// `buffers` and `presenter` are the output's: on `Err` neither is touched, so the output keeps presenting what it was
/// presenting, a flip in flight included. On `Ok` the buffers are the new ones and the presenter is reset onto them.
///
/// # Errors
///
/// [`ModesetError`] naming the step that refused.
pub(crate) fn configure<K: ModesetKernel>(
    kernel: &mut K,
    connector: &Connector,
    current: &OutputState,
    mode: DisplayMode,
    scale: Scale,
    buffers: &mut Option<K::Buffers>,
    presenter: &mut Presenter,
) -> Result<ModesetOutcome, ModesetError> {
    match modeset(connector, current, mode, scale).map_err(ModesetError::Refused)? {
        ModesetStep::Unchanged => Ok(ModesetOutcome {
            state: *current,
            kind: ModesetKind::Unchanged,
            gamma_failure: None,
        }),
        ModesetStep::Scale(state) => {
            presenter.mark_dirty();
            Ok(ModesetOutcome {
                state,
                kind: ModesetKind::Scale,
                gamma_failure: None,
            })
        }
        ModesetStep::Mode(state) => {
            let fresh = kernel
                .allocate(Size::new(mode.width, mode.height))
                .map_err(ModesetError::Buffers)?;
            kernel.test(mode, &fresh).map_err(ModesetError::Test)?;
            if let Err(reason) = kernel.commit(mode, &fresh) {
                let rollback_failure = kernel.unstage().err();
                return Err(ModesetError::Commit {
                    reason,
                    rollback_failure,
                });
            }
            // The commit is blocking and the kernel orders it after any flip in flight, so the old buffers are no
            // longer scanned out and may be dropped now, and not before.
            presenter.after_modeset(COMMITTED_SLOT);
            *buffers = Some(fresh);
            let gamma_failure = kernel.reapply_gamma().err();
            Ok(ModesetOutcome {
                state,
                kind: ModesetKind::Mode,
                gamma_failure,
            })
        }
    }
}

/// Checks that [`configure`] would be accepted, changing nothing: the pure decision, then buffers for the new size and
/// a test-only commit over them. A scale-only change needs no kernel.
///
/// # Errors
///
/// [`ModesetError`] naming the step that refused.
pub(crate) fn check<K: ModesetKernel>(
    kernel: &mut K,
    connector: &Connector,
    current: &OutputState,
    mode: DisplayMode,
    scale: Scale,
) -> Result<(), ModesetError> {
    match modeset(connector, current, mode, scale).map_err(ModesetError::Refused)? {
        ModesetStep::Unchanged | ModesetStep::Scale(_) => Ok(()),
        ModesetStep::Mode(_) => {
            let fresh = kernel
                .allocate(Size::new(mode.width, mode.height))
                .map_err(ModesetError::Buffers)?;
            kernel.test(mode, &fresh).map_err(ModesetError::Test)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gnoblin_output::VrrSupport;
    use gnoblin_types::{Orientation, OutputId};

    use super::*;
    use crate::frame::{Presented, Scanout, present};
    use crate::plan::PhysicalExtent;
    use crate::present::FrameStep;

    const FULL_HD: DisplayMode = DisplayMode::new(1920, 1080, 60_000);
    const FOUR_K: DisplayMode = DisplayMode::new(3840, 2160, 60_000);
    const UNLISTED: DisplayMode = DisplayMode::new(1280, 720, 60_000);

    type Log = Rc<RefCell<Vec<String>>>;

    /// Buffers that say when they are dropped, so the order against the commit is checked.
    struct Held {
        size: Size,
        log: Log,
    }

    impl Drop for Held {
        fn drop(&mut self) {
            self.log
                .borrow_mut()
                .push(format!("drop {}x{}", self.size.width, self.size.height));
        }
    }

    /// A kernel that records every call and refuses the ones it is told to.
    #[derive(Default)]
    struct Fake {
        log: Log,
        refuse_allocation: bool,
        refuse_test: bool,
        refuse_commit: bool,
        refuse_unstage: bool,
        refuse_gamma: bool,
    }

    impl Fake {
        fn note(&self, line: String) {
            self.log.borrow_mut().push(line);
        }

        fn held(&self, size: Size) -> Held {
            Held {
                size,
                log: Rc::clone(&self.log),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.log.borrow().clone()
        }

        fn clear(&self) {
            self.log.borrow_mut().clear();
        }
    }

    impl ModesetKernel for Fake {
        type Buffers = Held;

        fn allocate(&mut self, size: Size) -> Result<Held, RenderError> {
            self.note(format!("allocate {}x{}", size.width, size.height));
            if self.refuse_allocation {
                return Err(RenderError::Unavailable);
            }
            Ok(self.held(size))
        }

        fn test(&mut self, mode: DisplayMode, buffers: &Held) -> Result<(), String> {
            self.note(format!(
                "test {}x{} over {}",
                mode.width, mode.height, buffers.size.width
            ));
            if self.refuse_test {
                return Err("test-only commit refused".to_owned());
            }
            Ok(())
        }

        fn commit(&mut self, mode: DisplayMode, buffers: &Held) -> Result<(), String> {
            self.note(format!(
                "commit {}x{} over {}",
                mode.width, mode.height, buffers.size.width
            ));
            if self.refuse_commit {
                return Err("commit refused".to_owned());
            }
            Ok(())
        }

        fn unstage(&mut self) -> Result<(), String> {
            self.note("unstage".to_owned());
            if self.refuse_unstage {
                return Err("unstage refused".to_owned());
            }
            Ok(())
        }

        fn reapply_gamma(&mut self) -> Result<(), String> {
            self.note("gamma".to_owned());
            if self.refuse_gamma {
                return Err("gamma refused".to_owned());
            }
            Ok(())
        }
    }

    fn connector(connected: bool) -> Connector {
        Connector {
            id: OutputId::new(1),
            name: "DP-1".to_owned(),
            connected,
            edid: None,
            modes: vec![FULL_HD, FOUR_K],
            physical: PhysicalExtent::default(),
            vrr: VrrSupport::Supported,
            gamma_length: 256,
        }
    }

    fn lit() -> OutputState {
        OutputState {
            vrr: true,
            kelvin: Some(4000),
            ..OutputState::lit(FULL_HD, Scale::integral(1), Orientation::Normal)
        }
    }

    /// An output that has drawn and flipped once: clean, slot 0 on screen, over buffers of its mode.
    fn presenting(fake: &Fake) -> (Option<Held>, Presenter) {
        let mut presenter = Presenter::new();
        presenter.flip_submitted(0);
        presenter.vblank();
        (Some(fake.held(Size::new(1920, 1080))), presenter)
    }

    fn size_of(buffers: &Option<Held>) -> Option<(u32, u32)> {
        buffers.as_ref().map(|held| (held.size.width, held.size.height))
    }

    #[test]
    fn a_mode_the_connector_lists_is_committed_and_the_next_frame_is_presented_at_it() {
        let mut fake = Fake::default();
        let (mut buffers, mut presenter) = presenting(&fake);
        let outcome = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(2),
            &mut buffers,
            &mut presenter,
        )
        .expect("the connector lists 4K");
        assert_eq!(outcome.kind, ModesetKind::Mode);
        assert_eq!(
            (outcome.state.mode, outcome.state.scale),
            (FOUR_K, Scale::integral(2)),
            "the state the backend reports is the new one"
        );
        assert_eq!(
            size_of(&buffers),
            Some((3840, 2160)),
            "the buffers were made for the new size"
        );
        assert_eq!(
            fake.calls(),
            [
                "allocate 3840x2160",
                "test 3840x2160 over 3840",
                "commit 3840x2160 over 3840",
                "drop 1920x1080",
                "gamma"
            ],
            "test before commit, the old buffers dropped after it, the gamma last"
        );

        /// Draws into the buffers the output now has, and says which slot.
        struct Frames(Vec<String>);
        impl Scanout for Frames {
            type Frame = u8;
            fn draw(&mut self, slot: usize, _: &u8) -> Result<(), String> {
                self.0.push(format!("draw {slot}"));
                Ok(())
            }
            fn flip(&mut self, slot: usize) -> Result<(), String> {
                self.0.push(format!("flip {slot}"));
                Ok(())
            }
        }
        let mut frames = Frames(Vec::new());
        assert_eq!(
            present(&mut presenter, &mut frames, &0),
            Presented::Queued { slot: 1 },
            "the next frame repaints in full into the slot that is not on screen"
        );
        assert_eq!(frames.0, ["draw 1", "flip 1"]);
    }

    #[test]
    fn a_mode_the_connector_does_not_list_is_refused_with_no_state_change() {
        let mut fake = Fake::default();
        let (mut buffers, mut presenter) = presenting(&fake);
        let before = presenter.clone();
        fake.clear();
        let error = configure(
            &mut fake,
            &connector(true),
            &lit(),
            UNLISTED,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect_err("not listed");
        assert!(
            matches!(error, ModesetError::Refused(ModesetRefusal::UnsupportedMode { .. })),
            "{error}"
        );
        assert!(fake.calls().is_empty(), "nothing was allocated or asked of the kernel");
        assert_eq!(size_of(&buffers), Some((1920, 1080)));
        assert_eq!(presenter, before);
    }

    #[test]
    fn a_kernel_refusal_in_the_test_leaves_the_output_exactly_as_it_was() {
        let mut fake = Fake {
            refuse_test: true,
            ..Fake::default()
        };
        let (mut buffers, mut presenter) = presenting(&fake);
        // A flip is in flight, so a change that touched the swapchain would show.
        presenter.mark_dirty();
        assert_eq!(presenter.step(), FrameStep::Draw { slot: 1 });
        presenter.flip_submitted(1);
        let before = presenter.clone();
        fake.clear();
        let error = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect_err("the kernel refuses the test");
        assert_eq!(error, ModesetError::Test("test-only commit refused".to_owned()));
        assert_eq!(
            fake.calls(),
            ["allocate 3840x2160", "test 3840x2160 over 3840", "drop 3840x2160"],
            "no commit, and only the new buffers are dropped"
        );
        assert_eq!(
            size_of(&buffers),
            Some((1920, 1080)),
            "the old buffers were not dropped"
        );
        assert_eq!(presenter, before, "the output is still presenting what it was");
        assert_eq!(presenter.pending(), Some(1));
    }

    #[test]
    fn a_refusal_at_the_commit_restores_the_previous_mode_and_buffers() {
        let mut fake = Fake {
            refuse_commit: true,
            ..Fake::default()
        };
        let (mut buffers, mut presenter) = presenting(&fake);
        let before = presenter.clone();
        fake.clear();
        let error = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect_err("the kernel refuses the commit");
        assert_eq!(
            error,
            ModesetError::Commit {
                reason: "commit refused".to_owned(),
                rollback_failure: None
            }
        );
        assert_eq!(
            fake.calls(),
            [
                "allocate 3840x2160",
                "test 3840x2160 over 3840",
                "commit 3840x2160 over 3840",
                "unstage",
                "drop 3840x2160"
            ],
            "the staged mode is put back and the new buffers are the ones dropped"
        );
        assert_eq!(size_of(&buffers), Some((1920, 1080)));
        assert_eq!(presenter, before);
    }

    #[test]
    fn a_rollback_that_is_refused_is_named_in_the_error() {
        let mut fake = Fake {
            refuse_commit: true,
            refuse_unstage: true,
            ..Fake::default()
        };
        let (mut buffers, mut presenter) = presenting(&fake);
        let error = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect_err("refused twice");
        let text = error.to_string();
        assert!(
            text.contains("commit refused") && text.contains("unstage refused"),
            "{text}"
        );
    }

    #[test]
    fn buffers_that_cannot_be_made_refuse_before_the_kernel_is_asked() {
        let mut fake = Fake {
            refuse_allocation: true,
            ..Fake::default()
        };
        let (mut buffers, mut presenter) = presenting(&fake);
        fake.clear();
        let error = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect_err("no buffers");
        assert_eq!(error, ModesetError::Buffers(RenderError::Unavailable));
        assert_eq!(fake.calls(), ["allocate 3840x2160"]);
        assert_eq!(size_of(&buffers), Some((1920, 1080)));
    }

    #[test]
    fn a_scale_alone_commits_nothing_and_repaints_in_full() {
        let mut fake = Fake::default();
        let (mut buffers, mut presenter) = presenting(&fake);
        assert!(!presenter.is_dirty());
        fake.clear();
        let outcome = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FULL_HD,
            Scale::integral(2),
            &mut buffers,
            &mut presenter,
        )
        .expect("a scale");
        assert_eq!(outcome.kind, ModesetKind::Scale);
        assert_eq!(outcome.state.scale, Scale::integral(2));
        assert!(
            fake.calls().is_empty(),
            "no mode blob is changed, so the kernel is not asked"
        );
        assert_eq!(
            size_of(&buffers),
            Some((1920, 1080)),
            "the buffers are the same, at the same size"
        );
        assert!(presenter.is_dirty(), "the next frame repaints");
        assert_eq!(presenter.front(), Some(0), "the buffer on screen is still the front");
    }

    #[test]
    fn the_mode_and_scale_the_output_already_has_ask_nothing() {
        let mut fake = Fake::default();
        let (mut buffers, mut presenter) = presenting(&fake);
        fake.clear();
        let outcome = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FULL_HD,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect("unchanged");
        assert_eq!(outcome.kind, ModesetKind::Unchanged);
        assert!(fake.calls().is_empty());
        assert!(!presenter.is_dirty());
    }

    #[test]
    fn a_dark_output_and_one_without_a_monitor_are_refused() {
        let mut fake = Fake::default();
        let (mut buffers, mut presenter) = presenting(&fake);
        fake.clear();
        let dark = OutputState {
            power: crate::plan::Power::Off,
            ..lit()
        };
        let off = configure(
            &mut fake,
            &connector(true),
            &dark,
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect_err("dark");
        assert!(matches!(off, ModesetError::Refused(ModesetRefusal::Dark(_))), "{off}");
        let undriven = configure(
            &mut fake,
            &connector(false),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect_err("no CRTC");
        assert!(
            matches!(undriven, ModesetError::Refused(ModesetRefusal::NotDriven(_))),
            "{undriven}"
        );
        assert!(fake.calls().is_empty());
        assert_eq!(size_of(&buffers), Some((1920, 1080)));
    }

    #[test]
    fn the_gamma_table_is_written_again_after_the_commit() {
        let mut fake = Fake::default();
        let (mut buffers, mut presenter) = presenting(&fake);
        let outcome = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect("modeset");
        let calls = fake.calls();
        let commit = calls
            .iter()
            .position(|call| call.starts_with("commit"))
            .expect("commit");
        let gamma = calls.iter().position(|call| call == "gamma").expect("gamma");
        assert!(commit < gamma, "{calls:?}");
        assert_eq!(outcome.gamma_failure, None);
        assert!(outcome.state.vrr, "variable refresh survives the modeset");
        assert_eq!(outcome.state.kelvin, Some(4000), "the tint survives the modeset");
    }

    #[test]
    fn a_gamma_table_the_kernel_refuses_does_not_undo_the_mode() {
        let mut fake = Fake {
            refuse_gamma: true,
            ..Fake::default()
        };
        let (mut buffers, mut presenter) = presenting(&fake);
        let outcome = configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect("the mode changed");
        assert_eq!(outcome.gamma_failure.as_deref(), Some("gamma refused"));
        assert_eq!(size_of(&buffers), Some((3840, 2160)), "the new mode stays");
    }

    #[test]
    fn a_modeset_over_a_flip_in_flight_abandons_it_and_drops_the_old_buffers_after_the_commit() {
        let mut fake = Fake::default();
        let (mut buffers, mut presenter) = presenting(&fake);
        presenter.mark_dirty();
        assert_eq!(presenter.step(), FrameStep::Draw { slot: 1 });
        presenter.flip_submitted(1);
        assert_eq!(presenter.pending(), Some(1), "a flip is in flight over the old buffers");
        fake.clear();
        configure(
            &mut fake,
            &connector(true),
            &lit(),
            FOUR_K,
            Scale::integral(1),
            &mut buffers,
            &mut presenter,
        )
        .expect("modeset");
        let calls = fake.calls();
        let commit = calls
            .iter()
            .position(|call| call.starts_with("commit"))
            .expect("commit");
        let dropped = calls
            .iter()
            .position(|call| call == "drop 1920x1080")
            .expect("old buffers dropped");
        assert!(
            commit < dropped,
            "the old buffers are not freed while the kernel may scan them: {calls:?}"
        );
        assert_eq!(presenter.pending(), None, "nothing waits on the abandoned flip");
        assert_eq!(presenter.front(), Some(COMMITTED_SLOT));
        assert_eq!(presenter.abandoned(), 1);
        assert_eq!(
            presenter.step(),
            FrameStep::Draw { slot: 1 },
            "the committed slot is on screen and is not drawn into"
        );
        presenter.flip_submitted(1);
        assert!(!presenter.vblank(), "the abandoned flip's report is ignored");
        assert_eq!(presenter.pending(), Some(1), "the new flip is not completed by it");
    }

    #[test]
    fn a_check_allocates_and_tests_and_commits_nothing() {
        let mut fake = Fake::default();
        check(&mut fake, &connector(true), &lit(), FOUR_K, Scale::integral(1)).expect("would be accepted");
        assert_eq!(
            fake.calls(),
            ["allocate 3840x2160", "test 3840x2160 over 3840", "drop 3840x2160"]
        );
        fake.clear();
        check(&mut fake, &connector(true), &lit(), FULL_HD, Scale::integral(2)).expect("a scale needs no kernel");
        assert!(fake.calls().is_empty());
        let mut refusing = Fake {
            refuse_test: true,
            ..Fake::default()
        };
        assert!(matches!(
            check(&mut refusing, &connector(true), &lit(), FOUR_K, Scale::integral(1)),
            Err(ModesetError::Test(_))
        ));
        assert!(matches!(
            check(&mut refusing, &connector(true), &lit(), UNLISTED, Scale::integral(1)),
            Err(ModesetError::Refused(_))
        ));
    }
}
