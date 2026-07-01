use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::ffi::{c_void, CStr};
use std::io::ErrorKind;
use std::num::NonZeroU32;
use std::os::fd::AsRawFd;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use khronos_egl as egl;
use slint::platform::femtovg_renderer::{FemtoVGRenderer, OpenGLInterface};
use slint::platform::{Platform, PointerEventButton, Renderer, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, LogicalSize, PhysicalSize};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    backend::WaylandError,
    globals::{registry_queue_init, GlobalList},
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, EventQueue, Proxy, QueueHandle,
};

use gnoblin_core::{runtime_error, ClientArgs, RuntimeError};

const CONFIGURE_RENDER_DELAY: Duration = Duration::from_millis(50);
const IDLE_DISPATCH_TIMEOUT: Duration = Duration::from_millis(200);
const APP_TICK_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PopoutHandle(u32);

#[derive(Clone)]
pub struct RuntimeControl {
    commands: Rc<RefCell<Vec<RuntimeCommand>>>,
    next_popout_id: Rc<Cell<u32>>,
}

enum RuntimeCommand {
    OpenPopout(PopoutHandle, PopoutConfig),
    ClosePopout(PopoutHandle),
    ConfigurePopout(PopoutHandle, u32, u32, BarMargins),
}

/// Read the visual properties of a Slint item if it's a (Border)Rectangle: the
/// per-corner border radius, border width, and the background + border colours
/// (ARGB bytes). Returns a JSON fragment (leading comma) or None. Uses the
/// unstable item downcast + render-trait accessors.
fn slint_item_props(item: &i_slint_core::item_tree::ItemRc) -> Option<String> {
    use i_slint_core::items::{BorderRectangle, ComplexText, ItemRef, Rectangle, SimpleText};
    use std::fmt::Write;

    let mut out = String::new();
    if let Some(br) = ItemRef::downcast_pin::<BorderRectangle>(item.borrow()) {
        let bg = br.background().color();
        let bc = br.border_color().color();
        let _ = write!(
            out,
            ",\"radius\":{:.1},\"border_w\":{:.1},\
             \"bg\":[{},{},{},{}],\"border_col\":[{},{},{},{}]",
            br.border_radius().get(),
            br.border_width().get(),
            bg.red(),
            bg.green(),
            bg.blue(),
            bg.alpha(),
            bc.red(),
            bc.green(),
            bc.blue(),
            bc.alpha(),
        );
        return Some(out);
    }
    if let Some(rect) = ItemRef::downcast_pin::<Rectangle>(item.borrow()) {
        let bg = rect.background().color();
        let _ = write!(
            out,
            ",\"bg\":[{},{},{},{}]",
            bg.red(),
            bg.green(),
            bg.blue(),
            bg.alpha()
        );
        return Some(out);
    }
    // Text comes in two item flavours (SimpleText / ComplexText) with the same
    // text/font_size/font_weight/colour Property fields. Read them directly via
    // FIELD_OFFSETS (the text()/color() methods are ambiguous between inherent and
    // RenderText-trait versions).
    macro_rules! try_text {
        ($ty:ty) => {
            if let Some(t) = ItemRef::downcast_pin::<$ty>(item.borrow()) {
                let text = <$ty>::FIELD_OFFSETS.text().apply_pin(t).get();
                let fs = <$ty>::FIELD_OFFSETS.font_size().apply_pin(t).get().get();
                let fw = <$ty>::FIELD_OFFSETS.font_weight().apply_pin(t).get();
                let col = <$ty>::FIELD_OFFSETS.color().apply_pin(t).get().color();
                let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
                let _ = write!(
                    out,
                    ",\"text\":\"{}\",\"font_size\":{:.1},\"font_weight\":{},\"color\":[{},{},{},{}]",
                    esc(text.as_str()),
                    fs,
                    fw,
                    col.red(),
                    col.green(),
                    col.blue(),
                    col.alpha(),
                );
                return Some(out);
            }
        };
    }
    try_text!(SimpleText);
    try_text!(ComplexText);
    None
}

/// Recursively serialise a Slint item subtree (depth-first) into `out` as JSON
/// objects: depth, item index, geometry [x,y,w,h], accessible role, element
/// type name (empty unless built with SLINT_EMIT_DEBUG_INFO), and — for
/// rectangles — the visual props (radius/border/colours). Bounded so a
/// pathological tree can't run away.
fn walk_slint_elements(
    item: &i_slint_core::item_tree::ItemRc,
    depth: u32,
    out: &mut String,
    first: &mut bool,
    count: &mut usize,
) {
    use std::fmt::Write;
    if *count >= 800 || depth > 16 {
        return;
    }
    *count += 1;
    let g = item.geometry();
    let role = format!("{:?}", item.accessible_role());
    let ty = item
        .element_type_names_and_ids(0)
        .and_then(|v| v.into_iter().next())
        .map(|(t, _id)| t.to_string())
        .unwrap_or_default();
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    if !*first {
        out.push(',');
    }
    *first = false;
    let props = slint_item_props(item).unwrap_or_default();
    let _ = write!(
        out,
        "{{\"depth\":{},\"i\":{},\"geom\":[{:.1},{:.1},{:.1},{:.1}],\"role\":\"{}\",\"type\":\"{}\"{}}}",
        depth,
        item.index(),
        g.origin.x,
        g.origin.y,
        g.size.width,
        g.size.height,
        esc(&role),
        esc(&ty),
        props,
    );
    let mut child = item.first_child();
    while let Some(c) = child {
        walk_slint_elements(&c, depth + 1, out, first, count);
        child = c.next_sibling();
    }
}

