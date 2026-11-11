//! The frame loop: drives a [`Runtime`] through the OpenXR session states.
//!
//! Each [`FrameLoop::tick`] drains the runtime's pending events, then runs one
//! frame if the session is running. The rules follow the OpenXR specification:
//!
//! - `begin_session` is called once, on `Ready`, and only while the session is
//!   not already begun.
//! - Frames run only in `Ready` (once begun), `Synchronized`, `Visible` and `Focused`, and only while
//!   the session is begun. Each frame is `wait_frame`, `begin_frame`,
//!   `locate_head` and `end_frame`, in that order, and the last two always run
//!   so that `should_render: false` frames still pair up.
//! - `end_session` is called once, on `Stopping`, and only while begun.
//! - `Exiting` ends the loop. `LossPending` stops frames, and the loop waits for
//!   `Exiting`.
//!
//! [`FrameLoop::tick`] submits no composition layers. A caller that has layers splits the frame in two: [`FrameLoop::start`]
//! runs the frame up to `locate_head`, and [`FrameLoop::finish`] ends it with the layers. A frame the application
//! is told not to render ends with none.

use std::time::Duration;

use xr_input::ControllerState;
use xr_layers::SubmittedLayer;
use xr_scene::Pose;

use crate::runtime::{Result, Runtime, SessionEvent, SessionState};

/// How long [`FrameLoop::run`] sleeps when there is no frame to render.
const IDLE_INTERVAL: Duration = Duration::from_millis(10);

/// A frame that the loop has begun and ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// The predicted display time of the frame.
    pub display_time: i64,
    /// Whether the application should render this frame.
    pub should_render: bool,
    /// The head pose at `display_time`, if the runtime tracks the head.
    pub head: Option<Pose>,
}

/// The outcome of one [`FrameLoop::tick`] or [`FrameLoop::start`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tick {
    /// The session is not running, so there was no frame.
    Idle,
    /// A frame ran. From [`FrameLoop::start`] it is begun and not ended: the caller must call [`FrameLoop::finish`].
    Frame(Frame),
    /// The runtime asked the application to exit.
    Exit,
}

/// The OpenXR session state machine and frame pacing, over a [`Runtime`].
pub struct FrameLoop<R: Runtime> {
    runtime: R,
    state: SessionState,
    begun: bool,
}

impl<R: Runtime> FrameLoop<R> {
    /// The controller at `display_time`: its pointing ray and its inputs. Called by the caller
    /// alongside the frame, not inside [`FrameLoop::tick`], so frame pacing is unchanged.
    ///
    /// # Errors
    ///
    /// The runtime fails to locate the controller.
    pub fn locate_controller(&mut self, display_time: i64) -> Result<ControllerState> {
        self.runtime.locate_controller(display_time)
    }

    pub fn new(runtime: R) -> Self {
        Self {
            runtime,
            state: SessionState::Unknown,
            begun: false,
        }
    }

    /// The runtime, read only.
    pub fn runtime(&self) -> &R {
        &self.runtime
    }

    /// The runtime, for the calls that are not part of the frame: swapchains and view location.
    pub fn runtime_mut(&mut self) -> &mut R {
        &mut self.runtime
    }

    /// Processes the pending events, then runs one frame with no layers if the session is running.
    ///
    /// # Errors
    ///
    /// The runtime fails.
    pub fn tick(&mut self) -> Result<Tick> {
        let started = self.start()?;
        if let Tick::Frame(frame) = &started {
            self.finish(frame, &[])?;
        }
        Ok(started)
    }

    /// Processes the pending events, then waits for a frame and begins it if the session is running. A
    /// [`Tick::Frame`] result must be ended with [`FrameLoop::finish`].
    ///
    /// # Errors
    ///
    /// The runtime fails.
    pub fn start(&mut self) -> Result<Tick> {
        while let Some(event) = self.runtime.poll_event()? {
            let SessionEvent::StateChanged(state) = event;
            self.state = state;
            match state {
                SessionState::Ready if !self.begun => {
                    self.runtime.begin_session()?;
                    self.begun = true;
                }
                SessionState::Stopping if self.begun => {
                    self.runtime.end_session()?;
                    self.begun = false;
                }
                _ => {}
            }
        }

        if self.state == SessionState::Exiting {
            return Ok(Tick::Exit);
        }
        if !self.running() {
            return Ok(Tick::Idle);
        }

        let timing = self.runtime.wait_frame()?;
        self.runtime.begin_frame()?;
        let head = self.runtime.locate_head(timing.predicted_display_time)?;
        Ok(Tick::Frame(Frame {
            display_time: timing.predicted_display_time,
            should_render: timing.should_render,
            head,
        }))
    }

