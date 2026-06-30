//! gnoblin-wallpaper — desktop wallpaper as a wlr-layer-shell SHM client.
//!
//! This role is deliberately not rendered through the shared Slint/EGL runtime:
//! it is a static fullscreen buffer, and Mesa's Wayland EGL swap path currently
//! segfaults for this surface in the mutter devkit. SHM keeps the layer-shell
//! lifecycle simple: wait for configure, ack via SCTK, attach one ARGB buffer.

use gnoblin_core::config::Config;
use gnoblin_core::ClientArgs;
use image::{imageops, RgbaImage};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};
use std::{
    error::Error,
    io::ErrorKind,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use wayland_client::{
    backend::WaylandError,
    globals::{registry_queue_init, GlobalList},
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, EventQueue, QueueHandle,
};

const CONFIG_POLL_INTERVAL: Duration = Duration::from_millis(250);

type RuntimeError = Box<dyn Error>;

fn runtime_error(message: impl Into<String>) -> RuntimeError {
    Box::new(std::io::Error::other(message.into()))
}

#[derive(Clone, Copy)]
struct Color {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Zoom,
    Scaled,
    Centered,
    Stretched,
    Tiled,
}

struct WallpaperPixels {
    fill: Color,
    image: Option<RgbaImage>,
    style: Style,
}

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    layer: LayerSurface,
    pool: SlotPool,
    buffer: Option<Buffer>,
    pixels: WallpaperPixels,
    config_path: Option<PathBuf>,
    config_mtime: Option<SystemTime>,
    width: u32,
    height: u32,
    scale: u32,
    configured: bool,
    dirty: bool,
    exit: bool,
}

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
        eprintln!("gnoblin-wallpaper: --output '{name}' not found; available outputs: {names:?}");
    }
    found
}

fn parse_color(hex: &str) -> Color {
    let h = hex.trim().trim_start_matches('#');
    let n = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    match h.len() {
        6 => match (n(0), n(2), n(4)) {
            (Some(r), Some(g), Some(b)) => Color { r, g, b, a: 255 },
            _ => default_color(),
        },
        8 => match (n(0), n(2), n(4), n(6)) {
            (Some(r), Some(g), Some(b), Some(a)) => Color { r, g, b, a },
            _ => default_color(),
        },
        _ => default_color(),
    }
}

fn default_color() -> Color {
    Color {
        r: 0x1d,
        g: 0x1f,
        b: 0x21,
        a: 255,
    }
}

fn parse_style(s: Option<&str>) -> Style {
    match s.unwrap_or("zoom").trim() {
        "scaled" | "spanned" => Style::Scaled,
        "centered" => Style::Centered,
        "stretched" => Style::Stretched,
        "tiled" | "wallpaper" => Style::Tiled,
        _ => Style::Zoom,
    }
}

fn load_wallpaper(path: Option<&str>) -> Option<RgbaImage> {
    let path = path?.trim();
    if path.is_empty() {
        return None;
    }
    image::open(path).ok().map(|img| img.to_rgba8())
}

impl State {
    fn screen_size(&self) -> (u32, u32) {
        self.output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .and_then(|i| {
                i.logical_size
                    .or_else(|| i.modes.iter().find(|m| m.current).map(|m| m.dimensions))
            })
            .map(|(w, h)| (w.max(1) as u32, h.max(1) as u32))
            .unwrap_or((self.width.max(1), self.height.max(1)))
    }

    fn draw(&mut self) -> Result<(), RuntimeError> {
        if !self.configured || !self.dirty {
            return Ok(());
        }
        let width = (self.width * self.scale).max(1);
        let height = (self.height * self.scale).max(1);
        let stride = width as i32 * 4;

        let reuse_available = self
            .buffer
            .as_ref()
            .filter(|buffer| buffer.height() == height as i32 && buffer.stride() == stride)
            .and_then(|buffer| self.pool.canvas(buffer))
            .is_some();

        if reuse_available {
            let buffer = self
                .buffer
                .as_ref()
                .ok_or_else(|| runtime_error("wallpaper buffer missing during reuse"))?;
            let canvas = self
                .pool
                .canvas(buffer)
                .ok_or_else(|| runtime_error("reusable wallpaper buffer canvas was unavailable"))?;
            fill(canvas, width, height, &self.pixels);
        } else {
            let buffer = {
                let (buffer, canvas) = self
                    .pool
                    .create_buffer(
                        width as i32,
                        height as i32,
                        stride,
                        wl_shm::Format::Argb8888,
                    )
                    .map_err(|e| runtime_error(format!("create wallpaper buffer: {e}")))?;
                fill(canvas, width, height, &self.pixels);
                buffer
            };
            self.buffer = Some(buffer);
        }

        let surface = self.layer.wl_surface();
        if let Ok(region) = Region::new(&self.compositor) {
            region.add(0, 0, self.width as i32, self.height as i32);
            surface.set_opaque_region(Some(region.wl_region()));
        }
        surface.damage_buffer(0, 0, width as i32, height as i32);
        let buffer = self
            .buffer
            .as_ref()
            .ok_or_else(|| runtime_error("wallpaper buffer missing before attach"))?;
        buffer
            .attach_to(surface)
            .map_err(|e| runtime_error(format!("attach wallpaper buffer: {e}")))?;
        self.layer.commit();
        self.dirty = false;
        Ok(())
    }

