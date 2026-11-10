//! The frame loop: drives a [`Runtime`] through the OpenXR session states.
//!
//! Each [`FrameLoop::tick`] drains the runtime's pending events, then runs one
//! frame if the session is running. The rules follow the OpenXR specification:
//!
//! - `begin_session` is called once, on `Ready`, and only while the session is
//!   not already begun.
//! - Frames run only in `Synchronized`, `Visible` and `Focused`, and only while
//!   the session is begun. Each frame is `wait_frame`, `begin_frame`,
//!   `locate_head` and `end_frame`, in that order, and the last two always run
//!   so that `should_render: false` frames still pair up.
//! - `end_session` is called once, on `Stopping`, and only while begun.
//! - `Exiting` ends the loop. `LossPending` stops frames, and the loop waits for
//!   `Exiting`.
//!
//! The frame loop submits no composition layers, so `end_frame` carries none.

use std::time::Duration;

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

/// The outcome of one [`FrameLoop::tick`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tick {
    /// The session is not running, so there was no frame.
    Idle,
    /// A frame ran.
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
    pub fn new(runtime: R) -> Self {
        Self {
            runtime,
            state: SessionState::Unknown,
            begun: false,
        }
    }

    /// Processes the pending events, then runs one frame if the session is running.
    pub fn tick(&mut self) -> Result<Tick> {
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
        self.runtime.end_frame(timing.predicted_display_time)?;
        Ok(Tick::Frame(Frame {
            display_time: timing.predicted_display_time,
            should_render: timing.should_render,
            head,
        }))
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

    fn running(&self) -> bool {
        self.begun
            && matches!(
                self.state,
                SessionState::Synchronized | SessionState::Visible | SessionState::Focused
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
        ]));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert_eq!(frames.runtime.calls, vec![Call::BeginSession]);
    }

    #[test]
    fn renders_no_frame_before_synchronized() {
        let mut frames = FrameLoop::new(FakeRuntime::new([state(SessionState::Ready), timing(10, true)]));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert_eq!(frames.runtime.calls, vec![Call::BeginSession]);
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
        ]));
        assert_eq!(frames.tick(), Ok(Tick::Idle));
        assert_eq!(
            frames.runtime.calls,
            vec![Call::BeginSession, Call::EndSession, Call::BeginSession]
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
