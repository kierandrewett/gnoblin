//! The headless backend: a virtual output, no DRM, no GPU, no session.
//!
//! This is the backend CI and the integration tests run. It exists so protocol
//! behaviour can be exercised end to end — a real client, a real socket, the real
//! handlers — on a machine with no display, and so the compositor has one backend
//! that starts anywhere.
//!
//! It presents no pixels: no renderer is installed, so [`NullTarget::present`]
//! reports [`Present::Nothing`] and only counts the frames it was asked for. What
//! the target does keep is the clipped damage of each present, which is what a test
//! asserts on when it cares that the compositor asked for the right region.
//!
//! The virtual output's size is settable, which is how a test drives a mode change
//! and how the parity harness will run a session at a fixed size.

use std::collections::VecDeque;
use std::io;

use gnoblin_types::{OutputId, Rect, Scale, Size};
use smithay::reexports::calloop::LoopHandle;

use gnoblin_output::Mode;

use crate::{Backend, BackendError, BackendEvent, Host, InputEvent, InputSource, Output, Present, Target};

/// The size a fresh headless session starts at.
///
/// Large enough that the window manager's default 800x600 window fits with room
/// for placement, and small enough that a test's screenshots and damage lists stay
/// readable.
pub const DEFAULT_MODE: Mode = Mode::new(1024, 768, 60_000);

/// The connector name the first virtual output gets.
///
/// It is a real connector-shaped name rather than a made-up one so that a
/// configuration document can address it and a log line reads like a session.
pub const DEFAULT_CONNECTOR: &str = "HEADLESS-1";

/// A target that presents nothing and remembers what it was asked to present.
///
/// The damage it keeps is clipped to its own extent, so a test can prove that a
/// window hanging off the output produced damage inside the output and nothing
/// outside it, which is the property the software renderer will rely on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NullTarget {
    size: Size,
    scale: Scale,
    presents: u64,
    damage: Vec<Rect>,
}

impl NullTarget {
    /// A target of the given logical size and scale.
    #[must_use]
    pub fn new(size: Size, scale: Scale) -> Self {
        Self {
            size,
            scale,
            presents: 0,
            damage: Vec::new(),
        }
    }

    /// How many frames the target was asked to present.
    #[must_use]
    pub fn presents(&self) -> u64 {
        self.presents
    }

    /// The clipped damage of the last present, in the target's logical space.
    #[must_use]
    pub fn damage(&self) -> &[Rect] {
        &self.damage
    }

    /// The target's whole extent as a rectangle, for clipping.
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.size.width, self.size.height)
    }

    /// Replaces the target's extent after a mode change.
    fn resize(&mut self, size: Size, scale: Scale) {
        self.size = size;
        self.scale = scale;
    }
}

impl Target for NullTarget {
    fn size(&self) -> Size {
        self.size
    }

    fn scale(&self) -> Scale {
        self.scale
    }

    fn present(&mut self, damage: &[Rect]) -> Present {
        self.presents = self.presents.saturating_add(1);
        self.damage.clear();
        let bounds = self.bounds();
        self.damage
            .extend(damage.iter().filter_map(|rect| rect.intersection(bounds)));
        Present::Nothing
    }
}

/// The input queue a headless session delivers from.
///
/// A test pushes events into it and the compositor drains it through
/// [`Backend::dispatch`], so the synthetic path is the same path a libinput device
/// will take. The queue belongs to the backend and not to an output, so events
/// survive every output going away.
#[derive(Clone, Default, Debug)]
pub struct HeadlessInput {
    queue: VecDeque<InputEvent>,
}

impl HeadlessInput {
    /// Queues an event for the next dispatch.
    pub fn push(&mut self, event: InputEvent) {
        self.queue.push_back(event);
    }

    /// How many events are waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether no event is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl InputSource for HeadlessInput {
    fn next_event(&mut self) -> Option<InputEvent> {
        self.queue.pop_front()
    }
}