/// The bar-specific behaviour: build + show its Slint component, refresh its
/// data, and expose its window for input. The component is created inside
/// `show()` (after the Slint platform is installed).
pub trait BarApp {
    /// Create the Slint component, push initial data, show it. Called once the
    /// EGL/renderer platform is ready, with the surface's logical size and the
    /// containing output's size (for positioning the glass backdrop).
    fn show(
        &mut self,
        width: u32,
        height: u32,
        screen_w: u32,
        screen_h: u32,
    ) -> Result<(), RuntimeError>;
    /// Runtime control for auxiliary content-sized layer surfaces owned by this
    /// client. Existing single-surface clients can ignore it.
    fn set_runtime(&mut self, _runtime: RuntimeControl) {}
    /// Create and show the Slint component for an auxiliary popout surface.
    /// The runtime has already installed a WindowAdapter for the next component
    /// constructed by this call.
    fn show_popout(
        &mut self,
        _handle: PopoutHandle,
        _namespace: &'static str,
        _width: u32,
        _height: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
    /// The Slint window for a popout, used by the runtime for input and render
    /// invalidation. Apps that open popouts must return the matching component's
    /// window after `show_popout`.
    fn popout_window(&self, _handle: PopoutHandle) -> Option<&slint::Window> {
        None
    }
    /// A popout surface was resized after the component was shown.
    fn popout_resized(
        &mut self,
        _handle: PopoutHandle,
        _width: u32,
        _height: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) {
    }
    /// A popout was closed by the app, compositor policy, or protocol teardown.
    /// Apps should drop the matching Slint component and reset their open state.
    fn popout_closed(&mut self, _handle: PopoutHandle, _namespace: &'static str) {}
    /// The layer surface was resized after the component was shown. The Slint
    /// window itself has already received `WindowEvent::Resized`; clients that
    /// cache output-sized geometry can update their own properties here.
    fn resized(&mut self, _width: u32, _height: u32, _screen_w: u32, _screen_h: u32) {}
    /// Periodic refresh (clock, running apps, …). Return true if anything
    /// changed and a redraw is needed.
    fn tick(&mut self) -> bool;
    /// The component's window, for dispatching pointer events.
    fn window(&self) -> Option<&slint::Window>;
    /// When the surface is `full_height`, return true to make the WHOLE surface
    /// grab input (so an open dropdown's outside-clicks dismiss it); false keeps
    /// input to the top `input_height` strip so clicks pass through to windows.
    fn input_full(&self) -> bool {
        false
    }
    /// Override the input region with explicit surface-local rects `(x,y,w,h)`.
    /// When Some on a `full_height` surface, ONLY these rects accept input
    /// (everything else passes through) — used by the notification daemon so
    /// only the cards are clickable. Default None (use `input_full()`).
    fn input_rects(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        None
    }
    /// Return true to tear down the surface and exit `run()` — used by one-shot
    /// on-demand clients (e.g. the window menu) that close after a pick/dismiss.
    fn should_exit(&self) -> bool {
        false
    }
    /// A key was pressed (only when `BarConfig.keyboard`). `text` is the typed
    /// UTF-8 or a `slint::platform::Key` char for special keys. Clients that take
    /// text input (the launcher) handle it here. Default: no-op.
    fn key_pressed(&mut self, _text: &slint::SharedString) {}
}

/// Margins passed to wlr-layer-shell, in logical px.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BarMargins {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

/// Where + how big the layer-shell surface is.
pub struct BarConfig {
    pub namespace: &'static str,
    pub anchor: Anchor,
    pub layer: Layer,
    /// Logical width in px. 0 keeps the historical behaviour: span between
    /// horizontal anchors, or let the compositor choose when both edges are used.
    pub width: u32,
    /// Logical height in px (width spans the anchored edges).
    pub height: u32,
    /// Edge margins in logical px, applied before the first layer configure.
    pub margins: BarMargins,
    /// Reserved work-area edge in px (0 = none).
    pub exclusive_zone: i32,
    /// Make the surface span the whole output (anchored on all edges) while the
    /// visible bar still lives in the top `height` px. Lets in-scene dropdowns
    /// render below the bar over windows. Input is limited to the bar strip
    /// unless `BarApp::input_full()` returns true. Default false (current bars).
    pub full_height: bool,
    /// On a `full_height` surface, set an EMPTY input region so all input passes
    /// through to windows below (e.g. the wallpaper). Overrides `input_full()`.
    pub input_passthrough: bool,
    /// Request keyboard focus (KeyboardInteractivity::OnDemand) and forward key
    /// events into Slint — for clients with text input (e.g. the launcher).
    pub keyboard: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PopoutConfig {
    pub namespace: &'static str,
    pub anchor: Anchor,
    pub layer: Layer,
    pub width: u32,
    pub height: u32,
    pub margins: BarMargins,
    pub keyboard: bool,
}

impl RuntimeControl {
    fn new() -> Self {
        Self {
            commands: Rc::new(RefCell::new(Vec::new())),
            next_popout_id: Rc::new(Cell::new(1)),
        }
    }

    pub fn open_popout(&self, config: PopoutConfig) -> PopoutHandle {
        let id = self.next_popout_id.get();
        self.next_popout_id.set(id.saturating_add(1).max(1));
        let handle = PopoutHandle(id);
        self.commands
            .borrow_mut()
            .push(RuntimeCommand::OpenPopout(handle, config));
        handle
    }

    pub fn close_popout(&self, handle: PopoutHandle) {
        self.commands
            .borrow_mut()
            .push(RuntimeCommand::ClosePopout(handle));
    }

    /// Reconfigure an existing auxiliary layer surface's requested size and
    /// screen margins. The runtime waits for the layer-shell configure before
    /// resizing the Slint buffer.
    pub fn configure_popout(
        &self,
        handle: PopoutHandle,
        width: u32,
        height: u32,
        margins: BarMargins,
    ) {
        self.commands
            .borrow_mut()
            .push(RuntimeCommand::ConfigurePopout(
                handle, width, height, margins,
            ));
    }

    fn drain(&self) -> Vec<RuntimeCommand> {
        self.commands.borrow_mut().drain(..).collect()
    }
}

impl Default for BarConfig {
    fn default() -> Self {
        BarConfig {
            namespace: "gnoblin-bar",
            anchor: Anchor::TOP,
            layer: Layer::Top,
            width: 0,
            height: 34,
            margins: BarMargins::default(),
            exclusive_zone: 0,
            full_height: false,
            input_passthrough: false,
            keyboard: false,
        }
    }
}

// ── EGL context as a Slint OpenGLInterface ──────────────────────────────────

struct EglState {
    egl: egl::Instance<egl::Static>,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
    _wl_egl: wayland_egl::WlEglSurface,
}

// When the compositor goes away, eglSwapBuffers/eglMakeCurrent fail but
// eglGetError() returns SUCCESS (the failure is at the Wayland layer), so
// khronos-egl's `get_error().unwrap()` panics on `None`. Catch that so a dead
// display surfaces as an `Err` — Slint's render() then returns Err and the run
// loop exits cleanly instead of the process aborting.
fn guard_egl<T>(
    what: &str,
    f: impl FnOnce() -> Result<T, egl::Error>,
) -> Result<T, Box<dyn Error + Send + Sync>> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Box::new(e)),
        Err(_) => Err(format!("EGL {what} failed (compositor gone?)").into()),
    }
}

unsafe impl OpenGLInterface for EglState {
    fn ensure_current(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        guard_egl("make_current", || {
            self.egl.make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
        })
    }
    fn swap_buffers(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        guard_egl("swap_buffers", || {
            self.egl.swap_buffers(self.display, self.surface)
        })
    }
    fn resize(
        &self,
        width: NonZeroU32,
        height: NonZeroU32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self._wl_egl
            .resize(width.get() as i32, height.get() as i32, 0, 0);
        Ok(())
    }
    fn get_proc_address(&self, name: &CStr) -> *const c_void {
        match name
            .to_str()
            .ok()
            .and_then(|n| self.egl.get_proc_address(n))
        {
            Some(p) => p as *const c_void,
            None => std::ptr::null(),
        }
    }
}

fn setup_egl(
    conn: &Connection,
    surface: &wl_surface::WlSurface,
    w: u32,
    h: u32,
) -> Result<EglState, RuntimeError> {
    let egl = egl::Instance::new(egl::Static);
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let display =
        unsafe { egl.get_display(display_ptr) }.ok_or_else(|| runtime_error("EGL: no display"))?;
    egl.initialize(display)
        .map_err(|e| runtime_error(format!("EGL: initialize: {e}")))?;
    egl.bind_api(egl::OPENGL_ES_API)
        .map_err(|e| runtime_error(format!("EGL: bind GLES: {e}")))?;

    let config_attribs = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES2_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::NONE,
    ];
    let config = egl
        .choose_first_config(display, &config_attribs)
        .map_err(|e| runtime_error(format!("EGL: choose config: {e}")))?
        .ok_or_else(|| runtime_error("EGL: no matching config"))?;
    let context_attribs = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
    let context = egl
        .create_context(display, config, None, &context_attribs)
        .map_err(|e| runtime_error(format!("EGL: create context: {e}")))?;
    let wl_egl = wayland_egl::WlEglSurface::new(surface.id(), w.max(1) as i32, h.max(1) as i32)
        .map_err(|e| runtime_error(format!("wl_egl_window: {e}")))?;
    let egl_surface = unsafe {
        egl.create_window_surface(display, config, wl_egl.ptr() as *mut c_void, None)
            .map_err(|e| runtime_error(format!("EGL: window surface: {e}")))?
    };
    Ok(EglState {
        egl,
        display,
        surface: egl_surface,
        context,
        _wl_egl: wl_egl,
    })
}

// ── Slint WindowAdapter backed by the FemtoVG/EGL renderer ──────────────────

struct BarAdapter {
    window: slint::Window,
    renderer: FemtoVGRenderer,
    size: Cell<PhysicalSize>,
    needs_redraw: Cell<bool>,
}

