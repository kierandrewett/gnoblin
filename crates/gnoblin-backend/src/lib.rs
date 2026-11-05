//! Backends (DRM, nested, headless) and the extension traits the engine crates share.
//!
//! This crate is the "Backends and outputs" mechanism layer of
//! `design/rust-compositor.md`, and the one place the engine's crates are allowed
//! to meet. It owns two things:
//!
//! - **The backend contract.** [`Backend`] drives outputs, hands the compositor a
//!   [`Target`] to draw into, delivers [`InputEvent`]s from an [`InputSource`] and
//!   registers whatever event-loop sources it needs. The compositor implements
//!   [`Host`] and knows nothing else about where the pixels and the events come
//!   from.
//! - **The backends themselves.** [`headless`] is the first one: no DRM, no GPU, no
//!   session, a virtual output with a settable size. It is what CI and the
//!   integration tests run, and it is the contract's reference implementation.
//!
//! # Why the traits look like this
//!
//! The nested and DRM backends have to arrive without a trait change, so the
//! contract is stated in the terms all three share rather than in the terms the
//! headless one happens to need:
//!
//! - a backend is one value that owns its device, its targets and its input;
//! - the compositor is a separate [`Host`] value, passed in per call, so neither
//!   side owns the other and a backend can be replaced without rebuilding the
//!   server;
//! - the event-loop hook is `register`, and the source it adds calls back into the
//!   backend's own methods, because a DRM backend's source is where libseat and
//!   udev events turn into [`BackendEvent`]s;
//! - rendering is `render(host, output)`: the host composites into the target and
//!   the target presents, which is the same split for a GLES surface, a Pixman
//!   buffer and a headless frame that presents nothing.
//!
//! # This slice uses no unsafe code
//!
//! The workspace forbids `unsafe` and this crate does not opt out. The safe API is
//! enough for the headless backend, and the DRM backend — which must call libseat,
//! libdrm and GBM — will opt out with `#[allow(unsafe_code)]` on named FFI shim
//! modules when it lands, as `Cargo.toml` at the workspace root describes.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `src/headless.rs` unless it says
//! otherwise.
//!
//! 1. **A frame is rendered for an output that is gone.** An unplugged output is
//!    not in the backend's table, so [`Backend::render`] returns
//!    [`BackendError::UnknownOutput`] and the host is never called with a stale
//!    id: `rendering_an_unknown_output_is_refused`.
//! 2. **A resize storm leaves the compositor on an old size.** The backend keeps
//!    one pending [`BackendEvent::OutputResized`] per output, so a hundred resizes
//!    between two dispatches deliver the latest size once instead of a queue of
//!    stale ones: `a_resize_storm_delivers_one_event_with_the_latest_size`.
//! 3. **A degenerate mode makes an empty output.** A zero-width or zero-height mode
//!    is refused by [`BackendError::DegenerateMode`] instead of producing an output
//!    that can never show a window: `a_degenerate_mode_is_refused`.
//! 4. **Damage outside the target is drawn.** The headless target clips damage to
//!    its own extent, so a host that reports a window hanging off the output does
//!    not cause an out-of-bounds write in the software path:
//!    `damage_outside_the_target_is_clipped`.
//! 5. **Queued input is lost when every output goes away.** An input queue belongs
//!    to the backend, not to an output: with no output connected the queue still
//!    drains into the host and the frame path refuses the unknown id instead of
//!    panicking: `queued_input_reaches_the_host_with_no_output_connected`.
//! 6. **A backend without outputs holds a render target anyway.** With no outputs,
//!    [`Backend::outputs`] is empty, every id is unknown, and the host is never
//!    asked to draw: `a_backend_with_no_outputs_renders_nothing`.
//! 7. **A present is reported as rendered when nothing was drawn.** The headless
//!    target presents no pixels, so it reports [`Present::Nothing`] and counts the
//!    frames it was asked for; a host that presents itself is not presented twice:
//!    `a_headless_present_reports_that_it_drew_nothing` and
//!    `a_host_that_presents_itself_is_not_presented_twice`.
//! 8. **A change is reported more than once.** A start-up output is announced on
//!    one dispatch and never again, because the pending list is drained:
//!    `a_started_backend_reports_its_output_once`.

