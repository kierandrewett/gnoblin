//! The hardware backend: DRM and KMS through udev and libseat, with a GLES or a Vulkan scan-out path.
//!
//! This is the third backend of the Backends card, and the only one that drives a real
//! display. It is split the way the rest of the crate is:
//!
//! | Module       | Owns                                                                | Tested without a device |
//! | ------------ | ------------------------------------------------------------------- | ----------------------- |
//! | [`plan`]     | the output state, the ordered kernel operations, the rollback record | yes                     |
//! | `modeset`    | the order of a mode change and the rule for each way it fails        | yes, on a fake kernel   |
//! | [`gamma`]    | the lookup table and the length check before the ioctl               | yes                     |
//! | crate root (here) | the [`Backend`] implementation: devices, outputs, dispatch         | no                      |
//!
//! # One device, several connectors
//!
//! A GPU is opened once ([`crate::session::Seat`] lends the node), its connectors are the
//! outputs, and each output owns a KMS surface with its own swapchain. A connector that
//! appears becomes [`BackendEvent::OutputConnected`] with the descriptor [`plan::Connector`]
//! derives from its EDID; a connector that disappears becomes
//! [`BackendEvent::OutputDisconnected`]. A GPU that disappears is
//! [`BackendEvent::Disconnected`], because nothing this backend owns survives it.
//!
//! # Mode changes are off until asked for
//!
//! Changing the mode of a live output blanks a real display, and that code has only run against a fake kernel.
//! [`Drm::open`] therefore takes a [`RealModeset`], and while it is off a mode change is refused with
//! [`BackendError::ModesetDisabled`] before the kernel is touched. A scale-only change commits nothing and is not
//! affected. The process turns it on with the environment variable [`MODESET_ENV`] (`GNOBLIN_DRM_MODESET=1`).
//!
//! # Failure modes first
//!
//! The pure failures are in [`plan`] and [`gamma`]. These are the ones only a running
//! session can reach, and each is handled where it happens rather than surfaced as a frame
//! error, because a display that goes away must not stop a session:
//!
//! 1. **A VT switch leaves DRM master held.** [`crate::session::SeatState`] turns the
//!    pause into one transition, the device is paused through Smithay, and no frame is
//!    submitted until its resume: see `session`'s tests and [`Drm::render`].
//! 2. **A resume repaints only part of the screen.** The resume bumps the session's
//!    generation. Presenting an output is not built yet, so no full-output damage is issued
//!    from it; see [`Drm::render`].
//! 3. **A hotplug arrives during a modeset.** The connector event is queued and applied in
//!    the next dispatch, never from inside the commit it interrupted; a failure to create the
//!    surface leaves the output disconnected and the session running.
//! 4. **The GPU is removed mid-frame.** The udev event reports [`plan`]`::DeviceChange::
//!    PrimaryLost`, every output is reported gone and the backend answers
//!    [`BackendEvent::Disconnected`].
//! 5. **A gamma table does not fit.** [`gamma::GammaRamp::matches`] refuses it before the
//!    ioctl and the refusal is logged, so the display keeps the table it had.
//! 6. **A change is described while the output cannot draw it.** The host reports what
//!    changed once per iteration and does not repeat it. An output with a flip in flight, or
//!    one whose flip the kernel refused, keeps the frame it was last given until it is drawn
//!    and carries the damage of every frame it never drew, so the retry draws what changed
//!    and not an empty frame, which is a black screen:
//!    `frame::tests::a_change_described_while_a_flip_is_in_flight_is_drawn_when_the_flip_completes`
//!    and `frame::tests::a_refused_flip_retries_the_frame_that_was_described_not_an_empty_one`.
//! 7. **The Vulkan device is lost.** The output's renderer returns the loss and draws nothing more; the scan-out
//!    buffers and framebuffers are not touched, so the screen keeps the last frame. [`Drm::render`] rebuilds the
//!    renderer on a new device after 0 ms, 100 ms and 1 s, then every 5 s, wakes the loop for each wait, and draws the
//!    output whole when it is back: `vulkan::tests::the_rebuilds_back_off_and_settle`.

/// The DRM column of the backend status table, measured against a fake kernel.
#[cfg(test)]
mod conformance;
/// The render-then-flip step of one output; see the module docs.
pub mod frame;
/// The gamma lookup table and its length check; see the module docs.
pub mod gamma;
/// The GPU behind the hardware backend: the primary node, its connectors and KMS surfaces.
pub mod gpu;
/// The libinput devices: the seat's pointers, touch and keyboards, and how their events map.
pub mod libinput;
/// Changing the mode or scale of a live output: the order of operations and every way it fails.
pub(crate) mod modeset;
pub use modeset::{MODESET_ENV, RealModeset};
/// The pure planning of output state and kernel operations.
pub mod plan;
/// The per-output swapchain state machine: which buffer is drawn and when a flip may queue.
pub mod present;
/// The GBM scan-out buffers of one output and the adapter that draws into them.
pub(crate) mod scanout;
/// The libseat session the GPU and the input devices are opened through.
pub mod session;
/// The udev discovery of GPUs and the connectors they carry.
pub mod udev;
/// The Vulkan renderer's half of the scan-out path: slot damage, loss recovery and the redacted capture.
pub(crate) mod vulkan;

