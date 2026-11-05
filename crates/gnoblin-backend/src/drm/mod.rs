//! The hardware backend: DRM and KMS through udev and libseat, with a GLES scan-out path.
//!
//! This is the third backend of the Backends card, and the only one that drives a real
//! display. It is split the way the rest of the crate is:
//!
//! | Module       | Owns                                                                | Tested without a device |
//! | ------------ | ------------------------------------------------------------------- | ----------------------- |
//! | [`plan`]     | the output state, the ordered kernel operations, the rollback record | yes                     |
//! | [`gamma`]    | the lookup table and the length check before the ioctl               | yes                     |
//! | `mod` (here) | the [`Backend`] implementation: devices, outputs, dispatch           | no                      |
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

pub mod frame;
pub mod gamma;
pub mod gpu;
pub mod plan;
pub mod present;
pub(crate) mod scanout;

use std::fmt;
use std::io;

use tracing::warn;

use gnoblin_types::{OutputId, Scale};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::drm::control::crtc;

use crate::drm::frame::{Presented, present};
use crate::drm::present::{OutputPresenter, Presenter, complete_flips};
use crate::drm::scanout::{Buffers, Device as ScanoutDevice, Flip};
use crate::libinput::Libinput;
use crate::libinput::map::Screen;
use crate::render::Image;
use crate::render::OutputFrame;
use crate::session::{SeatEvent, SeatState, Transition};
use crate::{Backend, BackendError, BackendEvent, Host, InputEvent, InputSource, Output, Present, Target};
use std::collections::HashMap;

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
    /// The GLES context on the GPU could not be created.
    Renderer(crate::render::RenderError),
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
    input: Input,
    /// Whether the seat is ours, which decides whether the device may be used.
    seat: SeatState,
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
    /// # Errors
    ///
    /// [`DrmError`] naming what could not be opened. A session that gets this error runs the
    /// headless backend instead; it is not a frame error.
    pub fn open(seat_name: &str) -> Result<Self, DrmError> {
        let gpu = gpu::Gpu::open(seat_name)?;
        let outputs: Vec<Output> = gpu
            .outputs()
            .filter(|connector| connector.connected)
            .filter_map(|connector| {
                let descriptor = connector.descriptor();
                let mode = *descriptor.modes.first()?;
                let scale = descriptor.scales.first().copied().unwrap_or(Scale::integral(1));
                Some(Output::new(connector.id, connector.name.clone(), mode, scale))
            })
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
        let scanout = ScanoutDevice::open(gpu.device_fd()).map_err(DrmError::Renderer)?;
        let input = match Libinput::open(gpu.seat().libseat()) {
            Ok(libinput) => Input::Live(Box::new(libinput)),
            Err(error) => {
                tracing::warn!(%error, "[input] libinput is not open; the session runs without input");
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
            input,
            seat: SeatState::new(),
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
            tracing::error!(%error, "[drm] the device could not be re-activated after the seat returned");
        }
        for channel in &mut self.channels {
            channel.presenter.resume();
        }
        self.buffers.clear();
        if let Input::Live(libinput) = &mut self.input {
            libinput.resume();
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

    fn register(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        self.gpu.register(loop_handle)?;
        let screens = self.screens();
        if let Input::Live(libinput) = &mut self.input {
            libinput.set_screens(screens);
            libinput.register(loop_handle)?;
        }
        Ok(())
    }

    fn render(&mut self, host: &mut S, output: OutputId) -> Result<Present, BackendError> {
        let Some(known) = self.outputs.iter().find(|known| known.id == output) else {
            return Err(BackendError::UnknownOutput(output));
        };
        let (scale, size) = (known.scale, known.mode);
        let Some(index) = self.channels.iter().position(|channel| channel.id == output) else {
            return Err(BackendError::Unpresentable(output));
        };
        let crtc = self.channels[index].crtc;
        let mut frame = OutputFrame::new();
        if host.frame(output, &mut frame) {
            self.channels[index].presenter.mark_dirty();
        }
        let buffers = match self.buffers.entry(output) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let formats = self
                    .gpu
                    .plane_formats(crtc)
                    .ok_or(BackendError::Unpresentable(output))?;
                let physical = gnoblin_types::Size::new(size.width, size.height);
                let buffers = Buffers::new(&self.scanout, physical, &formats).map_err(BackendError::Render)?;
                entry.insert(buffers)
            }
        };
        let mut flip = Flip {
            buffers,
            gpu: &self.gpu,
            crtc,
            scale,
        };
        match present(&mut self.channels[index].presenter, &mut flip, &frame) {
            Presented::Idle | Presented::Waiting => Ok(Present::Nothing),
            Presented::Queued { .. } => Ok(Present::Rendered),
            Presented::Refused { slot, first, error } => {
                if first {
                    warn!(output = %output, slot, %error, "[drm] a frame was refused; the output retries it");
                }
                Ok(Present::Nothing)
            }
        }
    }

    fn screenshot(&mut self, _output: OutputId) -> Option<Image> {
        None
    }

    fn dispatch(&mut self, _host: &mut S) -> Vec<BackendEvent> {
        for event in self.gpu.seat().take_events() {
            match event {
                SeatEvent::Paused => self.pause(),
                SeatEvent::Activated => self.resume(),
            }
        }
        let crtcs = self.gpu.take_flips();
        complete_flips(&mut self.channels, &crtcs);
        if let Input::Live(libinput) = &mut self.input {
            libinput.pump();
        }
        Vec::new()
    }
}

/// The input events a hardware seat produces beyond the contract's [`InputEvent`].
///
/// Tablets and gestures cannot be [`InputEvent`]s: a tablet tool carries pressure, tilt and a
/// tool identity, and a gesture carries a phase and a scale, and none of them is a position a
/// seat can apply. They are offered here instead, so `gnoblin-input`'s touchpad router and
/// tablet handling read them from the same backend that produced the pointer events.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RawEvent {
    /// A gesture began, continued or ended on a touchpad.
    Gesture {
        /// The device that produced it.
        device: u64,
        /// Which recognised gesture it is.
        kind: RawGestureKind,
        /// Whether this is the first event of the gesture.
        begin: bool,
        /// Whether this is the last event of the gesture.
        end: bool,
        /// The gesture's frame rate in frames per second for a hold, the scale for a pinch.
        value: f64,
    },
}

/// Which gesture a [`RawEvent::Gesture`] reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawGestureKind {
    /// Two fingers swiping.
    Swipe,
    /// Two fingers pinching.
    Pinch,
    /// Fingers held still.
    Hold,
}

#[cfg(test)]
mod tests {
    use super::{InputRefusal, Refused};
    use crate::InputSource;

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
}
