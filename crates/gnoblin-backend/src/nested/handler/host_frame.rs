//! The window frame on the wire: what the host answered to `xdg-decoration`, the four strips, and the pointer over them.
//!
//! The decisions (is a frame wanted, how much room it takes, what lies under a point, what it looks like) are
//! [`super::super::frame`]'s, tested without a socket. This file keeps the objects that carry them:
//!
//! 1. **A configure is collected, then decided.** `xdg_toplevel.configure` states the window's size and states,
//!    `zxdg_toplevel_decoration_v1.configure` the host's answer, and the `xdg_surface.configure` that ends the batch is
//!    when they are read together ([`Handler::close_configure`]): the output's size is the window's less the room the
//!    frame takes then, so an answer that arrives after the size in the same batch still counts.
//! 2. **The strips follow the table, not the configure.** [`Handler::sync_frames`] puts the strips, the window geometry
//!    and the buffers in line with the layout and the scale the table has applied, so a scale that changes after a
//!    configure redraws them at the new scale and a hover redraws the title bar, with one routine for all three.
//! 3. **The strips are synchronised subsurfaces.** What is staged on them lands with the next commit of the session's
//!    own surface, which is the one that carries the new buffer after a resize, so the frame and the pixels it surrounds
//!    are never on screen at two sizes.
//! 4. **The pointer is the frame's while it is over a strip.** The strips are surfaces of their own, so the host's enter
//!    names one of them and the session's pointer leaves the session ([`PointerFocus`]); the session never sees a point
//!    of the frame, and the point it sees over its own surface is the surface's own, with no offset to apply.

use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use gnoblin_paint::SharedPainter;
use gnoblin_types::{Rect, Scale, Size};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_shm, wl_subcompositor, wl_subsurface, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::decoration::zv1::client::{zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{ResizeEdge, XdgToplevel};

use crate::log;
use crate::nested::frame::{
    Button, DOUBLE_CLICK_MS, DecorationAnswer, FrameHit, FrameLayout, HostWindowState, Inset, Look, Part, content_size,
    paint_list, wants_frame,
};
use crate::nested::surface::{anonymous_file, packed, plan};

use super::{Handler, Objects, OutputKey};

/// `BTN_LEFT` and `BTN_RIGHT` of `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// How far a second press may be from the first and still be a double click, in logical pixels.
const DOUBLE_CLICK_SLOP: f64 = 4.0;

/// Where the host's pointer is, as far as the frame is concerned.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(super) enum PointerFocus {
    /// Over the session's own surface, or over nothing of ours: the session's seat has it.
    #[default]
    Elsewhere,
    /// Over one strip of an output's frame, at this point of the strip.
    Frame {
        /// The output.
        index: usize,
        /// The strip.
        part: Part,
        /// Where in the strip.
        x: f64,
        /// Where in the strip.
        y: f64,
    },
}

/// What a strip was drawn as, so the same thing is not drawn again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Drawn {
    rect: Rect,
    scale: Scale,
    look: Look,
}

/// One strip of a frame: a subsurface of the session's surface and what is attached to it.
struct Strip {
    surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
    viewport: Option<wp_viewport::WpViewport>,
    buffer: Option<wl_buffer::WlBuffer>,
    drawn: Option<Drawn>,
}

impl Strip {
    /// Gives the strip back: the subsurface is unmapped at once, and the buffer is let go.
    fn destroy(self) {
        if let Some(buffer) = self.buffer {
            buffer.destroy();
        }
        if let Some(viewport) = self.viewport {
            viewport.destroy();
        }
        self.subsurface.destroy();
        self.surface.destroy();
    }
}

/// One output's window frame: the host's answer, the configure being read, what is drawn and what the pointer is doing.
pub(super) struct HostFrame {
    /// The title the title bar shows.
    title: String,
    /// What the host answered when asked for a server-side frame. [`None`] while it has not, and for a host that has no
    /// `xdg-decoration`: which the protocol states is client-side.
    pub(super) answer: Option<DecorationAnswer>,
    /// The window's size and states from the configure being collected.
    pending: Option<(i32, i32, HostWindowState)>,
    /// The states of the last configure.
    state: HostWindowState,
    /// The size of the session's output, from the last configure.
    content: Size,
    /// The frame the session draws now, or [`None`] when it draws none.
    wanted: Option<FrameLayout>,
    /// The four strips, by [`Part::index`].
    strips: [Option<Strip>; 4],
    /// The window geometry the host was last told, or [`None`] while it was told none.
    geometry: Option<Rect>,
    /// The title bar button the pointer is over.
    hover: Option<Button>,
    /// The title bar button the primary button is held down on.
    pressed: Option<Button>,
    /// The last primary press on the title bar: when, and where in the title bar.
    last_press: Option<(u32, f64, f64)>,
    /// The cursor shape last asked of the host for the pointer over the frame.
    shape: Option<wp_cursor_shape_device_v1::Shape>,
}