use std::fmt;
use std::io;
use std::time::{Duration, Instant};

use gnoblin_types::{Orientation, OutputId, Scale};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::drm::control::crtc;

use crate::frame::{Presented, UndrawnFrame, present};
use crate::libinput::Libinput;
use crate::libinput::map::Screen;
use crate::modeset::Composition;
use crate::present::{OutputPresenter, Presenter, complete_flips};
use crate::scanout::{Buffers, Device as ScanoutDevice, Flip, LiveModeset};
use crate::session::{SeatEvent, SeatState, Transition};
use crate::vulkan::RebuildPacing;
use gnoblin_backend::render::renderer::RendererKind;
use gnoblin_backend::{
    Backend, BackendError, BackendEvent, DmabufImportError, DmabufSupport, Host, InputEvent, InputSource, Output,
    Present, Target,
};
use gnoblin_render_frame::frame::SecureTreatment;
use gnoblin_render_frame::image::Image;
use std::collections::{HashMap, HashSet};

gnoblin_log::logger!(log = "backend");

/// Why the hardware backend could not start.
///
/// Every variant names the thing that was missing rather than "the backend failed", because
/// the answer is different for each: a machine with no seat cannot run a hardware session at
/// all, a machine with no GPU can run a headless one instead, and a GPU that cannot be opened
/// is a permissions problem a person can fix.
#[derive(Debug)]
pub enum DrmError {
    /// The session has no seat, so no device can be opened through it.
    Session(crate::session::SessionError),
    /// udev could not be reached, or the GPU list could not be read.
    Discovery(String),
    /// No primary GPU could be chosen.
    NoGpu(crate::udev::ChoiceError),
    /// The session's seat is not the one udev was asked about.
    Seat(String),
    /// The GPU could not be opened or initialised.
    Device(String),
    /// The renderer's device on the GPU could not be created, or the renderer cannot scan out.
    Renderer(gnoblin_render_frame::error::RenderError),
}

impl fmt::Display for DrmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(error) => write!(formatter, "{error}"),
            Self::Discovery(reason) => write!(formatter, "no DRM device discovery: {reason}"),
            Self::NoGpu(error) => write!(formatter, "{error}"),
            Self::Seat(reason) => write!(formatter, "{reason}"),
            Self::Device(reason) => write!(formatter, "the GPU could not be used: {reason}"),
            Self::Renderer(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for DrmError {}

/// The hardware backend.
///
/// It exists so that the type is nameable; the session that opens it is `Drm::open`, and the
/// internals it drives are added by [`gpu`]. A session built with no GPU cannot be created at
/// all, which is why [`Drm::open`] returns an error rather than a backend with no outputs: the
/// composition root chooses the headless backend for that case.
pub struct Drm {
    gpu: gpu::Gpu,
    /// The connected connectors, one output each, in the order the kernel lists them.
    outputs: Vec<Output>,
    /// Each connected output's presenter, keyed by the CRTC that scans it out.
    channels: Vec<OutputPresenter<crtc::Handle, OutputId>>,
    /// The GBM device and EGL display every output's scan-out buffers are allocated on.
    scanout: ScanoutDevice,
    /// Each output's scan-out buffers, allocated on its first draw and dropped on resume.
    buffers: HashMap<OutputId, Buffers>,
    /// What each output was last described and has not drawn yet: the host reports a change once, and an output with
    /// a flip in flight cannot draw it at once.
    waiting: HashMap<OutputId, UndrawnFrame>,
    /// The outputs switched off, which a mode change refuses: lighting them is a power change.
    dark: HashSet<OutputId>,
    /// Changes the backend made itself, reported by the next dispatch.
    pending: Vec<BackendEvent>,
    input: Input,
    /// Whether the seat is ours, which decides whether the device may be used.
    seat: SeatState,
    /// Whether the process opted in to changing the mode of a real display. Off, a mode change is refused.
    real_modeset: RealModeset,
    /// What draws the outputs.
    renderer: RendererKind,
    /// What a capture draws in place of a secure layer.
    treatment: SecureTreatment,
    /// How soon a lost Vulkan renderer is rebuilt.
    recovery: RebuildPacing,
    /// When the next rebuild of a lost renderer may happen, while one is waiting.
    retry_at: Option<Instant>,
    /// Wakes the event loop after a delay, so a rebuild that has waited is noticed; set when the backend registers.
    wake: Option<Box<dyn Fn(Duration)>>,
    /// How many frames were queued for a flip since the backend started, logged every 60 at debug level so a run can
    /// tell that frames keep completing.
    flipped: u64,
}

/// Why the hardware backend offers no input events.
///
/// Each variant is a state the input source is in, so a caller that asks for events can tell an
/// empty queue from a source that is not delivering at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputRefusal {
    /// No libinput context is open on the seat yet, so no device can be read.
    NotOpened,
}

impl fmt::Display for InputRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOpened => write!(formatter, "no libinput context is open on the seat"),
        }
    }
}

