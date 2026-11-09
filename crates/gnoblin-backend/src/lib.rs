//! Backends (nested, headless) and the extension traits the engine crates share. The DRM/KMS
//! backend is `gnoblin-backend-drm`.
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
//!   [`nested`] is the second: a Wayland *client* of a compositor that hosts the
//!   session, with one `xdg_toplevel` per output, sized and scaled by the parent's
//!   configure, drawn into shared memory and paced by the parent's frame
//!   callbacks. It is how a signed-in session runs under a display manager
//!   (design/rust-compositor.md, "Hosting sessions").
//! - **The render path they share.** [`render`] holds the frame description, the
//!   Pixman renderer, the RGBA8 readback and the session scene that produces a
//!   frame from the scene graph. It lives here for the same reason: this crate is
//!   the one place the engine's crates may meet, so `gnoblin-wayland` and
//!   `gnoblin-render` never name each other.
//! - **The Xwayland seam.** [`x11`] is the vocabulary `gnoblin-xwayland` (the start and
//!   restart policy, the X11 window table) and `gnoblin-wayland` (the process, the X11
//!   connection, the surfaces) share, for that same reason.
//!
//! # Why the traits look like this
//!
//! The nested and DRM (`gnoblin-backend-drm`) backends have to arrive without a trait change,
//! so the
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
//! - drawing is `frame(host, output)`: the host *describes* what the output shows,
//!   the backend's target draws it and presents it. The description is a
//!   [`gnoblin_render_frame::frame::OutputFrame`], so the host never learns which renderer is installed and a
//!   GLES target can replace a Pixman one without touching the compositor.
//! - the host is told when an iteration starts (`begin_frame`) because damage is
//!   accumulated once and consumed per output: taking it per output would clear the
//!   other outputs' damage and leave them stale.
//!
//! # Where this crate uses unsafe
//!
//! It does not, and the workspace's `unsafe_code = "forbid"` applies to it unchanged. The
//! unsafe calls live in the crates that own them: the GLES renderer's EGL and GL calls in
//! `gnoblin-render-gles` and the Vulkan driver calls in `gnoblin-render-vulkan`, each
//! opening with `#[allow(unsafe_code)]` and a `SAFETY:` comment per block. The `wl_shm`
//! pool mapping the frame writes through lives in `gnoblin-render-frame` (`shm.rs`).
//! `gnoblin-backend-drm` has none, and a session that draws with the software path
//! reaches none of the others.
//!
//! # Lifecycle of a frame
//!
//! One iteration of the compositor's loop, as this contract sees it:
//!
//! 1. **Wake.** A source the backend registered with [`Backend::register`] fires: a
//!    libseat or udev event for DRM, the parent's events for nested. The backend turns
//!    the wake-up into queued [`InputEvent`]s and pending [`BackendEvent`]s. It does not
//!    draw.
//! 2. **Input and changes.** [`Backend::dispatch`] hands every queued event to
//!    [`Host::input`] before it returns the pending [`BackendEvent`]s. Input goes first
//!    so the host reacts to a mode change with the pointer where the user left it.
//! 3. **Begin.** The caller calls [`Host::begin_frame`] once, before any output is
//!    rendered. Damage is snapshotted here and not per output (see the trait docs).
//! 4. **Describe, per output.** [`Backend::render`] checks the output is known, then calls
//!    [`Host::frame`] with a cleared [`gnoblin_render_frame::frame::OutputFrame`]. A `false` answer ends the
//!    call with nothing drawn and nothing presented.
//! 5. **Draw and present.** Otherwise the output's [`Target`] draws the frame and presents
//!    the damage it reports, and [`Backend::render`] returns that as a [`Present`]. A
//!    backend may also hold a present and return [`Present::Nothing`] (the headless
//!    backend does while its presenting is paused), which leaves the output as it was.
//! 6. **Capture, if asked.** [`Backend::capture`] reads back the last frame a target drew.
//!    It does not describe a new one.
//!
//! Who owns what:
//!
//! | Thing | Owned by | Lives until |
//! |---|---|---|
//! | Device (DRM fd, libinput context, parent connection) | the backend | the backend is dropped |
//! | Output table and [`OutputId`]s | the backend | the output is unplugged |
//! | [`Target`] and its renderer | the backend | the output is unplugged |
//! | Scene and damage accumulation | the host | the host is dropped |
//! | Input queue | the backend fills it, the host drains it | drained in `dispatch` |
//!
//! The host is passed as `&mut S` on every call, so the backend never stores a reference
//! to it and the two values stay separate owners.
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
//! 7. **A present is reported as rendered when nothing was drawn.** A headless
//!    target presents no pixels to a display, so it reports [`Present::Nothing`] and
//!    counts the frames it was asked to present; a host that presents the frame
//!    itself is not presented twice: `a_headless_present_reports_that_it_drew_nothing`
//!    and `a_host_that_presents_itself_is_not_presented_twice`.
//! 8. **A change is reported more than once.** A start-up output is announced on
//!    one dispatch and never again, because the pending list is drained:
//!    `a_started_backend_reports_its_output_once`.
//! 9. **A frame is drawn although the host reports no change.** An idle iteration
//!    draws nothing and presents nothing: `a_backend_that_changed_nothing_draws_nothing`.
//! 10. **A readback is taken from a target that has none.**
//!     [`Backend::screenshot`] answers [`None`] for an output this backend does not
//!     drive: `a_screenshot_of_an_unknown_output_is_none`.
//! 11. **A hotplug reuses the identity of an output that was unplugged.** Identities
//!     come from a counter that never repeats, so a lock's presentation barrier cannot
//!     mistake the new output for the old one:
//!     `a_hotplug_after_an_unplug_does_not_reuse_an_identity`. A caller behind the
//!     session reaches the backend through a [`headless::HeadlessHandle`], and the
//!     request is reported once, as its own dispatch:
//!     `a_hotplug_from_a_handle_is_reported_once`.
//! 12. **A frame is reported as presented when it was not.** A held present leaves the
//!     output as it was and never calls the host, so a lock waiting for a presented
//!     cover keeps waiting: `a_held_present_keeps_the_output_as_it_was`. A frame the
//!     host described but the target could not draw is not presented either, so a lock
//!     is not proven by it: `a_failed_cover_draw_does_not_prove_the_cover`.
//! 13. **A nested session keeps drawing into a parent that has gone.** Every failure
//!     of the parent connection, and every way a nested output can vanish, is in
//!     [`nested`]'s own list, because that backend's failure modes are about the wire
//!     rather than about the contract.