impl HostFrame {
    /// An output's frame before the host has said anything.
    pub(super) fn new(title: &str, initial: Size) -> Self {
        Self {
            title: title.to_owned(),
            answer: None,
            pending: None,
            state: HostWindowState::default(),
            content: initial,
            wanted: None,
            strips: [None, None, None, None],
            geometry: None,
            hover: None,
            pressed: None,
            last_press: None,
            shape: None,
        }
    }

    /// Whether any strip is on the wire.
    pub(super) fn is_drawn(&self) -> bool {
        self.strips.iter().any(Option::is_some)
    }

    /// Gives every strip back.
    pub(super) fn destroy(&mut self) {
        for strip in &mut self.strips {
            if let Some(strip) = strip.take() {
                strip.destroy();
            }
        }
        self.geometry = None;
    }

    /// The strip a surface is, if it is one.
    fn part_of(&self, surface: &wl_surface::WlSurface) -> Option<Part> {
        Part::ALL.into_iter().find(|part| {
            self.strips
                .get(part.index())
                .and_then(Option::as_ref)
                .is_some_and(|strip| strip.surface.id() == surface.id())
        })
    }

    /// How the strips look now.
    fn look(&self) -> Look {
        Look {
            active: self.state.activated,
            maximized: self.state.maximized,
            hover: self.hover,
            pressed: self.pressed,
        }
    }
}

/// What the strips are made and drawn with.
struct Wire<'a> {
    compositor: &'a wl_compositor::WlCompositor,
    subcompositor: &'a wl_subcompositor::WlSubcompositor,
    viewporter: Option<&'a wp_viewporter::WpViewporter>,
    shm: &'a wl_shm::WlShm,
    format: wl_shm::Format,
    painter: &'a SharedPainter,
    qh: &'a QueueHandle<Handler>,
}

/// The buffer of a strip. It carries no state: a strip's buffer is attached once and given back when the next one
/// replaces it, so the release the host sends for it is of no interest.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StripBuffer;

impl Handler {
    /// A toplevel configure arrived: its size and states are kept until the `xdg_surface.configure` that ends the batch.
    pub(super) fn note_window_configure(&mut self, index: usize, width: i32, height: i32, states: &[u8]) {
        if let Some(Some(objects)) = self.objects.get_mut(index) {
            objects.frame.pending = Some((width, height, HostWindowState::from_wire(states)));
        }
    }

    /// What the host answered when `index` asked it to draw the frame, if it has.
    pub(crate) fn decoration_answer(&self, index: usize) -> Option<DecorationAnswer> {
        self.objects.get(index)?.as_ref()?.frame.answer
    }