/// The input source a hardware backend hands out while it has no devices to read.
///
/// It never yields an event; [`Refused::refusal`] says why, and it is the same answer for every
/// call, so a dispatch loop that polls it does nothing rather than panicking.
#[derive(Debug)]
pub struct Refused {
    refusal: InputRefusal,
}

impl Refused {
    /// Why no event is delivered.
    #[must_use]
    pub fn refusal(&self) -> InputRefusal {
        self.refusal
    }
}

impl InputSource for Refused {
    fn next_event(&mut self) -> Option<InputEvent> {
        None
    }
}

/// Where the hardware backend's input comes from.
#[derive(Debug)]
enum Input {
    /// libinput is open on the seat; its devices are read in [`Backend::dispatch`].
    Live(Box<Libinput>),
    /// No libinput context could be opened, so the session has no input; see [`Refused`].
    Refused(Refused),
}

impl InputSource for Input {
    fn next_event(&mut self) -> Option<InputEvent> {
        match self {
            Self::Live(libinput) => libinput.next_event(),
            Self::Refused(refused) => refused.next_event(),
        }
    }
}

impl fmt::Debug for Drm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Drm").field("gpu", &self.gpu).finish()
    }
}

impl Drm {
    /// Opens the hardware session: the seat, the primary GPU, every connector on it, and the
    /// libinput context on the same seat.
    ///
    /// A libinput context that cannot be opened does not fail the session: the backend runs with
    /// no input, and the reason is logged here.
    ///
    /// `real_modeset` is whether mode changes on the real display are allowed; pass
    /// [`RealModeset::from_process_env`] so that they are off unless [`MODESET_ENV`] is set.
    ///
    /// `renderer` is what draws the outputs, [`RendererKind::Gles`] or [`RendererKind::Vulkan`]; the software renderer
    /// cannot scan out, and is refused as [`DrmError::Renderer`]. `treatment` is what a capture draws in place of a
    /// secure layer, which only a Vulkan output can be captured of.
    ///
    /// # Errors
    ///
    /// [`DrmError`] naming what could not be opened. A session that gets this error runs the
    /// headless backend instead; it is not a frame error.
    pub fn open(
        seat_name: &str,
        real_modeset: RealModeset,
        renderer: RendererKind,
        treatment: SecureTreatment,
    ) -> Result<Self, DrmError> {
        let gpu = gpu::Gpu::open(seat_name)?;
        let outputs: Vec<Output> = gpu
            .outputs()
            .filter(|connector| connector.connected)
            .filter_map(output_of)
            .collect();
        let channels = outputs
            .iter()
            .filter_map(|output| {
                let crtc = gpu.crtc_of(output.id)?;
                Some(OutputPresenter {
                    id: output.id,
                    crtc,
                    presenter: Presenter::new(),
                })
            })
            .collect();
        let scanout = ScanoutDevice::open(gpu.device_fd(), renderer).map_err(DrmError::Renderer)?;
        let input = match Libinput::open(gpu.seat().libseat()) {
            Ok(libinput) => Input::Live(Box::new(libinput)),
            Err(error) => {
                log::warn!(area = "input", %error, "libinput is not open; the session runs without input");
                Input::Refused(Refused {
                    refusal: InputRefusal::NotOpened,
                })
            }
        };
        Ok(Self {
            gpu,
            outputs,
            channels,
            scanout,
            buffers: HashMap::new(),
            waiting: HashMap::new(),
            dark: HashSet::new(),
            pending: Vec::new(),
            input,
            seat: SeatState::new(),
            real_modeset,
            renderer,
            treatment,
            recovery: RebuildPacing::default(),
            retry_at: None,
            wake: None,
            flipped: 0,
        })
    }

    /// Each output's logical extent, which is the space libinput states absolute positions in.
    fn screens(&self) -> Vec<Screen> {
        self.outputs
            .iter()
            .map(|output| Screen {
                id: output.id,
                logical: output.logical_size(),
            })
            .collect()
    }

    /// What a mode or scale change on `output` is decided from: its connector as the kernel lists it, the state it is in
    /// now, the index of its swapchain and the CRTC that scans it out.
    ///
    /// # Errors
    ///
    /// [`BackendError::UnknownOutput`] for an output this backend does not drive, and
    /// [`BackendError::Unpresentable`] for one that has no CRTC to present on.
    fn modeset_target(
        &self,
        output: OutputId,
    ) -> Result<(plan::Connector, plan::OutputState, usize, crtc::Handle), BackendError> {
        let known = self
            .outputs
            .iter()
            .find(|known| known.id == output)
            .ok_or(BackendError::UnknownOutput(output))?;
        let connector = self
            .gpu
            .outputs()
            .find(|connector| connector.id == output)
            .cloned()
            .ok_or(BackendError::UnknownOutput(output))?;
        let index = self
            .channels
            .iter()
            .position(|channel| channel.id == output)
            .ok_or(BackendError::Unpresentable(output))?;
        let state = if self.dark.contains(&output) {
            plan::OutputState::dark(known.mode, known.scale, Orientation::Normal)
        } else {
            plan::OutputState::lit(known.mode, known.scale, Orientation::Normal)
        };
        let state = plan::OutputState {
            supersample: known.supersample,
            ..state
        };
        Ok((connector, state, index, self.channels[index].crtc))
    }