impl BarAdapter {
    fn new(egl: EglState, w: u32, h: u32) -> Result<Rc<Self>, slint::PlatformError> {
        // FemtoVGRenderer::new reads GL_VERSION immediately, so the context must
        // already be current — it does not call ensure_current() for us.
        egl.ensure_current()?;
        let renderer = FemtoVGRenderer::new(egl)?;
        Ok(Rc::new_cyclic(|weak: &Weak<BarAdapter>| BarAdapter {
            window: slint::Window::new(weak.clone()),
            renderer,
            size: Cell::new(PhysicalSize::new(w, h)),
            needs_redraw: Cell::new(true),
        }))
    }
}

impl WindowAdapter for BarAdapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
    fn request_redraw(&self) {
        self.needs_redraw.set(true);
    }
}

struct PendingAdapter {
    egl: EglState,
    size: (u32, u32),
    shared: Rc<RefCell<Option<Rc<BarAdapter>>>>,
}

struct BarPlatformState {
    pending: RefCell<VecDeque<PendingAdapter>>,
    start: Instant,
}

impl BarPlatformState {
    fn new() -> Self {
        Self {
            pending: RefCell::new(VecDeque::new()),
            start: Instant::now(),
        }
    }

    fn queue_adapter(
        &self,
        egl: EglState,
        size: (u32, u32),
    ) -> Rc<RefCell<Option<Rc<BarAdapter>>>> {
        let shared = Rc::new(RefCell::new(None));
        self.pending.borrow_mut().push_back(PendingAdapter {
            egl,
            size,
            shared: shared.clone(),
        });
        shared
    }
}

struct BarPlatform {
    state: Rc<BarPlatformState>,
}

impl Platform for BarPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let pending = self
            .state
            .pending
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| slint::PlatformError::from("no pending layer surface adapter"))?;
        let adapter = BarAdapter::new(pending.egl, pending.size.0, pending.size.1)?;
        *pending.shared.borrow_mut() = Some(adapter.clone());
        Ok(adapter)
    }
    fn duration_since_start(&self) -> Duration {
        self.state.start.elapsed()
    }
}

// ── output resolution (multi-monitor) ──────────────────────────────────────
//
// A layer surface binds its output at CREATION time, but sctk's `OutputState`
// only knows the outputs after a roundtrip — and a roundtrip needs a dispatch
// target. To avoid threading an `Option<LayerSurface>` through `State`, we
// resolve the `--output <connector>` name on a throwaway event queue (the same
// connection, so the resulting `wl_output` is valid for `get_layer_surface`)
// before the real surface is created. With no `--output`, the compositor picks.

struct OutputProbe {
    registry_state: RegistryState,
    output_state: OutputState,
}

impl OutputHandler for OutputProbe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for OutputProbe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_output!(OutputProbe);
delegate_registry!(OutputProbe);

/// Find the `wl_output` whose connector name matches `name` (e.g. "DP-1",
/// "Meta-0"). Enumerates outputs on a dedicated queue so it can run before the
/// layer surface exists. Returns `None` if no output matches.
fn resolve_output(
    conn: &Connection,
    globals: &GlobalList,
    name: &str,
) -> Option<wl_output::WlOutput> {
    let mut queue = conn.new_event_queue::<OutputProbe>();
    let qh = queue.handle();
    let mut probe = OutputProbe {
        registry_state: RegistryState::new(globals),
        output_state: OutputState::new(globals, &qh),
    };
    // First roundtrip binds the wl_output globals; the second drains their
    // name/geometry events so `info().name` is populated.
    let _ = queue.roundtrip(&mut probe);
    let _ = queue.roundtrip(&mut probe);
    let found = probe
        .output_state
        .outputs()
        .find(|o| probe.output_state.info(o).and_then(|i| i.name).as_deref() == Some(name));
    if found.is_none() {
        let names: Vec<String> = probe
            .output_state
            .outputs()
            .filter_map(|o| probe.output_state.info(&o).and_then(|i| i.name))
            .collect();
        eprintln!("gnoblin: --output '{name}' not found; available outputs: {names:?}");
    }
    found
}

// ── sctk layer-shell client state ───────────────────────────────────────────

struct PopoutSurface {
    handle: PopoutHandle,
    namespace: &'static str,
    layer: LayerSurface,
    requested_width: u32,
    requested_height: u32,
    margins: BarMargins,
    width: u32,
    height: u32,
    scale: u32,
    configured: bool,
    adapter: Option<Rc<BarAdapter>>,
    frame_pending: bool,
    last_configure_at: Option<Instant>,
}

impl PopoutSurface {
    fn has_active_animations(&self) -> bool {
        self.adapter
            .as_ref()
            .map(|a| a.window.has_active_animations())
            .unwrap_or(false)
    }

    fn configure_settle_remaining(&self) -> Option<Duration> {
        let elapsed = self.last_configure_at?.elapsed();
        if elapsed >= CONFIGURE_RENDER_DELAY {
            None
        } else {
            Some(CONFIGURE_RENDER_DELAY - elapsed)
        }
    }

    fn ready_to_render(&mut self) -> bool {
        if self.frame_pending {
            return false;
        }
        if self.configure_settle_remaining().is_some() {
            return false;
        }
        if self.last_configure_at.is_some() {
            self.last_configure_at = None;
        }
        true
    }

    fn wants_redraw(&self) -> bool {
        let dirty = self
            .adapter
            .as_ref()
            .map(|a| a.needs_redraw.get())
            .unwrap_or(false);
        dirty || self.has_active_animations()
    }
}

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    layer: LayerSurface,
    qh: QueueHandle<State>,
    conn: Connection,
    target_output: Option<wl_output::WlOutput>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    // Logical surface size + the output's integer buffer scale (HiDPI). The EGL
    // buffer + the Slint window report PHYSICAL pixels = logical * scale.
    width: u32,
    height: u32,
    scale: u32,
    configured: bool,
    exit: bool,
    adapter: Option<Rc<BarAdapter>>,
    app: Box<dyn BarApp>,
    // full-height surface support: input limited to the top `input_height` strip
    // unless the app requests the whole surface (open dropdown).
    full_height: bool,
    input_height: u32,
    input_passthrough: bool,
    input_rects_applied: Option<Vec<(i32, i32, i32, i32)>>,
    input_region_dirty: bool,
    // Frame-callback throttling: a buffer commit (eglSwapBuffers) requests a frame
    // callback; we must NOT commit again until it's delivered, or mutter aborts on
    // `frame_callback_list` not being empty. `frame_pending` gates all rendering.
    frame_pending: bool,
    // Layer-shell configure storms can arrive in small bursts while the devkit's
    // virtual monitor materializes/resizes. If we render immediately after the
    // first configure in the burst, mutter can send another configure before it
    // processes our buffer commit and then reject the attach as "before
    // ack_configure". Delay the first post-configure buffer by one tick.
    last_configure_at: Option<Instant>,
    startup_error: Option<String>,
    platform_state: Option<Rc<BarPlatformState>>,
    runtime: RuntimeControl,
    popouts: Vec<PopoutSurface>,
    first_render_done: bool,
}