    fn reload_config_if_changed(&mut self) {
        let mtime = config_mtime(self.config_path.as_deref());
        if mtime == self.config_mtime {
            return;
        }
        self.config_mtime = mtime;
        self.pixels = load_pixels();
        self.dirty = true;
    }
}

fn config_mtime(path: Option<&Path>) -> Option<SystemTime> {
    path.and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

fn load_pixels() -> WallpaperPixels {
    let cfg = Config::load();
    let fill = parse_color(cfg.get("appearance", "background").unwrap_or("#1d1f21"));
    let wallpaper = std::env::var("GNOBLIN_WALLPAPER")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.get("appearance", "wallpaper").map(str::to_string));
    let style = parse_style(cfg.get("appearance", "wallpaper-style"));

    WallpaperPixels {
        fill,
        image: load_wallpaper(wallpaper.as_deref()),
        style,
    }
}

fn fill(canvas: &mut [u8], width: u32, height: u32, pixels: &WallpaperPixels) {
    for chunk in canvas.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[pixels.fill.b, pixels.fill.g, pixels.fill.r, pixels.fill.a]);
    }

    let Some(img) = &pixels.image else { return };
    let composed = compose_image(img, width, height, pixels.style);
    for (dst, src) in canvas.chunks_exact_mut(4).zip(composed.chunks_exact(4)) {
        let a = src[3] as u16;
        if a == 255 {
            dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
        } else if a > 0 {
            let inv = 255 - a;
            dst[0] = ((src[2] as u16 * a + dst[0] as u16 * inv) / 255) as u8;
            dst[1] = ((src[1] as u16 * a + dst[1] as u16 * inv) / 255) as u8;
            dst[2] = ((src[0] as u16 * a + dst[2] as u16 * inv) / 255) as u8;
            dst[3] = 255;
        }
    }
}