    /// Puts a change that was made into what the backend reports: the output's mode and scale, the extent libinput
    /// states absolute positions in, and one [`BackendEvent::OutputResized`] for the next dispatch.
    fn record_reconfigured(&mut self, output: OutputId, state: &plan::OutputState) {
        if let Some(known) = self.outputs.iter_mut().find(|known| known.id == output) {
            known.mode = state.mode;
            known.scale = state.scale;
            known.supersample = state.supersample;
        }
        let screens = self.screens();
        if let Input::Live(libinput) = &mut self.input {
            libinput.set_screens(screens);
        }
        // One pending change per output, so the latest size wins and a storm of changes cannot deliver sizes that
        // are already gone.
        self.pending.retain(
            |event| !matches!(event, BackendEvent::OutputResized { output: changed, .. } if *changed == output),
        );
        self.pending.push(BackendEvent::OutputResized {
            output,
            mode: state.mode,
            scale: state.scale,
            supersample: state.supersample,
        });
    }

    /// The seat was taken away: release the device and stop delivering input.
    ///
    /// A pause for a session that is already paused does nothing, since the device was released
    /// by the first event.
    fn pause(&mut self) {
        if self.seat.paused() == Transition::None {
            return;
        }
        self.gpu.pause();
        if let Input::Live(libinput) = &mut self.input {
            libinput.suspend();
        }
    }

    /// The seat is ours again: take the device back, reset every output's swapchain so it redraws
    /// in full, and deliver input again.
    ///
    /// Drawing the outputs is not built yet (see [`Backend::render`]), so the swapchains are reset
    /// and marked dirty for the frame path that will draw them. A failure to take the device back is
    /// logged; the swapchains and the input are reset regardless, because the seat is ours either way.
    fn resume(&mut self) {
        let Transition::Resume { .. } = self.seat.resumed() else {
            return;
        };
        if let Err(error) = self.gpu.activate() {
            log::error!(area = "drm", %error, "the device could not be re-activated after the seat returned");
        }
        for channel in &mut self.channels {
            channel.presenter.resume();
        }
        self.buffers.clear();
        if let Input::Live(libinput) = &mut self.input {
            libinput.resume();
        }
    }

    /// The primary GPU's removal, once it has happened.
    #[must_use]
    pub fn device_lost(&self) -> Option<&gpu::DeviceLost> {
        self.gpu.device_lost()
    }

    /// Applies one hardware change to the outputs and their presenters, and says what the
    /// compositor is told.
    ///
    /// A refused change is logged by name and leaves the output as it was. Losing the primary GPU
    /// is reported as [`BackendEvent::Disconnected`], and the host decides what that means.
    fn apply_hardware(&mut self, change: gpu::HardwareChange) -> Vec<BackendEvent> {
        match change {
            gpu::HardwareChange::OutputRemoved(id) => {
                self.outputs.retain(|output| output.id != id);
                self.channels.retain(|channel| channel.id != id);
                self.buffers.remove(&id);
                self.waiting.remove(&id);
                self.dark.remove(&id);
                vec![BackendEvent::OutputDisconnected(id)]
            }
            gpu::HardwareChange::OutputAdded(id) => {
                let output = self
                    .gpu
                    .outputs()
                    .find(|connector| connector.id == id)
                    .and_then(output_of);
                let crtc = self.gpu.crtc_of(id);
                let (Some(output), Some(crtc)) = (output, crtc) else {
                    log::warn!(area = "drm", output = %id, "an added output has no mode or CRTC and is not presented");
                    return Vec::new();
                };
                self.outputs.push(output.clone());
                self.channels.push(OutputPresenter {
                    id,
                    crtc,
                    presenter: Presenter::new(),
                });
                vec![BackendEvent::OutputConnected(output)]
            }
            gpu::HardwareChange::DeviceLost(lost) => {
                log::warn!(area = "drm", %lost, "the primary GPU is gone; the backend reports it disconnected");
                vec![BackendEvent::Disconnected]
            }
            gpu::HardwareChange::Refused { connector, reason } => {
                log::warn!(area = "drm", %connector, %reason, "a hotplug change was refused");
                Vec::new()
            }
        }
    }
}