    /// Whether the session's own frame is on the wire for `index`.
    pub(crate) fn frame_drawn(&self, index: usize) -> bool {
        self.objects
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|objects| objects.frame.is_drawn())
    }

    /// The host answered what it draws.
    pub(super) fn note_decoration_answer(&mut self, index: usize, answer: DecorationAnswer) {
        if let Some(Some(objects)) = self.objects.get_mut(index) {
            objects.frame.answer = Some(answer);
        }
    }

    /// An `xdg_surface.configure` ended a batch: decides whether the session draws a frame, and tells the table the size
    /// the session's output takes.
    pub(super) fn close_configure(&mut self, index: usize) {
        let setting = self.decorations;
        let can_draw = self.subcompositor.is_some();
        let initial = self.initial_size;
        let Some(Some(objects)) = self.objects.get_mut(index) else {
            return;
        };
        let frame = &mut objects.frame;
        let (width, height, state) = frame.pending.take().unwrap_or((0, 0, frame.state));
        let wanted = can_draw && wants_frame(setting, frame.answer, state);
        let inset = if wanted { Inset::of(state) } else { Inset::NONE };
        let content = content_size(width, height, inset, initial);
        if wanted != frame.wanted.is_some() {
            log::debug!(
                area = "nested",
                answer = ?frame.answer,
                frame = wanted,
                "the session's window frame changed"
            );
        }
        frame.state = state;
        frame.content = content;
        frame.wanted = wanted.then_some(FrameLayout { content, inset });
        self.table.configure(index, content);
    }

    /// Puts every output's frame in line with the layout, the scale and the look it has now. Answers the outputs that
    /// have something staged for the host: it lands with the next commit of the output's surface.
    pub(crate) fn sync_frames(&mut self, qh: &QueueHandle<Self>) -> Vec<usize> {
        let Some(format) = self.format else {
            return Vec::new();
        };
        let scales: Vec<Option<Scale>> = (0..self.objects.len())
            .map(|index| self.table.applied(index).map(|(_, scale)| scale))
            .collect();
        let Some(subcompositor) = self.subcompositor.as_ref() else {
            // No subsurfaces: a frame was never wanted, and there is nothing to give back.
            return Vec::new();
        };
        let wire = Wire {
            compositor: &self.compositor,
            subcompositor,
            viewporter: self.viewporter.as_ref(),
            shm: &self.shm,
            format,
            painter: &self.painter,
            qh,
        };
        let mut staged = Vec::new();
        for (index, objects) in self.objects.iter_mut().enumerate() {
            let (Some(objects), Some(Some(scale))) = (objects.as_mut(), scales.get(index)) else {
                continue;
            };
            if put_frame(&wire, objects, *scale) {
                staged.push(index);
            }
        }
        staged
    }

    /// The strip a surface is, and the output it frames.
    pub(super) fn frame_part(&self, surface: &wl_surface::WlSurface) -> Option<(usize, Part)> {
        self.objects.iter().enumerate().find_map(|(index, objects)| {
            objects
                .as_ref()
                .and_then(|objects| objects.frame.part_of(surface))
                .map(|part| (index, part))
        })
    }

    /// The host's pointer entered a strip.
    pub(super) fn frame_enter(&mut self, index: usize, part: Part, serial: u32, x: f64, y: f64) {
        self.pointer_serial = serial;
        if let Some(Some(objects)) = self.objects.get_mut(index) {
            // The host forgets the shape when the pointer leaves, so the next one is asked for again.
            objects.frame.shape = None;
        }
        self.pointer_on = PointerFocus::Frame { index, part, x, y };
        self.frame_over(index, part, x, y);
    }

    /// The host's pointer moved over a strip.
    pub(super) fn frame_motion(&mut self, x: f64, y: f64) {
        if let PointerFocus::Frame { index, part, .. } = self.pointer_on {
            self.pointer_on = PointerFocus::Frame { index, part, x, y };
            self.frame_over(index, part, x, y);
        }
    }

    /// The host's pointer left the strip it was over.
    pub(super) fn frame_leave(&mut self) {
        if let PointerFocus::Frame { index, .. } = self.pointer_on
            && let Some(Some(objects)) = self.objects.get_mut(index)
        {
            objects.frame.hover = None;
            objects.frame.pressed = None;
        }
        self.pointer_on = PointerFocus::Elsewhere;
    }

    /// Follows the pointer over a strip: the button under it is hovered, and the cursor is the shape its place asks for.
    fn frame_over(&mut self, index: usize, part: Part, x: f64, y: f64) {
        let hit = self.frame_hit(index, part, x, y);
        let shape = shape_of(hit);
        let Some(Some(objects)) = self.objects.get_mut(index) else {
            return;
        };
        objects.frame.hover = match hit {
            Some(FrameHit::Button(button)) => Some(button),
            _ => None,
        };
        if objects.frame.shape != Some(shape)
            && let Some(device) = &self.shape_device
        {
            device.set_shape(self.pointer_serial, shape);
            objects.frame.shape = Some(shape);
        }
    }

    /// What lies under a point of a strip.
    fn frame_hit(&self, index: usize, part: Part, x: f64, y: f64) -> Option<FrameHit> {
        let layout = self.objects.get(index)?.as_ref()?.frame.wanted?;
        layout.hit(part, x, y)
    }

    /// A pointer button was pressed or released while the pointer is over a strip.
    pub(super) fn frame_button(&mut self, time: u32, serial: u32, button: u32, pressed: bool) {
        let PointerFocus::Frame { index, part, x, y } = self.pointer_on else {
            return;
        };
        let hit = self.frame_hit(index, part, x, y);
        let Some(Some(objects)) = self.objects.get_mut(index) else {
            return;
        };
        let toplevel = objects.toplevel.clone();
        let state = objects.frame.state;
        let frame = &mut objects.frame;
        match (button, pressed) {
            (BTN_LEFT, true) => match hit {
                Some(FrameHit::Button(button)) => frame.pressed = Some(button),
                Some(FrameHit::Resize(edge)) => toplevel.resize(&self.seat_global, serial, edge),
                Some(FrameHit::Title) => {
                    let double = frame.last_press.is_some_and(|(before, at_x, at_y)| {
                        time.wrapping_sub(before) <= DOUBLE_CLICK_MS
                            && (x - at_x).abs() <= DOUBLE_CLICK_SLOP
                            && (y - at_y).abs() <= DOUBLE_CLICK_SLOP
                    });
                    if double {
                        frame.last_press = None;
                        toggle_maximised(&toplevel, state);
                    } else {
                        frame.last_press = Some((time, x, y));
                        toplevel._move(&self.seat_global, serial);
                    }
                }
                None => {}
            },
            (BTN_LEFT, false) => {
                // A button acts when it is let go over the button it was pressed on, so a press that slides off cancels.
                if let Some(held) = frame.pressed.take()
                    && hit == Some(FrameHit::Button(held))
                {
                    match held {
                        Button::Close => self.table.close(index),
                        Button::Minimise => toplevel.set_minimized(),
                        Button::Maximise => toggle_maximised(&toplevel, state),
                    }
                }
            }
            (BTN_RIGHT, true) if hit == Some(FrameHit::Title) => {
                // The menu's place is relative to the session's surface, which the title bar lies above.
                if let Some(rect) = frame.wanted.and_then(|layout| layout.part(part)) {
                    let at_x = rect.x + x as i32;
                    let at_y = rect.y + y as i32;
                    toplevel.show_window_menu(&self.seat_global, serial, at_x, at_y);
                }
            }
            _ => {}
        }
    }

    /// A touch began on a strip: on the title bar it moves the window, as a press does.
    pub(super) fn frame_touch(&mut self, index: usize, part: Part, serial: u32, x: f64, y: f64) {
        if self.frame_hit(index, part, x, y) != Some(FrameHit::Title) {
            return;
        }
        if let Some(Some(objects)) = self.objects.get(index) {
            objects.toplevel._move(&self.seat_global, serial);
        }
    }
}