    /// Ends a frame [`FrameLoop::start`] began, with `layers`. A frame the application should not render ends with
    /// no layers whatever `layers` holds.
    ///
    /// # Errors
    ///
    /// The runtime fails.
    pub fn finish(&mut self, frame: &Frame, layers: &[SubmittedLayer]) -> Result<()> {
        let layers = if frame.should_render { layers } else { &[] };
        self.runtime.end_frame(frame.display_time, layers)
    }

    /// Ticks until the runtime asks the application to exit, calling `on_frame`
    /// for each frame. Sleeps briefly on idle ticks so that it does not spin.
    pub fn run(&mut self, mut on_frame: impl FnMut(&Frame)) -> Result<()> {
        loop {
            match self.tick()? {
                Tick::Idle => std::thread::sleep(IDLE_INTERVAL),
                Tick::Frame(frame) => on_frame(&frame),
                Tick::Exit => return Ok(()),
            }
        }
    }

    /// Whether a frame may run. The frame loop starts as soon as the session is begun: `Ready` counts, because Monado
    /// sends `Synchronized` only after the first `xrWaitFrame` (checked on Monado `7336a90`), so waiting for it would
    /// never run a frame.
    fn running(&self) -> bool {
        self.begun
            && matches!(
                self.state,
                SessionState::Ready | SessionState::Synchronized | SessionState::Visible | SessionState::Focused
            )
    }
}

#[cfg(test)]
mod tests {
    use xr_scene::IDENTITY;

    use super::*;
    use crate::runtime::{FrameTiming, RuntimeError};
    use crate::testing::{Call, FakeRuntime, Step};

    fn state(state: SessionState) -> Step {
        Step::Event(SessionEvent::StateChanged(state))
    }

    fn timing(display_time: i64, should_render: bool) -> Step {
        Step::Frame(FrameTiming {
            predicted_display_time: display_time,
            should_render,
        })
    }

    fn head() -> Pose {
        Pose {
            position: [0.0, 1.6, 0.0],
            orientation: IDENTITY,
        }
    }

    #[test]
    fn idle_with_nothing_pending() {
        let mut frames = FrameLoop::new(FakeRuntime::new(Vec::new()));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert!(frames.runtime.calls.is_empty());
    }

