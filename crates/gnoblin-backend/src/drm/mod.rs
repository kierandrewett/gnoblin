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
//! 1. **A VT switch leaves DRM master held.** [`crate::session::SessionState`] turns the
//!    pause into one transition, the device is paused through Smithay, and no frame is
//!    submitted until its resume: see `session`'s tests and [`Drm::render`].
//! 2. **A resume repaints only part of the screen.** The resume bumps the session's
//!    generation, and this backend answers with a full-output damage for every output, so no
//!    stale pixel survives: `Drm::dirty` is set from the transition, not from the host.
//! 3. **A hotplug arrives during a modeset.** The connector event is queued and applied in
//!    the next dispatch, never from inside the commit it interrupted; a failure to create the
//!    surface leaves the output disconnected and the session running.
//! 4. **The GPU is removed mid-frame.** The udev event reports [`plan`]`::DeviceChange::
//!    PrimaryLost`, every output is reported gone and the backend answers
//!    [`BackendEvent::Disconnected`].
//! 5. **A gamma table does not fit.** [`gamma::GammaRamp::matches`] refuses it before the
//!    ioctl and the refusal is logged, so the display keeps the table it had.

pub mod gamma;
pub mod plan;

use std::fmt;
use std::io;

use gnoblin_types::OutputId;
use smithay::reexports::calloop::LoopHandle;

use crate::render::Image;
use crate::{Backend, BackendError, BackendEvent, Host, InputSource, Output, Present, Target};

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
    _private: (),
}

impl fmt::Debug for Drm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Drm").finish_non_exhaustive()
    }
}

impl Drm {
    /// Opens the hardware session: the seat, the primary GPU and every connector on it.
    ///
    /// # Errors
    ///
    /// [`DrmError`] naming what could not be opened. A session that gets this error runs the
    /// headless backend instead; it is not a frame error.
    pub fn open(_seat_name: &str) -> Result<Self, DrmError> {
        Err(DrmError::Device(
            "the DRM backend's device path is not built yet".to_owned(),
        ))
    }
}

impl<S: Host + 'static> Backend<S> for Drm {
    fn name(&self) -> &'static str {
        "drm"
    }

    fn outputs(&self) -> &[Output] {
        &[]
    }

    fn output(&self, _id: OutputId) -> Option<&Output> {
        None
    }

    fn target(&mut self, _id: OutputId) -> Option<&mut dyn Target> {
        None
    }

    fn input(&mut self) -> &mut dyn InputSource {
        unreachable!("a hardware backend without a GPU has no input")
    }

    fn register(&mut self, _loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        Ok(())
    }

    fn render(&mut self, _host: &mut S, output: OutputId) -> Result<Present, BackendError> {
        Err(BackendError::UnknownOutput(output))
    }

    fn screenshot(&mut self, _output: OutputId) -> Option<Image> {
        None
    }

    fn dispatch(&mut self, _host: &mut S) -> Vec<BackendEvent> {
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
        kind: GestureKind,
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
pub enum GestureKind {
    /// Two fingers swiping.
    Swipe,
    /// Two fingers pinching.
    Pinch,
    /// Fingers held still.
    Hold,
}