fn compose_image(img: &RgbaImage, width: u32, height: u32, style: Style) -> Vec<u8> {
    let mut out = RgbaImage::new(width, height);
    let (iw, ih) = img.dimensions();
    if iw == 0 || ih == 0 {
        return out.into_raw();
    }

    match style {
        Style::Stretched => {
            let scaled = imageops::resize(img, width, height, imageops::FilterType::Triangle);
            imageops::overlay(&mut out, &scaled, 0, 0);
        }
        Style::Scaled => {
            let scale = (width as f32 / iw as f32).min(height as f32 / ih as f32);
            let sw = ((iw as f32 * scale).round() as u32).clamp(1, width);
            let sh = ((ih as f32 * scale).round() as u32).clamp(1, height);
            let scaled = imageops::resize(img, sw, sh, imageops::FilterType::Triangle);
            let x = (width as i64 - sw as i64) / 2;
            let y = (height as i64 - sh as i64) / 2;
            imageops::overlay(&mut out, &scaled, x, y);
        }
        Style::Centered => {
            let x = (width as i64 - iw as i64) / 2;
            let y = (height as i64 - ih as i64) / 2;
            imageops::overlay(&mut out, img, x, y);
        }
        Style::Tiled => {
            let mut y = 0;
            while y < height {
                let mut x = 0;
                while x < width {
                    imageops::overlay(&mut out, img, x as i64, y as i64);
                    x += iw;
                }
                y += ih;
            }
        }
        Style::Zoom => {
            let scale = (width as f32 / iw as f32).max(height as f32 / ih as f32);
            let sw = ((iw as f32 * scale).ceil() as u32).max(1);
            let sh = ((ih as f32 * scale).ceil() as u32).max(1);
            let scaled = imageops::resize(img, sw, sh, imageops::FilterType::Triangle);
            let x = (width as i64 - sw as i64) / 2;
            let y = (height as i64 - sh as i64) / 2;
            imageops::overlay(&mut out, &scaled, x, y);
        }
    }

    out.into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn alpha_at(raw: &[u8], width: u32, x: u32, y: u32) -> u8 {
        raw[((y * width + x) * 4 + 3) as usize]
    }

    #[test]
    fn scaled_preserves_aspect_ratio_and_letterboxes() {
        let img = RgbaImage::from_pixel(2, 1, Rgba([255, 0, 0, 255]));
        let raw = compose_image(&img, 4, 4, Style::Scaled);

        assert_eq!(alpha_at(&raw, 4, 0, 0), 0);
        assert_eq!(alpha_at(&raw, 4, 3, 0), 0);
        assert_eq!(alpha_at(&raw, 4, 0, 1), 255);
        assert_eq!(alpha_at(&raw, 4, 3, 2), 255);
        assert_eq!(alpha_at(&raw, 4, 0, 3), 0);
        assert_eq!(alpha_at(&raw, 4, 3, 3), 0);
    }

    #[test]
    fn stretched_distorts_to_fill_the_output() {
        let img = RgbaImage::from_pixel(2, 1, Rgba([255, 0, 0, 255]));
        let raw = compose_image(&img, 4, 4, Style::Stretched);

        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(alpha_at(&raw, 4, x, y), 255);
            }
        }
    }

    #[test]
    fn zoom_preserves_aspect_ratio_and_crops_to_fill() {
        let img = RgbaImage::from_pixel(2, 1, Rgba([255, 0, 0, 255]));
        let raw = compose_image(&img, 4, 4, Style::Zoom);

        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(alpha_at(&raw, 4, x, y), 255);
            }
        }
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
        let scale = new_scale.max(1) as u32;
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        surface.set_buffer_scale(new_scale.max(1));
        self.dirty = true;
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
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

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
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
        self.width = w.max(1);
        self.height = h.max(1);
        self.configured = true;
        self.dirty = true;
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_layer!(State);
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
    } else {
        drop(guard);
    }

    event_queue.dispatch_pending(state)?;
    Ok(())
}

fn main() {
    if let Err(e) = try_main() {
        eprintln!("gnoblin-wallpaper: {e}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), RuntimeError> {
    let config_path = Config::path();
    let config_mtime = config_mtime(config_path.as_deref());
    let pixels = load_pixels();

    let conn = Connection::connect_to_env()
        .map_err(|e| runtime_error(format!("connect to Wayland: {e}")))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| runtime_error(format!("registry init: {e}")))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| runtime_error(format!("bind wl_compositor: {e}")))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| runtime_error(format!("bind wlr-layer-shell: {e}")))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| runtime_error(format!("bind wl_shm: {e}")))?;

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
        Layer::Background,
        Some("gnoblin-wallpaper"),
        target_output.as_ref(),
    );
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_size(0, 0);
    // Background surfaces should stretch under panels/docks rather than being
    // shifted into their exclusive work area. In wlr-layer-shell that is the
    // protocol-defined meaning of exclusive_zone = -1.
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    if let Ok(region) = Region::new(&compositor) {
        // Empty input region: the wallpaper paints pixels only and never
        // participates in pointer hit testing.
        layer
            .wl_surface()
            .set_input_region(Some(region.wl_region()));
    }
    layer.commit();

    let pool =
        SlotPool::new(1, &shm).map_err(|e| runtime_error(format!("create shm pool: {e}")))?;
    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        shm,
        layer,
        pool,
        buffer: None,
        pixels,
        config_path,
        config_mtime,
        width: 1280,
        height: 800,
        scale: 1,
        configured: false,
        dirty: false,
        exit: false,
    };

    while !state.exit {
        if dispatch_wayland(&mut event_queue, &mut state, CONFIG_POLL_INTERVAL).is_err()
            || conn.protocol_error().is_some()
        {
            break;
        }
        // SCTK acks layer-surface configures in the handler. Drain configures
        // already queued from the same burst before attaching the wallpaper
        // buffer, so resize storms cannot leave a newer configure unread.
        for _ in 0..4 {
            match event_queue.dispatch_pending(&mut state) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => {
                    state.exit = true;
                    break;
                }
            }
            if conn.protocol_error().is_some() {
                state.exit = true;
                break;
            }
        }
        if state.exit {
            break;
        }
        state.reload_config_if_changed();
        state.draw()?;
        if conn.protocol_error().is_some() {
            break;
        }
    }
    Ok(())
}