pub mod headless;

use std::fmt;
use std::io;

use gnoblin_types::{OutputId, Point, Rect, Scale, Size};
use smithay::reexports::calloop::LoopHandle;

use gnoblin_output::{DisplayMode, OutputDescriptor};

/// One output a backend drives.
///
/// A backend knows the connector it drives, the mode the display is in and the
/// scale the session composes at. It does not know where the output sits in the
/// desktop: position comes from the declarative layout in `gnoblin-output`, which
/// the composition root computes once and hands to both this crate's contract and
/// the window manager. That split is why a mode change is a backend event and a
/// move is not.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Output {
    /// The graph token the rest of the compositor keys its tables by.
    pub id: OutputId,
    /// The connector name a person and a saved configuration see, such as `"DP-1"`.
    pub connector: String,
    /// The mode being driven.
    pub mode: DisplayMode,
    /// The scale the output composes at.
    pub scale: Scale,
}

impl Output {
    /// An output from its token, connector name, mode and scale.
    #[must_use]
    pub fn new(id: OutputId, connector: impl Into<String>, mode: DisplayMode, scale: Scale) -> Self {
        Self {
            id,
            connector: connector.into(),
            mode,
            scale,
        }
    }

    /// The logical extent the mode and scale produce.
    ///
    /// Rotation is not part of a backend output in this slice: the headless output
    /// never rotates, and the DRM backend will carry the panel orientation in the
    /// descriptor it derives from EDID.
    #[must_use]
    pub fn logical_size(&self) -> Size {
        self.mode.logical_size(gnoblin_types::Orientation::Normal, self.scale)
    }

    /// The output as the layout engine sees it.
    ///
    /// The descriptor carries the mode list this backend can drive, which for a
    /// virtual output is the single mode it is in. The layout engine owns the
    /// position, the scale validation and the mirroring rules.
    #[must_use]
    pub fn descriptor(&self) -> OutputDescriptor {
        let mut descriptor = OutputDescriptor::new(self.id, self.connector.clone(), vec![self.mode]);
        descriptor.scales = vec![self.scale];
        descriptor
    }
}

/// What a target did with the frame it was given.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Present {
    /// The target presented the pixels the host drew.
    Rendered,
    /// The target has no pixels to present, so the frame is complete without a
    /// swap.
    ///
    /// The headless backend reports this: it exists so protocol behaviour can be
    /// tested without a GPU, and pretending a swap happened would make a test
    /// believe a software renderer is installed.
    Nothing,
}

/// A surface a backend can present to.
///
/// The trait is deliberately about presenting and not about drawing: the host
/// draws through its own renderer, and the backend only says how big the target is
/// and what to do with the finished frame. That is what lets one renderer serve a
/// GLES target, a Pixman buffer and the headless target that presents nothing.
pub trait Target {
    /// The target's size in logical pixels.
    fn size(&self) -> Size;

    /// The scale the target composes at.
    fn scale(&self) -> Scale;

    /// Present the frame the host drew, with the damage that changed.
    ///
    /// Damage rectangles are in the target's own logical space and may lie partly
    /// or wholly outside it; clipping is the target's job.
    fn present(&mut self, damage: &[Rect]) -> Present;
}

/// One input event a backend delivered.
///
/// The set is the smallest one a seat can act on: where the pointer is, whether a
/// button changed and whether a key changed. Axis, touch, tablet and gesture
/// events arrive with the input slice, and they extend this enum rather than the
/// trait.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputEvent {
    /// The pointer moved to a position in the output's logical space.
    PointerMotion {
        /// The output the position is relative to.
        output: OutputId,
        /// The position within that output.
        position: Point,
    },
    /// The pointer left every output.
    PointerLeave,
    /// A pointer button changed state; `button` is the Linux input event code.
    PointerButton {
        /// The button code.
        button: u32,
        /// Whether the button is now held.
        pressed: bool,
    },
    /// A key changed state; `keycode` is the evdev scan code the seat translates.
    Key {
        /// The evdev key code.
        keycode: u32,
        /// Whether the key is now held.
        pressed: bool,
    },
}

