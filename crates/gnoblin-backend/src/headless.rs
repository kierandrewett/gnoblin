//! The headless backend: a virtual output, no DRM, no GPU, no session.
//!
//! This is the backend CI and the integration tests run. It exists so protocol
//! behaviour can be exercised end to end — a real client, a real socket, the real
//! handlers, real pixels — on a machine with no display, and so the compositor has
//! one backend that starts anywhere.
//!
//! Each output owns an [`AnyRenderer`] drawing into an offscreen buffer, software unless the
//! configuration says otherwise, so a headless session produces the same pixels a DRM
//! session would: the difference is that nothing is scanned out, which is why
//! [`HeadlessTarget::present`] reports
//! [`Present::Nothing`] while still keeping the clipped damage of each present. The
//! buffer is what [`Backend::screenshot`] reads back.
//!
//! The virtual output's size is settable, which is how a test drives a mode change
//! and how the parity harness will run a session at a fixed size.
//!
//! ## Choosing the renderer
//!
//! [`HeadlessConfig`] holds the [`RendererKind`] every output draws with: software
//! (the default and the reference), [`Gles`] (the same frame drawn on a surfaceless GLES
//! context, D12's GPU half) or Vulkan (`gnoblin_render_vulkan::Vulkan`, behind the `vulkan` feature). The
//! `compositor.renderer` setting reaches this through [`HeadlessConfig::from_setting`], and
//! `auto` means [`RendererKind::AUTO`]. This file only holds the choice: building and
//! driving the renderer is [`AnyRenderer`]'s job.
//!
//! ## Driving a backend the session owns
//!
//! Once a session owns its backend a caller cannot reach it any more, so a test that
//! needs a hotplug, an unplug or a frame that is not presented holds a
//! [`HeadlessHandle`] taken before the session starts. The handle queues its requests
//! and [`Backend::dispatch`] applies them, which is the same path a DRM backend's udev
//! source takes when a monitor is plugged in: the compositor learns about it as a
//! [`BackendEvent`] and never as a call into the backend.
//!
//! A held present is a frame the backend did not put on the output: the host is not
//! asked to describe one and nothing is drawn. It exists because presentation is what
//! a locked session waits for — the lock may report itself locked only after every
//! output has drawn its cover — and a test can prove that by never presenting one.
//!
//! # Failure modes
//!
//! 1. **A frame is drawn on a target whose renderer was never created.** Constructing
//!    an output creates its buffer, and construction fails rather than deferring the
//!    failure to the first frame: see `# Panics` on [`Headless::push_output`].
//! 2. **A resize keeps the old pixels.** A mode change replaces the buffer, so the
//!    next frame is drawn against a buffer that holds nothing and the target counts a
//!    present with the new extent: `a_resize_replaces_the_pixels`.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_types::{OutputId, Rect, Scale, Size};
use smithay::reexports::calloop::LoopHandle;

use gnoblin_output::DisplayMode;

use gnoblin_config::RendererChoice;

use crate::render::renderer::{AnyRenderer, CaptureRequest, RendererKind, capture_redacted};
use crate::{
    Backend, BackendError, BackendEvent, DmabufCopy, DmabufCopyError, DmabufImportError, DmabufProvider, DmabufSource,
    DmabufSupport, GammaRamp, Host, InputEvent, InputSource, Output, OutputControls, Present, Target,
};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{OutputFrame, SecureTreatment};
use gnoblin_render_frame::image::Image;
use gnoblin_render_gles::Gles;

/// The number of entries in each gamma table a headless output has.
///
/// The size the kernel's legacy gamma interface uses for an ordinary CRTC, so a client that
/// sizes its ramp from the advertised size takes the same path it would on hardware.
pub const HEADLESS_GAMMA_SIZE: u32 = 256;

/// The colour state a headless backend was asked to apply, shared with its handle so a test
/// can read what reached the virtual hardware.
#[derive(Debug, Default)]
struct HeadlessControl {
    /// The gamma tables loaded per output, one per channel. No entry means the identity ramp.
    gamma: HashMap<OutputId, [Vec<u16>; 3]>,
    /// The power state set per output. No entry means the output is on.
    power: HashMap<OutputId, bool>,
    /// Mode changes still allowed before one is refused, or `None` for no injected failure. The refusal happens once:
    /// a rollback that follows it must succeed, or a test could not tell a restore from a stuck output.
    configure_budget: Option<u32>,
}

fn locked<T>(shared: &Mutex<T>) -> MutexGuard<'_, T> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The size a fresh headless session starts at.
///
/// Large enough that the window manager's default 800x600 window fits with room
/// for placement, and small enough that a test's screenshots and damage lists stay
/// readable.
pub const DEFAULT_MODE: DisplayMode = DisplayMode::new(1024, 768, 60_000);

/// The connector name the first virtual output gets.
///
/// It is a real connector-shaped name rather than a made-up one so that a
/// configuration document can address it and a log line reads like a session.
pub const DEFAULT_CONNECTOR: &str = "HEADLESS-1";

/// What a headless session starts with.
///
/// It is a value rather than a set of arguments so a caller that will grow properties — a
/// target scale, a connector name — keeps one name for "how this session starts" and one
/// place to add them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HeadlessConfig {
    /// The renderer every output draws with.
    pub renderer: RendererKind,
    /// What a capture draws in place of a secure layer.
    pub secure_treatment: SecureTreatment,
}

impl HeadlessConfig {
    /// The default configuration: one virtual output on the software path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The same configuration on the GLES path.
    #[must_use]
    pub fn gles() -> Self {
        Self {
            renderer: RendererKind::Gles,
            ..Self::default()
        }
    }

    /// The same configuration on the Vulkan path.
    #[must_use]
    pub fn vulkan() -> Self {
        Self {
            renderer: RendererKind::Vulkan,
            ..Self::default()
        }
    }

    /// The configuration a `compositor.renderer` setting selects.
    #[must_use]
    pub fn from_setting(setting: RendererChoice) -> Self {
        Self {
            renderer: RendererKind::from_setting(setting),
            ..Self::default()
        }
    }
}

/// A virtual output: an offscreen buffer, its damage and its present count.
///
/// The damage it keeps is clipped to its own extent, so a test can prove that a
/// window hanging off the output produced damage inside the output and nothing
/// outside it, and the software renderer relies on the same clipping.
pub struct HeadlessTarget {
    /// The logical size, which is what the compositor lays out against.
    size: Size,
    /// The physical size of the offscreen buffer.
    physical: Size,
    scale: Scale,
    presents: u64,
    damage: Vec<Rect>,
    renderer: AnyRenderer,
    /// The frame last drawn, kept only when it has a secure layer, so a capture can mask it.
    display: Option<OutputFrame>,
}