/// Maximises a window that is not, and restores one that is.
fn toggle_maximised(toplevel: &XdgToplevel, state: HostWindowState) {
    if state.maximized {
        toplevel.unset_maximized();
    } else {
        toplevel.set_maximized();
    }
}

/// The cursor shape a place of the frame asks for.
fn shape_of(hit: Option<FrameHit>) -> wp_cursor_shape_device_v1::Shape {
    use wp_cursor_shape_device_v1::Shape;
    match hit {
        Some(FrameHit::Resize(edge)) => match edge {
            ResizeEdge::Top => Shape::NResize,
            ResizeEdge::Bottom => Shape::SResize,
            ResizeEdge::Left => Shape::WResize,
            ResizeEdge::Right => Shape::EResize,
            ResizeEdge::TopLeft => Shape::NwResize,
            ResizeEdge::TopRight => Shape::NeResize,
            ResizeEdge::BottomLeft => Shape::SwResize,
            ResizeEdge::BottomRight => Shape::SeResize,
            _ => Shape::Default,
        },
        _ => Shape::Default,
    }
}

/// Puts one output's frame in line with what it should be: makes, moves, draws and gives back strips, and states the
/// window geometry. Whether anything is staged for the host.
fn put_frame(wire: &Wire<'_>, objects: &mut Objects, scale: Scale) -> bool {
    let look = objects.frame.look();
    let Objects {
        surface,
        xdg_surface,
        frame,
        ..
    } = objects;
    let Some(layout) = frame.wanted else {
        if !frame.is_drawn() && frame.geometry.is_none() {
            return false;
        }
        frame.destroy();
        // The window is the session's surface again.
        xdg_surface.set_window_geometry(
            0,
            0,
            i32::try_from(frame.content.width).unwrap_or(i32::MAX),
            i32::try_from(frame.content.height).unwrap_or(i32::MAX),
        );
        return true;
    };
    let mut staged = false;
    let window = layout.window();
    if frame.geometry != Some(window) {
        xdg_surface.set_window_geometry(
            window.x,
            window.y,
            i32::try_from(window.width).unwrap_or(i32::MAX),
            i32::try_from(window.height).unwrap_or(i32::MAX),
        );
        frame.geometry = Some(window);
        staged = true;
    }
    for part in Part::ALL {
        let Some(slot) = frame.strips.get_mut(part.index()) else {
            continue;
        };
        let Some(rect) = layout.part(part) else {
            if let Some(strip) = slot.take() {
                strip.destroy();
                staged = true;
            }
            continue;
        };
        let wanted = Drawn {
            rect,
            scale,
            look: Look::of(part, look),
        };
        if slot.as_ref().is_some_and(|strip| strip.drawn == Some(wanted)) {
            continue;
        }
        let strip = slot.get_or_insert_with(|| make_strip(wire, surface));
        strip.subsurface.set_position(rect.x, rect.y);
        if draw_strip(wire, strip, part, &layout, wanted, &frame.title) {
            strip.drawn = Some(wanted);
        }
        staged = true;
    }
    staged
}

