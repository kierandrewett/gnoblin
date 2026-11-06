//! The client side of the protocols a host compositor speaks to a nested session.
//!
//! Every `Dispatch` implementation below is an adapter: a protocol event becomes one
//! call on [`Table`] (the parent-facing state) or on the seat, and nothing else. No
//! handler decides anything, which is what lets the decisions be tested without a
//! socket and keeps the `wayland-client` types out of the rest of the crate.
//!
//! # What the parent drives and what it does not
//!
//! - **Size and scale.** `xdg_toplevel.configure` states the size the parent wants,
//!   and `wp_fractional_scale_v1.surface_preferred_scale` the scale to draw it at.
//!   The configure is acknowledged at once: the size it states is applied to the
//!   target before the next commit, so the buffer always matches the configure it
//!   answers.
//! - **Pacing.** `wl_surface.frame` callbacks say when the parent is ready for
//!   another frame. The callback carries the output's index as its user data, so a
//!   callback that arrives late for a target that has been resized still clears the
//!   right one.
//! - **Buffers.** `wl_buffer.release` carries the pool generation and the slot, so a
//!   release for a pool that a resize replaced is credited to the pool that made the
//!   buffer and ignored by the one that replaced it.
//! - **Input.** The seat's keyboard, pointer and touch objects are created from
//!   `wl_seat.capabilities` and their events are converted in [`super::input`]. The
//!   keyboard's keymap is read out of the file descriptor the parent sends and kept
//!   for the caller: the backend's [`InputEvent`](crate::InputEvent) states scan
//!   codes, so the layout has to be applied by whoever owns the keymap.
//!
//! # What is not here
//!
//! `zwp_relative_pointer_v1`, pointer constraints, tablets and text input are the
//! input slice's, not this one's: each needs an [`InputEvent`](crate::InputEvent)
//! variant this backend cannot honestly produce before the seat can act on it. A
//! parent that offers them simply sees them unbound.

use std::io::Read;

use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use gnoblin_types::{OutputId, Size};

use super::UNCONSTRAINED_SIZE;
use super::input::position;
use super::records::Table;
use super::scale_from_parent;

/// The index of one nested output, as an object's user data.
///
/// It is the record index rather than the [`OutputId`], and that is deliberate: the
/// index is what the target table is keyed by, so a callback does not have to search
/// for the output it belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct OutputKey(pub usize);

/// Which pool generation and which slot of it a buffer belongs to.
///
/// The generation is what makes a release unambiguous: a resize replaces the pool,
/// and a release for the pool that was replaced must not free a slot of the new one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SlotKey {
    /// The record index of the output the buffer is drawn for.
    pub index: usize,
    /// The pool generation the slot belongs to.
    pub generation: u32,
    /// The slot within that pool.
    pub slot: u8,
}

/// The proxies of one nested output.
pub(crate) struct Objects {
    /// The surface the output is drawn on.
    pub surface: wl_surface::WlSurface,
    /// The viewport a fractional scale is applied through, if the parent has one.
    pub viewport: Option<wp_viewport::WpViewport>,
    /// The xdg surface that gives the output's surface its window role.
    xdg_surface: xdg_surface::XdgSurface,
    /// The toplevel the parent shows for the output, which carries its title and app id.
    toplevel: xdg_toplevel::XdgToplevel,
    /// The fractional scale object, present only when the parent offers the protocol.
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
}

/// The state every event on the parent connection is dispatched into.
pub(crate) struct Handler {
    /// The per-output records and the parent connection state.
    table: Table,
    /// The proxies for each output, by output index; [`None`] once the output is closed.
    objects: Vec<Option<Objects>>,
    /// The parent's compositor global, used to create each output's surface.
    pub(crate) compositor: wl_compositor::WlCompositor,
    /// The parent's shared-memory global, used to create buffer pools.
    pub(crate) shm: wl_shm::WlShm,
    /// The parent's window manager base, used to give each surface an xdg role.
    wm_base: xdg_wm_base::XdgWmBase,
    /// The parent's seat, which the pointer, keyboard and touch objects come from.
    seat_global: wl_seat::WlSeat,
    /// The parent's fractional scale manager, if it offers one.
    fractional_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    /// The parent's viewporter, if it offers one, used to apply a fractional scale.
    viewporter: Option<wp_viewporter::WpViewporter>,
    /// The seat's pointer, present while the seat advertises pointer.
    pointer: Option<wl_pointer::WlPointer>,
    /// The seat's keyboard, present while the seat advertises keyboard.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The seat's touch device, present while the seat advertises touch.
    touch: Option<wl_touch::WlTouch>,
    /// The XKB keymap the parent sent for the keyboard, or [`None`] until one arrives.
    keymap: Option<String>,
    /// The shared-memory pixel format, chosen from the parent's formats with ARGB preferred.
    format: Option<wl_shm::Format>,
    /// The integer scale of the parent's output, the fallback for a parent that has
    /// no fractional-scale protocol.
    integer_scale: i32,
    /// The parent output's refresh rate in millihertz, or zero when the parent has not said.
    refresh_mhz: u32,
    /// The app id set on every toplevel.
    app_id: String,
}