pub mod headless;
pub mod nested;
pub mod render;
pub mod x11;

use std::fmt;
use std::io;

use gnoblin_types::{OutputId, Point, Rect, Scale, Size};
use smithay::reexports::calloop::LoopHandle;

use gnoblin_output::{DisplayMode, OutputDescriptor};

gnoblin_log::logger!(log = "backend");

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

/// A surface a backend can present to, and the renderer that draws into it.
///
/// The trait is about presenting and drawing, not about *what* is drawn: the host
/// describes a [`gnoblin_render_frame::frame::OutputFrame`] and the target draws it with whatever renderer the
/// backend installed. That is what lets one compositor serve a GLES surface, a
/// Pixman buffer and the headless target that presents nothing.
pub trait Target {
    /// The target's size in logical pixels.
    fn size(&self) -> Size;

    /// The scale the target composes at.
    fn scale(&self) -> Scale;

    /// Draw the described frame, repainting only what it damaged.
    ///
    /// # Errors
    ///
    /// [`gnoblin_render_frame::error::RenderError`] when the renderer refuses an operation. A layer whose
    /// client buffer cannot be read is not an error: a client may destroy its buffer
    /// at any time, and the rest of the frame still has to appear.
    fn draw(
        &mut self,
        frame: &gnoblin_render_frame::frame::OutputFrame,
    ) -> Result<(), gnoblin_render_frame::error::RenderError>;