/// A headless backend: the outputs it drives and the events it has to report.
#[derive(Debug)]
pub struct Headless {
    /// The outputs, and the target for each one, kept in step by every method that
    /// changes them. They are parallel vectors rather than a map so that
    /// [`Backend::outputs`] can hand out a slice in connection order.
    outputs: Vec<Output>,
    targets: Vec<NullTarget>,
    /// State changes waiting for the next dispatch, at most one per output so a
    /// resize storm cannot queue stale sizes.
    pending: Vec<BackendEvent>,
    input: HeadlessInput,
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

impl Headless {
    /// A backend with one virtual output at [`DEFAULT_MODE`].
    #[must_use]
    pub fn new() -> Self {
        let mut backend = Self::empty();
        backend.push_output(DEFAULT_CONNECTOR, DEFAULT_MODE, Scale::integral(1));
        backend
    }

    /// A backend with no outputs.
    ///
    /// A compositor started like this has no monitor to place a window on, which is
    /// the case the window manager refuses with `Rejection::NoOutput`; starting is
    /// still allowed, because an output can arrive later.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            outputs: Vec::new(),
            targets: Vec::new(),
            pending: Vec::new(),
            input: HeadlessInput::default(),
        }
    }

    /// Adds an output and its target.
    ///
    /// # Panics
    ///
    /// Panics if `mode` covers no pixels. The constructor is for callers that have a
    /// size to hand; a runtime request goes through [`Self::resize`], which reports
    /// the same problem as [`BackendError::DegenerateMode`].
    pub fn push_output(&mut self, connector: &str, mode: Mode, scale: Scale) {
        assert!(
            mode.width > 0 && mode.height > 0,
            "a mode that covers no pixels cannot be an output: {mode:?}"
        );
        let id = OutputId::new(self.outputs.len() as u64);
        let output = Output::new(id, connector, mode, scale);
        let mut target = NullTarget::new(output.logical_size(), scale);
        // A new output starts with a full-extent frame pending, so the first
        // dispatch after a start-up or a hotplug reports it like any other change.
        target.present(&[target.bounds()]);
        self.targets.push(target);
        self.pending.push(BackendEvent::OutputConnected(output.clone()));
        self.outputs.push(output);
    }

    /// Sets the mode and scale of an output, reporting the change on the next
    /// dispatch.
    ///
    /// # Errors
    ///
    /// [`BackendError::UnknownOutput`] when the id is not this backend's, and
    /// [`BackendError::DegenerateMode`] when the mode covers no pixels.
    pub fn resize(&mut self, output: OutputId, mode: Mode, scale: Scale) -> Result<(), BackendError> {
        let Some(index) = self.outputs.iter().position(|entry| entry.id == output) else {
            return Err(BackendError::UnknownOutput(output));
        };
        if mode.width == 0 || mode.height == 0 {
            return Err(BackendError::DegenerateMode {
                width: mode.width,
                height: mode.height,
            });
        }
        let entry = &mut self.outputs[index];
        entry.mode = mode;
        entry.scale = scale;
        let size = entry.logical_size();
        self.targets[index].resize(size, scale);
        // One pending change per output, so the latest size wins and a storm of
        // resizes cannot deliver a queue of sizes that are already gone.
        self.pending.retain(|event| {
            !matches!(
                event,
                BackendEvent::OutputResized { output: changed, .. } if *changed == output
            )
        });
        self.pending.push(BackendEvent::OutputResized { output, mode, scale });
        Ok(())
    }

    /// Queues an input event for the next dispatch.
    pub fn push_input(&mut self, event: InputEvent) {
        self.input.push(event);
    }

    /// The target for an output, for a test that wants the damage it recorded.
    #[must_use]
    pub fn target_of(&self, output: OutputId) -> Option<&NullTarget> {
        self.index(output).and_then(|index| self.targets.get(index))
    }

    /// The logical size of an output.
    #[must_use]
    pub fn logical_size(&self, output: OutputId) -> Option<Size> {
        self.outputs
            .iter()
            .find(|entry| entry.id == output)
            .map(Output::logical_size)
    }

    /// The position of an output in the backend's table.
    fn index(&self, output: OutputId) -> Option<usize> {
        self.outputs.iter().position(|entry| entry.id == output)
    }
}