    #[test]
    fn begins_session_once_on_ready() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Idle),
            state(SessionState::Ready),
            state(SessionState::Ready),
            timing(10, true),
        ]));
        assert!(matches!(frames.tick(), Ok(Tick::Frame(_))));
        assert_eq!(frames.runtime.count(Call::BeginSession), 1);
    }

    #[test]
    fn runs_the_first_frame_in_ready_without_waiting_for_synchronized() {
        // Monado sends `Synchronized` only after the first `xrWaitFrame`, so a begun session in `Ready` must run.
        let mut frames = FrameLoop::new(FakeRuntime::new([state(SessionState::Ready), timing(10, true)]));
        assert!(matches!(frames.tick(), Ok(Tick::Frame(_))));
        assert_eq!(
            frames.runtime.calls,
            vec![
                Call::BeginSession,
                Call::WaitFrame,
                Call::BeginFrame,
                Call::LocateHead(10),
                Call::EndFrame(10),
            ]
        );
    }

    #[test]
    fn runs_no_frame_before_ready() {
        let mut frames = FrameLoop::new(FakeRuntime::new([state(SessionState::Idle), timing(10, true)]));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert!(frames.runtime.calls.is_empty());
    }

    #[test]
    fn runs_paired_frames_while_synchronized() {
        let mut runtime = FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            timing(10, true),
            timing(20, false),
        ]);
        runtime.head = Some(head());
        let mut frames = FrameLoop::new(runtime);

        assert_eq!(
            frames.tick(),
            Ok(Tick::Frame(Frame {
                display_time: 10,
                should_render: true,
                head: Some(head()),
            }))
        );
        assert_eq!(
            frames.runtime.calls,
            vec![
                Call::BeginSession,
                Call::WaitFrame,
                Call::BeginFrame,
                Call::LocateHead(10),
                Call::EndFrame(10),
            ]
        );

        // A frame the application is told not to render is still begun and ended.
        assert_eq!(
            frames.tick(),
            Ok(Tick::Frame(Frame {
                display_time: 20,
                should_render: false,
                head: Some(head()),
            }))
        );
        assert_eq!(
            frames.runtime.calls[5..],
            [
                Call::WaitFrame,
                Call::BeginFrame,
                Call::LocateHead(20),
                Call::EndFrame(20),
            ]
        );
    }

    fn quad_layer() -> SubmittedLayer {
        SubmittedLayer::Quad(xr_layers::QuadLayer {
            view: 1,
            swapchain: xr_layers::SwapchainId(1),
            space: xr_scene::Space::World,
            pose: head(),
            size: [1.0, 1.0],
            eye_visibility: xr_layers::EyeVisibility::Both,
            rect: xr_layers::ImageRect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            color_scale: None,
        })
    }

    #[test]
    fn finish_ends_the_frame_with_the_layers() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            timing(10, true),
        ]));
        let Ok(Tick::Frame(frame)) = frames.start() else {
            panic!("a frame starts");
        };
        assert_eq!(
            frames.runtime.count(Call::EndFrame(10)),
            0,
            "start does not end the frame"
        );
        frames.finish(&frame, &[quad_layer()]).expect("finishes");
        assert_eq!(frames.runtime.submitted, vec![(10, vec![quad_layer()])]);
    }

    #[test]
    fn finish_submits_no_layers_for_a_frame_that_should_not_render() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            timing(10, false),
        ]));
        let Ok(Tick::Frame(frame)) = frames.start() else {
            panic!("a frame starts");
        };
        frames.finish(&frame, &[quad_layer()]).expect("finishes");
        assert_eq!(frames.runtime.submitted, vec![(10, Vec::new())]);
    }

    #[test]
    fn tick_ends_its_frame_with_no_layers() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            timing(10, true),
        ]));
        assert!(matches!(frames.tick(), Ok(Tick::Frame(_))));
        assert_eq!(frames.runtime.submitted, vec![(10, Vec::new())]);
    }

    #[test]
    fn runs_frames_in_every_visible_state() {
        for visible in [SessionState::Synchronized, SessionState::Visible, SessionState::Focused] {
            let mut frames = FrameLoop::new(FakeRuntime::new([
                state(SessionState::Ready),
                state(visible),
                timing(5, true),
            ]));
            assert!(matches!(frames.tick(), Ok(Tick::Frame(_))), "no frame in {visible:?}");
        }
    }

    #[test]
    fn stops_rendering_at_stopping_and_ends_session_once() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            timing(10, true),
            state(SessionState::Stopping),
            timing(20, true),
        ]));
        assert!(matches!(frames.tick(), Ok(Tick::Frame(_))));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert_eq!(frames.runtime.count(Call::EndSession), 1);
        assert_eq!(frames.runtime.count(Call::WaitFrame), 1);
    }

    #[test]
    fn begins_again_after_runtime_returns_to_ready() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            state(SessionState::Stopping),
            state(SessionState::Idle),
            state(SessionState::Ready),
            timing(10, true),
        ]));
        assert!(matches!(frames.tick(), Ok(Tick::Frame(_))));
        assert_eq!(
            frames.runtime.calls[..3],
            [Call::BeginSession, Call::EndSession, Call::BeginSession]
        );
    }

    #[test]
    fn loss_pending_stops_frames_without_ending_session() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            state(SessionState::LossPending),
            timing(10, true),
        ]));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert_eq!(frames.runtime.calls, vec![Call::BeginSession]);
    }

    #[test]
    fn exiting_ends_the_loop_without_frames() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            state(SessionState::Exiting),
            timing(10, true),
        ]));
        assert_eq!(frames.tick(), Ok(Tick::Exit));
        assert_eq!(frames.runtime.calls, vec![Call::BeginSession]);
    }

    #[test]
    fn run_renders_until_exiting() {
        let mut frames = FrameLoop::new(FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            timing(10, true),
            timing(20, false),
            state(SessionState::Stopping),
            state(SessionState::Exiting),
        ]));
        let mut rendered = Vec::new();
        frames.run(|frame| rendered.push(*frame)).expect("run succeeds");
        assert_eq!(
            rendered
                .iter()
                .map(|frame| (frame.display_time, frame.should_render))
                .collect::<Vec<_>>(),
            vec![(10, true), (20, false)]
        );
        assert_eq!(frames.runtime.count(Call::EndSession), 1);
    }

    #[test]
    fn run_returns_runtime_errors() {
        let mut runtime = FakeRuntime::new([
            state(SessionState::Ready),
            state(SessionState::Synchronized),
            timing(10, true),
        ]);
        // The second call is wait_frame.
        runtime.fail_on_call = Some(2);
        let mut frames = FrameLoop::new(runtime);
        let result = frames.run(|_| unreachable!("no frame runs after the error"));
        assert_eq!(result, Err(RuntimeError("injected failure".to_owned())));
    }
}