impl State {
    fn init_slint(&mut self) -> Result<(), RuntimeError> {
        let surface = self.layer.wl_surface().clone();
        // The Slint platform + EGL buffer are created ONCE here. The
        // scale_factor_changed event may not have fired before this first
        // configure, leaving self.scale at 1 — which would build the platform at
        // the wrong (1×) size on a HiDPI output and never fully recover. Seed the
        // scale from the output's actual factor now so the buffer is physical-
        // sized from the start.
        if let Some(s) = self
            .output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .map(|i| i.scale_factor.max(1) as u32)
        {
            self.scale = s;
        }
        let (pw, ph) = (self.width * self.scale, self.height * self.scale);
        // The EGL buffer is sized in PHYSICAL pixels (logical × output scale).
        if self.scale != 1 {
            surface.set_buffer_scale(self.scale as i32);
        }
        rt_tick("init_slint: begin (first configure received)");
        let egl = setup_egl(&self.conn, &surface, pw, ph)?;
        rt_tick("  setup_egl done (EGL ctx + surface)");
        let platform_state = Rc::new(BarPlatformState::new());
        let shared = platform_state.queue_adapter(egl, (pw, ph));
        rt_tick("  FemtoVG renderer ready");
        let platform = BarPlatform {
            state: platform_state.clone(),
        };
        slint::platform::set_platform(Box::new(platform))
            .map_err(|e| runtime_error(format!("set Slint platform: {e}")))?;
        self.platform_state = Some(platform_state);

        let (screen_w, screen_h) = self.screen_size();
        self.app.show(self.width, self.height, screen_w, screen_h)?;
        rt_tick("  app.show() (Slint component tree built)");
        let window = self
            .app
            .window()
            .ok_or_else(|| runtime_error("Slint app did not create a window"))?;
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: self.scale as f32,
        });
        window.dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(self.width as f32, self.height as f32),
        });
        self.adapter = shared.borrow().clone();
        self.inspect_log_window();
        Ok(())
    }

    /// When `GNOBLIN_INSPECT` is set, write this client's self-view —
    /// logical/physical size, output scale, theme, full-height/input strip — to
    /// `$XDG_RUNTIME_DIR/gnoblin-inspect/window-<pid>.json` (overwritten with the
    /// latest state). The inspector correlates it to the compositor surface by
    /// pid, so e.g. a client's `theme_dark` can be checked against the ring the
    /// compositor drew. No-op unless the env var is set.
    fn inspect_log_window(&self) {
        if std::env::var_os("GNOBLIN_INSPECT").is_none() {
            return;
        }
        let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(d) => std::path::PathBuf::from(d).join("gnoblin-inspect"),
            None => return,
        };
        let _ = std::fs::create_dir_all(&dir);
        // Raw output info the client sees (to debug HiDPI logical-size issues).
        let (ol, osf, om) = self
            .output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .map(|i| {
                let mode = i
                    .modes
                    .iter()
                    .find(|m| m.current)
                    .map(|m| m.dimensions)
                    .unwrap_or((0, 0));
                (i.logical_size, i.scale_factor, mode)
            })
            .unwrap_or((None, 1, (0, 0)));
        let ol = ol
            .map(|(w, h)| format!("[{w},{h}]"))
            .unwrap_or_else(|| "null".into());
        // The ACTUAL committed buffer size (the EGL/Slint adapter physical size) —
        // to tell whether a HiDPI render bug is the client buffer being wrong vs
        // mutter mis-scaling a correct buffer.
        let buf = self
            .adapter
            .as_ref()
            .map(|a| {
                let s = a.size.get();
                format!("[{},{}]", s.width, s.height)
            })
            .unwrap_or_else(|| "null".into());
        // What Slint ITSELF thinks: window physical size + its scale_factor. If
        // scale_factor is 4 (not 2), the renderer double-scales the content.
        let (slint_win, slint_sc) = self
            .app
            .window()
            .map(|w| {
                let s = w.size();
                let sc = i_slint_core::window::WindowInner::from_pub(w).scale_factor();
                (format!("[{},{}]", s.width, s.height), sc)
            })
            .unwrap_or_else(|| ("null".into(), 0.0));
        let json = format!(
            "{{\"pid\":{},\"theme_dark\":{},\"scale\":{},\"logical\":[{},{}],\
             \"physical\":[{},{}],\"egl_buffer\":{},\"slint_win\":{},\"slint_scale\":{:.2},\
             \"full_height\":{},\"input_height\":{},\
             \"out_logical\":{},\"out_scale\":{},\"out_mode\":[{},{}]}}\n",
            std::process::id(),
            crate::theme::is_dark(),
            self.scale,
            self.width,
            self.height,
            self.width * self.scale,
            self.height * self.scale,
            buf,
            slint_win,
            slint_sc,
            self.full_height,
            self.input_height,
            ol,
            osf,
            om.0,
            om.1,
        );
        let path = dir.join(format!("window-{}.json", std::process::id()));
        let _ = std::fs::write(path, json);
        self.inspect_log_elements(&dir);
    }

    /// Walk the live Slint item tree (per-element geometry/role/type) and write it
    /// to `elements-<pid>.json`. Uses the UNSTABLE `i_slint_core` item-tree API
    /// (the only way to read a compiled component's element tree). Element type
    /// names require `SLINT_EMIT_DEBUG_INFO=1` at build time; geometry + role work
    /// regardless. No-op unless `GNOBLIN_INSPECT` is set (checked by the caller).
    fn inspect_log_elements(&self, dir: &std::path::Path) {
        use i_slint_core::item_tree::ItemRc;

        let window = match self.app.window() {
            Some(w) => w,
            None => return,
        };
        // try_component (not component()) so a window whose root isn't set yet
        // can't panic the client from the inspector path.
        let comp = match i_slint_core::window::WindowInner::from_pub(window).try_component() {
            Some(c) => c,
            None => return,
        };
        let root = ItemRc::new(comp, 0);
        let mut out = String::from("[");
        let mut first = true;
        let mut count = 0usize;
        walk_slint_elements(&root, 0, &mut out, &mut first, &mut count);
        out.push(']');
        let path = dir.join(format!("elements-{}.json", std::process::id()));
        let _ = std::fs::write(path, out);
    }

    fn inspect_log_popout(&self, idx: usize) {
        if std::env::var_os("GNOBLIN_INSPECT").is_none() {
            return;
        }
        let Some(popout) = self.popouts.get(idx) else {
            return;
        };
        let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(d) => std::path::PathBuf::from(d).join("gnoblin-inspect"),
            None => return,
        };
        let _ = std::fs::create_dir_all(&dir);
        let (ol, osf, om) = self
            .output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .map(|i| {
                let mode = i
                    .modes
                    .iter()
                    .find(|m| m.current)
                    .map(|m| m.dimensions)
                    .unwrap_or((0, 0));
                (i.logical_size, i.scale_factor, mode)
            })
            .unwrap_or((None, 1, (0, 0)));
        let ol = ol
            .map(|(w, h)| format!("[{w},{h}]"))
            .unwrap_or_else(|| "null".into());
        let buf = popout
            .adapter
            .as_ref()
            .map(|a| {
                let s = a.size.get();
                format!("[{},{}]", s.width, s.height)
            })
            .unwrap_or_else(|| "null".into());
        let (slint_win, slint_sc) = self
            .app
            .popout_window(popout.handle)
            .map(|w| {
                let s = w.size();
                let sc = i_slint_core::window::WindowInner::from_pub(w).scale_factor();
                (format!("[{},{}]", s.width, s.height), sc)
            })
            .unwrap_or_else(|| ("null".into(), 0.0));
        let namespace = popout.namespace.replace('"', "\\\"");
        let json = format!(
            "{{\"pid\":{},\"namespace\":\"{}\",\"theme_dark\":{},\"scale\":{},\"logical\":[{},{}],\
             \"physical\":[{},{}],\"egl_buffer\":{},\"slint_win\":{},\"slint_scale\":{:.2},\
             \"full_height\":false,\"input_height\":{},\
             \"out_logical\":{},\"out_scale\":{},\"out_mode\":[{},{}]}}\n",
            std::process::id(),
            namespace,
            crate::theme::is_dark(),
            popout.scale,
            popout.width,
            popout.height,
            popout.width * popout.scale,
            popout.height * popout.scale,
            buf,
            slint_win,
            slint_sc,
            popout.height,
            ol,
            osf,
            om.0,
            om.1,
        );
        let filename_ns = popout
            .namespace
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = dir.join(format!(
            "window-{}-{}-{}.json",
            std::process::id(),
            filename_ns,
            popout.handle.0
        ));
        let _ = std::fs::write(path, json);
    }

    /// Re-apply the current logical size + scale to the live surface: the EGL
    /// buffer becomes PHYSICAL (logical × scale), the Slint window keeps LOGICAL
    /// coords. Called on a configure resize or an output scale change.
    fn apply_size(&mut self) {
        if self.adapter.is_none() {
            return;
        }
        let (pw, ph) = (self.width * self.scale, self.height * self.scale);
        if let Some(adapter) = &self.adapter {
            adapter.size.set(PhysicalSize::new(pw.max(1), ph.max(1)));
        }
        if let Some(window) = self.app.window() {
            window.dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: self.scale as f32,
            });
            window.dispatch_event(WindowEvent::Resized {
                size: LogicalSize::new(self.width as f32, self.height as f32),
            });
        } else {
            eprintln!("gnoblin-runtime: Slint app window disappeared during resize; exiting.");
            self.exit = true;
            return;
        }
        let (screen_w, screen_h) = self.screen_size();
        self.app
            .resized(self.width, self.height, screen_w, screen_h);
        self.inspect_log_window();
        // The input region is sized in surface-logical px. Apply it from the
        // post-dispatch commit point so it cannot race unread configures.
        self.input_region_dirty = true;
        // Draw the new size, but obey the single-outstanding-frame-callback rule:
        // if a frame is already in flight, the `frame` callback will redraw. Forcing
        // a render here (the old `frame_pending = false; render()`) committed a 2nd
        // frame callback while one was pending → mutter aborts on
        // `frame_callback_list` not being empty (seen as a devkit crash on the
        // rapid placeholder→real-size configures at startup).
        if let Some(a) = &self.adapter {
            a.needs_redraw.set(true);
        }
        // The post-dispatch render (run()'s loop) draws this — committing here
        // could race an unread resize configure.
    }

    /// The containing output's logical size. For full-height layer surfaces, the
    /// compositor-configured surface size is the authoritative output-sized value;
    /// output metadata can lag behind both startup and resize configures.
    fn screen_size(&self) -> (u32, u32) {
        if self.full_height && self.configured {
            return (self.width.max(1), self.height.max(1));
        }
        self.output_screen_size()
    }

    fn output_screen_size(&self) -> (u32, u32) {
        self.output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .and_then(|i| {
                // Prefer the LOGICAL output size. Fall back to the current mode
                // divided by the output scale — the mode is in PHYSICAL pixels, so
                // using it raw on a scaled (HiDPI) output sizes surfaces for the
                // unscaled resolution and renders only their top-left quarter.
                i.logical_size.or_else(|| {
                    let s = i.scale_factor.max(1);
                    i.modes
                        .iter()
                        .find(|m| m.current)
                        .map(|m| (m.dimensions.0 / s, m.dimensions.1 / s))
                })
            })
            .map(|(w, h)| (w.max(1) as u32, h.max(1) as u32))
            .unwrap_or((self.width.max(1), self.height.max(1)))
    }

    fn popout_index_for_surface(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.popouts
            .iter()
            .position(|p| p.layer.wl_surface() == surface)
    }

    fn popout_index_for_layer(&self, layer: &LayerSurface) -> Option<usize> {
        self.popouts.iter().position(|p| &p.layer == layer)
    }

    fn open_popout_surface(&mut self, handle: PopoutHandle, config: PopoutConfig) {
        if self.popouts.iter().any(|p| p.handle == handle) {
            return;
        }
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            config.layer,
            Some(config.namespace),
            self.target_output.as_ref(),
        );
        layer.set_anchor(config.anchor);
        layer.set_size(config.width.max(1), config.height.max(1));
        layer.set_margin(
            config.margins.top,
            config.margins.right,
            config.margins.bottom,
            config.margins.left,
        );
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(if config.keyboard {
            KeyboardInteractivity::OnDemand
        } else {
            KeyboardInteractivity::None
        });
        self.popouts.push(PopoutSurface {
            handle,
            namespace: config.namespace,
            layer,
            requested_width: config.width.max(1),
            requested_height: config.height.max(1),
            margins: config.margins,
            width: config.width.max(1),
            height: config.height.max(1),
            scale: self.scale.max(1),
            configured: false,
            adapter: None,
            frame_pending: false,
            last_configure_at: None,
        });
        if let Some(popout) = self.popouts.last() {
            popout.layer.commit();
        }
    }

    fn configure_popout_surface(
        &mut self,
        idx: usize,
        width: u32,
        height: u32,
        margins: BarMargins,
    ) {
        let Some(popout) = self.popouts.get_mut(idx) else {
            return;
        };
        let width = width.max(1);
        let height = height.max(1);
        if popout.requested_width == width
            && popout.requested_height == height
            && popout.margins == margins
        {
            return;
        }

        let size_changed = popout.requested_width != width || popout.requested_height != height;
        popout.requested_width = width;
        popout.requested_height = height;
        popout.margins = margins;
        popout.layer.set_size(width, height);
        popout
            .layer
            .set_margin(margins.top, margins.right, margins.bottom, margins.left);
        popout.layer.commit();

        if !size_changed {
            self.inspect_log_popout(idx);
        }
    }

    fn close_popout_surface(&mut self, idx: usize) {
        if idx >= self.popouts.len() {
            return;
        }
        let popout = self.popouts.remove(idx);
        self.app.popout_closed(popout.handle, popout.namespace);
        match popout.layer.kind().clone() {
            smithay_client_toolkit::shell::wlr_layer::SurfaceKind::Wlr(wlr) => wlr.destroy(),
            _ => {}
        }
        popout.layer.wl_surface().destroy();
    }

    fn process_runtime_commands(&mut self) -> Result<(), RuntimeError> {
        for command in self.runtime.drain() {
            match command {
                RuntimeCommand::OpenPopout(handle, config) => {
                    self.open_popout_surface(handle, config);
                }
                RuntimeCommand::ClosePopout(handle) => {
                    if let Some(idx) = self.popouts.iter().position(|p| p.handle == handle) {
                        self.close_popout_surface(idx);
                    }
                }
                RuntimeCommand::ConfigurePopout(handle, width, height, margins) => {
                    if let Some(idx) = self.popouts.iter().position(|p| p.handle == handle) {
                        self.configure_popout_surface(idx, width, height, margins);
                    }
                }
            }
        }
        Ok(())
    }

    fn init_popout_slint(&mut self, idx: usize) -> Result<(), RuntimeError> {
        let platform_state = self
            .platform_state
            .as_ref()
            .ok_or_else(|| runtime_error("Slint platform not ready for popout"))?
            .clone();
        let (handle, namespace, width, height, scale, surface) = {
            let popout = self
                .popouts
                .get(idx)
                .ok_or_else(|| runtime_error("popout disappeared before init"))?;
            (
                popout.handle,
                popout.namespace,
                popout.width,
                popout.height,
                popout.scale,
                popout.layer.wl_surface().clone(),
            )
        };
        let (pw, ph) = (width * scale, height * scale);
        if scale != 1 {
            surface.set_buffer_scale(scale as i32);
        }
        let egl = setup_egl(&self.conn, &surface, pw, ph)?;
        let shared = platform_state.queue_adapter(egl, (pw, ph));
        let (screen_w, screen_h) = self.output_screen_size();
        self.app
            .show_popout(handle, namespace, width, height, screen_w, screen_h)?;
        let window = self
            .app
            .popout_window(handle)
            .ok_or_else(|| runtime_error("Slint popout did not create a window"))?;
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale as f32,
        });
        window.dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(width as f32, height as f32),
        });
        if let Some(popout) = self.popouts.get_mut(idx) {
            popout.adapter = shared.borrow().clone();
        }
        self.inspect_log_popout(idx);
        Ok(())
    }

    fn apply_popout_size(&mut self, idx: usize) {
        let Some(popout) = self.popouts.get(idx) else {
            return;
        };
        let (handle, width, height, scale) =
            (popout.handle, popout.width, popout.height, popout.scale);
        let (pw, ph) = (width * scale, height * scale);
        if let Some(adapter) = &popout.adapter {
            adapter.size.set(PhysicalSize::new(pw.max(1), ph.max(1)));
        }
        if let Some(window) = self.app.popout_window(handle) {
            window.dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale as f32,
            });
            window.dispatch_event(WindowEvent::Resized {
                size: LogicalSize::new(width as f32, height as f32),
            });
        } else {
            eprintln!("gnoblin-runtime: Slint popout window disappeared during resize; closing.");
            self.close_popout_surface(idx);
            return;
        }
        let (screen_w, screen_h) = self.output_screen_size();
        self.app
            .popout_resized(handle, width, height, screen_w, screen_h);
        if let Some(popout) = self.popouts.get(idx) {
            if let Some(a) = &popout.adapter {
                a.needs_redraw.set(true);
            }
        }
        self.inspect_log_popout(idx);
    }

    fn has_active_animations(&self) -> bool {
        self.adapter
            .as_ref()
            .map(|a| a.window.has_active_animations())
            .unwrap_or(false)
    }

    fn has_any_ready_animation(&self) -> bool {
        (self.has_active_animations() && !self.frame_pending)
            || self
                .popouts
                .iter()
                .any(|p| p.has_active_animations() && !p.frame_pending)
    }

    fn configure_settle_remaining(&self) -> Option<Duration> {
        let elapsed = self.last_configure_at?.elapsed();
        if elapsed >= CONFIGURE_RENDER_DELAY {
            None
        } else {
            Some(CONFIGURE_RENDER_DELAY - elapsed)
        }
    }

    fn any_configure_settle_remaining(&self) -> Option<Duration> {
        self.configure_settle_remaining()
            .into_iter()
            .chain(
                self.popouts
                    .iter()
                    .filter_map(PopoutSurface::configure_settle_remaining),
            )
            .min()
    }

    fn next_dispatch_timeout(&self) -> Duration {
        if let Some(remaining) = self.any_configure_settle_remaining() {
            return remaining.min(IDLE_DISPATCH_TIMEOUT);
        }
        if self.has_any_ready_animation() {
            return Duration::ZERO;
        }
        slint::platform::duration_until_next_timer_update()
            .unwrap_or(IDLE_DISPATCH_TIMEOUT)
            .min(IDLE_DISPATCH_TIMEOUT)
    }

    /// Let Slint advance timers/animations before deciding whether a frame is
    /// needed. `has_active_animations()` describes the previous render pass; if
    /// it was true, the advanced animation time must produce at least one more
    /// draw so the animation can converge even when no pointer input arrives.
    fn pump_slint(&mut self) {
        if self.adapter.is_none() {
            return;
        }
        let had_active_animations = self.has_active_animations();
        slint::platform::update_timers_and_animations();
        if had_active_animations {
            if let Some(a) = &self.adapter {
                a.needs_redraw.set(true);
            }
        }
        for popout in &self.popouts {
            if popout.has_active_animations() {
                if let Some(a) = &popout.adapter {
                    a.needs_redraw.set(true);
                }
            }
        }
    }

    /// True if there's a frame worth drawing — the app/Slint flagged a redraw,
    /// or Slint reports an active animation from the last scene evaluation.
    fn wants_redraw(&self) -> bool {
        let dirty = self
            .adapter
            .as_ref()
            .map(|a| a.needs_redraw.get())
            .unwrap_or(false);
        dirty || self.has_active_animations()
    }

    fn ready_to_render(&mut self) -> bool {
        if self.frame_pending {
            return false;
        }
        if self.configure_settle_remaining().is_some() {
            return false;
        }
        if self.last_configure_at.is_some() {
            self.last_configure_at = None;
        }
        true
    }

    /// Draw + commit ONE frame.
    fn render(&mut self) {
        let surface = self.layer.wl_surface().clone();
        surface.frame(&self.qh, surface.clone());
        let result = self.adapter.as_ref().map(|adapter| {
            adapter.needs_redraw.set(false);
            adapter.renderer.render()
        });
        match result {
            Some(Ok(())) => {
                if !self.first_render_done {
                    self.first_render_done = true;
                    rt_tick("FIRST render() done (surface has pixels)");
                }
                self.input_region_committed_with_render();
                self.frame_pending = true;
            }
            // A render failure here means the EGL surface/display is gone (the
            // compositor exited) — stop cleanly rather than spin or abort.
            Some(Err(e)) => {
                eprintln!("gnoblin-runtime: render failed ({e}); compositor gone — exiting.");
                self.exit = true;
            }
            None => {}
        }
    }

    fn render_popout(&mut self, idx: usize) {
        let Some(popout) = self.popouts.get(idx) else {
            return;
        };
        let surface = popout.layer.wl_surface().clone();
        surface.frame(&self.qh, surface.clone());
        let result = popout.adapter.as_ref().map(|adapter| {
            adapter.needs_redraw.set(false);
            adapter.renderer.render()
        });
        match result {
            Some(Ok(())) => {
                if let Some(popout) = self.popouts.get_mut(idx) {
                    popout.frame_pending = true;
                }
            }
            Some(Err(e)) => {
                eprintln!("gnoblin-runtime: popout render failed ({e}); closing surface.");
                self.close_popout_surface(idx);
            }
            None => {}
        }
    }

    fn tick(&mut self) {
        if self.app.tick() {
            if let Some(a) = &self.adapter {
                a.needs_redraw.set(true);
            }
            for popout in &self.popouts {
                if let Some(a) = &popout.adapter {
                    a.needs_redraw.set(true);
                }
            }
        }
        self.input_region_dirty = true;
        // No render here — the main loop renders post-dispatch, once configures
        // are acked (see run()'s loop). Rendering from this timer could commit a
        // buffer while a configure sits unread on the socket.
    }

    /// On a full-height surface, scope the input region to the rects that should
    /// accept pointer input; everything else passes through to windows below.
    /// Priority: passthrough (nothing) → app `input_rects()` (e.g. notification
    /// cards) → whole surface while `input_full()` (open menu/launcher) → the
    /// bar strip.
    fn desired_input_rects(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        if self.input_passthrough {
            Some(Vec::new())
        } else if let Some(r) = self.app.input_rects() {
            // A custom region is honoured even on a non-full-height bar (e.g.
            // the dock, whose surface includes click-through headroom above
            // the band for its right-click menu).
            Some(r)
        } else if !self.full_height {
            // Non-full-height bar with no custom region: leave the compositor
            // default (the whole fixed-height surface stays interactive).
            None
        } else if self.app.input_full() {
            Some(vec![(0, 0, self.width as i32, self.height as i32)])
        } else {
            Some(vec![(0, 0, self.width as i32, self.input_height as i32)])
        }
    }

    /// Apply pending input-region changes from the main post-dispatch path. This
    /// keeps all layer-surface commits behind the configure-drain barrier.
    fn apply_input_region(&mut self, commit: bool) {
        if !self.input_region_dirty {
            return;
        }
        let rects = self.desired_input_rects();
        if self.input_rects_applied == rects {
            self.input_region_dirty = false;
            return;
        }
        let surface = self.layer.wl_surface();
        if let Some(rects) = rects {
            if let Ok(region) = Region::new(&self.compositor) {
                for (x, y, w, h) in &rects {
                    region.add(*x, *y, *w, *h);
                }
                surface.set_input_region(Some(region.wl_region()));
                if commit {
                    surface.commit();
                    self.input_rects_applied = Some(rects);
                    self.input_region_dirty = false;
                } else {
                    // This pending region is committed by the render path below.
                    // Keep it dirty until that render succeeds so skipped frames
                    // cannot make our bookkeeping lie about hit-test state.
                    self.input_rects_applied = None;
                }
            }
        } else {
            surface.set_input_region(None);
            if commit {
                surface.commit();
                self.input_rects_applied = None;
                self.input_region_dirty = false;
            } else {
                self.input_rects_applied = Some(Vec::new());
            }
        }
    }

    fn input_region_committed_with_render(&mut self) {
        if self.input_region_dirty {
            self.input_rects_applied = self.desired_input_rects();
            self.input_region_dirty = false;
        }
    }
}