impl<S: Host + 'static> Backend<S> for Drm {
    fn name(&self) -> &'static str {
        "drm"
    }

    fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.iter().find(|output| output.id == id)
    }

    fn target(&mut self, _id: OutputId) -> Option<&mut dyn Target> {
        None
    }

    fn input(&mut self) -> &mut dyn InputSource {
        &mut self.input
    }

    fn set_input_settings(
        &mut self,
        input: &gnoblin_config::Input,
    ) -> Result<(), gnoblin_backend::InputSettingsRefusal> {
        match &mut self.input {
            Input::Live(libinput) => {
                libinput.set_input_settings(input);
                Ok(())
            }
            // No libinput context is open, so there is no device to configure.
            Input::Refused(_) => Err(gnoblin_backend::InputSettingsRefusal::NoDevices),
        }
    }

    fn register(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        self.gpu.register(loop_handle)?;
        let screens = self.screens();
        if let Input::Live(libinput) = &mut self.input {
            libinput.set_screens(screens);
            libinput.register(loop_handle)?;
        }
        // A lost renderer is rebuilt when its backoff has passed, which only a turn of the loop notices: a timer
        // that does nothing wakes the loop for it.
        let handle = loop_handle.clone();
        self.wake = Some(Box::new(move |after| {
            let timer = Timer::from_duration(after);
            if let Err(error) = handle.insert_source(timer, |_, _, _| TimeoutAction::Drop) {
                log::warn!(area = "render", %error, "the loop could not be woken for the renderer's rebuild");
            }
        }));
        Ok(())
    }

    fn render(&mut self, host: &mut S, output: OutputId) -> Result<Present, BackendError> {
        let Some(known) = self.outputs.iter().find(|known| known.id == output) else {
            return Err(BackendError::UnknownOutput(output));
        };
        let (scale, size, composed) = (known.effective_scale(), known.mode, known.virtual_size());
        let logical = known.logical_size();
        let Some(index) = self.channels.iter().position(|channel| channel.id == output) else {
            return Err(BackendError::Unpresentable(output));
        };
        let crtc = self.channels[index].crtc;
        // The host is asked on every iteration, and what it describes is kept until it is drawn: an output with a
        // flip in flight, or one the kernel refused, cannot draw it now, and the host will not say it again.
        let waiting = self.waiting.entry(output).or_insert_with(UndrawnFrame::new);
        if waiting.describe(|frame| host.frame(output, frame)) {
            self.channels[index].presenter.mark_dirty();
        }
        let physical = gnoblin_types::Size::new(size.width, size.height);
        if self
            .buffers
            .get(&output)
            .is_some_and(|buffers| !buffers.fits(physical, composed))
        {
            self.buffers.remove(&output);
        }
        let buffers = match self.buffers.entry(output) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let formats = self
                    .gpu
                    .plane_formats(crtc)
                    .ok_or(BackendError::Unpresentable(output))?;
                let buffers =
                    Buffers::new(&mut self.scanout, physical, composed, &formats).map_err(BackendError::Render)?;
                entry.insert(buffers)
            }
        };
        if buffers.is_lost() {
            // The device went away under this output. The screen keeps the last frame (the scan-out buffers are not
            // touched) until a renderer is made on a device again, paced by `RebuildPacing`.
            let now = Instant::now();
            if self.retry_at.is_some_and(|at| now < at) {
                return Ok(Present::Nothing);
            }
            match buffers.rebuild(&mut self.scanout) {
                Ok(()) => {
                    log::info!(area = "render", output = %output, "the Vulkan renderer was rebuilt after the device was lost");
                    self.retry_at = None;
                    // The new target holds no pixels: the output is drawn whole.
                    waiting
                        .frame_mut()
                        .push_damage(gnoblin_types::Rect::new(0, 0, logical.width, logical.height));
                    self.channels[index].presenter.mark_dirty();
                }
                Err(error) => {
                    let wait = self.recovery.failed(now);
                    self.retry_at = Some(now + wait);
                    log::warn!(area = "render", output = %output, %error, retry_ms = wait.as_millis(), "the Vulkan renderer could not be rebuilt; the output keeps its last frame");
                    if let Some(wake) = &self.wake {
                        wake(wait);
                    }
                    return Ok(Present::Nothing);
                }
            }
        }
        let mut flip = Flip {
            buffers,
            gpu: &self.gpu,
            crtc,
            scale,
        };
        match present(&mut self.channels[index].presenter, &mut flip, waiting.frame()) {
            Presented::Idle | Presented::Waiting => Ok(Present::Nothing),
            Presented::Queued { .. } => {
                self.flipped += 1;
                if self.flipped.is_multiple_of(60) {
                    log::debug!(area = "drm", output = %output, flips = self.flipped, "frames flipped");
                }
                host.drawn(output, waiting.frame_mut());
                waiting.drawn();
                Ok(Present::Rendered)
            }
            Presented::Refused { slot, first, error } => {
                host.drawn(output, waiting.frame_mut());
                if flip.buffers.is_lost() {
                    let now = Instant::now();
                    let wait = self.recovery.failed(now);
                    self.retry_at = Some(now + wait);
                    log::warn!(area = "render", output = %output, retry_ms = wait.as_millis(), "the Vulkan device was lost; the output keeps its last frame while the renderer is rebuilt");
                    if let Some(wake) = &self.wake {
                        wake(wait);
                    }
                } else if first {
                    log::warn!(area = "drm", output = %output, slot, %error, "a frame was refused; the output retries it");
                }
                Ok(Present::Nothing)
            }
        }
    }

    /// The frame last drawn, read back from the Vulkan output's target; a GLES output keeps none, because its frame is
    /// only ever in the buffer the kernel shows, so a screenshot or a screen cast of it is refused.
    fn capture(&mut self, output: OutputId, with_cursor: bool) -> Option<Image> {
        let scale = self.outputs.iter().find(|known| known.id == output)?.effective_scale();
        match self
            .buffers
            .get_mut(&output)?
            .capture(with_cursor, scale, self.treatment)
        {
            Ok(image) => Some(image),
            Err(gnoblin_render_frame::error::RenderError::Unsupported) => None,
            Err(error) => {
                log::warn!(area = "drm", output = %output, %error, "the frame could not be read back");
                None
            }
        }
    }

    fn secure_treatment(&self) -> SecureTreatment {
        self.treatment
    }

    fn effect_kinds(&self) -> &'static [&'static str] {
        self.renderer.effect_kinds()
    }

    fn draws_filter_blur(&self) -> bool {
        self.renderer.draws_filter_blur()
    }

    fn dmabuf(&mut self) -> Option<DmabufSupport> {
        self.scanout.dmabuf_support()
    }

    fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<(), DmabufImportError> {
        self.scanout.import_dmabuf(dmabuf)
    }

    // A Vulkan output keeps its composed frame readable, so a capture client may be given the GPU path; a GLES output's
    // frame is only ever in the buffer the kernel shows, so there is none to copy (see `capture`). A View's picture is
    // on the CPU and is written either way.
    fn dmabuf_capture_support(&mut self, output: OutputId) -> Option<DmabufSupport> {
        if self.renderer != RendererKind::Vulkan || !self.outputs.iter().any(|known| known.id == output) {
            return None;
        }
        self.scanout.dmabuf_support()
    }

    fn dmabuf_picture_support(&mut self) -> Option<DmabufSupport> {
        self.scanout.dmabuf_support()
    }

    fn copy_to_dmabuf(
        &mut self,
        source: gnoblin_backend::DmabufSource<'_>,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
        copy: gnoblin_backend::DmabufCopy,
    ) -> Result<(), gnoblin_backend::DmabufCopyError> {
        match source {
            gnoblin_backend::DmabufSource::Picture(picture) => self.scanout.copy_picture(dmabuf, picture, copy),
            gnoblin_backend::DmabufSource::Frame { output, with_cursor } => {
                self.scanout.check_capture(dmabuf)?;
                let scale = self
                    .outputs
                    .iter()
                    .find(|known| known.id == output)
                    .map(Output::effective_scale)
                    .ok_or(gnoblin_backend::DmabufCopyError::NoFrame)?;
                self.buffers
                    .get_mut(&output)
                    .ok_or(gnoblin_backend::DmabufCopyError::NoFrame)?
                    .copy_frame_into_dmabuf(dmabuf, copy, with_cursor, scale, self.treatment)
                    .map_err(gnoblin_backend::DmabufCopyError::from)
            }
        }
    }

    fn read_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Result<Image, gnoblin_backend::DmabufCopyError> {
        self.scanout.read_dmabuf(dmabuf)
    }

    fn gpu_views(&self) -> bool {
        // GLES and Vulkan draw a View's subtree into a texture on the GPU.
        self.renderer.draws_views()
    }

    fn read_view(
        &mut self,
        scene: &gnoblin_render_frame::view_scene::ViewScene,
        capture: bool,
        live: &[u64],
    ) -> Result<Image, gnoblin_render_frame::error::RenderError> {
        self.scanout.read_view(scene, capture, live)
    }

    fn footprint(&self, output: OutputId) -> gnoblin_render_frame::footprint::Footprint {
        let own = self.buffers.get(&output).map(Buffers::footprint).unwrap_or_default();
        // The probe context serves every output and belongs to none, so it is counted once, on the first output.
        if self.outputs.first().is_some_and(|first| first.id == output) {
            own.plus(self.scanout.footprint())
        } else {
            own
        }
    }

    fn release_views(&mut self) {
        self.scanout.release_views();
        for buffers in self.buffers.values_mut() {
            buffers.release_views();
        }
    }

    fn release_scratch(&mut self) {
        self.scanout.release_scratch();
        for buffers in self.buffers.values_mut() {
            buffers.release_scratch();
        }
    }

    fn output_controls(&self) -> gnoblin_backend::OutputControls {
        gnoblin_backend::OutputControls {
            gamma: true,
            power: true,
            configure: true,
        }
    }

    fn gamma_size(&self, output: OutputId) -> Option<u32> {
        self.gpu
            .gamma_length(output)
            .and_then(|length| u32::try_from(length).ok())
    }

    fn set_gamma(&mut self, output: OutputId, ramp: gnoblin_backend::GammaRamp<'_>) -> Result<(), BackendError> {
        let ramp = gamma::GammaRamp::from_channels(ramp.red, ramp.green, ramp.blue)
            .map_err(|_| BackendError::InvalidGamma(output))?;
        self.gpu
            .set_gamma(output, &ramp)
            .map_err(|error| control_error(output, error))
    }

    fn restore_gamma(&mut self, output: OutputId) -> Result<(), BackendError> {
        self.gpu
            .restore_gamma(output)
            .map_err(|error| control_error(output, error))
    }

    fn set_power(&mut self, output: OutputId, on: bool) -> Result<(), BackendError> {
        self.gpu
            .set_power(output, on)
            .map_err(|error| control_error(output, error))?;
        if on {
            self.dark.remove(&output);
        } else {
            self.dark.insert(output);
        }
        Ok(())
    }

    fn output_modes(&self, output: OutputId) -> Vec<gnoblin_output::DisplayMode> {
        self.gpu
            .outputs()
            .find(|connector| connector.id == output)
            .map(|connector| connector.modes.clone())
            .unwrap_or_default()
    }

    fn configure_output(
        &mut self,
        output: OutputId,
        mode: gnoblin_output::DisplayMode,
        scale: Scale,
        supersample: Scale,
    ) -> Result<(), BackendError> {
        let (connector, current, index, crtc) = self.modeset_target(output)?;
        // The buffers leave the map for the length of the call so the kernel adapter can borrow the rest of the
        // backend; a refused change gives them back untouched.
        let mut buffers = self.buffers.remove(&output);
        let mut kernel = LiveModeset {
            gpu: &self.gpu,
            device: &mut self.scanout,
            output,
            crtc,
            real_modeset: self.real_modeset,
        };
        let result = modeset::configure(
            &mut kernel,
            &connector,
            &current,
            Composition {
                mode,
                scale,
                supersample,
            },
            &mut buffers,
            &mut self.channels[index].presenter,
        );
        if let Some(buffers) = buffers {
            self.buffers.insert(output, buffers);
        }
        let outcome = result.map_err(|error| modeset_error(output, &error))?;
        if let Some(reason) = &outcome.gamma_failure {
            log::warn!(area = "drm", output = %output, %reason, "the gamma table could not be written again after the modeset");
        }
        if outcome.kind != modeset::ModesetKind::Unchanged {
            self.record_reconfigured(output, &outcome.state);
        }
        Ok(())
    }

    fn test_output(
        &mut self,
        output: OutputId,
        mode: gnoblin_output::DisplayMode,
        scale: Scale,
        supersample: Scale,
    ) -> Result<(), BackendError> {
        let (connector, current, _, crtc) = self.modeset_target(output)?;
        let mut kernel = LiveModeset {
            gpu: &self.gpu,
            device: &mut self.scanout,
            output,
            crtc,
            real_modeset: self.real_modeset,
        };
        let wanted = Composition {
            mode,
            scale,
            supersample,
        };
        modeset::check(&mut kernel, &connector, &current, wanted).map_err(|error| modeset_error(output, &error))
    }

    fn max_virtual_edge(&mut self, _output: OutputId) -> u32 {
        self.scanout.max_composed_edge()
    }

    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent> {
        for event in self.gpu.seat().take_events() {
            match event {
                SeatEvent::Paused => self.pause(),
                SeatEvent::Activated => self.resume(),
            }
        }
        let mut events = std::mem::take(&mut self.pending);
        for change in self.gpu.sync_hardware() {
            events.extend(self.apply_hardware(change));
        }
        let crtcs = self.gpu.take_flips();
        complete_flips(&mut self.channels, &crtcs);
        if let Input::Live(libinput) = &mut self.input {
            libinput.pump();
        }
        while let Some(event) = self.input.next_event() {
            host.input(event);
        }
        events
    }
}