impl Handler {
    /// A handler for a connection whose globals have been bound.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        compositor: wl_compositor::WlCompositor,
        shm: wl_shm::WlShm,
        wm_base: xdg_wm_base::XdgWmBase,
        seat_global: wl_seat::WlSeat,
        fractional_manager: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
        viewporter: Option<wp_viewporter::WpViewporter>,
        app_id: String,
    ) -> Self {
        Self {
            table: Table::new(),
            objects: Vec::new(),
            compositor,
            shm,
            wm_base,
            seat_global,
            fractional_manager,
            viewporter,
            pointer: None,
            keyboard: None,
            touch: None,
            keymap: None,
            format: None,
            integer_scale: 0,
            refresh_mhz: 0,
            app_id,
        }
    }

    /// Creates one nested output's surface and asks the parent to place it.
    pub fn create_output(
        &mut self,
        qh: &QueueHandle<Self>,
        output: OutputId,
        connector: &str,
        title: &str,
        fullscreen: bool,
    ) -> usize {
        let index = self.objects.len();
        let surface = self.compositor.create_surface(qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, qh, OutputKey(index));
        let toplevel = xdg_surface.get_toplevel(qh, OutputKey(index));
        toplevel.set_app_id(self.app_id.clone());
        toplevel.set_title(title.to_owned());
        if fullscreen {
            // A session that fills the parent's output is what the host expects.
            // A parent that does not implement fullscreen answers with a configure
            // of its own choosing, which the output follows all the same.
            toplevel.set_fullscreen(None);
        }
        let fractional = self
            .fractional_manager
            .as_ref()
            .map(|manager| manager.get_fractional_scale(&surface, qh, OutputKey(index)));
        let viewport = self
            .viewporter
            .as_ref()
            .map(|viewporter| viewporter.get_viewport(&surface, qh, ()));
        // The commit with no buffer is the one that asks for the first configure:
        // xdg-shell states that a surface with no buffer and no configure gets one.
        surface.commit();
        self.objects.push(Some(Objects {
            surface,
            viewport,
            xdg_surface,
            toplevel,
            fractional,
        }));
        self.table.add(output, connector)
    }

    /// Destroys one output's objects, after the parent asked for it to close.
    pub fn destroy_output(&mut self, index: usize) {
        if let Some(objects) = self.objects.get_mut(index).and_then(Option::take) {
            if let Some(fractional) = objects.fractional {
                fractional.destroy();
            }
            if let Some(viewport) = objects.viewport {
                viewport.destroy();
            }
            objects.toplevel.destroy();
            objects.xdg_surface.destroy();
            objects.surface.destroy();
        }
    }

    /// One output's proxies.
    pub fn objects(&self, index: usize) -> Option<&Objects> {
        self.objects.get(index).and_then(Option::as_ref)
    }

    /// The parent-facing state.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// The parent-facing state, mutably.
    pub fn table_mut(&mut self) -> &mut Table {
        &mut self.table
    }

    /// The `wl_shm` format the parent and this backend agree on, once known.
    pub fn format(&self) -> Option<wl_shm::Format> {
        self.format
    }

    /// The refresh rate the parent's output reports, in millihertz, if it reported
    /// one.
    pub fn refresh_mhz(&self) -> u32 {
        self.refresh_mhz
    }

    /// The parent's keymap, as the `xkb_v1` text it sent.
    ///
    /// [`None`] until the parent has sent one, which is a parent with no keyboard.
    pub fn keymap(&self) -> Option<&str> {
        self.keymap.as_deref()
    }

    /// The output a surface this backend draws on belongs to.
    ///
    /// Answering [`None`] for a surface that has not been configured is what stops
    /// input being attributed to an output that does not exist yet.
    fn output_of(&self, surface: &wl_surface::WlSurface) -> Option<OutputId> {
        let id = surface.id();
        let index = self
            .objects
            .iter()
            .position(|objects| objects.as_ref().is_some_and(|objects| objects.surface.id() == id))?;
        self.table.applied(index)?;
        self.table.output(index)
    }

    /// Records one `wl_shm` format, keeping the first one this backend can draw.
    ///
    /// `Argb8888` is preferred because it is the only 32-bit format every compositor
    /// offers and it carries the alpha the frame already has; `Xrgb8888` is taken
    /// when it is all the parent offers.
    fn note_format(&mut self, format: wl_shm::Format) {
        match (self.format, format) {
            (Some(wl_shm::Format::Argb8888), _) => {}
            (_, wl_shm::Format::Argb8888) => self.format = Some(format),
            (None, wl_shm::Format::Xrgb8888) => self.format = Some(format),
            (Some(_), wl_shm::Format::Xrgb8888) => {}
            _ => {}
        }
    }

    /// Reads the keymap out of the descriptor the parent sent.
    ///
    /// A keymap that is not `xkb_v1`, or one that cannot be read, is left unset: the
    /// backend states scan codes either way, and a session with no keymap is a
    /// session whose seat composes its own.
    fn note_keymap(&mut self, format: &WEnum<wl_keyboard::KeymapFormat>, fd: std::os::fd::OwnedFd, size: u32) {
        if !matches!(format, WEnum::Value(wl_keyboard::KeymapFormat::XkbV1)) {
            return;
        }
        let file = std::fs::File::from(fd);
        let mut text = String::new();
        // The size is the parent's, so it is capped rather than trusted: a keymap
        // larger than this is a bug or an attack, and neither should be a read of
        // whatever the descriptor points at.
        let limit = size.min(MAX_KEYMAP_BYTES);
        if file.take(u64::from(limit)).read_to_string(&mut text).is_ok() {
            self.keymap = Some(text.trim_end_matches('\0').to_owned());
        }
    }

    /// Creates the seat objects the parent says it has.
    fn note_capabilities(&mut self, qh: &QueueHandle<Self>, capabilities: wl_seat::Capability) {
        if capabilities.contains(wl_seat::Capability::Keyboard) {
            if self.keyboard.is_none() {
                self.keyboard = Some(self.seat_global.get_keyboard(qh, ()));
            }
        } else if let Some(keyboard) = self.keyboard.take() {
            keyboard.release();
            self.keymap = None;
        }
        if capabilities.contains(wl_seat::Capability::Pointer) {
            if self.pointer.is_none() {
                self.pointer = Some(self.seat_global.get_pointer(qh, ()));
            }
        } else if let Some(pointer) = self.pointer.take() {
            pointer.release();
        }
        if capabilities.contains(wl_seat::Capability::Touch) {
            if self.touch.is_none() {
                self.touch = Some(self.seat_global.get_touch(qh, ()));
            }
        } else if let Some(touch) = self.touch.take() {
            touch.release();
        }
    }
}