    /// Present the frame the host drew, with the damage that changed.
    ///
    /// Damage rectangles are in the target's own logical space and may lie partly
    /// or wholly outside it; clipping is the target's job.
    fn present(&mut self, damage: &[Rect]) -> Present;
}

/// One input event a backend delivered.
///
/// The set is the smallest one a seat can act on: where the pointer is, whether a
/// button, a key or a scroll axis changed, and whether a touch point did. Tablet and
/// gesture events arrive with the input slice and extend this enum rather than the
/// trait.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputEvent {
    /// The pointer moved to a position in the output's logical space.
    PointerMotion {
        /// The output the position is relative to.
        output: OutputId,
        /// The position within that output.
        position: Point,
    },
    /// The pointer moved relative to where it is, in logical pixels.
    ///
    /// A mouse is a relative device: the hardware reports a delta rather than a
    /// position, so the seat applies the delta to the position it already holds and
    /// keeps the result inside the layout. The unit is whole logical pixels; a backend
    /// reading a finer device accumulates the fraction and reports the whole part,
    /// which is the same rounding the compositor's integer layout does anyway.
    PointerMotionRelative {
        /// Horizontal movement.
        dx: i32,
        /// Vertical movement.
        dy: i32,
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
    /// A scroll axis moved.
    ///
    /// The value is in 1/120 of a step, the unit `REL_WHEEL_HI_RES` uses, so a
    /// wheel notch is [`AXIS_STEP`](crate::nested::AXIS_STEP) and a touchpad's
    /// fractional scroll is the same unit at a smaller value. Horizontal is
    /// positive to the right, vertical positive downwards: the direction a
    /// document moves, which is what `wl_pointer.axis` already states.
    PointerAxis {
        /// Horizontal movement in 1/120 of a step.
        horizontal: i32,
        /// Vertical movement in 1/120 of a step.
        vertical: i32,
    },
    /// A touch point changed state.
    ///
    /// `slot` is the device's touch identity and stays the same for the whole
    /// gesture, so a seat tracks one point per slot. `position` is the output-local
    /// position of the point; a cancelled or lifted point reports the position it
    /// was last at.
    Touch {
        /// The output the touch is on.
        output: OutputId,
        /// The device's touch identity.
        slot: u32,
        /// The position within that output.
        position: Point,
        /// What happened to the point.
        phase: TouchPhase,
    },
    /// A device joined the seat.
    ///
    /// `id` is the backend's own number for the device; a later [`InputEvent::DeviceRemoved`]
    /// names the same number. `kind` is the collapsed kind the configuration reads.
    DeviceAdded {
        /// The backend's number for the device.
        id: u64,
        /// The kind of device.
        kind: DeviceKind,
    },
    /// A device left the seat.
    ///
    /// Any button or touch point it held was released by events sent before this one, so the
    /// seat holds nothing for it once this arrives.
    DeviceRemoved {
        /// The backend's number for the device, as the matching [`InputEvent::DeviceAdded`] stated.
        id: u64,
    },
    /// A touchpad gesture sample.
    ///
    /// The seat never applies these; the policy layer routes them to gesture bindings.
    Touchpad(RawTouchpadSample),
}

/// One touchpad gesture sample, as the device's recogniser reported it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RawTouchpadSample {
    /// The backend's number for the device that produced it.
    pub device: u64,
    /// Where the sample sits in its gesture.
    pub phase: RawTouchpadPhase,
    /// The gesture's kind. A hold has no binding kind and is routed as nothing.
    pub kind: RawTouchpadKind,
    /// The finger count, as stated by the gesture's begin.
    pub fingers: u8,
    /// Horizontal movement since the previous sample, in device units.
    pub dx: f64,
    /// Vertical movement since the previous sample, in device units.
    pub dy: f64,
    /// Cumulative scale since the begin; one for a swipe.
    pub scale: f64,
}