/// The output a connected connector is presented as, or [`None`] when it offers no mode to drive.
fn output_of(connector: &plan::Connector) -> Option<Output> {
    let descriptor = connector.descriptor();
    let mode = *descriptor.modes.first()?;
    let scale = descriptor.scales.first().copied().unwrap_or(Scale::integral(1));
    Some(
        Output::new(connector.id, connector.name.clone(), mode, scale)
            .with_identity(connector.edid.as_ref().and_then(gnoblin_output::Edid::identity)),
    )
}

/// The backend error a gamma or power change is reported as.
///
/// A table the CRTC cannot take is [`BackendError::InvalidGamma`]; a CRTC with no table at all is
/// [`BackendError::Unsupported`]; a kernel refusal is logged with its reason and reported as
/// [`BackendError::Unpresentable`], because the output stays in its previous state.
fn control_error(output: OutputId, error: gpu::ControlError) -> BackendError {
    match error {
        gpu::ControlError::NoConnector(_) => BackendError::UnknownOutput(output),
        gpu::ControlError::Gamma(gamma::GammaError::Degenerate) => BackendError::Unsupported(output),
        gpu::ControlError::Gamma(_) => BackendError::InvalidGamma(output),
        gpu::ControlError::Refused(reason) => {
            log::warn!(area = "drm", output = %output, %reason, "the kernel refused a gamma or power change");
            BackendError::Unpresentable(output)
        }
    }
}