impl<S: Host + 'static> Backend<S> for Headless {
    fn name(&self) -> &'static str {
        "headless"
    }

    fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.iter().find(|entry| entry.id == id)
    }

    fn target(&mut self, id: OutputId) -> Option<&mut dyn Target> {
        let index = self.index(id)?;
        self.targets.get_mut(index).map(|target| target as &mut dyn Target)
    }

    fn input(&mut self) -> &mut dyn InputSource {
        &mut self.input
    }

    fn register(&mut self, _loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        // Nothing to wait on: a headless session is driven by its clients, and the
        // synthetic input queue is filled by the caller.
        Ok(())
    }

    fn render(&mut self, host: &mut S, output: OutputId) -> Result<Present, BackendError> {
        let Some(index) = self.index(output) else {
            return Err(BackendError::UnknownOutput(output));
        };
        let Some(target) = self.targets.get_mut(index) else {
            return Err(BackendError::UnknownOutput(output));
        };
        let before = target.presents();
        host.render(output, target);
        if target.presents() == before {
            // The contract says `render` presents a frame; a host that drew without
            // presenting gets an empty present here rather than a frame that is
            // silently dropped.
            return Ok(target.present(&[]));
        }
        Ok(Present::Nothing)
    }

    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent> {
        while let Some(event) = self.input.next_event() {
            host.input(event);
        }
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_types::Point;

    use super::*;

    /// A host that records what the backend asked it to do.
    #[derive(Default)]
    struct Recorder {
        renders: Vec<OutputId>,
        inputs: Vec<InputEvent>,
    }

    impl Host for Recorder {
        fn render(&mut self, output: OutputId, _target: &mut dyn Target) {
            self.renders.push(output);
        }

        fn input(&mut self, event: InputEvent) {
            self.inputs.push(event);
        }
    }

    /// A host that presents the whole target itself, as the real compositor does.
    #[derive(Default)]
    struct Presenter {
        presents: Vec<Present>,
    }

    impl Host for Presenter {
        fn render(&mut self, _output: OutputId, target: &mut dyn Target) {
            let size = target.size();
            let full = Rect::new(0, 0, size.width, size.height);
            self.presents.push(target.present(&[full]));
        }

        fn input(&mut self, _event: InputEvent) {}
    }

    #[test]
    fn rendering_an_unknown_output_is_refused() {
        let mut backend = Headless::new();
        let mut host = Recorder::default();
        let error = backend.render(&mut host, OutputId::new(9)).expect_err("unknown output");
        assert_eq!(error, BackendError::UnknownOutput(OutputId::new(9)));
        assert!(host.renders.is_empty(), "the host was asked to draw a gone output");
    }

    #[test]
    fn a_resize_storm_delivers_one_event_with_the_latest_size() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        for step in 1..=100 {
            let mode = Mode::new(640 + step, 480 + step, 60_000);
            backend.resize(output, mode, Scale::integral(1)).expect("mode is valid");
        }
        let mut host = Recorder::default();
        let events = backend.dispatch(&mut host);
        let resizes: Vec<&BackendEvent> = events
            .iter()
            .filter(|event| matches!(event, BackendEvent::OutputResized { .. }))
            .collect();
        assert_eq!(resizes.len(), 1, "one change per output, got {resizes:#?}");
        assert_eq!(
            *resizes[0],
            BackendEvent::OutputResized {
                output,
                mode: Mode::new(740, 580, 60_000),
                scale: Scale::integral(1),
            }
        );
        assert_eq!(backend.logical_size(output), Some(Size::new(740, 580)));
    }

    #[test]
    fn a_degenerate_mode_is_refused() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        let error = backend
            .resize(output, Mode::new(0, 480, 60_000), Scale::integral(1))
            .expect_err("a mode with no pixels");
        assert_eq!(error, BackendError::DegenerateMode { width: 0, height: 480 });
        assert_eq!(
            backend.logical_size(output),
            Some(Size::new(1024, 768)),
            "the output kept its mode"
        );
    }

    #[test]
    fn damage_outside_the_target_is_clipped() {
        let mut target = NullTarget::new(Size::new(1024, 768), Scale::integral(1));
        let damage = [
            Rect::new(-100, -100, 200, 200),
            Rect::new(1000, 700, 500, 500),
            Rect::new(5000, 5000, 10, 10),
        ];
        assert_eq!(target.present(&damage), Present::Nothing);
        assert_eq!(
            target.damage(),
            &[Rect::new(0, 0, 100, 100), Rect::new(1000, 700, 24, 68)],
            "the damage inside the target is kept and the rest is dropped"
        );
    }

    #[test]
    fn a_backend_with_no_outputs_renders_nothing() {
        let mut backend = Headless::empty();
        assert!(backend.outputs.is_empty());
        let mut host = Recorder::default();
        for raw in 0..4 {
            let error = backend
                .render(&mut host, OutputId::new(raw))
                .expect_err("there is no output to draw on");
            assert_eq!(error, BackendError::UnknownOutput(OutputId::new(raw)));
        }
        assert!(
            host.renders.is_empty(),
            "the host drew on an output that does not exist"
        );
        assert!(backend.dispatch(&mut host).is_empty());
    }

    #[test]
    fn queued_input_reaches_the_host_with_no_output_connected() {
        let mut backend = Headless::empty();
        backend.push_input(InputEvent::PointerLeave);
        backend.push_input(InputEvent::Key {
            keycode: 30,
            pressed: true,
        });
        let mut host = Recorder::default();
        assert!(backend.dispatch(&mut host).is_empty(), "no output, so no state change");
        assert_eq!(
            host.inputs,
            vec![
                InputEvent::PointerLeave,
                InputEvent::Key {
                    keycode: 30,
                    pressed: true,
                },
            ]
        );
    }

    #[test]
    fn a_headless_present_reports_that_it_drew_nothing() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        let mut host = Recorder::default();
        assert_eq!(
            backend.render(&mut host, output),
            Ok(Present::Nothing),
            "the headless target presents no pixels"
        );
        assert_eq!(host.renders, vec![output]);
        let target = backend.target_of(output).expect("the output has a target");
        assert_eq!(target.presents(), 2, "the start-up frame and the render");
        assert_eq!(target.size(), Size::new(1024, 768));
    }

    #[test]
    fn a_host_that_presents_itself_is_not_presented_twice() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        let mut host = Presenter::default();
        backend.render(&mut host, output).expect("the output exists");
        assert_eq!(host.presents, vec![Present::Nothing]);
        assert_eq!(
            backend.target_of(output).map(NullTarget::presents),
            Some(2),
            "the host's own present is the frame, not an extra one"
        );
    }

    #[test]
    fn a_started_backend_reports_its_output_once() {
        let mut backend = Headless::new();
        let mut host = Recorder::default();
        let events = backend.dispatch(&mut host);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], BackendEvent::OutputConnected(_)));
        assert!(backend.dispatch(&mut host).is_empty(), "a change is reported once");
    }

    #[test]
    fn a_pointer_position_converts_to_the_outputs_logical_space() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        backend.push_input(InputEvent::PointerMotion {
            output,
            position: Point::new(10, 20),
        });
        let mut host = Recorder::default();
        backend.dispatch(&mut host);
        assert_eq!(
            host.inputs,
            vec![InputEvent::PointerMotion {
                output,
                position: Point::new(10, 20),
            }]
        );
    }
}