/// Where a [`RawTouchpadSample`] sits in its gesture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawTouchpadPhase {
    /// The gesture was recognised.
    Begin,
    /// Movement since the previous sample.
    Update,
    /// The gesture ended normally.
    End,
    /// The gesture ended as cancelled.
    Cancel,
}

/// Which gesture a [`RawTouchpadSample`] belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawTouchpadKind {
    /// Fingers moving together.
    Swipe,
    /// Fingers spreading or gathering.
    Pinch,
    /// Fingers held still.
    Hold,
}

/// What kind of device a setting applies to.
///
/// It is libinput's capability set collapsed to the kinds the configuration has a section for,
/// so one rule decides which section a device reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceKind {
    /// A touchpad, which has every setting the configuration offers.
    Touchpad,
    /// A mouse: acceleration, handedness, scroll direction.
    Mouse,
    /// A trackball: acceleration, middle-click emulation, scroll emulation.
    Trackball,
    /// A pointing stick: acceleration, scroll method.
    PointingStick,
    /// A touch screen: no settings, because its calibration belongs to the seat.
    Touch,
    /// A keyboard: repeat and delay are the keymap's, not libinput's.
    Keyboard,
}

/// What happened to a touch point.
///
/// It is a separate enum rather than four [`InputEvent`] variants because a seat
/// handles every phase the same way: it looks the slot up and applies the point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TouchPhase {
    /// The point went down.
    Down,
    /// The point moved while held.
    Motion,
    /// The point lifted.
    Up,
    /// The device or the compositor cancelled the gesture, so the point is gone
    /// without an `Up`.
    Cancel,
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
    /// The backend lost the connection to whatever drives it, so every output it
    /// reported is gone and none can come back.
    ///
    /// It is a separate event from [`BackendEvent::OutputDisconnected`] because the
    /// two mean different things to the session: an output can be unplugged and
    /// replugged, while a nested session whose parent compositor died has nothing
    /// left to draw on and must end. It is reported once, on the first dispatch
    /// after the failure.
    Disconnected,
}

/// What a backend needs from the compositor it drives.
///
/// The compositor implements this; the backend calls it. Keeping the two apart is
/// what lets `gnoblin-wayland` and the backends meet without either naming the
/// other's types, and it is why the host describes a frame rather than drawing one:
/// the backend owns the renderer, the host owns the scene.
pub trait Host {
    /// Start an iteration, before any output is rendered.
    ///
    /// A host that accumulates damage snapshots it here, because the backend then
    /// asks for one output at a time and a per-output snapshot would clear the other
    /// outputs' damage and leave them stale.
    fn begin_frame(&mut self);

    /// Describe what `output` shows now, and answer whether anything changed.
    ///
    /// The frame is cleared by [`gnoblin_render_frame::frame::OutputFrame::clear`] at the start of every call, so
    /// a host fills it from scratch. A `false` answer means the backend draws nothing
    /// for this output: an idle iteration must not repaint the screen.
    fn frame(&mut self, output: OutputId, frame: &mut gnoblin_render_frame::frame::OutputFrame) -> bool;

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
    /// Nothing is drawn when the host reports no change. Errors only when the output
    /// is unknown or the renderer refuses an operation: a frame that cannot be
    /// presented because the monitor went away is reported as a [`BackendEvent`] on
    /// the next dispatch, not as a failure of the frame.
    fn render(&mut self, host: &mut S, output: OutputId) -> Result<Present, BackendError>;