/// What a refused mode change tells the person to do. It is a constant because the error that carries it is `Copy`;
/// the test `a_mode_change_the_process_did_not_opt_in_to_names_the_variable` keeps it in step with [`MODESET_ENV`].
const DISABLED_HINT: &str = "set GNOBLIN_DRM_MODESET=1 to turn it on";

/// The backend error a mode or scale change is reported as, using the vocabulary the headless backend has.
///
/// A mode with no pixels is [`BackendError::DegenerateMode`]; a mode the connector does not list is
/// [`BackendError::Unsupported`]; an output with no monitor, no CRTC or no light is [`BackendError::Unpresentable`];
/// a supersample factor that does not give a whole virtual size within the GPU's limit is
/// [`BackendError::Supersample`] (the message naming the numbers is the policy's, which asks
/// [`Backend::max_virtual_edge`] first); buffers that cannot be made are [`BackendError::Render`]; a kernel refusal in
/// the test or the commit is logged with its reason and reported as [`BackendError::ModesetRefused`]; a mode change the
/// process has not opted in to is [`BackendError::ModesetDisabled`], with the variable that turns it on. In every case
/// the output stays as it was.
fn modeset_error(output: OutputId, error: &modeset::ModesetError) -> BackendError {
    match error {
        modeset::ModesetError::Refused(plan::ModesetRefusal::DegenerateMode(mode)) => BackendError::DegenerateMode {
            width: mode.width,
            height: mode.height,
        },
        modeset::ModesetError::Refused(plan::ModesetRefusal::UnsupportedMode { .. }) => {
            BackendError::Unsupported(output)
        }
        modeset::ModesetError::Refused(plan::ModesetRefusal::NotDriven(_) | plan::ModesetRefusal::Dark(_)) => {
            BackendError::Unpresentable(output)
        }
        modeset::ModesetError::Disabled => {
            log::warn!(area = "drm", output = %output, "a mode change was refused: real modesetting is off; set {}=1 to turn it on", MODESET_ENV);
            BackendError::ModesetDisabled(output, DISABLED_HINT)
        }
        modeset::ModesetError::Refused(plan::ModesetRefusal::Supersample(_)) => BackendError::Supersample(output),
        modeset::ModesetError::Buffers(error) => BackendError::Render(*error),
        modeset::ModesetError::Test(_) | modeset::ModesetError::Commit { .. } => {
            log::warn!(area = "drm", output = %output, reason = %error, "the kernel refused a mode change; the output keeps its mode");
            BackendError::ModesetRefused(output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InputRefusal, Refused, modeset_error};
    use crate::modeset::ModesetError;
    use crate::plan::ModesetRefusal;
    use gnoblin_backend::{BackendError, InputSource};
    use gnoblin_output::DisplayMode;
    use gnoblin_output::supersample::SupersampleError;
    use gnoblin_types::OutputId;

    #[test]
    fn a_refused_input_source_never_yields_an_event() {
        let mut input = Refused {
            refusal: InputRefusal::NotOpened,
        };
        assert_eq!(input.refusal(), InputRefusal::NotOpened);
        assert!(input.next_event().is_none());
        assert!(input.next_event().is_none());
    }

    #[test]
    fn a_refusal_names_what_is_missing() {
        let text = InputRefusal::NotOpened.to_string();
        assert!(text.contains("libinput"), "{text}");
    }

    #[test]
    fn a_mode_change_is_refused_in_the_headless_backends_vocabulary() {
        let output = OutputId::new(3);
        let mode = DisplayMode::new(1280, 720, 60_000);
        let unlisted = ModesetError::Refused(ModesetRefusal::UnsupportedMode {
            connector: "DP-1".to_owned(),
            mode,
        });
        assert_eq!(modeset_error(output, &unlisted), BackendError::Unsupported(output));
        let degenerate = ModesetError::Refused(ModesetRefusal::DegenerateMode(DisplayMode::new(0, 720, 60_000)));
        assert_eq!(
            modeset_error(output, &degenerate),
            BackendError::DegenerateMode { width: 0, height: 720 }
        );
        let dark = ModesetError::Refused(ModesetRefusal::Dark("DP-1".to_owned()));
        assert_eq!(modeset_error(output, &dark), BackendError::Unpresentable(output));
        let no_crtc = ModesetError::Refused(ModesetRefusal::NotDriven("DP-1".to_owned()));
        assert_eq!(modeset_error(output, &no_crtc), BackendError::Unpresentable(output));
        let factor = ModesetError::Refused(ModesetRefusal::Supersample(SupersampleError::TooLarge {
            factor: "8".to_owned(),
            width: 1920,
            height: 1080,
            virtual_width: 15360,
            virtual_height: 8640,
            limit: 8192,
        }));
        assert_eq!(
            modeset_error(output, &factor),
            BackendError::Supersample(output),
            "a factor the GPU cannot hold is the supersample refusal, not a mode refusal"
        );
    }

    #[test]
    fn a_mode_change_the_process_did_not_opt_in_to_names_the_variable() {
        let output = OutputId::new(3);
        let error = modeset_error(output, &ModesetError::Disabled);
        assert_eq!(error, BackendError::ModesetDisabled(output, super::DISABLED_HINT));
        let text = error.to_string();
        assert!(
            text.contains(crate::MODESET_ENV),
            "the variable that turns it on is named: {text}"
        );
        assert!(super::DISABLED_HINT.contains(crate::MODESET_ENV));
    }

    #[test]
    fn a_kernel_refusal_is_one_error_whether_the_test_or_the_commit_refused() {
        let output = OutputId::new(3);
        let test = ModesetError::Test("EINVAL".to_owned());
        let commit = ModesetError::Commit {
            reason: "EINVAL".to_owned(),
            rollback_failure: None,
        };
        assert_eq!(modeset_error(output, &test), BackendError::ModesetRefused(output));
        assert_eq!(modeset_error(output, &commit), BackendError::ModesetRefused(output));
    }
}