fn map_button(code: u32) -> PointerEventButton {
    match code {
        273 => PointerEventButton::Right,
        274 => PointerEventButton::Middle,
        _ => PointerEventButton::Left,
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_scale: i32,
    ) {
        if let Some(idx) = self.popout_index_for_surface(surface) {
            let scale = new_scale.max(1) as u32;
            let configured = {
                let popout = &mut self.popouts[idx];
                if scale == popout.scale {
                    return;
                }
                popout.scale = scale;
                popout.configured
            };
            surface.set_buffer_scale(new_scale.max(1));
            if configured {
                self.apply_popout_size(idx);
            }
            return;
        }
        // HiDPI: render the buffer at `new_scale`× so content is crisp + correctly
        // sized on a scaled output, instead of a 1× buffer the compositor upscales.
        let scale = new_scale.max(1) as u32;
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        surface.set_buffer_scale(new_scale.max(1));
        if self.configured {
            self.apply_size();
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if let Some(idx) = self.popout_index_for_surface(surface) {
            if let Some(popout) = self.popouts.get_mut(idx) {
                popout.frame_pending = false;
            }
            return;
        }
        // The throttling callback for our last commit arrived — free to draw the
        // next frame (continuing an animation only if there's still work).
        self.frame_pending = false;
        // The next frame (if an animation is still running) is drawn by the
        // post-dispatch render in run()'s loop.
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(ptr) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(ptr);
            }
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(kbd) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(kbd);
            }
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

/// Map an sctk key event to the text Slint expects — special keys become their
/// `slint::platform::Key` char; everything else uses the event's UTF-8.
fn key_to_text(event: &KeyEvent) -> Option<slint::SharedString> {
    use slint::platform::Key;
    let special: Option<Key> = match event.keysym {
        Keysym::Escape => Some(Key::Escape),
        Keysym::Return | Keysym::KP_Enter => Some(Key::Return),
        Keysym::BackSpace => Some(Key::Backspace),
        Keysym::Delete => Some(Key::Delete),
        Keysym::Tab => Some(Key::Tab),
        Keysym::Left => Some(Key::LeftArrow),
        Keysym::Right => Some(Key::RightArrow),
        Keysym::Up => Some(Key::UpArrow),
        Keysym::Down => Some(Key::DownArrow),
        Keysym::Home => Some(Key::Home),
        Keysym::End => Some(Key::End),
        _ => None,
    };
    if let Some(k) = special {
        return Some(char::from(k).into());
    }
    event.utf8.clone().filter(|s| !s.is_empty()).map(Into::into)
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if let Some(text) = key_to_text(&event) {
            self.app.key_pressed(&text);
            if let Some(a) = &self.adapter {
                a.needs_redraw.set(true);
            }
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: u32,
    ) {
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        use PointerEventKind::*;
        let our_surface = self.layer.wl_surface().clone();
        for e in events {
            let popout_idx = if e.surface == our_surface {
                None
            } else {
                self.popout_index_for_surface(&e.surface)
            };
            if e.surface != our_surface && popout_idx.is_none() {
                continue;
            }
            let pos = LogicalPosition::new(e.position.0 as f32, e.position.1 as f32);
            let ev = match e.kind {
                Enter { .. } | Motion { .. } => Some(WindowEvent::PointerMoved { position: pos }),
                Leave { .. } => Some(WindowEvent::PointerExited),
                Press { button, .. } => Some(WindowEvent::PointerPressed {
                    position: pos,
                    button: map_button(button),
                }),
                Release { button, .. } => Some(WindowEvent::PointerReleased {
                    position: pos,
                    button: map_button(button),
                }),
                Axis {
                    horizontal,
                    vertical,
                    ..
                } => Some(WindowEvent::PointerScrolled {
                    position: pos,
                    delta_x: -horizontal.absolute as f32,
                    delta_y: -vertical.absolute as f32,
                }),
            };
            if let Some(ev) = ev {
                if let Some(idx) = popout_idx {
                    let handle = self.popouts[idx].handle;
                    if let Some(window) = self.app.popout_window(handle) {
                        window.dispatch_event(ev);
                        if let Some(a) = &self.popouts[idx].adapter {
                            a.needs_redraw.set(true);
                        }
                    } else {
                        eprintln!(
                            "gnoblin-runtime: Slint popout window disappeared during input; closing."
                        );
                        self.close_popout_surface(idx);
                    }
                } else if let Some(window) = self.app.window() {
                    window.dispatch_event(ev);
                    // Slint callbacks can open/close menus/popouts, which changes
                    // BarApp::input_full()/input_rects(). Recompute hit testing in
                    // the same loop instead of waiting for the 100ms app tick.
                    self.input_region_dirty = true;
                    if let Some(a) = &self.adapter {
                        a.needs_redraw.set(true);
                    }
                } else {
                    eprintln!(
                        "gnoblin-runtime: Slint app window disappeared during input; exiting."
                    );
                    self.exit = true;
                }
            }
        }
        // Drawn by the post-dispatch render in run()'s loop.
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(idx) = self.popout_index_for_layer(layer) {
            self.close_popout_surface(idx);
            return;
        }
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(idx) = self.popout_index_for_layer(layer) {
            let mut init = false;
            let mut resize = false;
            {
                let Some(popout) = self.popouts.get_mut(idx) else {
                    return;
                };
                popout.last_configure_at = Some(Instant::now());
                let (mut w, mut h) = configure.new_size;
                if w == 0 {
                    w = popout.requested_width.max(1);
                }
                if h == 0 {
                    h = popout.requested_height.max(1);
                }
                if !popout.configured {
                    popout.width = w;
                    popout.height = h;
                    popout.configured = true;
                    init = true;
                } else if w != popout.width || h != popout.height {
                    popout.width = w;
                    popout.height = h;
                    resize = true;
                }
            }
            if init {
                if let Err(e) = self.init_popout_slint(idx) {
                    eprintln!("gnoblin-runtime: popout startup failed ({e}); closing.");
                    self.close_popout_surface(idx);
                }
            } else if resize {
                self.apply_popout_size(idx);
            }
            return;
        }
        self.last_configure_at = Some(Instant::now());
        // The compositor's chosen size. For an edge a dimension it leaves to us
        // comes back 0 — fall back to the output's size so we always span it.
        let (mut w, mut h) = configure.new_size;
        if w == 0 || h == 0 {
            let (ow, oh) = self.screen_size();
            if w == 0 {
                w = ow;
            }
            if h == 0 {
                h = oh;
            }
        }
        if !self.configured {
            self.width = w;
            self.height = h;
            self.configured = true;
            match self.init_slint() {
                Ok(()) => {
                    self.input_region_dirty = true;
                    // init_slint() leaves needs_redraw set; the post-dispatch render in
                    // run()'s loop draws the first frame once this configure is acked.
                }
                Err(e) => {
                    self.startup_error = Some(e.to_string());
                    self.exit = true;
                }
            }
        } else if w != self.width || h != self.height {
            // The output changed size (or sent the real size after a placeholder)
            // — resize the live surface to match instead of staying fixed.
            self.width = w;
            self.height = h;
            self.apply_size();
        }
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_keyboard!(State);
delegate_compositor!(State);
delegate_output!(State);
delegate_layer!(State);
delegate_seat!(State);
delegate_pointer!(State);
delegate_registry!(State);

fn poll_timeout_ms(timeout: Duration) -> libc::c_int {
    if timeout.is_zero() {
        return 0;
    }
    let millis = timeout.as_millis().max(1);
    millis.min(libc::c_int::MAX as u128) as libc::c_int
}

fn flush_wayland_queue(event_queue: &mut EventQueue<State>) -> Result<(), WaylandError> {
    match event_queue.flush() {
        Err(WaylandError::Io(err)) if err.kind() == ErrorKind::WouldBlock => Ok(()),
        result => result,
    }
}

fn dispatch_wayland(
    event_queue: &mut EventQueue<State>,
    state: &mut State,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    if event_queue.dispatch_pending(state)? > 0 {
        flush_wayland_queue(event_queue)?;
        return Ok(());
    }

    flush_wayland_queue(event_queue)?;

    let Some(guard) = event_queue.prepare_read() else {
        event_queue.dispatch_pending(state)?;
        return Ok(());
    };

    let mut pollfd = libc::pollfd {
        fd: guard.connection_fd().as_raw_fd(),
        events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
        revents: 0,
    };
    let timeout_ms = poll_timeout_ms(timeout);
    let ready = loop {
        let ready = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if ready >= 0 {
            break ready;
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != ErrorKind::Interrupted {
            drop(guard);
            return Err(Box::new(err));
        }
    };

    if ready > 0 && pollfd.revents != 0 {
        match guard.read() {
            Err(WaylandError::Io(err)) if err.kind() == ErrorKind::WouldBlock => {}
            result => {
                result?;
            }
        }
        event_queue.dispatch_pending(state)?;
    } else {
        drop(guard);
    }

    Ok(())
}

fn run_due_ticks(state: &mut State, next_tick: &mut Instant) {
    let now = Instant::now();
    let mut ticks = 0;
    while now >= *next_tick && ticks < 5 {
        state.tick();
        *next_tick += APP_TICK_INTERVAL;
        ticks += 1;
    }
    if now >= *next_tick {
        *next_tick = now + APP_TICK_INTERVAL;
    }
}

/// Run a Slint `BarApp` as a wlr-layer-shell client until the compositor exits.
/// Env-gated (`GNOBLIN_TIMING`) startup profiler. The first call seeds t0, so
/// call it at `try_run` entry. Cheap no-op otherwise.
pub fn rt_tick(label: &str) {
    thread_local! { static T0: Instant = Instant::now(); }
    if std::env::var_os("GNOBLIN_TIMING").is_some() {
        T0.with(|t0| {
            eprintln!(
                "[timing/rt] {:>6.1}ms  {}",
                t0.elapsed().as_secs_f64() * 1000.0,
                label
            )
        });
    }
}

pub fn run(config: BarConfig, app: Box<dyn BarApp>) {
    if let Err(e) = try_run(config, app) {
        eprintln!("gnoblin-runtime: {e}");
    }
}

fn try_run(config: BarConfig, mut app: Box<dyn BarApp>) -> Result<(), RuntimeError> {
    // khronos-egl panics (caught in guard_egl) when the compositor disappears —
    // silence their verbose backtrace; keep the default hook for real bugs.
    {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let from_egl = info
                .location()
                .map(|l| l.file().contains("khronos-egl"))
                .unwrap_or(false);
            if !from_egl {
                default(info);
            }
        }));
    }

    rt_tick("try_run: begin");
    let conn = Connection::connect_to_env()
        .map_err(|e| runtime_error(format!("connect to Wayland: {e}")))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| runtime_error(format!("registry init: {e}")))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| runtime_error(format!("bind wl_compositor: {e}")))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| runtime_error(format!("bind wlr-layer-shell: {e}")))?;
    rt_tick("wayland connect + registry + binds");

    // Multi-monitor: bind to the output named by `--output` if the compositor
    // launched us for a specific monitor; otherwise let it choose.
    // Explicit --output (per-output panels, window-menu role) wins; otherwise an
    // on-demand client falls back to GNOBLIN_ACTIVE_OUTPUT (the monitor the
    // compositor was focused on when it spawned us). Neither set → compositor picks.
    let target_output = ClientArgs::from_env()
        .output
        .or_else(|| {
            std::env::var("GNOBLIN_ACTIVE_OUTPUT")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .and_then(|name| resolve_output(&conn, &globals, &name));

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        config.layer,
        Some(config.namespace),
        target_output.as_ref(),
    );
    if config.full_height && config.exclusive_zone == 0 {
        // Span the whole output (all edges) so dropdowns can render below the
        // bar; nothing is reserved, so the ambiguous all-edges anchor is fine.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
    } else if config.full_height {
        // A bar that BOTH spans the output (for drop-downs) AND reserves an edge
        // (exclusive_zone): keep the real anchor (so the compositor can resolve
        // which edge the exclusive zone reserves — all-four anchors are
        // ambiguous and sctk has no v5 set_exclusive_edge), and request a huge
        // height that the compositor clamps to the output. We learn the real
        // size back from the configure event.
        layer.set_anchor(config.anchor);
        layer.set_size(0, 1 << 16);
    } else {
        layer.set_anchor(config.anchor);
        layer.set_size(config.width, config.height);
    }
    layer.set_margin(
        config.margins.top,
        config.margins.right,
        config.margins.bottom,
        config.margins.left,
    );
    layer.set_exclusive_zone(config.exclusive_zone);
    layer.set_keyboard_interactivity(if config.keyboard {
        KeyboardInteractivity::Exclusive
    } else {
        KeyboardInteractivity::None
    });

    let runtime = RuntimeControl::new();
    app.set_runtime(runtime.clone());

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        compositor,
        layer_shell,
        layer,
        qh: qh.clone(),
        conn: conn.clone(),
        target_output,
        pointer: None,
        keyboard: None,
        width: if config.width > 0 { config.width } else { 1280 },
        height: config.height.max(1),
        scale: 1,
        configured: false,
        exit: false,
        adapter: None,
        app,
        full_height: config.full_height,
        input_height: config.height.max(1),
        input_passthrough: config.input_passthrough,
        input_rects_applied: None,
        input_region_dirty: true,
        frame_pending: false,
        last_configure_at: None,
        startup_error: None,
        platform_state: None,
        runtime,
        popouts: Vec::new(),
        first_render_done: false,
    };

    // Bind pointer/keyboard resources before the initial layer-surface commit.
    // Exclusive layer surfaces can receive focus as soon as the compositor sees
    // that commit; if wl_keyboard is still unbound, early typed keys in the
    // launcher/window-menu startup path can be dropped before an enter arrives.
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| runtime_error(format!("initial input registry roundtrip: {e}")))?;
    state.layer.commit();
    rt_tick("initial roundtrip + first commit (now waiting for configure)");

    let mut next_tick = Instant::now();
    while !state.exit {
        let tick_timeout = next_tick.saturating_duration_since(Instant::now());
        let timeout = state.next_dispatch_timeout().min(tick_timeout);
        if dispatch_wayland(&mut event_queue, &mut state, timeout).is_err() {
            break;
        }
        // Drain any wayland events that arrived during/just after the blocking
        // dispatch above (e.g. the burst of placeholder→real-size configures while
        // the output settles at startup) so EVERY pending configure is read + acked
        // by sctk before we attach a buffer below. Without this, a commit can race
        // an unread configure → the compositor posts `cannot attach a buffer before
        // ack_configure` and the client dies.
        for _ in 0..4 {
            if dispatch_wayland(&mut event_queue, &mut state, Duration::ZERO).is_err() {
                state.exit = true;
                break;
            }
            if conn.protocol_error().is_some() {
                state.exit = true;
                break;
            }
        }
        if state.exit {
            break;
        }
        run_due_ticks(&mut state, &mut next_tick);
        state.process_runtime_commands()?;
        state.pump_slint();
        let will_render = state.ready_to_render() && state.wants_redraw();
        state.apply_input_region(!will_render);
        if conn.protocol_error().is_some() {
            break;
        }
        // Render exactly once per iteration, now that all readable configures are
        // acked. (Committing from inside a handler or the timer was the original
        // race; everything just marks dirty and we draw here.)
        if will_render {
            state.render();
        }
        let mut idx = 0;
        while idx < state.popouts.len() {
            let will_render_popout = {
                let popout = &mut state.popouts[idx];
                popout.ready_to_render() && popout.wants_redraw()
            };
            if will_render_popout {
                state.render_popout(idx);
            }
            idx += 1;
        }
        // If a protocol error did slip through anyway, the connection is dead —
        // exit cleanly instead of busy-looping on it.
        if conn.protocol_error().is_some() {
            break;
        }
        if state.app.should_exit() {
            break;
        }
    }
    if let Some(e) = state.startup_error.take() {
        return Err(runtime_error(e));
    }
    Ok(())
}