    /// Read the last frame drawn on `output` back as pixels, with the cursor composed in
    /// when `with_cursor` is set.
    ///
    /// [`None`] when this backend does not drive the output or has no renderer for
    /// it. The image is in physical pixels, top-left first, RGBA8: see
    /// [`gnoblin_render_frame::image::Image`]. Capture and the tests read through this, which is why it is
    /// on the contract and not on one backend.
    fn capture(&mut self, output: OutputId, with_cursor: bool) -> Option<gnoblin_render_frame::image::Image>;

    /// The last frame drawn on `output` without the cursor: [`Backend::capture`] with
    /// `with_cursor` off.
    fn screenshot(&mut self, output: OutputId) -> Option<gnoblin_render_frame::image::Image> {
        self.capture(output, false)
    }

    /// Deliver queued input to the host and report state changes.
    ///
    /// Input is delivered first so a host that reacts to a mode change sees the
    /// pointer where the user left it.
    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent>;

    /// What this backend can control on its outputs: gamma tables and power state.
    ///
    /// The default is nothing, so a backend that does not implement the controls is
    /// honest about it and the protocols that need them are not advertised.
    fn output_controls(&self) -> OutputControls {
        OutputControls::default()
    }

    /// The number of entries in each gamma table on `output`, if it has one.
    fn gamma_size(&self, _output: OutputId) -> Option<u32> {
        None
    }

    /// Load a gamma ramp onto `output`. Each channel has [`Backend::gamma_size`] entries.
    fn set_gamma(&mut self, output: OutputId, _ramp: GammaRamp<'_>) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(output))
    }

    /// Put `output`'s gamma back to the identity ramp.
    fn restore_gamma(&mut self, output: OutputId) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(output))
    }

    /// Switch `output` on or off. Off is a power state, not a disconnection.
    fn set_power(&mut self, output: OutputId, _on: bool) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(output))
    }

    /// Drive `output` at `mode` and `scale`. Backends that cannot re-mode a live output
    /// refuse, which is the default.
    fn configure_output(&mut self, output: OutputId, _mode: DisplayMode, _scale: Scale) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(output))
    }
}

/// Which per-output controls a backend can honour.
///
/// A host advertises a protocol only when the backend reports the matching control, so a
/// client never binds a global whose requests cannot be honoured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputControls {
    /// Gamma tables can be loaded and restored.
    pub gamma: bool,
    /// Output power can be switched on and off.
    pub power: bool,
}

/// One gamma ramp: a table per colour channel, each as long as [`Backend::gamma_size`].
#[derive(Clone, Copy, Debug)]
pub struct GammaRamp<'a> {
    /// Red output values, one per input step.
    pub red: &'a [u16],
    /// Green output values, one per input step.
    pub green: &'a [u16],
    /// Blue output values, one per input step.
    pub blue: &'a [u16],
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
    /// The output's renderer refused to draw.
    Render(gnoblin_render_frame::error::RenderError),
    /// The output exists but this backend has no scan-out path for it yet, so no frame can be
    /// presented to it. Distinct from [`BackendError::UnknownOutput`]: the output is real.
    Unpresentable(OutputId),
    /// The backend cannot control this output's gamma or power state.
    Unsupported(OutputId),
    /// The gamma ramp's length does not match the output's table size.
    InvalidGamma(OutputId),
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOutput(output) => write!(formatter, "the backend does not drive output {output}"),
            Self::DegenerateMode { width, height } => {
                write!(formatter, "a {width}x{height} mode covers no pixels")
            }
            Self::Render(error) => write!(formatter, "the output's renderer refused a frame: {error}"),
            Self::Unpresentable(output) => {
                write!(
                    formatter,
                    "the backend cannot present output {output}: its scan-out path is not built"
                )
            }
            Self::Unsupported(output) => {
                write!(
                    formatter,
                    "the backend has no gamma or power control for output {output}"
                )
            }
            Self::InvalidGamma(output) => {
                write!(
                    formatter,
                    "the gamma ramp for output {output} does not match its table size"
                )
            }
        }
    }
}

impl std::error::Error for BackendError {}