/// A strip with nothing on it, under `parent`.
fn make_strip(wire: &Wire<'_>, parent: &wl_surface::WlSurface) -> Strip {
    let surface = wire.compositor.create_surface(wire.qh, ());
    let subsurface = wire.subcompositor.get_subsurface(&surface, parent, wire.qh, ());
    let viewport = wire
        .viewporter
        .map(|viewporter| viewporter.get_viewport(&surface, wire.qh, ()));
    Strip {
        surface,
        subsurface,
        viewport,
        buffer: None,
        drawn: None,
    }
}

/// Draws a strip into a new buffer and attaches it. Whether it was drawn: a frame that cannot be drawn is left as it was.
fn draw_strip(
    wire: &Wire<'_>,
    strip: &mut Strip,
    part: Part,
    layout: &FrameLayout,
    wanted: Drawn,
    title: &str,
) -> bool {
    let Drawn { rect, scale, look } = wanted;
    let planned = plan(
        Size::new(rect.width, rect.height),
        scale,
        strip.viewport.is_some(),
        strip.surface.version() >= 3,
    );
    // The factor the strip is painted at is the one its buffer is planned at: a fractional scale's own when a viewport
    // states it, and the whole one the plan settled on when it did not.
    let factor = match (planned.destination, planned.buffer_scale) {
        (Some(_), _) => f64::from(scale.numerator()) / f64::from(scale.denominator()),
        (None, Some(steps)) => f64::from(steps),
        (None, None) => 1.0,
    };
    let painter = wire.painter.get();
    let list = paint_list(part, layout, look, title, painter);
    let Some(rendered) = painter.paint_scaled(&list, rect.width, rect.height, factor) else {
        log::warn!(area = "nested", ?part, "a frame strip could not be painted");
        return false;
    };
    let picture = rendered.picture;
    if Size::new(picture.width, picture.height) != planned.physical {
        log::warn!(
            area = "nested",
            ?part,
            "a frame strip was painted at a size its buffer does not have"
        );
        return false;
    }
    let pixels: Vec<u8> = picture
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| packed(pixel, wire.format))
        .collect();
    let (Ok(width), Ok(height), Ok(length)) = (
        i32::try_from(picture.width),
        i32::try_from(picture.height),
        i32::try_from(pixels.len()),
    ) else {
        return false;
    };
    let written = anonymous_file(pixels.len()).and_then(|file| file.write_all_at(&pixels, 0).map(|()| file));
    let file = match written {
        Ok(file) => file,
        Err(error) => {
            log::warn!(area = "nested", ?error, "a frame strip's buffer could not be made");
            return false;
        }
    };
    let pool = wire.shm.create_pool(file.as_fd(), length, wire.qh, ());
    let buffer = pool.create_buffer(0, width, height, width * 4, wire.format, wire.qh, StripBuffer);
    // The buffer keeps the memory alive for as long as the host reads it.
    pool.destroy();
    strip.surface.attach(Some(&buffer), 0, 0);
    if let Some(steps) = planned.buffer_scale {
        strip.surface.set_buffer_scale(steps);
    }
    if let Some(viewport) = &strip.viewport {
        match planned.destination {
            Some(size) => viewport.set_destination(
                i32::try_from(size.width).unwrap_or(i32::MAX),
                i32::try_from(size.height).unwrap_or(i32::MAX),
            ),
            None => viewport.set_destination(-1, -1),
        }
    }
    strip.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
    // The strip is synchronised: this stages the state, and the session's surface's next commit applies it.
    strip.surface.commit();
    if let Some(old) = strip.buffer.replace(buffer) {
        old.destroy();
    }
    true
}

// The manager and the subsurface protocol say nothing a session acts on, and the strips' buffers are let go when they
// are replaced.
wayland_client::delegate_noop!(Handler: ignore wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(Handler: ignore wl_subsurface::WlSubsurface);
wayland_client::delegate_noop!(Handler: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
wayland_client::delegate_noop!(Handler: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
wayland_client::delegate_noop!(Handler: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);

impl Dispatch<wl_buffer::WlBuffer, StripBuffer> for Handler {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &StripBuffer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = event {
            match mode {
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ServerSide) => {
                    state.note_decoration_answer(key.0, DecorationAnswer::Server);
                }
                WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ClientSide) => {
                    state.note_decoration_answer(key.0, DecorationAnswer::Client);
                }
                _ => {}
            }
        }
    }
}