/// Where a backend's input comes from.
///
/// A backend pushes what its device produced into this queue and the compositor
/// drains it in [`Backend::dispatch`], so the same code path serves a libinput
/// device and a test that injects a synthetic click.
pub trait InputSource {
    /// The next queued event, or [`None`] when the queue is empty.
    fn next_event(&mut self) -> Option<InputEvent>;
}

/// A change in the backend the compositor has to react to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BackendEvent {
    /// An output appeared.
    OutputConnected(Output),
    /// An output went away.
    OutputDisconnected(OutputId),
    /// An output's mode or scale changed.
    OutputResized {
        /// The output that changed.
        output: OutputId,
        /// The mode it is in now.
        mode: DisplayMode,
        /// The scale it composes at now.
        scale: Scale,
    },
}

/// What a backend needs from the compositor it drives.
///
/// The compositor implements this; the backend calls it. Keeping the two apart is
/// what lets `gnoblin-wayland` and the backends meet without either naming the
/// other's types.
pub trait Host {
    /// Composite the scene for `output` into `target`.
    ///
    /// The host decides what is drawn: this slice's scene is empty, so the call
    /// fills nothing and the target presents nothing.
    fn render(&mut self, output: OutputId, target: &mut dyn Target);

    /// Handle one input event the backend delivered.
    fn input(&mut self, event: InputEvent);
}

/// A backend: outputs, a target per output, an input source and the event-loop hook.
///
/// `S` is the compositor state, so a backend never owns the compositor and the
/// compositor never owns the backend: the composition root holds both and passes
/// one to the other. The trait is object-safe, so a session can swap a headless
/// backend for a nested one without changing its own type.
pub trait Backend<S: Host + 'static> {
    /// The backend's name, for logs and the startup report.
    fn name(&self) -> &'static str;

    /// Every output this backend drives, in connection order.
    fn outputs(&self) -> &[Output];

    /// One output, if this backend drives it.
    fn output(&self, id: OutputId) -> Option<&Output>;

    /// The target for an output, if this backend drives it.
    fn target(&mut self, id: OutputId) -> Option<&mut dyn Target>;

    /// The input queue this backend fills.
    fn input(&mut self) -> &mut dyn InputSource;

    /// Register the event-loop sources this backend owns.
    ///
    /// The headless backend has no file descriptor and no device, so it registers
    /// nothing. A DRM backend registers its libseat and udev sources here, and the
    /// sources call back into the backend's own methods; the compositor's loop is
    /// the only thing that has to know, so nothing about a device reaches a
    /// protocol handler.
    fn register(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()>;

    /// Draw and present one frame on `output`.
    ///
    /// Errors only when the output is unknown: a frame that cannot be presented
    /// because the monitor went away is reported as an [`BackendEvent`] on the next
    /// dispatch, not as a failure of the frame.
    fn render(&mut self, host: &mut S, output: OutputId) -> Result<Present, BackendError>;

    /// Deliver queued input to the host and report state changes.
    ///
    /// Input is delivered first so a host that reacts to a mode change sees the
    /// pointer where the user left it.
    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent>;
}

/// Why a backend refused an operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackendError {
    /// The backend does not drive that output.
    UnknownOutput(OutputId),
    /// The mode has no pixels on one axis, so the output could never show a
    /// window.
    DegenerateMode {
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOutput(output) => write!(formatter, "the backend does not drive output {output}"),
            Self::DegenerateMode { width, height } => {
                write!(formatter, "a {width}x{height} mode covers no pixels")
            }
        }
    }
}

impl std::error::Error for BackendError {}