/// The largest keymap this backend will read from the parent.
///
/// A real `xkb_v1` keymap for a large layout is tens of kilobytes; this is three
/// orders of magnitude above that, so the limit never truncates a working keymap and
/// always bounds a hostile one.
const MAX_KEYMAP_BYTES: u32 = 4 * 1024 * 1024;

/// Whether a pointer axis event is horizontal.
fn is_horizontal(axis: &WEnum<wl_pointer::Axis>) -> bool {
    matches!(axis, WEnum::Value(wl_pointer::Axis::HorizontalScroll))
}

// The interfaces this backend binds and only ever listens to, or binds and never
// hears anything from.
wayland_client::delegate_noop!(Handler: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Handler: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Handler: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Handler: ignore wp_viewporter::WpViewporter);
wayland_client::delegate_noop!(Handler: ignore wp_viewport::WpViewport);
wayland_client::delegate_noop!(Handler: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Handler {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` keeps the global list in the object's data.
    }
}

impl Dispatch<wl_shm::WlShm, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_shm::WlShm,
        event: <wl_shm::WlShm as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_shm::Event::Format {
            format: WEnum::Value(format),
        } = event
        {
            state.note_format(format);
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: <wl_output::WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => {
                // The parent's integer scale is the fallback for a surface it has
                // no fractional scale for. The largest one it reports is the one a
                // session on that output would compose at.
                state.integer_scale = state.integer_scale.max(factor.max(1));
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                refresh,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                state.refresh_mhz = u32::try_from(refresh).unwrap_or(0);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            state.note_capabilities(qh, capabilities);
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => state.note_keymap(&format, fd, size),
            wl_keyboard::Event::Key {
                key, state: key_state, ..
            } => match key_state {
                WEnum::Value(wl_keyboard::KeyState::Pressed) => state.table.seat().key(key, true),
                WEnum::Value(wl_keyboard::KeyState::Released) => state.table.seat().key(key, false),
                _ => {}
            },
            // Focus is the session's own: the parent says which of *its* surfaces
            // holds its keyboard focus, which is this session as a whole.
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: <wl_pointer::WlPointer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                if let Some(output) = state.output_of(&surface) {
                    state.table.seat().enter(output, position(surface_x, surface_y));
                }
            }
            wl_pointer::Event::Leave { .. } => state.table.seat().leave(),
            wl_pointer::Event::Motion {
                surface_x, surface_y, ..
            } => {
                state.table.seat().motion(position(surface_x, surface_y));
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } => match button_state {
                WEnum::Value(wl_pointer::ButtonState::Pressed) => state.table.seat().button(button, true),
                WEnum::Value(wl_pointer::ButtonState::Released) => state.table.seat().button(button, false),
                _ => {}
            },
            wl_pointer::Event::Axis { axis, value, .. } => {
                state.table.seat().axis(is_horizontal(&axis), value);
            }
            wl_pointer::Event::AxisDiscrete { axis, discrete } => {
                // `wl_pointer.axis_discrete` counts whole steps and is deprecated in
                // favour of `axis_value120`, which states the same movement at a
                // finer resolution. A step is 120 of those units.
                state
                    .table
                    .seat()
                    .axis_discrete(is_horizontal(&axis), discrete.saturating_mul(super::input::AXIS_STEP));
            }
            wl_pointer::Event::AxisValue120 { axis, value120 } => {
                state.table.seat().axis_discrete(is_horizontal(&axis), value120);
            }
            // A frame ends the events of one input timestamp, which is when the
            // accumulated scroll becomes one event.
            wl_pointer::Event::Frame => state.table.seat().frame(),
            _ => {}
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: <wl_touch::WlTouch as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down { surface, id, x, y, .. } => {
                let output = state.output_of(&surface);
                state.table.seat().touch_down(id.unsigned_abs(), output, position(x, y));
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                state.table.seat().touch_motion(id.unsigned_abs(), position(x, y));
            }
            wl_touch::Event::Up { id, .. } => state.table.seat().touch_up(id.unsigned_abs()),
            wl_touch::Event::Cancel => state.table.seat().touch_cancel(),
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, SlotKey> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: <wl_buffer::WlBuffer as Proxy>::Event,
        key: &SlotKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            state.table.released(key.index, key.generation, key.slot);
        }
    }
}

impl Dispatch<wl_callback::WlCallback, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: <wl_callback::WlCallback as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.table.frame_done(key.0);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Handler {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A parent that does not get its ping answered may treat the session as
        // unresponsive, so the answer is sent here rather than queued.
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, OutputKey> for Handler {
    fn event(
        _: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The size the configure belongs to arrives in `xdg_toplevel.configure` in
        // the same batch, and the target applies it before its next commit, so the
        // acknowledgement is immediate and the buffer always matches what it
        // acknowledges.
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                state.table.configure(key.0, configured_size(width, height));
            }
            xdg_toplevel::Event::Close => state.table.close(key.0),
            _ => {}
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, OutputKey> for Handler {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: <wp_fractional_scale_v1::WpFractionalScaleV1 as Proxy>::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.table.prefer_scale(key.0, scale_from_parent(scale));
        }
    }
}

/// The size a `xdg_toplevel.configure` asks for.
///
/// xdg-shell states zero on both axes for "choose for yourself". A nested output
/// that chooses gets [`UNCONSTRAINED_SIZE`]: it has to draw at *some* size, and a
/// size of no pixels is an output that can never show a window.
fn configured_size(width: i32, height: i32) -> Size {
    if width > 0 && height > 0 {
        Size::new(width.unsigned_abs(), height.unsigned_abs())
    } else {
        UNCONSTRAINED_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configure_size_is_the_size_the_parent_asked_for() {
        assert_eq!(configured_size(1024, 768), Size::new(1024, 768));
    }

    #[test]
    fn a_configure_that_leaves_the_size_to_us_is_not_an_empty_output() {
        assert_eq!(configured_size(0, 0), UNCONSTRAINED_SIZE);
        // One axis stated and one left out is not a size either.
        assert_eq!(configured_size(1024, 0), UNCONSTRAINED_SIZE);
        assert_eq!(configured_size(-1, 768), UNCONSTRAINED_SIZE);
    }

    #[test]
    fn an_horizontal_axis_is_the_one_the_protocol_calls_horizontal() {
        assert!(is_horizontal(&WEnum::Value(wl_pointer::Axis::HorizontalScroll)));
        assert!(!is_horizontal(&WEnum::Value(wl_pointer::Axis::VerticalScroll)));
        assert!(!is_horizontal(&WEnum::Unknown(7)));
    }
}