impl std::fmt::Debug for HeadlessTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HeadlessTarget")
            .field("size", &self.size)
            .field("physical", &self.physical)
            .field("scale", &self.scale)
            .field("presents", &self.presents)
            .field("damage", &self.damage)
            .field("renderer", &self.renderer())
            .finish_non_exhaustive()
    }
}

impl HeadlessTarget {
    /// A target of the given mode and scale.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the size covers no pixels or the software renderer
    /// cannot be created, which means the session cannot draw at all.
    pub fn new(mode: DisplayMode, scale: Scale) -> Result<Self, RenderError> {
        Self::with_renderer(RendererKind::Software, mode, scale)
    }

    /// A target of the given mode and scale, drawn by the given renderer.
    ///
    /// # Errors
    ///
    /// [`RenderError::Degenerate`] when the mode covers no pixels, and
    /// [`RenderError::Unavailable`] when the renderer cannot be created on this machine —
    /// which for [`RendererKind::Gles`] means no surfaceless EGL context.
    pub fn with_renderer(kind: RendererKind, mode: DisplayMode, scale: Scale) -> Result<Self, RenderError> {
        let physical = Size::new(mode.width, mode.height);
        let size = mode.logical_size(gnoblin_types::Orientation::Normal, scale);
        Ok(Self {
            size,
            physical,
            scale,
            presents: 0,
            damage: Vec::new(),
            renderer: AnyRenderer::new(kind, physical)?,
            display: None,
        })
    }

    /// The renderer this target draws with.
    #[must_use]
    pub fn renderer(&self) -> RendererKind {
        self.renderer.kind()
    }

    /// The GLES renderer, when this target draws with one.
    ///
    /// A test that wants the draw counter or a shader error reaches the renderer through
    /// this; the other renderers answer [`None`], because a question about a GL device has no
    /// answer without one.
    #[must_use]
    pub fn gles(&mut self) -> Option<&mut Gles> {
        self.renderer.gles_mut()
    }

    /// What this target's renderer imports of a client's GPU buffers; see [`AnyRenderer::dmabuf_support`].
    fn dmabuf_support(&mut self) -> Option<DmabufSupport> {
        self.renderer.dmabuf_support()
    }

    /// Imports a client's GPU buffer into this target's renderer.
    fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<(), DmabufImportError> {
        self.renderer.import_dmabuf(dmabuf)
    }

    /// Copies the frame last drawn into a client's GPU buffer, with each secure layer drawn as `treatment` says.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unsupported`] when the renderer draws into no client buffer, and the renderer's own error
    /// otherwise.
    fn copy_frame_into_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
        copy: DmabufCopy,
        with_cursor: bool,
        treatment: SecureTreatment,
    ) -> Result<(), RenderError> {
        let bounds = self.bounds();
        capture_redacted(
            &mut self.renderer,
            CaptureRequest {
                display: self.display.as_ref(),
                bounds,
                scale: self.scale,
                with_cursor,
                treatment,
            },
            AnyRenderer::draw,
            |renderer, cursor| renderer.copy_frame_into_dmabuf(dmabuf, copy, cursor),
        )
    }

    /// Writes a picture the CPU holds into a client's GPU buffer.
    fn copy_picture_into_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
        picture: &Image,
        copy: DmabufCopy,
    ) -> Result<(), RenderError> {
        self.renderer.copy_image_into_dmabuf(dmabuf, picture, copy)
    }

    /// Reads a client's GPU buffer back as a picture.
    fn read_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<Image, RenderError> {
        self.renderer.read_dmabuf(dmabuf)
    }

    /// Reads a View's picture back through this target's renderer, drawing the scene first when the renderer holds
    /// none of its current revision.
    fn read_view(
        &mut self,
        scene: &gnoblin_render_frame::view_scene::ViewScene,
        capture: bool,
        live: &[u64],
    ) -> Result<Image, RenderError> {
        self.renderer.view_pixels(scene, capture, live)
    }

    /// Gives back every View texture this target's renderer holds.
    pub fn release_views(&mut self) {
        self.renderer.release_views();
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

    /// The physical size of the offscreen buffer.
    #[must_use]
    pub fn physical_size(&self) -> Size {
        self.physical
    }

    /// Reads the offscreen buffer back, with each secure layer drawn as `treatment` says.
    ///
    /// # Errors
    ///
    /// [`RenderError::Failed`] when pixman cannot copy the buffer out.
    pub fn capture(&mut self, with_cursor: bool, treatment: SecureTreatment) -> Result<Image, RenderError> {
        let bounds = self.bounds();
        capture_redacted(
            &mut self.renderer,
            CaptureRequest {
                display: self.display.as_ref(),
                bounds,
                scale: self.scale,
                with_cursor,
                treatment,
            },
            AnyRenderer::draw,
            AnyRenderer::capture,
        )
    }

    /// The target's whole extent as a rectangle, for clipping.
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.size.width, self.size.height)
    }

    /// Replaces the target's extent and buffer after a mode change.
    fn resize(&mut self, mode: DisplayMode, scale: Scale) -> Result<(), RenderError> {
        self.renderer.resize(Size::new(mode.width, mode.height))?;
        self.physical = Size::new(mode.width, mode.height);
        self.size = mode.logical_size(gnoblin_types::Orientation::Normal, scale);
        self.scale = scale;
        self.damage.clear();
        Ok(())
    }
}

impl Target for HeadlessTarget {
    fn size(&self) -> Size {
        self.size
    }

    fn scale(&self) -> Scale {
        self.scale
    }

    fn draw(&mut self, frame: &OutputFrame) -> Result<(), RenderError> {
        self.renderer.draw(frame, self.scale)?;
        self.display = frame.has_secure().then(|| frame.clone());
        Ok(())
    }

