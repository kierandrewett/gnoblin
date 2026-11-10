//! A scripted [`Runtime`] for the frame loop tests.

use std::collections::VecDeque;

use xr_scene::Pose;

use crate::runtime::{FrameTiming, Result, Runtime, RuntimeError, SessionEvent};

/// One step of the script: an event the runtime reports, or a frame it wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Event(SessionEvent),
    Frame(FrameTiming),
}

/// A call the frame loop made on the runtime, with its time arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    BeginSession,
    EndSession,
    WaitFrame,
    BeginFrame,
    EndFrame(i64),
    LocateHead(i64),
}

/// Plays a script. `poll_event` returns the `Event` steps at the front of the
/// script; `wait_frame` consumes the next `Frame` step and fails if the script
/// holds an event there instead.
#[derive(Debug, Default)]
pub struct FakeRuntime {
    pub script: VecDeque<Step>,
    pub head: Option<Pose>,
    pub calls: Vec<Call>,
    /// Makes the call that is this many (1-based) calls in fail with
    /// `RuntimeError("injected failure")`. The failing call is still recorded.
    pub fail_on_call: Option<usize>,
}

impl FakeRuntime {
    pub fn new(script: impl IntoIterator<Item = Step>) -> Self {
        Self {
            script: script.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn count(&self, call: Call) -> usize {
        self.calls.iter().filter(|recorded| **recorded == call).count()
    }

    fn record(&mut self, call: Call) -> Result<()> {
        self.calls.push(call);
        if self.fail_on_call == Some(self.calls.len()) {
            return Err(RuntimeError("injected failure".to_owned()));
        }
        Ok(())
    }
}

impl Runtime for FakeRuntime {
    fn poll_event(&mut self) -> Result<Option<SessionEvent>> {
        match self.script.front() {
            Some(&Step::Event(event)) => {
                self.script.pop_front();
                Ok(Some(event))
            }
            _ => Ok(None),
        }
    }

    fn begin_session(&mut self) -> Result<()> {
        self.record(Call::BeginSession)
    }

    fn end_session(&mut self) -> Result<()> {
        self.record(Call::EndSession)
    }

    fn wait_frame(&mut self) -> Result<FrameTiming> {
        self.record(Call::WaitFrame)?;
        match self.script.pop_front() {
            Some(Step::Frame(timing)) => Ok(timing),
            other => Err(RuntimeError(format!("script expected a frame, found {other:?}"))),
        }
    }

    fn begin_frame(&mut self) -> Result<()> {
        self.record(Call::BeginFrame)
    }

    fn end_frame(&mut self, display_time: i64) -> Result<()> {
        self.record(Call::EndFrame(display_time))
    }

    fn locate_head(&mut self, display_time: i64) -> Result<Option<Pose>> {
        self.record(Call::LocateHead(display_time))?;
        Ok(self.head)
    }
}