    fn present(&mut self, damage: &[Rect]) -> Present {
        self.presents = self.presents.saturating_add(1);
        self.damage.clear();
        let bounds = self.bounds();
        self.damage
            .extend(damage.iter().filter_map(|rect| rect.intersection(bounds)));
        // Nothing is scanned out and no buffer is swapped, so the frame is complete
        // without a present. Saying otherwise would make a test believe a display is
        // attached.
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

/// What a [`HeadlessHandle`] asked the backend to do.
///
/// The list is drained by [`Backend::dispatch`], so every request takes the same path
/// a device event does and the compositor sees only its [`BackendEvent`]s.
#[derive(Debug, Default)]
struct HeadlessRequests {
    /// The presentation state the caller last set, if it set one.
    present: Option<bool>,
    /// Outputs to connect, with the connector, mode, scale and EDID identity to connect them at.
    connect: Vec<(String, DisplayMode, Scale, Option<String>)>,
    /// Outputs to disconnect.
    disconnect: Vec<OutputId>,
}

/// A caller's control over a headless backend a session already owns.
///
/// A test takes one with [`Headless::handle`] before handing the backend to a server,
/// and then hotplugs, unplugs or withholds presentation from its own thread while the
/// session runs.
#[derive(Clone, Debug)]
pub struct HeadlessHandle {
    requests: Arc<Mutex<HeadlessRequests>>,
    control: Arc<Mutex<HeadlessControl>>,
}

impl HeadlessHandle {
    /// Connects a new output on the next dispatch.
    ///
    /// # Errors
    ///
    /// [`BackendError::DegenerateMode`] when the mode covers no pixels; nothing is
    /// queued then, because an output that can show nothing is not an output.
    pub fn connect_output(&self, connector: &str, mode: DisplayMode, scale: Scale) -> Result<(), BackendError> {
        if mode.width == 0 || mode.height == 0 {
            return Err(BackendError::DegenerateMode {
                width: mode.width,
                height: mode.height,
            });
        }
        self.requests().connect.push((connector.to_owned(), mode, scale, None));
        Ok(())
    }

    /// [`Self::connect_output`] for a monitor that reports an EDID identity, so a saved profile can tell it from
    /// another monitor on the same connector.
    ///
    /// # Errors
    ///
    /// [`BackendError::DegenerateMode`] when the mode covers no pixels.
    pub fn connect_identified_output(
        &self,
        connector: &str,
        mode: DisplayMode,
        scale: Scale,
        identity: &str,
    ) -> Result<(), BackendError> {
        if mode.width == 0 || mode.height == 0 {
            return Err(BackendError::DegenerateMode {
                width: mode.width,
                height: mode.height,
            });
        }
        self.requests()
            .connect
            .push((connector.to_owned(), mode, scale, Some(identity.to_owned())));
        Ok(())
    }

    /// Makes the mode change after the next `allowed` ones fail once, as a driver that refuses a modeset half way
    /// through a configuration does. The change that fails and every one after it behave as before.
    pub fn refuse_configure_after(&self, allowed: u32) {
        locked(&self.control).configure_budget = Some(allowed);
    }

    /// Connects a 60 Hz output of `width` by `height` at scale one on the next dispatch: the output a test that needs
    /// no particular mode or scale asks for.
    ///
    /// # Errors
    ///
    /// [`BackendError::DegenerateMode`] when either side is zero.
    pub fn connect_virtual_output(&self, connector: &str, width: u32, height: u32) -> Result<(), BackendError> {
        self.connect_output(connector, DisplayMode::new(width, height, 60_000), Scale::integral(1))
    }

    /// Disconnects an output on the next dispatch.
    pub fn disconnect_output(&self, output: OutputId) {
        self.requests().disconnect.push(output);
    }

    /// Stops presenting frames, or resumes.
    ///
    /// A held frame is one the backend did not put on the output: the host is not asked
    /// to describe it and nothing is drawn, so a lock waiting for a presented cover
    /// keeps waiting.
    pub fn hold_presents(&self, hold: bool) {
        self.requests().present = Some(!hold);
    }

    /// The request queue, locked for writing.
    fn requests(&self) -> MutexGuard<'_, HeadlessRequests> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The gamma tables last loaded on `output`, one per channel, or [`None`] when it is at
    /// the identity ramp.
    #[must_use]
    pub fn applied_gamma(&self, output: OutputId) -> Option<[Vec<u16>; 3]> {
        locked(&self.control).gamma.get(&output).cloned()
    }

    /// Whether `output` is on. An output is on until a client switches it off.
    #[must_use]
    pub fn power(&self, output: OutputId) -> bool {
        locked(&self.control).power.get(&output).copied().unwrap_or(true)
    }
}

/// A headless backend: the outputs it drives and the events it has to report.
pub struct Headless {
    /// The outputs, and the target for each one, kept in step by every method that
    /// changes them. They are parallel vectors rather than a map so that
    /// [`Backend::outputs`] can hand out a slice in connection order.
    outputs: Vec<Output>,
    targets: Vec<HeadlessTarget>,
    /// State changes waiting for the next dispatch, at most one per output so a
    /// resize storm cannot queue stale sizes.
    pending: Vec<BackendEvent>,
    input: HeadlessInput,
    /// The frame description the host fills, kept here so a steady frame allocates
    /// nothing.
    frame: OutputFrame,
    /// The requests a [`HeadlessHandle`] queued, and the shared queue itself.
    requests: Arc<Mutex<HeadlessRequests>>,
    /// Whether [`Backend::render`] presents the frame it drew.
    presenting: bool,
    /// The next output identity. A counter, not the table length, so a hotplug after an
    /// unplug cannot name an output that already exists.
    next_output: u64,
    /// The renderer every output this backend connects draws with.
    renderer: RendererKind,
    /// What a capture draws in place of a secure layer, from the session's configuration.
    secure_treatment: SecureTreatment,
    /// The gamma and power state last applied, shared with the handle a test holds.
    control: Arc<Mutex<HeadlessControl>>,
    /// A stand-in for the renderers' answers to the dmabuf questions; see [`Headless::with_dmabuf_provider`].
    dmabuf: Option<Box<dyn DmabufProvider>>,
    /// What [`Backend::dmabuf`] reported, kept so each import checks a buffer against the formats
    /// clients were told.
    dmabuf_support: Option<DmabufSupport>,
    /// What [`Backend::dmabuf_capture_support`] and [`Backend::dmabuf_picture_support`] reported, kept so each copy
    /// checks a buffer against the formats a capture client was told.
    dmabuf_capture: Option<DmabufSupport>,
}

impl std::fmt::Debug for Headless {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Headless")
            .field("outputs", &self.outputs)
            .field("targets", &self.targets.len())
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

impl Headless {
    /// A backend with one virtual output at [`DEFAULT_MODE`].
    ///
    /// # Panics
    ///
    /// Panics if the virtual output's offscreen buffer cannot be created, which on a
    /// working system means the allocation failed. A headless session exists to draw,
    /// so a backend that cannot is not a session that can limp along.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(HeadlessConfig::default()).expect("a headless session with a software renderer")
    }

    /// A backend with one virtual output, drawn by the renderer `config` selects.
    ///
    /// # Errors
    ///
    /// [`RenderError::Unavailable`] when the selected renderer cannot be created on this
    /// machine: [`RendererKind::Gles`] asks for a surfaceless EGL context, and a headless
    /// session that cannot draw is better refused here than left to fail on its first frame.
    pub fn with_config(config: HeadlessConfig) -> Result<Self, RenderError> {
        let mut backend = Self::empty();
        backend.renderer = config.renderer;
        backend.secure_treatment = config.secure_treatment;
        backend.try_push_output(DEFAULT_CONNECTOR, DEFAULT_MODE, Scale::integral(1), None)?;
        Ok(backend)
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
            frame: OutputFrame::new(),
            requests: Arc::new(Mutex::new(HeadlessRequests::default())),
            presenting: true,
            next_output: 0,
            renderer: RendererKind::default(),
            secure_treatment: SecureTreatment::default(),
            control: Arc::new(Mutex::new(HeadlessControl::default())),
            dmabuf: None,
            dmabuf_support: None,
            dmabuf_capture: None,
        }
    }

    /// Another device's answers to the dmabuf questions, in place of this session's renderers'.
    ///
    /// A session with no GPU can serve `zwp_linux_dmabuf_v1` this way: the protocol layer is the
    /// real one and only the importer is the provider's. A test uses it where no device exists; see
    /// `crates/gnoblin-wayland/tests/dmabuf.rs`.
    #[must_use]
    pub fn with_dmabuf_provider(mut self, provider: Box<dyn DmabufProvider>) -> Self {
        self.dmabuf = Some(provider);
        self
    }

    /// What the session can import of a client's GPU buffers: the provider's answer, or the
    /// first output's renderer's. `None` when nothing imports (the software renderer, or no output).
    fn query_dmabuf(&mut self) -> Option<DmabufSupport> {
        let support = match self.dmabuf.as_mut() {
            Some(provider) => provider.support(),
            None => self.targets.first_mut().and_then(HeadlessTarget::dmabuf_support),
        };
        self.dmabuf_support.clone_from(&support);
        support
    }

    /// Imports `dmabuf` after the checks that need no driver: a format and modifier the clients
    /// were not offered, and a plane count the modifier cannot have.
    fn import(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<(), DmabufImportError> {
        self.dmabuf_support
            .as_ref()
            .ok_or(DmabufImportError::Unsupported)?
            .check(dmabuf)?;
        match self.dmabuf.as_mut() {
            Some(provider) => provider.import(dmabuf),
            None => self
                .targets
                .first_mut()
                .ok_or(DmabufImportError::Unsupported)?
                .import_dmabuf(dmabuf),
        }
    }

    /// What a capture may be written into: the provider's answer, or the first output's renderer's. `None` when
    /// nothing can be written into (the software renderer, or no output). Remembered, so each copy checks a
    /// buffer against the formats clients were told.
    fn query_capture(&mut self) -> Option<DmabufSupport> {
        let support = match self.dmabuf.as_mut() {
            Some(provider) => provider.capture_support(),
            None => self.targets.first_mut().and_then(HeadlessTarget::dmabuf_support),
        };
        self.dmabuf_capture.clone_from(&support);
        support
    }

    /// Copies `source` into `dmabuf`, after the checks that need no driver: a format and modifier a capture did not
    /// offer, and a plane count the modifier cannot have.
    fn copy_into_dmabuf(
        &mut self,
        source: DmabufSource<'_>,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
        copy: DmabufCopy,
    ) -> Result<(), DmabufCopyError> {
        self.dmabuf_capture
            .as_ref()
            .ok_or(DmabufCopyError::Unsupported)?
            .check(dmabuf)?;
        let treatment = self.secure_treatment;
        match source {
            DmabufSource::Frame { output, with_cursor } => {
                let index = self.index(output).ok_or(DmabufCopyError::NoFrame)?;
                let target = self.targets.get_mut(index).ok_or(DmabufCopyError::NoFrame)?;
                match self.dmabuf.as_mut() {
                    // The stand-in has no frame of its own: it is handed the one the output's renderer drew.
                    Some(provider) => {
                        let frame = target.capture(with_cursor, treatment).map_err(DmabufCopyError::from)?;
                        provider.write(dmabuf, &frame, copy)
                    }
                    None => target
                        .copy_frame_into_dmabuf(dmabuf, copy, with_cursor, treatment)
                        .map_err(DmabufCopyError::from),
                }
            }
            DmabufSource::Picture(picture) => match self.dmabuf.as_mut() {
                Some(provider) => provider.write(dmabuf, picture, copy),
                None => self
                    .targets
                    .first_mut()
                    .ok_or(DmabufCopyError::Unsupported)?
                    .copy_picture_into_dmabuf(dmabuf, picture, copy)
                    .map_err(DmabufCopyError::from),
            },
        }
    }

    /// Reads a client's GPU buffer back as a picture.
    fn read_from_dmabuf(
        &mut self,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Result<Image, DmabufCopyError> {
        match self.dmabuf.as_mut() {
            Some(provider) => provider.read(dmabuf),
            None => self
                .targets
                .first_mut()
                .ok_or(DmabufCopyError::Unsupported)?
                .read_dmabuf(dmabuf)
                .map_err(DmabufCopyError::from),
        }
    }

    /// A handle that can hotplug, unplug and withhold presentation after a session has
    /// taken this backend.
    #[must_use]
    pub fn handle(&self) -> HeadlessHandle {
        HeadlessHandle {
            requests: Arc::clone(&self.requests),
            control: Arc::clone(&self.control),
        }
    }

    /// Adds an output and its target.
    ///
    /// # Panics
    ///
    /// Panics if `mode` covers no pixels, or if the backend's renderer cannot create an
    /// offscreen buffer for it. A renderer this machine cannot create at all is refused by
    /// [`Headless::with_config`], so this panic means the allocation failed. The constructor is for callers that have a size to hand; a runtime
    /// request goes through [`Self::resize`], which reports the same problems as a
    /// [`BackendError`].
    pub fn push_output(&mut self, connector: &str, mode: DisplayMode, scale: Scale) {
        self.try_push_output(connector, mode, scale, None)
            .expect("a virtual output whose renderer this machine can create");
    }

    /// [`Self::push_output`], reporting a renderer that cannot be created instead of panicking.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when the renderer cannot create the output's buffer. The backend is
    /// unchanged in that case.
    ///
    /// # Panics
    ///
    /// Panics if `mode` covers no pixels.
    fn try_push_output(
        &mut self,
        connector: &str,
        mode: DisplayMode,
        scale: Scale,
        identity: Option<String>,
    ) -> Result<(), RenderError> {
        assert!(
            mode.width > 0 && mode.height > 0,
            "a mode that covers no pixels cannot be an output: {mode:?}"
        );
        let target = HeadlessTarget::with_renderer(self.renderer, mode, scale)?;
        let id = OutputId::new(self.next_output);
        self.next_output = self.next_output.saturating_add(1);
        let output = Output::new(id, connector, mode, scale).with_identity(identity);
        self.outputs.push(output.clone());
        self.targets.push(target);
        self.pending.push(BackendEvent::OutputConnected(output));
        Ok(())
    }

    /// Removes an output and its target, reporting the change on the next dispatch.
    ///
    /// # Errors
    ///
    /// [`BackendError::UnknownOutput`] when the id is not this backend's.
    pub fn remove_output(&mut self, output: OutputId) -> Result<(), BackendError> {
        let Some(index) = self.index(output) else {
            return Err(BackendError::UnknownOutput(output));
        };
        self.outputs.remove(index);
        self.targets.remove(index);
        self.pending.retain(|event| {
            !matches!(
                event,
                BackendEvent::OutputResized { output: changed, .. } if *changed == output
            )
        });
        self.pending.push(BackendEvent::OutputDisconnected(output));
        Ok(())
    }

    /// Sets the mode and scale of an output, reporting the change on the next
    /// dispatch.
    ///
    /// # Errors
    ///
    /// [`BackendError::UnknownOutput`] when the id is not this backend's,
    /// [`BackendError::DegenerateMode`] when the mode covers no pixels, and
    /// [`BackendError::Render`] when the new buffer cannot be created.
    pub fn resize(&mut self, output: OutputId, mode: DisplayMode, scale: Scale) -> Result<(), BackendError> {
        let Some(index) = self.index(output) else {
            return Err(BackendError::UnknownOutput(output));
        };
        if mode.width == 0 || mode.height == 0 {
            return Err(BackendError::DegenerateMode {
                width: mode.width,
                height: mode.height,
            });
        }
        let Some(target) = self.targets.get_mut(index) else {
            return Err(BackendError::UnknownOutput(output));
        };
        target.resize(mode, scale).map_err(BackendError::Render)?;
        let Some(entry) = self.outputs.get_mut(index) else {
            return Err(BackendError::UnknownOutput(output));
        };
        entry.mode = mode;
        entry.scale = scale;
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

    /// The target for an output, for a test that wants its damage or its pixels.
    #[must_use]
    pub fn target_of(&self, output: OutputId) -> Option<&HeadlessTarget> {
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

    /// The pixels of the last frame drawn on an output, read back from its buffer.
    ///
    /// It is an inherent method as well as a [`Backend`] one because a readback does
    /// not depend on the compositor type the backend drives, and a caller should not
    /// have to name one to take a screenshot.
    #[must_use]
    pub fn capture(&mut self, output: OutputId, with_cursor: bool) -> Option<Image> {
        let index = self.index(output)?;
        let target = self.targets.get_mut(index)?;
        target.capture(with_cursor, self.secure_treatment).ok()
    }

    /// Applies the requests a [`HeadlessHandle`] queued, as the next dispatch.
    fn apply_requests(&mut self) {
        let requests = std::mem::take(&mut *self.requests.lock().unwrap_or_else(PoisonError::into_inner));
        if let Some(present) = requests.present {
            self.presenting = present;
        }
        for output in requests.disconnect {
            // A caller that unplugs an output twice is not an error: the second request
            // names nothing and reports nothing.
            let _ = self.remove_output(output);
        }
        for (connector, mode, scale, identity) in requests.connect {
            self.try_push_output(&connector, mode, scale, identity)
                .expect("a virtual output whose renderer this machine can create");
        }
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
        let index = self.index(output).ok_or(BackendError::UnknownOutput(output))?;
        if !self.presenting {
            // A held present: the host is not asked to describe a frame and the output
            // stays as it was, so nothing reaches it.
            return Ok(Present::Nothing);
        }
        if !host.frame(output, &mut self.frame) {
            // Nothing changed, so nothing is drawn or presented. This is what makes an
            // idle iteration free.
            return Ok(Present::Nothing);
        }
        let Some(target) = self.targets.get_mut(index) else {
            return Err(BackendError::UnknownOutput(output));
        };
        target.draw(&self.frame).map_err(BackendError::Render)?;
        Ok(target.present(self.frame.damage()))
    }

    fn capture(&mut self, output: OutputId, with_cursor: bool) -> Option<Image> {
        Self::capture(self, output, with_cursor)
    }

    fn dispatch(&mut self, host: &mut S) -> Vec<BackendEvent> {
        while let Some(event) = self.input.next_event() {
            host.input(event);
        }
        self.apply_requests();
        std::mem::take(&mut self.pending)
    }

    fn secure_treatment(&self) -> SecureTreatment {
        self.secure_treatment
    }

    fn effect_kinds(&self) -> &'static [&'static str] {
        self.renderer.effect_kinds()
    }

    fn dmabuf(&mut self) -> Option<DmabufSupport> {
        self.query_dmabuf()
    }

    fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<(), DmabufImportError> {
        self.import(dmabuf)
    }

    fn dmabuf_capture_support(&mut self, output: OutputId) -> Option<DmabufSupport> {
        // An output this session does not drive has no frame to export.
        self.index(output)?;
        self.query_capture()
    }

    fn dmabuf_picture_support(&mut self) -> Option<DmabufSupport> {
        self.query_capture()
    }

    fn copy_to_dmabuf(
        &mut self,
        source: DmabufSource<'_>,
        dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
        copy: DmabufCopy,
    ) -> Result<(), DmabufCopyError> {
        self.copy_into_dmabuf(source, dmabuf, copy)
    }

    fn read_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<Image, DmabufCopyError> {
        self.read_from_dmabuf(dmabuf)
    }

    fn gpu_views(&self) -> bool {
        self.renderer.draws_views()
    }

    fn read_view(
        &mut self,
        scene: &gnoblin_render_frame::view_scene::ViewScene,
        capture: bool,
        live: &[u64],
    ) -> Result<Image, RenderError> {
        self.targets
            .first_mut()
            .ok_or(RenderError::Unsupported)?
            .read_view(scene, capture, live)
    }

    fn release_views(&mut self) {
        for target in &mut self.targets {
            target.release_views();
        }
    }

    fn output_controls(&self) -> OutputControls {
        OutputControls {
            gamma: true,
            power: true,
        }
    }

    fn gamma_size(&self, output: OutputId) -> Option<u32> {
        self.index(output).map(|_| HEADLESS_GAMMA_SIZE)
    }

    fn set_gamma(&mut self, output: OutputId, ramp: GammaRamp<'_>) -> Result<(), BackendError> {
        self.index(output).ok_or(BackendError::UnknownOutput(output))?;
        let fits = |table: &[u16]| u32::try_from(table.len()) == Ok(HEADLESS_GAMMA_SIZE);
        if ![ramp.red, ramp.green, ramp.blue].into_iter().all(fits) {
            return Err(BackendError::InvalidGamma(output));
        }
        let tables = [ramp.red.to_vec(), ramp.green.to_vec(), ramp.blue.to_vec()];
        locked(&self.control).gamma.insert(output, tables);
        Ok(())
    }

    fn restore_gamma(&mut self, output: OutputId) -> Result<(), BackendError> {
        self.index(output).ok_or(BackendError::UnknownOutput(output))?;
        locked(&self.control).gamma.remove(&output);
        Ok(())
    }

    fn set_power(&mut self, output: OutputId, on: bool) -> Result<(), BackendError> {
        self.index(output).ok_or(BackendError::UnknownOutput(output))?;
        locked(&self.control).power.insert(output, on);
        Ok(())
    }

    fn configure_output(&mut self, output: OutputId, mode: DisplayMode, scale: Scale) -> Result<(), BackendError> {
        {
            let mut control = locked(&self.control);
            match control.configure_budget {
                Some(0) => {
                    control.configure_budget = None;
                    return Err(BackendError::Unsupported(output));
                }
                Some(left) => control.configure_budget = Some(left - 1),
                None => {}
            }
        }
        self.resize(output, mode, scale)
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_types::Point;

    use super::*;
    use gnoblin_render_frame::frame::{Layer, Overlay, Solid};
    use gnoblin_types::DeviceId;

    /// A host that describes one red rectangle and records what it was asked.
    #[derive(Default)]
    struct Recorder {
        /// The outputs the host was asked to describe.
        described: Vec<OutputId>,
        inputs: Vec<InputEvent>,
        /// What to describe: a changed frame or nothing at all.
        changed: bool,
        /// The damage the last description held, for the tests to read back.
        damage: Vec<Rect>,
    }

    impl Host for Recorder {
        fn begin_frame(&mut self) {}

        fn frame(&mut self, output: OutputId, frame: &mut OutputFrame) -> bool {
            self.described.push(output);
            self.damage = frame.damage().to_vec();
            if !self.changed {
                return false;
            }
            frame.set_background(gnoblin_types::Color::BLACK);
            frame.push_damage(Rect::new(0, 0, 4, 4));
            frame.push_layer(Layer::Solid(Solid::new(
                Rect::new(0, 0, 4, 4),
                gnoblin_types::Color::from_rgba8(255, 0, 0, 255),
            )));
            true
        }

        fn input(&mut self, event: InputEvent) {
            self.inputs.push(event);
        }
    }

    fn changing() -> Recorder {
        Recorder {
            changed: true,
            ..Recorder::default()
        }
    }

    #[test]
    fn rendering_an_unknown_output_is_refused() {
        let mut backend = Headless::new();
        let mut host = changing();
        let error = backend.render(&mut host, OutputId::new(9)).expect_err("unknown output");
        assert_eq!(error, BackendError::UnknownOutput(OutputId::new(9)));
        assert!(
            host.described.is_empty(),
            "the host was asked to describe a gone output"
        );
    }

    #[test]
    fn a_resize_storm_delivers_one_event_with_the_latest_size() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        for step in 1..=100 {
            let mode = DisplayMode::new(640 + step, 480 + step, 60_000);
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
                mode: DisplayMode::new(740, 580, 60_000),
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
            .resize(output, DisplayMode::new(0, 480, 60_000), Scale::integral(1))
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
        let mut target = HeadlessTarget::new(DEFAULT_MODE, Scale::integral(1)).expect("a virtual output");
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
        let mut host = changing();
        for raw in 0..4 {
            let error = backend
                .render(&mut host, OutputId::new(raw))
                .expect_err("there is no output to draw on");
            assert_eq!(error, BackendError::UnknownOutput(OutputId::new(raw)));
        }
        assert!(
            host.described.is_empty(),
            "the host described an output that does not exist"
        );
        assert!(backend.dispatch(&mut host).is_empty());
    }

    #[test]
    fn queued_input_reaches_the_host_with_no_output_connected() {
        let mut backend = Headless::empty();
        backend.push_input(InputEvent::PointerLeave);
        backend.push_input(InputEvent::Key {
            device: DeviceId::VIRTUAL,
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
                    device: DeviceId::VIRTUAL,
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
        let mut host = changing();
        assert_eq!(
            backend.render(&mut host, output),
            Ok(Present::Nothing),
            "the headless target presents no pixels to a display"
        );
        assert_eq!(host.described, vec![output]);
        let target = backend.target_of(output).expect("the output has a target");
        assert_eq!(target.presents(), 1, "one present for the one frame that changed");
        assert_eq!(target.size(), Size::new(1024, 768));
        assert_eq!(target.damage(), &[Rect::new(0, 0, 4, 4)]);
        let image = backend.capture(output, false).expect("a readback");
        assert_eq!(
            image.colour_at(1, 1),
            Some([255, 0, 0, 255]),
            "the frame the host described is in the buffer"
        );
    }

    /// A host that describes a secure red square, with an ordinary blue overlay beside it.
    struct SecureScene;

    impl Host for SecureScene {
        fn begin_frame(&mut self) {}

        fn frame(&mut self, _output: OutputId, frame: &mut OutputFrame) -> bool {
            frame.set_background(gnoblin_types::Color::BLACK);
            frame.push_damage(Rect::new(0, 0, 8, 8));
            let mut secret = Solid::new(Rect::new(0, 0, 4, 4), gnoblin_types::Color::from_rgba8(255, 0, 0, 255));
            secret.secure = true;
            frame.push_layer(Layer::Solid(secret));
            let blue = Image::new(Size::new(2, 2), 8, [0, 0, 255, 255].repeat(4)).expect("a 2x2 image");
            let overlay = Overlay::new(Rect::new(4, 4, 2, 2), &blue).expect("an overlay");
            frame.push_layer(Layer::Overlay(overlay));
            true
        }

        fn input(&mut self, _event: InputEvent) {}
    }

    #[test]
    fn a_capture_masks_a_secure_layer_and_keeps_an_ordinary_overlay() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        let mut host = SecureScene;
        assert_eq!(backend.render(&mut host, output), Ok(Present::Nothing));
        let image = backend.capture(output, false).expect("a readback");
        let [red, green, blue, alpha] = image.colour_at(1, 1).expect("a pixel inside the secure layer");
        assert!(
            red == green && green == blue && alpha == 255 && (100..=155).contains(&red),
            "the secure layer reads as the neutral mask, not red"
        );
        assert_eq!(
            image.colour_at(5, 5),
            Some([0, 0, 255, 255]),
            "an ordinary overlay is in a capture"
        );
    }

    #[test]
    fn a_capture_omits_a_secure_layer_when_the_session_says_so() {
        let mut backend = Headless::with_config(HeadlessConfig {
            secure_treatment: SecureTreatment::Omit,
            ..HeadlessConfig::new()
        })
        .expect("a software headless session");
        let output = backend.outputs[0].id;
        let mut host = SecureScene;
        assert_eq!(backend.render(&mut host, output), Ok(Present::Nothing));
        let image = backend.capture(output, false).expect("a readback");
        assert_eq!(
            image.colour_at(1, 1),
            Some([0, 0, 0, 255]),
            "the omitted layer leaves the black background showing"
        );
        assert_eq!(
            image.colour_at(5, 5),
            Some([0, 0, 255, 255]),
            "an ordinary overlay is in a capture"
        );
    }

    #[test]
    fn a_headless_session_can_draw_with_the_gles_renderer() {
        // The renderer is a configuration choice, and the two paths must be
        // interchangeable: the same host frame, the same probe, the same pixel.
        let Ok(mut backend) = Headless::with_config(HeadlessConfig::gles()) else {
            println!("skipping the GLES headless test: no surfaceless EGL context on this machine");
            return;
        };
        assert_eq!(
            RendererKind::default(),
            RendererKind::Software,
            "the default stays software"
        );
        let output = backend.outputs[0].id;
        let mut host = changing();
        assert_eq!(backend.render(&mut host, output), Ok(Present::Nothing));
        let image = backend.capture(output, false).expect("a readback");
        assert_eq!(
            image.colour_at(1, 1),
            Some([255, 0, 0, 255]),
            "the GLES path draws the same frame the software path does"
        );
        let target = backend.target_of(output).expect("the output has a target");
        assert_eq!(target.renderer(), RendererKind::Gles);
        assert_eq!(
            target.presents(),
            1,
            "a headless target reports no present, whichever renderer drew"
        );
    }

    /// The lock releases every View texture: a readback made one, and giving the views back leaves the renderer
    /// holding none, so a View read afterwards is drawn again from the scene it has then.
    #[test]
    fn releasing_the_views_gives_back_every_texture_a_readback_made() {
        use gnoblin_render_frame::view_scene::ViewScene;
        let Ok(mut backend) = Headless::with_config(HeadlessConfig::gles()) else {
            println!("skipping the GLES headless test: no surfaceless EGL context on this machine");
            return;
        };
        let scene = ViewScene::new(1, 1, Size::new(8, 8), Vec::new(), None);
        let target = backend.targets.first_mut().expect("a target");
        target.read_view(&scene, true, &[1]).expect("a blank scene reads");
        let held =
            |target: &mut HeadlessTarget| target.renderer.gles_mut().expect("a GLES renderer").subtree_textures();
        assert_eq!(held(target), 1, "the readback made a texture");
        target.release_views();
        assert_eq!(held(target), 0, "releasing the views gave it back");
        target.read_view(&scene, true, &[1]).expect("drawn again");
        assert_eq!(held(target), 1, "and a later read draws the View again");
    }

    #[test]
    fn a_renderer_setting_reaches_the_headless_configuration() {
        assert_eq!(
            HeadlessConfig::from_setting(RendererChoice::Auto),
            HeadlessConfig::new(),
            "auto is what a session that says nothing gets"
        );
        assert_eq!(
            HeadlessConfig::from_setting(RendererChoice::Gles),
            HeadlessConfig::gles()
        );
        assert_eq!(
            HeadlessConfig::from_setting(RendererChoice::Vulkan),
            HeadlessConfig::vulkan()
        );
    }

    #[cfg(feature = "vulkan")]
    #[test]
    fn a_headless_session_can_draw_with_the_vulkan_renderer() {
        let Ok(mut backend) = Headless::with_config(HeadlessConfig::vulkan()) else {
            println!("skipping the Vulkan headless test: no Vulkan loader or device on this machine");
            return;
        };
        let output = backend.outputs[0].id;
        let mut host = changing();
        assert_eq!(backend.render(&mut host, output), Ok(Present::Nothing));
        let image = backend.capture(output, false).expect("a readback");
        assert_eq!(
            image.colour_at(1, 1),
            Some([255, 0, 0, 255]),
            "the Vulkan path draws the same frame the software path does"
        );
        let target = backend.target_of(output).expect("the output has a target");
        assert_eq!(target.renderer(), RendererKind::Vulkan);
    }

    #[test]
    fn a_backend_that_changed_nothing_draws_nothing() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        let mut host = Recorder::default();
        assert_eq!(backend.render(&mut host, output), Ok(Present::Nothing));
        let target = backend.target_of(output).expect("the output has a target");
        assert_eq!(target.presents(), 0, "an idle iteration does not present");
        assert!(target.damage().is_empty());
        let image = backend.capture(output, false).expect("a readback");
        assert_eq!(
            image.colour_at(0, 0),
            Some([0, 0, 0, 0]),
            "nothing was drawn, so the buffer holds no pixels"
        );
    }

    #[test]
    fn a_screenshot_of_an_unknown_output_is_none() {
        let mut backend = Headless::new();
        assert!(backend.capture(OutputId::new(4), false).is_none());
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
    fn a_hotplug_from_a_handle_is_reported_once() {
        let mut backend = Headless::new();
        let mut host = changing();
        let started = backend.dispatch(&mut host);
        assert_eq!(started.len(), 1, "the start-up output: {started:#?}");
        let handle = backend.handle();
        handle
            .connect_output("HEADLESS-2", DisplayMode::new(800, 600, 60_000), Scale::integral(1))
            .expect("a mode with pixels");
        let events = backend.dispatch(&mut host);
        assert_eq!(events.len(), 1, "one output connected: {events:#?}");
        assert_eq!(
            backend.outputs.len(),
            2,
            "the backend drives the output it was asked for"
        );
        assert!(backend.dispatch(&mut host).is_empty(), "a hotplug is reported once");
    }

    #[test]
    fn a_hotplug_after_an_unplug_does_not_reuse_an_identity() {
        let mut backend = Headless::new();
        let first = backend.outputs[0].id;
        let handle = backend.handle();
        handle.disconnect_output(first);
        handle
            .connect_output("HEADLESS-2", DisplayMode::new(800, 600, 60_000), Scale::integral(1))
            .expect("a mode with pixels");
        let mut host = changing();
        backend.dispatch(&mut host);
        let connected: Vec<OutputId> = backend.outputs.iter().map(|output| output.id).collect();
        assert_eq!(connected.len(), 1);
        assert_ne!(
            connected[0], first,
            "an identity names one output for the life of the backend"
        );
        assert!(
            backend.target_of(first).is_none(),
            "the unplugged output left no target behind"
        );
    }

    #[test]
    fn a_held_present_keeps_the_output_as_it_was() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        let presents = backend.target_of(output).map(HeadlessTarget::presents);
        backend.handle().hold_presents(true);
        let mut host = changing();
        backend.dispatch(&mut host);
        assert_eq!(backend.render(&mut host, output), Ok(Present::Nothing));
        assert!(host.described.is_empty(), "a held frame is not described");
        assert_eq!(
            backend.target_of(output).map(HeadlessTarget::presents),
            presents,
            "a held frame is not presented"
        );

        backend.handle().hold_presents(false);
        backend.dispatch(&mut host);
        assert_eq!(backend.render(&mut host, output), Ok(Present::Nothing));
        assert_eq!(host.described, vec![output], "a resumed present reaches the host");
    }

    #[test]
    fn a_pointer_position_converts_to_the_outputs_logical_space() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        backend.push_input(InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output,
            position: Point::new(10, 20),
        });
        let mut host = Recorder::default();
        backend.dispatch(&mut host);
        assert_eq!(
            host.inputs,
            vec![InputEvent::PointerMotion {
                device: DeviceId::VIRTUAL,
                output,
                position: Point::new(10, 20),
            }]
        );
    }

    #[test]
    fn a_resize_replaces_the_pixels() {
        let mut backend = Headless::new();
        let output = backend.outputs[0].id;
        let mut host = changing();
        backend.render(&mut host, output).expect("the first frame");
        backend
            .resize(output, DisplayMode::new(64, 48, 60_000), Scale::integral(1))
            .expect("a new mode");
        let target = backend.target_of(output).expect("the output has a target");
        assert_eq!(target.physical_size(), Size::new(64, 48));
        assert_eq!(target.size(), Size::new(64, 48));
        backend.render(&mut host, output).expect("the frame after the resize");
        let image = backend.capture(output, false).expect("a readback");
        assert_eq!(
            image.size(),
            Size::new(64, 48),
            "the readback follows the new buffer, not the old one"
        );
    }

    /// A `Dmabuf` of `size` in `format`, with `planes` planes of `/dev/null`: the shape of a GPU buffer.
    fn fake_dmabuf(
        format: smithay::backend::allocator::Fourcc,
        modifier: smithay::backend::allocator::Modifier,
        planes: u32,
    ) -> smithay::backend::allocator::dmabuf::Dmabuf {
        use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
        let mut builder = Dmabuf::builder((16, 16), format, modifier, DmabufFlags::empty());
        for _ in 0..planes {
            let fd = std::os::fd::OwnedFd::from(std::fs::File::open("/dev/null").expect("/dev/null opens"));
            assert!(builder.add_plane(fd, 0, 64));
        }
        builder.build().expect("a dmabuf with a plane")
    }

    /// An importer that accepts everything and remembers how many buffers it saw.
    struct Counting(Arc<Mutex<u32>>);

    impl DmabufProvider for Counting {
        fn support(&mut self) -> Option<DmabufSupport> {
            use smithay::backend::allocator::{Format, Fourcc, Modifier};
            Some(DmabufSupport::new(
                None,
                [Format {
                    code: Fourcc::Argb8888,
                    modifier: Modifier::Linear,
                }],
            ))
        }

        fn import(&mut self, _dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> Result<(), DmabufImportError> {
            *self.0.lock().expect("the counter") += 1;
            Ok(())
        }
    }

    #[test]
    fn the_software_renderer_offers_no_dmabuf() {
        use smithay::backend::allocator::{Fourcc, Modifier};
        let mut backend = Headless::new();
        assert!(
            backend.query_dmabuf().is_none(),
            "pixman reads CPU memory: it cannot draw a GPU buffer, so nothing is offered"
        );
        assert_eq!(
            backend.import(&fake_dmabuf(Fourcc::Argb8888, Modifier::Linear, 1)),
            Err(DmabufImportError::Unsupported),
            "and nothing is imported"
        );
        assert!(
            Headless::empty().query_dmabuf().is_none(),
            "a session with no output has no renderer to ask"
        );
    }

    #[test]
    fn an_import_checks_the_offer_and_the_planes_before_the_importer() {
        use smithay::backend::allocator::{Fourcc, Modifier};
        let seen = Arc::new(Mutex::new(0));
        let mut backend = Headless::new().with_dmabuf_provider(Box::new(Counting(Arc::clone(&seen))));
        assert!(
            backend.query_dmabuf().is_some(),
            "the provider's answer replaces the renderer's"
        );

        assert_eq!(
            backend.import(&fake_dmabuf(Fourcc::Argb8888, Modifier::Linear, 1)),
            Ok(()),
            "an offered pair of one plane is imported"
        );
        assert_eq!(
            backend.import(&fake_dmabuf(Fourcc::Argb8888, Modifier::Invalid, 1)),
            Err(DmabufImportError::NotOffered),
            "the same code with another modifier is a different pair"
        );
        assert_eq!(
            backend.import(&fake_dmabuf(Fourcc::Abgr8888, Modifier::Linear, 1)),
            Err(DmabufImportError::NotOffered),
            "a format that was never offered"
        );
        assert_eq!(
            backend.import(&fake_dmabuf(Fourcc::Argb8888, Modifier::Linear, 2)),
            Err(DmabufImportError::BadPlanes),
            "a linear buffer cannot have a second plane"
        );
        assert_eq!(
            *seen.lock().expect("the counter"),
            1,
            "only the first reached the importer"
        );
    }

    #[test]
    fn a_gles_session_offers_what_its_display_imports() {
        let Ok(mut backend) = Headless::with_config(HeadlessConfig::gles()) else {
            println!("skipping: no surfaceless EGL context on this machine");
            return;
        };
        let formats = backend
            .targets
            .first_mut()
            .and_then(HeadlessTarget::gles)
            .map(Gles::dmabuf_formats)
            .expect("a GLES target");
        let offered = backend.query_dmabuf();
        println!("the GLES session offers {} dmabuf pair(s)", formats.len());
        assert_eq!(
            offered.is_some(),
            !formats.is_empty(),
            "the session serves dmabuf exactly when its display can import a format"
        );
    }
}
